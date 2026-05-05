//! Process lifecycle management — spawn, track, kill, reap.
//!
//! Extracts the core process supervision patterns from `apps/mori/src/agent/connection.rs`
//! and `apps/mori/src/agent/mod.rs` into reusable, domain-agnostic primitives.
//!
//! The key abstraction is [`ProcessHandle`], which wraps a `tokio::process::Child` and
//! provides:
//! - Unique process identity via [`ProcessId`].
//! - Cooperative shutdown with configurable grace period.
//! - Stdout/stderr stream capture.
//! - Exit status tracking.
//!
//! [`ProcessSupervisor`] manages a pool of handles and provides bulk operations
//! (kill all, reap zombies, etc.).

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use nix::{
    errno::Errno,
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use parking_lot::Mutex;
use tokio::{
    process::{Child, Command},
    task::JoinSet,
    time::timeout,
};
use tracing::{debug, info, warn};

use crate::cancel::CancelToken;

const DEFAULT_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Default location for durable process-session state under a workspace.
#[must_use]
pub fn default_process_session_ledger_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("state")
        .join("process-sessions.json")
}

/// Monotonically increasing process identifier, unique within a single runtime.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ProcessId(pub u64);

impl fmt::Display for ProcessId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "pid:{}", self.0)
    }
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
        })
}

impl ProcessId {
    /// Allocate the next unique process ID.
    pub fn next() -> Self {
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Configuration for spawning a managed process.
#[derive(Debug, Clone)]
pub struct SpawnConfig {
    /// The executable to run.
    pub program: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Working directory. If `None`, inherits the current directory.
    pub working_dir: Option<PathBuf>,
    /// Environment variables to set (additive to the inherited env).
    pub env: HashMap<String, String>,
    /// How long to wait after asking the process to stop before force-killing it.
    pub grace_period: Duration,
    /// Optional external cancellation trigger for this specific child process.
    pub cancellation: Option<CancelToken>,
    /// Optional durable session metadata for resume and interruption diagnosis.
    pub session: Option<ProcessSessionConfig>,
    /// Human-readable label for logging.
    pub label: String,
}

/// Erlang/OTP-style supervision strategy for managed processes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SupervisionStrategy {
    /// Restart only the failed process.
    OneForOne {
        /// Maximum restarts in the configured time window.
        max_restarts: u32,
        /// Sliding restart window in milliseconds.
        within_ms: u64,
        /// Fallback tier label for escalation.
        fallback_tier: String,
    },
    /// Restart every managed process when one fails.
    OneForAll {
        /// Maximum restarts in the configured time window.
        max_restarts: u32,
    },
    /// Restart the failed process and those started after it.
    RestForOne {
        /// Maximum restarts in the configured time window.
        max_restarts: u32,
    },
}

impl Default for SupervisionStrategy {
    fn default() -> Self {
        Self::OneForOne {
            max_restarts: 0,
            within_ms: 0,
            fallback_tier: "standard".into(),
        }
    }
}

impl SupervisionStrategy {
    const fn max_restarts(&self) -> u32 {
        match self {
            Self::OneForOne { max_restarts, .. }
            | Self::OneForAll { max_restarts }
            | Self::RestForOne { max_restarts } => *max_restarts,
        }
    }

    const fn within_ms(&self) -> u64 {
        match self {
            Self::OneForOne { within_ms, .. } => *within_ms,
            Self::OneForAll { .. } | Self::RestForOne { .. } => 0,
        }
    }

    fn fallback_tier(&self) -> Option<&str> {
        match self {
            Self::OneForOne { fallback_tier, .. } => Some(fallback_tier.as_str()),
            _ => None,
        }
    }
}

impl Default for SpawnConfig {
    fn default() -> Self {
        Self {
            program: String::new(),
            args: Vec::new(),
            working_dir: None,
            env: HashMap::new(),
            grace_period: DEFAULT_GRACE_PERIOD,
            cancellation: None,
            session: None,
            label: String::from("unnamed"),
        }
    }
}

/// Durable metadata attached to a supervised child process.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSessionConfig {
    /// Stable session id shared by resume attempts.
    pub session_id: String,
    /// Unique invocation id for this child process.
    pub invocation_id: String,
    /// Backend/provider identifier.
    pub backend_id: String,
    /// Optional task id this process is serving.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Reuse policy id active for this invocation.
    #[serde(default)]
    pub reuse_policy_id: Option<String>,
    /// Whether this interrupted invocation may be resumed.
    #[serde(default)]
    pub resumable: bool,
    /// Optional request timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// JSON file where session records should be persisted.
    pub ledger_path: PathBuf,
}

