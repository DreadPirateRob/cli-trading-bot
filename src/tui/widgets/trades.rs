//! Recent trades table widget

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Row, Table},
};
use rust_decimal::Decimal;

use crate::tui::app::TuiApp;

/// Render the recent trades table
pub fn render_trades(frame: &mut Frame, area: Rect, app: &TuiApp) {
    // Table header
    let header = Row::new(vec![
        Cell::from("Time").style(Style::default().bold()),
        Cell::from("Symbol").style(Style::default().bold()),
        Cell::from("Side").style(Style::default().bold()),
        Cell::from("Qty").style(Style::default().bold()),
        Cell::from("Price").style(Style::default().bold()),
        Cell::from("P&L").style(Style::default().bold()),
    ])
    .bottom_margin(1);

    // Table rows from recent trades
    let rows: Vec<Row> = app
        .recent_trades
        .iter()
        .map(|trade| {
            let time = trade.timestamp.format("%H:%M:%S").to_string();
            let side_style = if trade.side.to_lowercase() == "buy" {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };
            let pnl_cell = match trade.pnl {
                Some(pnl) => {
                    let style = if pnl > Decimal::ZERO {
                        Style::default().fg(Color::Green)
                    } else if pnl < Decimal::ZERO {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    Cell::from(format!("{:.2}", pnl)).style(style)
                }
                None => Cell::from("-").style(Style::default().fg(Color::DarkGray)),
            };

            Row::new(vec![
                Cell::from(time),
                Cell::from(trade.symbol.clone()),
                Cell::from(trade.side.clone()).style(side_style),
                Cell::from(format!("{:.6}", trade.quantity)),
                Cell::from(format!("{:.2}", trade.price)),
                pnl_cell,
            ])
        })
        .collect();

    // Show empty state if no trades
    if rows.is_empty() {
        let empty = ratatui::widgets::Paragraph::new("No trades yet")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Recent Trades ")
                    .border_style(Style::default().fg(Color::Green)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(10), // Time
            Constraint::Length(10), // Symbol
            Constraint::Length(6),  // Side
            Constraint::Length(12), // Qty
            Constraint::Length(12), // Price
            Constraint::Length(12), // P&L
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Recent Trades ({}) ", app.recent_trades.len()))
            .border_style(Style::default().fg(Color::Green)),
    );

    frame.render_widget(table, area);
}
