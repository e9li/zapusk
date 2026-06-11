use anyhow::{Context, Result};
use std::io::{self, Write};

use crate::core::config::{config_path, parse_aliases, Config, ProjectConfig, ProjectType};

pub async fn run() -> Result<()> {
    println!("Add a new project to zapusk config\n");

    let tld = Config::tld_or_default();
    let name = prompt("Project name")?;
    let slug = crate::core::slugify(&name);
    let default_domain = format!("{}.{}", slug, tld);
    let domain = prompt_with_default("Domain", &default_domain)?;
    let aliases_raw = prompt_with_default("Additional domains (comma-separated)", "")?;
    let aliases = parse_aliases(&aliases_raw);
    let port: u16 = prompt("Port")?.parse().context("Port must be a number 1-65535")?;
    if port == 0 {
        anyhow::bail!("Port must be between 1 and 65535");
    }
    let project_type: ProjectType =
        prompt_with_default("Type (phoenix/symfony/kirby/axum/compose)", "phoenix")?
            .parse()
            .context("Invalid project type")?;
    let tls = prompt_bool_with_default("Enable TLS (https)", false)?;
    let path = prompt("Project directory (e.g. /home/user/projects/myapp)")?;
    if !std::path::Path::new(&path).is_dir() {
        anyhow::bail!("Directory not found: {}", path);
    }

    let php_version = if project_type == ProjectType::Kirby {
        Some(prompt_with_default("PHP version", "8.3")?)
    } else {
        None
    };

    let (compose_file, service) = if project_type == ProjectType::Compose {
        let file = prompt_with_default("Compose file (relative to project dir)", "auto-detect")?;
        let compose_file = if file == "auto-detect" {
            None
        } else {
            if !std::path::Path::new(&path).join(&file).is_file() {
                anyhow::bail!("Compose file not found: {}/{}", path, file);
            }
            Some(file)
        };
        let service = prompt_with_default("Main service name (optional)", "")?;
        let service = if service.is_empty() { None } else { Some(service) };
        (compose_file, service)
    } else {
        (None, None)
    };

    // Check for duplicates in existing config
    if let Ok(config) = Config::load() {
        if config.projects.iter().any(|p| p.name == name) {
            anyhow::bail!("A project named '{}' already exists in config", name);
        }
        let candidates: Vec<&str> = std::iter::once(domain.as_str())
            .chain(aliases.iter().map(String::as_str))
            .collect();
        for project in &config.projects {
            for existing in project.all_hostnames() {
                if candidates.iter().any(|c| *c == existing) {
                    anyhow::bail!(
                        "Domain '{}' is already used by project '{}'",
                        existing,
                        project.name
                    );
                }
            }
        }
        // Internal duplicates within the new candidate list
        for i in 0..candidates.len() {
            for j in (i + 1)..candidates.len() {
                if candidates[i] == candidates[j] {
                    anyhow::bail!("Duplicate hostname '{}'", candidates[i]);
                }
            }
        }
    }

    let new_project = ProjectConfig {
        name: name.clone(),
        domain,
        aliases,
        port,
        project_type,
        path,
        php_version,
        public_dir: None,
        command: None,
        compose_file,
        service,
        compose_profiles: vec![],
        upstream_host: None,
        args: vec![],
        env: Default::default(),
        autostart: false,
        tls,
    };

    let config_file = config_path();
    if !config_file.exists() {
        // Create config dir and a minimal config with TLD + caddy section
        if let Some(parent) = config_file.parent() {
            std::fs::create_dir_all(parent)?;
        } else {
            anyhow::bail!(
                "Could not determine parent dir for {}",
                config_file.display()
            );
        }
        let caddyfile_path = config_file
            .parent()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Could not determine parent dir for {}",
                    config_file.display()
                )
            })?
            .join("Caddyfile");
        let initial = format!(
            "# zapusk config\n\ntld = \"{tld}\"\n\n[caddy]\nconfig_path = \"{}\"\n",
            caddyfile_path.display()
        );
        std::fs::write(&config_file, initial)?;
        println!("Created {}", config_file.display());
    }

    // Serialize using TOML library to safely handle special characters in user input
    let project_toml = toml::to_string_pretty(&new_project)
        .context("Failed to serialize project config")?;
    let block = format!("\n\n[[projects]]\n{}", project_toml);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&config_file)
        .with_context(|| format!("Could not open {:?}", config_file))?;

    file.write_all(block.as_bytes())?;

    println!("\nAdded {} to {:?}", name, config_file);
    println!("Run `zapusk` to manage it, or `zapusk doctor` to verify the setup.");

    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{}: ", label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();
    if input.is_empty() {
        anyhow::bail!("{} cannot be empty", label);
    }
    Ok(input)
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    print!("{} [{}]: ", label, default);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_string();
    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input)
    }
}

fn prompt_bool_with_default(label: &str, default: bool) -> Result<bool> {
    let default_label = if default { "Y/n" } else { "y/N" };
    print!("{} [{}]: ", label, default_label);
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();
    if input.is_empty() {
        return Ok(default);
    }
    match input.as_str() {
        "y" | "yes" | "true" | "1" => Ok(true),
        "n" | "no" | "false" | "0" => Ok(false),
        _ => anyhow::bail!("Please enter y/n"),
    }
}
