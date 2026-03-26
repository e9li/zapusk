use anyhow::Result;
use std::io::{self, Write};
use std::path::Path;
use tokio::process::Command;

use crate::cli::spinner::Spinner;
use crate::core::config::{config_path, Config};
use crate::platform;

pub async fn run() -> Result<()> {
    println!("zapusk destroy");
    println!("==============\n");
    println!("This will undo the changes made by `zapusk init`.");
    println!("It will NOT uninstall Caddy or dnsmasq — only remove zapusk's configuration.\n");

    if !prompt_yn("Are you sure you want to continue?") {
        println!("\nAborted. Nothing was changed.");
        return Ok(());
    }

    let tld = Config::tld_or_default();
    let config = Config::load().ok();
    let mut removed = 0;

    // Step 1: Remove Caddyfile
    if let Some(ref cfg) = config {
        if let Some(ref caddy) = cfg.caddy {
            removed += step_remove_caddyfile(&caddy.config_path).await;
        }
    }

    // Step 2: Remove dnsmasq entry
    removed += step_remove_dnsmasq_entry(&tld).await;

    // Step 3: Remove macOS resolver file
    if cfg!(target_os = "macos") {
        removed += step_remove_resolver(&tld).await;
    }

    // Step 4: Remove config directory
    removed += step_remove_config().await;

    println!();
    if removed > 0 {
        println!("Done. Removed {} item(s).", removed);
        println!("\nYou may also want to:");
        println!("  sudo brew services stop dnsmasq  Stop dnsmasq if no longer needed");
        println!("  brew remove dnsmasq            Uninstall dnsmasq");
        println!("  brew remove caddy              Uninstall Caddy");
    } else {
        println!("Nothing to remove — zapusk was already clean.");
    }

    println!("\nGoodbye!");
    Ok(())
}

async fn step_remove_caddyfile(caddyfile_path: &str) -> usize {
    let path = Path::new(caddyfile_path);
    if !path.exists() {
        println!("\n[1/4] Caddyfile — not found, skipping");
        return 0;
    }

    println!("\n[1/4] Caddyfile at {}", caddyfile_path);
    if !prompt_yn("      Remove it?") {
        return 0;
    }

    match std::fs::remove_file(path) {
        Ok(()) => {
            println!("      \u{2713} Removed");
            let sp = Spinner::start("Stopping Caddy...");
            let _ = Command::new("caddy").arg("stop").output().await;
            sp.done("Caddy stopped").await;
            1
        }
        Err(e) => {
            println!("      \u{2717} Could not remove: {}", e);
            0
        }
    }
}

async fn step_remove_dnsmasq_entry(tld: &str) -> usize {
    let config_path = platform::dnsmasq_config_path();
    let entry = format!("address=/.{}/127.0.0.1", tld);

    let content = match std::fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(_) => {
            println!(
                "\n[2/4] dnsmasq config — could not read {}, skipping",
                config_path
            );
            return 0;
        }
    };

    if !content.contains(&entry) {
        println!("\n[2/4] dnsmasq config — no zapusk entry found, skipping");
        return 0;
    }

    println!("\n[2/4] dnsmasq config contains: {}", entry);
    if !prompt_yn("      Remove it?") {
        return 0;
    }

    // Remove the entry and the "# Added by zapusk" comment above it
    let cleaned: Vec<&str> = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed != entry && trimmed != "# Added by zapusk"
        })
        .collect();
    let cleaned = cleaned.join("\n") + "\n";

    match std::fs::write(config_path, &cleaned) {
        Ok(()) => {
            println!("      \u{2713} Entry removed from {}", config_path);
            let sp = Spinner::start("Restarting dnsmasq...");
            if cfg!(target_os = "macos") {
                let _ = Command::new("sudo")
                    .args(["brew", "services", "restart", "dnsmasq"])
                    .output()
                    .await;
            } else {
                let _ = Command::new("sudo")
                    .args(["systemctl", "restart", "dnsmasq"])
                    .output()
                    .await;
            }
            sp.done("dnsmasq restarted").await;
            1
        }
        Err(e) => {
            println!("      \u{2717} Could not write: {} — try with sudo", e);
            0
        }
    }
}

async fn step_remove_resolver(tld: &str) -> usize {
    let resolver_file = format!("/etc/resolver/{}", tld);
    let path = Path::new(&resolver_file);

    if !path.exists() {
        println!(
            "\n[3/4] macOS resolver — {} not found, skipping",
            resolver_file
        );
        return 0;
    }

    println!("\n[3/4] macOS resolver at {}", resolver_file);
    if !prompt_yn("      Remove it? (requires sudo)") {
        return 0;
    }

    let status = Command::new("sudo")
        .args(["rm", &resolver_file])
        .status()
        .await;

    match status {
        Ok(s) if s.success() => {
            println!("      \u{2713} Removed");
            1
        }
        _ => {
            println!(
                "      \u{2717} Failed — remove it manually: sudo rm {}",
                resolver_file
            );
            0
        }
    }
}

async fn step_remove_config() -> usize {
    let path = config_path();
    let config_dir = path.parent().unwrap_or(Path::new("."));

    if !config_dir.exists() {
        println!("\n[4/4] Config directory — not found, skipping");
        return 0;
    }

    println!("\n[4/4] Config directory at {}", config_dir.display());
    println!("      This contains your config.toml and any generated files.");
    if !prompt_yn("      Remove the entire directory?") {
        return 0;
    }

    match std::fs::remove_dir_all(config_dir) {
        Ok(()) => {
            println!("      \u{2713} Removed {}", config_dir.display());
            1
        }
        Err(e) => {
            println!("      \u{2717} Could not remove: {}", e);
            0
        }
    }
}

fn prompt_yn(question: &str) -> bool {
    print!("{} [y/N] ", question);
    if io::stdout().flush().is_err() {
        return false;
    }
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    let input = input.trim().to_lowercase();
    input == "y" || input == "yes"
}
