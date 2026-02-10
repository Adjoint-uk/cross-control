//! TUI rendering with ratatui.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use cross_control_daemon::DaemonStatus;

use crate::app::AppState;

pub fn draw(f: &mut Frame, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(10),    // Screens
            Constraint::Length(12), // Event log
            Constraint::Length(3),  // Help bar
        ])
        .split(f.area());

    draw_title(f, chunks[0]);
    draw_screens(f, chunks[1], app);
    draw_log(f, chunks[2], app);
    draw_help(f, chunks[3]);
}

fn draw_title(f: &mut Frame, area: Rect) {
    let title = Paragraph::new("cross-control TUI test")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, area);
}

fn draw_screens(f: &mut Frame, area: Rect, app: &AppState) {
    let screen_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let status_a = app.status_a_snapshot();
    let status_b = app.status_b_snapshot();

    draw_screen(
        f,
        screen_chunks[0],
        "Machine A (1920x1080)",
        &status_a,
        app,
        true,
    );
    draw_screen(
        f,
        screen_chunks[1],
        "Machine B (1920x1080)",
        &status_b,
        app,
        false,
    );
}

fn state_label(status: &DaemonStatus) -> &'static str {
    if status.controlling.is_some() {
        "Controlling"
    } else if status.controlled_by.is_some() {
        "Controlled"
    } else if status.session_count > 0 {
        "Idle"
    } else {
        "Disconnected"
    }
}

fn state_color(label: &str) -> Color {
    match label {
        "Controlling" => Color::Green,
        "Controlled" => Color::Yellow,
        "Idle" => Color::White,
        _ => Color::Red,
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn draw_screen(
    f: &mut Frame,
    area: Rect,
    title: &str,
    status: &DaemonStatus,
    app: &AppState,
    show_cursor: bool,
) {
    let label = state_label(status);
    let color = state_color(label);

    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Draw state label
    let state_line = Paragraph::new(Line::from(vec![
        Span::raw("State: "),
        Span::styled(
            label,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  Sessions: {}", status.session_count)),
    ]));
    if inner.height > 0 {
        f.render_widget(
            state_line,
            Rect {
                y: inner.y + inner.height.saturating_sub(1),
                height: 1,
                ..inner
            },
        );
    }

    // Draw cursor dot if this is the local screen (A)
    if show_cursor && inner.width > 2 && inner.height > 2 {
        let screen_w = f64::from(app.screen_width);
        let screen_h = f64::from(app.screen_height);
        let draw_w = f64::from(inner.width - 2);
        let draw_h = f64::from(inner.height - 2);

        let cx = ((f64::from(app.cursor_x) / screen_w) * draw_w) as u16;
        let cy = ((f64::from(app.cursor_y) / screen_h) * draw_h) as u16;

        let cursor_x = inner.x + 1 + cx.min(inner.width.saturating_sub(3));
        let cursor_y = inner.y + 1 + cy.min(inner.height.saturating_sub(3));

        let cursor = Paragraph::new("@").style(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            cursor,
            Rect {
                x: cursor_x,
                y: cursor_y,
                width: 1,
                height: 1,
            },
        );
    }
}

fn draw_log(f: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .log_lines
        .iter()
        .rev()
        .take(area.height.saturating_sub(2) as usize)
        .map(|line| {
            let color = if line.starts_with("A:") {
                Color::Cyan
            } else if line.starts_with("B:") {
                Color::Yellow
            } else {
                Color::White
            };
            ListItem::new(Span::styled(
                format!("> {line}"),
                Style::default().fg(color),
            ))
        })
        .collect();

    let log = List::new(items).block(Block::default().title("Event Log").borders(Borders::ALL));
    f.render_widget(log, area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let help = Paragraph::new("q: quit  arrows: cursor  letters: keys  Ctrl+Shift+Esc: release")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(ratatui::layout::Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, area);
}
