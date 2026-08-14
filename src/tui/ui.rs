use chrono::Local;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use std::sync::OnceLock;

use super::app::{ActivePane, AddField, AddForm, App, ConfirmAction, EditForm, ServiceState};
use crate::core::config::ThemeConfig;
use crate::core::discovery::ServiceInfo;
use crate::core::project::{ProcessOrigin, Project, ProjectStatus};
use crate::i18n::Msg;

// ── Theme ──────────────────────────────────────────────────────────────────

pub struct Theme {
    pub border: Color,
    pub border_focus: Color,
    pub text: Color,
    pub text_dim: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
    pub highlight_bg: Color,
}

impl Theme {
    pub const DEFAULT: Theme = Theme {
        border: Color::Gray,
        border_focus: Color::LightGreen,
        text: Color::White,
        text_dim: Color::DarkGray,
        accent: Color::LightCyan,
        ok: Color::LightGreen,
        warn: Color::Yellow,
        err: Color::LightRed,
        highlight_bg: Color::Black,
    };

    pub fn from_config(cfg: Option<&ThemeConfig>) -> Self {
        let d = &Self::DEFAULT;
        let Some(c) = cfg else {
            return Self::DEFAULT;
        };
        Self {
            border: c
                .border
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(d.border),
            border_focus: c
                .border_focus
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(d.border_focus),
            text: c.text.as_deref().and_then(parse_color).unwrap_or(d.text),
            text_dim: c
                .text_dim
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(d.text_dim),
            accent: c
                .accent
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(d.accent),
            ok: c.ok.as_deref().and_then(parse_color).unwrap_or(d.ok),
            warn: c.warn.as_deref().and_then(parse_color).unwrap_or(d.warn),
            err: c.err.as_deref().and_then(parse_color).unwrap_or(d.err),
            highlight_bg: c
                .highlight_bg
                .as_deref()
                .and_then(parse_color)
                .unwrap_or(d.highlight_bg),
        }
    }
}

/// Parse a color string: "#rrggbb" hex or named terminal colors.
fn parse_color(s: &str) -> Option<Color> {
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

static THEME: OnceLock<Theme> = OnceLock::new();

/// Initialize the global theme. Call once at startup from App::new().
pub fn init_theme(cfg: Option<&ThemeConfig>) {
    let _ = THEME.set(Theme::from_config(cfg));
}

/// Get the active theme.
fn t() -> &'static Theme {
    THEME.get_or_init(|| Theme::DEFAULT)
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn make_block(title: &str, focused: bool) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Plain
        })
        .border_style(if focused {
            Style::default().fg(t().border_focus)
        } else {
            Style::default().fg(t().border)
        })
}

fn make_popup_block(title: &str, accent: Color) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent))
}

fn detail_row(label: &str, value: &str, value_color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<10}", label), Style::default().fg(t().text_dim)),
        Span::styled(value.to_string(), Style::default().fg(value_color)),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn origin_badge(origin: Option<&ProcessOrigin>) -> Span<'static> {
    match origin {
        Some(ProcessOrigin::Managed) => Span::styled(" [M]", Style::default().fg(t().ok)),
        Some(ProcessOrigin::Adopted) => Span::styled(" [A]", Style::default().fg(t().warn)),
        None => Span::styled(" [ ]", Style::default().fg(t().border)),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars().take(max.saturating_sub(1)) {
        out.push(c);
    }
    out.push('\u{2026}');
    out
}

fn log_color(line: &str, is_stderr: bool) -> Color {
    let lower = line.to_lowercase();
    if lower.contains("error") || lower.contains("panic") || lower.contains("fatal") {
        t().err
    } else if lower.contains("warn") {
        t().warn
    } else if lower.contains("debug") || lower.contains("trace") {
        t().text_dim
    } else if is_stderr {
        t().warn
    } else {
        t().text
    }
}

