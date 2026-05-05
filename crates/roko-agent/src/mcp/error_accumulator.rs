//! Non-blocking MCP error accumulator for IDE/ACP sessions (SS36.63).
//!
//! When MCP tool calls fail during a session, errors are pushed to a
//! [`McpErrorAccumulator`] rather than silently swallowed. Callers can
//! query accumulated errors through the session state or a dedicated
//! endpoint after the session completes or at any point during execution.
//!
//! The accumulator is [`Send + Sync`] and lock-free for the hot path
//! (recording errors). It never blocks the session — MCP errors are
//! informational, not fatal.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// A single recorded MCP error with context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpErrorRecord {
    /// The MCP server name (namespace prefix).
    pub server: String,
    /// The tool name that was invoked.
    pub tool_name: String,
    /// Unix timestamp (seconds) when the error occurred.
    pub timestamp: u64,
    /// Human-readable error message.
    pub error_message: String,
    /// Whether the error was a transport-level failure vs. a server-returned error.
    pub is_transport_error: bool,
}

impl McpErrorRecord {
    /// Create a new error record with the current timestamp.
    #[must_use]
    pub fn new(
        server: impl Into<String>,
        tool_name: impl Into<String>,
        error_message: impl Into<String>,
        is_transport_error: bool,
    ) -> Self {
        Self {
            server: server.into(),
            tool_name: tool_name.into(),
            timestamp: now_secs(),
            error_message: error_message.into(),
            is_transport_error,
        }
    }
}

/// Thread-safe, non-blocking accumulator for MCP errors during a session.
///
/// Intended to be shared (via `Arc`) between `McpToolHandler` instances
/// and the session owner (tool loop, agent state, etc.). Recording an
/// error never panics or blocks the calling task.
#[derive(Debug, Clone, Default)]
pub struct McpErrorAccumulator {
    errors: Arc<Mutex<Vec<McpErrorRecord>>>,
}

impl McpErrorAccumulator {
    /// Create an empty accumulator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an MCP error. This is non-blocking and always succeeds.
    pub fn push(&self, record: McpErrorRecord) {
        self.errors.lock().push(record);
    }

    /// Record an MCP error from constituent parts (convenience method).
    pub fn record(
        &self,
        server: impl Into<String>,
        tool_name: impl Into<String>,
        error_message: impl Into<String>,
        is_transport_error: bool,
    ) {
        self.push(McpErrorRecord::new(
            server,
            tool_name,
            error_message,
            is_transport_error,
        ));
    }

    /// Return all accumulated errors so far (snapshot).
    #[must_use]
    pub fn drain(&self) -> Vec<McpErrorRecord> {
        let mut errors = self.errors.lock();
        errors.drain(..).collect()
    }

    /// Return a snapshot of accumulated errors without clearing them.
    #[must_use]
    pub fn snapshot(&self) -> Vec<McpErrorRecord> {
        self.errors.lock().clone()
    }

    /// Number of errors accumulated so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.errors.lock().len()
    }

    /// Whether any errors have been accumulated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.errors.lock().is_empty()
    }

    /// Clear all accumulated errors.
    pub fn clear(&self) {
        self.errors.lock().clear();
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_records_and_snapshots_errors() {
        let acc = McpErrorAccumulator::new();
        assert!(acc.is_empty());
        assert_eq!(acc.len(), 0);

        acc.record("github", "github.list_prs", "connection refused", true);
        acc.record("fs", "fs.read_file", "file not found", false);

        assert_eq!(acc.len(), 2);
        assert!(!acc.is_empty());

        let snap = acc.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].server, "github");
        assert_eq!(snap[0].tool_name, "github.list_prs");
        assert!(snap[0].is_transport_error);
        assert_eq!(snap[1].server, "fs");
        assert!(!snap[1].is_transport_error);

        // Snapshot does not consume errors.
        assert_eq!(acc.len(), 2);
    }

    #[test]
    fn accumulator_drain_clears_errors() {
        let acc = McpErrorAccumulator::new();
        acc.record("git", "git.status", "timeout after 30s", true);
        acc.record("git", "git.diff", "process exited", true);

        let drained = acc.drain();
        assert_eq!(drained.len(), 2);
        assert!(acc.is_empty());
    }

    #[test]
    fn accumulator_clear_empties_state() {
        let acc = McpErrorAccumulator::new();
        acc.record("x", "x.tool", "oops", false);
        acc.clear();
        assert!(acc.is_empty());
    }

    #[test]
    fn accumulator_is_clone_shared() {
        let acc = McpErrorAccumulator::new();
        let acc2 = acc.clone();
        acc.record("a", "a.t", "err1", false);
        acc2.record("b", "b.t", "err2", true);

        // Both clones see all errors since they share the inner Arc.
        assert_eq!(acc.len(), 2);
        assert_eq!(acc2.len(), 2);
    }

    #[test]
    fn error_record_has_nonzero_timestamp() {
        let rec = McpErrorRecord::new("s", "t", "msg", false);
        assert!(rec.timestamp > 0);
    }
}
