//! `roko status` subcommand — queries a running daemon or local substrate.
//!
//! When a daemon is running, `roko status` connects to its Unix socket and
//! retrieves live session information. When no daemon is found, it falls
//! back to reading the local `.roko/` substrate for signal counts.

use std::path::{Path, PathBuf};

use roko_learn::cfactor::CFactor;

/// Information about a session's current state.
#[derive(Debug, Clone)]
pub struct SessionStatus {
    /// Session identifier (if known).
    pub session_id: Option<String>,
    /// Working directory.
    pub workdir: PathBuf,
    /// Whether a daemon is running for this session.
    pub daemon_running: bool,
    /// Number of signals in the substrate.
    pub signal_count: Option<usize>,
    /// Number of episodes recorded.
    pub episode_count: Option<usize>,
    /// Last episode outcome (if any).
    pub last_episode_passed: Option<bool>,
    /// Optional current C-Factor snapshot.
    pub cfactor: Option<CFactor>,
}

impl SessionStatus {
    /// Create a status for a directory with no active daemon.
    #[must_use]
    pub const fn offline(workdir: PathBuf) -> Self {
        Self {
            session_id: None,
            workdir,
            daemon_running: false,
            signal_count: None,
            episode_count: None,
            last_episode_passed: None,
            cfactor: None,
        }
    }

    /// Format this status for human-readable display.
    #[must_use]
    pub fn display_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("workdir: {}", self.workdir.display()));

        if let Some(sid) = &self.session_id {
            lines.push(format!("session: {sid}"));
        }

        lines.push(format!(
            "daemon : {}",
            if self.daemon_running {
                "running"
            } else {
                "not running"
            }
        ));

        if let Some(n) = self.signal_count {
            lines.push(format!("signals: {n}"));
        }
        if let Some(n) = self.episode_count {
            lines.push(format!("episodes: {n}"));
        }
        if let Some(passed) = self.last_episode_passed {
            lines.push(format!(
                "last episode: {}",
                if passed { "passed" } else { "failed" }
            ));
        }
        if let Some(cfactor) = &self.cfactor {
            lines.push(format!(
                "cfactor: {:.3} (episodes: {}, computed: {})",
                cfactor.overall, cfactor.episode_count, cfactor.computed_at
            ));
            lines.push(format!(
                "  gate={:.3} cost={:.3} speed={:.3} flow={:.3} first_try={:.3} knowledge={:.3} turn={:.3} social={:.3}",
                cfactor.components.gate_pass_rate,
                cfactor.components.cost_efficiency,
                cfactor.components.speed,
                cfactor.components.information_flow_rate,
                cfactor.components.first_try_rate,
                cfactor.components.knowledge_growth,
                cfactor.components.turn_taking_equality,
                cfactor.components.social_sensitivity
            ));
        }

        lines.join("\n")
    }

    /// Format this status as JSON.
    #[must_use]
    pub fn display_json(&self) -> String {
        // Manual JSON to avoid pulling in extra serde derives just for status.
        let session = self
            .session_id
            .as_deref()
            .map_or_else(|| "null".to_string(), |s| format!("\"{s}\""));
        let signals = self
            .signal_count
            .map_or_else(|| "null".to_string(), |n| n.to_string());
        let episodes = self
            .episode_count
            .map_or_else(|| "null".to_string(), |n| n.to_string());
        let last = self
            .last_episode_passed
            .map_or_else(|| "null".to_string(), |b| b.to_string());
        let cfactor = self.cfactor.as_ref().map_or_else(
            || "null".to_string(),
            |value| serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
        );

        format!(
            r#"{{"workdir":"{}","session":{},"daemon_running":{},"signal_count":{},"episode_count":{},"last_episode_passed":{},"cfactor":{}}}"#,
            self.workdir.display(),
            session,
            self.daemon_running,
            signals,
            episodes,
            last,
            cfactor,
        )
    }
}

/// Check if a daemon socket exists at the expected location.
#[must_use]
pub fn daemon_socket_exists(workdir: &Path, session_id: &str) -> bool {
    let socket_path = workdir
        .join(".roko")
        .join("run")
        .join(format!("roko-{session_id}.sock"));
    socket_path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_status_defaults() {
        let status = SessionStatus::offline(PathBuf::from("/project"));
        assert!(status.session_id.is_none());
        assert!(!status.daemon_running);
        assert!(status.signal_count.is_none());
        assert!(status.cfactor.is_none());
    }

    #[test]
    fn display_text_basic() {
        let status = SessionStatus {
            session_id: Some("abc-123".into()),
            workdir: PathBuf::from("/project"),
            daemon_running: true,
            signal_count: Some(42),
            episode_count: Some(3),
            last_episode_passed: Some(true),
            cfactor: None,
        };
        let text = status.display_text();
        assert!(text.contains("/project"));
        assert!(text.contains("abc-123"));
        assert!(text.contains("running"));
        assert!(text.contains("42"));
        assert!(text.contains("3"));
        assert!(text.contains("passed"));
    }

    #[test]
    fn display_text_offline() {
        let status = SessionStatus::offline(PathBuf::from("/tmp"));
        let text = status.display_text();
        assert!(text.contains("not running"));
        assert!(!text.contains("signals"));
    }

    #[test]
    fn display_json_with_values() {
        let status = SessionStatus {
            session_id: Some("s1".into()),
            workdir: PathBuf::from("/w"),
            daemon_running: false,
            signal_count: Some(10),
            episode_count: Some(2),
            last_episode_passed: Some(false),
            cfactor: None,
        };
        let json = status.display_json();
        assert!(json.contains(r#""session":"s1""#));
        assert!(json.contains(r#""daemon_running":false"#));
        assert!(json.contains(r#""signal_count":10"#));
        assert!(json.contains(r#""last_episode_passed":false"#));
    }

    #[test]
    fn display_json_with_nulls() {
        let status = SessionStatus::offline(PathBuf::from("/w"));
        let json = status.display_json();
        assert!(json.contains(r#""session":null"#));
        assert!(json.contains(r#""signal_count":null"#));
    }

    #[test]
    fn daemon_socket_exists_returns_false_for_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!daemon_socket_exists(tmp.path(), "nonexistent"));
    }
}
