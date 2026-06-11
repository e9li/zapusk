use anyhow::Result;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::core::caddy;
use crate::core::config::{
    Config, IgnoredService, ProjectConfig, ProjectType, config_path, parse_aliases,
};
use crate::core::discovery::ServiceInfo;
use crate::core::discovery::{StackKind, discover_services};
use crate::core::manager::{Manager, ManagerEvent};
use crate::core::project::{LogEntry, ProcessOrigin, Project, ProjectStatus};

/// Which pane is focused
#[derive(Debug, Clone, PartialEq)]
pub enum ActivePane {
    ProjectList,
    Logs,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceState {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartupPhase {
    EnsuringCaddy,
    StartingProcess,
    VerifyingDomain,
}

#[derive(Debug)]
enum BackgroundEvent {
    UnmanagedRefreshed(Result<Vec<ServiceInfo>, String>),
    ServiceStatesRefreshed {
        caddy: ServiceState,
        dnsmasq: ServiceState,
    },
    DomainVerificationDone {
        project_name: String,
        result: Result<u16, String>,
    },
}

fn default_web_port_rules() -> Vec<String> {
    vec![
        "80".into(),
        "443".into(),
        "8080".into(),
        "8443".into(),
        "3000-9999".into(),
    ]
}

fn matches_port_rule(port: u16, rules: &[String]) -> bool {
    for rule in rules {
        let r = rule.trim();
        if r.is_empty() {
            continue;
        }

        if let Some((start, end)) = r.split_once('-') {
            let Ok(start) = start.trim().parse::<u16>() else {
                continue;
            };
            let Ok(end) = end.trim().parse::<u16>() else {
                continue;
            };
            if start <= end && (start..=end).contains(&port) {
                return true;
            }
            continue;
        }

        if let Ok(single) = r.parse::<u16>() {
            if single == port {
                return true;
            }
        }
    }

    false
}

fn parse_upstream_host(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn is_valid_upstream_host(host: &str) -> bool {
    !host.contains('/') && !host.chars().any(|c| c.is_whitespace()) && !host.is_empty()
}

/// What happens when the user confirms
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    StopProject(String),
    RemoveProject(String),
}

/// Inline add-project form fields
#[derive(Debug, Clone)]
pub enum AddField {
    Name,
    Domain,
    Aliases,
    Port,
    UpstreamHost,
    Type,
    Tls,
    Path,
}

pub const TYPE_OPTIONS: &[&str] = &["phoenix", "symfony", "kirby", "axum", "compose"];

/// State for the inline add-project form
#[derive(Debug, Clone)]
pub struct AddForm {
    pub field: AddField,
    pub name: String,
    pub domain: String,
    pub aliases: String,
    pub port: String,
    pub upstream_host: String,
    pub type_index: usize,
    pub tls: bool,
    pub path: String,
}

#[derive(Debug, Clone)]
pub struct EditForm {
    pub project_index: usize,
    pub field: AddField,
    pub name: String,
    pub domain: String,
    pub aliases: String,
    pub port: String,
    pub upstream_host: String,
    pub type_index: usize,
    pub tls: bool,
    pub path: String,
}

impl AddForm {
    pub fn new() -> Self {
        Self {
            field: AddField::Name,
            name: String::new(),
            domain: String::new(),
            aliases: String::new(),
            port: String::new(),
            upstream_host: String::new(),
            type_index: 0,
            tls: false,
            path: String::new(),
        }
    }

    pub fn project_type(&self) -> &str {
        TYPE_OPTIONS[self.type_index]
    }

    pub fn cycle_type_next(&mut self) {
        self.type_index = (self.type_index + 1) % TYPE_OPTIONS.len();
    }

    pub fn cycle_type_prev(&mut self) {
        if self.type_index == 0 {
            self.type_index = TYPE_OPTIONS.len() - 1;
        } else {
            self.type_index -= 1;
        }
    }

    pub fn toggle_tls(&mut self) {
        self.tls = !self.tls;
    }

    pub fn current_value(&self) -> &str {
        match self.field {
            AddField::Name => &self.name,
            AddField::Domain => &self.domain,
            AddField::Aliases => &self.aliases,
            AddField::Port => &self.port,
            AddField::UpstreamHost => &self.upstream_host,
            AddField::Type => self.project_type(),
            AddField::Tls => {
                if self.tls {
                    "on"
                } else {
                    "off"
                }
            }
            AddField::Path => &self.path,
        }
    }

    pub fn current_value_mut(&mut self) -> &mut String {
        match self.field {
            AddField::Name => &mut self.name,
            AddField::Domain => &mut self.domain,
            AddField::Aliases => &mut self.aliases,
            AddField::Port => &mut self.port,
            AddField::UpstreamHost => &mut self.upstream_host,
            AddField::Type => unreachable!("Type field uses cycle, not freetext"),
            AddField::Tls => unreachable!("TLS field uses toggle, not freetext"),
            AddField::Path => &mut self.path,
        }
    }

    pub fn label(&self) -> &str {
        match self.field {
            AddField::Name => "Name",
            AddField::Domain => "Domain",
            AddField::Aliases => "Aliases (comma-separated)",
            AddField::Port => "Port",
            AddField::UpstreamHost => "Upstream host",
            AddField::Type => "Type",
            AddField::Tls => "TLS",
            AddField::Path => "Project directory",
        }
    }

    /// Advance to next field, returns true if form is complete
    pub fn next_field(&mut self, tld: &str) -> bool {
        match self.field {
            AddField::Name => {
                if self.domain.is_empty() {
                    let slug = crate::core::slugify(&self.name);
                    self.domain = format!("{}.{}", slug, tld);
                }
                self.field = AddField::Domain;
                false
            }
            AddField::Domain => {
                self.field = AddField::Aliases;
                false
            }
            AddField::Aliases => {
                self.field = AddField::Port;
                false
            }
            AddField::Port => {
                self.field = AddField::UpstreamHost;
                false
            }
            AddField::UpstreamHost => {
                self.field = AddField::Type;
                false
            }
            AddField::Type => {
                self.field = AddField::Tls;
                false
            }
            AddField::Tls => {
                self.field = AddField::Path;
                false
            }
            AddField::Path => true,
        }
    }
}

impl EditForm {
    pub fn from_project(project_index: usize, project: &ProjectConfig) -> Self {
        let type_index = TYPE_OPTIONS
            .iter()
            .position(|t| *t == project.project_type.label())
            .unwrap_or(0);

        Self {
            project_index,
            field: AddField::Name,
            name: project.name.clone(),
            domain: project.domain.clone(),
            aliases: project.aliases.join(", "),
            port: project.port.to_string(),
            upstream_host: project.upstream_host.clone().unwrap_or_default(),
            type_index,
            tls: project.tls,
            path: project.path.clone(),
        }
    }

    pub fn project_type(&self) -> &str {
        TYPE_OPTIONS[self.type_index]
    }

    pub fn cycle_type_next(&mut self) {
        self.type_index = (self.type_index + 1) % TYPE_OPTIONS.len();
    }

    pub fn cycle_type_prev(&mut self) {
        if self.type_index == 0 {
            self.type_index = TYPE_OPTIONS.len() - 1;
        } else {
            self.type_index -= 1;
        }
    }

    pub fn toggle_tls(&mut self) {
        self.tls = !self.tls;
    }

    pub fn current_value(&self) -> &str {
        match self.field {
            AddField::Name => &self.name,
            AddField::Domain => &self.domain,
            AddField::Aliases => &self.aliases,
            AddField::Port => &self.port,
            AddField::UpstreamHost => &self.upstream_host,
            AddField::Type => self.project_type(),
            AddField::Tls => {
                if self.tls {
                    "on"
                } else {
                    "off"
                }
            }
            AddField::Path => &self.path,
        }
    }

    pub fn current_value_mut(&mut self) -> &mut String {
        match self.field {
            AddField::Name => &mut self.name,
            AddField::Domain => &mut self.domain,
            AddField::Aliases => &mut self.aliases,
            AddField::Port => &mut self.port,
            AddField::UpstreamHost => &mut self.upstream_host,
            AddField::Type => unreachable!("Type field uses cycle, not freetext"),
            AddField::Tls => unreachable!("TLS field uses toggle, not freetext"),
            AddField::Path => &mut self.path,
        }
    }

    pub fn label(&self) -> &str {
        match self.field {
            AddField::Name => "Name",
            AddField::Domain => "Domain",
            AddField::Aliases => "Aliases (comma-separated)",
            AddField::Port => "Port",
            AddField::UpstreamHost => "Upstream host",
            AddField::Type => "Type",
            AddField::Tls => "TLS",
            AddField::Path => "Project directory",
        }
    }

    pub fn next_field(&mut self) -> bool {
        match self.field {
            AddField::Name => {
                self.field = AddField::Domain;
                false
            }
            AddField::Domain => {
                self.field = AddField::Aliases;
                false
            }
            AddField::Aliases => {
                self.field = AddField::Port;
                false
            }
            AddField::Port => {
                self.field = AddField::UpstreamHost;
                false
            }
            AddField::UpstreamHost => {
                self.field = AddField::Type;
                false
            }
            AddField::Type => {
                self.field = AddField::Tls;
                false
            }
            AddField::Tls => {
                self.field = AddField::Path;
                false
            }
            AddField::Path => true,
        }
    }
}

/// A pending confirmation dialog
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub message: String,
    pub action: ConfirmAction,
}

/// Top-level application state
pub struct App {
    pub projects: Vec<Project>,
    pub selected: usize,
    pub active_pane: ActivePane,
    pub config: Config,
    /// Status bar message (shown at bottom)
    pub status_message: Option<String>,
    pub should_quit: bool,
    /// Log scroll offset: 0 = pinned to bottom (auto-scroll)
    pub log_scroll_offset: usize,
    /// Search mode active
    pub search_mode: bool,
    /// Current search query
    pub search_query: String,
    /// Show project detail popup
    pub show_detail_popup: bool,
    /// Pending confirmation dialog
    pub confirm_dialog: Option<ConfirmDialog>,
    /// Show help popup
    pub show_help: bool,
    /// Inline add-project form
    pub add_form: Option<AddForm>,
    /// Inline edit-project form
    pub edit_form: Option<EditForm>,
    /// Unmanaged discovered services
    pub unmanaged_all_services: Vec<ServiceInfo>,
    pub unmanaged_services: Vec<ServiceInfo>,
    pub unmanaged_selected: usize,
    pub unmanaged_show_unknown: bool,
    pub unmanaged_web_only: bool,
    pub show_unmanaged_popup: bool,
    pub show_unmanaged_detail: bool,
    last_discovery_refresh: Instant,
    last_service_refresh: Instant,
    pub caddy_state: ServiceState,
    pub dnsmasq_state: ServiceState,
    startup_phases: HashMap<String, StartupPhase>,
    spinner_frame: usize,
    discovery_refresh_in_flight: bool,
    service_refresh_in_flight: bool,
    app_event_tx: mpsc::Sender<BackgroundEvent>,
    app_event_rx: mpsc::Receiver<BackgroundEvent>,
    pub(crate) manager: Manager,
    pub(crate) event_rx: mpsc::Receiver<ManagerEvent>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let (app_event_tx, app_event_rx) = mpsc::channel(64);

