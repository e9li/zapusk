use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use crate::core::config::ProjectConfig;
use crate::platform;

const BUILTIN_TOMLS: &[(&str, &str)] = &[
    ("phoenix", include_str!("../frameworks/phoenix.toml")),
    ("symfony", include_str!("../frameworks/symfony.toml")),
    ("kirby", include_str!("../frameworks/kirby.toml")),
    ("axum", include_str!("../frameworks/axum.toml")),
    ("compose", include_str!("../frameworks/compose.toml")),
];

/// Newtype around a framework recipe id (`phoenix`, `rails`, …).
/// Stored as a plain string in `config.toml` (`type = "phoenix"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FrameworkId(pub String);

impl FrameworkId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn label(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for FrameworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for FrameworkId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl PartialEq<&str> for FrameworkId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl FromStr for FrameworkId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            bail!("framework type cannot be empty");
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!(
                "invalid framework id '{}': use letters, numbers, hyphens, or underscores",
                trimmed
            );
        }
        Ok(Self(trimmed.to_ascii_lowercase()))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FrameworkSpec {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub start: StartSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub lifecycle: LifecycleSpec,
    #[serde(default)]
    pub caddy: CaddySpec,
    #[serde(default)]
    pub doctor: DoctorSpec,
    #[serde(default)]
    pub discovery: DiscoverySpec,
    #[serde(default)]
    pub hooks: HooksSpec,
}

impl FrameworkSpec {
    pub fn label(&self) -> &str {
        self.label
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.id)
    }

    /// Resolve the start command after placeholder substitution and PHP hooks.
    pub fn resolve_start(&self, config: &ProjectConfig) -> (String, Vec<String>, Vec<String>) {
        let (php_bin, mut notes) = resolved_php(self, config);
        let ctx = SubstContext::from_project(config, php_bin.as_deref());
        let command = substitute(&self.start.command, &ctx);
        let args = self
            .start
            .args
            .iter()
            .map(|a| substitute(a, &ctx))
            .collect();
        if command.trim().is_empty() {
            notes.push("framework spec has an empty start.command".into());
        }
        (command, args, notes)
    }

    pub fn resolve_env(&self, config: &ProjectConfig) -> HashMap<String, String> {
        let (php_bin, _) = resolved_php(self, config);
        let ctx = SubstContext::from_project(config, php_bin.as_deref());
        self.env
            .iter()
            .map(|(k, v)| (k.clone(), substitute(v, &ctx)))
            .collect()
    }

    pub fn is_compose(&self) -> bool {
        self.lifecycle.kind == LifecycleKind::Compose
    }

    pub fn uses_php(&self) -> bool {
        self.hooks.require_php || self.hooks.resolve_php_binary || self.hooks.sync_php_version
    }
}

