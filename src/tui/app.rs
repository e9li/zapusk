use anyhow::Result;
use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::core::caddy;
use crate::core::config::{
    Config, IgnoredService, ProjectConfig, ThemeConfig, config_path, hash_config_bytes,
    parse_aliases,
};
use crate::core::discovery::ServiceInfo;
use crate::core::discovery::discover_services;
use crate::core::framework::{FrameworkId, FrameworkRegistry};
use crate::core::manager::{Manager, ManagerEvent};
use crate::core::project::{LogEntry, ProcessOrigin, Project, ProjectStatus};
use crate::i18n::{Language, Msg, fill};
use crate::tui::theme::{ThemeMeta, discover_themes};

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
pub(crate) enum BackgroundEvent {
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
    pub type_ids: Vec<String>,
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
    pub type_ids: Vec<String>,
    pub tls: bool,
    pub path: String,
}

impl AddForm {
    pub fn new(type_ids: Vec<String>) -> Self {
        Self {
            field: AddField::Name,
            name: String::new(),
            domain: String::new(),
            aliases: String::new(),
            port: String::new(),
            upstream_host: String::new(),
            type_index: 0,
            type_ids,
            tls: false,
            path: String::new(),
        }
    }

    pub fn project_type(&self) -> &str {
        self.type_ids
            .get(self.type_index)
            .map(String::as_str)
            .unwrap_or("phoenix")
    }

    pub fn cycle_type_next(&mut self) {
        if self.type_ids.is_empty() {
            return;
        }
        self.type_index = (self.type_index + 1) % self.type_ids.len();
    }

