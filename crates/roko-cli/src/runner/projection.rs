//! Projection facade — single normalized event vocabulary that the TUI,
//! HTTP/SSE, and non-TUI CLI all consume.
//!
//! ```text
//!   RunnerEvent ─┐
//!   AgentEvent  ─┼─► projection::publish ─► ProjectionEvent (broadcast)
//!   raw event    ┘                         + bounded dashboard snapshot
//!                                          + dropped/coerced counters
//! ```
//!
//! The projection is intentionally cheap and lock-light:
//!
//! - `events` — `tokio::sync::broadcast` — backpressure handled by counting
//!   dropped subscribers, never blocking the producer.
//! - `dashboard` — `Mutex<DashboardSnapshot>` keeping the last N events with
//!   tool output truncated to 4 KB so we never store megabyte-scale payloads.
//! - `dropped` / `coerced` — `AtomicU64` counters exposed via [`Projection::counters`].

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::types::{AgentEvent, EventCategory, RunnerEvent, StderrSeverity};

/// Maximum number of `ProjectionEvent`s buffered in the broadcast channel.
const PROJECTION_CHANNEL_CAPACITY: usize = 1024;
/// Maximum events retained in the bounded dashboard snapshot.
const DASHBOARD_MAX_EVENTS: usize = 200;
/// Maximum bytes preserved for tool / agent output payload previews.
pub const PROJECTION_OUTPUT_PREVIEW_BYTES: usize = 4096;

/// One normalized event delivered through the projection facade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionEvent {
    /// Stable run id this event belongs to.
    pub run_id: String,
    /// Provider-agnostic event category.
    pub category: EventCategory,
    /// Stable event type string (e.g. `agent.tool_call`, `gate.completed`).
    pub event_type: String,
    /// Wall-clock timestamp in milliseconds since the Unix epoch.
    pub timestamp_ms: u64,
    /// Plan id, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// Task id, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Task attempt number, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Agent id, if this event is associated with an agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Stderr classification, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<StderrSeverity>,
    /// Truncated output preview (raw tool / message payload, ≤ 4 KB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// Free-form structured payload — used by HTTP/SSE for richer rendering.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
    /// Whether this event was bucketed into [`EventCategory::Other`] from an
    /// unknown source category.
    #[serde(default, skip_serializing_if = "is_false")]
    pub coerced: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Raw event input accepted by [`Projection::publish`].
#[derive(Debug, Clone)]
pub enum RawRuntimeEvent {
    Runner(RunnerEvent),
    Agent(AgentEventInput),
    /// Custom event from another subsystem (dreams, knowledge, retry, etc.).
    /// `event_type` strings that don't match known prefixes are coerced into
    /// [`EventCategory::Other`] and the `coerced` counter is incremented.
    Custom {
        run_id: String,
        event_type: String,
        timestamp_ms: u64,
        plan_id: Option<String>,
        task_id: Option<String>,
        attempt: Option<u32>,
        agent_id: Option<String>,
        severity: Option<StderrSeverity>,
        preview: Option<String>,
        payload: serde_json::Value,
    },
}

/// Wrapper around an `AgentEvent` plus the run/plan/task context.
#[derive(Debug, Clone)]
pub struct AgentEventInput {
    pub run_id: String,
    pub plan_id: String,
    pub task_id: String,
    pub attempt: u32,
    pub agent_id: Option<String>,
    pub timestamp_ms: u64,
    pub event: AgentEvent,
}

/// Errors returned by the projection facade.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// Broadcast send failed because there are no live subscribers. Not fatal —
    /// the caller can ignore this; counters still tick.
    #[error("no live subscribers")]
    NoSubscribers,
}

/// Bounded snapshot of the most recent projection events, used by the TUI
/// and dashboards that need a rolling window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    /// Last N events in publish order (oldest first).
    pub events: VecDeque<ProjectionEvent>,
}

/// Counters exposed for diagnostics and tests.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProjectionCounters {
    /// Events the projection saw but could not deliver to any subscriber.
    pub dropped: u64,
    /// Events whose category had to be inferred / fell back to `Other`.
    pub coerced: u64,
    /// Events accepted and broadcast.
    pub published: u64,
}

/// Projection facade.
///
/// One projection per run. Cloning is cheap-ish (broadcast handles do not
/// clone the buffer), but most callers should hold a single shared instance.
pub struct Projection {
    run_id: String,
    events: broadcast::Sender<ProjectionEvent>,
    dropped: AtomicU64,
    coerced: AtomicU64,
    published: AtomicU64,
    dashboard: Mutex<DashboardSnapshot>,
}

