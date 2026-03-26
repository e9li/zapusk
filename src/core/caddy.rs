use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

use crate::core::config::{CaddyConfig, ProjectConfig, ProjectType};
use crate::platform;

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

    let fpm_template = caddy_config
        .fpm_socket_template
        .as_deref()
        .unwrap_or_else(|| platform::default_fpm_socket_template());

    for project in projects {
        let domain = if project.tls {
            format!("https://{}", project.domain)
        } else {
            format!("http://{}", project.domain)
        };

        let mut directives = Vec::new();

        if project.tls {
            directives.push("tls internal".to_string());
        }

        match project.project_type {
            ProjectType::Kirby => {
                let php_version = project.php_version.as_deref().unwrap_or("8.3");
                let fpm_sock = fpm_template.replace("{version}", php_version);
                directives.push(format!("root * {}/public", project.path));
                directives.push(format!("php_fastcgi unix/{}", fpm_sock));
                directives.push("file_server".to_string());
            }
            _ => {
                directives.push(format!("reverse_proxy localhost:{}", project.port));
            }
        }

        out.push_str(&format!("{} {{\n", domain));
        for d in &directives {
            out.push_str(&format!("\t{}\n", d));
        }
        out.push_str("}\n\n");
    }

    out
}

/// Write the Caddyfile and reload Caddy.
/// Skips the reload if the Caddyfile content hasn't changed and Caddy is already running.
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

    if existing == content && caddy_up {
        // Nothing changed and Caddy is running — skip
        return Ok(());
    }

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

/// Format the Caddyfile in-place using `caddy fmt`
async fn fmt(caddy_config: &CaddyConfig) {
    let bin = caddy_config.caddy_bin.as_deref().unwrap_or("caddy");
    let _ = Command::new(bin)
        .args(["fmt", "--overwrite", &caddy_config.config_path])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
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
    // The process runs in the background; we don't track the handle —
    // it will be cleaned up when zapusk exits or via `caddy stop`.
    let _child = Command::new(bin)
        .args(["run", "--config", &caddy_config.config_path])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn caddy run")?;

    // Give Caddy a moment to start its admin API
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if is_running().await {
            return Ok(());
        }
    }

    anyhow::bail!("Caddy started but admin API not reachable after 2s")
}
