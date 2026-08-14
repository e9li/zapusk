pub mod caddy;
pub mod config;
pub mod discovery;
pub mod docker;
pub mod framework;
pub mod manager;
pub mod project;

/// Convert a project name to a valid domain-safe slug.
/// Replaces underscores, spaces, and non-alphanumeric chars with hyphens,
/// lowercases, and trims leading/trailing hyphens.
///
/// Examples: "project_one" → "project-one", "My App!" → "my-app"
pub fn slugify(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse multiple hyphens and trim
    let mut result = String::new();
    let mut prev_hyphen = true; // treat start as hyphen to trim leading
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Trim trailing hyphen
    if result.ends_with('-') {
        result.pop();
    }

    result
}
