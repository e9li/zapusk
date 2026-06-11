use anyhow::Result;
use std::io::{self, Write};
use std::path::Path;
use tokio::process::Command;

use crate::cli::doctor;
use crate::cli::spinner::Spinner;
use crate::core::caddy;
use crate::core::config::Config;
use crate::platform;

pub async fn run() -> Result<()> {
    println!("Welcome to zapusk!");
    println!("Let us make sure your local dev stack is ready.\n");

    let tld = prompt_tld();

    // Check for conflicting tools before making any changes
    if !check_conflicts_before_init(&tld).await {
        println!("\nAborted. No changes were made.");
        return Ok(());
    }

    step_caddy().await?;
    step_dnsmasq_install().await?;
    step_dnsmasq_config(&tld).await?;
    step_dnsmasq_start().await?;
    step_config(&tld).await?;
    step_docker().await?;

    // Final doctor check
    println!("\n--- Final check ---");
    let sp = Spinner::start("Running diagnostics...");
    let all_ok = doctor::run_quiet().await.unwrap_or(false);
    if all_ok {
        sp.done("All checks passed").await;
        println!(
            "\nSetup complete. Run `zapusk add` to add a project, then `zapusk` to open the TUI."
        );
    } else {
        sp.fail("Some checks failed — run `zapusk doctor` for details")
            .await;
        println!("\nSetup finished with issues. Run `zapusk doctor` to see what needs fixing.");
    }

    Ok(())
}

async fn step_caddy() -> Result<()> {
    println!("[1/6] Checking Caddy...");
    let check = doctor::check_caddy().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Install Caddy via Homebrew?") {
            let sp = Spinner::start("Installing Caddy...");
            let status = Command::new("brew")
                .args(["install", "caddy"])
                .output()
                .await?;
            if status.status.success() {
                sp.done("Caddy installed").await;
            } else {
                sp.fail("Installation failed — install manually").await;
            }
        }
    } else {
        println!("      Install Caddy: https://caddyserver.com/docs/install");
        println!("      Then re-run `zapusk init`");
    }
    Ok(())
}

async fn step_dnsmasq_install() -> Result<()> {
    println!("\n[2/6] Checking dnsmasq...");
    let check = doctor::check_dnsmasq_installed().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Install dnsmasq via Homebrew?") {
            let sp = Spinner::start("Installing dnsmasq...");
            let status = Command::new("brew")
                .args(["install", "dnsmasq"])
                .output()
                .await?;
            if status.status.success() {
                sp.done("dnsmasq installed").await;
            } else {
                sp.fail("Installation failed").await;
            }
        }
    } else {
        println!("      Run: sudo apt install dnsmasq");
        println!("      Then re-run `zapusk init`");
    }
    Ok(())
}

async fn step_dnsmasq_config(tld: &str) -> Result<()> {
    println!("\n[3/6] Configuring dnsmasq for *.{}...", tld);
    let check = doctor::check_dnsmasq_config(tld).await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
    } else {
        let config_path = platform::dnsmasq_config_path();
        let entry = format!("address=/.{}/127.0.0.1", tld);

        if prompt_yn(&format!("      Add {} to {}?", entry, config_path)) {
            let content = format!("\n# Added by zapusk\n{}\n", entry);
            match std::fs::OpenOptions::new().append(true).open(config_path) {
                Ok(mut f) => {
                    use std::io::Write as _;
                    f.write_all(content.as_bytes())?;
                    println!("      \u{2713} Config updated");
                }
                Err(e) => {
                    println!("      \u{2717} Could not write: {} — try with sudo", e);
                }
            }
        }
    }

    // macOS resolver
    if cfg!(target_os = "macos") {
        let resolver_file = format!("/etc/resolver/{}", tld);
        ensure_macos_resolver(&resolver_file).await?;
    }

    Ok(())
}

