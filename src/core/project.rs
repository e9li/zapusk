use chrono::{DateTime, Local};
use std::collections::VecDeque;

use crate::core::config::ProjectConfig;

/// How many log lines to keep in memory per project
pub const LOG_BUFFER_SIZE: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectStatus {
    Stopped,
    Running,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProcessOrigin {
    Managed,
    Adopted,
}

impl ProjectStatus {
    pub fn label(&self) -> &str {
        match self {
            ProjectStatus::Stopped => "stopped",
            ProjectStatus::Running => "running",
            ProjectStatus::Failed(_) => "failed",
        }
    }
}

#[derive(Debug)]
pub struct LogEntry {
    pub timestamp: DateTime<Local>,
    pub line: String,
    pub is_stderr: bool,
}

impl LogEntry {
    pub fn new(line: String, is_stderr: bool) -> Self {
        Self {
            timestamp: Local::now(),
            line,
            is_stderr,
        }
    }
}

/// Runtime state for a project (combines config + live state)
#[derive(Debug)]
pub struct Project {
    pub config: ProjectConfig,
    pub status: ProjectStatus,
    pub logs: VecDeque<LogEntry>,
    /// Process ID if running
    pub pid: Option<u32>,
    /// Whether the running process was spawned by zapusk or adopted
    pub origin: Option<ProcessOrigin>,
    /// When the process was started (for uptime display)
    pub started_at: Option<DateTime<Local>>,
}

impl Project {
    pub fn new(config: ProjectConfig) -> Self {
        Self {
            config,
            status: ProjectStatus::Stopped,
            logs: VecDeque::with_capacity(LOG_BUFFER_SIZE),
            pid: None,
            origin: None,
            started_at: None,
        }
    }

    pub fn add_log(&mut self, line: String, is_stderr: bool) {
        if self.logs.len() >= LOG_BUFFER_SIZE {
            self.logs.pop_front();
        }
        self.logs.push_back(LogEntry::new(line, is_stderr));
    }

    pub fn is_running(&self) -> bool {
        matches!(self.status, ProjectStatus::Running)
    }
}
