use chrono::{DateTime, Local};
use std::collections::VecDeque;

use crate::core::config::ProjectConfig;

/// How many log lines to keep in memory per project
pub const LOG_BUFFER_SIZE: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectStatus {
    Stopped,
    Starting,
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
            ProjectStatus::Starting => "starting",
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
        let clean = strip_ansi(&line);
        if self.logs.len() >= LOG_BUFFER_SIZE {
            self.logs.pop_front();
        }
        self.logs.push_back(LogEntry::new(clean, is_stderr));
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.status,
            ProjectStatus::Running | ProjectStatus::Starting
        )
    }
}

/// Strip ANSI escape sequences (CSI, OSC, single-char escapes) from a string.
fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            if bytes[i] == b'[' {
                // CSI sequence: skip until final byte (0x40-0x7E)
                i += 1;
                while i < bytes.len() && !(0x40..=0x7E).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else if bytes[i] == b']' {
                // OSC sequence: skip until BEL or ST
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            } else {
                // Single-character escape
                i += 1;
            }
        } else {
            result.push(bytes[i] as char);
            i += 1;
        }
    }
    result
}
