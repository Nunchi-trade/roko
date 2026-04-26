//! CLI progress projection — turns [`ProjectionEvent`]s into compact
//! lines for users running `roko plan run` without a TUI.
//!
//! ## Design
//!
//! The non-TUI path historically dumped raw agent stdout. That hides
//! plan progress, gate verdicts, and retry decisions behind unstructured
//! text. The CLI progress projection consumes the same
//! [`ProjectionEvent`] stream the TUI sees and renders a pure-text
//! progress feed:
//!
//! ```text
//! ▶ plan p1 started
//! ▶ task t1 started (attempt 0)
//! …
//! ✓ task t1 completed in 12.3s
//! ✗ gate clippy failed for t2 (rung 2, 4.2s)
//! ↻ retry t2 in 4s (attempt 1)
//! ✓ plan p1 completed: 5/5 tasks ($0.05, 30.2s)
//! ```
//!
//! ## Non-rendering / library mode
//!
//! [`CliProgressPrinter::format`] is pure — it returns the rendered line
//! without writing it. This makes the printer trivially testable and
//! lets callers feed the lines into `tracing::info!` instead of stdout
//! when they need structured logs.

use crate::runner::projection::ProjectionEvent;
use crate::runner::types::{EventCategory, StderrSeverity};

/// Counters for diagnostics + tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliProgressStats {
    /// Lines successfully rendered (non-empty output).
    pub rendered: u64,
    /// Events the printer chose to suppress (e.g. tool-output deltas).
    pub suppressed: u64,
}

#[derive(Debug, Default)]
pub struct CliProgressPrinter {
    stats: parking_lot::Mutex<CliProgressStats>,
}

impl CliProgressPrinter {
    /// Construct a printer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Render an event. Returns `None` for events the printer chooses to
    /// suppress (e.g. token-delta noise).
    pub fn format(&self, event: &ProjectionEvent) -> Option<String> {
        let mut stats = self.stats.lock();
        let line = match event.category {
            EventCategory::Run => Some(format_run(event)),
            EventCategory::Plan => Some(format_plan(event)),
            EventCategory::Task => Some(format_task(event)),
            EventCategory::Gate => Some(format_gate(event)),
            EventCategory::Retry => Some(format_retry(event)),
            EventCategory::Dream => Some(format_dream(event)),
            EventCategory::Resume => Some(format_resume(event)),
            EventCategory::Cost => Some(format_cost(event)),
            EventCategory::AgentLifecycle => format_agent_lifecycle(event),
            EventCategory::AgentMessage
            | EventCategory::AgentTool
            | EventCategory::Token
            | EventCategory::Other => None,
        };
        if line.is_some() {
            stats.rendered += 1;
        } else {
            stats.suppressed += 1;
        }
        line
    }

    /// Snapshot per-printer counters.
    #[must_use]
    pub fn stats(&self) -> CliProgressStats {
        self.stats.lock().clone()
    }
}

fn format_run(event: &ProjectionEvent) -> String {
    match event.event_type.as_str() {
        "run.started" => format!("▶ run {} started", event.run_id),
        "run.completed" => format!("✓ run {} completed", event.run_id),
        other => format!("• run {other}"),
    }
}

fn format_plan(event: &ProjectionEvent) -> String {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    match event.event_type.as_str() {
        "plan.started" => format!("▶ plan {plan} started"),
        "plan.completed" => format!("✓ plan {plan} completed"),
        other => format!("• plan {plan}: {other}"),
    }
}

fn format_task(event: &ProjectionEvent) -> String {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    let task = event.task_id.as_deref().unwrap_or("?");
    let attempt = event.attempt.map(|a| format!(" (attempt {a})")).unwrap_or_default();
    match event.event_type.as_str() {
        "task.attempt.started" => format!("▶ task {plan}/{task} started{attempt}"),
        "task.attempt.completed" => format!("✓ task {plan}/{task} completed{attempt}"),
        other => format!("• task {plan}/{task}: {other}{attempt}"),
    }
}

