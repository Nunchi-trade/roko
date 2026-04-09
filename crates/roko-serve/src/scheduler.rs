//! Background cron scheduler bootstrap for daemon mode.

use std::sync::Arc;

use tokio::task::JoinHandle;

use roko_plugin::{CronEventSource, EventSource};

use crate::state::AppState;

/// Start the configured cron scheduler in the background.
#[must_use]
pub fn start_scheduler(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let scheduler = {
            let config = state.roko_config.read().await;
            if config.scheduler.is_empty() {
                Vec::new()
            } else {
                vec![
                    Box::new(CronEventSource::from_config(config.scheduler.clone()))
                        as Box<dyn EventSource>,
                ]
            }
        };

        let _ = crate::start_event_source_group(Arc::clone(&state), scheduler);
        state.cancel.cancelled().await;
    })
}
