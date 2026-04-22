//! Marketplace job types shared between `roko-serve`, the TUI, and the CLI.
//!
//! [`MarketplaceJob`] is the canonical representation of a job in the system.
//! It mirrors the `JobRecord` stored in `.roko/jobs/*.json` by `roko-serve`.

use serde::{Deserialize, Serialize};

/// Lifecycle status of a marketplace job.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    #[default]
    Open,
    Assigned,
    InProgress,
    Submitted,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    /// Parse a status string into a `JobStatus`, tolerating aliases.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "open" | "pending" => Some(Self::Open),
            "assigned" => Some(Self::Assigned),
            "in_progress" | "active" | "running" => Some(Self::InProgress),
            "submitted" => Some(Self::Submitted),
            "completed" | "done" | "evaluated" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Snake-case string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Assigned => "assigned",
            Self::InProgress => "in_progress",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Valid status transitions from this state.
    #[must_use]
    pub const fn valid_transitions(self) -> &'static [JobStatus] {
        match self {
            Self::Open => &[Self::Assigned, Self::InProgress, Self::Cancelled],
            Self::Assigned => &[Self::InProgress, Self::Open, Self::Cancelled],
            Self::InProgress => &[Self::Submitted, Self::Failed, Self::Cancelled],
            Self::Submitted => &[Self::Completed, Self::InProgress, Self::Failed],
            Self::Completed | Self::Failed | Self::Cancelled => &[],
        }
    }

    /// Whether this is a terminal (no further transitions) state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A marketplace job — the canonical shared type across serve, TUI, and CLI.
///
/// Mirrors the `JobRecord` persisted in `.roko/jobs/{id}.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MarketplaceJob {
    /// Unique job identifier.
    #[serde(default)]
    pub id: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: String,
    /// Detailed description of the work to be done.
    #[serde(default)]
    pub description: String,
    /// Job type: `research`, `coding_task`, `chain_monitor`, `chain_analysis`, `other`.
    #[serde(default)]
    pub job_type: String,
    /// Lifecycle status (preferred field).
    #[serde(default)]
    pub status: String,
    /// Fallback status field for backward-compat with older job files using `state`.
    #[serde(default)]
    pub state: String,
    /// Who posted the job.
    #[serde(default)]
    pub posted_by: String,
    /// Who is assigned to the job.
    #[serde(default, alias = "assignee")]
    pub assigned_to: String,
    /// Priority level: `low`, `medium`, `high`, `critical`.
    #[serde(default)]
    pub priority: String,
    /// RFC-3339 creation timestamp.
    #[serde(default)]
    pub created_at: String,
    /// RFC-3339 last-update timestamp.
    #[serde(default)]
    pub updated_at: String,
    /// Freeform tags for categorisation.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional reward string.
    #[serde(default)]
    pub reward: String,
    /// Optional associated plan identifier.
    #[serde(default)]
    pub plan_id: String,
    /// Submission payload (result_summary, artifacts, gate_results).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission: Option<serde_json::Value>,
    /// Evaluation payload (accepted, feedback).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<serde_json::Value>,
    /// Whether the job runner should auto-execute this job.
    /// Default: `true` for research/chain, `false` for coding.
    #[serde(default)]
    pub auto_execute: bool,
}

/// Summary of a PRD for the Atelier TUI view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrdSummary {
    /// URL-safe slug identifier.
    #[serde(default)]
    pub slug: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: String,
    /// Lifecycle status: `idea`, `draft`, `published`, `planned`.
    #[serde(default)]
    pub status: String,
    /// Number of associated plans.
    #[serde(default)]
    pub plan_count: usize,
    /// Total tasks across all plans.
    #[serde(default)]
    pub task_total: usize,
    /// Completed tasks.
    #[serde(default)]
    pub task_done: usize,
    /// Failed tasks.
    #[serde(default)]
    pub task_failed: usize,
}

/// Summary of a task for the Atelier TUI view.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskSummary {
    /// Task identifier.
    #[serde(default)]
    pub id: String,
    /// Human-readable title.
    #[serde(default)]
    pub title: String,
    /// Current status string.
    #[serde(default)]
    pub status: String,
    /// Agent assigned to this task.
    #[serde(default)]
    pub agent: String,
}

/// Progress entry for a running job (used by TUI).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobProgressEntry {
    /// Completion percentage (0-100).
    #[serde(default)]
    pub percent: u8,
    /// Latest progress message.
    #[serde(default)]
    pub message: String,
    /// Agent executing this job.
    #[serde(default)]
    pub agent_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_status_parse_aliases() {
        assert_eq!(JobStatus::parse("open"), Some(JobStatus::Open));
        assert_eq!(JobStatus::parse("pending"), Some(JobStatus::Open));
        assert_eq!(JobStatus::parse("in_progress"), Some(JobStatus::InProgress));
        assert_eq!(JobStatus::parse("active"), Some(JobStatus::InProgress));
        assert_eq!(JobStatus::parse("done"), Some(JobStatus::Completed));
        assert_eq!(JobStatus::parse("cancelled"), Some(JobStatus::Cancelled));
        assert_eq!(JobStatus::parse("canceled"), Some(JobStatus::Cancelled));
        assert_eq!(JobStatus::parse("bogus"), None);
    }

    #[test]
    fn job_status_transitions() {
        let open = JobStatus::Open;
        assert!(!open.is_terminal());
        assert_eq!(
            open.valid_transitions(),
            &[JobStatus::Assigned, JobStatus::InProgress, JobStatus::Cancelled]
        );

        let completed = JobStatus::Completed;
        assert!(completed.is_terminal());
        assert!(completed.valid_transitions().is_empty());
    }

    #[test]
    fn marketplace_job_serde_roundtrip() {
        let job = MarketplaceJob {
            id: "test-1".into(),
            title: "Test job".into(),
            job_type: "research".into(),
            status: "open".into(),
            auto_execute: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&job).unwrap();
        let parsed: MarketplaceJob = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, "test-1");
        assert!(parsed.auto_execute);
    }

    #[test]
    fn prd_summary_default() {
        let prd = PrdSummary::default();
        assert!(prd.slug.is_empty());
        assert_eq!(prd.task_total, 0);
    }
}
