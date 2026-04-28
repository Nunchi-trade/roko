//! RuntimeProjection -- reconstructs workflow state from JSONL event logs.
//!
//! Reads the JSONL file written by JsonlLogger and builds a snapshot of the
//! current state for each run_id. Used for resume and dashboard views.

use crate::effect_driver::Result;
use roko_core::runtime_event::{RuntimeEvent, RuntimeEventEnvelope};
use std::collections::HashMap;
use std::path::Path;

/// Summary of a workflow run reconstructed from events.
#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    /// Workflow run id.
    pub run_id: String,
    /// Workflow template label.
    pub template: Option<String>,
    /// Original user prompt.
    pub prompt: Option<String>,
    /// Current phase label.
    pub current_phase: Option<String>,
    /// Phase labels visited in event order.
    pub phases_visited: Vec<String>,
    /// Gate names that passed.
    pub gates_passed: Vec<String>,
    /// Gate names that failed.
    pub gates_failed: Vec<String>,
    /// Number of agent spawn events observed.
    pub agents_spawned: u32,
    /// Whether the workflow emitted a completion event.
    pub is_complete: bool,
    /// Final workflow outcome, when present.
    pub outcome: Option<String>,
    /// Number of completed agent turns observed.
    pub agents_completed: u32,
    /// Number of failed agent turns observed.
    pub agents_failed: u32,
    /// Cumulative tokens used across all agent calls (parsed from event log).
    pub total_tokens: u64,
    /// Cumulative cost in USD across all agent calls (parsed from event log).
    pub total_cost_usd: f64,
    /// Number of feedback_recorded events observed.
    pub feedback_count: u32,
    /// Last state checkpoint path recorded.
    pub last_checkpoint: Option<String>,
    /// Errors from agent_failed events.
    pub agent_errors: Vec<String>,
}

/// Reads JSONL event logs and produces per-run summaries.
pub struct RuntimeProjection;

impl RuntimeProjection {
    /// Read the event log and produce summaries for all runs.
    pub fn from_file(path: &Path) -> Result<HashMap<String, RunSummary>> {
        let mut runs: HashMap<String, RunSummary> = HashMap::new();

        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(runs),
            Err(err) => return Err(err.into()),
        };

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let envelope: RuntimeEventEnvelope = match serde_json::from_str(line) {
                Ok(envelope) => envelope,
                Err(_) => continue,
            };

            let run_id = envelope.run_id.clone();
            let summary = runs.entry(run_id.clone()).or_insert_with(|| RunSummary {
                run_id: run_id.clone(),
                ..Default::default()
            });

            apply_event(summary, &envelope.payload);
        }

        Ok(runs)
    }

    /// Get summary for a specific run.
    pub fn for_run(path: &Path, run_id: &str) -> Result<Option<RunSummary>> {
        let runs = Self::from_file(path)?;
        Ok(runs.into_iter().find_map(
            |(id, summary)| {
                if id == run_id { Some(summary) } else { None }
            },
        ))
    }
}

