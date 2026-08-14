use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::core::framework::FrameworkId;
use crate::i18n::Language;

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
    /// UI language (`en`, `de`, `sr`, `ru`). Unset → detect from LANG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,
}

/// Optional color theme. `name` selects a shipped or user TOML file;
/// the other fields overlay individual slots.
///
/// Each color accepts a hex string (`#64b4dc`), a named ANSI color
/// (`red`, `green`, `cyan`, `white`, `darkgray`, …), or `reset` to follow
/// the terminal default.
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub struct ThemeConfig {
    /// `groknight` (default), `terminal`, `nightfox`, `catppuccin`,
    /// `macintosh`, `macintosh-dark`, or a file in `~/.config/zapusk/themes/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub bg: Option<String>,
    pub border: Option<String>,
    pub border_focus: Option<String>,
    pub text: Option<String>,
    pub text_dim: Option<String>,
    pub accent: Option<String>,
    pub ok: Option<String>,
    pub warn: Option<String>,
    pub err: Option<String>,
    pub highlight_bg: Option<String>,
    pub highlight_fg: Option<String>,
}

fn default_tld() -> String {
    "test".into()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ProjectConfig {
    pub name: String,
    pub domain: String,
    /// Additional hostnames that should route to this project (besides `domain`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub port: u16,
    #[serde(rename = "type")]
    pub project_type: FrameworkId,
    pub path: String,
    /// Preferred PHP version. For Kirby projects it selects the `php` binary
    /// directly; for Symfony projects it drives the `.php-version` file that
    /// the Symfony CLI reads to pick the runtime PHP version.
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
        std::iter::once(self.domain.as_str()).chain(self.aliases.iter().map(String::as_str))
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

/// Keep a project's `.php-version` file in sync with `php_version` in config.
/// Used by recipes that set `hooks.sync_php_version` (Symfony CLI reads this
/// file; there is no command-line flag). When `php_version` is set the file is
/// written; when it is unset any existing file is removed. Returns notes.
pub fn ensure_php_version_file(config: &ProjectConfig) -> Vec<String> {
    let path = std::path::Path::new(&config.path).join(".php-version");
    let existing = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string());

    let version = config
        .php_version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    match version {
        Some(version) => {
            if existing.as_deref() == Some(version) {
                return vec![format!("PHP version {} (from .php-version)", version)];
            }
            match std::fs::write(&path, format!("{}\n", version)) {
                Ok(()) => vec![format!(
                    "PHP version {} (wrote .php-version from config)",
                    version
                )],
                Err(e) => vec![format!(
                    "could not write {} from configured php_version ({}): {}",
                    path.display(),
                    version,
                    e
                )],
            }
        }
        None => {
            // No version configured: remove a managed `.php-version` so the
            // project uses the default PHP instead of a stale pin.
            if existing.is_none() {
                return vec![];
            }
            match std::fs::remove_file(&path) {
                Ok(()) => vec!["removed .php-version (no php_version set in config)".to_string()],
                Err(e) => vec![format!(
                    "could not remove {} (no php_version in config): {}",
                    path.display(),
                    e
                )],
            }
        }
    }
}

/// Parse a comma-separated alias string from user input into a clean Vec.
pub fn parse_aliases(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct CaddyConfig {
    pub config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caddy_bin: Option<String>,
    /// Template for PHP-FPM socket path, with {version} placeholder.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fpm_socket_template: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
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

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
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
        && tld.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !tld.starts_with('-')
        && !tld.ends_with('-')
}

pub fn hash_config_bytes(bytes: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
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
        assert_eq!(project.project_type.as_str(), "compose");
        assert_eq!(
            project.compose_file.as_deref(),
            Some("docker-compose.dev.yml")
        );
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
        assert_eq!(project.project_type.as_str(), "axum");
        assert_eq!(project.compose_file, None);
        assert_eq!(project.service, None);
        assert!(project.compose_profiles.is_empty());
    }

    #[test]
    fn project_type_from_str_accepts_any_id() {
        assert_eq!(
            "compose".parse::<FrameworkId>().unwrap().as_str(),
            "compose"
        );
        assert_eq!("rails".parse::<FrameworkId>().unwrap().as_str(), "rails");
        assert!("docker compose".parse::<FrameworkId>().is_err());
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

    #[test]
    fn ensure_php_version_file_manages_file_from_config() {
        let dir = std::env::temp_dir().join(format!("zapusk-php-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let php_version_path = dir.join(".php-version");

        let mut project = parse_project(&format!(
            r#"
            [[projects]]
            name = "intranet"
            domain = "intranet.test"
            port = 8120
            type = "symfony"
            path = "{}"
            php_version = "8.3"
            "#,
            dir.display()
        ));

        // Missing file -> written from config, with a note.
        let notes = ensure_php_version_file(&project);
        assert_eq!(
            std::fs::read_to_string(&php_version_path).unwrap().trim(),
            "8.3"
        );
        assert!(notes.iter().any(|n| n.contains("wrote .php-version")));

        // Matching file -> note the effective version, no rewrite.
        let notes = ensure_php_version_file(&project);
        assert!(notes.iter().any(|n| n.contains("from .php-version")));

        // Differing file -> overwritten so config stays authoritative.
        std::fs::write(&php_version_path, "8.4\n").unwrap();
        let notes = ensure_php_version_file(&project);
        assert_eq!(
            std::fs::read_to_string(&php_version_path).unwrap().trim(),
            "8.3"
        );
        assert!(notes.iter().any(|n| n.contains("wrote .php-version")));

        // php_version cleared -> existing file removed.
        project.php_version = None;
        let notes = ensure_php_version_file(&project);
        assert!(!php_version_path.exists());
        assert!(notes.iter().any(|n| n.contains("removed .php-version")));

        // No config and no file -> no-op.
        assert!(ensure_php_version_file(&project).is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
