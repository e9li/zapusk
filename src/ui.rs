use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::{ActivePane, App};
use crate::project::ProjectStatus;

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
}

fn draw_project_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.active_pane == ActivePane::ProjectList;

    let block = Block::default()
        .title(" Projects ")
        .borders(Borders::ALL)
        .border_style(focus_style(focused));

    let items: Vec<ListItem> = app
        .projects
        .iter()
        .map(|project| {
            let (status_icon, status_color) = match &project.status {
                ProjectStatus::Running => ("●", Color::Green),
                ProjectStatus::Starting => ("◌", Color::Yellow),
                ProjectStatus::Stopped => ("○", Color::DarkGray),
                ProjectStatus::Failed(_) => ("✗", Color::Red),
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", status_icon),
                    Style::default().fg(status_color),
                ),
                Span::styled(
                    format!("{:<18}", project.config.name),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!(" {}", project.config.project_type.label()),
                    Style::default().fg(Color::Cyan),
                ),
            ]);

            ListItem::new(line)
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

    // Draw keybind hints below the list if there's space
    // (handled in status bar instead for simplicity)
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

    let inner_height = area.height.saturating_sub(2) as usize;
    let skip = logs.len().saturating_sub(inner_height);

    let text: Vec<Line> = logs
        .iter()
        .skip(skip)
        .map(|entry| {
            let color = if entry.is_stderr {
                Color::Yellow
            } else {
                Color::Reset
            };
            let ts = entry.timestamp.format("%H:%M:%S").to_string();
            Line::from(vec![
                Span::styled(
                    format!("{} ", ts),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(entry.line.clone(), Style::default().fg(color)),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let message = app
        .status_message
        .as_deref()
        .unwrap_or("");

    let hints = " s start  x stop  r restart  R reload caddy  tab switch pane  q quit";

    let line = Line::from(vec![
        Span::styled(
            format!(" {:<30}", message),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(hints, Style::default().fg(Color::DarkGray)),
    ]);

    let bar = Paragraph::new(line)
        .style(Style::default().bg(Color::Black));

    frame.render_widget(bar, area);
}

fn focus_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}
