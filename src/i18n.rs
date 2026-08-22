use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// UI language. Stored in `config.toml` as `language = "de"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    De,
    Fr,
    It,
    Sr,
    Ru,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::De => "de",
            Language::Fr => "fr",
            Language::It => "it",
            Language::Sr => "sr",
            Language::Ru => "ru",
        }
    }

    pub fn native_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::De => "Deutsch",
            Language::Fr => "Français",
            Language::It => "Italiano",
            Language::Sr => "Srpski",
            Language::Ru => "Русский",
        }
    }

    pub const ALL: [Language; 6] = [
        Language::En,
        Language::De,
        Language::Fr,
        Language::It,
        Language::Sr,
        Language::Ru,
    ];

    /// Best-effort from `LC_ALL` / `LC_MESSAGES` / `LANG`.
    pub fn from_env() -> Self {
        let raw = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_MESSAGES"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        let code = raw
            .split(['.', '_', '-', '@'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match code.as_str() {
            "de" => Language::De,
            "fr" => Language::Fr,
            "it" => Language::It,
            "sr" => Language::Sr,
            "ru" => Language::Ru,
            _ => Language::En,
        }
    }

    pub fn tr(self, msg: Msg) -> &'static str {
        lookup(self, msg.key())
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl FromStr for Language {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "english" => Ok(Language::En),
            "de" | "german" | "deutsch" => Ok(Language::De),
            "fr" | "french" | "français" | "francais" => Ok(Language::Fr),
            "it" | "italian" | "italiano" => Ok(Language::It),
            "sr" | "serbian" | "srpski" => Ok(Language::Sr),
            "ru" | "russian" | "русский" => Ok(Language::Ru),
            other => anyhow::bail!("unknown language '{}'. Use: en, de, fr, it, sr, ru", other),
        }
    }
}