// ── Main draw ──────────────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Root: title line | main | hints line
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // Title bar — right-aligned, no border
    let version = env!("CARGO_PKG_VERSION");
    let title = Paragraph::new(Line::from(vec![
        Span::styled("[ ", Style::default().fg(t().text)),
        Span::styled("ZAPUSK", Style::default().fg(t().accent)).add_modifier(Modifier::BOLD),
        Span::styled(format!(" - v{} ]", version), Style::default().fg(t().text)),
    ]))
    .alignment(Alignment::Right);
    frame.render_widget(title, root[0]);

    // Bottom hints — no border
    let hints: &[(&str, &str)] = &[
        ("s ", app.tr(Msg::HintStart)),
        ("x ", app.tr(Msg::HintStop)),
        ("r ", app.tr(Msg::HintRestart)),
        ("a ", app.tr(Msg::HintAdd)),
        ("e ", app.tr(Msg::HintEdit)),
        ("D ", app.tr(Msg::HintDel)),
        ("u ", app.tr(Msg::HintUnmanaged)),
        ("l ", app.tr(Msg::HintLang)),
        ("? ", app.tr(Msg::HintHelp)),
        ("q ", app.tr(Msg::HintQuit)),
    ];
    let mut hint_spans: Vec<Span> = vec![Span::raw(" ")];
    for (key, label) in hints {
        hint_spans.push(Span::styled(
            format!("[{}\u{2192}{}]", key, label),
            Style::default().fg(t().text_dim),
        ));
        hint_spans.push(Span::raw(" "));
    }
    let hints_bar = Paragraph::new(Line::from(hint_spans));
    frame.render_widget(hints_bar, root[2]);

    // Main: left column | right column
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(root[1]);

    // Left: projects | details | unmanaged
    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(35),
            Constraint::Percentage(25),
        ])
        .split(main[0]);

    // Right: logs | status message | services strip
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(3),
        ])
        .split(main[1]);

    draw_project_list(frame, app, left[0]);
    draw_project_details(frame, app, left[1]);
    draw_unmanaged_compact(frame, app, left[2]);
    draw_logs(frame, app, right[0]);
    draw_status_line(frame, app, right[1]);
    draw_service_strip(frame, app, right[2]);

    // Overlays
    if app.show_detail_popup {
        if let Some(project) = app.projects.get(app.selected) {
            draw_detail_popup(frame, app, project);
        }
    }
    if app.show_help {
        draw_help(frame, app);
    }
    if let Some(form) = &app.add_form {
        draw_add_form(frame, app, form);
    }
    if let Some(form) = &app.edit_form {
        draw_edit_form(frame, app, form);
    }
    if let Some(dialog) = &app.confirm_dialog {
        draw_confirm_dialog(frame, app, dialog.action.clone());
    }
    if app.show_unmanaged_popup {
        draw_unmanaged_popup(frame, app);
        if app.show_unmanaged_detail {
            if let Some(service) = app.selected_unmanaged() {
                draw_unmanaged_detail(frame, app, service);
            }
        }
    }
}

// ── Project list ───────────────────────────────────────────────────────────

fn draw_project_list(frame: &mut Frame, app: &App, area: Rect) {
    let projects_title = format!("{} ({})", app.tr(Msg::Projects), app.projects.len());
    let focused = app.active_pane == ActivePane::ProjectList;
    let block = make_block(&projects_title, focused);

    if app.projects.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::NoProjects)),
                Style::default().fg(t().text_dim),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::PressAToAdd)),
                Style::default().fg(t().text_dim),
            )),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::OrRunZapuskAdd)),
                Style::default().fg(t().text_dim),
            )),
        ])
        .block(block);
        frame.render_widget(empty, area);
        return;
    }

    // Display order groups running projects first, then stopped (see
    // App::display_order). A blank spacer separates the two groups. Spacers are
    // extra list rows, so the ratatui selection index is the selected project's
    // *display* row, not its index in app.projects.
    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_display = 0usize;
    let mut prev_running: Option<bool> = None;

    for idx in app.display_order() {
        let project = &app.projects[idx];
        let running = project.is_running();
        if prev_running == Some(true) && !running {
            // Blank spacer at the running -> stopped boundary.
            items.push(ListItem::new(Line::from("")));
        }
        prev_running = Some(running);

        if idx == app.selected {
            selected_display = items.len();
        }

        let item = {
            let startup_phase = app.startup_phase_label(&project.config.name);
            let (status_icon, status_color) = if startup_phase.is_some() {
                (app.spinner_glyph(), t().warn)
            } else {
                match &project.status {
                    ProjectStatus::Running => ("\u{25cf}", t().ok),
                    ProjectStatus::Starting => (app.spinner_glyph(), t().warn),
                    ProjectStatus::Stopped => ("\u{25cb}", t().text_dim),
                    ProjectStatus::Failed(_) => ("\u{2717}", t().err),
                }
            };

            let mut spans = vec![
                Span::styled(
                    format!("{} ", status_icon),
                    Style::default().fg(status_color),
                ),
                Span::styled(
                    if project.config.name.len() > 16 {
                        format!("{:.13}...", project.config.name)
                    } else {
                        format!("{:<16}", project.config.name)
                    },
                    Style::default().fg(t().text),
                ),
                origin_badge(project.origin.as_ref()),
                Span::styled(
                    format!(" :{}", project.config.port),
                    Style::default().fg(t().text_dim),
                ),
                Span::styled(
                    format!(" {}", project.config.project_type.label()),
                    Style::default().fg(t().accent),
                ),
            ];

            if let Some(phase) = startup_phase {
                spans.push(Span::styled(
                    format!(" {}", phase),
                    Style::default().fg(t().warn),
                ));
            }

            if project.is_running() {
                if let Some(started) = project.started_at {
                    let uptime = Local::now() - started;
                    let total_secs = uptime.num_seconds().max(0);
                    let mins = total_secs / 60;
                    let secs = total_secs % 60;
                    spans.push(Span::styled(
                        format!(" {}m{}s", mins, secs),
                        Style::default().fg(t().text_dim),
                    ));
                }
            }

            ListItem::new(Line::from(spans))
        };
        items.push(item);
    }

    let mut state = ListState::default();
    state.select(Some(selected_display));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(t().highlight_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{2192} ");

    frame.render_stateful_widget(list, area, &mut state);
}

