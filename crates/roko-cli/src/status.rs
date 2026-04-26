//! `roko status` subcommand — queries a running daemon or local substrate.
//!
//! When a daemon is running, `roko status` connects to its Unix socket and
//! retrieves live session information. When no daemon is found, it falls
//! back to reading the local `.roko/` substrate for signal counts.

use std::path::{Path, PathBuf};

use crate::daemon::DaemonInfo;
use roko_learn::cfactor::CFactor;
use roko_learn::episode_logger::Episode;
use roko_runtime::process::{
    ProcessSessionLedger, ProcessSessionStateSummary, default_process_session_ledger_path,
};
use sysinfo::{Pid, ProcessesToUpdate, System};

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
    /// Total recorded cost in USD.
    pub total_cost_usd: Option<f64>,
    /// Recorded cost for the current UTC day in USD.
    pub today_cost_usd: Option<f64>,
    /// Durable process-session ledger path, when readable or configured.
    pub process_session_ledger: Option<PathBuf>,
    /// Durable process-session state summary for restart/resume diagnosis.
    pub process_sessions: Option<ProcessSessionStateSummary>,
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
            total_cost_usd: None,
            today_cost_usd: None,
            process_session_ledger: None,
            process_sessions: None,
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
        if let Some(cost) = self.total_cost_usd {
            lines.push(format!("total cost: ${cost:.4}"));
        }
        if let Some(cost) = self.today_cost_usd {
            lines.push(format!("today cost: ${cost:.4}"));
        }
        if let Some(summary) = &self.process_sessions {
            lines.push(format!(
                "process sessions: total={} started={} timed_out={} cancelled={} failed={} resumable={} stale={}",
                summary.total,
                summary.started,
                summary.timed_out,
                summary.cancelled,
                summary.failed,
                summary.resumable,
                summary.stale
            ));
            if let Some(path) = &self.process_session_ledger {
                lines.push(format!("  ledger: {}", path.display()));
            }
        }
        if let Some(cfactor) = &self.cfactor {
            lines.push(format!(
                "cfactor: {:.3} (episodes: {}, computed: {})",
                cfactor.overall, cfactor.episode_count, cfactor.computed_at
            ));
            lines.push(format!(
                "  gate={:.3} cost={:.3} speed={:.3} flow={:.3} first_try={:.3} knowledge={:.3} integration={:.3} convergence={:.3} turn={:.3} social={:.3}",
                cfactor.components.gate_pass_rate,
                cfactor.components.cost_efficiency,
                cfactor.components.speed,
                cfactor.components.information_flow_rate,
                cfactor.components.first_try_rate,
                cfactor.components.knowledge_growth,
                cfactor.components.knowledge_integration_rate,
                cfactor.components.convergence_velocity,
                cfactor.components.turn_taking_equality,
                cfactor.components.social_perceptiveness
            ));
            if !cfactor.agent_contributions.is_empty() {
                let top = cfactor.top_agent_contribution_lines(3).join(", ");
                lines.push(format!("  agent contributions: {top}"));
            }
        }

        lines.join("\n")
    }

    /// Format this status as JSON.
    #[must_use]
    pub fn display_json(&self) -> String {
        serde_json::json!({
            "workdir": self.workdir.display().to_string(),
            "session": &self.session_id,
            "daemon_running": self.daemon_running,
            "signal_count": self.signal_count,
            "episode_count": self.episode_count,
            "last_episode_passed": self.last_episode_passed,
            "cfactor": &self.cfactor,
            "total_cost_usd": self.total_cost_usd,
            "today_cost_usd": self.today_cost_usd,
            "process_session_ledger": self
                .process_session_ledger
                .as_ref()
                .map(|path| path.display().to_string()),
            "process_sessions": &self.process_sessions,
        })
        .to_string()
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

/// Collect a lightweight status snapshot from on-disk workspace state.
#[must_use]
pub fn collect_session_status(workdir: &Path) -> SessionStatus {
    collect_session_status_with_process_ledger(
        workdir,
        &default_process_session_ledger_path(workdir),
        Some(24 * 60 * 60 * 1_000),
    )
}

/// Collect status and summarize a configured process-session ledger.
#[must_use]
pub fn collect_session_status_with_process_ledger(
    workdir: &Path,
    ledger_path: &Path,
    stale_after_ms: Option<u64>,
) -> SessionStatus {
    let daemon_info = read_daemon_info(workdir);
    let signal_count = Some(count_non_empty_lines(
        &workdir.join(".roko").join("engrams.jsonl"),
    ));
    let (episode_count, last_episode_passed) = read_episode_summary(workdir);
    let process_sessions = read_process_session_summary(ledger_path, stale_after_ms);

    SessionStatus {
        session_id: daemon_info.as_ref().map(|info| info.session_id.clone()),
        workdir: workdir.to_path_buf(),
        daemon_running: daemon_info
            .as_ref()
            .is_some_and(|info| process_is_alive(info.pid)),
        signal_count,
        episode_count: Some(episode_count),
        last_episode_passed,
        cfactor: None,
        total_cost_usd: None,
        today_cost_usd: None,
        process_session_ledger: Some(ledger_path.to_path_buf()),
        process_sessions,
    }
}

fn read_process_session_summary(
    ledger_path: &Path,
    stale_after_ms: Option<u64>,
) -> Option<ProcessSessionStateSummary> {
    if !ledger_path.is_file() {
        return None;
    }
    let ledger = ProcessSessionLedger::load(ledger_path).ok()?;
    Some(ledger.state_summary(stale_after_ms, unix_ms()))
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
        })
}