impl Projection {
    /// Create a new projection bound to a specific `run_id`.
    pub fn new(run_id: impl Into<String>) -> Self {
        let (tx, _rx) = broadcast::channel(PROJECTION_CHANNEL_CAPACITY);
        Self {
            run_id: run_id.into(),
            events: tx,
            dropped: AtomicU64::new(0),
            coerced: AtomicU64::new(0),
            published: AtomicU64::new(0),
            dashboard: Mutex::new(DashboardSnapshot::default()),
        }
    }

    /// Run id this projection is keyed under.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Subscribe to live projection events. The returned receiver lags
    /// independently of other subscribers.
    pub fn subscribe(&self) -> broadcast::Receiver<ProjectionEvent> {
        self.events.subscribe()
    }

    /// Snapshot of the current bounded dashboard buffer.
    pub fn dashboard_snapshot(&self) -> DashboardSnapshot {
        self.dashboard.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// Current counters.
    pub fn counters(&self) -> ProjectionCounters {
        ProjectionCounters {
            dropped: self.dropped.load(Ordering::Relaxed),
            coerced: self.coerced.load(Ordering::Relaxed),
            published: self.published.load(Ordering::Relaxed),
        }
    }

    /// Normalize and publish a raw runtime event.
    pub fn publish(&self, raw: RawRuntimeEvent) -> Result<(), ProjectionError> {
        let event = self.normalize(raw);
        if event.coerced {
            self.coerced.fetch_add(1, Ordering::Relaxed);
        }
        // Update the bounded dashboard snapshot first — we want the in-memory
        // ring to reflect the latest event even if no subscribers exist.
        if let Ok(mut dash) = self.dashboard.lock() {
            dash.events.push_back(event.clone());
            while dash.events.len() > DASHBOARD_MAX_EVENTS {
                dash.events.pop_front();
            }
        }
        match self.events.send(event) {
            Ok(_) => {
                self.published.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(_) => {
                // No subscribers right now — count as dropped but don't fail.
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Err(ProjectionError::NoSubscribers)
            }
        }
    }

    /// Convenience helper for emitting agent events.
    pub fn publish_agent_event(&self, input: AgentEventInput) -> Result<(), ProjectionError> {
        self.publish(RawRuntimeEvent::Agent(input))
    }

    /// Convenience helper for emitting runner lifecycle events.
    pub fn publish_runner_event(&self, event: RunnerEvent) -> Result<(), ProjectionError> {
        self.publish(RawRuntimeEvent::Runner(event))
    }

    fn normalize(&self, raw: RawRuntimeEvent) -> ProjectionEvent {
        match raw {
            RawRuntimeEvent::Runner(event) => self.from_runner(event),
            RawRuntimeEvent::Agent(input) => self.from_agent(input),
            RawRuntimeEvent::Custom {
                run_id,
                event_type,
                timestamp_ms,
                plan_id,
                task_id,
                attempt,
                agent_id,
                severity,
                preview,
                payload,
            } => {
                let (category, coerced) = EventCategory::from_event_type(&event_type);
                ProjectionEvent {
                    run_id,
                    category,
                    event_type,
                    timestamp_ms,
                    plan_id,
                    task_id,
                    attempt,
                    agent_id,
                    severity,
                    preview: preview.map(truncate_preview),
                    payload,
                    coerced,
                }
            }
        }
    }

    fn from_runner(&self, event: RunnerEvent) -> ProjectionEvent {
        let category = EventCategory::from_runner_event(&event);
        let event_type = event.event_type().to_string();
        let timestamp_ms = event.timestamp_ms();
        let plan_id = event.plan_id().map(str::to_string);
        let task_id = event.task_id().map(str::to_string);
        let run_id = match &event {
            RunnerEvent::ResumeMarker { run_id, .. }
            | RunnerEvent::RunStarted { run_id, .. }
            | RunnerEvent::RunCompleted { run_id, .. }
            | RunnerEvent::PlanStarted { run_id, .. }
            | RunnerEvent::PlanCompleted { run_id, .. }
            | RunnerEvent::TaskAttemptStarted { run_id, .. }
            | RunnerEvent::TaskAttemptCompleted { run_id, .. }
            | RunnerEvent::AgentDispatchStarted { run_id, .. }
            | RunnerEvent::AgentDispatchCompleted { run_id, .. }
            | RunnerEvent::AgentCompleted { run_id, .. }
            | RunnerEvent::GateDispatchStarted { run_id, .. }
            | RunnerEvent::GateCompleted { run_id, .. }
            | RunnerEvent::PromptAssembled { run_id, .. }
            | RunnerEvent::MergeBackendCompleted { run_id, .. }
            | RunnerEvent::RetryDecision { run_id, .. } => run_id.clone(),
        };
        let attempt = match &event {
            RunnerEvent::TaskAttemptStarted { attempt, .. }
            | RunnerEvent::TaskAttemptCompleted { attempt, .. }
            | RunnerEvent::AgentDispatchStarted { attempt, .. }
            | RunnerEvent::AgentDispatchCompleted { attempt, .. }
            | RunnerEvent::AgentCompleted { attempt, .. }
            | RunnerEvent::GateDispatchStarted { attempt, .. }
            | RunnerEvent::GateCompleted { attempt, .. }
            | RunnerEvent::PromptAssembled { attempt, .. }
            | RunnerEvent::MergeBackendCompleted { attempt, .. }
            | RunnerEvent::RetryDecision { attempt, .. } => Some(attempt.attempt),
            _ => None,
        };
        let agent_id = match &event {
            RunnerEvent::AgentDispatchStarted { agent_id, .. }
            | RunnerEvent::AgentDispatchCompleted { agent_id, .. }
            | RunnerEvent::AgentCompleted { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        };
        let preview = match &event {
            RunnerEvent::GateCompleted { output, .. } => Some(truncate_preview(output.clone())),
            RunnerEvent::MergeBackendCompleted { output, .. } => {
                Some(truncate_preview(output.clone()))
            }
            RunnerEvent::PromptAssembled {
                estimated_tokens,
                included_sections,
                dropped_sections,
                ..
            } => Some(truncate_preview(format!(
                "estimated_tokens={estimated_tokens} included={} dropped={}",
                included_sections.len(),
                dropped_sections.len()
            ))),
            _ => None,
        };

        let payload = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);

        ProjectionEvent {
            run_id,
            category,
            event_type,
            timestamp_ms,
            plan_id,
            task_id,
            attempt: attempt.map(|a| a.max(1)),
            agent_id,
            severity: None,
            preview,
            payload,
            coerced: false,
        }
    }

    fn from_agent(&self, input: AgentEventInput) -> ProjectionEvent {
        let AgentEventInput {
            run_id,
            plan_id,
            task_id,
            attempt,
            agent_id,
            timestamp_ms,
            event,
        } = input;

        let category = EventCategory::from_agent_event(&event);
        let event_type = event.event_type().to_string();

        let mut preview = None;
        let mut severity = None;
        let payload = match &event {
            AgentEvent::Started {
                agent_id: started_id,
                provider,
                model,
                pid,
            } => serde_json::json!({
                "agent_id": started_id,
                "provider": provider,
                "model": model,
                "pid": pid,
            }),
            AgentEvent::SystemInit { session_id, model } => {
                serde_json::json!({"session_id": session_id, "model": model})
            }
            AgentEvent::MessageDelta { text } => {
                preview = Some(truncate_preview(text.clone()));
                serde_json::json!({"text_len": text.len()})
            }
            AgentEvent::ToolCall { id, name } => {
                serde_json::json!({"tool_call_id": id, "tool_name": name})
            }
            AgentEvent::ToolOutput { id, output } => {
                preview = Some(truncate_preview(output.clone()));
                serde_json::json!({
                    "tool_call_id": id,
                    "output_bytes": output.len(),
                })
            }
            AgentEvent::TokenUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => serde_json::json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_read_tokens": cache_read_tokens,
                "cache_write_tokens": cache_write_tokens,
            }),
            AgentEvent::TurnCompleted {
                session_id,
                total_cost_usd,
                num_turns,
                is_error,
            } => serde_json::json!({
                "session_id": session_id,
                "total_cost_usd": total_cost_usd,
                "num_turns": num_turns,
                "is_error": is_error,
            }),
            AgentEvent::Error { message } => {
                let sev = StderrSeverity::from_message(message);
                severity = Some(sev);
                preview = Some(truncate_preview(message.clone()));
                serde_json::json!({
                    "message_len": message.len(),
                    "severity": sev.as_str(),
                })
            }
            AgentEvent::Exited { exit_code } => {
                serde_json::json!({"exit_code": exit_code})
            }
        };

        ProjectionEvent {
            run_id,
            category,
            event_type,
            timestamp_ms,
            plan_id: Some(plan_id),
            task_id: Some(task_id),
            attempt: Some(attempt.max(1)),
            agent_id,
            severity,
            preview,
            payload,
            coerced: false,
        }
    }
}

