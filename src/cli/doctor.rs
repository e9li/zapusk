use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use tokio::process::Command;

use crate::cli::spinner::Spinner;
use crate::core::config::{Config, ProjectConfig};
use crate::core::framework::{FrameworkRegistry, FrameworkSource};
use crate::platform;

pub struct CheckResult {
    pub ok: bool,
    pub detail: String,
    pub fix_hint: Option<String>,
    /// Warning — not a hard failure, just informational
    pub is_warning: bool,
}

impl CheckResult {
    fn pass(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: detail.into(),
            fix_hint: None,
            is_warning: false,
        }
    }

    fn fail(detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            fix_hint: Some(hint.into()),
            is_warning: false,
        }
    }

    fn warn(detail: impl Into<String>, hint: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            fix_hint: Some(hint.into()),
            is_warning: true,
        }
    }
}

pub async fn run() -> Result<()> {
    let config = Config::load().ok();
    let frameworks = FrameworkRegistry::load();
    let tld = config.as_ref().map(|c| c.tld.as_str()).unwrap_or("test");
    let mut issues = 0;
    let mut warnings = 0;

    // Conflicts (always check, even without config)
    {
        let sp = Spinner::start("Checking for conflicts...");
        let conflicts = check_conflicts(tld).await;
        sp.clear().await;
        if !conflicts.is_empty() {
            println!("\nConflicts");
            for r in &conflicts {
                print_check(r);
                warnings += 1;
            }
        }
    }

    // System checks
    {
        let sp = Spinner::start("Checking system dependencies...");
        let results = check_system(tld).await;
        sp.clear().await;
        println!("\nSystem");
        for r in results {
            print_check(&r);
            if !r.ok {
                issues += 1;
            }
        }
    }

    // Framework recipes
    {
        println!("\nFrameworks");
        for r in check_frameworks(&frameworks) {
            print_check(&r);
            if !r.ok {
                if r.is_warning {
                    warnings += 1;
                } else {
                    issues += 1;
                }
            }
        }
    }

    // Color themes
    {
        println!("\nThemes");
        for r in check_themes(config.as_ref()) {
            print_check(&r);
            if !r.ok {
                if r.is_warning {
                    warnings += 1;
                } else {
                    issues += 1;
                }
            }
        }
    }

    // PHP checks (only if a recipe asks for PHP)
    if let Some(ref cfg) = config {
        let php_versions = collect_php_versions(cfg, &frameworks);
        if !php_versions.is_empty() {
            let sp = Spinner::start("Checking PHP installations...");
            let mut results = vec![];
            for v in &php_versions {
                results.extend(check_php(v).await);
            }
            sp.clear().await;
            println!("\nPHP");
            for r in results {
                print_check(&r);
                if !r.ok {
                    issues += 1;
                }
            }
        }
    }

    // Docker checks (only if config has compose projects)
    if let Some(ref cfg) = config {
        if has_compose_projects(cfg, &frameworks) {
            let sp = Spinner::start("Checking Docker...");
            let results = check_docker().await;
            sp.clear().await;
            println!("\nDocker");
            for r in results {
                print_check(&r);
                if !r.ok {
                    if r.is_warning {
                        warnings += 1;
                    } else {
                        issues += 1;
                    }
                }
            }
        }
    }

    // Per-project checks
    if let Some(ref cfg) = config {
        let sp = Spinner::start("Checking projects...");
        let mut results = vec![];
        results.extend(check_project_config_conflicts(cfg));
        for project in &cfg.projects {
            results.push(check_project(project, &frameworks).await);
        }
        sp.clear().await;
        println!("\nProjects");
        for r in results {
            print_check(&r);
            if !r.ok {
                issues += 1;
            }
        }
    }

    // Caddy config checks
    if let Some(ref cfg) = config {
        if let Some(ref caddy) = cfg.caddy {
            let sp = Spinner::start("Validating Caddy config...");
            let results = check_caddy_config(caddy).await;
            sp.clear().await;
            println!("\nCaddy");
            for r in results {
                print_check(&r);
                if !r.ok {
                    issues += 1;
                }
            }
        }
    }

    println!();
    if issues > 0 {
        println!(
            "{} issue(s) found. Run `zapusk init` to fix setup issues.",
            issues
        );
    } else if warnings > 0 {
        println!("All checks passed ({} warning(s) — see above).", warnings);
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

/// Run all doctor checks silently and return whether everything passed.
pub async fn run_quiet() -> Result<bool> {
    let config = Config::load().ok();
    let frameworks = FrameworkRegistry::load();
    let tld = config.as_ref().map(|c| c.tld.as_str()).unwrap_or("test");
    let mut issues = 0;

    for r in check_system(tld).await {
        if !r.ok && !r.is_warning {
            issues += 1;
        }
    }

    for r in check_frameworks(&frameworks) {
        if !r.ok && !r.is_warning {
            issues += 1;
        }
    }

    if let Some(ref cfg) = config {
        for v in &collect_php_versions(cfg, &frameworks) {
            for r in check_php(v).await {
                if !r.ok {
                    issues += 1;
                }
            }
        }
        if has_compose_projects(cfg, &frameworks) {
            for r in check_docker().await {
                if !r.ok && !r.is_warning {
                    issues += 1;
                }
            }
        }
        for project in &cfg.projects {
            if !check_project(project, &frameworks).await.ok {
                issues += 1;
            }
        }
        for r in check_project_config_conflicts(cfg) {
            if !r.ok {
                issues += 1;
            }
        }
        if let Some(ref caddy) = cfg.caddy {
            for r in check_caddy_config(caddy).await {
                if !r.ok {
                    issues += 1;
                }
            }
        }
    }

    Ok(issues == 0)
}

fn check_project_config_conflicts(cfg: &Config) -> Vec<CheckResult> {
    let mut results = vec![];

    let mut by_port: HashMap<u16, Vec<&str>> = HashMap::new();
    let mut by_target: HashMap<String, Vec<&str>> = HashMap::new();

    for p in &cfg.projects {
        by_port.entry(p.port).or_default().push(&p.name);
        let target = format!("{}:{}", normalized_upstream_host(p), p.port);
        by_target.entry(target).or_default().push(&p.name);
    }

    for (port, names) in by_port {
        if names.len() > 1 {
            results.push(CheckResult::warn(
                format!(
                    "port {} is used by multiple projects ({})",
                    port,
                    names.join(", ")
                ),
                "if these are locally started services they will conflict; use distinct ports or explicit upstream_host values",
            ));
        }
    }

    for (target, names) in by_target {
        if names.len() > 1 {
            results.push(CheckResult::fail(
                format!(
                    "duplicate upstream target {} used by ({})",
                    target,
                    names.join(", ")
                ),
                "set different `upstream_host` and/or `port` per project to avoid proxy collisions",
            ));
        }
    }

    results
}

fn normalized_upstream_host(project: &ProjectConfig) -> String {
    if let Some(host) = project
        .upstream_host
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty())
    {
        host.to_ascii_lowercase()
    } else {
        "127.0.0.1-or-::1".into()
    }
}

fn print_check(r: &CheckResult) {
    if r.ok {
        println!("  \u{2713} {}", r.detail);
    } else if r.is_warning {
        println!("  \u{26a0} {}", r.detail);
    } else {
        println!("  \u{2717} {}", r.detail);
    }
    if let Some(hint) = &r.fix_hint {
        println!("    \u{2192} {}", hint);
    }
}

// --- System checks ---

pub async fn check_system(tld: &str) -> Vec<CheckResult> {
    let mut results = vec![];

    results.push(check_caddy().await);
    results.push(check_dnsmasq_installed().await);
    results.push(check_dnsmasq_running().await);
    results.push(check_dnsmasq_config(tld).await);
    results.push(check_dns_resolution(tld).await);

    results
}

pub async fn check_caddy() -> CheckResult {
    match Command::new("caddy").arg("version").output().await {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.split_whitespace().next().unwrap_or("unknown");
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
        Ok(output) if output.status.success() => CheckResult::pass("dnsmasq installed"),
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
                if text
                    .lines()
                    .any(|l| l.starts_with("dnsmasq") && l.contains("started"))
                {
                    return CheckResult::pass("dnsmasq running");
                }
            }
            Err(_) => {}
        }

        // Fallback: if a dnsmasq process is running, treat as healthy even when
        // brew services (current user context) does not report it.
        if Command::new("pgrep")
            .args(["-x", "dnsmasq"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return CheckResult::pass("dnsmasq running (detected via process check)");
        }

        CheckResult::fail(
            "dnsmasq not running",
            "run: brew services start dnsmasq (or `sudo brew services start dnsmasq`)",
        )
    } else {
        match Command::new("systemctl")
            .args(["is-active", "dnsmasq"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => CheckResult::pass("dnsmasq running"),
            _ => CheckResult::fail("dnsmasq not running", "run: sudo systemctl start dnsmasq"),
        }
    }
}

pub async fn check_dnsmasq_config(tld: &str) -> CheckResult {
    let config_path = platform::dnsmasq_config_path();
    let expected = format!("address=/.{}/127.0.0.1", tld);

    match std::fs::read_to_string(config_path) {
        Ok(content) => {
            if content.contains(&expected) {
                CheckResult::pass(format!("dnsmasq configured for *.{}", tld))
            } else {
                CheckResult::fail(
                    format!("dnsmasq missing *.{} config", tld),
                    format!("add `{}` to {}", expected, config_path),
                )
            }
        }
        Err(_) => CheckResult::fail(
            format!("could not read {}", config_path),
            "check dnsmasq configuration file exists",
        ),
    }
}

pub async fn check_dns_resolution(tld: &str) -> CheckResult {
    let test_host = format!("zapusk-check.{}", tld);
    match Command::new("dig")
        .args(["+short", &test_host, "@127.0.0.1"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let result = String::from_utf8_lossy(&output.stdout);
            if result.trim() == "127.0.0.1" {
                CheckResult::pass(format!("*.{} resolves to 127.0.0.1", tld))
            } else {
                CheckResult::fail(
                    format!(
                        "*.{} resolves to {} (expected 127.0.0.1)",
                        tld,
                        result.trim()
                    ),
                    format!("check dnsmasq config and /etc/resolver/{}", tld),
                )
            }
        }
        _ => CheckResult::fail(
            "DNS resolution check failed",
            format!("ensure dnsmasq is running and /etc/resolver/{} exists", tld),
        ),
    }
}

// --- PHP checks ---

pub fn collect_php_versions(config: &Config, frameworks: &FrameworkRegistry) -> Vec<String> {
    let mut versions: Vec<String> = config
        .projects
        .iter()
        .filter(|p| {
            frameworks
                .get(&p.project_type)
                .map(|s| s.hooks.require_php)
                .unwrap_or(false)
        })
        .filter_map(|p| p.php_version.clone())
        .collect();
    versions.sort();
    versions.dedup();
    versions
}

pub async fn check_php(version: &str) -> Vec<CheckResult> {
    let mut results = vec![];

    let php_path = platform::php_install_path(version);
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

    // Check FPM socket
    let fpm_sock = platform::php_fpm_socket_path(version);
    if Path::new(&fpm_sock).exists() {
        results.push(CheckResult::pass(format!(
            "php{}-fpm socket exists",
            version
        )));
    } else {
        results.push(CheckResult::fail(
            format!("php{}-fpm socket not found", version),
            format!("run: brew services start php@{}", version),
        ));
    }

    results
}

// --- Docker checks (compose projects only) ---

pub fn has_compose_projects(config: &Config, frameworks: &FrameworkRegistry) -> bool {
    config
        .projects
        .iter()
        .any(|p| frameworks.is_compose(&p.project_type))
}

fn check_frameworks(frameworks: &FrameworkRegistry) -> Vec<CheckResult> {
    let mut results = vec![];
    for id in frameworks.ids() {
        let source = frameworks
            .source(&id)
            .map(FrameworkSource::label)
            .unwrap_or_else(|| "unknown".into());
        results.push(CheckResult::pass(format!("{:<14} {}", id, source)));
    }
    for warning in frameworks.warnings() {
        results.push(CheckResult::warn(
            warning.clone(),
            "fix or remove the framework TOML file",
        ));
    }
    if results.is_empty() {
        results.push(CheckResult::fail(
            "no framework recipes loaded",
            "this is a bug — builtins should always be present",
        ));
    }
    results
}

fn check_themes(config: Option<&Config>) -> Vec<CheckResult> {
    let themes = crate::tui::theme::discover_themes();
    let mut results = vec![];
    for meta in &themes {
        let label = if meta.label != meta.id {
            format!("{:<14} {:<12} {}", meta.id, meta.source.label(), meta.label)
        } else {
            format!("{:<14} {}", meta.id, meta.source.label())
        };
        results.push(CheckResult::pass(label));
    }
    if let Some(name) = config
        .and_then(|c| c.theme.as_ref())
        .and_then(|t| t.name.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let key = name.to_ascii_lowercase();
        if !themes.iter().any(|m| m.id == key) {
            results.push(CheckResult::warn(
                format!("config theme '{name}' is not loaded"),
                format!(
                    "add ~/.config/zapusk/themes/{key}.toml or use groknight / terminal / nightfox / catppuccin"
                ),
            ));
        }
    }
    if results.is_empty() {
        results.push(CheckResult::fail(
            "no color themes loaded",
            "this is a bug — groknight and terminal should always be present",
        ));
    }
    results
}

pub async fn check_docker() -> Vec<CheckResult> {
    let mut results = vec![];
    results.push(check_docker_daemon().await);
    results.push(check_compose_cli().await);
    results
}

async fn check_docker_daemon() -> CheckResult {
    match Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            CheckResult::pass(format!("docker daemon running (server {})", version))
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            if stderr.contains("permission denied") {
                CheckResult::fail(
                    "docker daemon not accessible (permission denied)",
                    "run: sudo usermod -aG docker $USER — then log out and back in",
                )
            } else {
                CheckResult::fail(
                    "docker daemon not running",
                    if cfg!(target_os = "macos") {
                        "start Docker Desktop, OrbStack, or colima (e.g. `colima start`)"
                    } else {
                        "run: sudo systemctl start docker"
                    },
                )
            }
        }
        Err(_) => {
            // docker CLI missing entirely — check for podman as a hint
            let podman = Command::new("podman")
                .arg("--version")
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false);
            if podman {
                CheckResult::fail(
                    "docker CLI not found (podman detected)",
                    "install the podman-docker compat package, or install Docker",
                )
            } else {
                CheckResult::fail(
                    "docker not found",
                    if cfg!(target_os = "macos") {
                        "install Docker Desktop or OrbStack (brew install --cask orbstack)"
                    } else {
                        "install docker engine: https://docs.docker.com/engine/install/"
                    },
                )
            }
        }
    }
}

