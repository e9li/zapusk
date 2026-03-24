use anyhow::{bail, Result};
use std::collections::HashMap;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::config::ProjectConfig;
use crate::project::ProjectStatus;

/// Messages sent from background tasks back to the main app
#[derive(Debug)]
pub enum ManagerEvent {
    /// A line of stdout from a project
    LogLine {
        project_name: String,
        line: String,
        is_stderr: bool,
    },
    /// Process exited
    ProcessExited {
        project_name: String,
        success: bool,
    },
    /// Process confirmed running (heuristic: first stdout line received)
    ProcessStarted {
        project_name: String,
        pid: u32,
    },
}

/// Manages spawned child processes
pub struct Manager {
    /// project name → child process
    children: HashMap<String, Child>,
    pub event_tx: mpsc::Sender<ManagerEvent>,
}

impl Manager {
    pub fn new(event_tx: mpsc::Sender<ManagerEvent>) -> Self {
        Self {
            children: HashMap::new(),
            event_tx,
        }
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.children.contains_key(name)
    }

    pub async fn start(&mut self, config: &ProjectConfig) -> Result<ProjectStatus> {
        if self.is_running(&config.name) {
            bail!("Project {} is already running", config.name);
        }

        let (bin, args) = config.project_type.start_command(config);

        let mut child = Command::new(&bin)
            .args(&args)
            .current_dir(&config.path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        let name = config.name.clone();
        let tx = self.event_tx.clone();

        // Spawn a task to stream stdout
        if let Some(stdout) = child.stdout.take() {
            let tx_out = tx.clone();
            let name_out = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut first = true;
                while let Ok(Some(line)) = lines.next_line().await {
                    if first {
                        let _ = tx_out.send(ManagerEvent::ProcessStarted {
                            project_name: name_out.clone(),
                            pid,
                        }).await;
                        first = false;
                    }
                    let _ = tx_out.send(ManagerEvent::LogLine {
                        project_name: name_out.clone(),
                        line,
                        is_stderr: false,
                    }).await;
                }
            });
        }

        // Spawn a task to stream stderr
        if let Some(stderr) = child.stderr.take() {
            let tx_err = tx.clone();
            let name_err = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_err.send(ManagerEvent::LogLine {
                        project_name: name_err.clone(),
                        line,
                        is_stderr: true,
                    }).await;
                }
            });
        }

        // Spawn a task to watch for exit
        let tx_exit = tx.clone();
        let name_exit = name.clone();
        // We can't move child here since we store it — instead watch via pid
        // TODO: improve exit detection (currently relies on stdout/stderr closing)
        // A better approach: use child.wait() in a dedicated task before storing
        // For now, store child and poll status on tick
        drop(tx_exit); // placeholder
        drop(name_exit);

        self.children.insert(config.name.clone(), child);

        Ok(ProjectStatus::Starting)
    }

    pub async fn stop(&mut self, name: &str) -> Result<()> {
        if let Some(mut child) = self.children.remove(name) {
            child.kill().await?;
        } else {
            bail!("Project {} is not running", name);
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) {
        for (_, mut child) in self.children.drain() {
            let _ = child.kill().await;
        }
    }
}
