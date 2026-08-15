use ratatui::style::Color;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use crate::core::config::ThemeConfig;

const BUILTIN_TOMLS: &[(&str, &str)] = &[
    ("groknight", include_str!("../themes/groknight.toml")),
    ("terminal", include_str!("../themes/terminal.toml")),
    ("nightfox", include_str!("../themes/nightfox.toml")),
    ("catppuccin", include_str!("../themes/catppuccin.toml")),
    ("macintosh", include_str!("../themes/macintosh.toml")),
    (
        "macintosh-dark",
        include_str!("../themes/macintosh-dark.toml"),
    ),
];

/// Resolved TUI palette. All slots are concrete ratatui colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub bg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub text: Color,
    pub text_dim: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    pub highlight_bg: Color,
    pub highlight_fg: Color,
}

impl Theme {
    /// GrokNight — same values as `src/themes/groknight.toml`.
    pub const DEFAULT: Theme = Theme {
        bg: Color::Rgb(20, 20, 20),
        border: Color::Rgb(65, 65, 65),
        border_focus: Color::Rgb(187, 154, 247),
        text: Color::Rgb(225, 225, 225),
        text_dim: Color::Rgb(108, 108, 108),
        accent: Color::Rgb(187, 154, 247),
        ok: Color::Rgb(158, 206, 106),
        warn: Color::Rgb(224, 175, 104),
        err: Color::Rgb(247, 118, 142),
        highlight_bg: Color::Rgb(36, 36, 36),
        highlight_fg: Color::Rgb(225, 225, 225),
    };

