use anyhow::Result;
use chrono::Local;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::core::caddy;
use crate::core::config::{config_path, Config, ProjectConfig, ProjectType};
use crate::core::manager::{Manager, ManagerEvent};
use crate::core::project::{Project, ProjectStatus, LogEntry};

/// Which pane is focused
#[derive(Debug, Clone, PartialEq)]
pub enum ActivePane {
    ProjectList,
    Logs,
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
    Port,
    Type,
    Path,
}

pub const TYPE_OPTIONS: &[&str] = &["phoenix", "symfony", "kirby", "axum"];

/// State for the inline add-project form
#[derive(Debug, Clone)]
pub struct AddForm {
    pub field: AddField,
    pub name: String,
    pub domain: String,
    pub port: String,
    pub type_index: usize,
    pub path: String,
}

impl AddForm {
    pub fn new() -> Self {
        Self {
            field: AddField::Name,
            name: String::new(),
            domain: String::new(),
            port: String::new(),
            type_index: 0,
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

    pub fn current_value(&self) -> &str {
        match self.field {
            AddField::Name => &self.name,
            AddField::Domain => &self.domain,
            AddField::Port => &self.port,
            AddField::Type => self.project_type(),
            AddField::Path => &self.path,
        }
    }

    pub fn current_value_mut(&mut self) -> &mut String {
        match self.field {
            AddField::Name => &mut self.name,
            AddField::Domain => &mut self.domain,
            AddField::Port => &mut self.port,
            AddField::Type => unreachable!("Type field uses cycle, not freetext"),
            AddField::Path => &mut self.path,
        }
    }

    pub fn label(&self) -> &str {
        match self.field {
            AddField::Name => "Name",
            AddField::Domain => "Domain",
            AddField::Port => "Port",
            AddField::Type => "Type",
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
                self.field = AddField::Port;
                false
            }
            AddField::Port => {
                self.field = AddField::Type;
                false
            }
            AddField::Type => {
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
    pub(crate) manager: Manager,
    pub(crate) event_rx: mpsc::Receiver<ManagerEvent>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (tx, rx) = mpsc::channel(256);

        let projects = config
            .projects
            .iter()
            .cloned()
            .map(Project::new)
            .collect();

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
            manager: Manager::new(tx),
            event_rx: rx,
        }
    }

    /// Main event loop tick — call this repeatedly from main
    pub async fn tick(&mut self) -> Result<()> {
        // Poll child processes for exit (non-blocking)
        for event in self.manager.poll_exits() {
            self.handle_manager_event(event);
        }

        // Drain manager events (non-blocking)
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_manager_event(event);
        }

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

        Ok(())
    }

    fn handle_manager_event(&mut self, event: ManagerEvent) {
        match event {
            ManagerEvent::LogLine { project_name, line, is_stderr } => {
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.add_log(line, is_stderr);
                }
            }
            ManagerEvent::ProcessStarted { project_name, pid } => {
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.status = ProjectStatus::Running;
                    p.pid = Some(pid);
                    p.started_at = Some(Local::now());
                }
                self.status_message = Some(format!("{} started", project_name));
            }
            ManagerEvent::ProcessExited { project_name, success } => {
                self.manager.mark_exited(&project_name);
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.status = if success {
                        ProjectStatus::Stopped
                    } else {
                        ProjectStatus::Failed("exited with error".into())
                    };
                    p.pid = None;
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

            // Ensure Caddy is running with current config before starting the project
            self.ensure_caddy().await;

            match self.manager.start(&config).await {
                Ok(status) => {
                    if let Some(p) = self.find_project_mut(&name) {
                        p.status = status;
                    }
                    self.status_message = Some(format!("Starting {}…", name));
                }
                Err(e) => {
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
        }
    }

    /// Ensure Caddy is running with the current Caddyfile.
    /// Writes the Caddyfile and starts/reloads Caddy silently.
    async fn ensure_caddy(&mut self) {
        if let Some(caddy_cfg) = &self.config.caddy.clone() {
            let projects: Vec<_> = self.projects.iter().map(|p| p.config.clone()).collect();
            if let Err(e) = caddy::write_and_reload(&projects, caddy_cfg).await {
                self.status_message = Some(format!("Caddy warning: {}", e));
            }
        }
    }

    pub(crate) async fn stop_project(&mut self, name: &str) {
        let name = name.to_string();
        match self.manager.stop(&name).await {
            Ok(()) => {
                if let Some(p) = self.find_project_mut(&name) {
                    p.status = ProjectStatus::Stopped;
                    p.pid = None;
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
                self.status_message = Some(format!("Stop {} first before removing", project.config.name));
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
        if self.selected >= self.projects.len() && !self.projects.is_empty() {
            self.selected = self.projects.len() - 1;
        }

        // Rewrite config file
        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Removed from TUI but could not save config: {}", e));
            return;
        }

        // Update Caddyfile to remove the project's domain
        self.ensure_caddy().await;

        self.status_message = Some(format!("Removed {}", name));
    }

    pub(crate) async fn finalize_add(&mut self, form: AddForm) {
        let project_type = form
            .project_type()
            .parse::<ProjectType>()
            .unwrap_or(ProjectType::Phoenix);
        let port = match form.port.parse::<u16>() {
            Ok(p) => p,
            Err(_) => {
                self.status_message = Some(format!("Invalid port: {}", form.port));
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
        if self.projects.iter().any(|p| p.config.name == form.name) {
            self.status_message = Some(format!("Project '{}' already exists", form.name));
            return;
        }
        if self.projects.iter().any(|p| p.config.domain == form.domain) {
            self.status_message = Some(format!("Domain '{}' is already used", form.domain));
            return;
        }
        if self.projects.iter().any(|p| p.config.port == port) {
            self.status_message = Some(format!("Port {} is already used by another project", port));
            return;
        }

        let config = ProjectConfig {
            name: form.name.clone(),
            domain: form.domain,
            port,
            project_type,
            path: form.path,
            php_version: None,
            command: None,
            args: vec![],
            env: Default::default(),
            autostart: false,
            tls: false,
        };

        self.projects.push(Project::new(config));
        self.selected = self.projects.len() - 1;

        if let Err(e) = self.save_config() {
            self.status_message = Some(format!("Added but could not save config: {}", e));
            return;
        }

        // Update Caddyfile with the new project and start/reload Caddy
        self.ensure_caddy().await;

        self.status_message = Some(format!("Added {}", form.name));
    }

    /// Rewrite config.toml from current state
    fn save_config(&self) -> Result<()> {
        let path = config_path();
        let serialized = Config {
            tld: self.config.tld.clone(),
            projects: self.projects.iter().map(|p| p.config.clone()).collect(),
            caddy: self.config.caddy.clone(),
        };
        let mut out = String::from("# zapusk config\n\n");
        out.push_str(&toml::to_string_pretty(&serialized)?);
        std::fs::write(&path, &out)?;
        Ok(())
    }

    pub(crate) async fn quit(&mut self) {
        self.status_message = Some("Stopping managed projects (adopted processes stay running)…".into());
        self.manager.stop_all().await;
        self.should_quit = true;
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
