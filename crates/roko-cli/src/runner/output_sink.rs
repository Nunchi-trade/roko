//! Output sink abstraction for plan runner progress reporting.
//!
//! The [`RunOutputSink`] trait provides a uniform interface for receiving
//! structured progress events from the plan runner. Two implementations
//! are provided:
//!
//! - [`StderrSink`] — writes rich inline progress to stderr (delegates to
//!   `RunnerInlineTerminal` internally, owns `AgentStreamBuffer` state).
//! - [`NoopSink`] — discards all events (for testing / embedded use).
//!
//! # Design decision (Task 006)
//!
//! `AgentStreamBuffer` is moved into `StderrSink` as internal `Mutex<String>`
//! state. The trait uses `&self` throughout so it can be wrapped in
//! `Arc<dyn RunOutputSink + Send + Sync>`. Interior mutability is handled
//! via `std::sync::Mutex` inside `StderrSink`.

use std::fmt;
use std::sync::Mutex;

use super::inline_output::RunnerInlineTerminal;
use crate::inline::DiffEntry;

/// Coarse gate verdict passed to the output sink.
#[derive(Debug, Clone)]
pub struct GateResultSummary {
    pub rung: u32,
    pub passed: bool,
    pub gate_name: String,
    pub summary: String,
    pub duration_ms: u64,
}

/// Token usage reported by agent runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Structured progress events emitted by the plan runner.
///
/// Implementors receive callbacks as the runner progresses through tasks.
/// All methods have default no-op implementations so consumers can opt in
/// to only the events they care about.
pub trait RunOutputSink: Send + Sync + fmt::Debug {
    // ─── Task lifecycle ─────────────────────────────────────────────────

    /// A task is about to be dispatched to an agent.
    fn task_started(
        &self,
        _plan_id: &str,
        _task_id: &str,
        _role: &str,
        _title: &str,
        _attempt: u32,
    ) {
    }

    /// A task completed successfully (with progress counts).
    fn task_completed(
        &self,
        _plan_id: &str,
        _task_id: &str,
        _completed: usize,
        _total: usize,
        _duration_ms: u64,
    ) {
    }

    /// A task failed permanently (retries exhausted or non-retryable).
    fn task_failed(&self, _plan_id: &str, _task_id: &str, _error: &str) {}

    // ─── Agent events ───────────────────────────────────────────────────

    /// Agent process started.
    fn agent_started(&self, _plan_id: &str, _task_id: &str, _provider: &str, _model: &str, _pid: Option<u32>) {}

    /// A text delta was received from the agent (buffered internally).
    fn agent_text_delta(&self, _plan_id: &str, _task_id: &str, _text: &str) {}

    /// Flush buffered agent text (called before structural events).
    fn flush_agent_text(&self, _plan_id: &str, _task_id: &str) {}

    /// The agent invoked a tool.
    fn tool_call(&self, _plan_id: &str, _task_id: &str, _tool_id: &str, _tool_name: &str) {}

    /// A tool produced output.
    fn tool_output(&self, _plan_id: &str, _task_id: &str, _tool_id: &str, _output: &str) {}

    /// Token usage update from agent runtime.
    fn token_usage(&self, _plan_id: &str, _task_id: &str, _usage: TokenUsage) {}

    /// Agent turn completed.
    fn agent_turn_completed(
        &self,
        _plan_id: &str,
        _task_id: &str,
        _total_cost_usd: Option<f64>,
        _is_error: bool,
        _model: &str,
        _total_input_tokens: u64,
        _total_output_tokens: u64,
    ) {
    }

    /// Agent reported an error.
    fn agent_error(&self, _plan_id: &str, _task_id: &str, _message: &str) {}

    // ─── Gate events ────────────────────────────────────────────────────

    /// A gate rung completed with a verdict.
    fn gate_result(&self, _plan_id: &str, _task_id: &str, _result: &GateResultSummary) {}

    /// Gate failed, retrying after backoff.
    fn gate_retry(&self, _plan_id: &str, _task_id: &str, _next_attempt: u32, _cooldown_ms: u64) {}

    // ─── Warm cache ─────────────────────────────────────────────────────

    /// Cargo cache warm started.
    fn warm_cache_started(&self) {}

    /// Cargo cache warm completed.
    fn warm_cache_completed(&self, _warm_ms: u64) {}

