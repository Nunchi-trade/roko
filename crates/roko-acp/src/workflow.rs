//! Workflow run state and template definitions.
//!
//! A [`WorkflowRun`] tracks a single pipeline execution from prompt to commit.
//! It wraps [`PipelineState`] with timing, cost, and ACP-specific metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pipeline::{PipelinePhase, PipelineState, WorkflowTemplate};

/// A single workflow pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    /// Unique run identifier.
    pub run_id: String,
    /// The pipeline state machine.
    pub pipeline: PipelineState,
    /// When the run was started.
    pub started_at: DateTime<Utc>,
    /// When the run completed (if terminal).
    pub completed_at: Option<DateTime<Utc>>,
    /// Accumulated cost in USD.
    pub total_cost_usd: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Number of agents spawned during this run.
    pub agents_spawned: u32,
}

impl WorkflowRun {
    /// Create a new workflow run.
    pub fn new(template: WorkflowTemplate, prompt: String, max_iterations: u32) -> Self {
        Self {
            run_id: format!("run_{}", Uuid::new_v4()),
            pipeline: PipelineState::new(template, prompt, max_iterations),
            started_at: Utc::now(),
            completed_at: None,
            total_cost_usd: 0.0,
            total_tokens: 0,
            agents_spawned: 0,
        }
    }

    /// Returns whether this run is in a terminal state.
    pub fn is_done(&self) -> bool {
        self.pipeline.phase.is_terminal()
    }

    /// Mark the run as complete.
    pub fn mark_complete(&mut self) {
        self.completed_at = Some(Utc::now());
    }

    /// Returns the current phase of the pipeline.
    pub fn phase(&self) -> &PipelinePhase {
        &self.pipeline.phase
    }

    /// Returns the workflow template name.
    pub fn template_name(&self) -> &'static str {
        match self.pipeline.template {
            WorkflowTemplate::Express => "Express",
            WorkflowTemplate::Standard => "Standard",
            WorkflowTemplate::Full => "Full",
        }
    }

    /// Returns duration of the run so far.
    pub fn elapsed(&self) -> chrono::Duration {
        let end = self.completed_at.unwrap_or_else(Utc::now);
        end.signed_duration_since(self.started_at)
    }

    /// Formatted status summary for /workflow status.
    pub fn status_summary(&self) -> String {
        let elapsed = self.elapsed();
        let secs = elapsed.num_seconds();
        let phase_label = self.pipeline.phase_label();
        let template = self.template_name();
        let iteration = self.pipeline.iteration;
        let max_iter = self.pipeline.max_iterations;

        format!(
            "Active Workflow: {template}\n\
             Phase: {phase_label} (iteration {iteration}/{max_iter})\n\
             Duration: {secs}s\n\
             Cost: ${:.4}\n\
             Tokens: {}\n\
             Agents spawned: {}",
            self.total_cost_usd, self.total_tokens, self.agents_spawned,
        )
    }
}

/// Result of a single gate check.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateResult {
    /// Gate name (compile, test, clippy, fmt).
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Gate output (errors or success message).
    pub output: String,
    /// How long the gate took in milliseconds.
    pub duration_ms: u64,
}

/// A finding from a code review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    /// Severity: major, minor, nit.
    pub severity: String,
    /// Description of the finding.
    pub description: String,
    /// Optional file path.
    pub file: Option<String>,
    /// Optional line number.
    pub line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_run_creates_with_unique_id() {
        let run = WorkflowRun::new(WorkflowTemplate::Standard, "test".into(), 2);
        assert!(run.run_id.starts_with("run_"));
        assert!(!run.is_done());
    }

    #[test]
    fn status_summary_includes_template_name() {
        let run = WorkflowRun::new(WorkflowTemplate::Express, "fix bug".into(), 1);
        let summary = run.status_summary();
        assert!(summary.contains("Express"));
        assert!(summary.contains("Pending"));
    }
}
