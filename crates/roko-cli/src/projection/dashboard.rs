//! Dashboard projection — bridges normalized [`ProjectionEvent`]s into
//! the existing TUI / `StateHub` event vocabulary.
//!
//! ## Why a bridge instead of a rewrite
//!
//! The TUI subscribes to [`StateHub::dashboard`] events
//! ([`DashboardEvent`]) — there are 25+ variants and the TUI views
//! depend on each one. Rewriting the TUI to consume `ProjectionEvent`
//! directly is a large surface change. Instead the bridge does a small,
//! testable mapping and delegates rendering to the existing TUI code.
//!
//! ## Bounded mapping
//!
//! Not every `ProjectionEvent` becomes a `DashboardEvent` — only the
//! ones the TUI knows how to render. Unmappable events are counted via
//! [`DashboardProjection::stats`] so we can spot drift.
//!
//! Bounding policy: the bridge applies the same 4 KB preview cap as the
//! upstream projection facade. That guarantee carries through to the
//! TUI without each TUI panel having to re-truncate.

use std::sync::Arc;

use crate::runner::projection::{PROJECTION_OUTPUT_PREVIEW_BYTES, ProjectionEvent};
use crate::runner::types::EventCategory;

/// Counters surfaced by [`DashboardProjection::stats`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DashboardProjectionStats {
    /// Events successfully mapped into a TUI-shaped payload.
    pub mapped: u64,
    /// Events the bridge declined to map (TUI does not render them).
    pub unmapped: u64,
    /// Tool / message previews that were truncated by the cap.
    pub truncated: u64,
}

/// Bridge from [`ProjectionEvent`] to TUI-renderable
/// [`DashboardSnippet`].
///
/// The bridge is intentionally small — it does not own any TUI state
/// itself. Callers feed events through `map` and forward the resulting
/// snippets to whatever sink they have (StateHub, a test collector, ...).
#[derive(Debug, Default)]
pub struct DashboardProjection {
    stats: parking_lot::Mutex<DashboardProjectionStats>,
}

/// Render-friendly summary the TUI / non-TUI CLI both consume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardSnippet {
    pub run_id: String,
    pub category: EventCategory,
    pub event_type: String,
    pub plan_id: Option<String>,
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub headline: String,
    pub preview: Option<String>,
    pub timestamp_ms: u64,
}

impl DashboardProjection {
    /// Construct an empty bridge.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Map a [`ProjectionEvent`] to a [`DashboardSnippet`].
    ///
    /// Returns `None` if the event has no TUI-shaped rendering. The
    /// counter is updated in either case.
    pub fn map(&self, event: &ProjectionEvent) -> Option<DashboardSnippet> {
        let mut stats = self.stats.lock();
        let snippet = match event.category {
            EventCategory::Plan
            | EventCategory::Task
            | EventCategory::Run
            | EventCategory::Resume
            | EventCategory::Gate
            | EventCategory::Prompt
            | EventCategory::Merge
            | EventCategory::Retry
            | EventCategory::Dream => Some(self.basic_snippet(event)),
            EventCategory::AgentLifecycle
            | EventCategory::AgentMessage
            | EventCategory::AgentTool => Some(self.agent_snippet(event, &mut stats)),
            EventCategory::Token | EventCategory::Cost => Some(self.usage_snippet(event)),
            EventCategory::Other => None,
        };
        match snippet {
            Some(s) => {
                stats.mapped += 1;
                Some(s)
            }
            None => {
                stats.unmapped += 1;
                None
            }
        }
    }

    /// Snapshot per-bridge counters.
    #[must_use]
    pub fn stats(&self) -> DashboardProjectionStats {
        self.stats.lock().clone()
    }

    fn basic_snippet(&self, event: &ProjectionEvent) -> DashboardSnippet {
        DashboardSnippet {
            run_id: event.run_id.clone(),
            category: event.category,
            event_type: event.event_type.clone(),
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
            agent_id: event.agent_id.clone(),
            headline: format_headline(event),
            preview: None,
            timestamp_ms: event.timestamp_ms,
        }
    }

    fn agent_snippet(
        &self,
        event: &ProjectionEvent,
        stats: &mut DashboardProjectionStats,
    ) -> DashboardSnippet {
        let preview = event.preview.as_ref().map(|p| {
            if p.len() > PROJECTION_OUTPUT_PREVIEW_BYTES {
                stats.truncated += 1;
                let mut truncated = p[..PROJECTION_OUTPUT_PREVIEW_BYTES].to_string();
                truncated.push_str("…");
                truncated
            } else {
                p.clone()
            }
        });
        DashboardSnippet {
            run_id: event.run_id.clone(),
            category: event.category,
            event_type: event.event_type.clone(),
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
            agent_id: event.agent_id.clone(),
            headline: format_headline(event),
            preview,
            timestamp_ms: event.timestamp_ms,
        }
    }

