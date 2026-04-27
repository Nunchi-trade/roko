## Batch P1C: FeedbackService

### Write Scope
- **CREATE**: `crates/roko-learn/src/feedback_service.rs`
- **MODIFY**: `crates/roko-learn/src/lib.rs` (add `pub mod feedback_service;` and re-export)

### Dependencies
- P0A (RuntimeEvent types)
- P0B (FeedbackSink trait, FeedbackEvent)

### DO NOT
- Modify any other files
- Add Cargo.toml dependencies
- Create a new crate
- Duplicate existing feedback infrastructure

### Existing Code Context

`roko-learn` already has:
```rust
// EpisodeLogger — append-only JSONL of agent turns
pub struct EpisodeLogger { /* ... */ }
impl EpisodeLogger {
    pub fn append(&self, episode: &Episode) -> Result<()>;
}

// CascadeRouter — model routing with learning
pub struct CascadeRouter { /* ... */ }
impl CascadeRouter {
    pub fn select(&self, requirements: &TaskRequirements) -> ModelSpec;
    pub fn record_outcome(&mut self, spec: &ModelSpec, outcome: &TaskOutcome) -> Result<()>;
}

// Efficiency logging
pub mod efficiency;
```

### Task

Create `FeedbackService` — a concrete implementation of the `FeedbackSink` trait.
It bridges the foundation trait to the existing learning infrastructure.

#### File: `crates/roko-learn/src/feedback_service.rs`

```rust
//! FeedbackService — concrete implementation of `FeedbackSink`.
//!
//! Records model call feedback, gate results, and workflow outcomes
//! into the existing learning infrastructure (EpisodeLogger, efficiency events).

use anyhow::Result;
use async_trait::async_trait;
use roko_core::foundation::{FeedbackEvent, FeedbackSink};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Service that records feedback events for the learning subsystem.
///
/// This is the canonical way to record feedback in the workflow engine. It:
/// - Logs model call metrics (tokens, cost, latency) for efficiency analysis
/// - Records gate results for adaptive threshold tuning
/// - Tracks workflow outcomes for cascade router learning
pub struct FeedbackService {
    /// Directory for feedback data files
    data_dir: PathBuf,
    /// In-memory buffer of recent events (for batch writes)
    buffer: Mutex<Vec<FeedbackEvent>>,
    /// Maximum buffer size before flushing
    buffer_capacity: usize,
}

impl FeedbackService {
    /// Create a new FeedbackService writing to the given data directory.
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            buffer: Mutex::new(Vec::with_capacity(64)),
            buffer_capacity: 64,
        }
    }

    /// Create from the standard .roko directory.
    pub fn from_roko_dir(roko_dir: &Path) -> Self {
        Self::new(roko_dir.join("learn"))
    }

    /// Flush buffered events to disk.
    pub fn flush(&self) -> Result<()> {
        let events = {
            let mut buf = self.buffer.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
            std::mem::take(&mut *buf)
        };

        if events.is_empty() {
            return Ok(());
        }

        // Append to efficiency JSONL
        let efficiency_path = self.data_dir.join("efficiency.jsonl");
        std::fs::create_dir_all(&self.data_dir)?;

        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&efficiency_path)?;

        for event in &events {
            let json = match event {
                FeedbackEvent::ModelCall {
                    run_id, model, role, input_tokens, output_tokens,
                    cost_usd, latency_ms, success,
                } => {
                    serde_json::json!({
                        "kind": "model_call",
                        "run_id": run_id,
                        "model": model,
                        "role": role,
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens,
                        "cost_usd": cost_usd,
                        "latency_ms": latency_ms,
                        "success": success,
                        "ts": chrono::Utc::now().to_rfc3339(),
                    })
                }
                FeedbackEvent::GateResult {
                    run_id, gate_name, passed, duration_ms,
                } => {
                    serde_json::json!({
                        "kind": "gate_result",
                        "run_id": run_id,
                        "gate_name": gate_name,
                        "passed": passed,
                        "duration_ms": duration_ms,
                        "ts": chrono::Utc::now().to_rfc3339(),
                    })
                }
                FeedbackEvent::WorkflowComplete {
                    run_id, outcome, total_cost_usd, total_tokens, duration_ms,
                } => {
                    serde_json::json!({
                        "kind": "workflow_complete",
                        "run_id": run_id,
                        "outcome": outcome,
                        "total_cost_usd": total_cost_usd,
                        "total_tokens": total_tokens,
                        "duration_ms": duration_ms,
                        "ts": chrono::Utc::now().to_rfc3339(),
                    })
                }
            };
            writeln!(file, "{}", json)?;
        }

        Ok(())
    }
}

#[async_trait]
impl FeedbackSink for FeedbackService {
    async fn record(&self, event: FeedbackEvent) -> Result<()> {
        let should_flush = {
            let mut buf = self.buffer.lock().map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
            buf.push(event);
            buf.len() >= self.buffer_capacity
        };

        if should_flush {
            self.flush()?;
        }

        Ok(())
    }
}

impl Drop for FeedbackService {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_model_call() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record(FeedbackEvent::ModelCall {
            run_id: "r1".into(),
            model: "sonnet".into(),
            role: "implementer".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cost_usd: 0.01,
            latency_ms: 2000,
            success: true,
        })
        .await
        .unwrap();

        svc.flush().unwrap();

        let content = std::fs::read_to_string(dir.path().join("efficiency.jsonl")).unwrap();
        assert!(content.contains("model_call"));
        assert!(content.contains("sonnet"));
    }

    #[tokio::test]
    async fn records_gate_result() {
        let dir = tempfile::tempdir().unwrap();
        let svc = FeedbackService::new(dir.path().to_path_buf());

        svc.record(FeedbackEvent::GateResult {
            run_id: "r1".into(),
            gate_name: "compile".into(),
            passed: true,
            duration_ms: 3000,
        })
        .await
        .unwrap();

        svc.flush().unwrap();

        let content = std::fs::read_to_string(dir.path().join("efficiency.jsonl")).unwrap();
        assert!(content.contains("gate_result"));
    }
}
```

**Important**: Check that `roko-learn` has `serde_json` and `chrono` as dependencies before
using them. If not available, use simpler serialization. Also check if `tempfile` is available
for tests — if not, use a manual temp directory.

#### Modification: `crates/roko-learn/src/lib.rs`

Add:
```rust
pub mod feedback_service;
pub use feedback_service::FeedbackService;
```

### Done Criteria
```bash
grep -q 'pub struct FeedbackService' crates/roko-learn/src/feedback_service.rs
grep -q 'impl FeedbackSink for FeedbackService' crates/roko-learn/src/feedback_service.rs
grep -q 'pub mod feedback_service' crates/roko-learn/src/lib.rs
cargo check -p roko-learn
```
