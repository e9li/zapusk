use chrono::Local;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::app::{ActivePane, AddField, AddForm, App};
use crate::core::project::{Project, ProjectStatus};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Split into: top (main), bottom (status bar)
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    // Split main area into: left (project list), right (logs)
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(root[0]);

    draw_project_list(frame, app, main[0]);
    draw_logs(frame, app, main[1]);
    draw_status_bar(frame, app, root[1]);

    // Overlay: detail popup
    if app.show_detail_popup {
        if let Some(project) = app.projects.get(app.selected) {
            draw_detail_popup(frame, project);
        }
    }

    // Overlay: help
    if app.show_help {
        draw_help(frame);
    }

    // Overlay: add form
    if let Some(form) = &app.add_form {
        draw_add_form(frame, form);
    }

    // Overlay: confirmation dialog
    if let Some(dialog) = &app.confirm_dialog {
        draw_confirm_dialog(frame, &dialog.message);
    }
}

fn draw_project_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.active_pane == ActivePane::ProjectList;

    let block = Block::default()
        .title(" Projects ")
        .borders(Borders::ALL)
        .border_style(focus_style(focused));

    if app.projects.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No projects configured",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Press 'a' to add a project",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "  or run: zapusk add",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|project| {
            let (status_icon, status_color) = match &project.status {
                ProjectStatus::Running => ("\u{25cf}", Color::Green),
                ProjectStatus::Stopped => ("\u{25cb}", Color::DarkGray),
                ProjectStatus::Failed(_) => ("\u{2717}", Color::Red),
            };

            let mut spans = vec![
                Span::styled(
                    format!("{} ", status_icon),
                    Style::default().fg(status_color),
                ),
                Span::styled(
                    format!("{:<16}", project.config.name),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" {}", project.config.project_type.label()),
                    Style::default().fg(Color::Cyan),
                ),
            ];

            // Show port and uptime for running projects
            if project.is_running() {
                spans.push(Span::styled(
                    format!(" :{}", project.config.port),
                    Style::default().fg(Color::DarkGray),
                ));
                if let Some(started) = project.started_at {
                    let uptime = Local::now() - started;
                    let mins = uptime.num_minutes();
                    let secs = uptime.num_seconds() % 60;
                    spans.push(Span::styled(
                        format!(" {}m{}s", mins, secs),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");

    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_logs(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.active_pane == ActivePane::Logs;

    let title = app
        .projects
        .get(app.selected)
        .map(|p| format!(" Logs: {} ", p.config.name))
        .unwrap_or_else(|| " Logs ".into());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(focus_style(focused));

    let logs = app.selected_logs();

    // Reserve one line for search bar if in search mode
    let search_height = if app.search_mode || !app.search_query.is_empty() {
        1
    } else {
        0
    };
    let inner_height = area.height.saturating_sub(2 + search_height) as usize;

    // Scrollable log view
    let total = logs.len();
    let end = total.saturating_sub(app.log_scroll_offset);
    let start = end.saturating_sub(inner_height);

    let mut text: Vec<Line> = logs[start..end]
        .iter()
        .map(|entry| {
            let color = log_color(&entry.line, entry.is_stderr);
            let ts = entry.timestamp.format("%H:%M:%S").to_string();
            Line::from(vec![
                Span::styled(format!("{} ", ts), Style::default().fg(Color::DarkGray)),
                Span::styled(entry.line.clone(), Style::default().fg(color)),
            ])
        })
        .collect();

    // Show search bar
    if app.search_mode {
        text.push(Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::styled(app.search_query.clone(), Style::default().fg(Color::White)),
            Span::styled("\u{2588}", Style::default().fg(Color::Yellow)),
        ]));
    } else if !app.search_query.is_empty() {
        text.push(Line::from(vec![
            Span::styled(
                format!("filter: {} ", app.search_query),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "(/ to edit, Esc to clear)",
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    }

    let paragraph = Paragraph::new(text).block(block).wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let message = app.status_message.as_deref().unwrap_or("");

    let key_style = Style::default().fg(Color::Black).bg(Color::DarkGray);
    let label_style = Style::default().fg(Color::Gray);
    let sep = Span::styled(" ", Style::default());

    let mut spans = vec![Span::styled(
        format!(" {} ", message),
        Style::default().fg(Color::Yellow),
    )];

    // Only show a few essential hints — full list is in ? help
    let keys: &[(&str, &str)] = &[
        ("s", "start"),
        ("x", "stop"),
        ("r", "restart"),
        ("a", "add"),
        ("?", "help"),
        ("q", "quit"),
    ];

    for (key, label) in keys {
        spans.push(sep.clone());
        spans.push(Span::styled(format!(" {} ", key), key_style));
        spans.push(Span::styled(label.to_string(), label_style));
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(Style::default().bg(Color::Black));

    frame.render_widget(bar, area);
}

fn draw_detail_popup(frame: &mut Frame, project: &Project) {
    let area = centered_rect(50, 50, frame.area());
    frame.render_widget(Clear, area);

    let uptime = project
        .started_at
        .map(|s| {
            let d = Local::now() - s;
            format!("{}m {}s", d.num_minutes(), d.num_seconds() % 60)
        })
        .unwrap_or_else(|| "-".into());

    let text = vec![
        Line::from(format!("  Name:    {}", project.config.name)),
        Line::from(format!("  Domain:  {}", project.config.domain)),
        Line::from(format!("  Port:    {}", project.config.port)),
        Line::from(format!(
            "  Type:    {}",
            project.config.project_type.label()
        )),
        Line::from(format!("  Path:    {}", project.config.path)),
        Line::from(format!("  Status:  {}", project.status.label())),
        Line::from(format!(
            "  PID:     {}",
            project.pid.map_or("-".into(), |p| p.to_string())
        )),
        Line::from(format!("  Uptime:  {}", uptime)),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Esc or d to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let popup = Paragraph::new(text)
        .block(
            Block::default()
                .title(" Project Details ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(55, 70, frame.area());
    frame.render_widget(Clear, area);

    let key_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::White);
    let section_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let entries: Vec<Line> = vec![
        Line::from(""),
        Line::from(Span::styled("  Projects", section_style)),
        help_line("  s", "Start selected project", key_style, desc_style),
        help_line("  x", "Stop selected project", key_style, desc_style),
        help_line("  r", "Restart selected project", key_style, desc_style),
        help_line("  a", "Add a new project", key_style, desc_style),
        help_line("  D", "Remove selected project", key_style, desc_style),
        help_line("  d", "Show project details", key_style, desc_style),
        help_line("  o", "Open in browser", key_style, desc_style),
        help_line("  c", "Copy domain to clipboard", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Navigation", section_style)),
        help_line("  j/k", "Move up/down", key_style, desc_style),
        help_line(
            "  Tab",
            "Switch pane (projects/logs)",
            key_style,
            desc_style,
        ),
        help_line("  PgUp/PgDn", "Scroll logs", key_style, desc_style),
        help_line("  G", "Jump to latest log", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("  Other", section_style)),
        help_line("  /", "Search/filter logs", key_style, desc_style),
        help_line("  R", "Reload Caddy config", key_style, desc_style),
        help_line("  ?", "Toggle this help", key_style, desc_style),
        help_line("  q", "Quit", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled(
            "  Press Esc or ? to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let popup = Paragraph::new(entries)
        .block(
            Block::default()
                .title(" Keybindings ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

fn help_line<'a>(key: &'a str, desc: &'a str, key_style: Style, desc_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{:<14}", key), key_style),
        Span::styled(desc, desc_style),
    ])
}

fn draw_add_form(frame: &mut Frame, form: &AddForm) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let type_active = matches!(form.field, AddField::Type);

    // Text fields
    let text_fields: &[(&str, &str, bool)] = &[
        ("Name", &form.name, matches!(form.field, AddField::Name)),
        (
            "Domain",
            &form.domain,
            matches!(form.field, AddField::Domain),
        ),
        ("Port", &form.port, matches!(form.field, AddField::Port)),
    ];

    let mut lines = vec![Line::from("")];

    for &(label, value, active) in text_fields {
        let (label_color, value_str) = if active {
            (Color::Cyan, format!("{}\u{2588}", value))
        } else if value.is_empty() {
            (Color::DarkGray, "-".into())
        } else {
            (Color::DarkGray, value.to_string())
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<10}", label), Style::default().fg(label_color)),
            Span::styled(value_str, Style::default().fg(Color::White)),
        ]));
    }

    // Type field — selector with all options shown
    let label_color = if type_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let mut type_spans = vec![Span::styled(
        format!("  {:<10}", "Type"),
        Style::default().fg(label_color),
    )];
    let options = super::app::TYPE_OPTIONS;
    for (i, opt) in options.iter().enumerate() {
        if i == form.type_index {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
        } else {
            type_spans.push(Span::styled(
                format!(" {} ", opt),
                Style::default().fg(if type_active {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ));
        }
    }
    lines.push(Line::from(type_spans));

    // Path field
    let path_active = matches!(form.field, AddField::Path);
    let (path_label_color, path_str) = if path_active {
        (Color::Cyan, format!("{}\u{2588}", form.path))
    } else if form.path.is_empty() {
        (Color::DarkGray, "-".into())
    } else {
        (Color::DarkGray, form.path.clone())
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("  {:<10}", "Directory"),
            Style::default().fg(path_label_color),
        ),
        Span::styled(path_str, Style::default().fg(Color::White)),
    ]));

    lines.push(Line::from(""));
    let hint = if type_active {
        "  \u{2190}/\u{2192} select type  Enter: next  Esc: cancel"
    } else {
        "  Enter: next field  Esc: cancel"
    };
    lines.push(Line::from(Span::styled(
        hint,
        Style::default().fg(Color::DarkGray),
    )));

    let popup = Paragraph::new(lines)
        .block(
            Block::default()
                .title(" Add Project ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
}

fn draw_confirm_dialog(frame: &mut Frame, message: &str) {
    let area = centered_rect(40, 20, frame.area());
    frame.render_widget(Clear, area);

    let text = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", message),
            Style::default().fg(Color::Yellow),
        )),
    ];

    let popup = Paragraph::new(text)
        .block(
            Block::default()
                .title(" Confirm ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow)),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(popup, area);
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

fn log_color(line: &str, is_stderr: bool) -> Color {
    let lower = line.to_lowercase();
    if lower.contains("error") || lower.contains("panic") || lower.contains("fatal") {
        Color::Red
    } else if lower.contains("warn") {
        Color::Yellow
    } else if lower.contains("debug") || lower.contains("trace") {
        Color::DarkGray
    } else if is_stderr {
        Color::Yellow
    } else {
        Color::Reset
    }
}

fn focus_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}