// ── Project details (inline pane) ──────────────────────────────────────────

fn draw_project_details(frame: &mut Frame, app: &App, area: Rect) {
    let block = make_block(app.tr(Msg::Details), false);

    let Some(project) = app.projects.get(app.selected) else {
        let empty = Paragraph::new(Span::styled(
            format!(" {}", app.tr(Msg::NoProjectSelected)),
            Style::default().fg(t().text_dim),
        ))
        .block(block);
        frame.render_widget(empty, area);
        return;
    };

    let cfg = &project.config;
    let mut lines: Vec<Line> = vec![];

    lines.push(detail_row(app.tr(Msg::LabelDomain), &cfg.domain, t().text));
    for alias in &cfg.aliases {
        lines.push(detail_row(app.tr(Msg::LabelAlias), alias, t().text));
    }
    lines.push(detail_row(
        app.tr(Msg::LabelPort),
        &cfg.port.to_string(),
        t().text,
    ));

    let tls_label = if cfg.tls {
        app.tr(Msg::TlsOn)
    } else {
        app.tr(Msg::TlsOff)
    };
    let tls_color = if cfg.tls { t().ok } else { t().text_dim };
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {:<10}", app.tr(Msg::LabelType)),
            Style::default().fg(t().text_dim),
        ),
        Span::styled(
            cfg.project_type.label().to_string(),
            Style::default().fg(t().accent),
        ),
        Span::styled("  ", Style::default()),
        Span::styled(tls_label.to_string(), Style::default().fg(tls_color)),
    ]));

    lines.push(detail_row(
        app.tr(Msg::LabelPath),
        &truncate(&cfg.path, 28),
        t().text_dim,
    ));

    if let Some(ref cmd) = cfg.command {
        lines.push(detail_row(
            app.tr(Msg::LabelCommand),
            &truncate(cmd, 28),
            t().text_dim,
        ));
    }
    if let Some(ref host) = cfg.upstream_host {
        lines.push(detail_row(app.tr(Msg::LabelUpstream), host, t().text_dim));
    }
    if let Some(ref php) = cfg.php_version {
        lines.push(detail_row(app.tr(Msg::LabelPhp), php, t().text_dim));
    }
    if cfg.autostart {
        lines.push(detail_row(
            app.tr(Msg::LabelAutostart),
            app.tr(Msg::Yes),
            t().ok,
        ));
    }

    let status_color = match &project.status {
        ProjectStatus::Running => t().ok,
        ProjectStatus::Starting => t().warn,
        ProjectStatus::Stopped => t().text_dim,
        ProjectStatus::Failed(_) => t().err,
    };
    let mut status_line = vec![
        Span::styled(
            format!(" {:<10}", app.tr(Msg::LabelStatus)),
            Style::default().fg(t().text_dim),
        ),
        Span::styled(
            app.status_label(&project.status).to_string(),
            Style::default().fg(status_color),
        ),
    ];
    if let Some(ref origin) = project.origin {
        let (tag, label, color) = match origin {
            ProcessOrigin::Managed => ("[M]", app.tr(Msg::OriginManaged), t().ok),
            ProcessOrigin::Adopted => ("[A]", app.tr(Msg::OriginAdopted), t().warn),
        };
        status_line.push(Span::styled(
            format!("  {} {}", tag, label),
            Style::default().fg(color),
        ));
    }
    if let Some(pid) = project.pid {
        status_line.push(Span::styled(
            format!("  pid:{}", pid),
            Style::default().fg(t().text_dim),
        ));
    }
    lines.push(Line::from(status_line));

    if let Some(started) = project.started_at {
        let uptime = Local::now() - started;
        let total_secs = uptime.num_seconds().max(0);
        let mins = total_secs / 60;
        let secs = total_secs % 60;
        lines.push(detail_row(
            app.tr(Msg::LabelUptime),
            &format!("{}m {}s", mins, secs),
            t().text_dim,
        ));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

// ── Logs ───────────────────────────────────────────────────────────────────

fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.active_pane == ActivePane::Logs;

    let title = app
        .projects
        .get(app.selected)
        .map(|p| format!("{}: {}", app.tr(Msg::Logs), p.config.name))
        .unwrap_or_else(|| app.tr(Msg::Logs).into());

    let block = make_block(&title, focused);
    let logs = app.selected_logs();

    let search_height = if app.search_mode || !app.search_query.is_empty() {
        1
    } else {
        0
    };
    let inner_height = area.height.saturating_sub(2 + search_height) as usize;
    let inner_width = area.width.saturating_sub(2) as usize;
    let ts_width = 9usize; // "HH:MM:SS "

    // Approximate visual rows each log entry occupies after wrapping
    let visual_rows_of = |char_count: usize| -> usize {
        let w = ts_width + char_count;
        (w + inner_width.saturating_sub(1)) / inner_width.max(1)
    };

    let total_visual: usize = logs
        .iter()
        .map(|e| visual_rows_of(e.line.chars().count()))
        .sum();

    let offset_visual: usize = logs
        .iter()
        .rev()
        .take(app.log_scroll_offset)
        .map(|e| visual_rows_of(e.line.chars().count()))
        .sum();

    // Anchor the bottom of the content to the bottom of the pane, adjusted by scroll
    let scroll_row = total_visual
        .saturating_sub(inner_height)
        .saturating_sub(offset_visual) as u16;

    let mut text: Vec<Line> = logs
        .iter()
        .map(|entry| {
            let color = log_color(&entry.line, entry.is_stderr);
            let ts = entry.timestamp.format("%H:%M:%S").to_string();
            Line::from(vec![
                Span::styled(format!("{} ", ts), Style::default().fg(t().text_dim)),
                Span::styled(entry.line.clone(), Style::default().fg(color)),
            ])
        })
        .collect();

    if app.search_mode {
        text.push(Line::from(vec![
            Span::styled("/", Style::default().fg(t().warn)),
            Span::styled(app.search_query.clone(), Style::default().fg(t().text)),
            Span::styled("\u{2588}", Style::default().fg(t().warn)),
        ]));
    } else if !app.search_query.is_empty() {
        text.push(Line::from(vec![
            Span::styled(
                format!("filter: {} ", app.search_query),
                Style::default().fg(t().warn),
            ),
            Span::styled(
                "(/ to edit, Esc to clear)",
                Style::default().fg(t().text_dim),
            ),
        ]));
    }

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_row, 0));
    frame.render_widget(paragraph, area);
}

