use anyhow::{Context, Result};
use std::io::{self, Write};

use crate::core::config::{config_path, Config, ProjectType};

pub async fn run() -> Result<()> {
    println!("Add a new project to zapusk config\n");

    let tld = Config::tld_or_default();
    let name = prompt("Project name")?;
    let slug = crate::core::slugify(&name);
    let default_domain = format!("{}.{}", slug, tld);
    let domain = prompt_with_default("Domain", &default_domain)?;
    let port: u16 = prompt("Port")?
        .parse()
        .context("Port must be a number")?;
    let project_type: ProjectType = prompt_with_default("Type (phoenix/symfony/kirby/axum)", "phoenix")?
        .parse()
        .context("Invalid project type")?;
    let path = prompt("Project directory (e.g. /home/user/projects/myapp)")?;
    if !std::path::Path::new(&path).is_dir() {
        anyhow::bail!("Directory not found: {}", path);
    }

    let php_version_line = if project_type == ProjectType::Kirby {
        let v = prompt_with_default("PHP version", "8.3")?;
        format!("\nphp_version = \"{}\"", v)
    } else {
        String::new()
    };

    // Check for duplicates in existing config
    if let Ok(config) = Config::load() {
        if config.projects.iter().any(|p| p.name == name) {
            anyhow::bail!("A project named '{}' already exists in config", name);
        }
        if config.projects.iter().any(|p| p.domain == domain) {
            anyhow::bail!("Domain '{}' is already used by another project", domain);
        }
    }

    let block = format!(
        r#"

[[projects]]
name = "{name}"
domain = "{domain}"
port = {port}
type = "{project_type}"
path = "{path}"{php_version_line}
"#,
    );

    let config_file = config_path();
    if !config_file.exists() {
        // Create config dir and a minimal config with TLD + caddy section
        if let Some(parent) = config_file.parent() {
            std::fs::create_dir_all(parent)?;
        } else {
            anyhow::bail!("Could not determine parent dir for {}", config_file.display());
        }
        let caddyfile_path = config_file
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Could not determine parent dir for {}", config_file.display()))?
            .join("Caddyfile");
        let initial = format!(
            "# zapusk config\n\ntld = \"{tld}\"\n\n[caddy]\nconfig_path = \"{}\"\n",
            caddyfile_path.display()
        );
        std::fs::write(&config_file, initial)?;
        println!("Created {}", config_file.display());
    }

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
