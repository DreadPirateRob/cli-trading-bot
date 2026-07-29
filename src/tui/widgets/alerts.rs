//! Alerts panel widget

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem},
};

use crate::tui::app::{AlertLevel, TuiApp};

/// Render the alerts panel
pub fn render_alerts(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let items: Vec<ListItem> = app
        .alerts
        .iter()
        .map(|alert| {
            let time = alert.timestamp.format("%H:%M:%S").to_string();
            let (symbol, style) = match alert.level {
                AlertLevel::Info => ("i", Style::default().fg(Color::Blue)),
                AlertLevel::Warning => ("!", Style::default().fg(Color::Yellow)),
                AlertLevel::Error => ("x", Style::default().fg(Color::Red)),
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", symbol), style),
                Span::styled(format!("[{}] ", time), Style::default().fg(Color::DarkGray)),
                Span::raw(&alert.message),
            ]))
        })
        .collect();

    // Show empty state if no alerts
    if items.is_empty() {
        let empty = ratatui::widgets::Paragraph::new("No alerts")
            .style(Style::default().fg(Color::DarkGray))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Alerts ")
                    .border_style(Style::default().fg(Color::Yellow)),
            );
        frame.render_widget(empty, area);
        return;
    }

    let alerts_list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" Alerts ({}) ", app.alerts.len()))
            .border_style(Style::default().fg(Color::Yellow)),
    );

    frame.render_widget(alerts_list, area);
}
