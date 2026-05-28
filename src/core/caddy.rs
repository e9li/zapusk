use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

use crate::core::config::{CaddyConfig, ProjectConfig, ProjectType};

/// Generate a Caddyfile from all project configs.
/// If `project.tls = true`, the site uses `https://...` with `tls internal`.
/// Otherwise the site is served as plain `http://...`.
pub fn generate_caddyfile(projects: &[ProjectConfig], caddy_config: &CaddyConfig) -> String {
    let mut out = String::new();

    // Global options: send Caddy's own logs to a file so they don't pollute the terminal/TUI
    let log_path = Path::new(&caddy_config.config_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join("caddy.log");
    out.push_str(&format!(
        "{{\n\tlog {{\n\t\toutput file {} {{\n\t\t\troll_size 1mb\n\t\t\troll_keep 1\n\t\t}}\n\t}}\n}}\n\n",
        log_path.display()
    ));

    for project in projects {
        let scheme = if project.tls { "https://" } else { "http://" };
        let hosts: Vec<String> = project
            .all_hostnames()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|h| format!("{}{}", scheme, h))
            .collect();

        out.push_str(&format!("{} {{\n", hosts.join(", ")));

        if project.tls {
            out.push_str("\ttls internal\n");
        }

        if project.project_type == ProjectType::Kirby {
            // Kirby: serve static files from Caddy, proxy dynamic requests to PHP
            let doc_root = if project.public_dir.as_deref() == Some("/") {
                project.path.clone()
            } else {
                let sub = project.public_dir.as_deref().unwrap_or("public");
                format!("{}/{}", project.path, sub)
            };

            out.push_str(&format!("\troot * {}\n", doc_root));
            out.push_str("\tencode zstd gzip\n");
            out.push_str("\t@blocked {\n");
            out.push_str("\t\tpath /content/* /site/* /kirby/* /.*\n");
            out.push_str("\t}\n");
            out.push_str("\terror @blocked \"Not found\" 404\n");
            out.push_str("\t@static file\n");
            out.push_str("\thandle @static {\n");
            out.push_str("\t\tfile_server\n");
            out.push_str("\t}\n");
            out.push_str("\thandle {\n");

            if let Some(host) = project
                .upstream_host
                .as_deref()
                .map(str::trim)
                .filter(|h| !h.is_empty())
            {
                out.push_str(&format!(
                    "\t\treverse_proxy {}\n",
                    format_host_port(host, project.port)
                ));
            } else {
                out.push_str(&format!(
                    "\t\treverse_proxy 127.0.0.1:{} [::1]:{} {{\n",
                    project.port, project.port
                ));
                out.push_str("\t\t\tlb_policy first\n");
                out.push_str("\t\t}\n");
            }

            out.push_str("\t}\n");
        } else {
            // Non-Kirby: simple reverse proxy
            if let Some(host) = project
                .upstream_host
                .as_deref()
                .map(str::trim)
                .filter(|h| !h.is_empty())
            {
                out.push_str(&format!(
                    "\treverse_proxy {}\n",
                    format_host_port(host, project.port)
                ));
            } else {
                out.push_str(&format!(
                    "\treverse_proxy 127.0.0.1:{} [::1]:{} {{\n",
                    project.port, project.port
                ));
                out.push_str("\t\tlb_policy first\n");
                out.push_str("\t}\n");
            }
        }

        out.push_str("}\n\n");
    }

    out
}

fn format_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{}]:{}", host, port)
    } else {
        format!("{}:{}", host, port)
    }
}

/// Write the Caddyfile and reload Caddy.
/// Even if content is unchanged, we still issue a reload when Caddy is running
/// to ensure the active process applies this exact config path.
pub async fn write_and_reload(
    projects: &[ProjectConfig],
    caddy_config: &CaddyConfig,
) -> Result<()> {
    let content = generate_caddyfile(projects, caddy_config);
    let path = Path::new(&caddy_config.config_path);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create dir {:?}", parent))?;
    }

    // Check if content actually changed
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let caddy_up = is_running().await;

    if existing != content {
        std::fs::write(path, &content)
            .with_context(|| format!("Could not write Caddyfile to {:?}", path))?;
    }

    // Format first to avoid warnings
    fmt(caddy_config).await;

    if caddy_up {
        reload_via_api(caddy_config).await
    } else {
        start_caddy_run(caddy_config).await
    }
}

/// Format the Caddyfile in-place using `caddy fmt`.
/// Formatting failures are non-fatal but logged to stderr for debugging.
async fn fmt(caddy_config: &CaddyConfig) {
    let bin = caddy_config.caddy_bin.as_deref().unwrap_or("caddy");
    match Command::new(bin)
        .args(["fmt", "--overwrite", &caddy_config.config_path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
    {
        Ok(output) if !output.status.success() => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("caddy fmt warning: {}", stderr.trim());
        }
        Err(e) => {
            eprintln!("caddy fmt warning: {}", e);
        }
        _ => {}
    }
}

/// Check if Caddy's admin API is reachable (i.e. Caddy is running)
async fn is_running() -> bool {
    std::net::TcpStream::connect("127.0.0.1:2019").is_ok()
}

/// Reload Caddy via its admin API — no new process, no terminal output.
async fn reload_via_api(caddy_config: &CaddyConfig) -> Result<()> {
    let bin = caddy_config.caddy_bin.as_deref().unwrap_or("caddy");
    let output = Command::new(bin)
        .args(["reload", "--config", &caddy_config.config_path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .context("Failed to run caddy reload")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = stderr.lines().next().unwrap_or("unknown error");
        anyhow::bail!("caddy reload failed: {}", msg);
    }
    Ok(())
}

/// Start Caddy using `caddy run` (foreground mode) as a managed background process.
/// Unlike `caddy start`, this doesn't re-exec a daemon that inherits the terminal.
async fn start_caddy_run(caddy_config: &CaddyConfig) -> Result<()> {
    let bin = caddy_config.caddy_bin.as_deref().unwrap_or("caddy");

    // Spawn `caddy run` with all stdio suppressed.
    // A background task reaps the child to prevent zombie processes.
    let child = Command::new(bin)
        .args(["run", "--config", &caddy_config.config_path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn caddy run")?;

    tokio::spawn(async move {
        let _ = child.wait_with_output().await;
    });

    // Give Caddy a moment to start its admin API
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if is_running().await {
            return Ok(());
        }
    }

    anyhow::bail!("Caddy started but admin API not reachable after 2s")
}