    // ─── Diff ───────────────────────────────────────────────────────────

    /// Show a diff block for task output.
    fn diff_block(&self, _plan_id: &str, _task_id: &str, _entries: &[DiffEntry]) {}

    // ─── Plan summary ───────────────────────────────────────────────────

    /// The entire plan run finished — summary statistics.
    fn plan_summary(
        &self,
        _plan_id: &str,
        _tasks_passed: usize,
        _tasks_failed: usize,
        _total_duration_ms: u64,
    ) {
    }

    /// A line of output was received from the agent process.
    /// Legacy compatibility method — prefer `agent_text_delta`.
    fn agent_line(&self, _plan_id: &str, _task_id: &str, _line: &str) {}
}

// ─── StderrSink ─────────────────────────────────────────────────────────────

/// Writes rich inline progress to stderr, delegating to `RunnerInlineTerminal`.
///
/// Owns an internal `AgentStreamBuffer` (as `Mutex<String>`) so that text
/// deltas can be accumulated and flushed at structural boundaries (tool calls,
/// turn completions) without exposing mutable state to callers.
pub struct StderrSink {
    inner: Mutex<RunnerInlineTerminal>,
    text_buf: Mutex<String>,
}

impl StderrSink {
    /// Create a new `StderrSink`. The internal `RunnerInlineTerminal` is
    /// always enabled.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RunnerInlineTerminal::new(true)),
            text_buf: Mutex::new(String::new()),
        }
    }

    /// Drain the last N non-empty lines from the text buffer, truncating
    /// each to `max_chars`. Returns collected lines.
    fn drain_lines(&self, max_lines: usize, max_chars: usize) -> Vec<String> {
        let mut buf = self.text_buf.lock().unwrap_or_else(|e| e.into_inner());
        if buf.trim().is_empty() {
            buf.clear();
            return Vec::new();
        }

        let lines: Vec<&str> = buf.lines().filter(|l| !l.trim().is_empty()).collect();
        let start = lines.len().saturating_sub(max_lines);
        let drained = lines[start..]
            .iter()
            .map(|line| truncate_chars(line.trim(), max_chars))
            .filter(|line| !line.is_empty())
            .collect();
        buf.clear();
        drained
    }
}

impl Default for StderrSink {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for StderrSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StderrSink")
    }
}

impl RunOutputSink for StderrSink {
    fn task_started(
        &self,
        _plan_id: &str,
        task_id: &str,
        role: &str,
        title: &str,
        attempt: u32,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.task_started(task_id, role, title, attempt);
    }

    fn task_completed(
        &self,
        _plan_id: &str,
        _task_id: &str,
        completed: usize,
        total: usize,
        duration_ms: u64,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.task_done(completed, total, duration_ms);
    }

    fn task_failed(&self, _plan_id: &str, _task_id: &str, error: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.task_failed(error);
    }

    fn agent_started(
        &self,
        _plan_id: &str,
        _task_id: &str,
        provider: &str,
        model: &str,
        pid: Option<u32>,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.agent_started(provider, model, pid);
    }

    fn agent_text_delta(&self, _plan_id: &str, _task_id: &str, text: &str) {
        let mut buf = self.text_buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.push_str(text);
    }

    fn flush_agent_text(&self, _plan_id: &str, _task_id: &str) {
        let lines = self.drain_lines(3, 120);
        if !lines.is_empty() {
            let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            inner.agent_text(lines);
        }
    }

    fn tool_call(&self, _plan_id: &str, _task_id: &str, tool_id: &str, tool_name: &str) {
        // Flush buffered text before showing tool call.
        let lines = self.drain_lines(3, 120);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.agent_text(lines);
        inner.tool_call_started(tool_id, tool_name);
    }

    fn tool_output(&self, _plan_id: &str, _task_id: &str, tool_id: &str, output: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.tool_output(tool_id, output);
    }