/// Replace `{name}`-style placeholders.
pub fn fill(template: &str, pairs: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (key, value) in pairs {
        let token = format!("{{{key}}}");
        out = out.replace(&token, value);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    // chrome
    Projects,
    Details,
    Logs,
    Unmanaged,
    Keybindings,
    Confirm,
    ProjectDetails,
    AddProject,
    EditProject,
    NoProjects,
    PressAToAdd,
    OrRunZapuskAdd,
    NoProjectSelected,
    UnmanagedCount,
    // hints
    HintStart,
    HintStop,
    HintRestart,
    HintAdd,
    HintEdit,
    HintDel,
    HintUnmanaged,
    HintHelp,
    HintQuit,
    HintLang,
    HintTheme,
    HintSelect,
    // status words
    StatusStopped,
    StatusStarting,
    StatusRunning,
    StatusFailed,
    StatusPaused,
    OriginManaged,
    OriginAdopted,
    PhaseCaddy,
    PhaseSpawn,
    PhaseVerify,
    TlsOn,
    TlsOff,
    Yes,
    No,
    // field labels
    LabelName,
    LabelDomain,
    LabelAlias,
    LabelAliases,
    LabelPort,
    LabelType,
    LabelTls,
    LabelPath,
    LabelStatus,
    LabelSource,
    LabelPid,
    LabelUptime,
    LabelCommand,
    LabelCmdLine,
    LabelCwd,
    LabelUpstream,
    LabelPhp,
    LabelAutostart,
    LabelDirectory,
    LabelStackGuess,
    LabelComposeFile,
    LabelService,
    LabelProfiles,
    LabelRestart,
    // help sections + lines
    HelpProjects,
    HelpNavigation,
    HelpOther,
    HelpStart,
    HelpStop,
    HelpRestart,
    HelpAdd,
    HelpEdit,
    HelpRemove,
    HelpDetails,
    HelpOpen,
    HelpCopy,
    HelpBadges,
    HelpMove,
    HelpPane,
    HelpScrollLogs,
    HelpJumpLog,
    HelpSearch,
    HelpReloadCaddy,
    HelpUnmanaged,
    HelpToggle,
    HelpQuitSoft,
    HelpQuitHard,
    HelpLanguage,
    LanguagePicker,
    HelpTheme,
    ThemePicker,
    HelpClose,
    DetailRemoveHint,
    DetailCloseHint,
    FormHintSelect,
    FormHintText,
    FormConfigOnly,
    RestartNever,
    RestartOnCrash,
    UnmanagedEmpty,
    UnmanagedFilterHint,
    ActionInspect,
    ActionImport,
    ActionIgnore,
    ActionStack,
    ActionPorts,
    ActionRefresh,
    ActionClose,
    FilterAll,
    FilterDevOnly,
    FilterWeb,
    FilterAllPorts,
    // status / dialogs (templates use {name} {port} {lang} {error} {code} {count})
    Cancelled,
    AddingProject,
    EditingProject,
    LanguageSet,
    ThemeSet,
    ConfirmStop,
    ConfirmRemove,
    NotRunning,
    StopFirst,
    Removed,
    Added,
    Updated,
    Stopped,
    Started,
    StartedVerifying,
    AdoptedConflict,
    Copied,
    ClipboardError,
    BrowserError,
    CaddyReloaded,
    CaddyError,
    CaddyWarning,
    NoCaddySection,
    DetectedRunning,
    Autostarting,
    AutostartError,
    Error,
    NameEmpty,
    DomainEmpty,
    DirEmpty,
    DirNotFound,
    FieldEmpty,
    InvalidPort,
    InvalidPortRange,
    InvalidUpstream,
    ProjectExists,
    PortInUse,
    DuplicateHost,
    DomainUsed,
    DomainTld,
    NoProject,
    ProjectGone,
    StopBeforeEdit,
    SaveFailed,
    RemovedUnsaved,
    AddedUnsaved,
    UpdatedUnsaved,
    DiscoveryFailed,
    ConfigReloadRead,
    ConfigReloadParse,
    ConfigReloaded,
    ConfigReloadRestart,
    ConfigReloadTld,
    ConfigReloadForgotten,
    RecipesReloaded,
    RecipesReloadAdded,
    RecipesReloadRemoved,
    RecipesReloadWarn,
    UnmanagedHint,
    UnmanagedNoneSelected,
    ImportNoCwd,
    Imported,
    ImportSaveFailed,
    Ignored,
    IgnoreSaveFailed,
    FilterAllStacks,
    FilterDevStacks,
    FilterWebPorts,
    FilterAllPortsMsg,
    SoftQuit,
    ForceQuit,
    ForceQuitDone,
    StepEnsuringCaddy,
    StepVerifying,
    StartFailed,
    DomainReachable,
    DomainCheckFailed,
    DomainReachableStatus,
    DomainCheckFailedStatus,
    Crashed,
    CrashedRestarting,
    RestartGaveUp,
    ComposeFileMissing,
    PhpVersionEmpty,
}

impl Msg {
    pub fn key(self) -> &'static str {
        match self {
            Msg::Projects => "projects",
            Msg::Details => "details",
            Msg::Logs => "logs",
            Msg::Unmanaged => "unmanaged",
            Msg::Keybindings => "keybindings",
            Msg::Confirm => "confirm",
            Msg::ProjectDetails => "project_details",
            Msg::AddProject => "add_project",
            Msg::EditProject => "edit_project",
            Msg::NoProjects => "no_projects",
            Msg::PressAToAdd => "press_a_to_add",
            Msg::OrRunZapuskAdd => "or_run_zapusk_add",
            Msg::NoProjectSelected => "no_project_selected",
            Msg::UnmanagedCount => "unmanaged_count",
            Msg::HintStart => "hint_start",
            Msg::HintStop => "hint_stop",
            Msg::HintRestart => "hint_restart",
            Msg::HintAdd => "hint_add",
            Msg::HintEdit => "hint_edit",
            Msg::HintDel => "hint_del",
            Msg::HintUnmanaged => "hint_unmanaged",
            Msg::HintHelp => "hint_help",
            Msg::HintQuit => "hint_quit",
            Msg::HintLang => "hint_lang",
            Msg::HintTheme => "hint_theme",
            Msg::HintSelect => "hint_select",
            Msg::StatusStopped => "status_stopped",
            Msg::StatusStarting => "status_starting",
            Msg::StatusRunning => "status_running",
            Msg::StatusFailed => "status_failed",
            Msg::StatusPaused => "status_paused",
            Msg::OriginManaged => "origin_managed",
            Msg::OriginAdopted => "origin_adopted",
            Msg::PhaseCaddy => "phase_caddy",
            Msg::PhaseSpawn => "phase_spawn",
            Msg::PhaseVerify => "phase_verify",
            Msg::TlsOn => "tls_on",
            Msg::TlsOff => "tls_off",
            Msg::Yes => "yes",
            Msg::No => "no",
            Msg::LabelName => "label_name",
            Msg::LabelDomain => "label_domain",
            Msg::LabelAlias => "label_alias",
            Msg::LabelAliases => "label_aliases",
            Msg::LabelPort => "label_port",
            Msg::LabelType => "label_type",
            Msg::LabelTls => "label_tls",
            Msg::LabelPath => "label_path",
            Msg::LabelStatus => "label_status",
            Msg::LabelSource => "label_source",
            Msg::LabelPid => "label_pid",
            Msg::LabelUptime => "label_uptime",
            Msg::LabelCommand => "label_command",
            Msg::LabelCmdLine => "label_cmd_line",
            Msg::LabelCwd => "label_cwd",
            Msg::LabelUpstream => "label_upstream",
            Msg::LabelPhp => "label_php",
            Msg::LabelAutostart => "label_autostart",
            Msg::LabelDirectory => "label_directory",
            Msg::LabelStackGuess => "label_stack_guess",
            Msg::LabelComposeFile => "label_compose_file",
            Msg::LabelService => "label_service",
            Msg::LabelProfiles => "label_profiles",
            Msg::LabelRestart => "label_restart",
            Msg::HelpProjects => "help_projects",
            Msg::HelpNavigation => "help_navigation",
            Msg::HelpOther => "help_other",
            Msg::HelpStart => "help_start",
            Msg::HelpStop => "help_stop",
            Msg::HelpRestart => "help_restart",
            Msg::HelpAdd => "help_add",
            Msg::HelpEdit => "help_edit",
            Msg::HelpRemove => "help_remove",
            Msg::HelpDetails => "help_details",
            Msg::HelpOpen => "help_open",
            Msg::HelpCopy => "help_copy",
            Msg::HelpBadges => "help_badges",
            Msg::HelpMove => "help_move",
            Msg::HelpPane => "help_pane",
            Msg::HelpScrollLogs => "help_scroll_logs",
            Msg::HelpJumpLog => "help_jump_log",
            Msg::HelpSearch => "help_search",
            Msg::HelpReloadCaddy => "help_reload_caddy",
            Msg::HelpUnmanaged => "help_unmanaged",
            Msg::HelpToggle => "help_toggle",
            Msg::HelpQuitSoft => "help_quit_soft",
            Msg::HelpQuitHard => "help_quit_hard",
            Msg::HelpLanguage => "help_language",
            Msg::LanguagePicker => "language_picker",
            Msg::HelpTheme => "help_theme",
            Msg::ThemePicker => "theme_picker",
            Msg::HelpClose => "help_close",
            Msg::DetailRemoveHint => "detail_remove_hint",
            Msg::DetailCloseHint => "detail_close_hint",
            Msg::FormHintSelect => "form_hint_select",
            Msg::FormHintText => "form_hint_text",
            Msg::FormConfigOnly => "form_config_only",
            Msg::RestartNever => "restart_never",
            Msg::RestartOnCrash => "restart_on_crash",
            Msg::UnmanagedEmpty => "unmanaged_empty",
            Msg::UnmanagedFilterHint => "unmanaged_filter_hint",
            Msg::ActionInspect => "action_inspect",
            Msg::ActionImport => "action_import",
            Msg::ActionIgnore => "action_ignore",
            Msg::ActionStack => "action_stack",
            Msg::ActionPorts => "action_ports",
            Msg::ActionRefresh => "action_refresh",
            Msg::ActionClose => "action_close",
            Msg::FilterAll => "filter_all",
            Msg::FilterDevOnly => "filter_dev_only",
            Msg::FilterWeb => "filter_web",
            Msg::FilterAllPorts => "filter_all_ports",
            Msg::Cancelled => "cancelled",
            Msg::AddingProject => "adding_project",
            Msg::EditingProject => "editing_project",
            Msg::LanguageSet => "language_set",
            Msg::ThemeSet => "theme_set",
            Msg::ConfirmStop => "confirm_stop",
            Msg::ConfirmRemove => "confirm_remove",
            Msg::NotRunning => "not_running",
            Msg::StopFirst => "stop_first",
            Msg::Removed => "removed",
            Msg::Added => "added",
            Msg::Updated => "updated",
            Msg::Stopped => "stopped",
            Msg::Started => "started",
            Msg::StartedVerifying => "started_verifying",
            Msg::AdoptedConflict => "adopted_conflict",
            Msg::Copied => "copied",
            Msg::ClipboardError => "clipboard_error",
            Msg::BrowserError => "browser_error",
            Msg::CaddyReloaded => "caddy_reloaded",
            Msg::CaddyError => "caddy_error",
            Msg::CaddyWarning => "caddy_warning",
            Msg::NoCaddySection => "no_caddy_section",
            Msg::DetectedRunning => "detected_running",
            Msg::Autostarting => "autostarting",
            Msg::AutostartError => "autostart_error",
            Msg::Error => "error",
            Msg::NameEmpty => "name_empty",
            Msg::DomainEmpty => "domain_empty",
            Msg::DirEmpty => "dir_empty",
            Msg::DirNotFound => "dir_not_found",
            Msg::FieldEmpty => "field_empty",
            Msg::InvalidPort => "invalid_port",
            Msg::InvalidPortRange => "invalid_port_range",
            Msg::InvalidUpstream => "invalid_upstream",
            Msg::ProjectExists => "project_exists",
            Msg::PortInUse => "port_in_use",
            Msg::DuplicateHost => "duplicate_host",
            Msg::DomainUsed => "domain_used",
            Msg::DomainTld => "domain_tld",
            Msg::NoProject => "no_project",
            Msg::ProjectGone => "project_gone",
            Msg::StopBeforeEdit => "stop_before_edit",
            Msg::SaveFailed => "save_failed",
            Msg::RemovedUnsaved => "removed_unsaved",
            Msg::AddedUnsaved => "added_unsaved",
            Msg::UpdatedUnsaved => "updated_unsaved",
            Msg::DiscoveryFailed => "discovery_failed",
            Msg::ConfigReloadRead => "config_reload_read",
            Msg::ConfigReloadParse => "config_reload_parse",
            Msg::ConfigReloaded => "config_reloaded",
            Msg::ConfigReloadRestart => "config_reload_restart",
            Msg::ConfigReloadTld => "config_reload_tld",
            Msg::ConfigReloadForgotten => "config_reload_forgotten",
            Msg::RecipesReloaded => "recipes_reloaded",
            Msg::RecipesReloadAdded => "recipes_reload_added",
            Msg::RecipesReloadRemoved => "recipes_reload_removed",
            Msg::RecipesReloadWarn => "recipes_reload_warn",
            Msg::UnmanagedHint => "unmanaged_hint",
            Msg::UnmanagedNoneSelected => "unmanaged_none_selected",
            Msg::ImportNoCwd => "import_no_cwd",
            Msg::Imported => "imported",
            Msg::ImportSaveFailed => "import_save_failed",
            Msg::Ignored => "ignored",
            Msg::IgnoreSaveFailed => "ignore_save_failed",
            Msg::FilterAllStacks => "filter_all_stacks",
            Msg::FilterDevStacks => "filter_dev_stacks",
            Msg::FilterWebPorts => "filter_web_ports",
            Msg::FilterAllPortsMsg => "filter_all_ports_msg",
            Msg::SoftQuit => "soft_quit",
            Msg::ForceQuit => "force_quit",
            Msg::ForceQuitDone => "force_quit_done",
            Msg::StepEnsuringCaddy => "step_ensuring_caddy",
            Msg::StepVerifying => "step_verifying",
            Msg::StartFailed => "start_failed",
            Msg::DomainReachable => "domain_reachable",
            Msg::DomainCheckFailed => "domain_check_failed",
            Msg::DomainReachableStatus => "domain_reachable_status",
            Msg::DomainCheckFailedStatus => "domain_check_failed_status",
            Msg::Crashed => "crashed",
            Msg::CrashedRestarting => "crashed_restarting",
            Msg::RestartGaveUp => "restart_gave_up",
            Msg::ComposeFileMissing => "compose_file_missing",
            Msg::PhpVersionEmpty => "php_version_empty",
        }
    }
}

