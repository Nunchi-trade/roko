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
}
