use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use crate::platform;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    /// Top-level domain for wildcard DNS (default: "test")
    #[serde(default = "default_tld")]
    pub tld: String,
    #[serde(default, rename = "projects")]
    pub projects: Vec<ProjectConfig>,
    pub caddy: Option<CaddyConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discovery: Option<DiscoveryConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignored_services: Vec<IgnoredService>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeConfig>,
}

/// Optional color overrides for the TUI.
/// Each field accepts a hex color string like "#64b4dc" or a named color
/// like "red", "green", "cyan", "white", "darkgray", "lightgreen", etc.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct ThemeConfig {
    pub border: Option<String>,
    pub border_focus: Option<String>,
    pub text: Option<String>,
    pub text_dim: Option<String>,
    pub accent: Option<String>,
    pub ok: Option<String>,
    pub warn: Option<String>,
    pub err: Option<String>,
    pub highlight_bg: Option<String>,
}

fn default_tld() -> String {
    "test".into()
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub domain: String,
    /// Additional hostnames that should route to this project (besides `domain`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub port: u16,
    #[serde(rename = "type")]
    pub project_type: ProjectType,
    pub path: String,
    /// Only relevant for Kirby projects
    pub php_version: Option<String>,
    /// Document root subfolder for Kirby projects (default: "public")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_dir: Option<String>,
    /// Custom command override (bypasses built-in start_command)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Compose file relative to `path` (compose projects only; default: auto-detect)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compose_file: Option<String>,
    /// Main service name within the compose stack (compose projects only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// Compose profiles passed as --profile flags (compose projects only)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compose_profiles: Vec<String>,
    /// Optional reverse-proxy upstream host override (default: loopback fallback).
    /// Examples: "127.0.0.1", "localhost", "192.168.1.20"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_host: Option<String>,
    /// Structured command args for `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Extra environment variables forwarded to the child process
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// Start this project automatically when zapusk launches
    #[serde(default, skip_serializing_if = "is_false")]
    pub autostart: bool,
    /// Enable TLS via Caddy (uses `tls internal` for local certs)
    #[serde(default, skip_serializing_if = "is_false")]
    pub tls: bool,
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// Compose file names probed (in order) when `compose_file` is not set.
pub const COMPOSE_FILE_CANDIDATES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];

impl ProjectConfig {
    /// Iterator over the primary domain and all aliases.
    pub fn all_hostnames(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.domain.as_str())
            .chain(self.aliases.iter().map(String::as_str))
    }

    /// Resolve the compose file for a compose project: the explicit
    /// `compose_file` (relative to `path` unless absolute), or the first
    /// standard compose file name found in the project directory.
    pub fn resolve_compose_file(&self) -> Result<PathBuf> {
        let base = std::path::Path::new(&self.path);
        if let Some(ref file) = self.compose_file {
            let candidate = std::path::Path::new(file);
            let full = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                base.join(candidate)
            };
            if !full.is_file() {
                anyhow::bail!("compose file not found: {}", full.display());
            }
            return Ok(full);
        }
        for name in COMPOSE_FILE_CANDIDATES {
            let candidate = base.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        anyhow::bail!(
            "no compose file found in {} (looked for {}); set `compose_file` in config",
            self.path,
            COMPOSE_FILE_CANDIDATES.join(", ")
        )
    }
}