// ── Status line (between logs and services) ────────────────────────────────

fn draw_status_line(frame: &mut Frame, app: &App, area: Rect) {
    let message = app.status_message.as_deref().unwrap_or("");
    let mut spans = vec![Span::styled(
        format!(" {} ", message),
        Style::default().fg(t().warn),
    )];

    let unmanaged_count = app.unmanaged_services.len();
    if unmanaged_count > 0 {
        spans.push(Span::styled(
            format!(
                " {} ",
                app.trf(
                    Msg::UnmanagedCount,
                    &[("count", &unmanaged_count.to_string())]
                )
            ),
            Style::default().fg(Color::Black).bg(t().warn),
        ));
    }

    let bar = Paragraph::new(Line::from(spans));
    frame.render_widget(bar, area);
}

// ── Unmanaged compact ──────────────────────────────────────────────────────

fn draw_unmanaged_compact(frame: &mut Frame, app: &App, area: Rect) {
    let block = make_block(app.tr(Msg::Unmanaged), false);

    if app.unmanaged_all_services.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::NonePressU)),
                Style::default().fg(t().text_dim),
            )),
        ])
        .block(block);
        frame.render_widget(p, area);
        return;
    }

    let visible = area.height.saturating_sub(2) as usize;
    let rows: Vec<Line> = app
        .unmanaged_all_services
        .iter()
        .take(visible.max(1))
        .map(|s| {
            Line::from(vec![
                Span::styled(format!("{:>5} ", s.port), Style::default().fg(t().accent)),
                Span::styled(
                    format!("{:<7} ", s.stack.label()),
                    Style::default().fg(t().warn),
                ),
                Span::styled(truncate(&s.command, 16), Style::default().fg(t().text)),
            ])
        })
        .collect();

    let p = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