fn lookup(lang: Language, key: &str) -> &'static str {
    if let Some(s) = catalog(lang).get(key) {
        return s.as_str();
    }
    if lang != Language::En {
        if let Some(s) = catalog(Language::En).get(key) {
            return s.as_str();
        }
    }
    ""
}

fn catalog(lang: Language) -> &'static std::collections::HashMap<String, String> {
    match lang {
        Language::En => EN.get_or_init(|| load("en", include_str!("../locales/en.toml"))),
        Language::De => DE.get_or_init(|| load("de", include_str!("../locales/de.toml"))),
        Language::Fr => FR.get_or_init(|| load("fr", include_str!("../locales/fr.toml"))),
        Language::It => IT.get_or_init(|| load("it", include_str!("../locales/it.toml"))),
        Language::Sr => SR.get_or_init(|| load("sr", include_str!("../locales/sr.toml"))),
        Language::Ru => RU.get_or_init(|| load("ru", include_str!("../locales/ru.toml"))),
    }
}

static EN: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();
static DE: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();
static FR: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();
static IT: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();
static SR: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();
static RU: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();

fn load(code: &str, builtin: &str) -> std::collections::HashMap<String, String> {
    let mut map: std::collections::HashMap<String, String> = toml::from_str(builtin)
        .unwrap_or_else(|e| {
            eprintln!("zapusk: failed to parse builtin locales/{code}.toml: {e}");
            std::collections::HashMap::new()
        });
    let overlay = crate::core::config::config_path()
        .parent()
        .map(|p| p.join("locales").join(format!("{code}.toml")));
    if let Some(path) = overlay {
        if let Ok(text) = std::fs::read_to_string(&path) {
            match toml::from_str::<std::collections::HashMap<String, String>>(&text) {
                Ok(extra) => map.extend(extra),
                Err(e) => eprintln!("zapusk: {}: {e}", path.display()),
            }
        }
    }
    map
}