/// Truncate a string preview to `PROJECTION_OUTPUT_PREVIEW_BYTES` bytes,
/// preserving UTF-8 char boundaries.
pub fn truncate_preview(mut s: String) -> String {
    if s.len() <= PROJECTION_OUTPUT_PREVIEW_BYTES {
        return s;
    }
    let mut end = PROJECTION_OUTPUT_PREVIEW_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str("…[truncated]");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::types::TaskAttemptRef;

    #[tokio::test]
    async fn publish_runner_event_emits_projection_event() {
        let projection = Projection::new("run-1");
        let mut rx = projection.subscribe();

        let plan_started = RunnerEvent::plan_started("run-1", "plan-a");
        projection
            .publish_runner_event(plan_started)
            .expect("publish ok");

        let event = rx.recv().await.expect("receive event");
        assert_eq!(event.run_id, "run-1");
        assert_eq!(event.category, EventCategory::Plan);
        assert_eq!(event.plan_id.as_deref(), Some("plan-a"));
        let counters = projection.counters();
        assert_eq!(counters.published, 1);
        assert_eq!(counters.dropped, 0);
        assert_eq!(counters.coerced, 0);
    }

    #[test]
    fn publish_without_subscribers_increments_dropped() {
        let projection = Projection::new("run-1");
        let res = projection.publish_runner_event(RunnerEvent::plan_started("run-1", "plan-x"));
        assert!(matches!(res, Err(ProjectionError::NoSubscribers)));
        let counters = projection.counters();
        assert_eq!(counters.dropped, 1);
        assert_eq!(counters.published, 0);
    }

    #[test]
    fn agent_event_with_large_output_is_truncated() {
        let projection = Projection::new("run-1");
        let mut _rx = projection.subscribe();
        let big = "x".repeat(PROJECTION_OUTPUT_PREVIEW_BYTES * 4);
        let event = AgentEvent::ToolOutput {
            id: "t".into(),
            output: big.clone(),
        };
        projection
            .publish_agent_event(AgentEventInput {
                run_id: "run-1".into(),
                plan_id: "p".into(),
                task_id: "t".into(),
                attempt: 1,
                agent_id: Some("p/t".into()),
                timestamp_ms: 0,
                event,
            })
            .unwrap();
        let snap = projection.dashboard_snapshot();
        let preview = snap.events.back().and_then(|e| e.preview.clone()).unwrap();
        assert!(preview.len() <= PROJECTION_OUTPUT_PREVIEW_BYTES + "…[truncated]".len());
    }

    #[test]
    fn coerced_event_increments_counter() {
        let projection = Projection::new("run-1");
        let mut _rx = projection.subscribe();
        projection
            .publish(RawRuntimeEvent::Custom {
                run_id: "run-1".into(),
                event_type: "weird.thing".into(),
                timestamp_ms: 1,
                plan_id: None,
                task_id: None,
                attempt: None,
                agent_id: None,
                severity: None,
                preview: None,
                payload: serde_json::Value::Null,
            })
            .unwrap();
        let counters = projection.counters();
        assert_eq!(counters.coerced, 1);
    }

    #[test]
    fn dashboard_snapshot_is_bounded() {
        let projection = Projection::new("run-1");
        let mut _rx = projection.subscribe();
        for i in 0..(DASHBOARD_MAX_EVENTS + 50) {
            let event = RunnerEvent::TaskAttemptStarted {
                timestamp: chrono::Utc::now().to_rfc3339(),
                timestamp_ms: i as u64,
                run_id: "run-1".into(),
                attempt: TaskAttemptRef::new("plan", format!("task-{i}"), 1),
                title: "t".into(),
                status: super::super::types::TaskAttemptStatus::Started,
            };
            projection.publish_runner_event(event).unwrap();
        }
        let snap = projection.dashboard_snapshot();
        assert!(snap.events.len() <= DASHBOARD_MAX_EVENTS);
    }
}
