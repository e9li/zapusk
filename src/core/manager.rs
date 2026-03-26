use anyhow::{bail, Result};
use std::collections::HashMap;
use std::net::TcpListener;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::core::config::ProjectConfig;
use crate::core::project::ProjectStatus;

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
    /// project name -> pid (spawned by us)
    spawned: HashMap<String, u32>,
    /// project name -> pid (adopted external processes)
    adopted: HashMap<String, u32>,
    pub event_tx: mpsc::Sender<ManagerEvent>,
}

impl Manager {
    pub fn new(event_tx: mpsc::Sender<ManagerEvent>) -> Self {
        Self {
            spawned: HashMap::new(),
            adopted: HashMap::new(),
            event_tx,
        }
    }

    pub fn is_running(&self, name: &str) -> bool {
        self.spawned.contains_key(name) || self.adopted.contains_key(name)
    }

    pub async fn start(&mut self, config: &ProjectConfig) -> Result<ProjectStatus> {
        if self.is_running(&config.name) {
            bail!("Project {} is already running", config.name);
        }

        // Check if port is already in use — try to adopt the existing process
        if TcpListener::bind(("127.0.0.1", config.port)).is_err() {
            if let Some(pid) = find_port_pid(config.port).await {
                // Adopt: track this external process
                self.adopted.insert(config.name.clone(), pid);
                let _ = self.event_tx.send(ManagerEvent::ProcessStarted {
                    project_name: config.name.clone(),
                    pid,
                }).await;
                let _ = self.event_tx.send(ManagerEvent::LogLine {
                    project_name: config.name.clone(),
                    line: format!("Adopted existing process (pid {}) on port {}", pid, config.port),
                    is_stderr: false,
                }).await;
                return Ok(ProjectStatus::Running);
            }
            bail!(
                "Port {} is already in use but could not identify the process. Stop it or change the port in config.",
                config.port,
            );
        }

        let (bin, args) = if let Some(ref cmd_override) = config.command {
            if !config.args.is_empty() {
                (cmd_override.clone(), config.args.clone())
            } else {
                let parts = shell_words::split(cmd_override)
                    .map_err(|e| anyhow::anyhow!("Invalid command override for {}: {}", config.name, e))?;
                if parts.is_empty() {
                    bail!("Empty command override for {}", config.name);
                }
                let bin = parts[0].to_string();
                let args = parts[1..].to_vec();
                (bin, args)
            }
        } else {
            if !config.args.is_empty() {
                bail!("Project {} has `args` set without `command`", config.name);
            }
            config.project_type.start_command(config)
        };

        if bin.trim().is_empty() {
                bail!("Empty command override for {}", config.name);
        }

        let mut cmd = Command::new(&bin);
        cmd.args(&args)
            .current_dir(&config.path)
            .env("PORT", config.port.to_string());

        for (key, val) in &config.env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let pid = child.id().unwrap_or(0);
        if pid == 0 {
            bail!("Could not determine PID for {}", config.name);
        }
        let name = config.name.clone();
        let tx = self.event_tx.clone();

        let _ = self
            .event_tx
            .send(ManagerEvent::ProcessStarted {
                project_name: name.clone(),
                pid,
            })
            .await;

        // Spawn a task to stream stdout
        if let Some(stdout) = child.stdout.take() {
            let tx_out = tx.clone();
            let name_out = name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
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

        // Spawn a wait task for process exit detection.
        let tx_exit = tx.clone();
        let name_exit = name.clone();
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => {
                    let _ = tx_exit
                        .send(ManagerEvent::ProcessExited {
                            project_name: name_exit,
                            success: status.success(),
                        })
                        .await;
                }
                Err(_) => {
                    let _ = tx_exit
                        .send(ManagerEvent::ProcessExited {
                            project_name: name_exit,
                            success: false,
                        })
                        .await;
                }
            }
        });

        self.spawned.insert(config.name.clone(), pid);

        Ok(ProjectStatus::Running)
    }

    /// Poll adopted processes for exit status (non-blocking).
    /// Spawned processes are tracked by dedicated wait tasks.
    pub fn poll_exits(&mut self) -> Vec<ManagerEvent> {
        let mut events = vec![];

        // Check adopted processes (via kill(pid, 0) — checks if process exists)
        let mut adopted_exited = vec![];
        for (name, &pid) in &self.adopted {
            if !process_exists(pid) {
                adopted_exited.push(name.clone());
                events.push(ManagerEvent::ProcessExited {
                    project_name: name.clone(),
                    success: true,
                });
            }
        }
        for name in &adopted_exited {
            self.adopted.remove(name);
        }

        events
    }

    pub async fn stop(&mut self, name: &str) -> Result<()> {
        if let Some(pid) = self.spawned.remove(name) {
            send_sigterm(pid)?;
        } else if let Some(pid) = self.adopted.remove(name) {
            // Send SIGTERM to adopted process
            send_sigterm(pid)?;
        } else {
            bail!("Project {} is not running", name);
        }
        Ok(())
    }

    /// Check if a project's port is already in use and adopt the process if so.
    /// Returns Some(pid) if adopted, None if port is free.
    pub async fn detect_running(&mut self, config: &ProjectConfig) -> Option<u32> {
        if TcpListener::bind(("127.0.0.1", config.port)).is_ok() {
            return None; // Port is free, project is not running
        }

        if let Some(pid) = find_port_pid(config.port).await {
            self.adopted.insert(config.name.clone(), pid);
            let _ = self.event_tx.send(ManagerEvent::ProcessStarted {
                project_name: config.name.clone(),
                pid,
            }).await;
            Some(pid)
        } else {
            None
        }
    }

    pub fn mark_exited(&mut self, name: &str) {
        self.spawned.remove(name);
        self.adopted.remove(name);
    }

    pub async fn stop_all(&mut self) {
        // Only stop processes that were explicitly spawned by zapusk.
        for (_, pid) in self.spawned.drain() {
            let _ = send_sigterm(pid);
        }
    }
}

fn send_sigterm(pid: u32) -> Result<()> {
    let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ESRCH) {
            bail!("Failed to stop pid {}: {}", pid, err);
        }
    }
    Ok(())
}

fn process_exists(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        true
    } else {
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }
}

/// Find the PID of the process listening on a given port (best-effort, macOS/Linux)
async fn find_port_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-i", &format!(":{}", port), "-sTCP:LISTEN", "-t"])
        .output()
        .await
        .ok()?;

    let pids = String::from_utf8_lossy(&output.stdout);
    pids.trim().lines().next()?.trim().parse().ok()
}