        let projects = config.projects.iter().cloned().map(Project::new).collect();
        crate::tui::ui::init_theme(config.theme.as_ref());

        Self {
            projects,
            selected: 0,
            active_pane: ActivePane::ProjectList,
            config,
            status_message: None,
            should_quit: false,
            log_scroll_offset: 0,
            search_mode: false,
            search_query: String::new(),
            show_detail_popup: false,
            confirm_dialog: None,
            show_help: false,
            add_form: None,
            edit_form: None,
            unmanaged_all_services: vec![],
            unmanaged_services: vec![],
            unmanaged_selected: 0,
            unmanaged_show_unknown: false,
            unmanaged_web_only: false,
            show_unmanaged_popup: false,
            show_unmanaged_detail: false,
            last_discovery_refresh: Instant::now(),
            last_service_refresh: Instant::now(),
            caddy_state: ServiceState::Stopped,
            dnsmasq_state: ServiceState::Stopped,
            startup_phases: HashMap::new(),
            spinner_frame: 0,
            discovery_refresh_in_flight: false,
            service_refresh_in_flight: false,
            app_event_tx,
            app_event_rx,
            manager: Manager::new(tx),
            event_rx: rx,
        }
    }

    /// Main event loop tick — call this repeatedly from main
    pub async fn tick(&mut self) -> Result<()> {
        let tick_started = Instant::now();
        self.spinner_frame = self.spinner_frame.wrapping_add(1);

        // Poll child processes for exit (non-blocking)
        for event in self.manager.poll_exits() {
            self.handle_manager_event(event);
        }

        // Drain manager events (non-blocking)
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_manager_event(event);
        }

