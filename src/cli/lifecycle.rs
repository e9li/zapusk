use anyhow::Result;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::core::caddy;
use crate::core::config::{Config, lookup_project};
use crate::core::framework::FrameworkRegistry;
use crate::core::manager::Manager;
use crate::core::ready::{project_url, verify_project_domain};
use crate::platform;

pub async fn start(name: &str, no_wait: bool) -> Result<()> {
    let config = Config::load()?;
    let frameworks = FrameworkRegistry::load();
    let project = lookup_project(&config, name)?.clone();

    let (tx, _rx) = mpsc::channel(64);
    let mut manager = Manager::new(tx, frameworks.clone());

    if let Some(pid) = manager.detect_running(&project).await {
        println!("already running {} (pid {pid})", project.name);
        return Ok(());
    }

    ensure_caddy(&config, &frameworks).await;

    manager.start(&project).await?;
    let pid = manager.tracked_pid(&project.name);
    let url = project_url(&project);
    match pid {
        Some(pid) => println!("started {}  {url}  pid {pid}", project.name),
        None => println!("started {}  {url}", project.name),
    }

    if !no_wait {
        wait_ready(&project, &frameworks).await?;
    }
    Ok(())
}

pub async fn stop(name: &str) -> Result<()> {
    let config = Config::load()?;
    let frameworks = FrameworkRegistry::load();
    let project = lookup_project(&config, name)?.clone();

    let (tx, _rx) = mpsc::channel(64);
    let mut manager = Manager::new(tx, frameworks);

    if manager.detect_running(&project).await.is_none() {
        anyhow::bail!("{} is not running", project.name);
    }

    manager.stop(&project.name).await?;
    println!("stopped {}", project.name);
    Ok(())
}

pub async fn restart(name: &str, no_wait: bool) -> Result<()> {
    let config = Config::load()?;
    let frameworks = FrameworkRegistry::load();
    let project = lookup_project(&config, name)?.clone();

    let (tx, _rx) = mpsc::channel(64);
    let mut manager = Manager::new(tx, frameworks.clone());

    if manager.detect_running(&project).await.is_some() {
        manager.stop(&project.name).await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    ensure_caddy(&config, &frameworks).await;
    manager.start(&project).await?;
    let pid = manager.tracked_pid(&project.name);
    let url = project_url(&project);
    match pid {
        Some(pid) => println!("restarted {}  {url}  pid {pid}", project.name),
        None => println!("restarted {}  {url}", project.name),
    }

    if !no_wait {
        wait_ready(&project, &frameworks).await?;
    }
    Ok(())
}

pub async fn status(name: Option<&str>, json: bool) -> Result<()> {
    let config = Config::load()?;
    let frameworks = FrameworkRegistry::load();

    let projects: Vec<_> = if let Some(name) = name {
        vec![lookup_project(&config, name)?]
    } else {
        config.projects.iter().collect()
    };

    let mut rows = Vec::new();
    for project in projects {
        let pid = Manager::probe_running(project, &frameworks).await;
        rows.push(StatusRow {
            name: project.name.clone(),
            status: if pid.is_some() { "running" } else { "stopped" }.into(),
            pid: pid.filter(|p| *p != 0),
            project_type: project.project_type.label().to_string(),
            port: project.port,
            domain: project.domain.clone(),
            tls: project.tls,
        });
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!("no projects in config");
        return Ok(());
    }

    print!("{}", format_status_table(&rows));
    Ok(())
}

pub fn open(name: &str) -> Result<()> {
    let config = Config::load()?;
    let project = lookup_project(&config, name)?;
    let url = project_url(project);
    platform::open_url(&url)?;
    println!("opened {url}");
    Ok(())
}

fn format_status_table(rows: &[StatusRow]) -> String {
    let pids: Vec<String> = rows
        .iter()
        .map(|r| r.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into()))
        .collect();
    let ports: Vec<String> = rows.iter().map(|r| r.port.to_string()).collect();

    let name_w = col_width("NAME", rows.iter().map(|r| r.name.as_str()));
    let status_w = col_width("STATUS", rows.iter().map(|r| r.status.as_str()));
    let pid_w = col_width("PID", pids.iter().map(|s| s.as_str()));
    let type_w = col_width("TYPE", rows.iter().map(|r| r.project_type.as_str()));
    let port_w = col_width("PORT", ports.iter().map(|s| s.as_str()));

    let mut out = String::new();
    out.push_str(&format!(
        "{:<name_w$}  {:<status_w$}  {:>pid_w$}  {:<type_w$}  {:>port_w$}  {}\n",
        "NAME", "STATUS", "PID", "TYPE", "PORT", "DOMAIN"
    ));
    for (i, row) in rows.iter().enumerate() {
        out.push_str(&format!(
            "{:<name_w$}  {:<status_w$}  {:>pid_w$}  {:<type_w$}  {:>port_w$}  {}\n",
            row.name, row.status, pids[i], row.project_type, ports[i], row.domain
        ));
    }
    out
}

