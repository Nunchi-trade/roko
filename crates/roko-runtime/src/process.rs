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
    path::PathBuf,
    process::ExitStatus,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use tokio::{
    process::{Child, Command},
    sync::Mutex,
    task::JoinSet,
    time::timeout,
};
use tracing::{debug, info, warn};

use crate::cancel::CancelToken;

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
    fn max_restarts(&self) -> u32 {
        match self {
            Self::OneForOne { max_restarts, .. }
            | Self::OneForAll { max_restarts }
            | Self::RestForOne { max_restarts } => *max_restarts,
        }
    }

    fn within_ms(&self) -> u64 {
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
            grace_period: Duration::from_secs(5),
            label: String::from("unnamed"),
        }
    }
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
    started_at: Instant,
}

impl ProcessHandle {
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

    async fn force_kill(&mut self) -> ProcessOutcome {
        let _ = self.child.kill().await;
        let status = self.child.wait().await.ok();
        warn!(id = %self.id, label = %self.label, "process force-killed");
        self.outcome(status, true)
    }

    /// The OS-level PID, if available.
    pub const fn os_pid(&self) -> Option<u32> {
        self.os_pid
    }

    /// Check if the process has exited without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    /// Wait for the process to exit.
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

    /// Gracefully shut down: attempt clean exit via `kill()`, wait `grace_period`,
    /// then force kill.
    ///
    /// On Unix, Tokio's `Child::kill()` sends SIGKILL. For a gentler approach,
    /// callers should write a shutdown command to stdin before calling this.
    pub async fn shutdown(&mut self) -> ProcessOutcome {
        debug!(id = %self.id, label = %self.label, "shutting down process");
        self.cancel.cancel();
        if let Some(outcome) = self.wait_for_graceful_exit().await {
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
    TimedOut(ProcessHandle),
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
    handles: Mutex<HashMap<ProcessId, ProcessHandle>>,
    restart_history: Mutex<HashMap<String, Vec<Instant>>>,
    cancel: CancelToken,
    strategy: SupervisionStrategy,
}

impl ProcessSupervisor {
    /// Create a new supervisor with a root cancellation token.
    pub fn new(cancel: CancelToken) -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
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
    pub async fn spawn(&self, config: SpawnConfig) -> std::io::Result<ProcessId> {
        let id = ProcessId::next();
        let child_cancel = self.cancel.child();
        let spawn_config = config.clone();
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
            started_at: Instant::now(),
        };

        self.handles.lock().await.insert(id, handle);
        Ok(id)
    }

    /// Remove and return a process handle (for exclusive ownership).
    pub async fn take(&self, id: ProcessId) -> Option<ProcessHandle> {
        self.handles.lock().await.remove(&id)
    }

    /// Shut down a single process by ID.
    pub async fn shutdown(&self, id: ProcessId) -> Option<ProcessOutcome> {
        let mut handle = self.handles.lock().await.remove(&id)?;
        Some(handle.shutdown().await)
    }

    /// Shut down all managed processes, returning their outcomes.
    pub async fn shutdown_all(&self) -> Vec<ProcessOutcome> {
        self.cancel.cancel();
        let handles: Vec<_> = {
            let mut map = self.handles.lock().await;
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
            let mut map = self.handles.lock().await;
            map.drain().map(|(_, handle)| handle).collect()
        };

        let mut waiters = JoinSet::new();
        for mut handle in handles {
            waiters.spawn(async move {
                let id = handle.id;
                match timeout(wait_timeout, handle.wait()).await {
                    Ok(Ok(status)) => WaitResult::Completed(handle.outcome(Some(status), false)),
                    Ok(Err(err)) => {
                        warn!(id = %id, label = %handle.label, error = %err, "error waiting for process during shutdown");
                        WaitResult::TimedOut(handle)
                    }
                    Err(_) => {
                        debug!(id = %id, label = %handle.label, "process wait timed out during shutdown");
                        WaitResult::TimedOut(handle)
                    }
                }
            });
        }

        let mut completed = Vec::new();
        let mut timed_out = Vec::new();
        while let Some(result) = waiters.join_next().await {
            match result {
                Ok(WaitResult::Completed(outcome)) => completed.push(outcome),
                Ok(WaitResult::TimedOut(handle)) => timed_out.push(handle),
                Err(err) => {
                    warn!(error = %err, "process shutdown wait task failed");
                }
            }
        }

        if !timed_out.is_empty() {
            let mut map = self.handles.lock().await;
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
            let mut map = self.handles.lock().await;
            map.drain().map(|(_, handle)| handle).collect()
        };

        let mut outcomes = Vec::with_capacity(handles.len());
        for mut handle in handles {
            outcomes.push(handle.force_kill().await);
        }
        outcomes
    }

    /// Reap processes that have already exited (non-blocking).
    pub async fn reap_exited(&self) -> Vec<ProcessOutcome> {
        let mut map = self.handles.lock().await;
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
    pub async fn count(&self) -> usize {
        self.handles.lock().await.len()
    }

    /// List all tracked process IDs and their labels.
    pub async fn list(&self) -> Vec<(ProcessId, String)> {
        self.handles
            .lock()
            .await
            .iter()
            .map(|(id, h)| (*id, h.label.clone()))
            .collect()
    }

    /// List all tracked OS process IDs and their labels.
    ///
    /// Entries without an assigned OS PID are skipped.
    pub async fn active_pids(&self) -> Vec<(u32, String)> {
        self.handles
            .lock()
            .await
            .values()
            .filter_map(|handle| handle.os_pid.map(|pid| (pid, handle.label.clone())))
            .collect()
    }

    /// Restart one process according to the configured strategy.
    pub async fn restart_process(&self, id: ProcessId) -> Option<ProcessId> {
        let mut handle = self.handles.lock().await.remove(&id)?;
        let label = handle.label.clone();
        let strategy = self.strategy.clone();
        let fallback_tier = strategy.fallback_tier().unwrap_or("standard");

        if !self
            .allow_restart(&label, strategy.max_restarts(), strategy.within_ms())
            .await
        {
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
        let ids = self.recovery_targets(failed).await;
        let mut restarted = Vec::new();
        for id in ids {
            if let Some(new_id) = self.restart_process(id).await {
                restarted.push(new_id);
            }
        }
        restarted
    }

    async fn recovery_targets(&self, failed: ProcessId) -> Vec<ProcessId> {
        let mut ids: Vec<ProcessId> = self.handles.lock().await.keys().copied().collect();
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

    async fn allow_restart(&self, label: &str, max_restarts: u32, within_ms: u64) -> bool {
        if max_restarts == 0 {
            return false;
        }

        let mut history = self.restart_history.lock().await;
        let entries = history.entry(label.to_string()).or_default();
        let now = Instant::now();
        if within_ms > 0 {
            let window = Duration::from_millis(within_ms);
            entries.retain(|ts| now.duration_since(*ts) <= window);
        }

        if entries.len() as u32 >= max_restarts {
            return false;
        }

        entries.push(now);
        true
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