async fn check_compose_cli() -> CheckResult {
    match crate::core::docker::compose_cli().await {
        Ok(cli) if cli.is_v1 => CheckResult::warn(
            "docker-compose v1 found (EOL)",
            "install the compose v2 plugin (`docker-compose-plugin`; ships with Docker Desktop)",
        ),
        Ok(_) => CheckResult::pass("docker compose v2 available"),
        Err(_) => CheckResult::fail(
            "docker compose not found",
            "install the compose v2 plugin (ships with Docker Desktop/OrbStack; `docker-compose-plugin` on Linux)",
        ),
    }
}

// --- Per-project checks ---

pub async fn check_project(project: &ProjectConfig, frameworks: &FrameworkRegistry) -> CheckResult {
    let path = Path::new(&project.path);
    if !path.exists() {
        return CheckResult::fail(
            format!("{:<14} path not found: {}", project.name, project.path),
            "check the path in config.toml",
        );
    }

    let Some(spec) = frameworks.get(&project.project_type) else {
        return CheckResult::fail(
            format!(
                "{:<14} {} ({})",
                project.name,
                project.path,
                project.project_type.label(),
            ),
            format!(
                "unknown framework '{}'. Add ~/.config/zapusk/frameworks/{}.toml",
                project.project_type, project.project_type
            ),
        );
    };

    if spec.is_compose() {
        if let Err(e) = project.resolve_compose_file() {
            return CheckResult::fail(
                format!("{:<14} {} ({})", project.name, project.path, spec.label(),),
                e.to_string(),
            );
        }
    } else {
        for marker in &spec.doctor.marker_files {
            if !path.join(marker).exists() {
                return CheckResult::fail(
                    format!(
                        "{:<14} {} ({}) — missing {}",
                        project.name,
                        project.path,
                        spec.label(),
                        marker,
                    ),
                    format!("expected {} in project directory", marker),
                );
            }
        }
    }

    for binary in &spec.doctor.binaries {
        match Command::new("which").arg(binary).output().await {
            Ok(output) if output.status.success() => {}
            _ => {
                return CheckResult::fail(
                    format!("{:<14} {} ({})", project.name, project.path, spec.label(),),
                    format!("`{}` not found in PATH", binary),
                );
            }
        }
    }

    CheckResult::pass(format!(
        "{:<14} {} ({})",
        project.name,
        project.path,
        spec.label(),
    ))
}