fn apply_event(summary: &mut RunSummary, event: &RuntimeEvent) {
    match event {
        RuntimeEvent::WorkflowStarted {
            template, prompt, ..
        } => {
            summary.template = Some(template.clone());
            summary.prompt = Some(prompt.clone());
        }
        RuntimeEvent::PhaseTransition { to, .. } => {
            summary.current_phase = Some(to.clone());
            summary.phases_visited.push(to.clone());
        }
        RuntimeEvent::GatePassed { gate_name, .. } => {
            summary.gates_passed.push(gate_name.clone());
        }
        RuntimeEvent::GateFailed { gate_name, .. } => {
            summary.gates_failed.push(gate_name.clone());
        }
        RuntimeEvent::AgentSpawned { .. } => {
            summary.agents_spawned += 1;
        }
        RuntimeEvent::WorkflowCompleted { outcome, .. } => {
            summary.is_complete = true;
            summary.outcome = Some(outcome.to_string());
        }
        RuntimeEvent::AgentCompleted {
            tokens_used,
            cost_usd,
            ..
        } => {
            summary.agents_completed += 1;
            summary.total_tokens += tokens_used;
            summary.total_cost_usd += cost_usd;
        }
        RuntimeEvent::AgentFailed { error, .. } => {
            summary.agents_failed += 1;
            summary.agent_errors.push(error.clone());
        }
        RuntimeEvent::FeedbackRecorded { .. } => {
            summary.feedback_count += 1;
        }
        RuntimeEvent::StateCheckpointed { path, .. } => {
            summary.last_checkpoint = Some(path.clone());
        }
        RuntimeEvent::AgentOutput { .. } => {
            // AgentOutput is a streaming event; no aggregate RunSummary field yet.
        }
        RuntimeEvent::GateStarted { .. } => {
            // GateStarted is informational; pass/fail events update gate summary fields.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_line(seq: u64, event: RuntimeEvent) -> String {
        let run_id = event.run_id().to_string();
        serde_json::to_string(&RuntimeEventEnvelope::new(run_id, seq, "test", event)).unwrap()
    }

    #[test]
    fn parses_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        std::fs::write(&path, "").unwrap();

        let runs = RuntimeProjection::from_file(&path).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn handles_missing_file() {
        let path = Path::new("/nonexistent/events.jsonl");
        let runs = RuntimeProjection::from_file(path).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn reconstructs_run_summary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let lines = vec![
            event_line(
                0,
                RuntimeEvent::WorkflowStarted {
                    run_id: "r1".into(),
                    template: "express".into(),
                    prompt: "fix".into(),
                },
            ),
            event_line(
                1,
                RuntimeEvent::PhaseTransition {
                    run_id: "r1".into(),
                    from: "plan".into(),
                    to: "implement".into(),
                },
            ),
            event_line(
                2,
                RuntimeEvent::AgentSpawned {
                    run_id: "r1".into(),
                    agent_id: "a1".into(),
                    role: "implementer".into(),
                    model: "m".into(),
                },
            ),
            event_line(
                3,
                RuntimeEvent::GatePassed {
                    run_id: "r1".into(),
                    gate_name: "compile".into(),
                    duration_ms: 100,
                },
            ),
        ]
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let summary = RuntimeProjection::for_run(&path, "r1").unwrap().unwrap();
        assert_eq!(summary.template.as_deref(), Some("express"));
        assert_eq!(summary.prompt.as_deref(), Some("fix"));
        assert_eq!(summary.current_phase.as_deref(), Some("implement"));
        assert_eq!(summary.agents_spawned, 1);
        assert_eq!(summary.gates_passed, vec!["compile"]);
    }

    #[test]
    fn tracks_agent_completed_and_costs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let lines = vec![
            event_line(
                0,
                RuntimeEvent::WorkflowStarted {
                    run_id: "r2".into(),
                    template: "express".into(),
                    prompt: "test".into(),
                },
            ),
            event_line(
                1,
                RuntimeEvent::AgentSpawned {
                    run_id: "r2".into(),
                    agent_id: "a1".into(),
                    role: "implementer".into(),
                    model: "m".into(),
                },
            ),
            event_line(
                2,
                RuntimeEvent::AgentCompleted {
                    run_id: "r2".into(),
                    agent_id: "a1".into(),
                    output: "done".into(),
                    tokens_used: 500,
                    cost_usd: 0.01,
                },
            ),
        ]
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let summary = RuntimeProjection::for_run(&path, "r2").unwrap().unwrap();
        assert_eq!(summary.agents_spawned, 1);
        assert_eq!(summary.agents_completed, 1);
        assert_eq!(summary.total_tokens, 500);
        assert_eq!(summary.total_cost_usd, 0.01);
    }

    #[test]
    fn tracks_failures_feedback_and_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let lines = vec![
            event_line(
                0,
                RuntimeEvent::AgentOutput {
                    run_id: "r3".into(),
                    agent_id: "a1".into(),
                    chunk: "partial".into(),
                },
            ),
            event_line(
                1,
                RuntimeEvent::AgentFailed {
                    run_id: "r3".into(),
                    agent_id: "a1".into(),
                    error: "compile failed".into(),
                },
            ),
            event_line(
                2,
                RuntimeEvent::GateStarted {
                    run_id: "r3".into(),
                    gate_name: "compile".into(),
                    rung: 1,
                },
            ),
            event_line(
                3,
                RuntimeEvent::FeedbackRecorded {
                    run_id: "r3".into(),
                    kind: "gate".into(),
                    summary: "compile failed".into(),
                },
            ),
            event_line(
                4,
                RuntimeEvent::StateCheckpointed {
                    run_id: "r3".into(),
                    path: "/tmp/state.json".into(),
                },
            ),
        ]
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let summary = RuntimeProjection::for_run(&path, "r3").unwrap().unwrap();
        assert_eq!(summary.agents_failed, 1);
        assert_eq!(summary.agent_errors, vec!["compile failed"]);
        assert_eq!(summary.feedback_count, 1);
        assert_eq!(summary.last_checkpoint.as_deref(), Some("/tmp/state.json"));
    }
}
