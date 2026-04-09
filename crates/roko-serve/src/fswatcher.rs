//! Background filesystem watcher bootstrap for daemon mode.

use std::sync::Arc;

use tokio::task::JoinHandle;

use roko_plugin::{EventSource, FileWatchEventSource};

use crate::state::AppState;

/// Start the configured filesystem watchers in the background.
#[must_use]
pub fn start_watchers(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let watchers = {
            let config = state.roko_config.read().await;
            if config.watcher.is_empty() {
                Vec::new()
            } else {
                vec![
                    Box::new(FileWatchEventSource::from_config(config.watcher.clone()))
                        as Box<dyn EventSource>,
                ]
            }
        };

        let _ = crate::start_event_source_group(Arc::clone(&state), watchers);
        state.cancel.cancelled().await;
    })
}
