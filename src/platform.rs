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
    std::process::Command::new("open").arg(url).spawn()?.wait()?;

    #[cfg(not(target_os = "macos"))]
    std::process::Command::new("xdg-open").arg(url).spawn()?.wait()?;

    Ok(())
}

/// Resolve the PHP binary path for a given version.
/// On macOS, uses Homebrew paths. Falls back to `php` in PATH.
pub fn php_binary_path(version: Option<&str>) -> String {
    match version {
        #[cfg(target_os = "macos")]
        Some(v) => format!("/opt/homebrew/opt/php@{}/bin/php", v),
        #[cfg(not(target_os = "macos"))]
        Some(v) => format!("php{}", v),
        None => "php".into(),
    }
}

/// Default PHP-FPM socket path template (with `{version}` placeholder).
pub fn default_fpm_socket_template() -> &'static str {
    if cfg!(target_os = "macos") {
        "/opt/homebrew/var/run/php/php{version}-fpm.sock"
    } else {
        "/run/php/php{version}-fpm.sock"
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

/// PHP-FPM socket path for a specific version (used in doctor checks).
pub fn php_fpm_socket_path(version: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("/opt/homebrew/var/run/php/php{}-fpm.sock", version)
    } else {
        format!("/run/php/php{}-fpm.sock", version)
    }
}