#[cfg(test)]
const ALL_MSGS: &[Msg] = &[
    Msg::Projects,
    Msg::Details,
    Msg::Logs,
    Msg::Unmanaged,
    Msg::Keybindings,
    Msg::Confirm,
    Msg::ProjectDetails,
    Msg::AddProject,
    Msg::EditProject,
    Msg::NoProjects,
    Msg::PressAToAdd,
    Msg::OrRunZapuskAdd,
    Msg::NoProjectSelected,
    Msg::UnmanagedCount,
    Msg::HintStart,
    Msg::HintStop,
    Msg::HintRestart,
    Msg::HintAdd,
    Msg::HintEdit,
    Msg::HintDel,
    Msg::HintUnmanaged,
    Msg::HintHelp,
    Msg::HintQuit,
    Msg::HintLang,
    Msg::HintTheme,
    Msg::HintSelect,
    Msg::StatusStopped,
    Msg::StatusStarting,
    Msg::StatusRunning,
    Msg::StatusFailed,
    Msg::StatusPaused,
    Msg::OriginManaged,
    Msg::OriginAdopted,
    Msg::PhaseCaddy,
    Msg::PhaseSpawn,
    Msg::PhaseVerify,
    Msg::TlsOn,
    Msg::TlsOff,
    Msg::Yes,
    Msg::No,
    Msg::LabelName,
    Msg::LabelDomain,
    Msg::LabelAlias,
    Msg::LabelAliases,
    Msg::LabelPort,
    Msg::LabelType,
    Msg::LabelTls,
    Msg::LabelPath,
    Msg::LabelStatus,
    Msg::LabelSource,
    Msg::LabelPid,
    Msg::LabelUptime,
    Msg::LabelCommand,
    Msg::LabelCmdLine,
    Msg::LabelCwd,
    Msg::LabelUpstream,
    Msg::LabelPhp,
    Msg::LabelAutostart,
    Msg::LabelDirectory,
    Msg::LabelStackGuess,
    Msg::LabelComposeFile,
    Msg::LabelService,
    Msg::LabelProfiles,
    Msg::LabelRestart,
    Msg::HelpProjects,
    Msg::HelpNavigation,
    Msg::HelpOther,
    Msg::HelpStart,
    Msg::HelpStop,
    Msg::HelpRestart,
    Msg::HelpAdd,
    Msg::HelpEdit,
    Msg::HelpRemove,
    Msg::HelpDetails,
    Msg::HelpOpen,
    Msg::HelpCopy,
    Msg::HelpBadges,
    Msg::HelpMove,
    Msg::HelpPane,
    Msg::HelpScrollLogs,
    Msg::HelpJumpLog,
    Msg::HelpSearch,
    Msg::HelpReloadCaddy,
    Msg::HelpUnmanaged,
    Msg::HelpToggle,
    Msg::HelpQuitSoft,
    Msg::HelpQuitHard,
    Msg::HelpLanguage,
    Msg::LanguagePicker,
    Msg::HelpTheme,
    Msg::ThemePicker,
    Msg::HelpClose,
    Msg::DetailRemoveHint,
    Msg::DetailCloseHint,
    Msg::FormHintSelect,
    Msg::FormHintText,
    Msg::FormConfigOnly,
    Msg::RestartNever,
    Msg::RestartOnCrash,
    Msg::UnmanagedEmpty,
    Msg::UnmanagedFilterHint,
    Msg::ActionInspect,
    Msg::ActionImport,
    Msg::ActionIgnore,
    Msg::ActionStack,
    Msg::ActionPorts,
    Msg::ActionRefresh,
    Msg::ActionClose,
    Msg::FilterAll,
    Msg::FilterDevOnly,
    Msg::FilterWeb,
    Msg::FilterAllPorts,
    Msg::Cancelled,
    Msg::AddingProject,
    Msg::EditingProject,
    Msg::LanguageSet,
    Msg::ThemeSet,
    Msg::ConfirmStop,
    Msg::ConfirmRemove,
    Msg::NotRunning,
    Msg::StopFirst,
    Msg::Removed,
    Msg::Added,
    Msg::Updated,
    Msg::Stopped,
    Msg::Started,
    Msg::StartedVerifying,
    Msg::AdoptedConflict,
    Msg::Copied,
    Msg::ClipboardError,
    Msg::BrowserError,
    Msg::CaddyReloaded,
    Msg::CaddyError,
    Msg::CaddyWarning,
    Msg::NoCaddySection,
    Msg::DetectedRunning,
    Msg::Autostarting,
    Msg::AutostartError,
    Msg::Error,
    Msg::NameEmpty,
    Msg::DomainEmpty,
    Msg::DirEmpty,
    Msg::DirNotFound,
    Msg::FieldEmpty,
    Msg::InvalidPort,
    Msg::InvalidPortRange,
    Msg::InvalidUpstream,
    Msg::ProjectExists,
    Msg::PortInUse,
    Msg::DuplicateHost,
    Msg::DomainUsed,
    Msg::DomainTld,
    Msg::NoProject,
    Msg::ProjectGone,
    Msg::StopBeforeEdit,
    Msg::SaveFailed,
    Msg::RemovedUnsaved,
    Msg::AddedUnsaved,
    Msg::UpdatedUnsaved,
    Msg::DiscoveryFailed,
    Msg::ConfigReloadRead,
    Msg::ConfigReloadParse,
    Msg::ConfigReloaded,
    Msg::ConfigReloadRestart,
    Msg::ConfigReloadTld,
    Msg::ConfigReloadForgotten,
    Msg::RecipesReloaded,
    Msg::RecipesReloadAdded,
    Msg::RecipesReloadRemoved,
    Msg::RecipesReloadWarn,
    Msg::UnmanagedHint,
    Msg::UnmanagedNoneSelected,
    Msg::ImportNoCwd,
    Msg::Imported,
    Msg::ImportSaveFailed,
    Msg::Ignored,
    Msg::IgnoreSaveFailed,
    Msg::FilterAllStacks,
    Msg::FilterDevStacks,
    Msg::FilterWebPorts,
    Msg::FilterAllPortsMsg,
    Msg::SoftQuit,
    Msg::ForceQuit,
    Msg::ForceQuitDone,
    Msg::StepEnsuringCaddy,
    Msg::StepVerifying,
    Msg::StartFailed,
    Msg::DomainReachable,
    Msg::DomainCheckFailed,
    Msg::DomainReachableStatus,
    Msg::DomainCheckFailedStatus,
    Msg::Crashed,
    Msg::CrashedRestarting,
    Msg::RestartGaveUp,
    Msg::ComposeFileMissing,
    Msg::PhpVersionEmpty,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_message_has_english_toml_key() {
        let en = catalog(Language::En);
        for msg in ALL_MSGS {
            assert!(
                en.contains_key(msg.key()),
                "locales/en.toml missing key {}",
                msg.key()
            );
            assert!(
                !en[msg.key()].is_empty(),
                "empty en string for {}",
                msg.key()
            );
        }
    }

    #[test]
    fn shipped_locales_cover_all_english_keys() {
        let en = catalog(Language::En);
        for lang in [
            Language::De,
            Language::Fr,
            Language::It,
            Language::Sr,
            Language::Ru,
        ] {
            let cat = catalog(lang);
            for key in en.keys() {
                assert!(cat.contains_key(key), "{:?} missing key {}", lang, key);
            }
        }
    }

    #[test]
    fn fill_replaces_placeholders() {
        assert_eq!(
            fill(
                "Stop {name} on {port}",
                &[("name", "api"), ("port", "3000")]
            ),
            "Stop api on 3000"
        );
    }

    #[test]
    fn language_roundtrip() {
        assert_eq!("de".parse::<Language>().unwrap(), Language::De);
        assert_eq!("fr".parse::<Language>().unwrap(), Language::Fr);
        assert_eq!("it".parse::<Language>().unwrap(), Language::It);
        assert_eq!("sr".parse::<Language>().unwrap(), Language::Sr);
        assert_eq!("ru".parse::<Language>().unwrap(), Language::Ru);
        assert_eq!(Language::ALL[0], Language::En);
        assert_eq!(Language::ALL[1], Language::De);
        assert_eq!(*Language::ALL.last().unwrap(), Language::Ru);
    }

    #[test]
    fn missing_key_falls_back_to_english_then_empty() {
        assert_eq!(lookup(Language::De, "projects"), "Projekte");
        assert_eq!(lookup(Language::De, "definitely_missing_key"), "");
    }
}
