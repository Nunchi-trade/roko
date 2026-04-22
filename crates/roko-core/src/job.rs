//! Marketplace job types shared between `roko-serve`, the TUI, and the CLI.
//!
//! [`MarketplaceJob`] is the canonical representation of a job in the system.
//! It mirrors the `JobRecord` stored in `.roko/jobs/*.json` by `roko-serve`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

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
    /// Defaults to `false`; callers set it explicitly when creating a job.
    #[serde(default)]
    pub auto_execute: bool,
}

impl MarketplaceJob {
    /// Return the effective lifecycle status, preferring `status` but falling
    /// back to the deprecated `state` field for files written by roko-serve
    /// (which serializes via `#[serde(rename = "state")]`).
    #[must_use]
    pub fn effective_status(&self) -> &str {
        let s = self.status.trim();
        if !s.is_empty() {
            return s;
        }
        let fallback = self.state.trim();
        if !fallback.is_empty() {
            return fallback;
        }
        "open"
    }
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

// ---------------------------------------------------------------------------
// Typed job domain types
// ---------------------------------------------------------------------------

/// Categorisation of a marketplace job.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    Research,
    CodingTask,
    ChainMonitor,
    ChainAnalysis,
    Review,
    Documentation,
    Testing,
    Other(String),
}

impl Default for JobType {
    fn default() -> Self {
        Self::Other("other".to_string())
    }
}

impl std::fmt::Display for JobType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Research => f.write_str("research"),
            Self::CodingTask => f.write_str("coding_task"),
            Self::ChainMonitor => f.write_str("chain_monitor"),
            Self::ChainAnalysis => f.write_str("chain_analysis"),
            Self::Review => f.write_str("review"),
            Self::Documentation => f.write_str("documentation"),
            Self::Testing => f.write_str("testing"),
            Self::Other(s) => f.write_str(s),
        }
    }
}

impl FromStr for JobType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "research" => Self::Research,
            "coding_task" | "coding" => Self::CodingTask,
            "chain_monitor" => Self::ChainMonitor,
            "chain_analysis" => Self::ChainAnalysis,
            "review" => Self::Review,
            "documentation" | "docs" => Self::Documentation,
            "testing" | "test" => Self::Testing,
            other => Self::Other(other.to_string()),
        })
    }
}

/// Typed submission payload for a job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobSubmission {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub result_summary: String,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub gate_results: Vec<JobGateResult>,
    #[serde(default)]
    pub submitted_at: String,
}

/// Result of a gate check within a job submission.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobGateResult {
    #[serde(default)]
    pub gate: String,
    #[serde(default)]
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Typed evaluation payload for a job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobEvaluation {
    #[serde(default)]
    pub evaluator: String,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default)]
    pub feedback: String,
    #[serde(default)]
    pub evaluated_at: String,
}

/// Error type for job store operations.
#[derive(Debug)]
pub enum JobError {
    InvalidTransition { from: String, to: String },
    NotFound(String),
    Io(std::io::Error),
    Serde(serde_json::Error),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidTransition { from, to } => {
                write!(f, "invalid job transition from '{from}' to '{to}'")
            }
            Self::NotFound(id) => write!(f, "job '{id}' not found"),
            Self::Io(e) => write!(f, "job I/O error: {e}"),
            Self::Serde(e) => write!(f, "job serialization error: {e}"),
        }
    }
}

impl std::error::Error for JobError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Serde(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for JobError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<serde_json::Error> for JobError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serde(err)
    }
}

/// Filter criteria for listing jobs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<JobStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<String>,
}

impl JobFilter {
    /// Check whether a job matches all active filter criteria.
    #[must_use]
    pub fn matches(&self, job: &MarketplaceJob) -> bool {
        if let Some(ref status) = self.state {
            if JobStatus::parse(&job.status) != Some(*status) {
                return false;
            }
        }
        if let Some(ref jt) = self.job_type {
            if !jt.is_empty() && job.job_type != *jt {
                return false;
            }
        }
        if let Some(ref assignee) = self.assigned_to {
            if !assignee.is_empty() && job.assigned_to != *assignee {
                return false;
            }
        }
        true
    }
}

