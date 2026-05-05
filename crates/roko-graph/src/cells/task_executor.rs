//! Stub `TaskExecutorCell` -- placeholder cell for plan-to-graph converted tasks.
//!
//! In dry-run mode, returns a synthetic "task-output" engram without calling an LLM.
//! In live mode, this will delegate to the existing Runner v2 agent dispatch path
//! (to be implemented in a future task when the Engine replaces Runner v2).

use std::time::Duration;

use roko_core::{Body, Engram, Kind, error::Result};

use crate::cell::{Cell, CellContext, CellVersion};

/// Stub cell that represents a plan task in the Graph Engine.
///
/// When `dry_run` is `true`, it returns a synthetic output engram.
/// When `dry_run` is `false`, it will eventually delegate to the real
/// agent dispatch path. For now it always does a dry-run pass-through.
pub struct TaskExecutorCell {
    /// Whether to skip real LLM dispatch and return synthetic output.
    pub dry_run: bool,
}

impl TaskExecutorCell {
    /// Create a new `TaskExecutorCell`.
    pub const fn new(dry_run: bool) -> Self {
        Self { dry_run }
    }
}

impl Default for TaskExecutorCell {
    fn default() -> Self {
        Self { dry_run: true }
    }
}

#[async_trait::async_trait]
impl Cell for TaskExecutorCell {
    fn cell_id(&self) -> &str {
        "task-executor"
    }

    fn cell_name(&self) -> &str {
        "TaskExecutorCell"
    }

    fn cell_version(&self) -> CellVersion {
        (0, 1, 0)
    }

    fn protocols(&self) -> &[&str] {
        &["TaskExecution"]
    }

    fn estimated_cost(&self) -> Option<f64> {
        None
    }

    fn estimated_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(120))
    }

    async fn execute(&self, input: Vec<Engram>, _ctx: &CellContext) -> Result<Vec<Engram>> {
        // Extract task title from the node config if available in input metadata.
        let task_label = input
            .first()
            .and_then(|e| e.body.as_text().ok())
            .map(|s| s.chars().take(60).collect::<String>())
            .unwrap_or_else(|| "(unknown task)".to_string());

        if self.dry_run {
            tracing::info!(
                task = %task_label,
                "TaskExecutorCell dry-run: skipping LLM dispatch"
            );
            // Return a synthetic output engram.
            let output = Engram::builder(Kind::AgentOutput)
                .body(Body::text(format!("task-output:dry-run:{task_label}")))
                .build();
            Ok(vec![output])
        } else {
            // Live mode: not yet implemented. Fall back to dry-run behavior
            // with a warning. The real implementation will delegate to the
            // Runner v2 agent dispatch path.
            tracing::warn!(
                task = %task_label,
                "TaskExecutorCell live dispatch not yet implemented; using dry-run fallback"
            );
            let output = Engram::builder(Kind::AgentOutput)
                .body(Body::text(format!("task-output:stub:{task_label}")))
                .build();
            Ok(vec![output])
        }
    }
}