async fn ensure_macos_resolver(resolver_file: &str) -> Result<()> {
    let resolver_path = Path::new(resolver_file);
    let expected = "nameserver 127.0.0.1";
    let content_to_write = "\n# Added by zapusk\nnameserver 127.0.0.1\n";

    if resolver_path.exists() {
        match std::fs::read_to_string(resolver_path) {
            Ok(content) if content.lines().any(|line| line.trim() == expected) => {
                println!("      \u{2713} {} already configured", resolver_file);
                return Ok(());
            }
            Ok(_) => {
                println!(
                    "      ! {} exists but is missing `{}`",
                    resolver_file, expected
                );
                if prompt_yn("      Append nameserver entry to existing file? (requires sudo)") {
                    let status = Command::new("sudo")
                        .args(["tee", "-a", resolver_file])
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::null())
                        .spawn();
                    match status {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.as_mut() {
                                use tokio::io::AsyncWriteExt;
                                let _ = stdin.write_all(content_to_write.as_bytes()).await;
                            }
                            let result = child.wait().await?;
                            if result.success() {
                                println!("      \u{2713} Updated existing resolver file");
                            } else {
                                println!("      \u{2717} Failed to update resolver file");
                            }
                        }
                        Err(e) => println!("      \u{2717} Failed to run sudo: {}", e),
                    }
                }
                return Ok(());
            }
            Err(_) => {
                println!(
                    "      ! {} exists but could not be read without elevated permissions",
                    resolver_file
                );
                if prompt_yn(
                    "      Ensure nameserver entry with sudo? (will keep existing content)",
                ) {
                    let status = Command::new("sudo")
                        .args(["tee", "-a", resolver_file])
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::null())
                        .spawn();
                    match status {
                        Ok(mut child) => {
                            if let Some(stdin) = child.stdin.as_mut() {
                                use tokio::io::AsyncWriteExt;
                                let _ = stdin.write_all(content_to_write.as_bytes()).await;
                            }
                            let result = child.wait().await?;
                            if result.success() {
                                println!("      \u{2713} Resolver file ensured");
                            } else {
                                println!("      \u{2717} Failed to ensure resolver file");
                            }
                        }
                        Err(e) => println!("      \u{2717} Failed to run sudo: {}", e),
                    }
                }
                return Ok(());
            }
        }
    }

    if prompt_yn(&format!("      Create {}? (requires sudo)", resolver_file)) {
        // Ensure /etc/resolver exists
        let _ = Command::new("sudo")
            .args(["mkdir", "-p", "/etc/resolver"])
            .status()
            .await;

        let status = Command::new("sudo")
            .args(["tee", resolver_file])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn();
        match status {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(b"nameserver 127.0.0.1\n").await;
                }
                let result = child.wait().await?;
                if result.success() {
                    println!("      \u{2713} {} created", resolver_file);
                } else {
                    println!("      \u{2717} Failed — create it manually");
                }
            }
            Err(e) => println!("      \u{2717} Failed to run sudo: {}", e),
        }
    }

    Ok(())
}

async fn step_dnsmasq_start() -> Result<()> {
    println!("\n[4/6] Starting dnsmasq...");
    let check = doctor::check_dnsmasq_running().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Start dnsmasq via Homebrew? (requires sudo)") {
            let sp = Spinner::start("Starting dnsmasq with sudo...");
            let status = Command::new("sudo")
                .args(["brew", "services", "start", "dnsmasq"])
                .status()
                .await?;
            if status.success() {
                sp.done("dnsmasq started").await;
            } else {
                sp.fail("Failed to start — try manually: sudo brew services start dnsmasq")
                    .await;
            }
        }
    } else {
        println!("      Run: sudo systemctl start dnsmasq");
    }
    Ok(())
}

async fn step_config(tld: &str) -> Result<()> {
    let config_file = crate::core::config::config_path();

    println!("\n[5/6] Config file...");
    if config_file.exists() {
        println!("      \u{2713} Found at {}", config_file.display());
    } else {
        println!("      No config found at {}", config_file.display());
        if prompt_yn("      Create a starter config?") {
            if let Some(parent) = config_file.parent() {
                std::fs::create_dir_all(parent)?;
            } else {
                println!("      ✗ Could not determine config directory");
                return Ok(());
            }
            let caddyfile_path = config_file
                .parent()
                .map(|p| p.join("Caddyfile"))
                .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
            let initial = format!(
                "# zapusk config\n\ntld = \"{tld}\"\n\n[caddy]\nconfig_path = \"{}\"\n",
                caddyfile_path.display()
            );
            std::fs::write(&config_file, &initial)?;
            println!("      \u{2713} Created {}", config_file.display());
        } else {
            println!("      Skipping — create it later with `zapusk add`");
            return Ok(());
        }
    }

    // Try to generate Caddyfile if config has projects
    if let Ok(config) = Config::load() {
        if let Some(ref caddy_cfg) = config.caddy {
            if config.projects.is_empty() {
                println!("      No projects yet — add one with `zapusk add`");
            } else if prompt_yn("      Write Caddyfile and reload Caddy?") {
                let sp = Spinner::start("Writing Caddyfile and reloading Caddy...");
                match caddy::write_and_reload(&config.projects, caddy_cfg).await {
                    Ok(()) => sp.done("Caddyfile written and Caddy reloaded").await,
                    Err(e) => sp.fail(&format!("{}", e)).await,
                }
            }
        }
    }

    Ok(())
}