// ── Services strip (horizontal, below logs) ────────────────────────────────

fn draw_service_strip(frame: &mut Frame, app: &App, area: Rect) {
    let block = make_block(app.tr(Msg::Services), false);

    let mut spans = vec![Span::raw("  ")];
    spans.extend(service_indicator("caddy", app.caddy_state, app));
    spans.push(Span::raw("      "));
    spans.extend(service_indicator("dnsmasq", app.dnsmasq_state, app));
    let line = Line::from(spans);

    let p = Paragraph::new(line).block(block);
    frame.render_widget(p, area);
}

fn service_indicator(name: &str, state: ServiceState, app: &App) -> Vec<Span<'static>> {
    let (icon, color, label) = match state {
        ServiceState::Running => ("\u{25cf}", t().ok, app.tr(Msg::StatusRunning)),
        ServiceState::Paused => ("\u{25d0}", t().warn, app.tr(Msg::StatusPaused)),
        ServiceState::Stopped => ("\u{25cb}", t().text_dim, app.tr(Msg::StatusStopped)),
    };
    vec![
        Span::styled(icon.to_string(), Style::default().fg(color)),
        Span::styled(format!(" {} ", name), Style::default().fg(t().text)),
        Span::styled(label.to_string(), Style::default().fg(color)),
    ]
}

// ── Detail popup ───────────────────────────────────────────────────────────

fn draw_detail_popup(frame: &mut Frame, app: &App, project: &Project) {
    let area = centered_rect(50, 50, frame.area());
    frame.render_widget(Clear, area);

    let uptime = project
        .started_at
        .map(|s| {
            let d = Local::now() - s;
            let total_secs = d.num_seconds().max(0);
            format!("{}m {}s", total_secs / 60, total_secs % 60)
        })
        .unwrap_or_else(|| "-".into());

    let origin = match project.origin.as_ref() {
        Some(ProcessOrigin::Managed) => app.tr(Msg::OriginManaged),
        Some(ProcessOrigin::Adopted) => app.tr(Msg::OriginAdopted),
        None => "-",
    };
    let mut text = vec![
        Line::from(format!(
            "  {}:    {}",
            app.tr(Msg::LabelName),
            project.config.name
        )),
        Line::from(format!(
            "  {}:  {}",
            app.tr(Msg::LabelDomain),
            project.config.domain
        )),
    ];
    for alias in &project.config.aliases {
        text.push(Line::from(format!(
            "  {}:   {}",
            app.tr(Msg::LabelAlias),
            alias
        )));
    }
    text.extend(vec![
        Line::from(format!(
            "  {}:    {}",
            app.tr(Msg::LabelPort),
            project.config.port
        )),
        Line::from(format!(
            "  {}:    {}",
            app.tr(Msg::LabelType),
            project.config.project_type.label()
        )),
        Line::from(format!(
            "  {}:    {}",
            app.tr(Msg::LabelPath),
            project.config.path
        )),
        Line::from(format!(
            "  {}:  {}",
            app.tr(Msg::LabelStatus),
            app.status_label(&project.status)
        )),
        Line::from(format!("  {}:  {}", app.tr(Msg::LabelSource), origin)),
        Line::from(format!(
            "  {}:     {}",
            app.tr(Msg::LabelPid),
            project.pid.map_or("-".into(), |p| p.to_string())
        )),
        Line::from(format!("  {}:  {}", app.tr(Msg::LabelUptime), uptime)),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::DetailRemoveHint)),
            Style::default().fg(t().text_dim),
        )),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::DetailCloseHint)),
            Style::default().fg(t().text_dim),
        )),
    ]);

    let popup = Paragraph::new(text)
        .block(make_popup_block(app.tr(Msg::ProjectDetails), t().accent))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

// ── Help popup ─────────────────────────────────────────────────────────────

