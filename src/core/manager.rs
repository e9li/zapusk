use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::core::config::{ProjectConfig, ProjectType};
use crate::core::docker::{self, StackRef};
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

/// How a tracked process must be stopped.
#[derive(Debug, Clone)]
enum ProcKind {
    /// Plain child process — signal the PID / process group.
    Native,
    /// Compose CLI process (`up` or `logs -f`) — the containers belong to the
    /// docker daemon, so stopping means `docker compose stop`, never SIGKILL.
    Compose(StackRef),
}

#[derive(Debug, Clone)]
struct Tracked {
    pid: u32,
    kind: ProcKind,
}

fn proc_kind(config: &ProjectConfig) -> ProcKind {
    if config.project_type == ProjectType::Compose {
        ProcKind::Compose(StackRef::from_config(config))
    } else {
        ProcKind::Native
    }
}

/// Manages spawned child processes
pub struct Manager {
    /// project name -> process (spawned by us)
    spawned: HashMap<String, Tracked>,
    /// project name -> process (adopted external processes)
    adopted: HashMap<String, Tracked>,
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
            if config.project_type == ProjectType::Compose {
                // lsof on a docker-published port returns docker-proxy (Linux)
                // or Docker Desktop's VM process (macOS) — never adopt or
                // signal those. Check the compose stack itself instead.
                if docker::ps_running(config).await {
                    self.adopt_compose_stack(config).await?;
                    return Ok(ProjectStatus::Running);
                }
                bail!(
                    "Port {} is already in use by another process. Stop it or change the port in config.",
                    config.port,
                );
            }
            if let Some(pid) = find_port_pid(config.port).await {
                // Adopt: track this external process
                self.adopted.insert(
                    config.name.clone(),
                    Tracked {
                        pid,
                        kind: ProcKind::Native,
                    },
                );
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

        let (bin, args, notes) = if config.project_type == ProjectType::Compose {
            // Foreground `up`: the compose CLI becomes the tracked child, so
            // log tailing, pidfiles, and exit detection work unchanged.
            docker::up_command(config).await?
        } else if let Some(ref cmd_override) = config.command {
            if !config.args.is_empty() {
                (cmd_override.clone(), config.args.clone(), vec![])
            } else {
                let parts = shell_words::split(cmd_override).map_err(|e| {
                    anyhow::anyhow!("Invalid command override for {}: {}", config.name, e)
                })?;
                if parts.is_empty() {
                    bail!("Empty command override for {}", config.name);
                }
                let bin = parts[0].to_string();
                let args = parts[1..].to_vec();
                (bin, args, vec![])
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

        // Framework-aware reverse proxy env vars
        match config.project_type {
            ProjectType::Symfony => {
                cmd.env("TRUSTED_PROXIES", "127.0.0.1,::1");
            }
            ProjectType::Phoenix => {
                cmd.env("PHX_HOST", &config.domain);
                cmd.env("PHX_SERVER", "true");
            }
            _ => {}
        }

        // Redirect stdout/stderr to log files so the child process is not killed
        // by SIGPIPE when zapusk exits with `q` (soft quit).
        let log_dir = log_dir();
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("Could not create log directory {:?}", log_dir))?;
        let stdout_file = open_truncated(&log_path(&config.name))?;
        let stderr_file = open_truncated(&err_log_path(&config.name))?;

        // Log diagnostic notes (e.g. PHP fallback warnings) before attempting spawn
        for note in &notes {
            let _ = self
                .event_tx
                .send(ManagerEvent::LogLine {
                    project_name: config.name.clone(),
                    line: format!("[zapusk] {}", note),
                    is_stderr: true,
                })
                .await;
        }

        // Log the exact command that will be run
        let _ = self
            .event_tx
            .send(ManagerEvent::LogLine {
                project_name: config.name.clone(),
                line: format!("[zapusk] command: {} {}", bin, args.join(" ")),
                is_stderr: false,
            })
            .await;

        let spawn_result = cmd
            .stdout(std::process::Stdio::from(stdout_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .process_group(0)
            .spawn()
            .with_context(|| format!("could not run '{}': not found or not executable", bin));

        let mut child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                let _ = self
                    .event_tx
                    .send(ManagerEvent::LogLine {
                        project_name: config.name.clone(),
                        line: format!("[zapusk] start failed: {}", e),
                        is_stderr: true,
                    })
                    .await;
                return Err(e);
            }
        };

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

        self.spawned.insert(
            config.name.clone(),
            Tracked {
                pid,
                kind: proc_kind(config),
            },
        );

        Ok(ProjectStatus::Running)
    }

    /// Re-attach to a compose stack that is already running but has no live
    /// pidfile (started externally with `docker compose up -d`, or left over
    /// after zapusk was killed). Spawns `docker compose logs -f` as the
    /// tracked process so the existing log-tail and exit machinery applies:
    /// `logs -f` exits when the stack stops.
    async fn adopt_compose_stack(&mut self, config: &ProjectConfig) -> Result<u32> {
        let (bin, args) = docker::logs_follow_command(config).await?;

        let log_dir = log_dir();
        std::fs::create_dir_all(&log_dir)
            .with_context(|| format!("Could not create log directory {:?}", log_dir))?;
        let stdout_file = open_truncated(&log_path(&config.name))?;
        let stderr_file = open_truncated(&err_log_path(&config.name))?;

        let mut child = Command::new(&bin)
            .args(&args)
            .current_dir(&config.path)
            .stdout(std::process::Stdio::from(stdout_file))
            .stderr(std::process::Stdio::from(stderr_file))
            .process_group(0)
            .spawn()
            .with_context(|| format!("could not run '{}': not found or not executable", bin))?;

        let pid = match child.id() {
            Some(id) => id,
            None => bail!("Could not determine PID for {}", config.name),
        };
        write_pid(&config.name, pid);

        let name = config.name.clone();
        let tx = self.event_tx.clone();
        spawn_log_tail(log_path(&name), name.clone(), false, tx.clone());
        spawn_log_tail(err_log_path(&name), name.clone(), true, tx.clone());

        let tx_exit = tx.clone();
        let name_exit = name.clone();
        tokio::spawn(async move {
            let success = matches!(child.wait().await, Ok(status) if status.success());
            let _ = tx_exit
                .send(ManagerEvent::ProcessExited {
                    project_name: name_exit,
                    success,
                })
                .await;
        });

        self.adopted.insert(
            config.name.clone(),
            Tracked {
                pid,
                kind: ProcKind::Compose(StackRef::from_config(config)),
            },
        );
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
                line: "Adopted running compose stack (logs attached)".into(),
                is_stderr: false,
            })
            .await;

