//! Strategy state display widget

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use serde_json::Value;

use crate::tui::app::TuiApp;

/// Render the strategy state panel
pub fn render_strategy(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let mut lines = vec![
        Line::from(vec![
            Span::raw("  Active: "),
            Span::styled(&app.strategy_name, Style::default().fg(Color::Cyan).bold()),
        ]),
        Line::from(""),
    ];

    // Format strategy state as key-value pairs
    if let Value::Object(map) = &app.strategy_state {
        for (key, value) in map.iter().take(8) { // Limit to 8 fields
            let value_str = format_json_value(value);
            lines.push(Line::from(vec![
                Span::raw(format!("  {}: ", key)),
                Span::styled(value_str, Style::default().fg(Color::Yellow)),
            ]));
        }
    } else if app.strategy_state != Value::Null {
        let state_str = app.strategy_state.to_string();
        lines.push(Line::from(vec![
            Span::raw("  State: "),
            Span::styled(state_str, Style::default().fg(Color::Yellow)),
        ]));
    } else {
        lines.push(Line::from(Span::styled(
            "  No state available",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let strategy_panel = Paragraph::new(lines)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Strategy ")
            .border_style(Style::default().fg(Color::Magenta)))
        .wrap(Wrap { trim: true });

    frame.render_widget(strategy_panel, area);
}

/// Format a JSON value for display
fn format_json_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            // Format numbers nicely
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 {
                    format!("{:.0}", f)
                } else {
                    format!("{:.4}", f)
                }
            } else {
                n.to_string()
            }
        }
        Value::String(s) => s.clone(),
        Value::Array(arr) => format!("[{} items]", arr.len()),
        Value::Object(obj) => format!("{{{} fields}}", obj.len()),
    }
}