fn draw_help(frame: &mut Frame, app: &App) {
    let area = centered_rect(55, 70, frame.area());
    frame.render_widget(Clear, area);

    let key_style = Style::default().fg(t().accent).add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(t().text);
    let section_style = Style::default().fg(t().warn).add_modifier(Modifier::BOLD);

    let entries: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::HelpProjects)),
            section_style,
        )),
        help_line("  s", app.tr(Msg::HelpStart), key_style, desc_style),
        help_line("  x", app.tr(Msg::HelpStop), key_style, desc_style),
        help_line("  r", app.tr(Msg::HelpRestart), key_style, desc_style),
        help_line("  a", app.tr(Msg::HelpAdd), key_style, desc_style),
        help_line("  e", app.tr(Msg::HelpEdit), key_style, desc_style),
        help_line("  D / Del", app.tr(Msg::HelpRemove), key_style, desc_style),
        help_line("  d", app.tr(Msg::HelpDetails), key_style, desc_style),
        help_line("  o", app.tr(Msg::HelpOpen), key_style, desc_style),
        help_line("  c", app.tr(Msg::HelpCopy), key_style, desc_style),
        help_line("  [M]/[A]", app.tr(Msg::HelpBadges), key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::HelpNavigation)),
            section_style,
        )),
        help_line("  j/k", app.tr(Msg::HelpMove), key_style, desc_style),
        help_line("  Tab", app.tr(Msg::HelpPane), key_style, desc_style),
        help_line(
            "  PgUp/PgDn",
            app.tr(Msg::HelpScrollLogs),
            key_style,
            desc_style,
        ),
        help_line("  G", app.tr(Msg::HelpJumpLog), key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::HelpOther)),
            section_style,
        )),
        help_line("  /", app.tr(Msg::HelpSearch), key_style, desc_style),
        help_line("  R", app.tr(Msg::HelpReloadCaddy), key_style, desc_style),
        help_line("  u", app.tr(Msg::HelpUnmanaged), key_style, desc_style),
        help_line("  l", app.tr(Msg::HelpLanguage), key_style, desc_style),
        help_line("  ?", app.tr(Msg::HelpToggle), key_style, desc_style),
        help_line("  q", app.tr(Msg::HelpQuitSoft), key_style, desc_style),
        help_line("  Q", app.tr(Msg::HelpQuitHard), key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::HelpClose)),
            Style::default().fg(t().text_dim),
        )),
    ];

    let popup = Paragraph::new(entries)
        .block(make_popup_block(app.tr(Msg::Keybindings), t().accent))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

fn help_line<'a>(key: &'a str, desc: &'a str, key_style: Style, desc_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<14}", key), key_style),
        Span::styled(desc, desc_style),
    ])
}

// ── Add form popup ─────────────────────────────────────────────────────────

fn draw_add_form(frame: &mut Frame, app: &App, form: &AddForm) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let type_active = matches!(form.field, AddField::Type);
    let tls_active = matches!(form.field, AddField::Tls);

    let text_fields: &[(&str, &str, bool)] = &[
        (
            app.tr(Msg::LabelName),
            &form.name,
            matches!(form.field, AddField::Name),
        ),
        (
            app.tr(Msg::LabelDomain),
            &form.domain,
            matches!(form.field, AddField::Domain),
        ),
        (
            app.tr(Msg::LabelAliases),
            &form.aliases,
            matches!(form.field, AddField::Aliases),
        ),
        (
            app.tr(Msg::LabelPort),
            &form.port,
            matches!(form.field, AddField::Port),
        ),
        (
            app.tr(Msg::LabelUpstream),
            &form.upstream_host,
            matches!(form.field, AddField::UpstreamHost),
        ),
    ];

    let mut lines = vec![Line::from("")];

    for &(label, value, active) in text_fields {
        let (label_color, value_str) = if active {
            (t().accent, format!("{}\u{2588}", value))
        } else if value.is_empty() {
            (t().text_dim, "-".into())
        } else {
            (t().text_dim, value.to_string())
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10}", label), Style::default().fg(label_color)),
            Span::styled(value_str, Style::default().fg(t().text)),
        ]));
    }

    // Type selector
    let label_color = if type_active {
        t().accent
    } else {
        t().text_dim
    };
    let mut type_spans = vec![Span::styled(
        format!("  {:<10}", app.tr(Msg::LabelType)),
        Style::default().fg(label_color),
    )];
    let options = &form.type_ids;
    for (i, opt) in options.iter().enumerate() {
        if i == form.type_index {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(Color::Black).bg(t().accent),
            ));
        } else {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(if type_active { t().text } else { t().text_dim }),
            ));
        }
    }
    lines.push(Line::from(type_spans));

    // TLS selector
    let tls_label_color = if tls_active { t().accent } else { t().text_dim };
    let mut tls_spans = vec![Span::styled(
        format!("  {:<10}", app.tr(Msg::LabelTls)),
        Style::default().fg(tls_label_color),
    )];
    for (is_on, label) in [(false, "off"), (true, "on")] {
        if form.tls == is_on {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Black).bg(t().accent),
            ));
        } else {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(if tls_active { t().text } else { t().text_dim }),
            ));
        }
    }
    lines.push(Line::from(tls_spans));

    // Path field
    let path_active = matches!(form.field, AddField::Path);
    let (path_label_color, path_str) = if path_active {
        (t().accent, format!("{}\u{2588}", form.path))
    } else if form.path.is_empty() {
        (t().text_dim, "-".into())
    } else {
        (t().text_dim, form.path.clone())
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<10}", app.tr(Msg::LabelDirectory)),
            Style::default().fg(path_label_color),
        ),
        Span::styled(path_str, Style::default().fg(t().text)),
    ]));

    if let Some(error) = app.add_form_error(form) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ! {}", error),
            Style::default().fg(t().err),
        )));
    }

    lines.push(Line::from(""));
    let hint = if type_active || tls_active {
        app.tr(Msg::FormHintSelect)
    } else {
        app.tr(Msg::FormHintText)
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(t().text_dim),
    )));

    let popup = Paragraph::new(lines)
        .block(make_popup_block(app.tr(Msg::AddProject), t().accent))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

