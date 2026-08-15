use anyhow::Result;
use std::io::Write;

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    #[cfg(not(target_os = "macos"))]
    let mut child = std::process::Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        write!(stdin, "{}", text)?;
    }
    child.wait()?;
    Ok(())
}

/// Open a URL in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg(url)
        .spawn()?
        .wait()?;

    #[cfg(not(target_os = "macos"))]
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()?
        .wait()?;

    Ok(())
}

/// Resolve the PHP binary and return `(path, notes)`.
/// Notes are non-empty when a fallback was used — suitable for logging to the project log.
pub fn php_binary_resolved(version: Option<&str>) -> (String, Vec<String>) {
    match version {
        #[cfg(target_os = "macos")]
        Some(v) => {
            let arm = format!("/opt/homebrew/opt/php@{}/bin/php", v);
            let intel = format!("/usr/local/opt/php@{}/bin/php", v);
            if std::path::Path::new(&arm).exists() {
                (arm, vec![])
            } else if std::path::Path::new(&intel).exists() {
                (
                    intel.clone(),
                    vec![format!(
                        "php@{}: arm64 Homebrew path not found, using Intel path: {}",
                        v, intel
                    )],
                )
            } else {
                (
                    "php".into(),
                    vec![format!(
                        "php@{}: not found in Homebrew (/opt/homebrew or /usr/local), falling back to system `php` from PATH",
                        v
                    )],
                )
            }
        }
        #[cfg(not(target_os = "macos"))]
        Some(v) => (format!("php{}", v), vec![]),
        None => ("php".into(), vec![]),
    }
}

/// Path to the dnsmasq config file.
pub fn dnsmasq_config_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "/opt/homebrew/etc/dnsmasq.conf"
    } else {
        "/etc/dnsmasq.conf"
    }
}

/// PHP binary path used in doctor checks.
pub fn php_install_path(version: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("/opt/homebrew/opt/php@{}/bin/php", version)
    } else {
        format!("/usr/bin/php{}", version)
    }
}
