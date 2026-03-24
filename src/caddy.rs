use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

use crate::config::{CaddyConfig, ProjectConfig, ProjectType};

const DEFAULT_FPM_SOCKET_TEMPLATE: &str = "/opt/homebrew/var/run/php/php{version}-fpm.sock";

/// Generate a Caddyfile from all project configs
pub fn generate_caddyfile(projects: &[ProjectConfig], caddy_config: &CaddyConfig) -> String {
    let mut out = String::new();

    let fpm_template = caddy_config
        .fpm_socket_template
        .as_deref()
        .unwrap_or(DEFAULT_FPM_SOCKET_TEMPLATE);

    for project in projects {
        let domain = if project.tls {
            format!("https://{}", project.domain)
        } else {
            project.domain.clone()
        };

        let tls_line = if project.tls {
            "\n    tls internal"
        } else {
            ""
        };

        match project.project_type {
            ProjectType::Kirby => {
                let php_version = project.php_version.as_deref().unwrap_or("8.3");
                let fpm_sock = fpm_template.replace("{version}", php_version);
                out.push_str(&format!(
                    r#"{domain} {{{tls_line}
    root * {path}/public
    php_fastcgi unix/{sock}
    file_server
}}

"#,
                    domain = domain,
                    tls_line = tls_line,
                    path = project.path,
                    sock = fpm_sock,
                ));
            }
            _ => {
                out.push_str(&format!(
                    r#"{domain} {{{tls_line}
    reverse_proxy localhost:{port}
}}

"#,
                    domain = domain,
                    tls_line = tls_line,
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
    let content = generate_caddyfile(projects, caddy_config);
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