/// Docker is only required for `type = "compose"` projects, so this step is
/// informational when none are configured. Installing Docker is left to the
/// user — the step prints per-OS hints instead.
async fn step_docker() -> Result<()> {
    println!("\n[6/6] Checking Docker (compose projects)...");

    let has_compose = Config::load()
        .map(|c| doctor::has_compose_projects(&c))
        .unwrap_or(false);
    if !has_compose {
        println!("      \u{2713} Skipped — no compose projects in config");
        println!("      (add one with `zapusk add`, type \"compose\", if your team uses Docker)");
        return Ok(());
    }

    for check in doctor::check_docker().await {
        if check.ok {
            println!("      \u{2713} {}", check.detail);
        } else {
            let mark = if check.is_warning { '\u{26a0}' } else { '\u{2717}' };
            println!("      {} {}", mark, check.detail);
            if let Some(hint) = &check.fix_hint {
                println!("      \u{2192} {}", hint);
            }
        }
    }

    Ok(())
}

/// Run conflict checks and warn the user before proceeding.
/// Returns true if we should continue, false if the user aborted.
async fn check_conflicts_before_init(tld: &str) -> bool {
    let mut found: Vec<(&str, String)> = vec![];

    let tools: &[(&str, String)] = &[
        (
            "ddev",
            format!(
                "ddev manages its own DNS router and may conflict with dnsmasq (.{} domains)",
                tld
            ),
        ),
        (
            "herd",
            format!(
                "Laravel Herd manages DNS and nginx on port 80/443 (.{} domains)",
                tld
            ),
        ),
        (
            "valet",
            format!(
                "Laravel Valet manages dnsmasq and nginx on port 80/443 (.{} domains)",
                tld
            ),
        ),
    ];

    for (binary, description) in tools {
        if is_tool_actively_conflicting(binary).await {
            found.push((*binary, description.clone()));
        }
    }

    // Check port 80
    let port_80_taken = std::net::TcpListener::bind(("127.0.0.1", 80)).is_err();

    if found.is_empty() && !port_80_taken {
        return true;
    }

    println!("\u{26a0}  Potential conflicts detected:\n");

    for (name, description) in &found {
        println!("   \u{2022} {} is installed — {}", name, description);
    }
    if port_80_taken {
        println!("   \u{2022} Port 80 is already in use — Caddy needs this port");
    }

    println!();
    println!("   zapusk uses dnsmasq + Caddy to manage .{} domains.", tld);
    println!("   This can coexist with other tools, but may need manual");
    println!("   configuration to avoid conflicts.\n");

    prompt_yn("   Continue with setup?")
}

async fn is_tool_actively_conflicting(binary: &str) -> bool {
    let installed = Command::new("which")
        .arg(binary)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !installed {
        return false;
    }

    match binary {
        "ddev" => Command::new("docker")
            .args(["ps", "--format", "{{.Names}}"])
            .output()
            .await
            .ok()
            .map(|o| {
                o.status.success()
                    && String::from_utf8_lossy(&o.stdout)
                        .lines()
                        .any(|n| n.contains("ddev-router"))
            })
            .unwrap_or(false),
        "herd" => Command::new("pgrep")
            .args(["-f", "[Hh]erd"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false),
        "valet" => Command::new("pgrep")
            .args(["-f", "valet"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false),
        _ => false,
    }
}

fn prompt_tld() -> String {
    let default = Config::tld_or_default();
    loop {
        print!("Which TLD do you want for local domains? [{}] ", default);
        if io::stdout().flush().is_err() {
            return default;
        }
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() {
            return default;
        }
        let input = input.trim().trim_start_matches('.');
        if input.is_empty() {
            return default;
        }
        if crate::core::config::is_valid_tld(input) {
            return input.to_string();
        }
        println!("Invalid TLD: must be alphanumeric and hyphens only (e.g. 'test', 'local')");
    }
}

fn prompt_yn(question: &str) -> bool {
    print!("{} [Y/n] ", question);
    if io::stdout().flush().is_err() {
        return true;
    }
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return true;
    }
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}