/// Durable state for a process-backed invocation.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSessionState {
    /// Process was spawned and has not recorded a terminal state.
    Started,
    /// Process exited with status 0.
    Succeeded,
    /// Process exited with a non-zero status.
    Failed,
    /// Supervisor wait timed out and the process remained tracked.
    TimedOut,
    /// Process was cancelled or force-killed by the supervisor.
    Cancelled,
}

impl ProcessSessionState {
    /// Whether this state is terminal for normal execution.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }

    /// Whether a session in this state can be offered for resume.
    #[must_use]
    pub const fn is_resumable(self) -> bool {
        matches!(self, Self::Started | Self::TimedOut | Self::Cancelled)
    }
}

/// One durable process-session ledger row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSessionRecord {
    /// Stable session id shared by resume attempts.
    pub session_id: String,
    /// Unique invocation id for this child process.
    pub invocation_id: String,
    /// Backend/provider identifier.
    pub backend_id: String,
    /// Optional task id this process is serving.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Reuse policy id active for this invocation.
    #[serde(default)]
    pub reuse_policy_id: Option<String>,
    /// Whether interrupted states may be resumed.
    #[serde(default)]
    pub resumable: bool,
    /// Supervisor process id.
    pub process_id: ProcessId,
    /// OS process id, if known.
    #[serde(default)]
    pub os_pid: Option<u32>,
    /// Human-readable process label.
    pub label: String,
    /// Program path.
    pub program: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Unix milliseconds when the process was spawned.
    pub started_at_ms: u64,
    /// Unix milliseconds when the latest state was recorded.
    pub updated_at_ms: u64,
    /// Unix milliseconds when the process ended, if known.
    #[serde(default)]
    pub ended_at_ms: Option<u64>,
    /// Timeout configured for the invocation.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Current durable state.
    pub state: ProcessSessionState,
    /// Optional structured reason string.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Operator-facing aggregate over the durable process-session ledger.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSessionStateSummary {
    /// Total durable records in the ledger.
    pub total: usize,
    /// Records currently in `started`.
    pub started: usize,
    /// Records that reached `succeeded`.
    pub succeeded: usize,
    /// Records that reached `failed`.
    pub failed: usize,
    /// Records that reached `timed_out`.
    pub timed_out: usize,
    /// Records that reached `cancelled`.
    pub cancelled: usize,
    /// Latest records that are eligible for resume under their own metadata.
    pub resumable: usize,
    /// Resumable records older than the operator's configured staleness window.
    pub stale: usize,
    /// Latest update timestamp observed in the ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_updated_at_ms: Option<u64>,
}

/// Resume compatibility policy applied to a durable process-session record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessResumePolicy {
    /// Expected backend/provider id. Mismatches fail closed.
    pub expected_backend_id: Option<String>,
    /// Expected task id. Mismatches fail closed.
    pub expected_task_id: Option<String>,
    /// Maximum allowed age since the latest ledger update.
    pub max_staleness_ms: Option<u64>,
    /// Current wall-clock time in Unix milliseconds.
    pub now_ms: Option<u64>,
}

/// Process-session ledger persisted as JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSessionLedger {
    /// Ledger rows.
    #[serde(default)]
    pub records: Vec<ProcessSessionRecord>,
}