        // Drain app background events (non-blocking)
        while let Ok(event) = self.app_event_rx.try_recv() {
            self.handle_background_event(event);
        }

        self.schedule_background_refreshes();

        // Handle keyboard input (100ms timeout so we don't busy-loop)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Always handle Ctrl+C
                if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.quit().await;
                    return Ok(());
                }
                self.handle_key(key).await?;
            }
        }

        let elapsed = tick_started.elapsed();
        if elapsed > Duration::from_millis(250) {
            app_diag_log(&format!("slow tick: {}ms", elapsed.as_millis()));
        }

        Ok(())
    }

    fn schedule_background_refreshes(&mut self) {
        if self.last_discovery_refresh.elapsed() >= Duration::from_secs(10)
            && !self.discovery_refresh_in_flight
        {
            self.discovery_refresh_in_flight = true;
            self.last_discovery_refresh = Instant::now();
            let cfg = self.config.clone();
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                let result = discover_services(Some(&cfg))
                    .await
                    .map_err(|e| e.to_string());
                let _ = tx.send(BackgroundEvent::UnmanagedRefreshed(result)).await;
            });
        }

        if self.last_service_refresh.elapsed() >= Duration::from_secs(6)
            && !self.service_refresh_in_flight
        {
            self.service_refresh_in_flight = true;
            self.last_service_refresh = Instant::now();
            let tx = self.app_event_tx.clone();
            let tld = self.config.tld.clone();
            tokio::spawn(async move {
                let caddy = detect_caddy_state().await;
                let dnsmasq = detect_dnsmasq_state(&tld).await;
                let _ = tx
                    .send(BackgroundEvent::ServiceStatesRefreshed { caddy, dnsmasq })
                    .await;
            });
        }
    }

    fn handle_background_event(&mut self, event: BackgroundEvent) {
        match event {
            BackgroundEvent::UnmanagedRefreshed(result) => {
                self.discovery_refresh_in_flight = false;
                match result {
                    Ok(services) => {
                        self.unmanaged_all_services =
                            services.into_iter().filter(|s| !s.managed).collect();
                        self.apply_unmanaged_filter();
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Discovery failed: {}", e));
                        app_diag_log(&format!("discovery refresh failed: {}", e));
                    }
                }
            }
            BackgroundEvent::ServiceStatesRefreshed { caddy, dnsmasq } => {
                self.service_refresh_in_flight = false;
                self.caddy_state = caddy;
                self.dnsmasq_state = dnsmasq;
            }
            BackgroundEvent::DomainVerificationDone {
                project_name,
                result,
            } => {
                self.startup_phases.remove(&project_name);
                if let Some(project) = self.find_project_mut(&project_name) {
                    if matches!(project.status, ProjectStatus::Starting) {
                        project.status = ProjectStatus::Running;
                    }
                }

                match result {
                    Ok(code) => {
                        self.append_system_log(
                            &project_name,
                            format!("domain reachable (HTTP {})", code),
                            false,
                        );
                        self.status_message = Some(format!(
                            "{} started - domain reachable (HTTP {})",
                            project_name, code
                        ));
                    }
                    Err(reason) => {
                        self.append_system_log(
                            &project_name,
                            format!("domain check failed: {}", reason),
                            true,
                        );
                        self.status_message = Some(format!(
                            "{} started, but domain check failed: {}",
                            project_name, reason
                        ));
                    }
                }
            }
        }
    }

    pub fn startup_phase_label(&self, project_name: &str) -> Option<&'static str> {
        self.startup_phases
            .get(project_name)
            .map(|phase| match phase {
                StartupPhase::EnsuringCaddy => "caddy",
                StartupPhase::StartingProcess => "spawn",
                StartupPhase::VerifyingDomain => "verify",
            })
    }

    pub fn spinner_glyph(&self) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[self.spinner_frame % FRAMES.len()]
    }

    fn handle_manager_event(&mut self, event: ManagerEvent) {
        match event {
            ManagerEvent::LogLine {
                project_name,
                line,
                is_stderr,
            } => {
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.add_log(line, is_stderr);
                }
            }
            ManagerEvent::ProcessStarted {
                project_name,
                pid,
                adopted,
            } => {
                let keep_starting = self.startup_phases.contains_key(&project_name);
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.status = if keep_starting {
                        ProjectStatus::Starting
                    } else {
                        ProjectStatus::Running
                    };
                    p.pid = Some(pid);
                    p.origin = Some(if adopted {
                        ProcessOrigin::Adopted
                    } else {
                        ProcessOrigin::Managed
                    });
                    p.started_at = Some(Local::now());
                }
                self.status_message = Some(if adopted {
                    format!(
                        "{} conflict: port already in use, adopted existing process (pid {})",
                        project_name, pid
                    )
                } else if keep_starting {
                    format!("{}: process started, verifying domain...", project_name)
                } else {
                    format!("{} started", project_name)
                });
            }
            ManagerEvent::ProcessExited {
                project_name,
                success,
            } => {
                self.manager.mark_exited(&project_name);
                self.startup_phases.remove(&project_name);
                if let Some(p) = self.find_project_mut(&project_name) {
                    // If already Stopped (set by stop_project), don't override to Failed.
                    // Signal-killed processes exit non-success, which is expected on stop.
                    if p.status != ProjectStatus::Stopped {
                        p.status = if success {
                            ProjectStatus::Stopped
                        } else {
                            ProjectStatus::Failed("exited with error".into())
                        };
                    }
                    p.pid = None;
                    p.origin = None;
                    p.started_at = None;
                }
            }
        }
    }

    pub(crate) fn find_project_mut(&mut self, name: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.config.name == name)
    }

    pub fn selected_project(&self) -> Option<&Project> {
        self.projects.get(self.selected)
    }

    pub fn selected_unmanaged(&self) -> Option<&ServiceInfo> {
        self.unmanaged_services.get(self.unmanaged_selected)
    }

    fn append_system_log(
        &mut self,
        project_name: &str,
        message: impl Into<String>,
        is_stderr: bool,
    ) {
        if let Some(p) = self.find_project_mut(project_name) {
            p.add_log(format!("[zapusk] {}", message.into()), is_stderr);
        }
    }

    pub(crate) fn select_next(&mut self) {
        if !self.projects.is_empty() {
            self.selected = (self.selected + 1) % self.projects.len();
            self.log_scroll_offset = 0;
        }
    }

    pub(crate) fn select_prev(&mut self) {
        if !self.projects.is_empty() {
            if self.selected == 0 {
                self.selected = self.projects.len() - 1;
            } else {
                self.selected -= 1;
            }
            self.log_scroll_offset = 0;
        }
    }

    pub(crate) fn toggle_pane(&mut self) {
        self.active_pane = match self.active_pane {
            ActivePane::ProjectList => ActivePane::Logs,
            ActivePane::Logs => ActivePane::ProjectList,
        };
    }

    pub(crate) fn scroll_logs_up(&mut self, amount: usize) {
        let max = self.selected_logs().len();
        self.log_scroll_offset = (self.log_scroll_offset + amount).min(max);
    }

    pub(crate) fn scroll_logs_down(&mut self, amount: usize) {
        self.log_scroll_offset = self.log_scroll_offset.saturating_sub(amount);
    }

    pub(crate) async fn start_selected(&mut self) {
        if let Some(project) = self.projects.get(self.selected) {
            let config = project.config.clone();
            let name = config.name.clone();

            if let Some(p) = self.find_project_mut(&name) {
                p.status = ProjectStatus::Starting;
            }
            self.startup_phases
                .insert(name.clone(), StartupPhase::EnsuringCaddy);

            // Ensure Caddy is running with current config before starting the project
            self.append_system_log(&name, "step 1/3: ensuring Caddy config", false);
            self.status_message = Some(format!("{}: step 1/3 ensuring Caddy config...", name));
            if let Some(caddy_err) = self.ensure_caddy().await {
                self.append_system_log(&name, caddy_err, true);
            }

            self.startup_phases
                .insert(name.clone(), StartupPhase::StartingProcess);
            match self.manager.start(&config).await {
                Ok(_status) => {
                    if let Some(p) = self.find_project_mut(&name) {
                        p.status = ProjectStatus::Starting;
                        p.origin = Some(ProcessOrigin::Managed);
                    }

                    self.append_system_log(&name, "step 2/3: process start requested", false);

                    self.startup_phases
                        .insert(name.clone(), StartupPhase::VerifyingDomain);
                    self.append_system_log(&name, "step 3/3: verifying domain with curl", false);
                    self.status_message = Some(format!("{}: step 3/3 verifying domain...", name));

                    let tx = self.app_event_tx.clone();
                    tokio::spawn(async move {
                        let result = verify_project_domain_static(&config).await;
                        let _ = tx
                            .send(BackgroundEvent::DomainVerificationDone {
                                project_name: name,
                                result,
                            })
                            .await;
                    });
                }
                Err(e) => {
                    self.startup_phases.remove(&name);
                    if let Some(p) = self.find_project_mut(&name) {
                        p.status = ProjectStatus::Failed(e.to_string());
                    }
                    self.append_system_log(&name, format!("start failed: {}", e), true);
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
        }
    }

    /// Ensure Caddy is running with the current Caddyfile.
    /// Writes the Caddyfile and starts/reloads Caddy silently.
    /// Returns the error message if Caddy reload failed, so callers can log it.
    async fn ensure_caddy(&mut self) -> Option<String> {
        if let Some(caddy_cfg) = &self.config.caddy.clone() {
            let projects: Vec<_> = self.projects.iter().map(|p| p.config.clone()).collect();
            if let Err(e) = caddy::write_and_reload(&projects, caddy_cfg).await {
                let msg = format!("Caddy warning: {}", e);
                self.status_message = Some(msg.clone());
                return Some(msg);
            }
        }
        None
    }

    pub(crate) async fn stop_project(&mut self, name: &str) {
        let name = name.to_string();
        self.startup_phases.remove(&name);
        match self.manager.stop(&name).await {
            Ok(()) => {
                if let Some(p) = self.find_project_mut(&name) {
                    p.status = ProjectStatus::Stopped;
                    p.pid = None;
                    p.origin = None;
                    p.started_at = None;
                }
                self.status_message = Some(format!("Stopped {}", name));
            }
            Err(e) => {
                self.status_message = Some(format!("Error: {}", e));
            }
        }
    }

    pub(crate) async fn restart_selected(&mut self) {
        if let Some(project) = self.projects.get(self.selected) {
            let name = project.config.name.clone();
            self.stop_project(&name).await;
            // Small delay so the port is released before restarting
            tokio::time::sleep(Duration::from_millis(500)).await;
            self.start_selected().await;
        }
    }

    pub(crate) async fn reload_caddy(&mut self) {
        if let Some(caddy_cfg) = &self.config.caddy.clone() {
            let projects: Vec<_> = self.projects.iter().map(|p| p.config.clone()).collect();
            match caddy::write_and_reload(&projects, caddy_cfg).await {
                Ok(()) => self.status_message = Some("Caddy reloaded".into()),
                Err(e) => self.status_message = Some(format!("Caddy error: {}", e)),
            }
        } else {
            self.status_message = Some("No [caddy] section in config".into());
        }
    }

    /// Detect projects that are already running (port in use) and adopt them.
    /// Called on TUI startup before autostart.
    pub async fn detect_running(&mut self) {
        let mut adopted = 0;
        for i in 0..self.projects.len() {
            let config = self.projects[i].config.clone();
            if let Some(pid) = self.manager.detect_running(&config).await {
                self.projects[i].status = ProjectStatus::Running;
                self.projects[i].pid = Some(pid);
                self.projects[i].origin = Some(ProcessOrigin::Adopted);
                self.projects[i].started_at = Some(Local::now());
                adopted += 1;
            }
        }
        if adopted > 0 {
            self.status_message = Some(format!("Detected {} running project(s)", adopted));
        }
    }

    /// Start all projects with autostart = true
    pub async fn autostart(&mut self) {
        let autostart_names: Vec<String> = self
            .projects
            .iter()
            .filter(|p| p.config.autostart)
            .map(|p| p.config.name.clone())
            .collect();

        for name in autostart_names {
            if let Some(idx) = self.projects.iter().position(|p| p.config.name == name) {
                let config = self.projects[idx].config.clone();
                match self.manager.start(&config).await {
                    Ok(status) => {
                        self.projects[idx].status = status;
                        self.projects[idx].origin = Some(ProcessOrigin::Managed);
                        self.status_message = Some(format!("Autostarting {}…", name));
                    }
                    Err(e) => {
                        self.status_message = Some(format!("Autostart error: {}", e));
                    }
                }
            }
        }
    }

    pub(crate) fn confirm_stop_selected(&mut self) {
        if let Some(project) = self.selected_project() {
            if project.is_running() {
                self.confirm_dialog = Some(ConfirmDialog {
                    message: format!("Stop {}? (y/n)", project.config.name),
                    action: ConfirmAction::StopProject(project.config.name.clone()),
                });
            } else {
                self.status_message = Some(format!("{} is not running", project.config.name));
            }
        }
    }

    pub(crate) async fn execute_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::StopProject(name) => {
                self.stop_project(&name).await;
            }
            ConfirmAction::RemoveProject(name) => {
                self.remove_project(&name).await;
            }
        }
    }

    pub(crate) fn confirm_remove_selected(&mut self) {
        if let Some(project) = self.selected_project() {
            if project.is_running() {
                self.status_message = Some(format!(
                    "Stop {} first before removing",
                    project.config.name
                ));
                return;
            }
            self.confirm_dialog = Some(ConfirmDialog {
                message: format!("Remove {} from config? (y/n)", project.config.name),
                action: ConfirmAction::RemoveProject(project.config.name.clone()),
            });
        }
    }

    async fn remove_project(&mut self, name: &str) {
        // Remove from runtime
        self.projects.retain(|p| p.config.name != name);
        if self.projects.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.projects.len() {
            self.selected = self.projects.len() - 1;
        }

        // Rewrite config file
        if let Err(e) = self.save_config() {
            self.status_message =
                Some(format!("Removed from TUI but could not save config: {}", e));
            return;
        }

        // Update Caddyfile to remove the project's domain
        self.ensure_caddy().await;

        self.refresh_unmanaged().await;

        self.status_message = Some(format!("Removed {}", name));
    }

    pub(crate) async fn finalize_add(&mut self, form: AddForm) {
        let project_type = form
            .project_type()
            .parse::<ProjectType>()
            .unwrap_or(ProjectType::Phoenix);
        let port = match form.port.parse::<u16>() {
            Ok(0) => {
                self.status_message = Some("Port must be between 1 and 65535".into());
                return;
            }
            Ok(p) => p,
            Err(_) => {
                self.status_message = Some(format!(
                    "Invalid port '{}': must be a number 1-65535",
                    form.port
                ));
                return;
            }
        };

        if form.name.trim().is_empty() {
            self.status_message = Some("Name cannot be empty".into());
            return;
        }
        if form.domain.trim().is_empty() {
            self.status_message = Some("Domain cannot be empty".into());
            return;
        }
        if !std::path::Path::new(&form.path).is_dir() {
            self.status_message = Some(format!("Directory not found: {}", form.path));
            return;
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                self.status_message = Some(format!("Invalid upstream host: {}", host));
                return;
            }
        }
        if self.projects.iter().any(|p| p.config.name == form.name) {
            self.status_message = Some(format!("Project '{}' already exists", form.name));
            return;
        }
        let aliases = parse_aliases(&form.aliases);
        let hosts: Vec<&str> = std::iter::once(form.domain.as_str())
            .chain(aliases.iter().map(String::as_str))
            .collect();
        if let Some(err) = self.hostname_conflict(&hosts, None) {
            self.status_message = Some(err);
            return;
        }
        if self.projects.iter().any(|p| p.config.port == port) {
            self.status_message = Some(format!("Port {} is already used by another project", port));
            return;
        }

        let config = ProjectConfig {
            name: form.name.clone(),
            domain: form.domain,
            aliases,
            port,
            project_type,
            path: form.path,
            php_version: None,
            public_dir: None,
            command: None,
            compose_file: None,
            service: None,
            compose_profiles: vec![],
            upstream_host: parse_upstream_host(&form.upstream_host),
            args: vec![],
            env: Default::default(),
            autostart: false,
            tls: form.tls,
        };

        self.projects.push(Project::new(config));
        self.selected = self.projects.len() - 1;

        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Added but could not save config: {}", e));
            return;
        }

        // Update Caddyfile with the new project and start/reload Caddy
        self.ensure_caddy().await;
        self.refresh_unmanaged().await;

        self.status_message = Some(format!("Added {}", form.name));
    }

    /// Check that the given hostnames don't collide with other projects or
    /// with each other. `exclude_index`, when set, skips that project (for edits).
    fn hostname_conflict(
        &self,
        hosts: &[&str],
        exclude_index: Option<usize>,
    ) -> Option<String> {
        for i in 0..hosts.len() {
            for j in (i + 1)..hosts.len() {
                if hosts[i] == hosts[j] {
                    return Some(format!("Duplicate hostname '{}'", hosts[i]));
                }
            }
        }
        for (idx, project) in self.projects.iter().enumerate() {
            if Some(idx) == exclude_index {
                continue;
            }
            for existing in project.config.all_hostnames() {
                if hosts.iter().any(|h| *h == existing) {
                    return Some(format!(
                        "Domain '{}' is already used by '{}'",
                        existing, project.config.name
                    ));
                }
            }
        }
        None
    }

    pub fn add_form_error(&self, form: &AddForm) -> Option<String> {
        if form.name.trim().is_empty() {
            return Some("Name cannot be empty".into());
        }
        if self.projects.iter().any(|p| p.config.name == form.name) {
            return Some(format!("Project '{}' already exists", form.name));
        }

        if form.domain.trim().is_empty() {
            return Some("Domain cannot be empty".into());
        }
        let aliases = parse_aliases(&form.aliases);
        let hosts: Vec<&str> = std::iter::once(form.domain.as_str())
            .chain(aliases.iter().map(String::as_str))
            .collect();
        if let Some(err) = self.hostname_conflict(&hosts, None) {
            return Some(err);
        }

        let port = match form.port.parse::<u16>() {
            Ok(port) => port,
            Err(_) => return Some(format!("Invalid port: {}", form.port)),
        };
        if self.projects.iter().any(|p| p.config.port == port) {
            return Some(format!("Port {} is already used by another project", port));
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                return Some(format!("Invalid upstream host: {}", host));
            }
        }

        if form.path.trim().is_empty() {
            return Some("Project directory cannot be empty".into());
        }
        if !std::path::Path::new(&form.path).is_dir() {
            return Some(format!("Directory not found: {}", form.path));
        }

        None
    }

    pub(crate) fn start_edit_selected(&mut self) {
        let Some(project) = self.selected_project() else {
            self.status_message = Some("No project selected".into());
            return;
        };

        let running = project.is_running();
        let project_name = project.config.name.clone();
        let project_config = project.config.clone();

        if running {
            self.status_message = Some(format!("Stop {} before editing", project_name));
            return;
        }

        let idx = self.selected;
        self.add_form = None;
        self.edit_form = Some(EditForm::from_project(idx, &project_config));
        self.status_message = Some("Editing project...".into());
    }

    pub(crate) async fn finalize_edit(&mut self, form: EditForm) {
        if form.project_index >= self.projects.len() {
            self.status_message = Some("Project no longer exists".into());
            return;
        }

        let project_type = form
            .project_type()
            .parse::<ProjectType>()
            .unwrap_or(ProjectType::Phoenix);
        let port = match form.port.parse::<u16>() {
            Ok(0) => {
                self.status_message = Some("Port must be between 1 and 65535".into());
                return;
            }
            Ok(p) => p,
            Err(_) => {
                self.status_message = Some(format!(
                    "Invalid port '{}': must be a number 1-65535",
                    form.port
                ));
                return;
            }
        };

        if form.name.trim().is_empty() {
            self.status_message = Some("Name cannot be empty".into());
            return;
        }
        if form.domain.trim().is_empty() {
            self.status_message = Some("Domain cannot be empty".into());
            return;
        }
        if !std::path::Path::new(&form.path).is_dir() {
            self.status_message = Some(format!("Directory not found: {}", form.path));
            return;
        }

        for (idx, project) in self.projects.iter().enumerate() {
            if idx == form.project_index {
                continue;
            }
            if project.config.name == form.name {
                self.status_message = Some(format!("Project '{}' already exists", form.name));
                return;
            }
            if project.config.port == port {
                self.status_message =
                    Some(format!("Port {} is already used by another project", port));
                return;
            }
        }

        let aliases = parse_aliases(&form.aliases);
        let hosts: Vec<&str> = std::iter::once(form.domain.as_str())
            .chain(aliases.iter().map(String::as_str))
            .collect();
        if let Some(err) = self.hostname_conflict(&hosts, Some(form.project_index)) {
            self.status_message = Some(err);
            return;
        }

        let existing = self.projects[form.project_index].config.clone();
        let updated = ProjectConfig {
            name: form.name.clone(),
            domain: form.domain,
            aliases,
            port,
            project_type: project_type.clone(),
            path: form.path,
            php_version: if project_type == ProjectType::Kirby {
                existing.php_version.or_else(|| Some("8.3".into()))
            } else {
                None
            },
            public_dir: existing.public_dir,
            command: existing.command,
            compose_file: if project_type == ProjectType::Compose {
                existing.compose_file
            } else {
                None
            },
            service: if project_type == ProjectType::Compose {
                existing.service
            } else {
                None
            },
            compose_profiles: if project_type == ProjectType::Compose {
                existing.compose_profiles
            } else {
                vec![]
            },
            upstream_host: parse_upstream_host(&form.upstream_host),
            args: existing.args,
            env: existing.env,
            autostart: existing.autostart,
            tls: form.tls,
        };

        self.projects[form.project_index].config = updated;

        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Updated but could not save config: {}", e));
            return;
        }

        self.ensure_caddy().await;
        self.refresh_unmanaged().await;
        self.status_message = Some(format!("Updated {}", form.name));
    }

    pub fn edit_form_error(&self, form: &EditForm) -> Option<String> {
        if form.name.trim().is_empty() {
            return Some("Name cannot be empty".into());
        }
        if self
            .projects
            .iter()
            .enumerate()
            .any(|(idx, p)| idx != form.project_index && p.config.name == form.name)
        {
            return Some(format!("Project '{}' already exists", form.name));
        }

        if form.domain.trim().is_empty() {
            return Some("Domain cannot be empty".into());
        }
        let aliases = parse_aliases(&form.aliases);
        let hosts: Vec<&str> = std::iter::once(form.domain.as_str())
            .chain(aliases.iter().map(String::as_str))
            .collect();
        if let Some(err) = self.hostname_conflict(&hosts, Some(form.project_index)) {
            return Some(err);
        }

        let port = match form.port.parse::<u16>() {
            Ok(port) => port,
            Err(_) => return Some(format!("Invalid port: {}", form.port)),
        };
        if self
            .projects
            .iter()
            .enumerate()
            .any(|(idx, p)| idx != form.project_index && p.config.port == port)
        {
            return Some(format!("Port {} is already used by another project", port));
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                return Some(format!("Invalid upstream host: {}", host));
            }
        }

        if form.path.trim().is_empty() {
            return Some("Project directory cannot be empty".into());
        }
        if !std::path::Path::new(&form.path).is_dir() {
            return Some(format!("Directory not found: {}", form.path));
        }

        None
    }

    pub async fn refresh_unmanaged(&mut self) {
        match discover_services(Some(&self.config)).await {
            Ok(services) => {
                self.unmanaged_all_services = services.into_iter().filter(|s| !s.managed).collect();
                self.apply_unmanaged_filter();
            }
            Err(e) => {
                self.status_message = Some(format!("Discovery failed: {}", e));
            }
        }
    }

    pub async fn refresh_service_states(&mut self) {
        self.caddy_state = detect_caddy_state().await;
        self.dnsmasq_state = detect_dnsmasq_state(&self.config.tld).await;
    }

    fn apply_unmanaged_filter(&mut self) {
        let web_rules = self
            .config
            .discovery
            .as_ref()
            .map(|d| d.web_ports.clone())
            .unwrap_or_else(default_web_port_rules);

        self.unmanaged_services = self
            .unmanaged_all_services
            .iter()
            .filter(|s| self.unmanaged_show_unknown || !matches!(s.stack, StackKind::Unknown))
            .filter(|s| !self.unmanaged_web_only || matches_port_rule(s.port, &web_rules))
            .cloned()
            .collect();

        if self.unmanaged_services.is_empty() {
            self.unmanaged_selected = 0;
        } else if self.unmanaged_selected >= self.unmanaged_services.len() {
            self.unmanaged_selected = self.unmanaged_services.len() - 1;
        }
    }

    pub fn toggle_unmanaged_filter(&mut self) {
        self.unmanaged_show_unknown = !self.unmanaged_show_unknown;
        self.apply_unmanaged_filter();
        self.status_message = Some(if self.unmanaged_show_unknown {
            "Unmanaged filter: showing all stacks".into()
        } else {
            "Unmanaged filter: showing php/elixir/rust only".into()
        });
    }

    pub fn toggle_unmanaged_web_filter(&mut self) {
        self.unmanaged_web_only = !self.unmanaged_web_only;
        self.apply_unmanaged_filter();
        self.status_message = Some(if self.unmanaged_web_only {
            "Unmanaged filter: web-ish ports only".into()
        } else {
            "Unmanaged filter: all ports".into()
        });
    }

    pub fn select_unmanaged_next(&mut self) {
        if !self.unmanaged_services.is_empty() {
            self.unmanaged_selected = (self.unmanaged_selected + 1) % self.unmanaged_services.len();
        }
    }

    pub fn select_unmanaged_prev(&mut self) {
        if !self.unmanaged_services.is_empty() {
            if self.unmanaged_selected == 0 {
                self.unmanaged_selected = self.unmanaged_services.len() - 1;
            } else {
                self.unmanaged_selected -= 1;
            }
        }
    }

    pub async fn toggle_unmanaged_popup(&mut self) {
        if !self.show_unmanaged_popup {
            self.refresh_unmanaged().await;
        }
        self.show_unmanaged_popup = !self.show_unmanaged_popup;
        self.show_unmanaged_detail = false;
        if self.show_unmanaged_popup {
            self.status_message =
                Some("Unmanaged: Enter inspect, i import, I ignore, f stack, w ports".into());
        }
    }

    pub async fn import_selected_unmanaged(&mut self) {
        let Some(service) = self.selected_unmanaged().cloned() else {
            self.status_message = Some("No unmanaged service selected".into());
            return;
        };

        if service
            .cwd
            .as_ref()
            .map(|p| std::path::Path::new(p).is_dir())
            != Some(true)
        {
            self.status_message = Some(format!(
                "Cannot import pid {}: working directory unavailable",
                service.pid
            ));
            return;
        }
        if self.projects.iter().any(|p| p.config.port == service.port) {
            self.status_message = Some(format!("Port {} already exists in config", service.port));
            return;
        }

        let base_name = self.base_name_for_service(&service);
        let name = self.unique_project_name(&base_name);
        let domain = self.unique_domain_for_name(&name);
        let (project_type, php_version) = match service.stack {
            StackKind::Php => (ProjectType::Symfony, None),
            StackKind::Elixir => (ProjectType::Phoenix, None),
            StackKind::Rust => (ProjectType::Axum, None),
            StackKind::Unknown => (ProjectType::Axum, None),
        };

        let (command, args) = if let Some(cmdline) = &service.command_line {
            match shell_words::split(cmdline) {
                Ok(parts) if !parts.is_empty() => (Some(parts[0].clone()), parts[1..].to_vec()),
                _ => (Some(service.command.clone()), vec![]),
            }
        } else {
            (Some(service.command.clone()), vec![])
        };

        let project = ProjectConfig {
            name: name.clone(),
            domain,
            aliases: vec![],
            port: service.port,
            project_type,
            path: service.cwd.clone().unwrap_or_default(),
            php_version,
            public_dir: None,
            command,
            compose_file: None,
            service: None,
            compose_profiles: vec![],
            upstream_host: None,
            args,
            env: Default::default(),
            autostart: false,
            tls: false,
        };

        self.projects.push(Project::new(project));
        self.selected = self.projects.len().saturating_sub(1);

        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Imported but failed to save config: {}", e));
            return;
        }

        self.ensure_caddy().await;
        self.refresh_unmanaged().await;
        self.status_message = Some(format!("Imported {} from port {}", name, service.port));
    }

    pub async fn ignore_selected_unmanaged(&mut self) {
        let Some(service) = self.selected_unmanaged().cloned() else {
            self.status_message = Some("No unmanaged service selected".into());
            return;
        };

        let already =
            self.config.ignored_services.iter().any(|i| {
                i.port == service.port && i.command.eq_ignore_ascii_case(&service.command)
            });
        if !already {
            self.config.ignored_services.push(IgnoredService {
                port: service.port,
                command: service.command.clone(),
            });
        }

        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Failed to save ignore list: {}", e));
            return;
        }

        self.refresh_unmanaged().await;
        self.status_message = Some(format!(
            "Ignored {} on port {}",
            service.command, service.port
        ));
    }

    fn base_name_for_service(&self, service: &ServiceInfo) -> String {
        if let Some(cwd) = &service.cwd {
            if let Some(base) = std::path::Path::new(cwd)
                .file_name()
                .and_then(|s| s.to_str())
            {
                let slug = crate::core::slugify(base);
                if !slug.is_empty() {
                    return slug;
                }
            }
        }

        let from_cmd = crate::core::slugify(&service.command);
        if from_cmd.is_empty() {
            format!("service-{}", service.port)
        } else {
            format!("{}-{}", from_cmd, service.port)
        }
    }

    fn unique_project_name(&self, base: &str) -> String {
        if !self.projects.iter().any(|p| p.config.name == base) {
            return base.to_string();
        }

        for i in 2..=1000 {
            let candidate = format!("{}-{}", base, i);
            if !self.projects.iter().any(|p| p.config.name == candidate) {
                return candidate;
            }
        }
        format!("{}-{}", base, chrono::Local::now().timestamp())
    }

    fn unique_domain_for_name(&self, name: &str) -> String {
        let base = crate::core::slugify(name);
        let mut domain = format!("{}.{}", base, self.config.tld);
        if !self
            .projects
            .iter()
            .any(|p| p.config.all_hostnames().any(|h| h == domain))
        {
            return domain;
        }

        for i in 2..=1000 {
            domain = format!("{}-{}.{}", base, i, self.config.tld);
            if !self
                .projects
                .iter()
                .any(|p| p.config.all_hostnames().any(|h| h == domain))
            {
                return domain;
            }
        }
        format!(
            "{}-{}.{}",
            base,
            chrono::Local::now().timestamp(),
            self.config.tld
        )
    }

    /// Rewrite config.toml from current state.
    /// Uses atomic write (temp file + rename) to prevent corruption on interruption.
    fn save_config(&self) -> Result<()> {
        let path = config_path();
        let serialized = Config {
            tld: self.config.tld.clone(),
            projects: self.projects.iter().map(|p| p.config.clone()).collect(),
            caddy: self.config.caddy.clone(),
            discovery: self.config.discovery.clone(),
            ignored_services: self.config.ignored_services.clone(),
            theme: self.config.theme.clone(),
        };
        let mut out = String::from("# zapusk config\n\n");
        out.push_str(&toml::to_string_pretty(&serialized)?);

        // Write to a temp file in the same directory, then rename for atomicity
        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, &out)?;
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    pub(crate) async fn quit(&mut self) {
        self.status_message = Some("Exiting TUI (running processes stay running)…".into());
        self.should_quit = true;
    }

    pub(crate) async fn force_quit(&mut self) {
        self.status_message = Some("Force quitting: stopping projects/services...".into());
        self.manager.stop_all().await;

        let mut notes: Vec<String> = vec![];

        if let Some(caddy) = &self.config.caddy {
            let bin = caddy.caddy_bin.as_deref().unwrap_or("caddy");
            match Command::new(bin).arg("stop").output().await {
                Ok(out) if out.status.success() => notes.push("caddy stopped".into()),
                Ok(_) => notes.push("could not stop caddy".into()),
                Err(_) => notes.push("could not run caddy stop".into()),
            }
        }

        match stop_dnsmasq_best_effort().await {
            Some(note) => notes.push(note),
            None => notes.push("dnsmasq stop skipped".into()),
        }

        self.should_quit = true;
        self.status_message = Some(format!("Force quit complete: {}", notes.join(", ")));
    }

    /// Logs for the currently selected project, optionally filtered by search query
    pub fn selected_logs(&self) -> Vec<&LogEntry> {
        let logs: Vec<_> = self
            .selected_project()
            .map(|p| p.logs.iter().collect())
            .unwrap_or_default();

        if self.search_query.is_empty() {
            logs
        } else {
            let query = self.search_query.to_lowercase();
            logs.into_iter()
                .filter(|entry| entry.line.to_lowercase().contains(&query))
                .collect()
        }
    }
}

