//! Status bar widget showing bot operational state

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::TuiApp;

/// Render the status bar at the top of the TUI
pub fn render_status(frame: &mut Frame, area: Rect, app: &TuiApp) {
    // Mode indicator with color
    let (mode_text, mode_style) = if app.paper_mode {
        ("[PAPER]", Style::default().fg(Color::Yellow).bold())
    } else {
        ("[LIVE]", Style::default().fg(Color::Red).bold())
    };

    // Status indicator with color
    let (status_style, status_symbol) = match app.bot_status.as_str() {
        "running" => (Style::default().fg(Color::Green), "●"),
        "stopped" => (Style::default().fg(Color::Gray), "○"),
        "error" => (Style::default().fg(Color::Red), "✖"),
        _ => (Style::default(), "?"),
    };

    // Format uptime
    let uptime = format_uptime(app.uptime_seconds);

    // Build status line
    let status_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(mode_text, mode_style),
        Span::raw(" | Status: "),
        Span::styled(status_symbol, status_style),
        Span::raw(" "),
        Span::styled(&app.bot_status, status_style),
        Span::raw(" | Strategy: "),
        Span::styled(&app.strategy_name, Style::default().fg(Color::Cyan)),
        Span::raw(" | Uptime: "),
        Span::raw(uptime),
        Span::raw(" "),
    ]);

    let status_bar = Paragraph::new(status_line)
        .block(Block::default().borders(Borders::BOTTOM));

    frame.render_widget(status_bar, area);
}

/// Format seconds into human-readable uptime
fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}
