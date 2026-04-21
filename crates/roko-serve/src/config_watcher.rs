//! Configuration file watcher for hot-reload (LIFE-07).
//!
//! Watches `roko.toml` and `STRATEGY.md` for changes and triggers
//! hot-reload of the running configuration. Hot-reloadable sections
//! (`[budget]`, `[tools]`, `[learning]`, etc.) are applied immediately.
//! Non-hot-reloadable sections log a warning suggesting restart.
//!
//! STRATEGY.md changes are always fully reloaded (full hot-reload).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::routes::reload_config_from_disk;
use crate::state::AppState;

/// Debounce interval for filesystem events. Multiple rapid writes to the
/// same file within this window produce only a single reload.
const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(500);

/// Start a background task that watches `roko.toml` and `STRATEGY.md`
/// for changes and triggers hot-reload.
///
/// Returns a `JoinHandle` that runs until the server's cancellation token
/// is triggered.
#[must_use]
pub fn start_config_watcher(state: Arc<AppState>) -> JoinHandle<()> {
    let workdir = state.workdir.clone();
    let cancel = state.cancel.clone();

    tokio::spawn(async move {
        let config_path = workdir.join("roko.toml");
        let strategy_path = workdir.join("STRATEGY.md");

        // Track file modification times for change detection.
        let mut config_mtime = file_mtime(&config_path);
        let mut strategy_mtime = file_mtime(&strategy_path);
        let mut last_reload = Instant::now();

        // Poll every 2 seconds for file changes. This is simpler and more
        // portable than notify-based watching, and the 2-second latency is
        // acceptable for config changes.
        let mut interval = tokio::time::interval(Duration::from_secs(2));

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("config watcher shutting down");
                    break;
                }
                _ = interval.tick() => {
                    let now = Instant::now();

                    // Check roko.toml for changes.
                    let current_config_mtime = file_mtime(&config_path);
                    if current_config_mtime != config_mtime
                        && now.duration_since(last_reload) > DEBOUNCE_INTERVAL
                    {
                        config_mtime = current_config_mtime;
                        last_reload = now;

                        info!(path = %config_path.display(), "roko.toml changed, reloading");
                        match reload_config_from_disk(&state) {
                            Ok(warnings) => {
                                if warnings.is_empty() {
                                    info!("config hot-reload complete");
                                } else {
                                    for w in &warnings {
                                        warn!("config reload: {w}");
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "failed to reload roko.toml");
                            }
                        }
                    }

                    // Check STRATEGY.md for changes.
                    let current_strategy_mtime = file_mtime(&strategy_path);
                    if current_strategy_mtime != strategy_mtime
                        && now.duration_since(last_reload) > DEBOUNCE_INTERVAL
                    {
                        strategy_mtime = current_strategy_mtime;
                        last_reload = now;

                        info!(path = %strategy_path.display(), "STRATEGY.md changed, reloading");
                        match crate::routes::config::reload_strategy_from_disk(&state) {
                            Some(doc) => {
                                info!(
                                    goals = doc.goals.len(),
                                    tactics = doc.tactics.len(),
                                    risk_bounds = doc.risk_bounds.len(),
                                    "STRATEGY.md hot-reload complete"
                                );
                            }
                            None => {
                                warn!("STRATEGY.md not found or could not be parsed");
                            }
                        }
                    }
                }
            }
        }
    })
}

/// Return the modification time of a file as a duration since UNIX epoch,
/// or `Duration::ZERO` if the file does not exist.
fn file_mtime(path: &PathBuf) -> Duration {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
        })
        .unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_mtime_returns_zero_for_nonexistent() {
        let path = PathBuf::from("/nonexistent/path/roko.toml");
        assert_eq!(file_mtime(&path), Duration::ZERO);
    }

    #[test]
    fn file_mtime_returns_nonzero_for_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "test").expect("write");
        assert_ne!(file_mtime(&path.to_path_buf()), Duration::ZERO);
    }
}
