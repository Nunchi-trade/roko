## Batch P3C: JsonlLogger + RuntimeProjection

### Write Scope
- **CREATE**: `crates/roko-runtime/src/jsonl_logger.rs`
- **CREATE**: `crates/roko-runtime/src/projection.rs`
- **MODIFY**: `crates/roko-runtime/src/lib.rs` (add `pub mod jsonl_logger; pub mod projection;`)

### Dependencies
- P0B (EventConsumer trait)
- P0C (RuntimeEvent bus)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Create a new crate

### Task

Create two modules:

1. **JsonlLogger** — implements `EventConsumer` to write RuntimeEvents to a JSONL file
2. **RuntimeProjection** — reads the JSONL file to reconstruct workflow state (for resume)

#### File: `crates/roko-runtime/src/jsonl_logger.rs`

```rust
//! JsonlLogger — persists RuntimeEvents to a JSONL file.
//!
//! Implements EventConsumer. Each event is serialized as a single JSON line
//! with a timestamp, enabling replay and state reconstruction.

use roko_core::foundation::EventConsumer;
use roko_core::runtime_event::RuntimeEvent;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Logger that writes RuntimeEvents as JSONL (one JSON object per line).
pub struct JsonlLogger {
    path: PathBuf,
    writer: Mutex<Option<std::io::BufWriter<std::fs::File>>>,
}

impl JsonlLogger {
    /// Create a new JsonlLogger writing to the given path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            writer: Mutex::new(None),
        }
    }

    /// Create from the standard .roko directory.
    pub fn from_roko_dir(roko_dir: &Path) -> Self {
        Self::new(roko_dir.join("runtime-events.jsonl"))
    }

    /// Ensure the writer is initialized.
    fn ensure_writer(&self) -> Result<(), std::io::Error> {
        let mut writer = self.writer.lock().unwrap();
        if writer.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            *writer = Some(std::io::BufWriter::new(file));
        }
        Ok(())
    }

    /// Write a single event as a JSONL line.
    fn write_event(&self, event: &RuntimeEvent) {
        if self.ensure_writer().is_err() {
            return;
        }

        let json = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "kind": event.kind(),
            "run_id": event.run_id(),
            "event": format!("{:?}", event),
        });

        let mut writer = self.writer.lock().unwrap();
        if let Some(ref mut w) = *writer {
            let _ = writeln!(w, "{}", json);
            let _ = w.flush();
        }
    }

    /// Path to the log file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl EventConsumer for JsonlLogger {
    fn consume(&self, event: &RuntimeEvent) {
        self.write_event(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_events_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let logger = JsonlLogger::new(path.clone());

        logger.consume(&RuntimeEvent::WorkflowStarted {
            run_id: "r1".into(),
            template: "express".into(),
            prompt: "fix".into(),
        });

        logger.consume(&RuntimeEvent::GatePassed {
            run_id: "r1".into(),
            gate_name: "compile".into(),
            duration_ms: 100,
        });

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("workflow_started"));
        assert!(lines[1].contains("gate_passed"));
    }
}
```

#### File: `crates/roko-runtime/src/projection.rs`

```rust
//! RuntimeProjection — reconstructs workflow state from JSONL event log.
//!
//! Reads the JSONL file written by JsonlLogger and builds a snapshot
//! of the current state for each run_id. Used for resume and dashboard.

use std::collections::HashMap;
use std::path::Path;

/// Summary of a workflow run reconstructed from events.
#[derive(Debug, Clone, Default)]
pub struct RunSummary {
    pub run_id: String,
    pub template: Option<String>,
    pub prompt: Option<String>,
    pub current_phase: Option<String>,
    pub phases_visited: Vec<String>,
    pub gates_passed: Vec<String>,
    pub gates_failed: Vec<String>,
    pub agents_spawned: u32,
    pub is_complete: bool,
    pub outcome: Option<String>,
}

/// Reads JSONL event log and produces per-run summaries.
pub struct RuntimeProjection;

impl RuntimeProjection {
    /// Read the event log and produce summaries for all runs.
    pub fn from_file(path: &Path) -> anyhow::Result<HashMap<String, RunSummary>> {
        let mut runs: HashMap<String, RunSummary> = HashMap::new();

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(runs),
            Err(e) => return Err(e.into()),
        };

        for line in content.lines() {
            if line.is_empty() {
                continue;
            }

            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed lines
            };

            let run_id = value["run_id"].as_str().unwrap_or("unknown").to_string();
            let kind = value["kind"].as_str().unwrap_or("").to_string();

            let summary = runs.entry(run_id.clone()).or_insert_with(|| RunSummary {
                run_id: run_id.clone(),
                ..Default::default()
            });

            match kind.as_str() {
                "workflow_started" => {
                    // Parse template and prompt from event debug string
                    summary.template = value["event"]
                        .as_str()
                        .and_then(|s| {
                            s.find("template: \"")
                                .map(|i| {
                                    let rest = &s[i + 11..];
                                    rest.find('"').map(|j| rest[..j].to_string())
                                })
                                .flatten()
                        });
                }
                "phase_transition" => {
                    if let Some(to) = value["event"].as_str().and_then(|s| {
                        s.find("to: \"").map(|i| {
                            let rest = &s[i + 5..];
                            rest.find('"').map(|j| rest[..j].to_string())
                        }).flatten()
                    }) {
                        summary.current_phase = Some(to.clone());
                        summary.phases_visited.push(to);
                    }
                }
                "gate_passed" => {
                    if let Some(name) = value["event"].as_str().and_then(|s| {
                        s.find("gate_name: \"").map(|i| {
                            let rest = &s[i + 12..];
                            rest.find('"').map(|j| rest[..j].to_string())
                        }).flatten()
                    }) {
                        summary.gates_passed.push(name);
                    }
                }
                "gate_failed" => {
                    if let Some(name) = value["event"].as_str().and_then(|s| {
                        s.find("gate_name: \"").map(|i| {
                            let rest = &s[i + 12..];
                            rest.find('"').map(|j| rest[..j].to_string())
                        }).flatten()
                    }) {
                        summary.gates_failed.push(name);
                    }
                }
                "agent_spawned" => {
                    summary.agents_spawned += 1;
                }
                "workflow_completed" => {
                    summary.is_complete = true;
                    summary.outcome = Some(kind);
                }
                _ => {}
            }
        }

        Ok(runs)
    }

    /// Get summary for a specific run.
    pub fn for_run(path: &Path, run_id: &str) -> anyhow::Result<Option<RunSummary>> {
        let runs = Self::from_file(path)?;
        Ok(runs.into_iter().find(|(id, _)| id == run_id).map(|(_, s)| s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
}
```

#### Modification: `crates/roko-runtime/src/lib.rs`

Add:
```rust
pub mod jsonl_logger;
pub mod projection;
pub use jsonl_logger::JsonlLogger;
pub use projection::{RuntimeProjection, RunSummary};
```

### Done Criteria
```bash
grep -q 'pub struct JsonlLogger' crates/roko-runtime/src/jsonl_logger.rs
grep -q 'impl EventConsumer for JsonlLogger' crates/roko-runtime/src/jsonl_logger.rs
grep -q 'pub struct RuntimeProjection' crates/roko-runtime/src/projection.rs
cargo check -p roko-runtime
```
