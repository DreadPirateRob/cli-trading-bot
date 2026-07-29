//! File watcher for automatic configuration reloading.
//!
//! Watches the config directory for changes and triggers reload
//! with debouncing to handle multiple rapid events.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::manager::ConfigManager;

/// Watches config files and triggers reloads on changes.
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    shutdown_tx: mpsc::Sender<()>,
}

impl ConfigWatcher {
    /// Start watching the config directory for changes.
    ///
    /// # Arguments
    /// * `config_dir` - Path to the config directory
    /// * `manager` - ConfigManager to reload on changes
    /// * `debounce_ms` - Minimum milliseconds between reloads (default 500)
    ///
    /// # Returns
    /// ConfigWatcher handle - drop to stop watching
    pub fn start(
        config_dir: &Path,
        manager: Arc<ConfigManager>,
        debounce_ms: u64,
    ) -> Result<Self, WatcherError> {
        let (event_tx, mut event_rx) = mpsc::channel::<Event>(100);
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Create file watcher
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    // Only care about modify/create events for YAML files
                    match event.kind {
                        EventKind::Modify(_) | EventKind::Create(_) => {
                            let is_yaml = event.paths.iter().any(|p| {
                                p.extension()
                                    .map(|ext| ext == "yaml" || ext == "yml")
                                    .unwrap_or(false)
                            });
                            if is_yaml {
                                let _ = event_tx.blocking_send(event);
                            }
                        }
                        _ => {}
                    }
                }
            },
            Config::default(),
        )
        .map_err(|e| WatcherError::Creation { reason: e.to_string() })?;

        // Watch config directory
        watcher
            .watch(config_dir, RecursiveMode::NonRecursive)
            .map_err(|e| WatcherError::Watch {
                path: config_dir.display().to_string(),
                reason: e.to_string(),
            })?;

        // Spawn debounced reload task
        let config_dir_str = config_dir.display().to_string();
        tokio::spawn(async move {
            let debounce_duration = Duration::from_millis(debounce_ms);
            let mut last_reload = Instant::now() - debounce_duration; // Allow immediate first reload

            loop {
                tokio::select! {
                    Some(_event) = event_rx.recv() => {
                        let now = Instant::now();
                        if now.duration_since(last_reload) >= debounce_duration {
                            tracing::debug!(
                                config_dir = %config_dir_str,
                                "Config file change detected, reloading..."
                            );
                            if let Err(e) = manager.reload() {
                                tracing::error!(
                                    error = %e,
                                    "Hot-reload failed, keeping previous config"
                                );
                            }
                            last_reload = now;
                        } else {
                            tracing::trace!("Config change debounced");
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::debug!("Config watcher shutting down");
                        break;
                    }
                }
            }
        });

        tracing::info!(
            config_dir = %config_dir.display(),
            debounce_ms = %debounce_ms,
            "Config file watcher started"
        );

        Ok(Self {
            _watcher: watcher,
            shutdown_tx,
        })
    }

    /// Stop watching (also happens automatically on drop).
    pub async fn stop(self) {
        let _ = self.shutdown_tx.send(()).await;
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        // Best-effort shutdown signal
        let _ = self.shutdown_tx.try_send(());
    }
}

/// Errors from config watching
#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    #[error("Failed to create file watcher: {reason}")]
    Creation { reason: String },

    #[error("Failed to watch path '{path}': {reason}")]
    Watch { path: String, reason: String },
}
