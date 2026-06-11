use anyhow::{bail, Result};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::core::config::{config_path, CaddyConfig, Config, ProjectConfig, ProjectType};
use crate::core::discovery::{discover_services, ServiceInfo, StackKind};

pub async fn run(json: bool, import: Option<String>) -> Result<()> {
    let loaded = Config::load().ok();
    let mut services = discover_services(loaded.as_ref()).await?;

    if let Some(target) = import {
        let mut config = loaded.unwrap_or_else(default_config);
        import_service(&target, &services, &mut config)?;
        save_config(&config)?;
        println!(
            "Imported service {} into {}",
            target,
            config_path().display()
        );
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&services)?);
        return Ok(());
    }

    if services.is_empty() {
        println!("No listening services found.");
        return Ok(());
    }

    services.sort_by_key(|s| (s.managed, s.port, s.pid));

    println!("Discovered listening services\n");
    println!(
        "{:<6} {:<8} {:<8} {:<18} {:<10} {}",
        "PORT", "PID", "STACK", "COMMAND", "MANAGED", "CWD"
    );
    for s in services {
        let managed = s
            .managed_by
            .map(|m| format!("yes ({})", m))
            .unwrap_or_else(|| "no".into());
        println!(
            "{:<6} {:<8} {:<8} {:<18} {:<10} {}",
            s.port,
            s.pid,
            s.stack.label(),
            truncate(&s.command, 18),
            truncate(&managed, 10),
            s.cwd.unwrap_or_else(|| "-".into())
        );
    }

    println!("\nTip: import one with `zapusk discover --import <port-or-pid>`");
    Ok(())
}

fn import_service(target: &str, services: &[ServiceInfo], config: &mut Config) -> Result<()> {
    let parsed = target
        .parse::<u32>()
        .map_err(|_| anyhow::anyhow!("`{}` is not a valid pid/port", target))?;

    let service = services
        .iter()
        .find(|s| s.pid == parsed || s.port as u32 == parsed)
        .ok_or_else(|| anyhow::anyhow!("No discovered service matches `{}`", target))?;

    if service.managed {
        bail!(
            "Service {}:{} is already managed{}",
            service.pid,
            service.port,
            service
                .managed_by
                .as_ref()
                .map(|m| format!(" by {}", m))
                .unwrap_or_default()
        );
    }

    let cwd = service.cwd.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "Could not determine working directory for pid {}",
            service.pid
        )
    })?;
    if !std::path::Path::new(cwd).is_dir() {
        bail!("Working directory not found: {}", cwd);
    }

    if config.projects.iter().any(|p| p.port == service.port) {
        bail!("Port {} is already used in config", service.port);
    }

    let base = project_base_name(service);
    let used_names: HashSet<&str> = config.projects.iter().map(|p| p.name.as_str()).collect();
    let name = unique_name(&base, &used_names);

    let mut domain = format!("{}.{}", crate::core::slugify(&name), config.tld);
    let mut i = 2;
    while config
        .projects
        .iter()
        .any(|p| p.all_hostnames().any(|h| h == domain))
    {
        domain = format!("{}-{}.{}", crate::core::slugify(&name), i, config.tld);
        i += 1;
    }

    let (project_type, php_version) = match service.stack {
        StackKind::Php => (ProjectType::Symfony, None),
        StackKind::Elixir => (ProjectType::Phoenix, None),
        StackKind::Rust => (ProjectType::Axum, None),
        StackKind::Unknown => (ProjectType::Axum, None),
    };

    let (command, args) = command_override_from_service(service);

    config.projects.push(ProjectConfig {
        name,
        domain,
        aliases: vec![],
        port: service.port,
        project_type,
        path: cwd.clone(),
        php_version,
        public_dir: None,
        command,
        compose_file: None,
        service: None,
        compose_profiles: vec![],
        upstream_host: None,
        args,
        env: Default::default(),
        autostart: false,
        tls: false,
    });

    Ok(())
}

fn project_base_name(service: &ServiceInfo) -> String {
    if let Some(cwd) = &service.cwd {
        if let Some(base) = PathBuf::from(cwd).file_name().and_then(|s| s.to_str()) {
            let slug = crate::core::slugify(base);
            if !slug.is_empty() {
                return slug;
            }
        }
    }

    let from_cmd = crate::core::slugify(&service.command);
    if from_cmd.is_empty() {
        format!("service-{}", service.port)
    } else {
        format!("{}-{}", from_cmd, service.port)
    }
}

fn unique_name(base: &str, existing: &HashSet<&str>) -> String {
    if !existing.contains(base) {
        return base.to_string();
    }

    let mut i = 2;
    loop {
        let candidate = format!("{}-{}", base, i);
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
        i += 1;
    }
}

fn command_override_from_service(service: &ServiceInfo) -> (Option<String>, Vec<String>) {
    if let Some(cmdline) = &service.command_line {
        if let Ok(parts) = shell_words::split(cmdline) {
            if let Some(first) = parts.first() {
                return (Some(first.clone()), parts[1..].to_vec());
            }
        }
    }
    (Some(service.command.clone()), vec![])
}

fn default_config() -> Config {
    let config_file = config_path();
    let caddyfile_path = config_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("Caddyfile");
    Config {
        tld: "test".into(),
        projects: vec![],
        caddy: Some(CaddyConfig {
            config_path: caddyfile_path.display().to_string(),
            caddy_bin: None,
            fpm_socket_template: None,
        }),
        discovery: None,
        ignored_services: vec![],
        theme: None,
    }
}

fn save_config(config: &Config) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = String::from("# zapusk config\n\n");
    out.push_str(&toml::to_string_pretty(config)?);
    std::fs::write(path, out)?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars().take(max.saturating_sub(1)) {
        out.push(c);
    }
    out.push('…');
    out
}
