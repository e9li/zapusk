use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

use crate::config::{CaddyConfig, ProjectConfig, ProjectType};

/// Generate a Caddyfile from all project configs
pub fn generate_caddyfile(projects: &[ProjectConfig]) -> String {
    let mut out = String::new();

    for project in projects {
        match project.project_type {
            ProjectType::Kirby => {
                // Kirby: Caddy handles PHP directly via FPM
                let php_version = project.php_version.as_deref().unwrap_or("8.3");
                let fpm_sock = format!(
                    "/opt/homebrew/var/run/php/php{}-fpm.sock",
                    php_version
                );
                out.push_str(&format!(
                    r#"{domain} {{
    root * {path}/public
    php_fastcgi unix/{sock}
    file_server
}}

"#,
                    domain = project.domain,
                    path = project.path,
                    sock = fpm_sock,
                ));
            }
            _ => {
                // Everything else: reverse proxy to local port
                out.push_str(&format!(
                    r#"{domain} {{
    reverse_proxy localhost:{port}
}}

"#,
                    domain = project.domain,
                    port = project.port,
                ));
            }
        }
    }

    out
}

/// Write the Caddyfile and reload Caddy
pub async fn write_and_reload(
    projects: &[ProjectConfig],
    caddy_config: &CaddyConfig,
) -> Result<()> {
    let content = generate_caddyfile(projects);
    let path = Path::new(&caddy_config.config_path);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Could not create dir {:?}", parent))?;
    }

    std::fs::write(path, &content)
        .with_context(|| format!("Could not write Caddyfile to {:?}", path))?;

    reload(caddy_config).await
}

/// Signal Caddy to reload its config
pub async fn reload(caddy_config: &CaddyConfig) -> Result<()> {
    let bin = caddy_config
        .caddy_bin
        .as_deref()
        .unwrap_or("caddy");

    let status = Command::new(bin)
        .args(["reload", "--config", &caddy_config.config_path])
        .status()
        .await
        .context("Failed to run caddy reload")?;

    if !status.success() {
        anyhow::bail!("caddy reload exited with status {}", status);
    }

    Ok(())
}
