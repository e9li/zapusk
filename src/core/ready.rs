use crate::core::config::ProjectConfig;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

/// `http://` or `https://` plus the project's primary domain.
pub fn project_url(config: &ProjectConfig) -> String {
    let scheme = if config.tls { "https" } else { "http" };
    format!("{}://{}", scheme, config.domain)
}

/// Probe the project domain with curl until it answers or attempts run out.
/// TLS uses `-k` (Caddy `tls internal`).
pub async fn verify_project_domain(
    config: &ProjectConfig,
    ready_attempts: u32,
) -> Result<u16, String> {
    let url = project_url(config);
    let mut last_error = String::from("unreachable");
    let attempts = ready_attempts;

    for _ in 0..attempts {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-o")
            .arg("/dev/null")
            .arg("-w")
            .arg("%{http_code}")
            .arg("--max-time")
            .arg("2");

        if config.tls {
            cmd.arg("-k");
        }

        let output = timeout(Duration::from_secs(3), cmd.arg(&url).output()).await;

        match output {
            Ok(Ok(out)) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if let Ok(code) = text.parse::<u16>() {
                    if code > 0 {
                        return Ok(code);
                    }
                }
                last_error = format!("unexpected curl output: {}", text);
            }
            Ok(Ok(out)) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                if !stderr.is_empty() {
                    last_error = stderr;
                }
            }
            Ok(Err(e)) => {
                last_error = e.to_string();
            }
            Err(_) => {
                last_error = "curl timed out".into();
            }
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(last_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::framework::FrameworkId;

    fn project(tls: bool) -> ProjectConfig {
        ProjectConfig {
            name: "demo".into(),
            domain: "demo.test".into(),
            aliases: vec![],
            port: 4000,
            project_type: FrameworkId::new("phoenix"),
            path: "/tmp".into(),
            php_version: None,
            public_dir: None,
            command: None,
            compose_file: None,
            service: None,
            compose_profiles: vec![],
            upstream_host: None,
            args: vec![],
            env: Default::default(),
            autostart: false,
            restart: crate::core::config::RestartPolicy::Never,
            tls,
        }
    }

    #[test]
    fn project_url_uses_tls_flag() {
        assert_eq!(project_url(&project(false)), "http://demo.test");
        assert_eq!(project_url(&project(true)), "https://demo.test");
    }
}