    fn usage_snippet(&self, event: &ProjectionEvent) -> DashboardSnippet {
        DashboardSnippet {
            run_id: event.run_id.clone(),
            category: event.category,
            event_type: event.event_type.clone(),
            plan_id: event.plan_id.clone(),
            task_id: event.task_id.clone(),
            agent_id: event.agent_id.clone(),
            headline: format_headline(event),
            preview: None,
            timestamp_ms: event.timestamp_ms,
        }
    }
}

fn format_headline(event: &ProjectionEvent) -> String {
    match (event.plan_id.as_deref(), event.task_id.as_deref()) {
        (Some(plan), Some(task)) => format!("{}: {} / {}", event.event_type, plan, task),
        (Some(plan), None) => format!("{}: {}", event.event_type, plan),
        (None, Some(task)) => format!("{}: {}", event.event_type, task),
        (None, None) => event.event_type.clone(),
    }
}

/// Convenience: spawn a background task that subscribes to `projection`
/// and forwards every mapped snippet through `sink`.
///
/// `sink` is async to allow callers to push into a `mpsc::Sender`,
/// `StateHub`, or whatever broadcast they expose. The forwarder exits
/// cleanly when the projection broadcast closes.
pub fn forward_to<F, Fut>(
    projection: Arc<crate::runner::projection::Projection>,
    bridge: Arc<DashboardProjection>,
    mut sink: F,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut(DashboardSnippet) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send,
{
    tokio::spawn(async move {
        let mut sub = super::ProjectionSubscriber::new(&projection);
        loop {
            match sub.recv().await {
                Ok(Some(event)) => {
                    if let Some(snippet) = bridge.map(&event) {
                        sink(snippet).await;
                    }
                }
                Ok(None) => break,
                Err(super::SubscribeError::Lagged { dropped }) => {
                    tracing::warn!(dropped, "dashboard projection lagged");
                    continue;
                }
            }
        }
    })
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::projection::ProjectionEvent;
    use crate::runner::types::EventCategory;

    fn event(category: EventCategory, event_type: &str) -> ProjectionEvent {
        ProjectionEvent {
            run_id: "run-1".into(),
            category,
            event_type: event_type.into(),
            timestamp_ms: 1_700_000_000_000,
            plan_id: Some("p1".into()),
            task_id: Some("t1".into()),
            attempt: None,
            agent_id: Some("agent-1".into()),
            severity: None,
            preview: None,
            payload: serde_json::Value::Null,
            coerced: false,
        }
    }

    #[test]
    fn run_plan_task_events_become_basic_snippets() {
        let bridge = DashboardProjection::new();
        for category in [
            EventCategory::Run,
            EventCategory::Plan,
            EventCategory::Task,
            EventCategory::Resume,
            EventCategory::Gate,
            EventCategory::Prompt,
            EventCategory::Merge,
            EventCategory::Retry,
            EventCategory::Dream,
        ] {
            let snip = bridge.map(&event(category, "x")).expect("mapped");
            assert!(snip.headline.contains("p1"));
            assert!(snip.preview.is_none());
        }
        let stats = bridge.stats();
        assert_eq!(stats.mapped, 9);
        assert_eq!(stats.unmapped, 0);
    }

    #[test]
    fn agent_events_carry_preview_and_truncate_when_oversized() {
        let bridge = DashboardProjection::new();
        let mut e = event(EventCategory::AgentMessage, "agent.message_delta");
        e.preview = Some("a".repeat(PROJECTION_OUTPUT_PREVIEW_BYTES + 1024));
        let snip = bridge.map(&e).expect("mapped");
        assert!(snip.preview.is_some());
        assert!(
            snip.preview.unwrap().len() <= PROJECTION_OUTPUT_PREVIEW_BYTES + 4 // ellipsis bytes
        );
        assert_eq!(bridge.stats().truncated, 1);
    }

    #[test]
    fn other_category_events_are_skipped_with_unmapped_increment() {
        let bridge = DashboardProjection::new();
        let snip = bridge.map(&event(EventCategory::Other, "?"));
        assert!(snip.is_none());
        assert_eq!(bridge.stats().unmapped, 1);
    }

    #[test]
    fn headline_omits_missing_plan_or_task_ids() {
        let bridge = DashboardProjection::new();
        let mut e = event(EventCategory::Run, "run.started");
        e.plan_id = None;
        e.task_id = None;
        let snip = bridge.map(&e).unwrap();
        assert_eq!(snip.headline, "run.started");
    }
}
