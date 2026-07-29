//! Equity and P&L display widget

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph},
};
use rust_decimal::Decimal;

use crate::tui::app::TuiApp;

/// Render the equity and P&L panel
pub fn render_equity(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let equity_str = format_decimal(app.equity);
    let unrealized_str = format_decimal_with_sign(app.unrealized_pnl);
    let daily_str = format_decimal_with_sign(app.daily_pnl);

    // Color based on P&L
    let unrealized_style = pnl_style(app.unrealized_pnl);
    let daily_style = pnl_style(app.daily_pnl);

    let content = vec![
        Line::from(vec![
            Span::raw("  Equity:       "),
            Span::styled(format!("${}", equity_str), Style::default().fg(Color::White).bold()),
        ]),
        Line::from(vec![
            Span::raw("  Unrealized:   "),
            Span::styled(format!("${}", unrealized_str), unrealized_style),
        ]),
        Line::from(vec![
            Span::raw("  Daily P&L:    "),
            Span::styled(format!("${}", daily_str), daily_style),
        ]),
    ];

    let equity_panel = Paragraph::new(content)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Equity & P&L ")
            .border_style(Style::default().fg(Color::Blue)));

    frame.render_widget(equity_panel, area);
}

/// Get style based on P&L value
fn pnl_style(value: Decimal) -> Style {
    if value > Decimal::ZERO {
        Style::default().fg(Color::Green)
    } else if value < Decimal::ZERO {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Gray)
    }
}

/// Format decimal for display
fn format_decimal(value: Decimal) -> String {
    format!("{:.2}", value)
}

/// Format decimal with + sign for positive
fn format_decimal_with_sign(value: Decimal) -> String {
    if value > Decimal::ZERO {
        format!("+{:.2}", value)
    } else {
        format!("{:.2}", value)
    }
}