impl ProcessSessionLedger {
    /// Load a ledger from disk. Missing files load as empty ledgers.
    ///
    /// # Errors
    ///
    /// Returns JSON or filesystem errors other than missing-file.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// Save the ledger to disk via tmp + rename.
    ///
    /// # Errors
    ///
    /// Returns filesystem or JSON serialization errors.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    /// Insert or replace a record by invocation id.
    pub fn upsert(&mut self, record: ProcessSessionRecord) {
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|existing| existing.invocation_id == record.invocation_id)
        {
            *existing = record;
        } else {
            self.records.push(record);
        }
    }

    /// Find the newest record for a session id.
    #[must_use]
    pub fn latest_for_session(&self, session_id: &str) -> Option<&ProcessSessionRecord> {
        self.records
            .iter()
            .filter(|record| record.session_id == session_id)
            .max_by_key(|record| record.updated_at_ms)
    }

    /// Validate that a session has a resumable latest record.
    pub fn validate_resume(
        &self,
        session_id: &str,
    ) -> Result<&ProcessSessionRecord, ProcessResumeError> {
        self.validate_resume_with_policy(session_id, &ProcessResumePolicy::default())
    }

    /// Validate that a session has a resumable latest record matching a policy.
    pub fn validate_resume_with_policy(
        &self,
        session_id: &str,
        policy: &ProcessResumePolicy,
    ) -> Result<&ProcessSessionRecord, ProcessResumeError> {
        let record = self
            .latest_for_session(session_id)
            .ok_or(ProcessResumeError::NotFound)?;
        if !record.resumable {
            return Err(ProcessResumeError::NotResumable);
        }
        if !record.state.is_resumable() {
            return Err(ProcessResumeError::TerminalState(record.state));
        }
        if let Some(expected) = &policy.expected_backend_id
            && &record.backend_id != expected
        {
            return Err(ProcessResumeError::BackendMismatch {
                expected: expected.clone(),
                actual: record.backend_id.clone(),
            });
        }
        if let Some(expected) = &policy.expected_task_id
            && record.task_id.as_ref() != Some(expected)
        {
            return Err(ProcessResumeError::TaskMismatch {
                expected: expected.clone(),
                actual: record.task_id.clone(),
            });
        }
        if let Some(max_staleness_ms) = policy.max_staleness_ms {
            let now_ms = policy.now_ms.unwrap_or_else(unix_ms);
            let age_ms = now_ms.saturating_sub(record.updated_at_ms);
            if age_ms > max_staleness_ms {
                return Err(ProcessResumeError::Stale {
                    max_staleness_ms,
                    age_ms,
                });
            }
        }
        Ok(record)
    }

    /// Build an operator-facing state summary.
    #[must_use]
    pub fn state_summary(
        &self,
        stale_after_ms: Option<u64>,
        now_ms: u64,
    ) -> ProcessSessionStateSummary {
        let mut summary = ProcessSessionStateSummary::default();
        for record in &self.records {
            summary.total += 1;
            summary.latest_updated_at_ms = Some(
                summary
                    .latest_updated_at_ms
                    .map_or(record.updated_at_ms, |latest| {
                        latest.max(record.updated_at_ms)
                    }),
            );
            match record.state {
                ProcessSessionState::Started => summary.started += 1,
                ProcessSessionState::Succeeded => summary.succeeded += 1,
                ProcessSessionState::Failed => summary.failed += 1,
                ProcessSessionState::TimedOut => summary.timed_out += 1,
                ProcessSessionState::Cancelled => summary.cancelled += 1,
            }
            if record.resumable && record.state.is_resumable() {
                summary.resumable += 1;
                if stale_after_ms
                    .is_some_and(|max_age| now_ms.saturating_sub(record.updated_at_ms) > max_age)
                {
                    summary.stale += 1;
                }
            }
        }
        summary
    }
}

/// Resume validation error for process session ledgers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessResumeError {
    /// No record exists for the requested session.
    #[error("process session not found")]
    NotFound,
    /// Record is not marked resumable.
    #[error("process session is not marked resumable")]
    NotResumable,
    /// Latest record is terminal.
    #[error("process session reached terminal state {0:?}")]
    TerminalState(ProcessSessionState),
    /// Latest record belongs to a different backend/provider.
    #[error("process session backend mismatch: expected {expected}, found {actual}")]
    BackendMismatch {
        /// Expected backend id.
        expected: String,
        /// Actual backend id in the ledger.
        actual: String,
    },
    /// Latest record belongs to a different task.
    #[error("process session task mismatch: expected {expected}, found {actual:?}")]
    TaskMismatch {
        /// Expected task id.
        expected: String,
        /// Actual task id in the ledger.
        actual: Option<String>,
    },
    /// Latest record is older than the configured resume staleness window.
    #[error("process session is stale: age {age_ms} ms exceeds {max_staleness_ms} ms")]
    Stale {
        /// Configured maximum staleness.
        max_staleness_ms: u64,
        /// Observed age.
        age_ms: u64,
    },
}

/// Outcome of a completed process.
#[derive(Debug, Clone)]
pub struct ProcessOutcome {
    /// The internal process identifier.
    pub id: ProcessId,
    /// Human-readable label.
    pub label: String,
    /// Exit status, if available.
    pub exit_status: Option<ExitStatus>,
    /// True if the process was killed by the supervisor (not a natural exit).
    pub was_killed: bool,
}

/// Live snapshot of a supervised child process.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProcessSnapshot {
    /// The internal process identifier.
    pub id: ProcessId,
    /// Human-readable label.
    pub label: String,
    /// OS process id, if available.
    #[serde(default)]
    pub os_pid: Option<u32>,
    /// Process uptime in milliseconds.
    pub uptime_ms: u64,
    /// Durable session id attached to this process, if any.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Durable invocation id attached to this process, if any.
    #[serde(default)]
    pub invocation_id: Option<String>,
    /// Backend/provider id attached to this process, if any.
    #[serde(default)]
    pub backend_id: Option<String>,
    /// Task id attached to this process, if any.
    #[serde(default)]
    pub task_id: Option<String>,
    /// Reuse policy id attached to this process, if any.
    #[serde(default)]
    pub reuse_policy_id: Option<String>,
    /// Whether this process-session can be offered for resume.
    pub resumable: bool,
    /// Configured timeout in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