// --- Conflict checks ---

/// Detect tools that may conflict with zapusk's dnsmasq + Caddy stack.
/// These are warnings, not hard failures — the tools can coexist if configured carefully.
async fn check_conflicts(tld: &str) -> Vec<CheckResult> {
    let mut results = vec![];

    // Tools that manage local domains or web servers
    let tools: &[(&str, String)] = &[
        (
            "ddev",
            format!(
                "ddev manages its own DNS and router — .{} domains may conflict",
                tld
            ),
        ),
        (
            "herd",
            format!(
                "Laravel Herd manages its own DNS and nginx — port 80/443 and .{} domains may conflict",
                tld
            ),
        ),
        (
            "valet",
            format!(
                "Laravel Valet manages its own dnsmasq and nginx — port 80/443 and .{} domains may conflict",
                tld
            ),
        ),
    ];

    for (binary, message) in tools {
        if is_tool_actively_conflicting(binary).await {
            results.push(CheckResult::warn(
                format!("{} detected as active", binary),
                message.clone(),
            ));
        }
    }

    // Check if something else is already listening on port 80 (Caddy needs it)
    if std::net::TcpListener::bind(("127.0.0.1", 80)).is_err() {
        match find_listening_process(80).await {
            Some((_pid, cmd)) if cmd.to_lowercase().contains("caddy") => {}
            Some((pid, cmd)) => {
                results.push(CheckResult::warn(
                    format!("port 80 is already in use by {} (pid {})", cmd, pid),
                    "stop that process or reconfigure it before using zapusk",
                ));
            }
            None => {
                results.push(CheckResult::warn(
                    "port 80 is already in use",
                    "another web server may be listening — Caddy needs port 80",
                ));
            }
        }
    }

    results
}

