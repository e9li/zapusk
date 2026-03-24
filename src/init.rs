use anyhow::Result;
use std::io::{self, Write};
use std::path::Path;
use tokio::process::Command;

use crate::caddy;
use crate::config::Config;
use crate::doctor;

pub async fn run() -> Result<()> {
    println!("Welcome to zapusk!");
    println!("Let us make sure your local dev stack is ready.\n");

    step_caddy().await?;
    step_dnsmasq_install().await?;
    step_dnsmasq_config().await?;
    step_dnsmasq_start().await?;
    step_caddyfile().await?;

    // Final doctor check
    println!("\n--- Final check ---");
    doctor::run().await?;

    println!("\nSetup complete. Run `zapusk` to open the TUI.");
    Ok(())
}

async fn step_caddy() -> Result<()> {
    println!("[1/5] Checking Caddy...");
    let check = doctor::check_caddy().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Install Caddy via Homebrew?") {
            println!("      Running: brew install caddy");
            let status = Command::new("brew")
                .args(["install", "caddy"])
                .status()
                .await?;
            if status.success() {
                println!("      \u{2713} Caddy installed");
            } else {
                println!("      \u{2717} Installation failed — install manually");
            }
        }
    } else {
        println!("      Install Caddy: https://caddyserver.com/docs/install");
        println!("      Then re-run `zapusk init`");
    }
    Ok(())
}

async fn step_dnsmasq_install() -> Result<()> {
    println!("\n[2/5] Checking dnsmasq...");
    let check = doctor::check_dnsmasq_installed().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Install dnsmasq via Homebrew?") {
            println!("      Running: brew install dnsmasq");
            let status = Command::new("brew")
                .args(["install", "dnsmasq"])
                .status()
                .await?;
            if status.success() {
                println!("      \u{2713} dnsmasq installed");
            } else {
                println!("      \u{2717} Installation failed");
            }
        }
    } else {
        println!("      Run: sudo apt install dnsmasq");
        println!("      Then re-run `zapusk init`");
    }
    Ok(())
}

async fn step_dnsmasq_config() -> Result<()> {
    println!("\n[3/5] Configuring dnsmasq for *.test...");
    let check = doctor::check_dnsmasq_config().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
    } else {
        let config_path = if cfg!(target_os = "macos") {
            "/opt/homebrew/etc/dnsmasq.conf"
        } else {
            "/etc/dnsmasq.conf"
        };

        if prompt_yn(&format!("      Add address=/.test/127.0.0.1 to {}?", config_path)) {
            let content = format!("\n# Added by zapusk\naddress=/.test/127.0.0.1\n");
            match std::fs::OpenOptions::new()
                .append(true)
                .open(config_path)
            {
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
        let resolver_path = Path::new("/etc/resolver/test");
        if resolver_path.exists() {
            println!("      \u{2713} /etc/resolver/test exists");
        } else if prompt_yn("      Create /etc/resolver/test? (requires sudo)") {
            let status = Command::new("sudo")
                .args([
                    "bash",
                    "-c",
                    "mkdir -p /etc/resolver && echo 'nameserver 127.0.0.1' > /etc/resolver/test",
                ])
                .status()
                .await?;
            if status.success() {
                println!("      \u{2713} /etc/resolver/test created");
            } else {
                println!("      \u{2717} Failed — create it manually");
            }
        }
    }

    Ok(())
}

async fn step_dnsmasq_start() -> Result<()> {
    println!("\n[4/5] Starting dnsmasq...");
    let check = doctor::check_dnsmasq_running().await;
    if check.ok {
        println!("      \u{2713} {}", check.detail);
        return Ok(());
    }

    println!("      \u{2717} {}", check.detail);
    if cfg!(target_os = "macos") {
        if prompt_yn("      Start dnsmasq via Homebrew?") {
            println!("      Running: brew services start dnsmasq");
            let status = Command::new("brew")
                .args(["services", "start", "dnsmasq"])
                .status()
                .await?;
            if status.success() {
                println!("      \u{2713} dnsmasq started");
            } else {
                println!("      \u{2717} Failed to start");
            }
        }
    } else {
        println!("      Run: sudo systemctl start dnsmasq");
    }
    Ok(())
}

async fn step_caddyfile() -> Result<()> {
    println!("\n[5/5] Generating Caddyfile from config...");
    match Config::load() {
        Ok(config) => {
            println!("      \u{2713} Config found at {:?}", crate::config::config_path());
            if let Some(ref caddy_cfg) = config.caddy {
                let projects: Vec<_> = config.projects.clone();
                if prompt_yn("      Write Caddyfile and reload Caddy?") {
                    match caddy::write_and_reload(&projects, caddy_cfg).await {
                        Ok(()) => println!("      \u{2713} Caddyfile written and Caddy reloaded"),
                        Err(e) => println!("      \u{2717} {}", e),
                    }
                }
            } else {
                println!("      No [caddy] section in config — skipping Caddyfile generation");
            }
        }
        Err(_) => {
            println!(
                "      No config found at {:?}",
                crate::config::config_path()
            );
            println!("      Create one based on config.example.toml, then re-run `zapusk init`");
        }
    }
    Ok(())
}

fn prompt_yn(question: &str) -> bool {
    print!("{} [Y/n] ", question);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}