async fn stop_dnsmasq_best_effort() -> Option<String> {
    if cfg!(target_os = "macos") {
        return match Command::new("brew")
            .args(["services", "stop", "dnsmasq"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => Some("dnsmasq stop requested".into()),
            Ok(_) => Some("could not stop dnsmasq (try sudo brew services stop dnsmasq)".into()),
            Err(_) => Some("could not run brew services stop dnsmasq".into()),
        };
    }

    if cfg!(target_os = "linux") {
        return match Command::new("systemctl")
            .args(["stop", "dnsmasq"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => Some("dnsmasq stop requested".into()),
            Ok(_) => Some("could not stop dnsmasq (try sudo systemctl stop dnsmasq)".into()),
            Err(_) => Some("could not run systemctl stop dnsmasq".into()),
        };
    }

    None
}

async fn detect_caddy_state() -> ServiceState {
    let admin_up = TcpStream::connect("127.0.0.1:2019").is_ok();

    let on_port_80 = timeout(
        Duration::from_millis(1200),
        Command::new("lsof")
            .args(["-nP", "-iTCP:80", "-sTCP:LISTEN", "-Fpc"])
            .output(),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|o| {
        o.status.success()
            && String::from_utf8_lossy(&o.stdout).lines().any(|line| {
                line.strip_prefix('c')
                    .map(|c| c.contains("caddy"))
                    .unwrap_or(false)
            })
    })
    .unwrap_or(false);

    match (admin_up, on_port_80) {
        (true, true) => ServiceState::Running,
        (true, false) | (false, true) => ServiceState::Paused,
        (false, false) => ServiceState::Stopped,
    }
}

async fn detect_dnsmasq_state(tld: &str) -> ServiceState {
    let running = timeout(
        Duration::from_millis(1200),
        Command::new("pgrep").args(["-x", "dnsmasq"]).output(),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .map(|o| o.status.success())
    .unwrap_or(false);

    if !running {
        return ServiceState::Stopped;
    }

    let test_host = format!("zapusk-health.{}", tld);
    let resolves = timeout(
        Duration::from_millis(1500),
        Command::new("dig")
            .args(["+short", &test_host, "@127.0.0.1"])
            .output(),
    )
    .await
    .ok()
    .and_then(|r| r.ok())
    .and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    })
    .map(|ip| ip == "127.0.0.1")
    .unwrap_or(false);

    if resolves {
        ServiceState::Running
    } else {
        ServiceState::Paused
    }
}

async fn verify_project_domain_static(config: &ProjectConfig) -> Result<u16, String> {
    let scheme = if config.tls { "https" } else { "http" };
    let url = format!("{}://{}", scheme, config.domain);
    let mut last_error = String::from("unreachable");

    // Compose stacks take longer to come up (image pulls, db init) than
    // native processes — give them a much wider verification window.
    let attempts = if config.project_type == ProjectType::Compose {
        40
    } else {
        8
    };

    for _ in 0..attempts {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{http_code}")
            .arg("--max-time")
            .arg("2");

        if config.tls {
            cmd.arg("-k");
        }

        let output = timeout(Duration::from_secs(3), cmd.arg(&url).output()).await;

        match output {
            Ok(Ok(out)) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if let Ok(code) = text.parse::<u16>() {
                    if code > 0 {
                        return Ok(code);
                    }
                }
                last_error = format!("unexpected curl output: {}", text);
            }
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if !stderr.is_empty() {
                    last_error = stderr;
                }
            }
            Ok(Err(e)) => {
                last_error = e.to_string();
            }
            Err(_) => {
                last_error = "curl timed out".into();
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(last_error)
}

fn app_diag_log(message: &str) {
    let path = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".config/zapusk/logs/zapusk.app.log");

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{}] {}", ts, message);
    }
}
