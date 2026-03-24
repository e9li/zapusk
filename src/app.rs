use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;
use tokio::sync::mpsc;

use crate::caddy;
use crate::config::Config;
use crate::manager::{Manager, ManagerEvent};
use crate::project::{Project, ProjectStatus};

/// Which pane is focused
#[derive(Debug, Clone, PartialEq)]
pub enum ActivePane {
    ProjectList,
    Logs,
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
    manager: Manager,
    event_rx: mpsc::Receiver<ManagerEvent>,
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
            manager: Manager::new(tx),
            event_rx: rx,
        }
    }

    /// Main event loop tick — call this repeatedly from main
    pub async fn tick(&mut self) -> Result<()> {
        // Drain manager events (non-blocking)
        while let Ok(event) = self.event_rx.try_recv() {
            self.handle_manager_event(event);
        }

        // Handle keyboard input (100ms timeout so we don't busy-loop)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                self.handle_key(key).await?;
            }
        }

        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            // Quit
            KeyCode::Char('q') => self.quit().await,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit().await
            }

            // Navigation
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),

            // Switch panes
            KeyCode::Tab => self.toggle_pane(),

            // Project actions
            KeyCode::Char('s') => self.start_selected().await,
            KeyCode::Char('x') => self.stop_selected().await,
            KeyCode::Char('r') => self.restart_selected().await,

            // Caddy
            KeyCode::Char('R') => self.reload_caddy().await,

            _ => {}
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
                }
                self.status_message = Some(format!("{} started", project_name));
            }
            ManagerEvent::ProcessExited { project_name, success } => {
                if let Some(p) = self.find_project_mut(&project_name) {
                    p.status = if success {
                        ProjectStatus::Stopped
                    } else {
                        ProjectStatus::Failed("exited with error".into())
                    };
                    p.pid = None;
                }
            }
        }
    }

    fn find_project_mut(&mut self, name: &str) -> Option<&mut Project> {
        self.projects.iter_mut().find(|p| p.config.name == name)
    }

    fn selected_project(&self) -> Option<&Project> {
        self.projects.get(self.selected)
    }

    fn select_next(&mut self) {
        if !self.projects.is_empty() {
            self.selected = (self.selected + 1) % self.projects.len();
        }
    }

    fn select_prev(&mut self) {
        if !self.projects.is_empty() {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    fn toggle_pane(&mut self) {
        self.active_pane = match self.active_pane {
            ActivePane::ProjectList => ActivePane::Logs,
            ActivePane::Logs => ActivePane::ProjectList,
        };
    }

    async fn start_selected(&mut self) {
        if let Some(project) = self.projects.get(self.selected) {
            let config = project.config.clone();
            let name = config.name.clone();
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

    async fn stop_selected(&mut self) {
        if let Some(project) = self.projects.get(self.selected) {
            let name = project.config.name.clone();
            match self.manager.stop(&name).await {
                Ok(()) => {
                    if let Some(p) = self.find_project_mut(&name) {
                        p.status = ProjectStatus::Stopped;
                        p.pid = None;
                    }
                    self.status_message = Some(format!("Stopped {}", name));
                }
                Err(e) => {
                    self.status_message = Some(format!("Error: {}", e));
                }
            }
        }
    }

    async fn restart_selected(&mut self) {
        self.stop_selected().await;
        // Small delay so the port is released before restarting
        tokio::time::sleep(Duration::from_millis(500)).await;
        self.start_selected().await;
    }

    async fn reload_caddy(&mut self) {
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

    async fn quit(&mut self) {
        self.status_message = Some("Stopping all projects…".into());
        self.manager.stop_all().await;
        self.should_quit = true;
    }

    /// Logs for the currently selected project
    pub fn selected_logs(&self) -> Vec<&crate::project::LogEntry> {
        self.selected_project()
            .map(|p| p.logs.iter().collect())
            .unwrap_or_default()
    }
}
