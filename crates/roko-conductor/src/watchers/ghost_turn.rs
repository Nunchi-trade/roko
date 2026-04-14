//! Ghost turn watcher: detects agent turns with zero meaningful output.
//!
//! An agent turn that produces no meaningful output and no file changes is
//! burning tokens without progress. After [`MAX_GHOST_TURNS`] consecutive
//! ghost turns, this watcher fires a restart signal.

use roko_core::{Body, Context, Engram, Kind, Policy};
use serde::Deserialize;

/// Maximum consecutive wasted turns before firing.
pub const MAX_GHOST_TURNS: usize = 3;

/// Tag key used to mark an intervention signal from this watcher.
pub const WATCHER_NAME: &str = "ghost-turn";

/// Custom conductor signal kind emitted for wasted-cost turns.
pub const TURN_SIGNAL_KIND: &str = "conductor.ghost_turn";

#[derive(Debug, Clone, Deserialize)]
struct GhostTurnEvent {
    plan_id: String,
    task: String,
    role: String,
    model: String,
    cost_usd: f64,
    duration_ms: u64,
    changed_files_before: Vec<String>,
    changed_files_after: Vec<String>,
    net_new_changes: usize,
    output_meaningful: bool,
    wasted_cost: bool,
}

/// Detects agent turns that produce no meaningful output.
///
/// Scans the signal stream for consecutive `conductor.ghost_turn` signals
/// emitted by the CLI when a turn produced no meaningful output and no file
/// changes. Fires when the count reaches [`MAX_GHOST_TURNS`].
#[derive(Debug, Clone)]
pub struct GhostTurnWatcher {
    /// Consecutive ghost turns before firing.
    max_ghost_turns: usize,
}

impl Default for GhostTurnWatcher {
    fn default() -> Self {
        Self {
            max_ghost_turns: MAX_GHOST_TURNS,
        }
    }
}

impl GhostTurnWatcher {
    /// Create a watcher with a custom threshold.
    #[must_use]
    pub const fn new(max_ghost_turns: usize) -> Self {
        Self { max_ghost_turns }
    }
}

fn is_ghost_turn_signal(signal: &Engram) -> bool {
    matches!(signal.kind, Kind::Custom(ref kind) if kind == TURN_SIGNAL_KIND)
}

fn extract_ghost_turn_event(signal: &Engram) -> Option<GhostTurnEvent> {
    if !is_ghost_turn_signal(signal) {
        return None;
    }

    signal
        .body
        .as_json::<GhostTurnEvent>()
        .ok()
        .and_then(|event| {
            if event.output_meaningful || event.net_new_changes != 0 {
                None
            } else {
                Some(event)
            }
        })
}

