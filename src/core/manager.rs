use anyhow::{bail, Result};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use tokio::io::AsyncBufReadExt;
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
    ProcessExited { project_name: String, success: bool },
    /// Process confirmed running
    ProcessStarted {
        project_name: String,
        pid: u32,
        adopted: bool,
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
                let _ = self
                    .event_tx
                    .send(ManagerEvent::ProcessStarted {
                        project_name: config.name.clone(),
                        pid,
                        adopted: true,
                    })
                    .await;
                let _ = self
                    .event_tx
                    .send(ManagerEvent::LogLine {
                        project_name: config.name.clone(),
                        line: format!(
                            "Adopted existing process (pid {}) on port {}",
                            pid, config.port
                        ),
                        is_stderr: false,
                    })
                    .await;
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
                let parts = shell_words::split(cmd_override).map_err(|e| {
                    anyhow::anyhow!("Invalid command override for {}: {}", config.name, e)
                })?;
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

        // Redirect stdout/stderr to log files so the child process is not killed
        // by SIGPIPE when zapusk exits with `q` (soft quit).
        std::fs::create_dir_all(log_dir())?;
        let stdout_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(log_path(&config.name))?;
        let stderr_file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(err_log_path(&config.name))?;

        let mut child = cmd
            .stdout(std::process::Stdio::from(stdout_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .process_group(0)
            .spawn()?;

        let pid = match child.id() {
            Some(id) => id,
            None => bail!("Could not determine PID for {}", config.name),
        };
        let name = config.name.clone();
        let tx = self.event_tx.clone();

        let _ = self
            .event_tx
            .send(ManagerEvent::ProcessStarted {
                project_name: name.clone(),
                pid,
                adopted: false,
            })
            .await;

        write_pid(&config.name, pid);

        // Tail stdout and stderr log files for live log streaming
        spawn_log_tail(log_path(&name), name.clone(), false, tx.clone());
        spawn_log_tail(err_log_path(&name), name.clone(), true, tx.clone());

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
            remove_pid(name);
        } else if let Some(pid) = self.adopted.remove(name) {
            send_sigterm(pid)?;
            remove_pid(name);
        } else {
            bail!("Project {} is not running", name);
        }
        Ok(())
    }

    /// Check if a project's port is already in use and adopt the process if so.
    /// Checks pidfiles first (previously managed by zapusk), then falls back to lsof.
    /// Returns Some(pid) if adopted, None if port is free.
    pub async fn detect_running(&mut self, config: &ProjectConfig) -> Option<u32> {
        // Check pidfile first — process was previously managed by zapusk
        if let Some(pid) = read_pid(&config.name) {
            if process_exists(pid) {
                self.adopted.insert(config.name.clone(), pid);
                let _ = self
                    .event_tx
                    .send(ManagerEvent::ProcessStarted {
                        project_name: config.name.clone(),
                        pid,
                        adopted: true,
                    })
                    .await;
                // Replay recent log history from files
                send_log_history(
                    log_path(&config.name),
                    config.name.clone(),
                    false,
                    &self.event_tx,
                )
                .await;
                send_log_history(
                    err_log_path(&config.name),
                    config.name.clone(),
                    true,
                    &self.event_tx,
                )
                .await;
                // Continue live tailing
                spawn_log_tail(
                    log_path(&config.name),
                    config.name.clone(),
                    false,
                    self.event_tx.clone(),
                );
                spawn_log_tail(
                    err_log_path(&config.name),
                    config.name.clone(),
                    true,
                    self.event_tx.clone(),
                );
                return Some(pid);
            } else {
                remove_pid(&config.name); // stale pidfile
            }
        }

        // Fall back to port-based detection (external processes — no log files)
        if TcpListener::bind(("127.0.0.1", config.port)).is_ok() {
            return None; // Port is free, project is not running
        }

        if let Some(pid) = find_port_pid(config.port).await {
            self.adopted.insert(config.name.clone(), pid);
            let _ = self
                .event_tx
                .send(ManagerEvent::ProcessStarted {
                    project_name: config.name.clone(),
                    pid,
                    adopted: true,
                })
                .await;
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
        for (name, pid) in self.spawned.drain() {
            let _ = send_sigterm(pid);
            remove_pid(&name);
        }
        for (name, pid) in self.adopted.drain() {
            let _ = send_sigterm(pid);
            remove_pid(&name);
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

// ── Log / PID file helpers ──────────────────────────────────────────────────

fn log_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/zapusk/logs")
}

fn pid_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/zapusk/pids")
}

fn log_path(name: &str) -> PathBuf {
    log_dir().join(format!("{}.out", name))
}

fn err_log_path(name: &str) -> PathBuf {
    log_dir().join(format!("{}.err", name))
}

fn pid_path(name: &str) -> PathBuf {
    pid_dir().join(format!("{}.pid", name))
}

fn write_pid(name: &str, pid: u32) {
    let _ = std::fs::create_dir_all(pid_dir());
    let _ = std::fs::write(pid_path(name), pid.to_string());
}

fn remove_pid(name: &str) {
    let _ = std::fs::remove_file(pid_path(name));
}

fn read_pid(name: &str) -> Option<u32> {
    std::fs::read_to_string(pid_path(name))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Spawn a background task that tails a log file and forwards lines as ManagerEvents.
/// Polls for new content every 100ms after reaching EOF (tail -f style).
fn spawn_log_tail(
    path: PathBuf,
    project_name: String,
    is_stderr: bool,
    tx: mpsc::Sender<ManagerEvent>,
) {
    tokio::spawn(async move {
        // Brief delay to let the process write the first bytes
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let file = match tokio::fs::File::open(&path).await {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut reader = tokio::io::BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    // EOF — wait and poll for more content
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Ok(_) => {
                    let l = line
                        .trim_end_matches('\n')
                        .trim_end_matches('\r')
                        .to_string();
                    if !l.is_empty() {
                        let _ = tx
                            .send(ManagerEvent::LogLine {
                                project_name: project_name.clone(),
                                line: l,
                                is_stderr,
                            })
                            .await;
                    }
                }
                Err(_) => break,
            }
        }
    });
}

/// Read the last ~50 lines from a log file and send them as LogLine events.
/// Used when re-adopting a previously-managed process on zapusk restart.
async fn send_log_history(
    path: PathBuf,
    project_name: String,
    is_stderr: bool,
    tx: &mpsc::Sender<ManagerEvent>,
) {
    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(50);
        for line in &lines[start..] {
            if !line.is_empty() {
                let _ = tx
                    .send(ManagerEvent::LogLine {
                        project_name: project_name.clone(),
                        line: line.to_string(),
                        is_stderr,
                    })
                    .await;
            }
        }
    }
}
