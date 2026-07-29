//! Terminal User Interface for bot status monitoring
//!
//! Uses ratatui with crossterm backend for cross-platform terminal UI.
//! Follows Elm Architecture: Model (app.rs), View (ui.rs), Update (event.rs).

mod app;
mod event;
mod ui;
pub mod widgets;

pub use app::TuiApp;
pub use event::{handle_key_event, EventHandler, TuiEvent};

use std::io::{self, Stdout};
use crossterm::{
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Initialize terminal for TUI mode.
/// Enables raw mode and switches to alternate screen.
pub fn init_terminal() -> io::Result<Tui> {
    init_panic_hook();
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

/// Restore terminal to normal mode.
/// Disables raw mode and leaves alternate screen.
pub fn restore_terminal() -> io::Result<()> {
    io::stdout().execute(LeaveAlternateScreen)?;
    terminal::disable_raw_mode()
}

/// Set up panic hook to restore terminal before panic message displays.
/// Critical for preventing terminal corruption on crashes.
fn init_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore terminal BEFORE displaying panic
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().execute(LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}

use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::api::types::BotUpdate;
use crate::notifications::AlertEvent;

/// Run the TUI application
///
/// # Arguments
/// * `bot_rx` - Optional receiver for live bot updates (None for standalone mode)
/// * `alert_rx` - Optional receiver for alert events (None for standalone mode)
///
/// When both receivers are None, TUI runs in standalone mode with sample data.
/// When receivers are provided, TUI displays live data from the running bot.
pub async fn run_tui(
    bot_rx: Option<broadcast::Receiver<BotUpdate>>,
    alert_rx: Option<mpsc::Receiver<AlertEvent>>,
) -> io::Result<()> {
    // Initialize terminal
    let mut terminal = init_terminal()?;

    // Create app state - use sample data if standalone, empty if connected
    let mut app = if bot_rx.is_none() && alert_rx.is_none() {
        TuiApp::with_sample_data()
    } else {
        TuiApp::new()
    };

    // Create shutdown token
    let shutdown_token = CancellationToken::new();

    // Create event handler with 100ms tick rate and optional live data receivers
    let mut events = EventHandler::new(
        Duration::from_millis(100),
        shutdown_token.clone(),
        bot_rx,
        alert_rx,
    );

    // Main loop
    while app.running {
        // Render current state
        terminal.draw(|frame| ui::render(&app, frame))?;

        // Handle next event
        if let Some(event) = events.next().await {
            match event {
                TuiEvent::Key(key) => {
                    if event::handle_key_event(key) {
                        app.quit();
                    }
                }
                TuiEvent::Tick => {
                    // Update uptime
                    app.uptime_seconds += 1;
                }
                TuiEvent::BotUpdate(update) => {
                    app.apply_bot_update(&update);
                }
                TuiEvent::Alert(alert) => {
                    app.apply_alert(&alert);
                }
                TuiEvent::Shutdown => {
                    app.quit();
                }
            }
        }
    }

    // Restore terminal
    restore_terminal()
}
