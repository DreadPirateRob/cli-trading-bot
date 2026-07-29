//! TUI rendering logic (View in Elm Architecture)
//!
//! Composes widgets into a complete dashboard layout.

use ratatui::prelude::*;

use super::app::TuiApp;
use super::widgets;

/// Render the complete TUI dashboard
pub fn render(app: &TuiApp, frame: &mut Frame) {
    // Main vertical layout
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),  // Status bar (top)
            Constraint::Min(0),     // Main content area
            Constraint::Length(3),  // Footer/help (bottom)
        ])
        .split(frame.area());

    // Render status bar
    widgets::render_status(frame, main_chunks[0], app);

    // Main content: horizontal split
    // Left side: Equity + Strategy (stacked)
    // Right side: Trades table
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // Left panel
            Constraint::Percentage(65), // Right panel (trades)
        ])
        .split(main_chunks[1]);

    // Left panel: vertical split for equity and strategy
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // Equity panel
            Constraint::Min(0),    // Strategy panel
            Constraint::Length(8), // Alerts panel
        ])
        .split(content_chunks[0]);

    widgets::render_equity(frame, left_chunks[0], app);
    widgets::render_strategy(frame, left_chunks[1], app);
    widgets::render_alerts(frame, left_chunks[2], app);

    // Right panel: trades table
    widgets::render_trades(frame, content_chunks[1], app);

    // Footer with keyboard shortcuts
    render_footer(frame, main_chunks[2]);
}

/// Render the footer with keyboard shortcuts
fn render_footer(frame: &mut Frame, area: Rect) {
    let help_text = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Yellow).bold()),
        Span::raw(" Quit  "),
        Span::styled("r", Style::default().fg(Color::Yellow).bold()),
        Span::raw(" Refresh  "),
        Span::styled("^/v", Style::default().fg(Color::Yellow).bold()),
        Span::raw(" Scroll  "),
    ]);

    let footer = ratatui::widgets::Paragraph::new(help_text)
        .style(Style::default().bg(Color::DarkGray));

    frame.render_widget(footer, area);
}