async fn is_tool_actively_conflicting(binary: &str) -> bool {
    let installed = Command::new("which")
        .arg(binary)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !installed {
        return false;
    }

    match binary {
        "ddev" => {
            // ddev router is a docker container when ddev is active.
            Command::new("docker")
                .args(["ps", "--format", "{{.Names}}"])
                .output()
                .await
                .ok()
                .map(|o| {
                    o.status.success()
                        && String::from_utf8_lossy(&o.stdout)
                            .lines()
                            .any(|n| n.contains("ddev-router"))
                })
                .unwrap_or(false)
        }
        "herd" => Command::new("pgrep")
            .args(["-f", "[Hh]erd"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false),
        "valet" => Command::new("pgrep")
            .args(["-f", "valet"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false),
        _ => false,
    }
}

async fn find_listening_process(port: u16) -> Option<(u32, String)> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{}", port), "-sTCP:LISTEN", "-Fpc"])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut pid: Option<u32> = None;
    let mut cmd: Option<String> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            if pid.is_none() {
                pid = rest.trim().parse::<u32>().ok();
            }
        } else if let Some(rest) = line.strip_prefix('c') {
            if cmd.is_none() && !rest.trim().is_empty() {
                cmd = Some(rest.trim().to_string());
            }
        }

        if pid.is_some() && cmd.is_some() {
            break;
        }
    }

    Some((pid?, cmd?))
}

// --- Caddy config checks ---

pub async fn check_caddy_config(caddy: &crate::core::config::CaddyConfig) -> Vec<CheckResult> {
    let mut results = vec![];

    let path = Path::new(&caddy.config_path);
    if path.exists() {
        results.push(CheckResult::pass("Caddyfile present"));
    } else {
        results.push(CheckResult::fail(
            "Caddyfile not found",
            format!(
                "run `zapusk` and press R to generate, or create {}",
                caddy.config_path
            ),
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