fn resolved_php(spec: &FrameworkSpec, config: &ProjectConfig) -> (Option<String>, Vec<String>) {
    if !spec.hooks.resolve_php_binary {
        return (None, vec![]);
    }
    let (bin, notes) = platform::php_binary_resolved(config.php_version.as_deref());
    (Some(bin), notes)
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StartSpec {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LifecycleSpec {
    #[serde(default)]
    pub kind: LifecycleKind,
    #[serde(default = "default_ready_attempts")]
    pub ready_attempts: u32,
}

impl Default for LifecycleSpec {
    fn default() -> Self {
        Self {
            kind: LifecycleKind::Native,
            ready_attempts: default_ready_attempts(),
        }
    }
}

fn default_ready_attempts() -> u32 {
    8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleKind {
    #[default]
    Native,
    Compose,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CaddySpec {
    #[serde(default)]
    pub profile: CaddyProfile,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub block_paths: Vec<String>,
}

impl Default for CaddySpec {
    fn default() -> Self {
        Self {
            profile: CaddyProfile::Proxy,
            root: None,
            block_paths: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaddyProfile {
    #[default]
    Proxy,
    StaticPlusProxy,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DoctorSpec {
    #[serde(default)]
    pub binaries: Vec<String>,
    #[serde(default)]
    pub marker_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DiscoverySpec {
    #[serde(default)]
    pub command_contains: Vec<String>,
    #[serde(default)]
    pub cwd_contains: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HooksSpec {
    #[serde(default)]
    pub sync_php_version: bool,
    #[serde(default)]
    pub resolve_php_binary: bool,
    #[serde(default)]
    pub require_php: bool,
}

#[derive(Debug, Clone)]
pub enum FrameworkSource {
    Builtin,
    User(PathBuf),
}

impl FrameworkSource {
    pub fn label(&self) -> String {
        match self {
            FrameworkSource::Builtin => "builtin".into(),
            FrameworkSource::User(path) => format!("user ({})", path.display()),
        }
    }
}

struct RegistryInner {
    specs: HashMap<String, FrameworkSpec>,
    sources: HashMap<String, FrameworkSource>,
    builtin_order: Vec<String>,
    warnings: Vec<String>,
}

/// Loaded framework recipes: shipped builtins plus `~/.config/zapusk/frameworks/*.toml`.
#[derive(Clone)]
pub struct FrameworkRegistry {
    inner: Arc<RegistryInner>,
}

impl FrameworkRegistry {
    /// Builtins + user directory. Never fails: bad user files become warnings.
    pub fn load() -> Self {
        let mut specs = HashMap::new();
        let mut sources = HashMap::new();
        let mut builtin_order = Vec::new();
        let mut warnings = Vec::new();

        for (id, toml_src) in BUILTIN_TOMLS {
            match parse_spec(toml_src) {
                Ok(spec) => {
                    if spec.id != *id {
                        warnings.push(format!(
                            "builtin {} has id '{}'; using filename id",
                            id, spec.id
                        ));
                    }
                    let mut spec = spec;
                    spec.id = (*id).to_string();
                    builtin_order.push((*id).to_string());
                    sources.insert((*id).to_string(), FrameworkSource::Builtin);
                    specs.insert((*id).to_string(), spec);
                }
                Err(e) => {
                    // A broken builtin is a programming error; still keep going so
                    // the TUI can start, but surface it loudly.
                    warnings.push(format!("builtin {} failed to parse: {}", id, e));
                }
            }
        }

        load_user_dir(&frameworks_dir(), &mut specs, &mut sources, &mut warnings);

        Self {
            inner: Arc::new(RegistryInner {
                specs,
                sources,
                builtin_order,
                warnings,
            }),
        }
    }

    /// Builtins only — used by tests so a developer's user dir cannot leak in.
    #[cfg(test)]
    pub fn builtins_only() -> Self {
        let mut specs = HashMap::new();
        let mut sources = HashMap::new();
        let mut builtin_order = Vec::new();

        for (id, toml_src) in BUILTIN_TOMLS {
            let mut spec = parse_spec(toml_src)
                .unwrap_or_else(|e| panic!("invalid builtin framework {id}: {e}"));
            spec.id = (*id).to_string();
            builtin_order.push((*id).to_string());
            sources.insert((*id).to_string(), FrameworkSource::Builtin);
            specs.insert((*id).to_string(), spec);
        }

        Self {
            inner: Arc::new(RegistryInner {
                specs,
                sources,
                builtin_order,
                warnings: vec![],
            }),
        }
    }

    pub fn get(&self, id: &FrameworkId) -> Option<&FrameworkSpec> {
        self.inner.specs.get(id.as_str())
    }

    #[cfg(test)]
    pub fn get_str(&self, id: &str) -> Option<&FrameworkSpec> {
        self.inner.specs.get(id)
    }

    pub fn get_required(&self, id: &FrameworkId) -> Result<&FrameworkSpec> {
        self.get(id).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown framework '{}'. Add ~/.config/zapusk/frameworks/{}.toml \
                 or use a built-in: {}",
                id,
                id,
                self.ids().join(", ")
            )
        })
    }

    pub fn contains(&self, id: &str) -> bool {
        self.inner.specs.contains_key(id)
    }

    /// Builtin ids first (shipped order), then extra user ids alphabetically.
    pub fn ids(&self) -> Vec<String> {
        let mut ids = self.inner.builtin_order.clone();
        let mut extras: Vec<String> = self
            .inner
            .specs
            .keys()
            .filter(|k| !self.inner.builtin_order.iter().any(|b| b == *k))
            .cloned()
            .collect();
        extras.sort();
        ids.extend(extras);
        ids
    }

    pub fn source(&self, id: &str) -> Option<&FrameworkSource> {
        self.inner.sources.get(id)
    }

    pub fn warnings(&self) -> &[String] {
        &self.inner.warnings
    }

    pub fn is_compose(&self, id: &FrameworkId) -> bool {
        self.get(id).map(|s| s.is_compose()).unwrap_or(false)
    }

    /// First spec whose discovery needles match the haystack (command + cwd).
    pub fn match_discovery(&self, hay: &str) -> Option<&str> {
        let hay = hay.to_ascii_lowercase();
        for id in self.ids() {
            let Some(spec) = self.inner.specs.get(&id) else {
                continue;
            };
            let cmd_hit = spec
                .discovery
                .command_contains
                .iter()
                .any(|n| !n.is_empty() && hay.contains(&n.to_ascii_lowercase()));
            let cwd_hit = spec
                .discovery
                .cwd_contains
                .iter()
                .any(|n| !n.is_empty() && hay.contains(&n.to_ascii_lowercase()));
            if cmd_hit || cwd_hit {
                return Some(spec.id.as_str());
            }
        }
        None
    }
}

fn parse_spec(toml_src: &str) -> Result<FrameworkSpec> {
    let spec: FrameworkSpec = toml::from_str(toml_src).context("invalid framework TOML")?;
    if spec.id.trim().is_empty() {
        bail!("framework spec is missing `id`");
    }
    let _id: FrameworkId = spec.id.parse()?;
    Ok(spec)
}

fn load_user_dir(
    dir: &Path,
    specs: &mut HashMap<String, FrameworkSpec>,
    sources: &mut HashMap<String, FrameworkSource>,
    warnings: &mut Vec<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            warnings.push(format!("could not read {}: {}", dir.display(), e));
            return;
        }
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("toml"))
        .collect();
    files.sort();

    for path in files {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("{}: could not read ({})", path.display(), e));
                continue;
            }
        };
        match parse_spec(&content) {
            Ok(spec) => {
                let id = spec.id.clone();
                if let Some(FrameworkSource::User(prev)) = sources.get(&id) {
                    warnings.push(format!(
                        "framework '{}' redefined by {} (was {})",
                        id,
                        path.display(),
                        prev.display()
                    ));
                }
                sources.insert(id.clone(), FrameworkSource::User(path));
                specs.insert(id, spec);
            }
            Err(e) => {
                warnings.push(format!("{}: {}", path.display(), e));
            }
        }
    }
}

pub fn frameworks_dir() -> PathBuf {
    crate::core::config::config_path()
        .parent()
        .map(|p| p.join("frameworks"))
        .unwrap_or_else(|| PathBuf::from("frameworks"))
}

pub fn ensure_frameworks_dir() -> Result<PathBuf> {
    let dir = frameworks_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;
    Ok(dir)
}

pub struct SubstContext {
    pub port: String,
    pub path: String,
    pub domain: String,
    pub name: String,
    pub php_version: String,
    pub public_dir: String,
    pub root: String,
    pub php: String,
}

impl SubstContext {
    pub fn from_project(config: &ProjectConfig, php_bin: Option<&str>) -> Self {
        let public_dir = config
            .public_dir
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("public")
            .to_string();
        let root = if public_dir == "/" {
            config.path.clone()
        } else {
            format!(
                "{}/{}",
                config.path.trim_end_matches('/'),
                public_dir.trim_start_matches('/')
            )
        };
        Self {
            port: config.port.to_string(),
            path: config.path.clone(),
            domain: config.domain.clone(),
            name: config.name.clone(),
            php_version: config.php_version.clone().unwrap_or_default(),
            public_dir,
            root,
            php: php_bin.unwrap_or("php").to_string(),
        }
    }
}

/// Replace `{port}`, `{path}`, `{domain}`, `{name}`, `{php_version}`,
/// `{public_dir}`, `{root}`, `{php}`. Longer tokens are applied first so
/// `{php}` cannot eat `{php_version}`.
pub fn substitute(template: &str, ctx: &SubstContext) -> String {
    let replacements = [
        ("{php_version}", ctx.php_version.as_str()),
        ("{public_dir}", ctx.public_dir.as_str()),
        ("{domain}", ctx.domain.as_str()),
        ("{path}", ctx.path.as_str()),
        ("{port}", ctx.port.as_str()),
        ("{name}", ctx.name.as_str()),
        ("{root}", ctx.root.as_str()),
        ("{php}", ctx.php.as_str()),
    ];
    let mut out = template.to_string();
    for (token, value) in replacements {
        if out.contains(token) {
            out = out.replace(token, value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, path: &str, port: u16) -> ProjectConfig {
        ProjectConfig {
            name: "demo".into(),
            domain: "demo.test".into(),
            aliases: vec![],
            port,
            project_type: FrameworkId::new(id),
            path: path.into(),
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

    #[test]
    fn builtins_parse_and_keep_order() {
        let reg = FrameworkRegistry::builtins_only();
        assert_eq!(
            reg.ids(),
            vec!["phoenix", "symfony", "kirby", "axum", "compose"]
        );
        assert!(reg.warnings().is_empty());
        for id in ["phoenix", "symfony", "kirby", "axum", "compose"] {
            let spec = reg.get_str(id).expect(id);
            assert_eq!(spec.id, id);
            assert_eq!(spec.label(), id);
        }
    }

    #[test]
    fn phoenix_start_and_env() {
        let reg = FrameworkRegistry::builtins_only();
        let spec = reg.get_str("phoenix").unwrap();
        let cfg = project("phoenix", "/tmp/shop", 4000);
        let (bin, args, notes) = spec.resolve_start(&cfg);
        assert_eq!(bin, "mix");
        assert_eq!(args, vec!["phx.server"]);
        assert!(notes.is_empty());
        let env = spec.resolve_env(&cfg);
        assert_eq!(env.get("PHX_HOST").map(String::as_str), Some("demo.test"));
        assert_eq!(env.get("PHX_SERVER").map(String::as_str), Some("true"));
        assert!(!spec.is_compose());
        assert_eq!(spec.lifecycle.ready_attempts, 8);
    }

    #[test]
    fn symfony_start_substitutes_port() {
        let reg = FrameworkRegistry::builtins_only();
        let spec = reg.get_str("symfony").unwrap();
        let cfg = project("symfony", "/tmp/blog", 8002);
        let (bin, args, _) = spec.resolve_start(&cfg);
        assert_eq!(bin, "symfony");
        assert_eq!(args, vec!["server:start", "--no-tls", "--port", "8002"]);
        assert!(spec.hooks.sync_php_version);
        assert_eq!(
            spec.resolve_env(&cfg)
                .get("TRUSTED_PROXIES")
                .map(String::as_str),
            Some("127.0.0.1,::1")
        );
    }

    #[test]
    fn kirby_start_uses_root_and_php_placeholder() {
        let reg = FrameworkRegistry::builtins_only();
        let spec = reg.get_str("kirby").unwrap();
        let mut cfg = project("kirby", "/tmp/site", 8001);
        cfg.php_version = Some("8.1".into());
        let (bin, args, _) = spec.resolve_start(&cfg);
        assert!(bin.ends_with("php") || bin.contains("php@"));
        assert_eq!(
            args,
            vec![
                "-S".to_string(),
                "demo.test:8001".into(),
                "-t".into(),
                "/tmp/site/public".into(),
                "kirby/router.php".into(),
            ]
        );
        assert_eq!(spec.caddy.profile, CaddyProfile::StaticPlusProxy);
        assert!(spec.hooks.resolve_php_binary);
        assert!(spec.hooks.require_php);
    }

    #[test]
    fn kirby_root_slash_is_project_path() {
        let mut cfg = project("kirby", "/tmp/site", 8001);
        cfg.public_dir = Some("/".into());
        let ctx = SubstContext::from_project(&cfg, None);
        assert_eq!(ctx.root, "/tmp/site");
    }

    #[test]
    fn compose_is_compose_lifecycle() {
        let reg = FrameworkRegistry::builtins_only();
        let spec = reg.get_str("compose").unwrap();
        assert!(spec.is_compose());
        assert_eq!(spec.lifecycle.ready_attempts, 40);
        assert!(reg.is_compose(&FrameworkId::new("compose")));
        assert!(!reg.is_compose(&FrameworkId::new("axum")));
    }

    #[test]
    fn substitute_does_not_eat_php_version() {
        let cfg = {
            let mut p = project("x", "/app", 3000);
            p.php_version = Some("8.3".into());
            p
        };
        let ctx = SubstContext::from_project(&cfg, Some("/opt/php@8.3/bin/php"));
        assert_eq!(
            substitute("php{php_version} {php}", &ctx),
            "php8.3 /opt/php@8.3/bin/php"
        );
        assert_eq!(substitute("{domain}:{port}", &ctx), "demo.test:3000");
        assert_eq!(substitute("{root}", &ctx), "/app/public");
    }

    #[test]
    fn unknown_hook_is_rejected() {
        let src = r#"
            id = "x"
            [hooks]
            invent_new_hook = true
        "#;
        assert!(parse_spec(src).is_err());
    }

    #[test]
    fn unknown_caddy_profile_is_rejected() {
        let src = r#"
            id = "x"
            [caddy]
            profile = "raw_snippet"
        "#;
        assert!(parse_spec(src).is_err());
    }

    #[test]
    fn user_override_replaces_builtin() {
        let dir = std::env::temp_dir().join(format!(
            "zapusk-fw-override-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("phoenix.toml"),
            r#"
            id = "phoenix"
            [start]
            command = "custom-mix"
            args = ["phx.server"]
            "#,
        )
        .unwrap();

        let mut specs = HashMap::new();
        let mut sources = HashMap::new();
        let mut warnings = Vec::new();
        let builtin = parse_spec(include_str!("../frameworks/phoenix.toml")).unwrap();
        specs.insert("phoenix".into(), builtin);
        sources.insert("phoenix".into(), FrameworkSource::Builtin);
        load_user_dir(&dir, &mut specs, &mut sources, &mut warnings);

        assert_eq!(specs["phoenix"].start.command, "custom-mix");
        assert!(matches!(sources["phoenix"], FrameworkSource::User(_)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn match_discovery_prefers_builtin_order() {
        let reg = FrameworkRegistry::builtins_only();
        assert_eq!(reg.match_discovery("mix phx.server"), Some("phoenix"));
        assert_eq!(
            reg.match_discovery("/usr/bin/php artisan serve"),
            Some("symfony")
        );
        assert_eq!(
            reg.match_discovery("cargo run /target/debug/api"),
            Some("axum")
        );
        assert_eq!(reg.match_discovery("postgres"), None);
    }

    #[test]
    fn framework_id_from_str_lowercases() {
        assert_eq!("Rails".parse::<FrameworkId>().unwrap().as_str(), "rails");
        assert!("".parse::<FrameworkId>().is_err());
        assert!("has space".parse::<FrameworkId>().is_err());
    }

    #[test]
    fn example_recipes_parse() {
        for (name, src) in [
            ("rails", include_str!("../../frameworks.example/rails.toml")),
            (
                "laravel",
                include_str!("../../frameworks.example/laravel.toml"),
            ),
            (
                "express",
                include_str!("../../frameworks.example/express.toml"),
            ),
        ] {
            let spec = parse_spec(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(spec.id, name);
        }
    }
}
