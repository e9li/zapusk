use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

use crate::config::{Config, ProjectConfig, ProjectType};

pub struct CheckResult {
    pub ok: bool,
    pub detail: String,
    pub fix_hint: Option<String>,
}

impl CheckResult {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
            fix_hint: None,
        }
    }

    fn fail(detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            fix_hint: Some(hint.into()),
        }
    }
}

pub async fn run() -> Result<()> {
    let config = Config::load().ok();
    let mut issues = 0;

    // System checks
    println!("\nSystem");
    for r in check_system().await {
        print_check(&r);
        if !r.ok {
            issues += 1;
        }
    }

    // PHP checks (only if config has Kirby projects)
    if let Some(ref cfg) = config {
        let php_versions = collect_php_versions(cfg);
        if !php_versions.is_empty() {
            println!("\nPHP");
            for v in &php_versions {
                for r in check_php(v).await {
                    print_check(&r);
                    if !r.ok {
                        issues += 1;
                    }
                }
            }
        }
    }

    // Per-project checks
    if let Some(ref cfg) = config {
        println!("\nProjects");
        for project in &cfg.projects {
            let r = check_project(project).await;
            print_check(&r);
            if !r.ok {
                issues += 1;
            }
        }
    }

    // Caddy config checks
    if let Some(ref cfg) = config {
        if let Some(ref caddy) = cfg.caddy {
            println!("\nCaddy");
            for r in check_caddy_config(caddy).await {
                print_check(&r);
                if !r.ok {
                    issues += 1;
                }
            }
        }
    }

    println!();
    if issues > 0 {
        println!("{} issue(s) found. Run `zapusk init` to fix setup issues.", issues);
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

fn print_check(r: &CheckResult) {
    let icon = if r.ok { "  \u{2713}" } else { "  \u{2717}" };
    println!("{} {}", icon, r.detail);
    if let Some(hint) = &r.fix_hint {
        println!("    \u{2192} {}", hint);
    }
}

// --- System checks ---

pub async fn check_system() -> Vec<CheckResult> {
    let mut results = vec![];

    // caddy
    results.push(check_caddy().await);

    // dnsmasq installed
    results.push(check_dnsmasq_installed().await);

    // dnsmasq running
    results.push(check_dnsmasq_running().await);

    // dnsmasq config
    results.push(check_dnsmasq_config().await);

    // DNS resolution
    results.push(check_dns_resolution().await);

    results
}

pub async fn check_caddy() -> CheckResult {
    match Command::new("caddy").arg("version").output().await {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.trim().split_whitespace().next().unwrap_or("unknown");
            CheckResult::pass(format!("caddy {}", version))
        }
        _ => CheckResult::fail(
            "caddy not found",
            if cfg!(target_os = "macos") {
                "run: brew install caddy"
            } else {
                "install caddy: https://caddyserver.com/docs/install"
            },
        ),
    }
}

pub async fn check_dnsmasq_installed() -> CheckResult {
    match Command::new("which").arg("dnsmasq").output().await {
        Ok(output) if output.status.success() => {
            CheckResult::pass("dnsmasq installed")
        }
        _ => CheckResult::fail(
            "dnsmasq not found",
            if cfg!(target_os = "macos") {
                "run: brew install dnsmasq"
            } else {
                "run: sudo apt install dnsmasq"
            },
        ),
    }
}

pub async fn check_dnsmasq_running() -> CheckResult {
    if cfg!(target_os = "macos") {
        match Command::new("brew")
            .args(["services", "list"])
            .output()
            .await
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout);
                if text.lines().any(|l| l.starts_with("dnsmasq") && l.contains("started")) {
                    CheckResult::pass("dnsmasq running")
                } else {
                    CheckResult::fail("dnsmasq not running", "run: brew services start dnsmasq")
                }
            }
            _ => CheckResult::fail("could not check dnsmasq status", "run: brew services list"),
        }
    } else {
        match Command::new("systemctl")
            .args(["is-active", "dnsmasq"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => CheckResult::pass("dnsmasq running"),
            _ => CheckResult::fail(
                "dnsmasq not running",
                "run: sudo systemctl start dnsmasq",
            ),
        }
    }
}

pub async fn check_dnsmasq_config() -> CheckResult {
    let config_path = if cfg!(target_os = "macos") {
        "/opt/homebrew/etc/dnsmasq.conf"
    } else {
        "/etc/dnsmasq.conf"
    };

    match std::fs::read_to_string(config_path) {
        Ok(content) => {
            if content.contains("address=/.test/127.0.0.1") {
                CheckResult::pass("dnsmasq configured for *.test")
            } else {
                CheckResult::fail(
                    "dnsmasq missing *.test config",
                    format!("add `address=/.test/127.0.0.1` to {}", config_path),
                )
            }
        }
        Err(_) => CheckResult::fail(
            format!("could not read {}", config_path),
            "check dnsmasq configuration file exists",
        ),
    }
}

