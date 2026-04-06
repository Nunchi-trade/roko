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
    time::Duration,
};

use tokio::{
    process::{Child, Command},
    sync::Mutex,
    time::timeout,
};
use tracing::{debug, info, warn};

use crate::cancel::CancelToken;

/// Monotonically increasing process identifier, unique within a single runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
}

impl ProcessHandle {
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

        // First try waiting briefly — the process may already be exiting.
        match timeout(self.grace_period, self.child.wait()).await {
            Ok(Ok(status)) => {
                info!(id = %self.id, label = %self.label, code = ?status.code(), "process exited within grace period");
                return ProcessOutcome {
                    id: self.id,
                    label: self.label.clone(),
                    exit_status: Some(status),
                    was_killed: false,
                };
            }
            Ok(Err(e)) => {
                warn!(id = %self.id, label = %self.label, error = %e, "error waiting for process");
            }
            Err(_) => {
                debug!(id = %self.id, label = %self.label, "grace period expired, force killing");
            }
        }

        // Grace period expired — force kill.
        let _ = self.child.kill().await;
        let status = self.child.wait().await.ok();
        warn!(id = %self.id, label = %self.label, "process force-killed");
        ProcessOutcome {
            id: self.id,
            label: self.label.clone(),
            exit_status: status,
            was_killed: true,
        }
    }

    /// The cancellation token associated with this process.
    pub const fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
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
    cancel: CancelToken,
}

impl ProcessSupervisor {
    /// Create a new supervisor with a root cancellation token.
    pub fn new(cancel: CancelToken) -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            cancel,
        }
    }

    /// Spawn a new managed process.
    pub async fn spawn(&self, config: SpawnConfig) -> std::io::Result<ProcessId> {
        let id = ProcessId::next();
        let child_cancel = self.cancel.child();

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
            label: config.label,
            child,
            os_pid,
            grace_period: config.grace_period,
            cancel: child_cancel,
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

    #[test]
    fn process_id_uniqueness() {
        let a = ProcessId::next();
        let b = ProcessId::next();
        assert_ne!(a, b);
    }
}