    fn token_usage(&self, _plan_id: &str, _task_id: &str, usage: TokenUsage) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // The model field is not available here; use empty string — the
        // inline terminal accumulates per-model stats via the full
        // `agent_turn_completed` path.
        inner.token_usage(
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens,
            "",
        );
    }

    fn agent_turn_completed(
        &self,
        _plan_id: &str,
        _task_id: &str,
        total_cost_usd: Option<f64>,
        is_error: bool,
        model: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
    ) {
        // Flush any remaining buffered text.
        let lines = self.drain_lines(3, 120);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.agent_text(lines);
        inner.agent_turn_completed(total_cost_usd, is_error, model, total_input_tokens, total_output_tokens);
    }

    fn agent_error(&self, _plan_id: &str, _task_id: &str, message: &str) {
        // Flush any remaining buffered text.
        let lines = self.drain_lines(3, 120);
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.agent_text(lines);
        inner.agent_error(message);
    }

    fn gate_result(&self, _plan_id: &str, _task_id: &str, result: &GateResultSummary) {
        use super::types::{GateCompletion, GateCompletionKind, GateVerdictSummary};
        let completion = GateCompletion {
            kind: GateCompletionKind::Gate,
            plan_id: String::new(),
            task_id: String::new(),
            rung: result.rung,
            passed: result.passed,
            failure_kind: None,
            verdicts: vec![GateVerdictSummary {
                gate_name: result.gate_name.clone(),
                passed: result.passed,
                summary: result.summary.clone(),
                error_digest: None,
                failure_kind: None,
            }],
            output: String::new(),
            duration_ms: result.duration_ms,
        };
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.gate_completed(&completion);
    }

    fn gate_retry(&self, _plan_id: &str, _task_id: &str, next_attempt: u32, cooldown_ms: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.gate_retry(next_attempt, cooldown_ms);
    }

    fn warm_cache_started(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.warm_cache_started();
    }

    fn warm_cache_completed(&self, warm_ms: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.warm_cache_completed(warm_ms);
    }

    fn diff_block(&self, _plan_id: &str, _task_id: &str, entries: &[DiffEntry]) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.diff_block(entries);
    }

    fn plan_summary(
        &self,
        plan_id: &str,
        tasks_passed: usize,
        tasks_failed: usize,
        total_duration_ms: u64,
    ) {
        eprintln!(
            "[{plan_id}] summary: {tasks_passed} passed, {tasks_failed} failed, \
             total {total_duration_ms}ms"
        );
    }

    fn agent_line(&self, plan_id: &str, task_id: &str, line: &str) {
        eprintln!("[{plan_id}/{task_id}]   {line}");
    }
}

// ─── NoopSink ───────────────────────────────────────────────────────────────

/// Discards all output events. Useful for testing or embedded/library usage
/// where no user-facing output is desired.
pub struct NoopSink;

impl Default for NoopSink {
    fn default() -> Self {
        Self
    }
}

impl fmt::Debug for NoopSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoopSink")
    }
}

impl RunOutputSink for NoopSink {}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn noop_sink_in_arc_does_not_panic() {
        let sink: Arc<dyn RunOutputSink> = Arc::new(NoopSink);
        sink.task_started("plan-1", "task-1", "implementer", "Build feature X", 1);
        sink.agent_text_delta("plan-1", "task-1", "hello ");
        sink.agent_text_delta("plan-1", "task-1", "world\n");
        sink.flush_agent_text("plan-1", "task-1");
        sink.tool_call("plan-1", "task-1", "tc-1", "Read");
        sink.tool_output("plan-1", "task-1", "tc-1", "file contents...");
        sink.token_usage("plan-1", "task-1", TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            cache_write_tokens: 5,
        });
        sink.agent_turn_completed("plan-1", "task-1", Some(0.01), false, "claude-sonnet-4-6", 100, 50);
        sink.gate_result("plan-1", "task-1", &GateResultSummary {
            rung: 0,
            passed: true,
            gate_name: "compile".to_string(),
            summary: "ok".to_string(),
            duration_ms: 1200,
        });
        sink.gate_retry("plan-1", "task-1", 2, 5000);
        sink.warm_cache_started();
        sink.warm_cache_completed(1500);
        sink.diff_block("plan-1", "task-1", &[]);
        sink.task_completed("plan-1", "task-1", 3, 10, 5000);
        sink.task_failed("plan-1", "task-1", "compile error");
        sink.plan_summary("plan-1", 9, 1, 60000);
    }

    #[test]
    fn stderr_sink_can_be_constructed() {
        let sink = StderrSink::new();
        // Just verify it doesn't panic on construction.
        assert_eq!(format!("{sink:?}"), "StderrSink");
    }
}
