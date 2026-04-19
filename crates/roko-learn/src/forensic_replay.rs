//! Forensic replay API for debugging failed tasks (GATE-07).
//!
//! Reconstructs the full execution context of a task from episode logs and
//! gate results, producing a timeline of events for post-mortem analysis.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::episode_logger::{Episode, EpisodeLogger, GateVerdict};

/// A timestamped event in a task's execution timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampedEvent {
    /// When the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Event kind (e.g., "agent_start", "gate_check", "failure").
    pub kind: String,
    /// Human-readable description.
    pub description: String,
    /// Optional extra metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl TimestampedEvent {
    /// Create a new timestamped event.
    pub fn new(
        timestamp: DateTime<Utc>,
        kind: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            timestamp,
            kind: kind.into(),
            description: description.into(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Attach metadata to the event.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Complete forensic replay of a task's execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForensicReplay {
    /// Task identifier being replayed.
    pub task_id: String,
    /// Gate results from all episodes for this task.
    pub gate_results: Vec<GateVerdict>,
    /// Agent episodes (turns) for this task, in chronological order.
    pub agent_turns: Vec<Episode>,
    /// Reconstructed timeline of events.
    pub timeline: Vec<TimestampedEvent>,
    /// Overall success status.
    pub success: bool,
    /// Total duration across all turns.
    pub total_duration_secs: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Failure reason, if any.
    pub failure_reason: Option<String>,
}

impl ForensicReplay {
    /// Reconstruct a forensic replay from a list of episodes for a given task.
    ///
    /// Filters episodes by `task_id`, sorts them chronologically, and builds
    /// a timeline of events from agent turns and gate verdicts.
    #[must_use]
    pub fn from_episodes(task_id: &str, all_episodes: &[Episode]) -> Self {
        let mut agent_turns: Vec<Episode> = all_episodes
            .iter()
            .filter(|ep| ep.task_id == task_id)
            .cloned()
            .collect();

        // Sort by started_at timestamp.
        agent_turns.sort_by_key(|ep| ep.started_at);

        let mut gate_results = Vec::new();
        let mut timeline = Vec::new();
        let mut total_duration = 0.0;
        let mut total_tokens = 0_u64;
        let mut failure_reason = None;
        let mut any_success = false;

        for (turn_idx, episode) in agent_turns.iter().enumerate() {
            // Agent start event.
            timeline.push(TimestampedEvent::new(
                episode.started_at,
                "agent_start",
                format!(
                    "Turn {} started: agent={}, model={}",
                    turn_idx + 1,
                    episode.agent_id,
                    episode.model,
                ),
            ));

            // Gate check events.
            for verdict in &episode.gate_verdicts {
                let event_kind = if verdict.passed {
                    "gate_pass"
                } else {
                    "gate_fail"
                };
                timeline.push(
                    TimestampedEvent::new(
                        episode.completed_at,
                        event_kind,
                        format!(
                            "Gate '{}': {}{}",
                            verdict.gate,
                            if verdict.passed { "PASSED" } else { "FAILED" },
                            verdict
                                .signature
                                .as_deref()
                                .map(|s| format!(" (sig: {s})"))
                                .unwrap_or_default(),
                        ),
                    )
                    .with_metadata(serde_json::json!({
                        "gate": verdict.gate,
                        "passed": verdict.passed,
                    })),
                );
                gate_results.push(verdict.clone());
            }

            // Agent completion event.
            let status = if episode.success { "success" } else { "failure" };
            timeline.push(TimestampedEvent::new(
                episode.completed_at,
                format!("agent_{status}"),
                format!(
                    "Turn {} completed: {} ({:.1}s, {} tokens)",
                    turn_idx + 1,
                    status,
                    episode.duration_secs,
                    episode.tokens_used,
                ),
            ));

            total_duration += episode.duration_secs;
            total_tokens += episode.tokens_used;

            if episode.success {
                any_success = true;
            }

            if let Some(ref reason) = episode.failure_reason {
                failure_reason = Some(reason.clone());
            }
        }

        // Sort timeline chronologically.
        timeline.sort_by_key(|e| e.timestamp);

        Self {
            task_id: task_id.to_string(),
            gate_results,
            agent_turns,
            timeline,
            success: any_success,
            total_duration_secs: total_duration,
            total_tokens,
            failure_reason,
        }
    }

    /// Number of agent turns in the replay.
    #[must_use]
    pub fn turn_count(&self) -> usize {
        self.agent_turns.len()
    }

    /// Number of gates that failed.
    #[must_use]
    pub fn failed_gate_count(&self) -> usize {
        self.gate_results.iter().filter(|g| !g.passed).count()
    }

    /// Number of gates that passed.
    #[must_use]
    pub fn passed_gate_count(&self) -> usize {
        self.gate_results.iter().filter(|g| g.passed).count()
    }

    /// Whether any gates failed.
    #[must_use]
    pub fn has_gate_failures(&self) -> bool {
        self.gate_results.iter().any(|g| !g.passed)
    }

    /// Return the names of gates that failed.
    #[must_use]
    pub fn failed_gate_names(&self) -> Vec<String> {
        self.gate_results
            .iter()
            .filter(|g| !g.passed)
            .map(|g| g.gate.clone())
            .collect()
    }

    /// Produce a human-readable summary of the replay.
    #[must_use]
    pub fn summary(&self) -> String {
        let status = if self.success { "SUCCESS" } else { "FAILURE" };
        let gates_passed = self.passed_gate_count();
        let gates_failed = self.failed_gate_count();

        let mut lines = vec![
            format!("Task: {}", self.task_id),
            format!("Status: {status}"),
            format!("Turns: {}", self.turn_count()),
            format!("Duration: {:.1}s", self.total_duration_secs),
            format!("Tokens: {}", self.total_tokens),
            format!("Gates: {gates_passed} passed, {gates_failed} failed"),
        ];

        if let Some(ref reason) = self.failure_reason {
            lines.push(format!("Failure reason: {reason}"));
        }

        if !self.failed_gate_names().is_empty() {
            lines.push(format!("Failed gates: {}", self.failed_gate_names().join(", ")));
        }

        lines.join("\n")
    }
}

/// Load episodes from a JSONL file and reconstruct a forensic replay for a task.
///
/// # Errors
///
/// Returns an error if the episode log cannot be read.
pub async fn replay(
    episode_log_path: &str,
    task_id: &str,
) -> Result<ForensicReplay, crate::episode_logger::LoggerError> {
    let episodes = EpisodeLogger::read_all(episode_log_path).await?;
    Ok(ForensicReplay::from_episodes(task_id, &episodes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode_logger::Episode;

    fn make_episode(task_id: &str, success: bool, turn: usize) -> Episode {
        let mut ep = Episode::new("agent-1", task_id);
        ep.success = success;
        ep.duration_secs = 10.0 * turn as f64;
        ep.tokens_used = 1000 * turn as u64;
        ep.model = "claude-sonnet".to_string();
        if !success {
            ep.failure_reason = Some("compile error".to_string());
        }
        ep.gate_verdicts = vec![
            GateVerdict::new("compile", success),
            GateVerdict::new("test", success),
        ];
        ep
    }

    #[test]
    fn forensic_replay_from_episodes() {
        let episodes = vec![
            make_episode("task-1", false, 1),
            make_episode("task-1", true, 2),
            make_episode("task-2", true, 1), // Different task, should be filtered.
        ];

        let replay = ForensicReplay::from_episodes("task-1", &episodes);

        assert_eq!(replay.task_id, "task-1");
        assert_eq!(replay.turn_count(), 2);
        assert!(replay.success); // Second turn succeeded.
        assert_eq!(replay.gate_results.len(), 4); // 2 gates per turn * 2 turns.
        assert!(replay.has_gate_failures());
        assert_eq!(replay.failed_gate_count(), 2);
        assert_eq!(replay.passed_gate_count(), 2);
        assert!(replay.total_tokens > 0);
        assert!(!replay.timeline.is_empty());
    }

    #[test]
    fn forensic_replay_summary() {
        let episodes = vec![make_episode("task-1", false, 1)];
        let replay = ForensicReplay::from_episodes("task-1", &episodes);
        let summary = replay.summary();

        assert!(summary.contains("FAILURE"));
        assert!(summary.contains("task-1"));
        assert!(summary.contains("compile"));
    }

    #[test]
    fn forensic_replay_empty_task() {
        let replay = ForensicReplay::from_episodes("nonexistent", &[]);
        assert_eq!(replay.turn_count(), 0);
        assert!(!replay.success);
        assert!(replay.timeline.is_empty());
    }
}
