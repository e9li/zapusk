use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tokio::sync::OnceCell;

use crate::core::config::ProjectConfig;

/// Resolved compose CLI: `docker compose ...` (v2 plugin) or
/// `docker-compose ...` (legacy v1 standalone binary).
#[derive(Debug, Clone)]
pub struct ComposeCli {
    pub bin: String,
    /// Prefix args before the compose subcommand ("compose" for the v2 plugin,
    /// empty for the standalone docker-compose binary).
    pub prefix: Vec<String>,
    /// True when falling back to the EOL v1 standalone binary.
    pub is_v1: bool,
}

static COMPOSE_CLI: OnceCell<Option<ComposeCli>> = OnceCell::const_new();

/// Detect the available compose CLI, preferring the v2 plugin. Cached for the
/// lifetime of the process.
pub async fn compose_cli() -> Result<ComposeCli> {
    let detected = COMPOSE_CLI
        .get_or_init(|| async {
            let v2_ok = Command::new("docker")
                .args(["compose", "version"])
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false);
            if v2_ok {
                return Some(ComposeCli {
                    bin: "docker".into(),
                    prefix: vec!["compose".into()],
                    is_v1: false,
                });
            }
            let v1_ok = Command::new("docker-compose")
                .arg("--version")
                .output()
                .await
                .map(|o| o.status.success())
                .unwrap_or(false);
            if v1_ok {
                return Some(ComposeCli {
                    bin: "docker-compose".into(),
                    prefix: vec![],
                    is_v1: true,
                });
            }
            None
        })
        .await;

    match detected {
        Some(cli) => Ok(cli.clone()),
        None => bail!(
            "docker compose not found. Install Docker (Desktop/OrbStack/colima on macOS, \
             docker engine + compose plugin on Linux) and make sure `docker` is in PATH."
        ),
    }
}

/// Stack identity captured at start/adopt time so stop works without a config.
#[derive(Debug, Clone)]
pub struct StackRef {
    pub dir: PathBuf,
    /// Resolved compose file; None lets the compose CLI auto-detect in `dir`.
    pub file: Option<PathBuf>,
    pub profiles: Vec<String>,
}

impl StackRef {
    pub fn from_config(config: &ProjectConfig) -> Self {
        Self {
            dir: PathBuf::from(&config.path),
            file: config.resolve_compose_file().ok(),
            profiles: config.compose_profiles.clone(),
        }
    }
}

/// Base args for a compose invocation: `[compose] [-f <file>] [--profile <p> ...]`.
fn base_args(cli: &ComposeCli, file: Option<&Path>, profiles: &[String]) -> Vec<String> {
    let mut args = cli.prefix.clone();
    if let Some(file) = file {
        args.push("-f".into());
        args.push(file.display().to_string());
    }
    for profile in profiles {
        args.push("--profile".into());
        args.push(profile.clone());
    }
    args
}

/// Build the foreground `up` command for a compose project.
/// Returns `(bin, args, notes)` matching the shape of `FrameworkSpec::resolve_start`.
pub async fn up_command(config: &ProjectConfig) -> Result<(String, Vec<String>, Vec<String>)> {
    let cli = compose_cli().await?;
    let compose_file = config.resolve_compose_file()?;
    let mut args = base_args(&cli, Some(&compose_file), &config.compose_profiles);
    args.push("up".into());
    args.push("--no-color".into());
    args.push("--remove-orphans".into());
    // Pull/create/start progress is NOT suppressed: stdout/stderr go to log
    // files (non-TTY), so compose uses plain line-by-line progress — visible
    // in the log pane so the user sees that something is happening.
    let mut notes = vec![
        "compose: image pulls and container steps appear below (first start may take a while)"
            .to_string(),
    ];
    if cli.is_v1 {
        notes.push(
            "using legacy docker-compose v1 (EOL) — install the compose v2 plugin".to_string(),
        );
    }
    Ok((cli.bin.clone(), args, notes))
}