fn col_width<'a>(header: &str, values: impl Iterator<Item = &'a str>) -> usize {
    values
        .map(|s| s.chars().count())
        .chain(std::iter::once(header.chars().count()))
        .max()
        .unwrap_or(header.chars().count())
}

#[derive(serde::Serialize)]
struct StatusRow {
    name: String,
    status: String,
    pid: Option<u32>,
    #[serde(rename = "type")]
    project_type: String,
    port: u16,
    domain: String,
    tls: bool,
}

async fn ensure_caddy(config: &Config, frameworks: &FrameworkRegistry) {
    let Some(caddy_cfg) = &config.caddy else {
        return;
    };
    if let Err(e) = caddy::write_and_reload(&config.projects, caddy_cfg, frameworks).await {
        eprintln!("warning: caddy: {e}");
    }
}

async fn wait_ready(
    project: &crate::core::config::ProjectConfig,
    frameworks: &FrameworkRegistry,
) -> Result<()> {
    let attempts = frameworks
        .get(&project.project_type)
        .map(|s| s.lifecycle.ready_attempts)
        .unwrap_or(8);
    match verify_project_domain(project, attempts).await {
        Ok(code) => {
            println!("ready {} ({code})", project_url(project));
            Ok(())
        }
        Err(e) => anyhow::bail!(
            "started {} but {} is not ready: {e}",
            project.name,
            project_url(project)
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::core::config::{Config, ProjectConfig, lookup_project};
    use crate::core::framework::FrameworkId;

    fn project(name: &str) -> ProjectConfig {
        ProjectConfig {
            name: name.into(),
            domain: format!("{name}.test"),
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
            tls: false,
        }
    }

    fn cfg(projects: Vec<ProjectConfig>) -> Config {
        Config {
            tld: "test".into(),
            projects,
            caddy: None,
            discovery: None,
            ignored_services: vec![],
            theme: None,
            language: None,
        }
    }

    #[test]
    fn lookup_finds_exact_name() {
        let config = cfg(vec![project("alpha"), project("beta")]);
        assert_eq!(lookup_project(&config, "beta").unwrap().name, "beta");
    }

    #[test]
    fn lookup_lists_known_on_miss() {
        let config = cfg(vec![project("alpha")]);
        let err = lookup_project(&config, "nope").unwrap_err().to_string();
        assert!(err.contains("nope"));
        assert!(err.contains("alpha"));
    }

    #[test]
    fn lookup_empty_config() {
        let err = lookup_project(&cfg(vec![]), "x").unwrap_err().to_string();
        assert!(err.contains("(none)"));
    }

    #[test]
    fn status_table_aligns_long_names() {
        let table = super::format_status_table(&[
            super::StatusRow {
                name: "kkrr-reservationen".into(),
                status: "stopped".into(),
                pid: None,
                project_type: "symfony".into(),
                port: 8160,
                domain: "reservationen.kkrr.test".into(),
                tls: false,
            },
            super::StatusRow {
                name: "hech".into(),
                status: "running".into(),
                pid: Some(54223),
                project_type: "kirby".into(),
                port: 8220,
                domain: "hech.test".into(),
                tls: false,
            },
        ]);
        let lines: Vec<&str> = table.lines().collect();
        let type_at = lines[0].find("TYPE").expect("TYPE");
        let domain_at = lines[0].find("DOMAIN").expect("DOMAIN");
        assert!(lines[1][type_at..].starts_with("symfony"));
        assert!(lines[2][type_at..].starts_with("kirby  "));
        assert!(lines[1][domain_at..].starts_with("reservationen.kkrr.test"));
        assert!(lines[2][domain_at..].starts_with("hech.test"));
    }
}
