//! RuntimeProjection -- reconstructs workflow state from JSONL event logs.
//!
//! Reads the JSONL file written by JsonlLogger and builds a snapshot of the
//! current state for each run_id. Used for resume and dashboard views.

use crate::effect_driver::Result;
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

            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(value) => value,
                Err(_) => continue,
            };

            let run_id = value["run_id"].as_str().unwrap_or("unknown").to_string();
            let kind = value["kind"].as_str().unwrap_or_default();
            let summary = runs.entry(run_id.clone()).or_insert_with(|| RunSummary {
                run_id: run_id.clone(),
                ..Default::default()
            });

            apply_event(summary, kind, &value);
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

fn apply_event(summary: &mut RunSummary, kind: &str, value: &serde_json::Value) {
    match kind {
        "workflow_started" => {
            summary.template = event_field(value, "template");
            summary.prompt = event_field(value, "prompt");
        }
        "phase_transition" => {
            if let Some(to) = event_field(value, "to") {
                summary.current_phase = Some(to.clone());
                summary.phases_visited.push(to);
            }
        }
        "gate_passed" => {
            if let Some(name) = event_field(value, "gate_name") {
                summary.gates_passed.push(name);
            }
        }
        "gate_failed" => {
            if let Some(name) = event_field(value, "gate_name") {
                summary.gates_failed.push(name);
            }
        }
        "agent_spawned" => {
            summary.agents_spawned += 1;
        }
        "workflow_completed" => {
            summary.is_complete = true;
            summary.outcome = event_field(value, "outcome").or_else(|| Some(kind.to_string()));
        }
        "agent_completed" => {
            summary.agents_completed += 1;
            if let Some(tokens_str) = event_scalar(value, "tokens_used") {
                if let Ok(tokens) = tokens_str.parse::<u64>() {
                    summary.total_tokens += tokens;
                }
            }
            if let Some(cost_str) = event_scalar(value, "cost_usd") {
                if let Ok(cost) = cost_str.parse::<f64>() {
                    summary.total_cost_usd += cost;
                }
            }
        }
        "agent_failed" => {
            summary.agents_failed += 1;
            if let Some(error) = event_field(value, "error") {
                summary.agent_errors.push(error);
            }
        }
        "feedback_recorded" => {
            summary.feedback_count += 1;
        }
        "state_checkpointed" => {
            summary.last_checkpoint = event_field(value, "path");
        }
        "agent_output" => {
            // AgentOutput is a streaming event; no aggregate RunSummary field yet.
        }
        "gate_started" => {
            // GateStarted is informational; pass/fail events update gate summary fields.
        }
        _ => {}
    }
}

fn event_field(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get("event")
        .and_then(|event| event.get(field))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("event")
                .and_then(serde_json::Value::as_str)
                .and_then(|event| debug_field(event, field))
        })
}

fn debug_field(event: &str, field: &str) -> Option<String> {
    let needle = format!("{field}: \"");
    let start = event.find(&needle)? + needle.len();
    let rest = &event[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn event_scalar(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get("event")
        .and_then(|event| event.get(field))
        .map(|value| match value {
            serde_json::Value::String(value) => value.clone(),
            other => other.to_string(),
        })
        .or_else(|| {
            value
                .get("event")
                .and_then(serde_json::Value::as_str)
                .and_then(|event| debug_scalar(event, field))
        })
}

fn debug_scalar(event: &str, field: &str) -> Option<String> {
    let needle = format!("{field}: ");
    let start = event.find(&needle)? + needle.len();
    let rest = event[start..].trim_start();

    if let Some(rest) = rest.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }

    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let lines = [
            r#"{"kind":"workflow_started","run_id":"r1","event":"WorkflowStarted { run_id: \"r1\", template: \"express\", prompt: \"fix\" }"}"#,
            r#"{"kind":"phase_transition","run_id":"r1","event":"PhaseTransition { run_id: \"r1\", from: \"plan\", to: \"implement\" }"}"#,
            r#"{"kind":"agent_spawned","run_id":"r1","event":"AgentSpawned { run_id: \"r1\", agent_id: \"a1\", role: \"implementer\", model: \"m\" }"}"#,
            r#"{"kind":"gate_passed","run_id":"r1","event":"GatePassed { run_id: \"r1\", gate_name: \"compile\", duration_ms: 100 }"}"#,
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
        let lines = [
            r#"{"kind":"workflow_started","run_id":"r2","event":"WorkflowStarted { run_id: \"r2\", template: \"express\", prompt: \"test\" }"}"#,
            r#"{"kind":"agent_spawned","run_id":"r2","event":"AgentSpawned { run_id: \"r2\", agent_id: \"a1\", role: \"implementer\", model: \"m\" }"}"#,
            r#"{"kind":"agent_completed","run_id":"r2","event":"AgentCompleted { run_id: \"r2\", agent_id: \"a1\", output: \"done\", tokens_used: 500, cost_usd: 0.01 }"}"#,
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
        let lines = [
            r#"{"kind":"agent_output","run_id":"r3","event":"AgentOutput { run_id: \"r3\", agent_id: \"a1\", chunk: \"partial\" }"}"#,
            r#"{"kind":"agent_failed","run_id":"r3","event":"AgentFailed { run_id: \"r3\", agent_id: \"a1\", error: \"compile failed\" }"}"#,
            r#"{"kind":"gate_started","run_id":"r3","event":"GateStarted { run_id: \"r3\", gate_name: \"compile\", rung: 1 }"}"#,
            r#"{"kind":"feedback_recorded","run_id":"r3","event":"FeedbackRecorded { run_id: \"r3\", kind: \"gate\", summary: \"compile failed\" }"}"#,
            r#"{"kind":"state_checkpointed","run_id":"r3","event":"StateCheckpointed { run_id: \"r3\", path: \"/tmp/state.json\" }"}"#,
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