// ── Edit form popup ────────────────────────────────────────────────────────

fn draw_edit_form(frame: &mut Frame, app: &App, form: &EditForm) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let type_active = matches!(form.field, AddField::Type);
    let tls_active = matches!(form.field, AddField::Tls);

    let text_fields: &[(&str, &str, bool)] = &[
        (
            app.tr(Msg::LabelName),
            &form.name,
            matches!(form.field, AddField::Name),
        ),
        (
            app.tr(Msg::LabelDomain),
            &form.domain,
            matches!(form.field, AddField::Domain),
        ),
        (
            app.tr(Msg::LabelAliases),
            &form.aliases,
            matches!(form.field, AddField::Aliases),
        ),
        (
            app.tr(Msg::LabelPort),
            &form.port,
            matches!(form.field, AddField::Port),
        ),
        (
            app.tr(Msg::LabelUpstream),
            &form.upstream_host,
            matches!(form.field, AddField::UpstreamHost),
        ),
    ];

    let mut lines = vec![Line::from("")];

    for &(label, value, active) in text_fields {
        let (label_color, value_str) = if active {
            (t().accent, format!("{}\u{2588}", value))
        } else if value.is_empty() {
            (t().text_dim, "-".into())
        } else {
            (t().text_dim, value.to_string())
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10}", label), Style::default().fg(label_color)),
            Span::styled(value_str, Style::default().fg(t().text)),
        ]));
    }

    // Type selector
    let label_color = if type_active {
        t().accent
    } else {
        t().text_dim
    };
    let mut type_spans = vec![Span::styled(
        format!("  {:<10}", app.tr(Msg::LabelType)),
        Style::default().fg(label_color),
    )];
    let options = &form.type_ids;
    for (i, opt) in options.iter().enumerate() {
        if i == form.type_index {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(Color::Black).bg(t().accent),
            ));
        } else {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(if type_active { t().text } else { t().text_dim }),
            ));
        }
    }
    lines.push(Line::from(type_spans));

    // TLS selector
    let tls_label_color = if tls_active { t().accent } else { t().text_dim };
    let mut tls_spans = vec![Span::styled(
        format!("  {:<10}", app.tr(Msg::LabelTls)),
        Style::default().fg(tls_label_color),
    )];
    for (is_on, label) in [(false, "off"), (true, "on")] {
        if form.tls == is_on {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Black).bg(t().accent),
            ));
        } else {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(if tls_active { t().text } else { t().text_dim }),
            ));
        }
    }
    lines.push(Line::from(tls_spans));

    // Path field
    let path_active = matches!(form.field, AddField::Path);
    let (path_label_color, path_str) = if path_active {
        (t().accent, format!("{}\u{2588}", form.path))
    } else if form.path.is_empty() {
        (t().text_dim, "-".into())
    } else {
        (t().text_dim, form.path.clone())
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<10}", app.tr(Msg::LabelDirectory)),
            Style::default().fg(path_label_color),
        ),
        Span::styled(path_str, Style::default().fg(t().text)),
    ]));

    if let Some(error) = app.edit_form_error(form) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ! {}", error),
            Style::default().fg(t().err),
        )));
    }

    lines.push(Line::from(""));
    let hint = if type_active || tls_active {
        app.tr(Msg::FormHintSelect)
    } else {
        app.tr(Msg::FormHintText)
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(t().text_dim),
    )));

    let popup = Paragraph::new(lines)
        .block(make_popup_block(app.tr(Msg::EditProject), t().accent))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