/// A handle to a managed child process.
pub struct ProcessHandle {
    /// Internal unique identifier.
    pub id: ProcessId,
    /// Human-readable label.
    pub label: String,
    child: Child,
    os_pid: Option<u32>,
    grace_period: Duration,
    cancel: CancelToken,
    spawn_config: SpawnConfig,
    session: Option<ProcessSessionConfig>,
    started_at: Instant,
}

impl ProcessHandle {
    fn record_session_state(&self, state: ProcessSessionState, reason: Option<String>) {
        let Some(session) = &self.session else {
            return;
        };
        let mut ledger = match ProcessSessionLedger::load(&session.ledger_path) {
            Ok(ledger) => ledger,
            Err(err) => {
                warn!(
                    path = %session.ledger_path.display(),
                    error = %err,
                    "failed to load process session ledger"
                );
                ProcessSessionLedger::default()
            }
        };
        let now = unix_ms();
        let existing_started_at = ledger
            .records
            .iter()
            .find(|record| record.invocation_id == session.invocation_id)
            .map(|record| record.started_at_ms)
            .unwrap_or(now);
        ledger.upsert(ProcessSessionRecord {
            session_id: session.session_id.clone(),
            invocation_id: session.invocation_id.clone(),
            backend_id: session.backend_id.clone(),
            task_id: session.task_id.clone(),
            reuse_policy_id: session.reuse_policy_id.clone(),
            resumable: session.resumable,
            process_id: self.id,
            os_pid: self.os_pid,
            label: self.label.clone(),
            program: self.spawn_config.program.clone(),
            args: self.spawn_config.args.clone(),
            started_at_ms: existing_started_at,
            updated_at_ms: now,
            ended_at_ms: state.is_terminal().then_some(now),
            timeout_ms: session.timeout_ms,
            state,
            reason,
        });
        if let Err(err) = ledger.save(&session.ledger_path) {
            warn!(
                path = %session.ledger_path.display(),
                error = %err,
                "failed to save process session ledger"
            );
        }
    }

    fn outcome(&self, exit_status: Option<ExitStatus>, was_killed: bool) -> ProcessOutcome {
        ProcessOutcome {
            id: self.id,
            label: self.label.clone(),
            exit_status,
            was_killed,
        }
    }

    async fn wait_for_graceful_exit(&mut self) -> Option<ProcessOutcome> {
        match timeout(self.grace_period, self.child.wait()).await {
            Ok(Ok(status)) => {
                info!(id = %self.id, label = %self.label, code = ?status.code(), "process exited within grace period");
                Some(self.outcome(Some(status), false))
            }
            Ok(Err(e)) => {
                warn!(id = %self.id, label = %self.label, error = %e, "error waiting for process");
                None
            }
            Err(_) => {
                debug!(id = %self.id, label = %self.label, "grace period expired, force killing");
                None
            }
        }
    }