    /// Named theme (builtin or `~/.config/zapusk/themes/`) plus optional slot overrides.
    pub fn resolve(cfg: Option<&ThemeConfig>) -> Self {
        let name = cfg
            .and_then(|c| c.name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(canonical_theme_id)
            .unwrap_or_else(|| "groknight".into());
        let mut theme = load_named(&name).unwrap_or_else(|| {
            if name != "groknight" {
                eprintln!(
                    "zapusk: unknown theme '{name}'. Using groknight. \
                     Add ~/.config/zapusk/themes/{name}.toml"
                );
            }
            Self::DEFAULT
        });
        if let Some(c) = cfg {
            theme.apply_overrides(c);
        }
        theme
    }

    fn apply_overrides(&mut self, c: &ThemeConfig) {
        let d = Self::DEFAULT;
        let slot =
            |raw: Option<&str>, fallback: Color| raw.and_then(parse_color).unwrap_or(fallback);
        // Only override slots the user actually set.
        if c.bg.is_some() {
            self.bg = slot(c.bg.as_deref(), d.bg);
        }
        if c.border.is_some() {
            self.border = slot(c.border.as_deref(), d.border);
        }
        if c.border_focus.is_some() {
            self.border_focus = slot(c.border_focus.as_deref(), d.border_focus);
        }
        if c.text.is_some() {
            self.text = slot(c.text.as_deref(), d.text);
        }
        if c.text_dim.is_some() {
            self.text_dim = slot(c.text_dim.as_deref(), d.text_dim);
        }
        if c.accent.is_some() {
            self.accent = slot(c.accent.as_deref(), d.accent);
        }
        if c.ok.is_some() {
            self.ok = slot(c.ok.as_deref(), d.ok);
        }
        if c.warn.is_some() {
            self.warn = slot(c.warn.as_deref(), d.warn);
        }
        if c.err.is_some() {
            self.err = slot(c.err.as_deref(), d.err);
        }
        if c.highlight_bg.is_some() {
            self.highlight_bg = slot(c.highlight_bg.as_deref(), d.highlight_bg);
        }
        if c.highlight_fg.is_some() {
            self.highlight_fg = slot(c.highlight_fg.as_deref(), d.highlight_fg);
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ThemeFile {
    id: String,
    #[serde(default)]
    label: Option<String>,
    bg: Option<String>,
    border: Option<String>,
    border_focus: Option<String>,
    text: Option<String>,
    text_dim: Option<String>,
    accent: Option<String>,
    ok: Option<String>,
    warn: Option<String>,
    err: Option<String>,
    highlight_bg: Option<String>,
    highlight_fg: Option<String>,
}

impl ThemeFile {
    fn into_theme(self) -> Theme {
        let d = Theme::DEFAULT;
        let slot =
            |raw: Option<&str>, fallback: Color| raw.and_then(parse_color).unwrap_or(fallback);
        let text = slot(self.text.as_deref(), d.text);
        Theme {
            bg: slot(self.bg.as_deref(), d.bg),
            border: slot(self.border.as_deref(), d.border),
            border_focus: slot(self.border_focus.as_deref(), d.border_focus),
            text,
            text_dim: slot(self.text_dim.as_deref(), d.text_dim),
            accent: slot(self.accent.as_deref(), d.accent),
            ok: slot(self.ok.as_deref(), d.ok),
            warn: slot(self.warn.as_deref(), d.warn),
            err: slot(self.err.as_deref(), d.err),
            highlight_bg: slot(self.highlight_bg.as_deref(), d.highlight_bg),
            highlight_fg: slot(self.highlight_fg.as_deref(), text),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSource {
    Builtin,
    User,
}

impl ThemeSource {
    pub fn label(self) -> &'static str {
        match self {
            ThemeSource::Builtin => "builtin",
            ThemeSource::User => "user",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ThemeMeta {
    pub id: String,
    pub label: String,
    pub source: ThemeSource,
}

/// Builtins plus `~/.config/zapusk/themes/*.toml`. User files with the same
/// `id` replace a builtin.
pub fn discover_themes() -> Vec<ThemeMeta> {
    let (files, _) = load_catalog();
    let mut metas: Vec<ThemeMeta> = files
        .into_iter()
        .map(|(id, (file, source))| ThemeMeta {
            label: file.label.clone().unwrap_or_else(|| id.clone()),
            id,
            source,
        })
        .collect();
    metas.sort_by(|a, b| a.id.cmp(&b.id));
    metas
}

fn canonical_theme_id(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "cappucine" | "cappuccin" | "catppucin" | "catppuccin-mocha" => "catppuccin".into(),
        "mac1984" | "macintosh-1984" | "classic-mac" | "macintosh-light" => "macintosh".into(),
        "mac1984-dark" | "macintosh-1984-dark" | "classic-mac-dark" => "macintosh-dark".into(),
        other => other.to_string(),
    }
}

fn load_named(name: &str) -> Option<Theme> {
    let key = name.to_ascii_lowercase();
    let (files, _) = load_catalog();
    files.get(&key).map(|(file, _)| file.clone().into_theme())
}

fn load_catalog() -> (HashMap<String, (ThemeFile, ThemeSource)>, Vec<String>) {
    let mut files = HashMap::new();
    let mut warnings = Vec::new();

    for (id, src) in BUILTIN_TOMLS {
        match toml::from_str::<ThemeFile>(src) {
            Ok(mut file) => {
                file.id = (*id).to_string();
                files.insert((*id).to_string(), (file, ThemeSource::Builtin));
            }
            Err(e) => warnings.push(format!("builtin theme {id} failed to parse: {e}")),
        }
    }

    let dir = themes_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return (files, warnings),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        match toml::from_str::<ThemeFile>(&text) {
            Ok(mut file) => {
                if file.id.trim().is_empty() {
                    file.id = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("theme")
                        .to_ascii_lowercase();
                } else {
                    file.id = file.id.trim().to_ascii_lowercase();
                }
                files.insert(file.id.clone(), (file, ThemeSource::User));
            }
            Err(e) => warnings.push(format!("{}: {e}", path.display())),
        }
    }

    let _ = warnings;
    (files, warnings)
}

pub fn themes_dir() -> PathBuf {
    crate::core::config::config_path()
        .parent()
        .map(|p| p.join("themes"))
        .unwrap_or_else(|| PathBuf::from("themes"))
}

pub fn ensure_themes_dir() -> anyhow::Result<PathBuf> {
    let dir = themes_dir();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Parse a color string: "#rrggbb" hex, named ANSI colors, or `reset`.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some(Color::Rgb(r, g, b));
        }
        return None;
    }
    match s.to_lowercase().as_str() {
        "reset" | "default" | "none" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "darkgrey" => Some(Color::DarkGray),
        "lightred" => Some(Color::LightRed),
        "lightgreen" => Some(Color::LightGreen),
        "lightyellow" => Some(Color::LightYellow),
        "lightblue" => Some(Color::LightBlue),
        "lightmagenta" => Some(Color::LightMagenta),
        "lightcyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => None,
    }
}

static THEME: RwLock<Option<Theme>> = RwLock::new(None);

/// Apply a theme. Safe to call again on config hot-reload.
pub fn init_theme(cfg: Option<&ThemeConfig>) {
    if let Ok(mut slot) = THEME.write() {
        *slot = Some(Theme::resolve(cfg));
    }
}

pub(super) fn t() -> Theme {
    THEME
        .read()
        .ok()
        .and_then(|guard| *guard)
        .unwrap_or(Theme::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_builtin(id: &str) -> Theme {
        let src = BUILTIN_TOMLS
            .iter()
            .find(|(i, _)| *i == id)
            .unwrap_or_else(|| panic!("missing builtin {id}"))
            .1;
        toml::from_str::<ThemeFile>(src)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .into_theme()
    }

    #[test]
    fn groknight_builtin_matches_default() {
        assert_eq!(parse_builtin("groknight"), Theme::DEFAULT);
    }

    #[test]
    fn terminal_uses_reset_for_canvas_and_text() {
        let theme = parse_builtin("terminal");
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.text, Color::Reset);
        assert_eq!(theme.accent, Color::Magenta);
        assert_eq!(theme.ok, Color::Green);
    }

    #[test]
    fn parse_color_accepts_hex_named_and_reset() {
        assert_eq!(parse_color("#141414"), Some(Color::Rgb(20, 20, 20)));
        assert_eq!(parse_color("reset"), Some(Color::Reset));
        assert_eq!(parse_color("DEFAULT"), Some(Color::Reset));
        assert_eq!(parse_color("magenta"), Some(Color::Magenta));
        assert!(parse_color("not-a-color").is_none());
    }

    #[test]
    fn unknown_name_falls_back_to_default() {
        let theme = Theme::resolve(Some(&ThemeConfig {
            name: Some("does-not-exist".into()),
            ..ThemeConfig::default()
        }));
        assert_eq!(theme, Theme::DEFAULT);
    }

    #[test]
    fn named_theme_accepts_slot_overrides() {
        let mut theme = parse_builtin("terminal");
        theme.apply_overrides(&ThemeConfig {
            accent: Some("#ff00aa".into()),
            ..ThemeConfig::default()
        });
        assert_eq!(theme.bg, Color::Reset);
        assert_eq!(theme.accent, Color::Rgb(255, 0, 170));
    }

    #[test]
    fn builtins_are_discoverable() {
        let ids: Vec<_> = BUILTIN_TOMLS.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&"groknight"));
        assert!(ids.contains(&"terminal"));
        assert!(ids.contains(&"nightfox"));
        assert!(ids.contains(&"catppuccin"));
        assert!(ids.contains(&"macintosh"));
        assert!(ids.contains(&"macintosh-dark"));
        for (id, src) in BUILTIN_TOMLS {
            let file: ThemeFile = toml::from_str(src).expect(id);
            assert_eq!(file.id, *id);
        }
    }

    #[test]
    fn nightfox_and_catppuccin_use_official_accents() {
        let nightfox = parse_builtin("nightfox");
        assert_eq!(nightfox.bg, Color::Rgb(0x19, 0x23, 0x30));
        assert_eq!(nightfox.accent, Color::Rgb(0x9d, 0x79, 0xd6));
        assert_eq!(nightfox.ok, Color::Rgb(0x81, 0xb2, 0x9a));

        let cat = parse_builtin("catppuccin");
        assert_eq!(cat.bg, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(cat.accent, Color::Rgb(0xcb, 0xa6, 0xf7));
        assert_eq!(cat.err, Color::Rgb(0xf3, 0x8b, 0xa8));
    }

    #[test]
    fn macintosh_light_and_dark_invert_selection() {
        let light = parse_builtin("macintosh");
        assert_eq!(light.bg, Color::Rgb(0xc9, 0xb8, 0x96));
        assert_eq!(light.text, Color::Rgb(0x2a, 0x24, 0x1c));
        assert_eq!(light.highlight_bg, Color::Rgb(0x2a, 0x24, 0x1c));
        assert_eq!(light.highlight_fg, Color::Rgb(0xc9, 0xb8, 0x96));

        let dark = parse_builtin("macintosh-dark");
        assert_eq!(dark.bg, Color::Rgb(0x2c, 0x26, 0x1c));
        assert_eq!(dark.text, Color::Rgb(0xd8, 0xcc, 0xb4));
        assert_eq!(dark.highlight_bg, Color::Rgb(0xd8, 0xcc, 0xb4));
        assert_eq!(dark.highlight_fg, Color::Rgb(0x2c, 0x26, 0x1c));

        assert_eq!(canonical_theme_id("mac1984"), "macintosh");
        assert_eq!(canonical_theme_id("macintosh-1984-dark"), "macintosh-dark");
    }
}