// ── Confirm dialog ─────────────────────────────────────────────────────────

fn draw_confirm_dialog(frame: &mut Frame, app: &App, action: ConfirmAction) {
    let area = centered_rect(40, 20, frame.area());
    frame.render_widget(Clear, area);

    let message = match action {
        ConfirmAction::StopProject(name) => app.trf(Msg::ConfirmStop, &[("name", &name)]),
        ConfirmAction::RemoveProject(name) => app.trf(Msg::ConfirmRemove, &[("name", &name)]),
    };

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", message),
            Style::default().fg(t().warn),
        )),
    ];

    let popup = Paragraph::new(text)
        .block(make_popup_block(app.tr(Msg::Confirm), t().warn))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

// ── Unmanaged popup ────────────────────────────────────────────────────────

fn draw_unmanaged_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(75, 65, frame.area());
    frame.render_widget(Clear, area);

    let filter_label = if app.unmanaged_show_unknown {
        app.tr(Msg::FilterAll)
    } else {
        app.tr(Msg::FilterDevOnly)
    };
    let port_label = if app.unmanaged_web_only {
        app.tr(Msg::FilterWeb)
    } else {
        app.tr(Msg::FilterAllPorts)
    };

    let block = make_popup_block(
        &format!(
            "{} [{}|{}] {}/{}",
            app.tr(Msg::Unmanaged),
            filter_label,
            port_label,
            app.unmanaged_services.len(),
            app.unmanaged_all_services.len()
        ),
        t().warn,
    );

    if app.unmanaged_services.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::UnmanagedEmpty)),
                Style::default().fg(t().text_dim),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", app.tr(Msg::UnmanagedFilterHint)),
                Style::default().fg(t().text_dim),
            )),
        ])
        .block(block)
        .wrap(Wrap { trim: false });
        frame.render_widget(empty, area);
        return;
    }

    let mut items: Vec<ListItem> = vec![];
    for service in &app.unmanaged_services {
        let line = Line::from(vec![
            Span::styled(
                format!("{:>5} ", service.port),
                Style::default().fg(t().accent),
            ),
            Span::styled(
                format!("pid {:<7}", service.pid),
                Style::default().fg(t().text_dim),
            ),
            Span::styled(
                format!(" {:<7}", service.stack.label()),
                Style::default().fg(t().warn),
            ),
            Span::styled(
                format!(" {}", service.command),
                Style::default().fg(t().text),
            ),
        ]);
        items.push(ListItem::new(line));
    }

    let hint_style = Style::default().fg(t().text_dim);
    let key_bg = Style::default().fg(Color::Black).bg(t().border);
    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(" Enter ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionInspect)), hint_style),
        Span::styled(" i ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionImport)), hint_style),
        Span::styled(" I ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionIgnore)), hint_style),
        Span::styled(" f ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionStack)), hint_style),
        Span::styled(" w ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionPorts)), hint_style),
        Span::styled(" r ", key_bg),
        Span::styled(format!("{}  ", app.tr(Msg::ActionRefresh)), hint_style),
        Span::styled(" Esc ", key_bg),
        Span::styled(app.tr(Msg::ActionClose), hint_style),
    ])));

    let mut state = ListState::default();
    state.select(Some(app.unmanaged_selected));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(t().highlight_bg)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("\u{2192} ");
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_unmanaged_detail(frame: &mut Frame, app: &App, service: &ServiceInfo) {
    let area = centered_rect(65, 50, frame.area());
    frame.render_widget(Clear, area);

    let text = vec![
        Line::from(format!(
            "  {}:        {}",
            app.tr(Msg::LabelPort),
            service.port
        )),
        Line::from(format!(
            "  {}:         {}",
            app.tr(Msg::LabelPid),
            service.pid
        )),
        Line::from(format!(
            "  {}: {}",
            app.tr(Msg::LabelStackGuess),
            service.stack.label()
        )),
        Line::from(format!(
            "  {}:     {}",
            app.tr(Msg::LabelCommand),
            service.command
        )),
        Line::from(format!(
            "  {}:         {}",
            app.tr(Msg::LabelCwd),
            service.cwd.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "  {}:{}{}",
            app.tr(Msg::LabelCmdLine),
            if service.command_line.is_some() {
                " "
            } else {
                ""
            },
            service.command_line.as_deref().unwrap_or("-")
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", app.tr(Msg::DetailCloseHint)),
            Style::default().fg(t().text_dim),
        )),
    ];

    let popup = Paragraph::new(text)
        .block(make_popup_block(app.tr(Msg::Details), t().warn))
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}
