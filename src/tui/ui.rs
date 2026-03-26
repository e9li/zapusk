use chrono::Local;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use super::app::{ActivePane, AddField, AddForm, App, EditForm, ServiceState};
use crate::core::discovery::ServiceInfo;
use crate::core::project::{ProcessOrigin, Project, ProjectStatus};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Split into: top (main), bottom (status bar)
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    // Split main area into: left (projects + unmanaged + services), right (logs)
    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(root[0]);

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(58),
            Constraint::Percentage(24),
            Constraint::Percentage(18),
        ])
        .split(main[0]);

    draw_project_list(frame, app, left[0]);
    draw_unmanaged_compact(frame, app, left[1]);
    draw_service_status(frame, app, left[2]);
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
        draw_add_form(frame, app, form);
    }

    // Overlay: edit form
    if let Some(form) = &app.edit_form {
        draw_edit_form(frame, app, form);
    }

    // Overlay: confirmation dialog
    if let Some(dialog) = &app.confirm_dialog {
        draw_confirm_dialog(frame, &dialog.message);
    }

    // Overlay: unmanaged services
    if app.show_unmanaged_popup {
        draw_unmanaged_popup(frame, app);
        if app.show_unmanaged_detail {
            if let Some(service) = app.selected_unmanaged() {
                draw_unmanaged_detail(frame, service);
            }
        }
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
                origin_badge(project.origin.as_ref()),
                Span::styled(
                    format!(" {}", project.config.project_type.label()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    if project.config.tls {
                        " tls:on"
                    } else {
                        " tls:off"
                    },
                    Style::default().fg(if project.config.tls {
                        Color::Green
                    } else {
                        Color::DarkGray
                    }),
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

fn draw_unmanaged_compact(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(
            " Unmanaged ({}) ",
            app.unmanaged_all_services.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    if app.unmanaged_all_services.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  none", Style::default().fg(Color::DarkGray))),
            Line::from(Span::styled(
                "  (press u)",
                Style::default().fg(Color::DarkGray),
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
                Span::styled(format!("{:>5} ", s.port), Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{:<7} ", s.stack.label()),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(truncate(&s.command, 16), Style::default().fg(Color::White)),
            ])
        })
        .collect();

    let p = Paragraph::new(rows).block(block).wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn draw_service_status(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Services ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let lines = vec![
        service_line("caddy", app.caddy_state),
        service_line("dnsmasq", app.dnsmasq_state),
        Line::from(Span::styled(
            "  refreshes automatically",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let p = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(p, area);
}

fn service_line(name: &str, state: ServiceState) -> Line<'static> {
    let (icon, color, label) = match state {
        ServiceState::Running => ("●", Color::Green, "running"),
        ServiceState::Paused => ("◐", Color::Yellow, "paused"),
        ServiceState::Stopped => ("○", Color::DarkGray, "stopped"),
    };

    Line::from(vec![
        Span::styled(format!(" {} ", icon), Style::default().fg(color)),
        Span::styled(format!("{:<8}", name), Style::default().fg(Color::White)),
        Span::styled(label, Style::default().fg(color)),
    ])
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

    spans.push(sep.clone());
    spans.push(Span::styled(
        format!(" U:{} ", app.unmanaged_services.len()),
        Style::default().fg(Color::Black).bg(Color::Yellow),
    ));

    // Only show a few essential hints — full list is in ? help
    let keys: &[(&str, &str)] = &[
        ("s", "start"),
        ("x", "stop"),
        ("r", "restart"),
        ("a", "add"),
        ("e", "edit"),
        ("D", "delete"),
        ("u", "unmanaged"),
        ("?", "help"),
        ("q", "quit"),
        ("Q", "force"),
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
            "  Source:  {}",
            match project.origin.as_ref() {
                Some(ProcessOrigin::Managed) => "managed",
                Some(ProcessOrigin::Adopted) => "adopted",
                None => "-",
            }
        )),
        Line::from(format!(
            "  PID:     {}",
            project.pid.map_or("-".into(), |p| p.to_string())
        )),
        Line::from(format!("  Uptime:  {}", uptime)),
        Line::from(""),
        Line::from(Span::styled(
            "  D/Del removes this project",
            Style::default().fg(Color::DarkGray),
        )),
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
        help_line("  e", "Edit selected project", key_style, desc_style),
        help_line(
            "  D / Del",
            "Remove selected project",
            key_style,
            desc_style,
        ),
        help_line("  d", "Show project details", key_style, desc_style),
        help_line("  o", "Open in browser", key_style, desc_style),
        help_line("  c", "Copy domain to clipboard", key_style, desc_style),
        help_line(
            "  [M]/[A]",
            "Managed / Adopted process",
            key_style,
            desc_style,
        ),
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
        help_line("  u", "Unmanaged services", key_style, desc_style),
        help_line("  ?", "Toggle this help", key_style, desc_style),
        help_line("  q", "Quit (keep running)", key_style, desc_style),
        help_line("  Q", "Force quit (stop all)", key_style, desc_style),
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

fn draw_add_form(frame: &mut Frame, app: &App, form: &AddForm) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let type_active = matches!(form.field, AddField::Type);
    let tls_active = matches!(form.field, AddField::Tls);

    // Text fields
    let text_fields: &[(&str, &str, bool)] = &[
        ("Name", &form.name, matches!(form.field, AddField::Name)),
        (
            "Domain",
            &form.domain,
            matches!(form.field, AddField::Domain),
        ),
        ("Port", &form.port, matches!(form.field, AddField::Port)),
        (
            "Upstream",
            &form.upstream_host,
            matches!(form.field, AddField::UpstreamHost),
        ),
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

    let tls_label_color = if tls_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let mut tls_spans = vec![Span::styled(
        format!("  {:<10}", "TLS"),
        Style::default().fg(tls_label_color),
    )];
    for (is_on, label) in [(false, "off"), (true, "on")] {
        if form.tls == is_on {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
        } else {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(if tls_active {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ));
        }
    }
    lines.push(Line::from(tls_spans));

    let tls_label_color = if tls_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let mut tls_spans = vec![Span::styled(
        format!("  {:<10}", "TLS"),
        Style::default().fg(tls_label_color),
    )];
    for (is_on, label) in [(false, "off"), (true, "on")] {
        if form.tls == is_on {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
        } else {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(if tls_active {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ));
        }
    }
    lines.push(Line::from(tls_spans));

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

    if let Some(error) = app.add_form_error(form) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ! {}", error),
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    let hint = if type_active || tls_active {
        "  \u{2190}/\u{2192} select option  Enter: next  Esc: cancel"
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

fn draw_edit_form(frame: &mut Frame, app: &App, form: &EditForm) {
    let area = centered_rect(60, 50, frame.area());
    frame.render_widget(Clear, area);

    let type_active = matches!(form.field, AddField::Type);
    let tls_active = matches!(form.field, AddField::Tls);

    let text_fields: &[(&str, &str, bool)] = &[
        ("Name", &form.name, matches!(form.field, AddField::Name)),
        (
            "Domain",
            &form.domain,
            matches!(form.field, AddField::Domain),
        ),
        ("Port", &form.port, matches!(form.field, AddField::Port)),
        (
            "Upstream",
            &form.upstream_host,
            matches!(form.field, AddField::UpstreamHost),
        ),
    ];

    let mut lines = vec![Line::from("")];

    for &(label, value, active) in text_fields {
        let (label_color, value_str) = if active {
            (Color::Cyan, format!("{}█", value))
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

    let tls_label_color = if tls_active {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let mut tls_spans = vec![Span::styled(
        format!("  {:<10}", "TLS"),
        Style::default().fg(tls_label_color),
    )];
    for (is_on, label) in [(false, "off"), (true, "on")] {
        if form.tls == is_on {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ));
        } else {
            tls_spans.push(Span::styled(
                format!(" {} ", label),
                Style::default().fg(if tls_active {
                    Color::White
                } else {
                    Color::DarkGray
                }),
            ));
        }
    }
    lines.push(Line::from(tls_spans));

    let path_active = matches!(form.field, AddField::Path);
    let (path_label_color, path_str) = if path_active {
        (Color::Cyan, format!("{}█", form.path))
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

    if let Some(error) = app.edit_form_error(form) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  ! {}", error),
            Style::default().fg(Color::Red),
        )));
    }

    lines.push(Line::from(""));
    let hint = if type_active || tls_active {
        "  ←/→ select option  Enter: next  Esc: cancel"
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
                .title(" Edit Project ")
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

fn draw_unmanaged_popup(frame: &mut Frame, app: &App) {
    let area = centered_rect(75, 65, frame.area());
    frame.render_widget(Clear, area);

    let filter_label = if app.unmanaged_show_unknown {
        "all"
    } else {
        "dev-only"
    };
    let port_label = if app.unmanaged_web_only {
        "web"
    } else {
        "all-ports"
    };

    let block = Block::default()
        .title(format!(
            " Unmanaged Services [{}|{}] {}/{} ",
            filter_label,
            port_label,
            app.unmanaged_services.len(),
            app.unmanaged_all_services.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    if app.unmanaged_services.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "  No unmanaged listening services found",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  f stack-filter  w port-filter  r refresh  Esc close",
                Style::default().fg(Color::DarkGray),
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
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("pid {:<7}", service.pid),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!(" {:<7}", service.stack.label()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(" {}", service.command),
                Style::default().fg(Color::White),
            ),
        ]);
        items.push(ListItem::new(line));
    }

    items.push(ListItem::new(Line::from("")));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            " Enter ",
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ),
        Span::styled("inspect  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" i ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled("import  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" I ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled("ignore  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" f ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled("stack  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" w ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled("ports  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::DarkGray)),
        Span::styled("refresh  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Esc ",
            Style::default().fg(Color::Black).bg(Color::DarkGray),
        ),
        Span::styled("close", Style::default().fg(Color::DarkGray)),
    ])));

    let mut state = ListState::default();
    state.select(Some(app.unmanaged_selected));

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

fn draw_unmanaged_detail(frame: &mut Frame, service: &ServiceInfo) {
    let area = centered_rect(65, 50, frame.area());
    frame.render_widget(Clear, area);

    let text = vec![
        Line::from(format!("  Port:        {}", service.port)),
        Line::from(format!("  PID:         {}", service.pid)),
        Line::from(format!("  Stack guess: {}", service.stack.label())),
        Line::from(format!("  Command:     {}", service.command)),
        Line::from(format!(
            "  CWD:         {}",
            service.cwd.as_deref().unwrap_or("-")
        )),
        Line::from(format!(
            "  Command line:{}{}",
            if service.command_line.is_some() {
                " "
            } else {
                ""
            },
            service.command_line.as_deref().unwrap_or("-")
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Enter/Esc closes this detail",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let popup = Paragraph::new(text)
        .block(
            Block::default()
                .title(" Service Details ")
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

fn origin_badge(origin: Option<&ProcessOrigin>) -> Span<'static> {
    match origin {
        Some(ProcessOrigin::Managed) => Span::styled(" [M]", Style::default().fg(Color::Green)),
        Some(ProcessOrigin::Adopted) => Span::styled(" [A]", Style::default().fg(Color::Yellow)),
        None => Span::styled(" [ ]", Style::default().fg(Color::DarkGray)),
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
    out.push('…');
    out
}