pub async fn check_dns_resolution() -> CheckResult {
    // Try resolving via dig against localhost DNS
    match Command::new("dig")
        .args(["+short", "zapusk-check.test", "@127.0.0.1"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let result = String::from_utf8_lossy(&output.stdout);
            if result.trim() == "127.0.0.1" {
                CheckResult::pass("*.test resolves to 127.0.0.1")
            } else {
                CheckResult::fail(
                    format!("*.test resolves to {} (expected 127.0.0.1)", result.trim()),
                    "check dnsmasq config and /etc/resolver/test",
                )
            }
        }
        _ => CheckResult::fail(
            "DNS resolution check failed",
            "ensure dnsmasq is running and /etc/resolver/test exists",
        ),
    }
}

// --- PHP checks ---

pub fn collect_php_versions(config: &Config) -> Vec<String> {
    let mut versions: Vec<String> = config
        .projects
        .iter()
        .filter(|p| p.project_type == ProjectType::Kirby)
        .filter_map(|p| p.php_version.clone())
        .collect();
    versions.sort();
    versions.dedup();
    versions
}

pub async fn check_php(version: &str) -> Vec<CheckResult> {
    let mut results = vec![];

    let php_path = format!("/opt/homebrew/opt/php@{}/bin/php", version);
    if Path::new(&php_path).exists() {
        results.push(CheckResult::pass(format!(
            "php@{} found at {}",
            version, php_path
        )));
    } else {
        results.push(CheckResult::fail(
            format!("php@{} not found", version),
            format!("run: brew install php@{}", version),
        ));
    }

    // Check FPM running
    let fpm_sock = format!(
        "/opt/homebrew/var/run/php/php{}-fpm.sock",
        version
    );
    if Path::new(&fpm_sock).exists() {
        results.push(CheckResult::pass(format!("php{}-fpm socket exists", version)));
    } else {
        results.push(CheckResult::fail(
            format!("php{}-fpm socket not found", version),
            format!("run: brew services start php@{}", version),
        ));
    }

    results
}

// --- Per-project checks ---

pub async fn check_project(project: &ProjectConfig) -> CheckResult {
    let path = Path::new(&project.path);
    if !path.exists() {
        return CheckResult::fail(
            format!("{:<14} path not found: {}", project.name, project.path),
            "check the path in config.toml",
        );
    }

    // Check for project-type-specific files
    let (expected_file, binary) = match project.project_type {
        ProjectType::Phoenix => ("mix.exs", "mix"),
        ProjectType::Symfony => ("composer.json", "symfony"),
        ProjectType::Kirby => ("composer.json", "php"),
        ProjectType::Axum => ("Cargo.toml", "cargo"),
    };

    if !path.join(expected_file).exists() {
        return CheckResult::fail(
            format!(
                "{:<14} {} ({}) — missing {}",
                project.name,
                project.path,
                project.project_type.label(),
                expected_file,
            ),
            format!("expected {} in project directory", expected_file),
        );
    }

    // Check binary in PATH
    match Command::new("which").arg(binary).output().await {
        Ok(output) if output.status.success() => {}
        _ => {
            return CheckResult::fail(
                format!(
                    "{:<14} {} ({})",
                    project.name,
                    project.path,
                    project.project_type.label(),
                ),
                format!("`{}` not found in PATH", binary),
            );
        }
    }

    CheckResult::pass(format!(
        "{:<14} {} ({})",
        project.name,
        project.path,
        project.project_type.label(),
    ))
}

// --- Caddy config checks ---

pub async fn check_caddy_config(caddy: &crate::config::CaddyConfig) -> Vec<CheckResult> {
    let mut results = vec![];

    let path = Path::new(&caddy.config_path);
    if path.exists() {
        results.push(CheckResult::pass("Caddyfile present"));
    } else {
        results.push(CheckResult::fail(
            "Caddyfile not found",
            format!("run `zapusk` and press R to generate, or create {}", caddy.config_path),
        ));
        return results;
    }

    let bin = caddy.caddy_bin.as_deref().unwrap_or("caddy");
    match Command::new(bin)
        .args(["validate", "--config", &caddy.config_path])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            results.push(CheckResult::pass("caddy validate passed"));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first_line = stderr.lines().next().unwrap_or("unknown error");
            results.push(CheckResult::fail(
                "caddy validate failed",
                first_line.to_string(),
            ));
        }
        Err(_) => {
            results.push(CheckResult::fail(
                "could not run caddy validate",
                format!("ensure `{}` is in PATH", bin),
            ));
        }
    }

    results
}
