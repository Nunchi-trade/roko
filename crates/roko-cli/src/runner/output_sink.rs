//! Output sink abstraction for plan runner progress reporting.
//!
//! The [`RunOutputSink`] trait provides a uniform interface for receiving
//! structured progress events from the plan runner. Two implementations
//! are provided:
//!
//! - [`StderrSink`] — writes human-readable progress to stderr (for CLI use).
//! - [`NoopSink`] — discards all events (for testing / embedded use).

use std::fmt;

/// Coarse gate verdict passed to the output sink.
#[derive(Debug, Clone)]
pub struct GateResultSummary {
    pub rung: u32,
    pub passed: bool,
    pub gate_name: String,
    pub summary: String,
}

/// Token usage reported by agent runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
}

/// Structured progress events emitted by the plan runner.
///
/// Implementors receive callbacks as the runner progresses through tasks.
/// All methods have default no-op implementations so consumers can opt in
/// to only the events they care about.
pub trait RunOutputSink: Send + Sync {
    /// A task is about to be dispatched to an agent.
    fn task_started(&self, plan_id: &str, task_id: &str, description: &str) {
        let _ = (plan_id, task_id, description);
    }

    /// A line of output was received from the agent process.
    fn agent_line(&self, plan_id: &str, task_id: &str, line: &str) {
        let _ = (plan_id, task_id, line);
    }

    /// The agent invoked a tool.
    fn tool_call(&self, plan_id: &str, task_id: &str, tool_name: &str) {
        let _ = (plan_id, task_id, tool_name);
    }

    /// A tool produced output.
    fn tool_output(&self, plan_id: &str, task_id: &str, tool_name: &str, truncated_output: &str) {
        let _ = (plan_id, task_id, tool_name, truncated_output);
    }

    /// Token usage update from agent runtime.
    fn token_usage(&self, plan_id: &str, task_id: &str, usage: TokenUsage) {
        let _ = (plan_id, task_id, usage);
    }

    /// A gate rung completed with a verdict.
    fn gate_result(&self, plan_id: &str, task_id: &str, result: &GateResultSummary) {
        let _ = (plan_id, task_id, result);
    }

    /// A task completed successfully.
    fn task_completed(&self, plan_id: &str, task_id: &str, duration_ms: u64) {
        let _ = (plan_id, task_id, duration_ms);
    }

    /// A task failed.
    fn task_failed(&self, plan_id: &str, task_id: &str, error: &str) {
        let _ = (plan_id, task_id, error);
    }

    /// The entire plan run finished — summary statistics.
    fn plan_summary(
        &self,
        plan_id: &str,
        tasks_passed: usize,
        tasks_failed: usize,
        total_duration_ms: u64,
    ) {
        let _ = (plan_id, tasks_passed, tasks_failed, total_duration_ms);
    }
}

// ─── StderrSink ─────────────────────────────────────────────────────────────

/// Writes human-readable progress lines to stderr.
///
/// Intended for non-TUI CLI invocations where a simple log stream suffices.
pub struct StderrSink;

impl fmt::Debug for StderrSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StderrSink")
    }
}

impl RunOutputSink for StderrSink {
    fn task_started(&self, plan_id: &str, task_id: &str, description: &str) {
        eprintln!("[{plan_id}/{task_id}] started: {description}");
    }

    fn agent_line(&self, plan_id: &str, task_id: &str, line: &str) {
        eprintln!("[{plan_id}/{task_id}]   {line}");
    }

    fn tool_call(&self, plan_id: &str, task_id: &str, tool_name: &str) {
        eprintln!("[{plan_id}/{task_id}] tool: {tool_name}");
    }

    fn tool_output(&self, _plan_id: &str, _task_id: &str, _tool_name: &str, _output: &str) {
        // Tool output is typically too verbose for stderr; skip by default.
    }

    fn token_usage(&self, plan_id: &str, task_id: &str, usage: TokenUsage) {
        eprintln!(
            "[{plan_id}/{task_id}] tokens: in={} out={} cache={}",
            usage.input_tokens, usage.output_tokens, usage.cache_read_tokens
        );
    }

    fn gate_result(&self, plan_id: &str, task_id: &str, result: &GateResultSummary) {
        let status = if result.passed { "PASS" } else { "FAIL" };
        eprintln!(
            "[{plan_id}/{task_id}] gate rung {}: {} [{}] {}",
            result.rung, status, result.gate_name, result.summary
        );
    }

    fn task_completed(&self, plan_id: &str, task_id: &str, duration_ms: u64) {
        eprintln!("[{plan_id}/{task_id}] completed in {duration_ms}ms");
    }

    fn task_failed(&self, plan_id: &str, task_id: &str, error: &str) {
        eprintln!("[{plan_id}/{task_id}] FAILED: {error}");
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
}

// ─── NoopSink ───────────────────────────────────────────────────────────────

/// Discards all output events. Useful for testing or embedded/library usage
/// where no user-facing output is desired.
pub struct NoopSink;

impl fmt::Debug for NoopSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NoopSink")
    }
}

impl RunOutputSink for NoopSink {}
