use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    #[serde(rename = "projects")]
    pub projects: Vec<ProjectConfig>,
    pub caddy: Option<CaddyConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub domain: String,
    pub port: u16,
    #[serde(rename = "type")]
    pub project_type: ProjectType,
    pub path: String,
    /// Only relevant for Kirby projects
    pub php_version: Option<String>,
    /// Custom command override (bypasses built-in start_command)
    pub command: Option<String>,
    /// Extra environment variables forwarded to the child process
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Start this project automatically when zapusk launches
    #[serde(default)]
    pub autostart: bool,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Phoenix,
    Symfony,
    Kirby,
    Axum,
    // TODO: add more as needed
}

impl ProjectType {
    pub fn label(&self) -> &str {
        match self {
            ProjectType::Phoenix => "phoenix",
            ProjectType::Symfony => "symfony",
            ProjectType::Kirby => "kirby",
            ProjectType::Axum => "axum",
        }
    }

    /// Returns the command to start this project type.
    /// The command is run from the project's `path`.
    pub fn start_command(&self, config: &ProjectConfig) -> (String, Vec<String>) {
        match self {
            ProjectType::Phoenix => (
                "mix".into(),
                vec!["phx.server".into()],
            ),
            ProjectType::Symfony => {
                let mut args = vec![
                    "server:start".into(),
                    "--no-tls".into(),
                    "--port".into(),
                    config.port.to_string(),
                ];
                // Read .php-version from project dir if present
                let php_version_path = std::path::Path::new(&config.path).join(".php-version");
                if let Ok(version) = std::fs::read_to_string(&php_version_path) {
                    let version = version.trim().to_string();
                    if !version.is_empty() {
                        args.push("--php-version".into());
                        args.push(version);
                    }
                }
                ("symfony".into(), args)
            }
            ProjectType::Kirby => {
                let php_bin = php_binary(config.php_version.as_deref());
                (
                    php_bin,
                    vec![
                        "-S".into(),
                        format!("localhost:{}", config.port),
                        "-t".into(),
                        "public".into(),
                    ],
                )
            }
            ProjectType::Axum => (
                "cargo".into(),
                vec!["run".into()],
            ),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct CaddyConfig {
    pub config_path: String,
    pub caddy_bin: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Could not read config at {:?}\nCreate one based on config.example.toml", path))?;

        toml::from_str(&content).context("Failed to parse config.toml")
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("zapusk")
        .join("config.toml")
}

fn php_binary(version: Option<&str>) -> String {
    match version {
        Some(v) => format!("/opt/homebrew/opt/php@{}/bin/php", v),
        None => "php".into(),
    }
}
