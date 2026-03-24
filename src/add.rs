use anyhow::{Context, Result};
use std::io::{self, Write};

use crate::config::config_path;

pub async fn run() -> Result<()> {
    println!("Add a new project to zapusk config\n");

    let name = prompt("Project name")?;
    let default_domain = format!("{}.test", name);
    let domain = prompt_with_default("Domain", &default_domain)?;
    let port: u16 = prompt("Port")?
        .parse()
        .context("Port must be a number")?;
    let project_type = prompt_with_default("Type (phoenix/symfony/kirby/axum)", "phoenix")?;
    let path = prompt("Path (absolute)")?;

    let php_version_line = if project_type == "kirby" {
        let v = prompt_with_default("PHP version", "8.3")?;
        format!("\nphp_version = \"{}\"", v)
    } else {
        String::new()
    };

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
        // Create config dir and empty file
        if let Some(parent) = config_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&config_file, "")?;
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