    fn try_wait_outcome(&mut self, was_killed: bool) -> Option<ProcessOutcome> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(self.outcome(Some(status), was_killed)),
            Ok(None) => None,
            Err(err) => {
                warn!(id = %self.id, label = %self.label, error = %err, "error checking process status");
                None
            }
        }
    }

    #[cfg(unix)]
    fn send_signal(&mut self, signal: Signal) {
        match self.child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {}
            Err(err) => {
                warn!(id = %self.id, label = %self.label, error = %err, "error checking process status before signalling");
            }
        }

        let Some(pid) = self.os_pid else {
            return;
        };

        let Ok(raw_pid) = i32::try_from(pid) else {
            warn!(id = %self.id, label = %self.label, os_pid = pid, "child pid does not fit into a platform pid_t");
            return;
        };

        match kill(Pid::from_raw(raw_pid), signal) {
            Ok(()) => {
                debug!(id = %self.id, label = %self.label, signal = ?signal, "sent signal to child process");
            }
            Err(Errno::ESRCH) => {
                debug!(id = %self.id, label = %self.label, signal = ?signal, "child already exited before signalling");
            }
            Err(err) => {
                warn!(id = %self.id, label = %self.label, signal = ?signal, error = %err, "failed to send signal to child process");
            }
        }
    }

    async fn force_kill(&mut self) -> ProcessOutcome {
        if let Some(outcome) = self.try_wait_outcome(false) {
            self.record_session_state(
                if outcome
                    .exit_status
                    .as_ref()
                    .is_some_and(std::process::ExitStatus::success)
                {
                    ProcessSessionState::Succeeded
                } else {
                    ProcessSessionState::Failed
                },
                Some("process exited before force kill".to_string()),
            );
            return outcome;
        }

        self.cancel.cancel();
        drop(self.child.stdin.take());

        #[cfg(unix)]
        self.send_signal(Signal::SIGKILL);

        let _ = self.child.start_kill();
        let status = self.child.wait().await.ok();
        warn!(id = %self.id, label = %self.label, "process force-killed");
        self.record_session_state(
            ProcessSessionState::Cancelled,
            Some("force killed".to_string()),
        );
        self.outcome(status, true)
    }

    fn force_kill_sync(&mut self) {
        self.cancel.cancel();
        drop(self.child.stdin.take());

        #[cfg(unix)]
        self.send_signal(Signal::SIGKILL);

        let _ = self.child.start_kill();
    }

    /// The OS-level PID, if available.
    pub const fn os_pid(&self) -> Option<u32> {
        self.os_pid
    }

    /// Check if the process has exited without blocking.
    ///
    /// # Errors
    ///
    /// Returns any I/O error reported by the underlying child-process handle
    /// while querying its exit status.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Wait for the process to exit.
    ///
    /// # Errors
    ///
    /// Returns any I/O error reported by the underlying child-process handle
    /// while waiting for the process to exit.
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        self.child.wait().await
    }

    /// Take stdout from the child. Can only be called once.
    #[allow(clippy::missing_const_for_fn)] // &mut Child is not const-compatible
    pub fn take_stdout(&mut self) -> Option<tokio::process::ChildStdout> {
        self.child.stdout.take()
    }

    /// Take stderr from the child. Can only be called once.
    #[allow(clippy::missing_const_for_fn)] // &mut Child is not const-compatible
    pub fn take_stderr(&mut self) -> Option<tokio::process::ChildStderr> {
        self.child.stderr.take()
    }

    /// Take stdin from the child. Can only be called once.
    #[allow(clippy::missing_const_for_fn)] // &mut Child is not const-compatible
    pub fn take_stdin(&mut self) -> Option<tokio::process::ChildStdin> {
        self.child.stdin.take()
    }

    /// Gracefully shut down: on Unix send `SIGTERM`, wait `grace_period`, then
    /// escalate to a force kill. On Windows the supervisor waits for the grace
    /// period and then uses `Child::kill()`.
    pub async fn shutdown(&mut self) -> ProcessOutcome {
        debug!(id = %self.id, label = %self.label, "shutting down process");
        self.cancel.cancel();
        drop(self.child.stdin.take());

        if let Some(outcome) = self.try_wait_outcome(false) {
            self.record_session_state(
                if outcome
                    .exit_status
                    .as_ref()
                    .is_some_and(std::process::ExitStatus::success)
                {
                    ProcessSessionState::Succeeded
                } else {
                    ProcessSessionState::Failed
                },
                Some("process exited before shutdown".to_string()),
            );
            return outcome;
        }

        #[cfg(unix)]
        self.send_signal(Signal::SIGTERM);

        if let Some(outcome) = self.wait_for_graceful_exit().await {
            self.record_session_state(
                ProcessSessionState::Cancelled,
                Some("shutdown requested".to_string()),
            );
            return outcome;
        }
        self.force_kill().await
    }

    /// The cancellation token associated with this process.
    pub const fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }

    /// How long this process has been alive.
    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }
}

enum WaitResult {
    Completed(ProcessOutcome),
    TimedOut(Box<ProcessHandle>),
}

impl fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessHandle")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("os_pid", &self.os_pid)
            .finish_non_exhaustive()
    }
}

/// Manages a pool of child processes with bulk lifecycle operations.
pub struct ProcessSupervisor {
    handles: Arc<Mutex<HashMap<ProcessId, ProcessHandle>>>,
    restart_history: Mutex<HashMap<String, Vec<Instant>>>,
    cancel: CancelToken,
    strategy: SupervisionStrategy,
}

impl ProcessSupervisor {
    /// Create a new supervisor with a root cancellation token.
    pub fn new(cancel: CancelToken) -> Self {
        Self {
            handles: Arc::new(Mutex::new(HashMap::new())),
            restart_history: Mutex::new(HashMap::new()),
            cancel,
            strategy: SupervisionStrategy::default(),
        }
    }