/// Parse a comma-separated alias string from user input into a clean Vec.
pub fn parse_aliases(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectType {
    Phoenix,
    Symfony,
    Kirby,
    Axum,
    Compose,
}

impl ProjectType {
    pub fn label(&self) -> &str {
        match self {
            ProjectType::Phoenix => "phoenix",
            ProjectType::Symfony => "symfony",
            ProjectType::Kirby => "kirby",
            ProjectType::Axum => "axum",
            ProjectType::Compose => "compose",
        }
    }

    /// Returns `(binary, args, notes)` to start this project type.
    /// Notes are diagnostic strings to be logged before the process starts.
    /// The command is run from the project's `path`.
    pub fn start_command(&self, config: &ProjectConfig) -> (String, Vec<String>, Vec<String>) {
        match self {
            ProjectType::Phoenix => ("mix".into(), vec!["phx.server".into()], vec![]),
            ProjectType::Symfony => {
                let mut args = vec![
                    "server:start".into(),
                    "--no-tls".into(),
                    "--port".into(),
                    config.port.to_string(),
                ];
                let mut notes = vec![];
                // Read .php-version from project dir if present
                let php_version_path = std::path::Path::new(&config.path).join(".php-version");
                if let Ok(version) = std::fs::read_to_string(&php_version_path) {
                    let version = version.trim().to_string();
                    if !version.is_empty() {
                        args.push("--php-version".into());
                        args.push(version.clone());
                        notes.push(format!("PHP version {} (from .php-version)", version));
                    }
                }
                ("symfony".into(), args, notes)
            }
            ProjectType::Kirby => {
                let (php_bin, notes) =
                    platform::php_binary_resolved(config.php_version.as_deref());
                let doc_root = config.public_dir.as_deref().unwrap_or("public");
                let mut args = vec![
                    "-S".into(),
                    format!("{}:{}", config.domain, config.port),
                ];
                if doc_root != "/" {
                    args.push("-t".into());
                    args.push(doc_root.into());
                }
                args.push("kirby/router.php".into());
                (php_bin, args, notes)
            }
            ProjectType::Axum => ("cargo".into(), vec!["run".into()], vec![]),
            // Compose projects are normally started via core::docker (compose
            // CLI detection + resolved compose file); this is a plain fallback.
            ProjectType::Compose => (
                "docker".into(),
                vec!["compose".into(), "up".into(), "--no-color".into()],
                vec![],
            ),
        }
    }
}

impl fmt::Display for ProjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for ProjectType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "phoenix" => Ok(ProjectType::Phoenix),
            "symfony" => Ok(ProjectType::Symfony),
            "kirby" => Ok(ProjectType::Kirby),
            "axum" => Ok(ProjectType::Axum),
            "compose" => Ok(ProjectType::Compose),
            other => anyhow::bail!(
                "Unknown project type: '{}'. Use: phoenix, symfony, kirby, axum, compose",
                other
            ),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CaddyConfig {
    pub config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caddy_bin: Option<String>,
    /// Template for PHP-FPM socket path, with {version} placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fpm_socket_template: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DiscoveryConfig {
    #[serde(default = "default_web_ports")]
    pub web_ports: Vec<String>,
}

fn default_web_ports() -> Vec<String> {
    vec![
        "80".into(),
        "443".into(),
        "8080".into(),
        "8443".into(),
        "3000-9999".into(),
    ]
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct IgnoredService {
    pub port: u16,
    pub command: String,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        let content = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "Could not read config at {:?}\nCreate one based on config.example.toml",
                path
            )
        })?;

        toml::from_str(&content).context("Failed to parse config.toml")
    }

    /// Load TLD from config, falling back to "test" if config doesn't exist.
    pub fn tld_or_default() -> String {
        Config::load()
            .map(|c| c.tld)
            .unwrap_or_else(|_| default_tld())
    }
}

/// Validate that a TLD contains only safe characters (alphanumeric and hyphens).
pub fn is_valid_tld(tld: &str) -> bool {
    !tld.is_empty()
        && tld.len() <= 63
        && tld
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !tld.starts_with('-')
        && !tld.ends_with('-')
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("zapusk")
        .join("config.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_project(toml_src: &str) -> ProjectConfig {
        let config: Config = toml::from_str(toml_src).expect("config should parse");
        config.projects.into_iter().next().expect("one project")
    }

    #[test]
    fn parses_compose_project() {
        let project = parse_project(
            r#"
            [[projects]]
            name = "shop"
            domain = "shop.test"
            port = 8080
            type = "compose"
            path = "/tmp/shop"
            compose_file = "docker-compose.dev.yml"
            service = "web"
            compose_profiles = ["dev"]
            "#,
        );
        assert_eq!(project.project_type, ProjectType::Compose);
        assert_eq!(project.compose_file.as_deref(), Some("docker-compose.dev.yml"));
        assert_eq!(project.service.as_deref(), Some("web"));
        assert_eq!(project.compose_profiles, vec!["dev"]);
    }

    #[test]
    fn parses_legacy_project_without_compose_fields() {
        let project = parse_project(
            r#"
            [[projects]]
            name = "api"
            domain = "api.test"
            port = 3000
            type = "axum"
            path = "/tmp/api"
            "#,
        );
        assert_eq!(project.project_type, ProjectType::Axum);
        assert_eq!(project.compose_file, None);
        assert_eq!(project.service, None);
        assert!(project.compose_profiles.is_empty());
    }

    #[test]
    fn project_type_from_str_accepts_compose() {
        assert_eq!(
            "compose".parse::<ProjectType>().unwrap(),
            ProjectType::Compose
        );
        assert!("docker".parse::<ProjectType>().is_err());
    }

    #[test]
    fn resolve_compose_file_auto_detects_and_errors() {
        let dir = std::env::temp_dir().join(format!("zapusk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut project = parse_project(&format!(
            r#"
            [[projects]]
            name = "shop"
            domain = "shop.test"
            port = 8080
            type = "compose"
            path = "{}"
            "#,
            dir.display()
        ));

        // Nothing in the directory -> error mentioning the candidates
        let err = project.resolve_compose_file().unwrap_err().to_string();
        assert!(err.contains("compose.yaml"));

        // Auto-detect picks up a standard file name
        std::fs::write(dir.join("docker-compose.yml"), "services: {}\n").unwrap();
        let resolved = project.resolve_compose_file().unwrap();
        assert_eq!(resolved, dir.join("docker-compose.yml"));

        // Explicit compose_file must exist
        project.compose_file = Some("missing.yml".into());
        assert!(project.resolve_compose_file().is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