    pub fn cycle_type_prev(&mut self) {
        if self.type_ids.is_empty() {
            return;
        }
        if self.type_index == 0 {
            self.type_index = self.type_ids.len() - 1;
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

    pub fn label_msg(&self) -> Msg {
        match self.field {
            AddField::Name => Msg::LabelName,
            AddField::Domain => Msg::LabelDomain,
            AddField::Aliases => Msg::LabelAliases,
            AddField::Port => Msg::LabelPort,
            AddField::UpstreamHost => Msg::LabelUpstream,
            AddField::Type => Msg::LabelType,
            AddField::Tls => Msg::LabelTls,
            AddField::Path => Msg::LabelDirectory,
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
    pub fn from_project(
        project_index: usize,
        project: &ProjectConfig,
        mut type_ids: Vec<String>,
    ) -> Self {
        let current = project.project_type.as_str();
        if !type_ids.iter().any(|t| t == current) {
            type_ids.insert(0, current.to_string());
        }
        let type_index = type_ids.iter().position(|t| t == current).unwrap_or(0);

        Self {
            project_index,
            field: AddField::Name,
            name: project.name.clone(),
            domain: project.domain.clone(),
            aliases: project.aliases.join(", "),
            port: project.port.to_string(),
            upstream_host: project.upstream_host.clone().unwrap_or_default(),
            type_index,
            type_ids,
            tls: project.tls,
            path: project.path.clone(),
        }
    }

    pub fn project_type(&self) -> &str {
        self.type_ids
            .get(self.type_index)
            .map(String::as_str)
            .unwrap_or("phoenix")
    }

    pub fn cycle_type_next(&mut self) {
        if self.type_ids.is_empty() {
            return;
        }
        self.type_index = (self.type_index + 1) % self.type_ids.len();
    }

    pub fn cycle_type_prev(&mut self) {
        if self.type_ids.is_empty() {
            return;
        }
        if self.type_index == 0 {
            self.type_index = self.type_ids.len() - 1;
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

    pub fn label_msg(&self) -> Msg {
        match self.field {
            AddField::Name => Msg::LabelName,
            AddField::Domain => Msg::LabelDomain,
            AddField::Aliases => Msg::LabelAliases,
            AddField::Port => Msg::LabelPort,
            AddField::UpstreamHost => Msg::LabelUpstream,
            AddField::Type => Msg::LabelType,
            AddField::Tls => Msg::LabelTls,
            AddField::Path => Msg::LabelDirectory,
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
    /// Language picker popup
    pub show_language_popup: bool,
    pub language_selected: usize,
    /// Theme picker popup
    pub show_theme_popup: bool,
    pub theme_selected: usize,
    pub theme_choices: Vec<ThemeMeta>,
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
    /// Taken out of `App` once, into the run loop, via `take_receivers()`.
    app_event_rx: Option<mpsc::Receiver<BackgroundEvent>>,
    pub(crate) manager: Manager,
    /// Taken out of `App` once, into the run loop, via `take_receivers()`.
    event_rx: Option<mpsc::Receiver<ManagerEvent>>,
    pub frameworks: FrameworkRegistry,
    pub lang: Language,
    config_hash: u64,
    config_mtime: Option<std::time::SystemTime>,
    config_len: u64,
    last_config_poll: Instant,
}

/// The two channel receivers, handed from `App` to the main run loop so that
/// `tokio::select!` can await them independently of the `&mut App` the event
/// handlers need. See `App::take_receivers`.
pub(crate) struct AppReceivers {
    pub(crate) manager_rx: mpsc::Receiver<ManagerEvent>,
    pub(crate) background_rx: mpsc::Receiver<BackgroundEvent>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (tx, rx) = mpsc::channel(256);
        let (app_event_tx, app_event_rx) = mpsc::channel(64);

        let projects = config.projects.iter().cloned().map(Project::new).collect();
        crate::tui::ui::init_theme(config.theme.as_ref());
        let frameworks = FrameworkRegistry::load();
        let lang = config.language.unwrap_or_else(Language::from_env);

        let mut app = Self {
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
            show_language_popup: false,
            language_selected: 0,
            show_theme_popup: false,
            theme_selected: 0,
            theme_choices: vec![],
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
            app_event_rx: Some(app_event_rx),
            manager: Manager::new(tx, frameworks.clone()),
            event_rx: Some(rx),
            frameworks,
            lang,
            config_hash: 0,
            config_mtime: None,
            config_len: 0,
            last_config_poll: Instant::now(),
        };
        app.prime_config_watch();
        app
    }

    pub fn tr(&self, msg: Msg) -> &'static str {
        self.lang.tr(msg)
    }

    pub fn trf(&self, msg: Msg, pairs: &[(&str, &str)]) -> String {
        fill(self.lang.tr(msg), pairs)
    }

    pub fn status_label(&self, status: &ProjectStatus) -> &'static str {
        match status {
            ProjectStatus::Stopped => self.tr(Msg::StatusStopped),
            ProjectStatus::Starting => self.tr(Msg::StatusStarting),
            ProjectStatus::Running => self.tr(Msg::StatusRunning),
            ProjectStatus::Failed(_) => self.tr(Msg::StatusFailed),
        }
    }

    pub(crate) fn open_language_picker(&mut self) {
        self.language_selected = Language::ALL
            .iter()
            .position(|l| *l == self.lang)
            .unwrap_or(0);
        self.show_language_popup = true;
    }

    pub(crate) fn select_language_next(&mut self) {
        let n = Language::ALL.len();
        self.language_selected = (self.language_selected + 1) % n;
    }

    pub(crate) fn select_language_prev(&mut self) {
        let n = Language::ALL.len();
        self.language_selected = if self.language_selected == 0 {
            n - 1
        } else {
            self.language_selected - 1
        };
    }

    pub(crate) fn apply_language_selection(&mut self) {
        let Some(lang) = Language::ALL.get(self.language_selected).copied() else {
            self.show_language_popup = false;
            return;
        };
        self.show_language_popup = false;
        if lang == self.lang {
            return;
        }
        self.lang = lang;
        self.config.language = Some(self.lang);
        if let Err(e) = self.save_config() {
            self.status_message = Some(self.trf(Msg::SaveFailed, &[("error", &e.to_string())]));
            return;
        }
        self.status_message =
            Some(self.trf(Msg::LanguageSet, &[("lang", self.lang.native_name())]));
    }

    pub(crate) fn current_theme_id(&self) -> String {
        self.config
            .theme
            .as_ref()
            .and_then(|t| t.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "groknight".into())
    }

    pub(crate) fn open_theme_picker(&mut self) {
        self.theme_choices = discover_themes();
        let current = self.current_theme_id();
        self.theme_selected = self
            .theme_choices
            .iter()
            .position(|t| t.id == current)
            .unwrap_or(0);
        self.show_theme_popup = true;
        self.preview_theme_at_cursor();
    }

    pub(crate) fn select_theme_next(&mut self) {
        let n = self.theme_choices.len();
        if n == 0 {
            return;
        }
        self.theme_selected = (self.theme_selected + 1) % n;
        self.preview_theme_at_cursor();
    }

    pub(crate) fn select_theme_prev(&mut self) {
        let n = self.theme_choices.len();
        if n == 0 {
            return;
        }
        self.theme_selected = if self.theme_selected == 0 {
            n - 1
        } else {
            self.theme_selected - 1
        };
        self.preview_theme_at_cursor();
    }

    fn preview_theme_at_cursor(&mut self) {
        let Some(choice) = self.theme_choices.get(self.theme_selected) else {
            return;
        };
        let mut preview = self
            .config
            .theme
            .clone()
            .unwrap_or_else(ThemeConfig::default);
        preview.name = Some(choice.id.clone());
        crate::tui::ui::init_theme(Some(&preview));
    }

    pub(crate) fn cancel_theme_picker(&mut self) {
        self.show_theme_popup = false;
        crate::tui::ui::init_theme(self.config.theme.as_ref());
        self.status_message = Some(self.tr(Msg::Cancelled).into());
    }

    pub(crate) fn apply_theme_selection(&mut self) {
        let Some(choice) = self.theme_choices.get(self.theme_selected).cloned() else {
            self.cancel_theme_picker();
            return;
        };
        self.show_theme_popup = false;
        if choice.id == self.current_theme_id() {
            crate::tui::ui::init_theme(self.config.theme.as_ref());
            return;
        }
        let previous = self.config.theme.clone();
        let mut theme = previous.clone().unwrap_or_else(ThemeConfig::default);
        theme.name = Some(choice.id.clone());
        self.config.theme = Some(theme);
        crate::tui::ui::init_theme(self.config.theme.as_ref());
        if let Err(e) = self.save_config() {
            self.config.theme = previous;
            crate::tui::ui::init_theme(self.config.theme.as_ref());
            self.status_message = Some(self.trf(Msg::SaveFailed, &[("error", &e.to_string())]));
            return;
        }
        self.status_message = Some(self.trf(Msg::ThemeSet, &[("name", &choice.label)]));
    }

    /// Project indices in display order: running projects first, then stopped,
    /// each group alphabetical by name. This is a view only — `self.projects`
    /// (and therefore `config.toml`) keeps its original order, since active
    /// status changes constantly and should not reorder the saved config.
    pub(crate) fn display_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.projects.len()).collect();
        order.sort_by(|&a, &b| {
            let pa = &self.projects[a];
            let pb = &self.projects[b];
            // running (false) sorts before stopped (true)
            (!pa.is_running()).cmp(&!pb.is_running()).then_with(|| {
                pa.config
                    .name
                    .to_lowercase()
                    .cmp(&pb.config.name.to_lowercase())
            })
        });
        order
    }

    /// Hand the manager + background channel receivers to the run loop. Called
    /// exactly once at startup; panics if called twice.
    pub(crate) fn take_receivers(&mut self) -> AppReceivers {
        AppReceivers {
            manager_rx: self.event_rx.take().expect("event_rx already taken"),
            background_rx: self
                .app_event_rx
                .take()
                .expect("app_event_rx already taken"),
        }
    }

    /// Drain everything currently queued on both channels without awaiting, so a
    /// burst of events (e.g. many log lines) coalesces into a single redraw.
    /// Returns true if any event was processed.
    pub(crate) fn drain_pending(&mut self, rx: &mut AppReceivers) -> bool {
        let mut changed = false;
        while let Ok(event) = rx.manager_rx.try_recv() {
            self.handle_manager_event(event);
            changed = true;
        }
        while let Ok(event) = rx.background_rx.try_recv() {
            self.handle_background_event(event);
            changed = true;
        }
        changed
    }

    /// Periodic housekeeping run on the interval tick: detect adopted-process
    /// exits, kick off background refreshes, and advance the spinner only while
    /// something is animating. Returns true if visible state changed (so the
    /// run loop knows whether a redraw is warranted).
    pub(crate) async fn housekeeping_tick(&mut self) -> bool {
        let mut changed = false;

        // Adopted processes have no exit channel, so poll for their exit here.
        for event in self.manager.poll_exits() {
            self.handle_manager_event(event);
            changed = true;
        }

        if self.poll_config_reload().await {
            changed = true;
        }

        self.schedule_background_refreshes();

        if self.spinner_active() {
            self.spinner_frame = self.spinner_frame.wrapping_add(1);
            changed = true;
        }

        changed
    }

    /// True when any project is Starting or has an active startup phase — the
    /// only states that render an animated spinner (see `ui::draw_project_list`).
    fn spinner_active(&self) -> bool {
        !self.startup_phases.is_empty()
            || self
                .projects
                .iter()
                .any(|p| matches!(p.status, ProjectStatus::Starting))
    }

    /// Dispatch a key event: Ctrl+C always quits; everything else is handled by
    /// `handle_key`.
    pub(crate) async fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit().await;
            return Ok(());
        }
        self.handle_key(key).await
    }

    fn schedule_background_refreshes(&mut self) {
        if self.last_discovery_refresh.elapsed() >= Duration::from_secs(10)
            && !self.discovery_refresh_in_flight
        {
            self.discovery_refresh_in_flight = true;
            self.last_discovery_refresh = Instant::now();
            let cfg = self.config.clone();
            let frameworks = self.frameworks.clone();
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                let result = discover_services(Some(&cfg), Some(&frameworks))
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

    pub(crate) fn handle_background_event(&mut self, event: BackgroundEvent) {
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
                        let err = e.to_string();
                        self.status_message =
                            Some(self.trf(Msg::DiscoveryFailed, &[("error", &err)]));
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
                        let code = code.to_string();
                        self.append_system_log(
                            &project_name,
                            self.trf(Msg::DomainReachable, &[("code", &code)]),
                            false,
                        );
                        self.status_message = Some(self.trf(
                            Msg::DomainReachableStatus,
                            &[("name", &project_name), ("code", &code)],
                        ));
                    }
                    Err(reason) => {
                        self.append_system_log(
                            &project_name,
                            self.trf(Msg::DomainCheckFailed, &[("reason", &reason)]),
                            true,
                        );
                        self.status_message = Some(self.trf(
                            Msg::DomainCheckFailedStatus,
                            &[("name", &project_name), ("reason", &reason)],
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
                StartupPhase::EnsuringCaddy => self.tr(Msg::PhaseCaddy),
                StartupPhase::StartingProcess => self.tr(Msg::PhaseSpawn),
                StartupPhase::VerifyingDomain => self.tr(Msg::PhaseVerify),
            })
    }

    pub fn spinner_glyph(&self) -> &'static str {
        const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        FRAMES[self.spinner_frame % FRAMES.len()]
    }

    pub(crate) fn handle_manager_event(&mut self, event: ManagerEvent) {
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
                let pid_s = pid.to_string();
                self.status_message = Some(if adopted {
                    self.trf(
                        Msg::AdoptedConflict,
                        &[("name", &project_name), ("pid", &pid_s)],
                    )
                } else if keep_starting {
                    self.trf(Msg::StartedVerifying, &[("name", &project_name)])
                } else {
                    self.trf(Msg::Started, &[("name", &project_name)])
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
            self.selected = self.neighbor_in_display_order(1);
            self.log_scroll_offset = 0;
        }
    }

    pub(crate) fn select_prev(&mut self) {
        if !self.projects.is_empty() {
            self.selected = self.neighbor_in_display_order(-1);
            self.log_scroll_offset = 0;
        }
    }

    /// Step `delta` (+1/-1) through the on-screen (display) order and return the
    /// resulting `self.projects` index, wrapping around. Keeps j/k navigation
    /// consistent with the running/stopped grouping shown in the list.
    fn neighbor_in_display_order(&self, delta: isize) -> usize {
        let order = self.display_order();
        let len = order.len();
        if len == 0 {
            return self.selected;
        }
        let pos = order.iter().position(|&i| i == self.selected).unwrap_or(0);
        let next = (pos as isize + delta).rem_euclid(len as isize) as usize;
        order[next]
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
            self.status_message = Some(self.trf(Msg::StepEnsuringCaddy, &[("name", &name)]));
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
                    self.status_message = Some(self.trf(Msg::StepVerifying, &[("name", &name)]));

                    let tx = self.app_event_tx.clone();
                    let attempts = self
                        .frameworks
                        .get(&config.project_type)
                        .map(|s| s.lifecycle.ready_attempts)
                        .unwrap_or(8);
                    tokio::spawn(async move {
                        let result = verify_project_domain_static(&config, attempts).await;
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
                    let err = e.to_string();
                    self.append_system_log(
                        &name,
                        self.trf(Msg::StartFailed, &[("error", &err)]),
                        true,
                    );
                    self.status_message = Some(self.trf(Msg::Error, &[("error", &err)]));
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
            if let Err(e) = caddy::write_and_reload(&projects, caddy_cfg, &self.frameworks).await {
                let msg = self.trf(Msg::CaddyWarning, &[("error", &e.to_string())]);
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
                self.status_message = Some(self.trf(Msg::Stopped, &[("name", &name)]));
            }
            Err(e) => {
                self.status_message = Some(self.trf(Msg::Error, &[("error", &e.to_string())]));
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
            match caddy::write_and_reload(&projects, caddy_cfg, &self.frameworks).await {
                Ok(()) => self.status_message = Some(self.tr(Msg::CaddyReloaded).into()),
                Err(e) => {
                    self.status_message =
                        Some(self.trf(Msg::CaddyError, &[("error", &e.to_string())]))
                }
            }
        } else {
            self.status_message = Some(self.tr(Msg::NoCaddySection).into());
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
            self.status_message =
                Some(self.trf(Msg::DetectedRunning, &[("count", &adopted.to_string())]));
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
                        self.status_message = Some(self.trf(Msg::Autostarting, &[("name", &name)]));
                    }
                    Err(e) => {
                        self.status_message =
                            Some(self.trf(Msg::AutostartError, &[("error", &e.to_string())]));
                    }
                }
            }
        }
    }

    pub(crate) fn confirm_stop_selected(&mut self) {
        if let Some(project) = self.selected_project() {
            if project.is_running() {
                self.confirm_dialog = Some(ConfirmDialog {
                    action: ConfirmAction::StopProject(project.config.name.clone()),
                });
            } else {
                self.status_message =
                    Some(self.trf(Msg::NotRunning, &[("name", &project.config.name)]));
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
                self.status_message =
                    Some(self.trf(Msg::StopFirst, &[("name", &project.config.name)]));
                return;
            }
            self.confirm_dialog = Some(ConfirmDialog {
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
            self.status_message = Some(self.trf(Msg::RemovedUnsaved, &[("error", &e.to_string())]));
            return;
        }

        // Update Caddyfile to remove the project's domain
        self.ensure_caddy().await;

        self.refresh_unmanaged().await;

        self.status_message = Some(self.trf(Msg::Removed, &[("name", name)]));
    }

    pub(crate) async fn finalize_add(&mut self, form: AddForm) {
        let project_type = form
            .project_type()
            .parse::<FrameworkId>()
            .unwrap_or_else(|_| FrameworkId::new("phoenix"));
        let port = match form.port.parse::<u16>() {
            Ok(0) => {
                self.status_message = Some(self.tr(Msg::InvalidPortRange).into());
                return;
            }
            Ok(p) => p,
            Err(_) => {
                self.status_message = Some(self.trf(Msg::InvalidPort, &[("port", &form.port)]));
                return;
            }
        };

        if form.name.trim().is_empty() {
            self.status_message = Some(self.tr(Msg::NameEmpty).into());
            return;
        }
        if form.domain.trim().is_empty() {
            self.status_message = Some(self.tr(Msg::DomainEmpty).into());
            return;
        }
        if !std::path::Path::new(&form.path).is_dir() {
            self.status_message = Some(self.trf(Msg::DirNotFound, &[("path", &form.path)]));
            return;
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                self.status_message = Some(self.trf(Msg::InvalidUpstream, &[("host", &host)]));
                return;
            }
        }
        if self.projects.iter().any(|p| p.config.name == form.name) {
            self.status_message = Some(self.trf(Msg::ProjectExists, &[("name", &form.name)]));
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
            self.status_message = Some(self.trf(Msg::PortInUse, &[("port", &port.to_string())]));
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
            self.status_message = Some(self.trf(Msg::AddedUnsaved, &[("error", &e.to_string())]));
            return;
        }

        // Update Caddyfile with the new project and start/reload Caddy
        self.ensure_caddy().await;
        self.refresh_unmanaged().await;

        self.status_message = Some(self.trf(Msg::Added, &[("name", &form.name)]));
    }

    /// Check that the given hostnames don't collide with other projects or
    /// with each other. `exclude_index`, when set, skips that project (for edits).
    fn hostname_conflict(&self, hosts: &[&str], exclude_index: Option<usize>) -> Option<String> {
        for i in 0..hosts.len() {
            for j in (i + 1)..hosts.len() {
                if hosts[i] == hosts[j] {
                    return Some(self.trf(Msg::DuplicateHost, &[("host", hosts[i])]));
                }
            }
        }
        for (idx, project) in self.projects.iter().enumerate() {
            if Some(idx) == exclude_index {
                continue;
            }
            for existing in project.config.all_hostnames() {
                if hosts.iter().any(|h| *h == existing) {
                    return Some(self.trf(
                        Msg::DomainUsed,
                        &[("host", existing), ("name", &project.config.name)],
                    ));
                }
            }
        }
        None
    }

    pub fn add_form_error(&self, form: &AddForm) -> Option<String> {
        if form.name.trim().is_empty() {
            return Some(self.tr(Msg::NameEmpty).into());
        }
        if self.projects.iter().any(|p| p.config.name == form.name) {
            return Some(self.trf(Msg::ProjectExists, &[("name", &form.name)]));
        }

        if form.domain.trim().is_empty() {
            return Some(self.tr(Msg::DomainEmpty).into());
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
            Err(_) => return Some(self.trf(Msg::InvalidPort, &[("port", &form.port)])),
        };
        if self.projects.iter().any(|p| p.config.port == port) {
            return Some(self.trf(Msg::PortInUse, &[("port", &port.to_string())]));
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                return Some(self.trf(Msg::InvalidUpstream, &[("host", &host)]));
            }
        }

        if form.path.trim().is_empty() {
            return Some(self.tr(Msg::DirEmpty).into());
        }
        if !std::path::Path::new(&form.path).is_dir() {
            return Some(self.trf(Msg::DirNotFound, &[("path", &form.path)]));
        }

        None
    }

    pub(crate) fn start_edit_selected(&mut self) {
        let Some(project) = self.selected_project() else {
            self.status_message = Some(self.tr(Msg::NoProject).into());
            return;
        };

        let running = project.is_running();
        let project_name = project.config.name.clone();
        let project_config = project.config.clone();

        if running {
            self.status_message = Some(self.trf(Msg::StopBeforeEdit, &[("name", &project_name)]));
            return;
        }

        let idx = self.selected;
        self.add_form = None;
        self.edit_form = Some(EditForm::from_project(
            idx,
            &project_config,
            self.frameworks.ids(),
        ));
        self.status_message = Some(self.tr(Msg::EditingProject).into());
    }

    pub(crate) async fn finalize_edit(&mut self, form: EditForm) {
        if form.project_index >= self.projects.len() {
            self.status_message = Some(self.tr(Msg::ProjectGone).into());
            return;
        }

        let project_type = form
            .project_type()
            .parse::<FrameworkId>()
            .unwrap_or_else(|_| FrameworkId::new("phoenix"));
        let port = match form.port.parse::<u16>() {
            Ok(0) => {
                self.status_message = Some(self.tr(Msg::InvalidPortRange).into());
                return;
            }
            Ok(p) => p,
            Err(_) => {
                self.status_message = Some(self.trf(Msg::InvalidPort, &[("port", &form.port)]));
                return;
            }
        };

        if form.name.trim().is_empty() {
            self.status_message = Some(self.tr(Msg::NameEmpty).into());
            return;
        }
        if form.domain.trim().is_empty() {
            self.status_message = Some(self.tr(Msg::DomainEmpty).into());
            return;
        }
        if !std::path::Path::new(&form.path).is_dir() {
            self.status_message = Some(self.trf(Msg::DirNotFound, &[("path", &form.path)]));
            return;
        }

        for (idx, project) in self.projects.iter().enumerate() {
            if idx == form.project_index {
                continue;
            }
            if project.config.name == form.name {
                self.status_message = Some(self.trf(Msg::ProjectExists, &[("name", &form.name)]));
                return;
            }
            if project.config.port == port {
                self.status_message =
                    Some(self.trf(Msg::PortInUse, &[("port", &port.to_string())]));
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
        let spec = self.frameworks.get(&project_type);
        let uses_php = spec.map(|s| s.uses_php()).unwrap_or(false);
        let is_compose = spec.map(|s| s.is_compose()).unwrap_or(false);
        let updated = ProjectConfig {
            name: form.name.clone(),
            domain: form.domain,
            aliases,
            port,
            project_type: project_type.clone(),
            path: form.path,
            php_version: if uses_php {
                existing.php_version.or_else(|| Some("8.3".into()))
            } else {
                None
            },
            public_dir: existing.public_dir,
            command: existing.command,
            compose_file: if is_compose {
                existing.compose_file
            } else {
                None
            },
            service: if is_compose { existing.service } else { None },
            compose_profiles: if is_compose {
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
            self.status_message = Some(self.trf(Msg::UpdatedUnsaved, &[("error", &e.to_string())]));
            return;
        }

        self.ensure_caddy().await;
        self.refresh_unmanaged().await;
        self.status_message = Some(self.trf(Msg::Updated, &[("name", &form.name)]));
    }

    pub fn edit_form_error(&self, form: &EditForm) -> Option<String> {
        if form.name.trim().is_empty() {
            return Some(self.tr(Msg::NameEmpty).into());
        }
        if self
            .projects
            .iter()
            .enumerate()
            .any(|(idx, p)| idx != form.project_index && p.config.name == form.name)
        {
            return Some(self.trf(Msg::ProjectExists, &[("name", &form.name)]));
        }

        if form.domain.trim().is_empty() {
            return Some(self.tr(Msg::DomainEmpty).into());
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
            Err(_) => return Some(self.trf(Msg::InvalidPort, &[("port", &form.port)])),
        };
        if self
            .projects
            .iter()
            .enumerate()
            .any(|(idx, p)| idx != form.project_index && p.config.port == port)
        {
            return Some(self.trf(Msg::PortInUse, &[("port", &port.to_string())]));
        }
        if let Some(host) = parse_upstream_host(&form.upstream_host) {
            if !is_valid_upstream_host(&host) {
                return Some(self.trf(Msg::InvalidUpstream, &[("host", &host)]));
            }
        }

        if form.path.trim().is_empty() {
            return Some(self.tr(Msg::DirEmpty).into());
        }
        if !std::path::Path::new(&form.path).is_dir() {
            return Some(self.trf(Msg::DirNotFound, &[("path", &form.path)]));
        }

        None
    }

    pub async fn refresh_unmanaged(&mut self) {
        match discover_services(Some(&self.config), Some(&self.frameworks)).await {
            Ok(services) => {
                self.unmanaged_all_services = services.into_iter().filter(|s| !s.managed).collect();
                self.apply_unmanaged_filter();
            }
            Err(e) => {
                self.status_message =
                    Some(self.trf(Msg::DiscoveryFailed, &[("error", &e.to_string())]));
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
            .filter(|s| self.unmanaged_show_unknown || !s.stack.is_unknown())
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
            self.tr(Msg::FilterAllStacks).into()
        } else {
            self.tr(Msg::FilterDevStacks).into()
        });
    }

    pub fn toggle_unmanaged_web_filter(&mut self) {
        self.unmanaged_web_only = !self.unmanaged_web_only;
        self.apply_unmanaged_filter();
        self.status_message = Some(if self.unmanaged_web_only {
            self.tr(Msg::FilterWebPorts).into()
        } else {
            self.tr(Msg::FilterAllPortsMsg).into()
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
            self.status_message = Some(self.tr(Msg::UnmanagedHint).into());
        }
    }

    pub async fn import_selected_unmanaged(&mut self) {
        let Some(service) = self.selected_unmanaged().cloned() else {
            self.status_message = Some(self.tr(Msg::UnmanagedNoneSelected).into());
            return;
        };

        if service
            .cwd
            .as_ref()
            .map(|p| std::path::Path::new(p).is_dir())
            != Some(true)
        {
            self.status_message =
                Some(self.trf(Msg::ImportNoCwd, &[("pid", &service.pid.to_string())]));
            return;
        }
        if self.projects.iter().any(|p| p.config.port == service.port) {
            self.status_message =
                Some(self.trf(Msg::PortInUse, &[("port", &service.port.to_string())]));
            return;
        }

        let base_name = self.base_name_for_service(&service);
        let name = self.unique_project_name(&base_name);
        let domain = self.unique_domain_for_name(&name);
        let guessed = service.stack.label();
        let project_type = if self.frameworks.contains(guessed) {
            FrameworkId::new(guessed)
        } else {
            FrameworkId::new("axum")
        };
        let php_version = None;

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
            self.status_message =
                Some(self.trf(Msg::ImportSaveFailed, &[("error", &e.to_string())]));
            return;
        }

        self.ensure_caddy().await;
        self.refresh_unmanaged().await;
        self.status_message = Some(self.trf(
            Msg::Imported,
            &[("name", &name), ("port", &service.port.to_string())],
        ));
    }

    pub async fn ignore_selected_unmanaged(&mut self) {
        let Some(service) = self.selected_unmanaged().cloned() else {
            self.status_message = Some(self.tr(Msg::UnmanagedNoneSelected).into());
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
            self.status_message =
                Some(self.trf(Msg::IgnoreSaveFailed, &[("error", &e.to_string())]));
            return;
        }

        self.refresh_unmanaged().await;
        self.status_message = Some(self.trf(
            Msg::Ignored,
            &[
                ("command", &service.command),
                ("port", &service.port.to_string()),
            ],
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

    fn prime_config_watch(&mut self) {
        let path = config_path();
        if let Ok(meta) = std::fs::metadata(&path) {
            self.config_mtime = meta.modified().ok();
            self.config_len = meta.len();
        }
        if let Ok(bytes) = std::fs::read(&path) {
            self.config_hash = hash_config_bytes(&bytes);
        }
    }

    fn record_applied_config(&mut self, bytes: &[u8]) {
        self.config_hash = hash_config_bytes(bytes);
        if let Ok(meta) = std::fs::metadata(config_path()) {
            self.config_mtime = meta.modified().ok();
            self.config_len = meta.len();
        }
    }

    async fn poll_config_reload(&mut self) -> bool {
        if self.add_form.is_some()
            || self.edit_form.is_some()
            || self.confirm_dialog.is_some()
            || self.show_language_popup
            || self.show_theme_popup
        {
            return false;
        }
        if self.last_config_poll.elapsed() < Duration::from_millis(500) {
            return false;
        }
        self.last_config_poll = Instant::now();

        let path = config_path();
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        let mtime = meta.modified().ok();
        let len = meta.len();
        if mtime == self.config_mtime && len == self.config_len {
            return false;
        }

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                self.status_message =
                    Some(self.trf(Msg::ConfigReloadRead, &[("error", &e.to_string())]));
                return true;
            }
        };
        self.config_mtime = mtime;
        self.config_len = len;
        let hash = hash_config_bytes(&bytes);
        if hash == self.config_hash {
            return false;
        }
        self.config_hash = hash;

        match toml::from_str::<Config>(&String::from_utf8_lossy(&bytes)) {
            Ok(new) => self.apply_external_config(new).await,
            Err(e) => {
                self.status_message =
                    Some(self.trf(Msg::ConfigReloadParse, &[("error", &e.to_string())]));
                true
            }
        }
    }

    /// Merge an externally-edited config into the live TUI. Identity is project name.
    /// Running processes are not started or stopped.
    pub(crate) async fn apply_external_config(&mut self, new: Config) -> bool {
        let selected_name = self.selected_project().map(|p| p.config.name.clone());

        let mut by_name: HashMap<String, Project> = self
            .projects
            .drain(..)
            .map(|p| (p.config.name.clone(), p))
            .collect();

        let mut needs_caddy = new.caddy != self.config.caddy;
        let mut restart_names = Vec::new();
        let mut forgotten = Vec::new();

        let mut next = Vec::with_capacity(new.projects.len());
        for cfg in &new.projects {
            if let Some(mut existing) = by_name.remove(&cfg.name) {
                if existing.is_running() && start_relevant_changed(&existing.config, cfg) {
                    restart_names.push(cfg.name.clone());
                }
                if proxy_relevant_changed(&existing.config, cfg) {
                    needs_caddy = true;
                }
                existing.config = cfg.clone();
                next.push(existing);
            } else {
                next.push(Project::new(cfg.clone()));
                needs_caddy = true;
            }
        }

        for (name, leftover) in by_name {
            if leftover.is_running() || self.manager.is_running(&name) {
                self.manager.forget(&name);
                forgotten.push(name);
            }
            needs_caddy = true;
        }

        self.projects = next;

        if let Some(name) = selected_name {
            if let Some(idx) = self.projects.iter().position(|p| p.config.name == name) {
                self.selected = idx;
            } else {
                self.selected = self.selected.min(self.projects.len().saturating_sub(1));
            }
        } else {
            self.selected = 0;
        }

        let theme_changed = self.config.theme != new.theme;
        let discovery_changed = self.config.discovery != new.discovery
            || self.config.ignored_services != new.ignored_services;
        let tld_changed = self.config.tld != new.tld;
        if let Some(lang) = new.language {
            self.lang = lang;
        }

        self.config.tld = new.tld;
        self.config.caddy = new.caddy;
        self.config.discovery = new.discovery;
        self.config.ignored_services = new.ignored_services;
        self.config.theme = new.theme;
        self.config.language = new.language;
        self.config.projects = self.projects.iter().map(|p| p.config.clone()).collect();

        if theme_changed {
            crate::tui::ui::init_theme(self.config.theme.as_ref());
        }
        if needs_caddy {
            let _ = self.ensure_caddy().await;
        }
        if discovery_changed {
            self.refresh_unmanaged().await;
        }

        let mut parts = vec![self.tr(Msg::ConfigReloaded).to_string()];
        if !restart_names.is_empty() {
            let names = restart_names.join(", ");
            parts.push(self.trf(Msg::ConfigReloadRestart, &[("name", &names)]));
        }
        if tld_changed {
            parts.push(self.tr(Msg::ConfigReloadTld).into());
        }
        if !forgotten.is_empty() {
            let names = forgotten.join(", ");
            parts.push(self.trf(Msg::ConfigReloadForgotten, &[("name", &names)]));
        }
        self.status_message = Some(parts.join(" — "));
        true
    }

    /// Rewrite config.toml from current state.
    /// Uses atomic write (temp file + rename) to prevent corruption on interruption.
    fn save_config(&mut self) -> Result<()> {
        let path = config_path();
        let serialized = Config {
            tld: self.config.tld.clone(),
            projects: self.projects.iter().map(|p| p.config.clone()).collect(),
            caddy: self.config.caddy.clone(),
            discovery: self.config.discovery.clone(),
            ignored_services: self.config.ignored_services.clone(),
            theme: self.config.theme.clone(),
            language: Some(self.lang),
        };
        let mut out = String::from("# zapusk config\n\n");
        out.push_str(&toml::to_string_pretty(&serialized)?);

        // Write to a temp file in the same directory, then rename for atomicity
        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, &out)?;
        std::fs::rename(&tmp_path, &path)?;
        self.record_applied_config(out.as_bytes());
        Ok(())
    }

    pub(crate) async fn quit(&mut self) {
        self.status_message = Some(self.tr(Msg::SoftQuit).into());
        self.should_quit = true;
    }

    pub(crate) async fn force_quit(&mut self) {
        self.status_message = Some(self.tr(Msg::ForceQuit).into());
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
        self.status_message = Some(self.trf(Msg::ForceQuitDone, &[("notes", &notes.join(", "))]));
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

fn start_relevant_changed(old: &ProjectConfig, new: &ProjectConfig) -> bool {
    old.path != new.path
        || old.project_type != new.project_type
        || old.command != new.command
        || old.args != new.args
        || old.env != new.env
        || old.port != new.port
        || old.compose_file != new.compose_file
        || old.service != new.service
        || old.compose_profiles != new.compose_profiles
}

fn proxy_relevant_changed(old: &ProjectConfig, new: &ProjectConfig) -> bool {
    old.domain != new.domain
        || old.aliases != new.aliases
        || old.tls != new.tls
        || old.upstream_host != new.upstream_host
}

async fn verify_project_domain_static(
    config: &ProjectConfig,
    ready_attempts: u32,
) -> Result<u16, String> {
    let scheme = if config.tls { "https" } else { "http" };
    let url = format!("{}://{}", scheme, config.domain);
    let mut last_error = String::from("unreachable");

    // Compose stacks take longer to come up (image pulls, db init) than
    // native processes — give them a much wider verification window.
    let attempts = ready_attempts;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn project(name: &str, port: u16, ptype: &str) -> ProjectConfig {
        ProjectConfig {
            name: name.into(),
            domain: format!("{}.test", name),
            aliases: vec![],
            port,
            project_type: FrameworkId::new(ptype),
            path: "/tmp".into(),
            php_version: None,
            public_dir: None,
            command: None,
            compose_file: None,
            service: None,
            compose_profiles: vec![],
            upstream_host: None,
            args: vec![],
            env: Default::default(),
            autostart: false,
            tls: false,
        }
    }

    fn app_with(projects: Vec<ProjectConfig>) -> App {
        let config = Config {
            tld: "test".into(),
            projects,
            caddy: None,
            discovery: None,
            ignored_services: vec![],
            theme: None,
            language: None,
        };
        App::new(config)
    }

    fn display_names(app: &App) -> Vec<String> {
        app.display_order()
            .into_iter()
            .map(|i| app.projects[i].config.name.clone())
            .collect()
    }

    fn set_running(app: &mut App, name: &str, running: bool) {
        let p = app
            .projects
            .iter_mut()
            .find(|p| p.config.name == name)
            .unwrap();
        p.status = if running {
            ProjectStatus::Running
        } else {
            ProjectStatus::Stopped
        };
    }

    #[test]
    fn display_order_groups_running_then_stopped_alphabetically() {
        let mut app = app_with(vec![
            project("zeta", 1, "phoenix"),
            project("beta", 2, "axum"),
            project("alpha", 3, "phoenix"),
            project("gamma", 4, "axum"),
        ]);
        // running: zeta, gamma — stopped: alpha, beta
        set_running(&mut app, "zeta", true);
        set_running(&mut app, "gamma", true);
        set_running(&mut app, "alpha", false);
        set_running(&mut app, "beta", false);

        // running group first (gamma, zeta), then stopped group (alpha, beta),
        // each alphabetical by name.
        assert_eq!(display_names(&app), vec!["gamma", "zeta", "alpha", "beta"]);
    }

    #[test]
    fn display_order_does_not_reorder_underlying_vec() {
        let mut app = app_with(vec![
            project("zeta", 1, "phoenix"),
            project("beta", 2, "axum"),
        ]);
        set_running(&mut app, "beta", true);

        // config/Vec order is preserved; only the view is reordered.
        let vec_order: Vec<_> = app.projects.iter().map(|p| p.config.name.clone()).collect();
        assert_eq!(vec_order, vec!["zeta", "beta"]);
        assert_eq!(display_names(&app), vec!["beta", "zeta"]);
    }

    #[test]
    fn navigation_follows_display_order() {
        let mut app = app_with(vec![
            project("zeta", 1, "phoenix"),
            project("beta", 2, "axum"),
            project("alpha", 3, "phoenix"),
        ]);
        set_running(&mut app, "zeta", true); // only running one -> top of list

        // Start at the first display row (zeta), then step down twice.
        let zeta = app
            .projects
            .iter()
            .position(|p| p.config.name == "zeta")
            .unwrap();
        app.selected = zeta;
        // display order: zeta (running), then alpha, beta (stopped, alphabetical)
        app.select_next();
        assert_eq!(app.selected_project().unwrap().config.name, "alpha");
        app.select_next();
        assert_eq!(app.selected_project().unwrap().config.name, "beta");
        app.select_next(); // wraps back to top
        assert_eq!(app.selected_project().unwrap().config.name, "zeta");
    }

    #[test]
    fn language_picker_starts_on_current_language() {
        let mut app = app_with(vec![project("alpha", 1, "phoenix")]);
        app.lang = Language::It;
        app.open_language_picker();
        assert!(app.show_language_popup);
        assert_eq!(Language::ALL[app.language_selected], Language::It);
        app.select_language_next();
        assert_eq!(Language::ALL[app.language_selected], Language::Sr);
        app.select_language_prev();
        assert_eq!(Language::ALL[app.language_selected], Language::It);
    }

    #[test]
    fn theme_picker_starts_on_current_theme() {
        let mut app = app_with(vec![project("alpha", 1, "phoenix")]);
        app.config.theme = Some(ThemeConfig {
            name: Some("terminal".into()),
            ..ThemeConfig::default()
        });
        app.open_theme_picker();
        assert!(app.show_theme_popup);
        assert_eq!(app.theme_choices[app.theme_selected].id, "terminal");
        app.select_theme_next();
        assert_ne!(app.theme_choices[app.theme_selected].id, "terminal");
        app.select_theme_prev();
        assert_eq!(app.theme_choices[app.theme_selected].id, "terminal");
    }

    #[test]
    fn theme_picker_preview_does_not_write_config_until_enter() {
        let mut app = app_with(vec![project("alpha", 1, "phoenix")]);
        app.config.theme = Some(ThemeConfig {
            name: Some("terminal".into()),
            ..ThemeConfig::default()
        });
        app.open_theme_picker();
        let before = app.config.theme.clone();
        app.select_theme_next();
        assert_eq!(app.config.theme, before);
        app.cancel_theme_picker();
        assert!(!app.show_theme_popup);
        assert_eq!(app.current_theme_id(), "terminal");
    }

    fn config_from(projects: Vec<ProjectConfig>) -> Config {
        Config {
            tld: "test".into(),
            projects,
            caddy: None,
            discovery: None,
            ignored_services: vec![],
            theme: None,
            language: None,
        }
    }

    #[tokio::test]
    async fn reload_adds_stopped_project_without_autostart() {
        let mut app = app_with(vec![project("alpha", 1, "phoenix")]);
        let mut incoming = project("beta", 2, "axum");
        incoming.autostart = true;
        app.apply_external_config(config_from(vec![project("alpha", 1, "phoenix"), incoming]))
            .await;
        assert_eq!(app.projects.len(), 2);
        let beta = app
            .projects
            .iter()
            .find(|p| p.config.name == "beta")
            .unwrap();
        assert!(!beta.is_running());
        assert_eq!(beta.status, ProjectStatus::Stopped);
    }

    #[tokio::test]
    async fn reload_removes_stopped_project_and_clamps_selection() {
        let mut app = app_with(vec![
            project("alpha", 1, "phoenix"),
            project("beta", 2, "axum"),
        ]);
        app.selected = 1;
        app.apply_external_config(config_from(vec![project("alpha", 1, "phoenix")]))
            .await;
        assert_eq!(app.projects.len(), 1);
        assert_eq!(app.selected_project().unwrap().config.name, "alpha");
    }

    #[tokio::test]
    async fn reload_forgets_running_project_without_keeping_it_tracked() {
        let mut app = app_with(vec![
            project("alpha", 1, "phoenix"),
            project("beta", 2, "axum"),
        ]);
        set_running(&mut app, "beta", true);
        app.manager.track_for_test("beta");
        assert!(app.manager.is_running("beta"));

        app.apply_external_config(config_from(vec![project("alpha", 1, "phoenix")]))
            .await;

        assert_eq!(app.projects.len(), 1);
        assert!(!app.manager.is_running("beta"));
        assert!(
            app.status_message
                .as_deref()
                .unwrap_or("")
                .contains("beta left running")
        );
    }

    #[tokio::test]
    async fn reload_updates_port_on_running_project_and_keeps_status() {
        let mut app = app_with(vec![project("alpha", 4000, "phoenix")]);
        set_running(&mut app, "alpha", true);
        let mut updated = project("alpha", 4001, "phoenix");
        updated.path = "/tmp/moved".into();
        app.apply_external_config(config_from(vec![updated])).await;

        assert_eq!(app.projects[0].config.port, 4001);
        assert_eq!(app.projects[0].status, ProjectStatus::Running);
        let msg = app.status_message.as_deref().unwrap_or("");
        assert!(msg.contains("restart alpha"));
    }

    #[test]
    fn hash_equality_is_stable_for_same_bytes() {
        let a = hash_config_bytes(b"hello");
        let b = hash_config_bytes(b"hello");
        let c = hash_config_bytes(b"world");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