/// Aggregate statistics for the job store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStats {
    pub total: usize,
    #[serde(default)]
    pub by_state: HashMap<String, usize>,
    #[serde(default)]
    pub by_type: HashMap<String, usize>,
}

/// Payload for creating a new job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateJobRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub job_type: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub reward: String,
    #[serde(default)]
    pub auto_execute: bool,
}

/// File-system backed job store rooted at `.roko/jobs/`.
pub struct FileJobStore {
    root: PathBuf,
}

impl FileJobStore {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn job_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    /// Persist a job with atomic write (tmp + rename).
    pub async fn save(&self, job: &MarketplaceJob) -> Result<(), JobError> {
        tokio::fs::create_dir_all(&self.root).await?;
        let path = self.job_path(&job.id);
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(job)?;
        tokio::fs::write(&tmp, json).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Load a single job by id.
    pub async fn get(&self, id: &str) -> Result<MarketplaceJob, JobError> {
        let path = self.job_path(id);
        let data = tokio::fs::read_to_string(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                JobError::NotFound(id.to_string())
            } else {
                JobError::Io(e)
            }
        })?;
        let job: MarketplaceJob = serde_json::from_str(&data)?;
        Ok(job)
    }

    /// List all jobs, optionally filtered.
    pub async fn list(&self, filter: &JobFilter) -> Result<Vec<MarketplaceJob>, JobError> {
        if !self.root.is_dir() {
            return Ok(Vec::new());
        }
        let mut jobs = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.root).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let data = match tokio::fs::read_to_string(&path).await {
                Ok(d) => d,
                Err(_) => continue,
            };
            let mut job: MarketplaceJob = match serde_json::from_str(&data) {
                Ok(j) => j,
                Err(_) => continue,
            };
            if job.id.is_empty() {
                job.id = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
            }
            if filter.matches(&job) {
                jobs.push(job);
            }
        }
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        Ok(jobs)
    }

    /// Create a new job from a request payload.
    pub async fn create(&self, req: &CreateJobRequest) -> Result<MarketplaceJob, JobError> {
        let now = chrono::Utc::now();
        let id = format!("job-{}", now.timestamp_millis());
        let now = now.to_rfc3339();
        let job = MarketplaceJob {
            id,
            title: req.title.clone(),
            description: req.description.clone(),
            job_type: if req.job_type.is_empty() {
                "other".to_string()
            } else {
                req.job_type.clone()
            },
            status: "open".to_string(),
            priority: req.priority.clone(),
            tags: req.tags.clone(),
            reward: req.reward.clone(),
            auto_execute: req.auto_execute,
            created_at: now.clone(),
            updated_at: now,
            ..Default::default()
        };
        self.save(&job).await?;
        Ok(job)
    }

    /// Compute aggregate statistics across all jobs.
    pub async fn stats(&self) -> Result<JobStats, JobError> {
        let all = self.list(&JobFilter::default()).await?;
        let mut by_state: HashMap<String, usize> = HashMap::new();
        let mut by_type: HashMap<String, usize> = HashMap::new();
        for job in &all {
            let status_key = if job.status.is_empty() {
                "open".to_string()
            } else {
                job.status.clone()
            };
            *by_state.entry(status_key).or_default() += 1;
            let type_key = if job.job_type.is_empty() {
                "other".to_string()
            } else {
                job.job_type.clone()
            };
            *by_type.entry(type_key).or_default() += 1;
        }
        Ok(JobStats {
            total: all.len(),
            by_state,
            by_type,
        })
    }
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
            &[
                JobStatus::Assigned,
                JobStatus::InProgress,
                JobStatus::Cancelled
            ]
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
