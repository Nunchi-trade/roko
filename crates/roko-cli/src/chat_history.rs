//! Chat session history: write summaries and list past sessions.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Summary of a single chat session, written on exit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// `<timestamp>-<agent_id>` - also the filename stem.
    pub session_id: String,
    /// Agent identifier passed to the chat command.
    pub agent_id: String,
    /// Provider name (for example, `anthropic_api`).
    pub provider: String,
    /// Model key used for the session.
    #[serde(default)]
    pub model_key: String,
    /// ISO-8601 UTC timestamp when the session started.
    pub started_at: String,
    /// ISO-8601 UTC timestamp when the session ended.
    #[serde(default)]
    pub ended_at: String,
    /// Number of completed turns.
    #[serde(default)]
    pub turn_count: u32,
    /// First user message, truncated to 120 chars.
    #[serde(default)]
    pub first_message: String,
    /// Last user message, truncated to 120 chars.
    #[serde(default)]
    pub last_message: String,
    /// Total input + output tokens across all turns.
    #[serde(default)]
    pub total_tokens: u64,
    /// Approximate total cost in USD (0.0 if not tracked).
    #[serde(default)]
    pub total_cost_usd: f64,
}

/// Compute the sessions directory for a workspace.
pub fn sessions_dir(workdir: &Path) -> PathBuf {
    workdir.join(".roko").join("sessions")
}

/// Write a session summary file non-blockingly.
///
/// The write runs in a separate `tokio::spawn` task. Errors are logged but
/// never bubbled back to the caller.
pub fn save_summary_nonblocking(workdir: PathBuf, summary: SessionSummary) {
    let _ = tokio::spawn(async move {
        if let Err(error) = write_summary(&workdir, &summary) {
            tracing::warn!(error = %error, "failed to persist chat session summary");
        }
    });
}

fn write_summary(workdir: &Path, summary: &SessionSummary) -> Result<()> {
    let dir = sessions_dir(workdir);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{}.json", summary.session_id));
    let json = serde_json::to_string_pretty(summary)?;
    std::fs::write(&path, json).with_context(|| format!("write {}", path.display()))
}

/// Load and return summaries sorted by `started_at` descending (newest first).
pub fn list_sessions(workdir: &Path, limit: usize) -> Vec<SessionSummary> {
    let dir = sessions_dir(workdir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut summaries: Vec<SessionSummary> = entries
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.path()).ok()?;
            serde_json::from_str(&text).ok()
        })
        .collect();

    summaries.sort_by(|left, right| right.started_at.cmp(&left.started_at));
    summaries.truncate(limit);
    summaries
}

/// Load a single session summary by ID (the filename stem).
pub fn load_session(workdir: &Path, session_id: &str) -> Option<SessionSummary> {
    let path = sessions_dir(workdir).join(format!("{session_id}.json"));
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}
