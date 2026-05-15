use anyhow::Result;
use serde::Serialize;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use crate::core::config::Config;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StackKind {
    Php,
    Elixir,
    Rust,
    Unknown,
}

impl StackKind {
    pub fn label(&self) -> &'static str {
        match self {
            StackKind::Php => "php",
            StackKind::Elixir => "elixir",
            StackKind::Rust => "rust",
            StackKind::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceInfo {
    pub pid: u32,
    pub port: u16,
    pub command: String,
    pub command_line: Option<String>,
    pub cwd: Option<String>,
    pub stack: StackKind,
    pub managed: bool,
    pub managed_by: Option<String>,
}

pub async fn discover_services(config: Option<&Config>) -> Result<Vec<ServiceInfo>> {
    let Some(output) =
        run_command_timeout("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpcn"], 1800).await
    else {
        return Ok(vec![]);
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut entries = parse_lsof_listeners(&text);

    entries.sort_by_key(|e| (e.port, e.pid));
    entries.dedup_by_key(|e| (e.port, e.pid));

    let mut out = Vec::with_capacity(entries.len());
    for mut entry in entries {
        if is_ignored(config, entry.port, &entry.command) {
            continue;
        }

        entry.command_line = process_command_line(entry.pid).await;
        entry.cwd = process_cwd(entry.pid).await;
        entry.stack = guess_stack(
            &entry.command,
            entry.command_line.as_deref(),
            entry.cwd.as_deref(),
        );

        if let Some(cfg) = config {
            if let Some(project) = cfg.projects.iter().find(|p| {
                p.port == entry.port
                    || entry
                        .cwd
                        .as_deref()
                        .map(|cwd| cwd.starts_with(&p.path))
                        .unwrap_or(false)
            }) {
                entry.managed = true;
                entry.managed_by = Some(project.name.clone());
            }
        }

        out.push(entry);
    }

    out.sort_by_key(|s| (s.managed, s.port, s.pid));
    Ok(out)
}

fn is_ignored(config: Option<&Config>, port: u16, command: &str) -> bool {
    let Some(cfg) = config else {
        return false;
    };

    cfg.ignored_services
        .iter()
        .any(|ignore| ignore.port == port && ignore.command.eq_ignore_ascii_case(command))
}

fn parse_lsof_listeners(text: &str) -> Vec<ServiceInfo> {
    let mut services = vec![];

    let mut pid: Option<u32> = None;
    let mut command: Option<String> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.trim().parse().ok();
            command = None;
            continue;
        }

        if let Some(rest) = line.strip_prefix('c') {
            command = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix('n') {
            if let (Some(pid), Some(command)) = (pid, command.clone()) {
                if let Some(port) = parse_port(rest) {
                    services.push(ServiceInfo {
                        pid,
                        port,
                        command,
                        command_line: None,
                        cwd: None,
                        stack: StackKind::Unknown,
                        managed: false,
                        managed_by: None,
                    });
                }
            }
        }
    }

    services
}

fn parse_port(addr: &str) -> Option<u16> {
    let first = addr.trim().split_whitespace().next()?;
    let listener = first.rsplit("->").next().unwrap_or(first);
    let raw = listener.rsplit(':').next()?;
    raw.parse::<u16>().ok()
}

async fn process_command_line(pid: u32) -> Option<String> {
    let pid_text = pid.to_string();
    let out = run_command_timeout("ps", &["-p", &pid_text, "-o", "command="], 900).await?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

async fn process_cwd(pid: u32) -> Option<String> {
    let pid_text = pid.to_string();
    let out =
        run_command_timeout("lsof", &["-a", "-p", &pid_text, "-d", "cwd", "-Fn"], 900).await?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('n') {
            let cwd = rest.trim();
            if !cwd.is_empty() {
                return Some(cwd.to_string());
            }
        }
    }
    None
}

fn guess_stack(command: &str, command_line: Option<&str>, cwd: Option<&str>) -> StackKind {
    let mut hay = command.to_lowercase();
    if let Some(line) = command_line {
        hay.push(' ');
        hay.push_str(&line.to_lowercase());
    }
    if let Some(dir) = cwd {
        hay.push(' ');
        hay.push_str(&dir.to_lowercase());
    }

    if hay.contains("php") || hay.contains("symfony") {
        StackKind::Php
    } else if hay.contains("beam.smp") || hay.contains("elixir") || hay.contains("mix") {
        StackKind::Elixir
    } else if hay.contains("cargo") || hay.contains("rust") || hay.contains("/target/debug/") {
        StackKind::Rust
    } else {
        StackKind::Unknown
    }
}

async fn run_command_timeout(
    cmd: &str,
    args: &[&str],
    timeout_ms: u64,
) -> Option<std::process::Output> {
    let mut command = Command::new(cmd);
    command.args(args);

    timeout(Duration::from_millis(timeout_ms), command.output())
        .await
        .ok()
        .and_then(|r| r.ok())
}
