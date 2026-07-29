//! TUI widget components
//!
//! Each widget renders a specific panel of the dashboard.

mod status;
mod equity;
mod strategy;
mod trades;
mod alerts;

pub use status::render_status;
pub use equity::render_equity;
pub use strategy::render_strategy;
pub use trades::render_trades;
pub use alerts::render_alerts;
