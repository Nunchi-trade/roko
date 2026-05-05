//! Projection — single normalized event vocabulary that the TUI,
//! HTTP/SSE, and non-TUI CLI all consume.
//!
//! ## Architectural role
//!
//! The runner emits typed events (`AgentEvent`, `RunnerEvent`,
//! `GateCompletion`, ...). Each consumer historically had its own
//! private mapping from those events into its preferred shape. That
//! drifted: TUI showed one set of fields, HTTP showed another,
//! non-TUI CLI rendered a third.
//!
//! This module is the single seam that turns runner events into:
//!
//! - A normalized [`ProjectionEvent`] (already produced by
//!   [`runner::projection::Projection`]). Every consumer subscribes to
//!   the same broadcast channel and never branches on event payload.
//! - A dashboard projection ([`dashboard::DashboardProjection`]) that
//!   bridges the broadcast into [`StateHub`] / [`DashboardEvent`].
//! - A CLI progress projection ([`cli_progress::CliProgressPrinter`])
//!   for users running plans without a TUI.
//!
//! Crucially, this module *wraps* the lower-level runner projection
//! facade — it does not duplicate it. The facade owns event ingestion;
//! this module owns the rendering paths.

use std::sync::Arc;

use tokio::sync::broadcast;

pub mod cli_progress;
pub mod dashboard;

pub use cli_progress::CliProgressPrinter;
pub use dashboard::DashboardProjection;

use crate::runner::projection::{Projection, ProjectionEvent};

// ─── Subscriber facade ─────────────────────────────────────────────────

/// One subscription to the projection broadcast.
///
/// Wraps `broadcast::Receiver<ProjectionEvent>` with explicit handling
/// for lagged subscribers — instead of returning a `RecvError::Lagged`
/// to the caller, the receiver counts dropped events and emits a
/// synthesized "lagged" event so renderers can display a hint that
/// they're behind.
pub struct ProjectionSubscriber {
    rx: broadcast::Receiver<ProjectionEvent>,
}

impl ProjectionSubscriber {
    /// Create a subscriber from a shared projection.
    #[must_use]
    pub fn new(projection: &Projection) -> Self {
        Self {
            rx: projection.subscribe(),
        }
    }

    /// Receive the next event.
    ///
    /// Returns:
    /// - `Ok(Some(event))` for a regular event,
    /// - `Ok(None)` if the broadcast was closed,
    /// - `Err(SubscribeError)` if the subscriber lagged so far behind
    ///   the broadcast that events were dropped — the receiver tells
    ///   the caller how many.
    pub async fn recv(&mut self) -> Result<Option<ProjectionEvent>, SubscribeError> {
        match self.rx.recv().await {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::RecvError::Closed) => Ok(None),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                Err(SubscribeError::Lagged { dropped: n })
            }
        }
    }
}

/// Why a subscriber failed to receive an event.
#[derive(Debug, thiserror::Error, Clone, Copy)]
pub enum SubscribeError {
    /// The subscriber fell behind the broadcast and the buffer evicted
    /// some events. `dropped` reports how many were lost.
    #[error("projection subscriber lagged; dropped {dropped} events")]
    Lagged {
        /// Number of events evicted before the receiver caught up.
        dropped: u64,
    },
}

/// Construct a fresh broadcast-based subscriber on a projection.
///
/// Provided as a convenience so call sites do not have to import
/// `tokio::sync::broadcast`.
#[must_use]
pub fn subscribe(projection: &Arc<Projection>) -> ProjectionSubscriber {
    ProjectionSubscriber::new(projection)
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::projection::RawRuntimeEvent;
    use crate::runner::types::RunnerEvent;

    #[tokio::test]
    async fn subscriber_receives_published_event() {
        let projection = Arc::new(Projection::new("run-1"));
        let mut sub = subscribe(&projection);
        projection
            .publish(RawRuntimeEvent::Runner(RunnerEvent::PlanStarted {
                timestamp: "now".into(),
                timestamp_ms: 0,
                run_id: "run-1".into(),
                plan_id: "plan-1".into(),
            }))
            .unwrap();
        let evt = tokio::time::timeout(std::time::Duration::from_secs(2), sub.recv())
            .await
            .unwrap()
            .unwrap()
            .expect("event arrives");
        assert_eq!(evt.plan_id.as_deref(), Some("plan-1"));
    }

    #[tokio::test]
    async fn subscriber_receives_none_when_broadcast_closed() {
        let projection = Arc::new(Projection::new("run-1"));
        let mut sub = subscribe(&projection);
        // Drop the projection — broadcast sender goes away.
        drop(projection);
        let evt = sub.recv().await.unwrap();
        assert!(evt.is_none(), "closed broadcast must surface as None");
    }
}