fn format_gate(event: &ProjectionEvent) -> String {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    let task = event.task_id.as_deref().unwrap_or("?");
    let passed = event
        .payload
        .get("passed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let rung = event
        .payload
        .get("rung")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if passed {
        format!("✓ gate rung {rung} passed for {plan}/{task}")
    } else {
        format!("✗ gate rung {rung} failed for {plan}/{task}")
    }
}

fn format_retry(event: &ProjectionEvent) -> String {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    let task = event.task_id.as_deref().unwrap_or("?");
    let attempt = event
        .payload
        .get("attempt")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let backoff_secs = event
        .payload
        .get("backoff_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    format!("↻ retry {plan}/{task} in {backoff_secs}s (attempt {attempt})")
}

fn format_dream(_event: &ProjectionEvent) -> String {
    "✦ dream cycle triggered".to_string()
}

fn format_resume(event: &ProjectionEvent) -> String {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    format!("⤴ resume marker for plan {plan}")
}

fn format_cost(event: &ProjectionEvent) -> String {
    let cost = event
        .payload
        .get("total_cost_usd")
        .and_then(|v| v.as_f64())
        .unwrap_or_default();
    format!("$ cost {cost:.4}")
}

fn format_agent_lifecycle(event: &ProjectionEvent) -> Option<String> {
    let plan = event.plan_id.as_deref().unwrap_or("?");
    let task = event.task_id.as_deref().unwrap_or("?");
    match event.event_type.as_str() {
        "agent.error" => Some(format!(
            "✗ agent error in {plan}/{task}: {}",
            event
                .payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        )),
        "agent.exited" => {
            let severity = event.severity.unwrap_or(StderrSeverity::Infra);
            let exit_code = event
                .payload
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .unwrap_or_default();
            Some(format!(
                "{} agent exit {exit_code} ({}) in {plan}/{task}",
                severity_glyph(severity),
                severity.as_str()
            ))
        }
        // Suppress lifecycle noise the user does not need to read.
        _ => None,
    }
}

fn severity_glyph(severity: StderrSeverity) -> &'static str {
    match severity {
        StderrSeverity::Warning => "!",
        StderrSeverity::Error => "✗",
        StderrSeverity::Infra => "·",
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::projection::ProjectionEvent;

    fn event(category: EventCategory, event_type: &str) -> ProjectionEvent {
        ProjectionEvent {
            run_id: "run-1".into(),
            category,
            event_type: event_type.into(),
            timestamp_ms: 1,
            plan_id: Some("p1".into()),
            task_id: Some("t1".into()),
            attempt: Some(0),
            agent_id: None,
            severity: None,
            preview: None,
            payload: serde_json::Value::Null,
            coerced: false,
        }
    }

    #[test]
    fn task_started_renders_progress_glyph() {
        let printer = CliProgressPrinter::new();
        let line = printer.format(&event(EventCategory::Task, "task.attempt.started"));
        let line = line.expect("rendered");
        assert!(line.contains("▶"));
        assert!(line.contains("p1/t1"));
    }

    #[test]
    fn gate_failure_renders_failed_glyph() {
        let printer = CliProgressPrinter::new();
        let mut e = event(EventCategory::Gate, "gate.completed");
        e.payload = serde_json::json!({ "passed": false, "rung": 2 });
        let line = printer.format(&e).unwrap();
        assert!(line.starts_with("✗"));
        assert!(line.contains("rung 2"));
    }

    #[test]
    fn retry_renders_circle_arrow_with_backoff() {
        let printer = CliProgressPrinter::new();
        let mut e = event(EventCategory::Retry, "retry.decision");
        e.payload = serde_json::json!({ "attempt": 2, "backoff_secs": 4 });
        let line = printer.format(&e).unwrap();
        assert!(line.starts_with("↻"));
        assert!(line.contains("4s"));
        assert!(line.contains("attempt 2"));
    }

    #[test]
    fn token_events_are_suppressed_to_avoid_spam() {
        let printer = CliProgressPrinter::new();
        let line = printer.format(&event(EventCategory::Token, "agent.token_usage"));
        assert!(line.is_none());
        assert_eq!(printer.stats().suppressed, 1);
    }

    #[test]
    fn agent_message_deltas_are_suppressed() {
        let printer = CliProgressPrinter::new();
        let line = printer.format(&event(
            EventCategory::AgentMessage,
            "agent.message_delta",
        ));
        assert!(line.is_none());
    }

    #[test]
    fn agent_error_renders_an_explicit_failure_line() {
        let printer = CliProgressPrinter::new();
        let mut e = event(EventCategory::AgentLifecycle, "agent.error");
        e.payload = serde_json::json!({ "message": "OOM killed" });
        let line = printer.format(&e).unwrap();
        assert!(line.starts_with("✗"));
        assert!(line.contains("OOM killed"));
    }

    #[test]
    fn other_category_event_does_not_render_and_increments_suppressed() {
        let printer = CliProgressPrinter::new();
        let line = printer.format(&event(EventCategory::Other, "x"));
        assert!(line.is_none());
        assert_eq!(printer.stats().suppressed, 1);
        assert_eq!(printer.stats().rendered, 0);
    }
}