        Ok(pid)
    }

    /// Poll adopted processes for exit status (non-blocking).
    /// Spawned processes are tracked by dedicated wait tasks.
    pub fn poll_exits(&mut self) -> Vec<ManagerEvent> {
        let mut events = vec![];

        // Check adopted processes (via kill(pid, 0) — checks if process exists)
        let mut adopted_exited = vec![];
        for (name, tracked) in &self.adopted {
            if !process_exists(tracked.pid) {
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
        if let Some(t) = self.spawned.remove(name) {
            match &t.kind {
                ProcKind::Native => stop_process(t.pid, true).await?, // process group
                ProcKind::Compose(stack) => stop_compose(t.pid, stack, true).await?,
            }
            remove_pid(name);
        } else if let Some(t) = self.adopted.remove(name) {
            match &t.kind {
                ProcKind::Native => stop_process(t.pid, false).await?, // single PID
                ProcKind::Compose(stack) => stop_compose(t.pid, stack, false).await?,
            }
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
                self.adopted.insert(
                    config.name.clone(),
                    Tracked {
                        pid,
                        kind: proc_kind(config),
                    },
                );
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

        // Compose stacks: detect via the compose CLI, not lsof — the PID on a
        // docker-published port is docker-proxy / Docker Desktop's VM process.
        if config.project_type == ProjectType::Compose {
            if docker::ps_running(config).await {
                return self.adopt_compose_stack(config).await.ok();
            }
            return None;
        }

        // Fall back to port-based detection (external processes — no log files)
        if TcpListener::bind(("127.0.0.1", config.port)).is_ok() {
            return None; // Port is free, project is not running
        }

        if let Some(pid) = find_port_pid(config.port).await {
            self.adopted.insert(
                config.name.clone(),
                Tracked {
                    pid,
                    kind: ProcKind::Native,
                },
            );
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
        for (name, t) in self.spawned.drain() {
            match &t.kind {
                ProcKind::Native => {
                    // Process group kill — no timeout on force quit
                    let target = -(t.pid as i32);
                    unsafe { libc::kill(target, libc::SIGTERM) };
                }
                ProcKind::Compose(stack) => {
                    // Stop the containers via compose, then let the foreground
                    // `up` process wind down (SIGTERM nudge, never SIGKILL).
                    let _ = docker::stop_stack(stack, 10).await;
                    unsafe { libc::kill(-(t.pid as i32), libc::SIGTERM) };
                }
            }
            remove_pid(&name);
        }
        for (name, t) in self.adopted.drain() {
            if let ProcKind::Compose(stack) = &t.kind {
                let _ = docker::stop_stack(stack, 10).await;
            }
            unsafe { libc::kill(t.pid as i32, libc::SIGTERM) };
            remove_pid(&name);
        }
    }
}

/// Stop a compose stack gracefully, then make sure the tracked compose CLI
/// process (`up` or `logs -f`) is gone. Never SIGKILLs the group: the
/// containers belong to the docker daemon and would be orphaned mid-shutdown.
async fn stop_compose(pid: u32, stack: &StackRef, kill_group: bool) -> Result<()> {
    let stop_result = docker::stop_stack(stack, 10).await;

    // The compose CLI exits on its own once the containers stop.
    for _ in 0..50 {
        if !process_exists(pid) {
            return stop_result;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Still alive (e.g. compose stop failed because docker is gone) — SIGTERM
    // triggers the compose CLI's own graceful shutdown as a fallback.
    let target = if kill_group { -(pid as i32) } else { pid as i32 };
    unsafe { libc::kill(target, libc::SIGTERM) };
    stop_result
}

/// Stop a process with SIGTERM, wait up to 3s, then escalate to SIGKILL.
/// When `kill_group` is true, signals are sent to the process group (negative PID).
async fn stop_process(pid: u32, kill_group: bool) -> Result<()> {
    let target = if kill_group { -(pid as i32) } else { pid as i32 };

    let rc = unsafe { libc::kill(target, libc::SIGTERM) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(()); // already dead
        }
        bail!("Failed to stop pid {}: {}", pid, err);
    }

    // Wait up to 3 seconds for process to exit
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if !process_exists(pid) {
            return Ok(());
        }
    }

    // Escalate to SIGKILL
    unsafe { libc::kill(target, libc::SIGKILL) };
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

fn open_truncated(path: &PathBuf) -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .with_context(|| format!("Could not open log file {:?}", path))
}

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

