//! TUI rendering with ratatui.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use cross_control_daemon::DaemonStatus;

use crate::app::AppState;

/// Colors assigned to each screen slot.
const SCREEN_COLORS: [Color; 4] = [Color::Cyan, Color::Green, Color::Yellow, Color::Magenta];

pub fn draw(f: &mut Frame, app: &AppState) {
    // Check if we're still waiting for connections
    let status_0 = app.status_snapshot(0);
    if status_0.session_count == 0 {
        let msg = Paragraph::new("Connecting daemons...")
            .style(Style::default().fg(Color::Yellow))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(msg, f.area());
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title
            Constraint::Min(10),    // Screens
            Constraint::Length(10), // Event log
            Constraint::Length(3),  // Help bar
        ])
        .split(f.area());

    draw_title(f, chunks[0], app);
    draw_screens(f, chunks[1], app);
    draw_log(f, chunks[2], app);
    draw_help(f, chunks[3]);
}

fn draw_title(f: &mut Frame, area: Rect, app: &AppState) {
    let count = app.screens.len();
    let title = Paragraph::new(format!("cross-control TUI test — {count} screens"))
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
    let count = app.screens.len();

    match count {
        1 => {
            draw_screen_at(f, area, app, 0);
        }
        2 => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            draw_screen_at(f, cols[0], app, 0);
            draw_screen_at(f, cols[1], app, 1);
        }
        3..=4 => {
            // 2x2 grid
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[0]);
            let bot = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(rows[1]);

            draw_screen_at(f, top[0], app, 0);
            draw_screen_at(f, top[1], app, 1);
            if count > 2 {
                draw_screen_at(f, bot[0], app, 2);
            }
            if count > 3 {
                draw_screen_at(f, bot[1], app, 3);
            }
        }
        _ => {}
    }
}

fn draw_screen_at(f: &mut Frame, area: Rect, app: &AppState, idx: usize) {
    let status = app.status_snapshot(idx);
    let name = &app.screens[idx].name;

    // Show cursor on a screen if it's the primary (idx 0 and not controlling)
    // or if it's being controlled by someone
    let show_cursor = if idx == 0 {
        status.controlling.is_none()
    } else {
        status.controlled_by.is_some()
    };

    let color = SCREEN_COLORS[idx % SCREEN_COLORS.len()];

    draw_screen(
        f,
        area,
        name,
        &status,
        status.cursor_x,
        status.cursor_y,
        app.screen_width,
        app.screen_height,
        show_cursor,
        color,
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

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_arguments
)]
fn draw_screen(
    f: &mut Frame,
    area: Rect,
    title: &str,
    status: &DaemonStatus,
    cursor_x: i32,
    cursor_y: i32,
    screen_width: u32,
    screen_height: u32,
    show_cursor: bool,
    border_color: Color,
) {
    let label = state_label(status);
    let color = state_color(label);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
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

    // Draw cursor dot
    if show_cursor && inner.width > 2 && inner.height > 2 {
        let screen_w = f64::from(screen_width);
        let screen_h = f64::from(screen_height);
        let draw_w = f64::from(inner.width - 2);
        let draw_h = f64::from(inner.height - 2);

        let cx = ((f64::from(cursor_x) / screen_w) * draw_w) as u16;
        let cy = ((f64::from(cursor_y) / screen_h) * draw_h) as u16;

        let draw_x = inner.x + 1 + cx.min(inner.width.saturating_sub(3));
        let draw_y = inner.y + 1 + cy.min(inner.height.saturating_sub(3));

        let cursor = Paragraph::new("@").style(
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(
            cursor,
            Rect {
                x: draw_x,
                y: draw_y,
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
            // Color by screen name prefix
            let color = if line.starts_with("A:") {
                SCREEN_COLORS[0]
            } else if line.starts_with("B:") {
                SCREEN_COLORS[1]
            } else if line.starts_with("C:") {
                SCREEN_COLORS[2]
            } else if line.starts_with("D:") {
                SCREEN_COLORS[3]
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
    let help =
        Paragraph::new("q: quit  arrows: move cursor  letters: send keys  F12: release control")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(ratatui::layout::Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, area);
}
