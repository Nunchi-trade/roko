//! Background cron scheduler bootstrap for daemon mode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::task::JoinHandle;

use roko_plugin::{CronEventSource, EventSource};

use crate::state::AppState;

/// Process-wide guard preventing duplicate cron scheduler starts.
static SCHEDULER_STARTED: AtomicBool = AtomicBool::new(false);

/// Start the configured cron scheduler in the background.
///
/// If a scheduler has already been started in this process (e.g. via the
/// serve path's `start_builtin_event_sources`), this returns a no-op
/// handle and logs a warning.
#[must_use]
pub fn start_scheduler(state: Arc<AppState>) -> JoinHandle<()> {
    if SCHEDULER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        tracing::warn!("scheduler already running; skipping duplicate start");
        return tokio::spawn(async {});
    }
    tokio::spawn(async move {
        let scheduler = {
            let config = state.load_roko_config();
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

/// Returns `true` if the scheduler guard has already been claimed (for
/// use by `start_builtin_event_sources`).
pub fn claim_scheduler_guard() -> bool {
    SCHEDULER_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}