    /// Override the supervision strategy.
    #[must_use]
    pub fn with_strategy(mut self, strategy: SupervisionStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Current supervision strategy.
    #[must_use]
    pub const fn strategy(&self) -> &SupervisionStrategy {
        &self.strategy
    }

    /// Spawn a new managed process.
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from configuring or spawning the child process.
    pub async fn spawn(&self, config: SpawnConfig) -> std::io::Result<ProcessId> {
        let id = ProcessId::next();
        let child_cancel = self.cancel.child();
        let external_cancellation = config.cancellation.clone();
        let spawn_config = config.clone();
        let session = config.session.clone();
        let label = config.label.clone();

        let mut cmd = Command::new(&config.program);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        if let Some(ref dir) = config.working_dir {
            cmd.current_dir(dir);
        }
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let child = cmd.spawn()?;
        let os_pid = child.id();

        info!(
            id = %id,
            label = %config.label,
            os_pid = ?os_pid,
            program = %config.program,
            "spawned process"
        );

        let handle = ProcessHandle {
            id,
            label,
            child,
            os_pid,
            grace_period: config.grace_period,
            cancel: child_cancel,
            spawn_config,
            session,
            started_at: Instant::now(),
        };

        handle.record_session_state(ProcessSessionState::Started, None);
        self.handles.lock().insert(id, handle);

        if let Some(token) = external_cancellation {
            let handles = Arc::clone(&self.handles);
            std::mem::drop(tokio::spawn(async move {
                token.cancelled().await;
                let mut handle = { handles.lock().remove(&id) };
                if let Some(mut handle) = handle.take() {
                    let _ = handle.shutdown().await;
                }
            }));
        }

        Ok(id)
    }

    /// Remove and return a process handle (for exclusive ownership).
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    pub async fn take(&self, id: ProcessId) -> Option<ProcessHandle> {
        self.handles.lock().remove(&id)
    }

    /// Shut down a single process by ID.
    pub async fn shutdown(&self, id: ProcessId) -> Option<ProcessOutcome> {
        let mut handle = self.handles.lock().remove(&id)?;
        Some(handle.shutdown().await)
    }

    /// Shut down all managed processes, returning their outcomes.
    pub async fn shutdown_all(&self) -> Vec<ProcessOutcome> {
        self.cancel.cancel();
        let handles: Vec<_> = {
            let mut map = self.handles.lock();
            map.drain().map(|(_, h)| h).collect()
        };

        let mut outcomes = Vec::with_capacity(handles.len());
        for mut handle in handles {
            outcomes.push(handle.shutdown().await);
        }
        outcomes
    }

    /// Wait for all managed processes to exit within `timeout` per process.
    ///
    /// Any process that does not exit in time is reinserted into the
    /// supervisor so a subsequent [`ProcessSupervisor::kill_all`] call can
    /// forcefully terminate it.
    pub async fn wait_all(&self, wait_timeout: Duration) -> Vec<ProcessOutcome> {
        let handles: Vec<_> = {
            let mut map = self.handles.lock();
            map.drain().map(|(_, handle)| handle).collect()
        };

        let mut waiters = JoinSet::new();
        for mut handle in handles {
            waiters.spawn(async move {
                let id = handle.id;
                match timeout(wait_timeout, handle.wait()).await {
                    Ok(Ok(status)) => {
                        let state = if status.success() {
                            ProcessSessionState::Succeeded
                        } else {
                            ProcessSessionState::Failed
                        };
                        handle.record_session_state(state, Some("process exited".to_string()));
                        WaitResult::Completed(handle.outcome(Some(status), false))
                    }
                    Ok(Err(err)) => {
                        warn!(id = %id, label = %handle.label, error = %err, "error waiting for process during shutdown");
                        handle.record_session_state(
                            ProcessSessionState::TimedOut,
                            Some(format!("wait error: {err}")),
                        );
                        WaitResult::TimedOut(Box::new(handle))
                    }
                    Err(_) => {
                        debug!(id = %id, label = %handle.label, "process wait timed out during shutdown");
                        handle.record_session_state(
                            ProcessSessionState::TimedOut,
                            Some(format!("wait timed out after {} ms", wait_timeout.as_millis())),
                        );
                        WaitResult::TimedOut(Box::new(handle))
                    }
                }
            });
        }

        let mut completed = Vec::new();
        let mut timed_out = Vec::new();
        while let Some(result) = waiters.join_next().await {
            match result {
                Ok(WaitResult::Completed(outcome)) => completed.push(outcome),
                Ok(WaitResult::TimedOut(handle)) => timed_out.push(*handle),
                Err(err) => {
                    warn!(error = %err, "process shutdown wait task failed");
                }
            }
        }

        if !timed_out.is_empty() {
            let mut map = self.handles.lock();
            for handle in timed_out {
                map.insert(handle.id, handle);
            }
        }

        completed
    }

    /// Force-kill all managed processes and return their outcomes.
    pub async fn kill_all(&self) -> Vec<ProcessOutcome> {
        self.cancel.cancel();
        let handles: Vec<_> = {
            let mut map = self.handles.lock();
            map.drain().map(|(_, handle)| handle).collect()
        };

        let mut outcomes = Vec::with_capacity(handles.len());
        for mut handle in handles {
            outcomes.push(handle.force_kill().await);
        }
        outcomes
    }

    /// Reap processes that have already exited (non-blocking).
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    pub async fn reap_exited(&self) -> Vec<ProcessOutcome> {
        let mut map = self.handles.lock();
        let mut exited_ids = Vec::new();

        for (id, handle) in map.iter_mut() {
            match handle.try_wait() {
                Ok(Some(status)) => {
                    exited_ids.push((*id, status));
                }
                Ok(None) => {} // still running
                Err(e) => {
                    warn!(id = %id, error = %e, "error checking process status");
                }
            }
        }

        let mut outcomes = Vec::new();
        for (id, status) in exited_ids {
            if let Some(handle) = map.remove(&id) {
                debug!(id = %id, label = %handle.label, code = ?status.code(), "reaped exited process");
                handle.record_session_state(
                    if status.success() {
                        ProcessSessionState::Succeeded
                    } else {
                        ProcessSessionState::Failed
                    },
                    Some("reaped exited process".to_string()),
                );
                outcomes.push(ProcessOutcome {
                    id: handle.id,
                    label: handle.label,
                    exit_status: Some(status),
                    was_killed: false,
                });
            }
        }
        outcomes
    }

    /// Number of currently tracked processes.
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    pub async fn count(&self) -> usize {
        self.handles.lock().len()
    }

    /// List all tracked process IDs and their labels.
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    pub async fn list(&self) -> Vec<(ProcessId, String)> {
        self.handles
            .lock()
            .iter()
            .map(|(id, h)| (*id, h.label.clone()))
            .collect()
    }

    /// List detailed live process snapshots for operator status surfaces.
    #[allow(clippy::unused_async)] // Preserve the existing async API style for supervisor reads.
    pub async fn snapshots(&self) -> Vec<ProcessSnapshot> {
        self.handles
            .lock()
            .values()
            .map(|handle| {
                let uptime_ms =
                    u64::try_from(handle.uptime().as_millis().min(u128::from(u64::MAX)))
                        .unwrap_or(u64::MAX);
                ProcessSnapshot {
                    id: handle.id,
                    label: handle.label.clone(),
                    os_pid: handle.os_pid,
                    uptime_ms,
                    session_id: handle
                        .session
                        .as_ref()
                        .map(|session| session.session_id.clone()),
                    invocation_id: handle
                        .session
                        .as_ref()
                        .map(|session| session.invocation_id.clone()),
                    backend_id: handle
                        .session
                        .as_ref()
                        .map(|session| session.backend_id.clone()),
                    task_id: handle
                        .session
                        .as_ref()
                        .and_then(|session| session.task_id.clone()),
                    reuse_policy_id: handle
                        .session
                        .as_ref()
                        .and_then(|session| session.reuse_policy_id.clone()),
                    resumable: handle
                        .session
                        .as_ref()
                        .is_some_and(|session| session.resumable),
                    timeout_ms: handle
                        .session
                        .as_ref()
                        .and_then(|session| session.timeout_ms),
                }
            })
            .collect()
    }

    /// List all tracked OS process IDs and their labels.
    ///
    /// Entries without an assigned OS PID are skipped.
    #[allow(clippy::unused_async)] // Preserve the existing async API for callers across crates.
    pub async fn active_pids(&self) -> Vec<(u32, String)> {
        self.handles
            .lock()
            .values()
            .filter_map(|handle| handle.os_pid.map(|pid| (pid, handle.label.clone())))
            .collect()
    }

    /// Restart one process according to the configured strategy.
    pub async fn restart_process(&self, id: ProcessId) -> Option<ProcessId> {
        let mut handle = self.handles.lock().remove(&id)?;
        let label = handle.label.clone();
        let strategy = self.strategy.clone();
        let fallback_tier = strategy.fallback_tier().unwrap_or("standard");

        if !self.allow_restart(&label, strategy.max_restarts(), strategy.within_ms()) {
            warn!(
                id = %id,
                label = %label,
                strategy = ?strategy,
                fallback_tier = %fallback_tier,
                "restart budget exhausted"
            );
            return None;
        }

        let _ = handle.shutdown().await;
        let config = handle.spawn_config.clone();
        match self.spawn(config).await {
            Ok(new_id) => {
                info!(old_id = %id, new_id = %new_id, label = %label, "restarted process");
                Some(new_id)
            }
            Err(err) => {
                warn!(old_id = %id, label = %label, error = %err, "failed to restart process");
                None
            }
        }
    }

    /// Restart the failed process and any peers selected by the strategy.
    pub async fn restart_wave(&self, failed: ProcessId) -> Vec<ProcessId> {
        let ids = self.recovery_targets(failed);
        let mut restarted = Vec::new();
        for id in ids {
            if let Some(new_id) = self.restart_process(id).await {
                restarted.push(new_id);
            }
        }
        restarted
    }

    fn recovery_targets(&self, failed: ProcessId) -> Vec<ProcessId> {
        let mut ids: Vec<ProcessId> = self.handles.lock().keys().copied().collect();
        ids.sort_by_key(|id| id.0);

        match self.strategy {
            SupervisionStrategy::OneForOne { .. } => {
                ids.into_iter().filter(|id| *id == failed).collect()
            }
            SupervisionStrategy::OneForAll { .. } => ids,
            SupervisionStrategy::RestForOne { .. } => {
                ids.into_iter().filter(|id| id.0 >= failed.0).collect()
            }
        }
    }

    fn allow_restart(&self, label: &str, max_restarts: u32, within_ms: u64) -> bool {
        if max_restarts == 0 {
            return false;
        }

        let mut history = self.restart_history.lock();
        let entries = history.entry(label.to_string()).or_default();
        let now = Instant::now();
        if within_ms > 0 {
            let window = Duration::from_millis(within_ms);
            entries.retain(|ts| now.duration_since(*ts) <= window);
        }

        let max_restarts = usize::try_from(max_restarts).unwrap_or(usize::MAX);
        if entries.len() >= max_restarts {
            drop(history);
            return false;
        }

        entries.push(now);
        drop(history);
        true
    }
}

impl Drop for ProcessSupervisor {
    fn drop(&mut self) {
        self.cancel.cancel();

        let children = {
            let mut handles = self.handles.lock();
            if handles.is_empty() {
                return;
            }

            warn!(
                count = handles.len(),
                "ProcessSupervisor dropped with live children; force-killing"
            );
            std::mem::take(&mut *handles)
        };

        for (_, mut handle) in children {
            handle.force_kill_sync();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_and_reap() {
        let cancel = CancelToken::new();
        let supervisor = ProcessSupervisor::new(cancel);

        let id = supervisor
            .spawn(SpawnConfig {
                program: "echo".into(),
                args: vec!["hello".into()],
                label: "test-echo".into(),
                ..Default::default()
            })
            .await
            .expect("spawn should succeed");

        // Give process time to finish.
        tokio::time::sleep(Duration::from_millis(100)).await;

        let reaped = supervisor.reap_exited().await;
        assert_eq!(reaped.len(), 1);
        assert_eq!(reaped[0].id, id);
        assert!(!reaped[0].was_killed);
    }

    #[tokio::test]
    async fn shutdown_all() {
        let cancel = CancelToken::new();
        let supervisor = ProcessSupervisor::new(cancel);

        // Spawn a long-running process.
        supervisor
            .spawn(SpawnConfig {
                program: "sleep".into(),
                args: vec!["60".into()],
                label: "test-sleep".into(),
                grace_period: Duration::from_millis(100),
                ..Default::default()
            })
            .await
            .expect("spawn should succeed");

        assert_eq!(supervisor.count().await, 1);

        let outcomes = supervisor.shutdown_all().await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(supervisor.count().await, 0);
    }

    #[tokio::test]
    async fn active_pids_reports_spawned_processes() {
        let cancel = CancelToken::new();
        let supervisor = ProcessSupervisor::new(cancel);

        let id = supervisor
            .spawn(SpawnConfig {
                program: "sleep".into(),
                args: vec!["60".into()],
                label: "test-active-pids".into(),
                ..Default::default()
            })
            .await
            .expect("spawn should succeed");

        let active_pids = supervisor.active_pids().await;
        assert_eq!(active_pids.len(), 1);
        assert_eq!(
            supervisor.list().await,
            vec![(id, "test-active-pids".into())]
        );
        assert_eq!(active_pids[0].1, "test-active-pids");
        assert!(active_pids[0].0 > 0);
    }

    #[test]
    fn process_id_uniqueness() {
        let a = ProcessId::next();
        let b = ProcessId::next();
        assert_ne!(a, b);
    }
}