impl Policy for GhostTurnWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        // Count consecutive wasted turns from the end of the stream.
        let mut consecutive = 0usize;
        let mut last_event: Option<GhostTurnEvent> = None;

        for signal in stream.iter().rev() {
            let Some(event) = extract_ghost_turn_event(signal) else {
                break;
            };
            consecutive += 1;
            if last_event.is_none() {
                last_event = Some(event);
            }
        }

        if consecutive >= self.max_ghost_turns {
            let event = last_event.expect("consecutive implies at least one event");
            let GhostTurnEvent {
                plan_id,
                task,
                role,
                model,
                cost_usd,
                duration_ms,
                changed_files_before,
                changed_files_after,
                net_new_changes: _,
                output_meaningful: _,
                wasted_cost,
            } = event;
            let before_count = changed_files_before.len();
            let after_count = changed_files_after.len();
            let reason = format!(
                "wasted cost: {} consecutive turns with no meaningful output and no net file changes (last=${cost_usd:.4}, {duration_ms}ms, {plan_id}/{task}; role={role}, model={model}, before={before_count}, after={after_count})",
                consecutive
            );
            vec![
                Engram::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(reason))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("consecutive", consecutive.to_string())
                    .tag("plan_id", plan_id)
                    .tag("task_id", task)
                    .tag("model", model)
                    .tag("role", role)
                    .tag("cost_usd", format!("{cost_usd:.4}"))
                    .tag("duration_ms", duration_ms.to_string())
                    .tag("file_changes_before", before_count.to_string())
                    .tag("file_changes_after", after_count.to_string())
                    .tag("wasted_cost", wasted_cost.to_string())
                    .build(),
            ]
        } else {
            Vec::new()
        }
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ghost_turn_signal(cost_usd: f64, changed_files: Vec<&str>) -> Engram {
        let changed_files_before = changed_files.clone();
        let changed_files_after = changed_files;
        let body = Body::from_json(&serde_json::json!({
            "plan_id": "plan-1",
            "task": "task-1",
            "role": "Implementer",
            "model": "claude-sonnet-4-6",
            "cost_usd": cost_usd,
            "duration_ms": 1234,
            "changed_files_before": changed_files_before,
            "changed_files_after": changed_files_after,
            "net_new_changes": 0,
            "output_meaningful": false,
            "wasted_cost": true,
        }))
        .expect("serialize ghost turn event");
        Engram::builder(Kind::Custom(TURN_SIGNAL_KIND.into()))
            .body(body)
            .build()
    }

    fn non_ghost_turn_signal() -> Engram {
        let body = Body::from_json(&serde_json::json!({
            "plan_id": "plan-1",
            "task": "task-1",
            "role": "Implementer",
            "model": "claude-sonnet-4-6",
            "cost_usd": 0.0,
            "duration_ms": 1234,
            "changed_files_before": [],
            "changed_files_after": ["src/lib.rs"],
            "net_new_changes": 1,
            "output_meaningful": true,
            "wasted_cost": false,
        }))
        .expect("serialize non-ghost turn event");
        Engram::builder(Kind::Custom(TURN_SIGNAL_KIND.into()))
            .body(body)
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = GhostTurnWatcher::default();
        let out = w.decide(&[], &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn real_output_no_fire() {
        let w = GhostTurnWatcher::default();
        let stream = vec![non_ghost_turn_signal(), non_ghost_turn_signal()];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_turn_signal(1.25, vec![]),
            ghost_turn_signal(1.10, vec![]),
        ]; // 2 < 3
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }

    #[test]
    fn at_threshold_fires() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_turn_signal(1.25, vec![]),
            ghost_turn_signal(1.10, vec![]),
            ghost_turn_signal(0.90, vec![]),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn empty_changed_files_counts_as_ghost() {
        let w = GhostTurnWatcher::new(2);
        let stream = vec![
            ghost_turn_signal(0.5, vec![]),
            ghost_turn_signal(0.4, vec![]),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn meaningful_output_breaks_consecutive_chain() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_turn_signal(0.5, vec![]),
            ghost_turn_signal(0.4, vec![]),
            ghost_turn_signal(0.3, vec![]),
            non_ghost_turn_signal(), // breaks the chain
            ghost_turn_signal(0.2, vec![]),
            ghost_turn_signal(0.1, vec![]),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty()); // only 2 consecutive at the end
    }

    #[test]
    fn non_agent_output_breaks_chain() {
        let w = GhostTurnWatcher::default();
        let stream = vec![
            ghost_turn_signal(0.5, vec![]),
            ghost_turn_signal(0.4, vec![]),
            Engram::builder(Kind::GateVerdict)
                .body(Body::empty())
                .build(),
            ghost_turn_signal(0.3, vec![]),
            ghost_turn_signal(0.2, vec![]),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty()); // gate verdict breaks the chain
    }

    #[test]
    fn changed_files_prevent_fire() {
        let w = GhostTurnWatcher::new(1);
        let stream = vec![
            Engram::builder(Kind::Custom(TURN_SIGNAL_KIND.into()))
                .body(
                    Body::from_json(&serde_json::json!({
                        "plan_id": "plan-1",
                        "task": "task-1",
                        "role": "Implementer",
                        "model": "claude-sonnet-4-6",
                        "cost_usd": 0.75,
                        "duration_ms": 1234,
                        "changed_files_before": ["src/lib.rs"],
                        "changed_files_after": ["src/lib.rs", "src/main.rs"],
                        "net_new_changes": 1,
                        "output_meaningful": false,
                        "wasted_cost": true,
                    }))
                    .expect("serialize changed-files event"),
                )
                .build(),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert!(out.is_empty());
    }
}