/// Check whether the project's compose stack has running containers.
/// Uses `ps -q` (stable across compose versions, unlike `--format json`).
/// Returns false when the compose CLI is unavailable.
pub async fn ps_running(config: &ProjectConfig) -> bool {
    let Ok(cli) = compose_cli().await else {
        return false;
    };
    let stack = StackRef::from_config(config);
    let mut args = base_args(&cli, stack.file.as_deref(), &stack.profiles);
    args.push("ps".into());
    args.push("-q".into());
    if !cli.is_v1 {
        args.push("--status".into());
        args.push("running".into());
    }
    match Command::new(&cli.bin)
        .args(&args)
        .current_dir(&stack.dir)
        .output()
        .await
    {
        Ok(out) if out.status.success() => !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        _ => false,
    }
}

/// Build a `logs -f` command used to re-attach to an externally started stack.
pub async fn logs_follow_command(config: &ProjectConfig) -> Result<(String, Vec<String>)> {
    let cli = compose_cli().await?;
    let stack = StackRef::from_config(config);
    let mut args = base_args(&cli, stack.file.as_deref(), &stack.profiles);
    args.push("logs".into());
    args.push("-f".into());
    args.push("--no-color".into());
    args.push("--tail".into());
    args.push("50".into());
    Ok((cli.bin.clone(), args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::framework::FrameworkId;
    use std::os::unix::fs::PermissionsExt;

    fn compose_project(dir: &Path) -> ProjectConfig {
        ProjectConfig {
            name: "shop".into(),
            domain: "shop.test".into(),
            aliases: vec![],
            port: 8080,
            project_type: FrameworkId::new("compose"),
            path: dir.display().to_string(),
            php_version: None,
            public_dir: None,
            command: None,
            compose_file: None,
            service: None,
            compose_profiles: vec!["dev".into()],
            upstream_host: None,
            args: vec![],
            env: Default::default(),
            autostart: false,
            tls: false,
        }
    }

    #[tokio::test]
    async fn up_command_builds_v2_invocation() {
        let dir = std::env::temp_dir().join(format!("zapusk-docker-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("compose.yaml"), "services: {}\n").unwrap();

        // Fake `docker` CLI so compose_cli() detects the v2 plugin
        let shim = dir.join("docker");
        std::fs::write(&shim, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old_path = std::env::var("PATH").unwrap_or_default();
        unsafe { std::env::set_var("PATH", format!("{}:{}", dir.display(), old_path)) };

        let project = compose_project(&dir);
        let (bin, args, notes) = up_command(&project).await.unwrap();

        assert_eq!(bin, "docker");
        assert_eq!(
            args,
            vec![
                "compose".to_string(),
                "-f".into(),
                dir.join("compose.yaml").display().to_string(),
                "--profile".into(),
                "dev".into(),
                "up".into(),
                "--no-color".into(),
                "--remove-orphans".into(),
            ]
        );
        assert_eq!(notes.len(), 1);

        let (logs_bin, logs_args) = logs_follow_command(&project).await.unwrap();
        assert_eq!(logs_bin, "docker");
        assert!(logs_args.windows(2).any(|w| w == ["logs", "-f"]));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Gracefully stop a compose stack: `docker compose ... stop -t <secs>`.
/// Synchronous from the caller's perspective: the compose CLI waits for the
/// containers to stop before returning.
pub async fn stop_stack(stack: &StackRef, timeout_secs: u32) -> Result<()> {
    let cli = compose_cli().await?;
    let mut args = base_args(&cli, stack.file.as_deref(), &stack.profiles);
    args.push("stop".into());
    args.push("-t".into());
    args.push(timeout_secs.to_string());
    let out = Command::new(&cli.bin)
        .args(&args)
        .current_dir(&stack.dir)
        .output()
        .await?;
    if !out.status.success() {
        bail!(
            "docker compose stop failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}