fn read_daemon_info(workdir: &Path) -> Option<DaemonInfo> {
    let path = workdir.join(".roko").join("daemon.json");
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn process_is_alive(pid: u32) -> bool {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.process(Pid::from_u32(pid)).is_some()
}

fn count_non_empty_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

fn read_episode_summary(workdir: &Path) -> (usize, Option<bool>) {
    let primary = workdir.join(".roko").join("memory").join("episodes.jsonl");
    let fallback = workdir.join(".roko").join("episodes.jsonl");
    let path = if primary.exists() { primary } else { fallback };
    let Ok(text) = std::fs::read_to_string(path) else {
        return (0, None);
    };

    let mut count = 0;
    let mut last_episode = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        count += 1;
        last_episode = serde_json::from_str::<Episode>(line).ok().or(last_episode);
    }

    (count, last_episode.map(|episode| episode.success))
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
        assert!(status.total_cost_usd.is_none());
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
            total_cost_usd: Some(12.5),
            today_cost_usd: Some(1.25),
            process_session_ledger: None,
            process_sessions: None,
        };
        let text = status.display_text();
        assert!(text.contains("/project"));
        assert!(text.contains("abc-123"));
        assert!(text.contains("running"));
        assert!(text.contains("42"));
        assert!(text.contains("3"));
        assert!(text.contains("passed"));
        assert!(text.contains("12.5000"));
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
            total_cost_usd: Some(4.2),
            today_cost_usd: Some(0.7),
            process_session_ledger: None,
            process_sessions: None,
        };
        let json = status.display_json();
        assert!(json.contains(r#""session":"s1""#));
        assert!(json.contains(r#""daemon_running":false"#));
        assert!(json.contains(r#""signal_count":10"#));
        assert!(json.contains(r#""last_episode_passed":false"#));
        assert!(json.contains(r#""total_cost_usd":4.2"#));
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

    #[test]
    fn collect_session_status_reads_signal_and_episode_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let roko = tmp.path().join(".roko");
        let memory = roko.join("memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(roko.join("engrams.jsonl"), "a\nb\n").unwrap();
        std::fs::write(
            memory.join("episodes.jsonl"),
            "{\"success\":false}\n{\"success\":true}\n",
        )
        .unwrap();

        let status = collect_session_status(tmp.path());
        assert_eq!(status.signal_count, Some(2));
        assert_eq!(status.episode_count, Some(2));
        assert_eq!(status.last_episode_passed, Some(true));
        assert!(!status.daemon_running);
    }

    #[test]
    fn collect_session_status_reads_process_session_summary() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger_path = tmp.path().join(".roko/state/process-sessions.json");
        let mut ledger = ProcessSessionLedger::default();
        ledger.upsert(roko_runtime::process::ProcessSessionRecord {
            session_id: "session-1".into(),
            invocation_id: "invocation-1".into(),
            backend_id: "backend".into(),
            task_id: Some("task-1".into()),
            reuse_policy_id: None,
            resumable: true,
            process_id: roko_runtime::process::ProcessId(1),
            os_pid: None,
            label: "agent".into(),
            program: "sleep".into(),
            args: Vec::new(),
            started_at_ms: 1,
            updated_at_ms: 1,
            ended_at_ms: None,
            timeout_ms: None,
            state: roko_runtime::process::ProcessSessionState::TimedOut,
            reason: None,
        });
        ledger.save(&ledger_path).unwrap();

        let status = collect_session_status_with_process_ledger(tmp.path(), &ledger_path, Some(1));
        let summary = status.process_sessions.expect("process summary");
        assert_eq!(summary.total, 1);
        assert_eq!(summary.timed_out, 1);
        assert_eq!(summary.resumable, 1);
        assert_eq!(summary.stale, 1);
    }
}
