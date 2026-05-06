//! Output sink abstraction for plan runner progress reporting.
//!
//! The [`RunOutputSink`] trait provides a uniform interface for receiving
//! structured progress events from the plan runner. Implementations:
//!
//! - [`StderrSink`] — writes rich inline progress to stderr (delegates to
//!   `RunnerInlineTerminal` internally, owns `AgentStreamBuffer` state).
//! - [`FormattedStderrSink`] — writes structured `[plan/task]` prefixed
//!   output to stderr with color support, agent output truncation, and
//!   progress indicators. This is the default for `roko do` / `roko plan run`.
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
    fn agent_started(
        &self,
        _plan_id: &str,
        _task_id: &str,
        _provider: &str,
        _model: &str,
        _pid: Option<u32>,
    ) {
    }

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
    fn task_started(&self, _plan_id: &str, task_id: &str, role: &str, title: &str, attempt: u32) {
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
        inner.agent_turn_completed(
            total_cost_usd,
            is_error,
            model,
            total_input_tokens,
            total_output_tokens,
        );
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

// ─── FormattedStderrSink ────────────────────────────────────────────────────

/// Maximum number of agent output lines before truncation kicks in.
const AGENT_OUTPUT_TRUNCATE_THRESHOLD: usize = 50;
/// Number of head lines to show when truncating.
const AGENT_OUTPUT_HEAD_LINES: usize = 10;
/// Number of tail lines to show when truncating.
const AGENT_OUTPUT_TAIL_LINES: usize = 10;

/// Writes structured `[plan/task]` prefixed progress to stderr with
/// optional ANSI color, agent output truncation, and progress indicators.
///
/// This is the default output sink for `roko do` and `roko plan run`.
/// It formats output as:
/// ```text
/// [plan-id/task-id] > Agent starting: "Add rate limiting middleware"
/// [plan-id/task-id] | Writing crates/roko-serve/src/middleware/rate_limit.rs
/// [plan-id/task-id] > Running gate: compile
/// [plan-id/task-id] + Gate passed: compile (2.3s)
/// [plan-id/task-id] x Gate failed: test -- 2 test failures
/// ```
///
/// Agent code output longer than 50 lines is truncated: first 10, last 10,
/// with an omission notice in between.
pub struct FormattedStderrSink {
    color: bool,
    text_buf: Mutex<String>,
}

impl FormattedStderrSink {
    /// Create a new sink. When `color` is true, ANSI escape codes are emitted.
    ///
    /// Callers should resolve color from CLI flags + `NO_COLOR` + `CLICOLOR`
    /// before constructing the sink.
    pub fn new(color: bool) -> Self {
        Self {
            color,
            text_buf: Mutex::new(String::new()),
        }
    }

    /// Write a formatted line to stderr.
    fn emit(&self, line: &str) {
        let mut stderr = std::io::stderr().lock();
        use std::io::Write;
        let _ = writeln!(stderr, "{line}");
    }

    /// Format a `[plan/task]` prefix.
    fn prefix(plan_id: &str, task_id: &str) -> String {
        if plan_id.is_empty() && task_id.is_empty() {
            String::new()
        } else if plan_id.is_empty() {
            format!("[{task_id}]")
        } else if task_id.is_empty() {
            format!("[{plan_id}]")
        } else {
            format!("[{plan_id}/{task_id}]")
        }
    }

    /// Drain and format accumulated agent text with truncation.
    fn drain_text(&self, plan_id: &str, task_id: &str) {
        let text = {
            let mut buf = self.text_buf.lock().unwrap_or_else(|e| e.into_inner());
            let t = buf.clone();
            buf.clear();
            t
        };

        if text.trim().is_empty() {
            return;
        }

        let pfx = Self::prefix(plan_id, task_id);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let formatted = format_truncated_lines(&lines);

        for line in &formatted {
            let trimmed = truncate_chars(line.trim(), 120);
            if self.color {
                self.emit(&format!("{pfx} \x1b[2m| {trimmed}\x1b[0m"));
            } else {
                self.emit(&format!("{pfx} | {trimmed}"));
            }
        }
    }

    /// Emit a structural event line with an icon.
    fn emit_event(&self, plan_id: &str, task_id: &str, icon: &str, msg: &str) {
        let pfx = Self::prefix(plan_id, task_id);
        self.emit(&format!("{pfx} {icon} {msg}"));
    }

    /// Emit a success line (green when color is on).
    fn emit_pass(&self, plan_id: &str, task_id: &str, msg: &str) {
        let pfx = Self::prefix(plan_id, task_id);
        if self.color {
            self.emit(&format!("{pfx} \x1b[32m+ {msg}\x1b[0m"));
        } else {
            self.emit(&format!("{pfx} + {msg}"));
        }
    }

    /// Emit a failure line (red when color is on).
    fn emit_fail(&self, plan_id: &str, task_id: &str, msg: &str) {
        let pfx = Self::prefix(plan_id, task_id);
        if self.color {
            self.emit(&format!("{pfx} \x1b[31mx {msg}\x1b[0m"));
        } else {
            self.emit(&format!("{pfx} x {msg}"));
        }
    }

    /// Emit an in-progress line (yellow when color is on).
    fn emit_progress(&self, plan_id: &str, task_id: &str, msg: &str) {
        let pfx = Self::prefix(plan_id, task_id);
        if self.color {
            self.emit(&format!("{pfx} \x1b[33m> {msg}\x1b[0m"));
        } else {
            self.emit(&format!("{pfx} > {msg}"));
        }
    }
}

impl Default for FormattedStderrSink {
    fn default() -> Self {
        Self::new(false)
    }
}

impl fmt::Debug for FormattedStderrSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("FormattedStderrSink")
    }
}

impl RunOutputSink for FormattedStderrSink {
    fn task_started(
        &self,
        plan_id: &str,
        task_id: &str,
        role: &str,
        title: &str,
        attempt: u32,
    ) {
        let attempt_str = if attempt > 1 {
            format!(" (attempt {attempt})")
        } else {
            String::new()
        };
        let title_trunc = truncate_chars(title, 100);
        self.emit_progress(
            plan_id,
            task_id,
            &format!("Agent starting{attempt_str}: \"{title_trunc}\" [{role}]"),
        );
    }

    fn task_completed(
        &self,
        plan_id: &str,
        task_id: &str,
        completed: usize,
        total: usize,
        duration_ms: u64,
    ) {
        self.drain_text(plan_id, task_id);
        let secs = duration_ms as f64 / 1000.0;
        self.emit_pass(
            plan_id,
            task_id,
            &format!("Task completed ({completed}/{total}) in {secs:.1}s"),
        );
    }

    fn task_failed(&self, plan_id: &str, task_id: &str, error: &str) {
        self.drain_text(plan_id, task_id);
        let first_line = error.lines().next().unwrap_or("unknown error");
        let err_trunc = truncate_chars(first_line, 120);
        self.emit_fail(plan_id, task_id, &format!("Task failed: {err_trunc}"));
    }

    fn agent_started(
        &self,
        plan_id: &str,
        task_id: &str,
        provider: &str,
        model: &str,
        pid: Option<u32>,
    ) {
        let pid_str = pid.map(|p| format!(" pid {p}")).unwrap_or_default();
        self.emit_event(
            plan_id,
            task_id,
            ">",
            &format!("Agent: {model} ({provider}{pid_str})"),
        );
    }

    fn agent_text_delta(&self, _plan_id: &str, _task_id: &str, text: &str) {
        let mut buf = self.text_buf.lock().unwrap_or_else(|e| e.into_inner());
        buf.push_str(text);
    }

    fn flush_agent_text(&self, plan_id: &str, task_id: &str) {
        self.drain_text(plan_id, task_id);
    }

    fn tool_call(&self, plan_id: &str, task_id: &str, _tool_id: &str, tool_name: &str) {
        self.drain_text(plan_id, task_id);
        self.emit_event(plan_id, task_id, ">", &format!("Tool: {tool_name}"));
    }

    fn tool_output(&self, plan_id: &str, task_id: &str, _tool_id: &str, output: &str) {
        let first = output.lines().next().unwrap_or("").trim();
        if !first.is_empty() {
            let pfx = Self::prefix(plan_id, task_id);
            let preview = truncate_chars(first, 80);
            if self.color {
                self.emit(&format!("{pfx} \x1b[2m    {preview}\x1b[0m"));
            } else {
                self.emit(&format!("{pfx}     {preview}"));
            }
        }
    }

    fn token_usage(&self, plan_id: &str, task_id: &str, usage: TokenUsage) {
        let total = usage.input_tokens + usage.output_tokens;
        if total > 0 {
            self.emit_event(
                plan_id,
                task_id,
                " ",
                &format!(
                    "Tokens: {} in / {} out (cache: {} read, {} write)",
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_tokens,
                    usage.cache_write_tokens
                ),
            );
        }
    }

    fn agent_turn_completed(
        &self,
        plan_id: &str,
        task_id: &str,
        total_cost_usd: Option<f64>,
        is_error: bool,
        model: &str,
        total_input_tokens: u64,
        total_output_tokens: u64,
    ) {
        self.drain_text(plan_id, task_id);
        let cost = total_cost_usd
            .map(|c| format!("${c:.4}"))
            .unwrap_or_else(|| String::from("$?.????"));
        let summary = format!(
            "Agent turn {}: {cost} | {} in / {} out | {model}",
            if is_error { "failed" } else { "complete" },
            total_input_tokens,
            total_output_tokens,
        );
        if is_error {
            self.emit_fail(plan_id, task_id, &summary);
        } else {
            self.emit_pass(plan_id, task_id, &summary);
        }
    }

    fn agent_error(&self, plan_id: &str, task_id: &str, message: &str) {
        self.drain_text(plan_id, task_id);
        let msg = truncate_chars(message, 120);
        self.emit_fail(plan_id, task_id, &format!("Agent error: {msg}"));
    }

    fn gate_result(&self, plan_id: &str, task_id: &str, result: &GateResultSummary) {
        let secs = result.duration_ms as f64 / 1000.0;
        if result.passed {
            self.emit_pass(
                plan_id,
                task_id,
                &format!(
                    "Gate passed: {} ({secs:.1}s)",
                    result.gate_name
                ),
            );
        } else {
            let summary_trunc = truncate_chars(&result.summary, 80);
            self.emit_fail(
                plan_id,
                task_id,
                &format!(
                    "Gate failed: {} ({secs:.1}s) -- {summary_trunc}",
                    result.gate_name
                ),
            );
        }
    }

    fn gate_retry(
        &self,
        plan_id: &str,
        task_id: &str,
        next_attempt: u32,
        cooldown_ms: u64,
    ) {
        let secs = cooldown_ms as f64 / 1000.0;
        self.emit_progress(
            plan_id,
            task_id,
            &format!("Gate retry: attempt {next_attempt}, backoff {secs:.1}s"),
        );
    }

    fn warm_cache_started(&self) {
        self.emit_progress("", "", "Warming cargo cache...");
    }

    fn warm_cache_completed(&self, warm_ms: u64) {
        let secs = warm_ms as f64 / 1000.0;
        self.emit_pass("", "", &format!("Cargo cache warm ({secs:.1}s)"));
    }

    fn diff_block(&self, plan_id: &str, task_id: &str, entries: &[DiffEntry]) {
        if entries.is_empty() {
            return;
        }
        let pfx = Self::prefix(plan_id, task_id);
        for entry in entries {
            let path = &entry.path;
            let adds = entry.additions;
            let dels = entry.deletions;
            if self.color {
                self.emit(&format!(
                    "{pfx} \x1b[2m  {path} \x1b[32m+{adds}\x1b[0m\x1b[2m/\x1b[31m-{dels}\x1b[0m"
                ));
            } else {
                self.emit(&format!("{pfx}   {path} +{adds}/-{dels}"));
            }
        }
    }

    fn plan_summary(
        &self,
        plan_id: &str,
        tasks_passed: usize,
        tasks_failed: usize,
        total_duration_ms: u64,
    ) {
        let secs = total_duration_ms as f64 / 1000.0;
        let total = tasks_passed + tasks_failed;
        if tasks_failed == 0 {
            self.emit_pass(
                plan_id,
                "",
                &format!("Plan complete: {tasks_passed}/{total} passed in {secs:.1}s"),
            );
        } else {
            self.emit_fail(
                plan_id,
                "",
                &format!(
                    "Plan finished: {tasks_passed} passed, {tasks_failed} failed ({secs:.1}s)"
                ),
            );
        }
    }

    fn agent_line(&self, plan_id: &str, task_id: &str, line: &str) {
        let pfx = Self::prefix(plan_id, task_id);
        if self.color {
            self.emit(&format!("{pfx} \x1b[2m| {line}\x1b[0m"));
        } else {
            self.emit(&format!("{pfx} | {line}"));
        }
    }
}

// ─── Shared formatting ─────────────────────────────────────────────────────

/// Format a `DashboardEvent` (from SSE or local state hub) into a single
/// human-readable line suitable for terminal output.
///
/// Returns `None` for events that are not interesting for CLI streaming
/// (e.g. bulk data refreshes like `MarketplaceJobsUpdated`).
///
/// This function is used by both `FormattedStderrSink` (for local events
/// mapped to DashboardEvent) and the SSE client (for remote events).
pub fn format_dashboard_event(
    event: &roko_core::dashboard_snapshot::DashboardEvent,
    color: bool,
) -> Option<String> {
    use roko_core::dashboard_snapshot::DashboardEvent;

    let (pfx, icon, msg) = match event {
        DashboardEvent::PlanStarted { plan_id } => {
            (format!("[{plan_id}]"), ">", format!("Plan started"))
        }
        DashboardEvent::PlanCompleted { plan_id, success } => {
            let outcome = if *success { "completed" } else { "failed" };
            let icon = if *success { "+" } else { "x" };
            (format!("[{plan_id}]"), icon, format!("Plan {outcome}"))
        }
        DashboardEvent::TaskStarted {
            plan_id,
            task_id,
            title,
            phase,
        } => {
            let t = if title.is_empty() {
                String::new()
            } else {
                format!(": \"{title}\"")
            };
            (
                format!("[{plan_id}/{task_id}]"),
                ">",
                format!("Task started{t} [{phase}]"),
            )
        }
        DashboardEvent::TaskCompleted {
            plan_id,
            task_id,
            outcome,
        } => {
            let icon = if outcome == "pass" || outcome == "success" {
                "+"
            } else {
                "x"
            };
            (
                format!("[{plan_id}/{task_id}]"),
                icon,
                format!("Task {outcome}"),
            )
        }
        DashboardEvent::TaskPhaseChanged {
            plan_id,
            task_id,
            old_phase,
            new_phase,
        } => (
            format!("[{plan_id}/{task_id}]"),
            ">",
            format!("Phase: {old_phase} -> {new_phase}"),
        ),
        DashboardEvent::AgentSpawned {
            agent_id,
            role,
            model,
        } => (
            format!("[{agent_id}]"),
            ">",
            format!("Agent spawned: {role} ({model})"),
        ),
        DashboardEvent::AgentOutput { agent_id, content } => {
            let preview = content.lines().next().unwrap_or("").trim();
            if preview.is_empty() {
                return None;
            }
            let trunc = truncate_chars(preview, 100);
            (format!("[{agent_id}]"), "|", trunc)
        }
        DashboardEvent::AgentCompleted { agent_id } => (
            format!("[{agent_id}]"),
            "+",
            format!("Agent completed"),
        ),
        DashboardEvent::GateResult {
            plan_id,
            task_id,
            gate,
            passed,
        } => {
            let (icon, word) = if *passed { ("+", "passed") } else { ("x", "failed") };
            (
                format!("[{plan_id}/{task_id}]"),
                icon,
                format!("Gate {word}: {gate}"),
            )
        }
        DashboardEvent::PhaseTransition {
            plan_id,
            from,
            to,
        } => (
            format!("[{plan_id}]"),
            ">",
            format!("Phase: {from} -> {to}"),
        ),
        DashboardEvent::Error { message } => {
            let trunc = truncate_chars(message, 120);
            (String::new(), "x", format!("Error: {trunc}"))
        }
        DashboardEvent::EventLogEntry {
            event_type,
            plan_id,
            task_id,
            message,
            ..
        } => {
            let scope = if plan_id.is_empty() && task_id.is_empty() {
                String::new()
            } else if task_id.is_empty() {
                format!("[{plan_id}]")
            } else {
                format!("[{plan_id}/{task_id}]")
            };
            (scope, ">", format!("{event_type}: {message}"))
        }
        DashboardEvent::JobExecutionStarted {
            job_id,
            job_type,
            agent_id,
        } => (
            format!("[{job_id}]"),
            ">",
            format!("Job started: {job_type} (agent {agent_id})"),
        ),
        DashboardEvent::JobProgress {
            job_id,
            percent,
            message,
        } => (
            format!("[{job_id}]"),
            ">",
            format!("Job {percent}%: {message}"),
        ),
        // Bulk data refresh events are not useful for CLI streaming.
        DashboardEvent::EfficiencyEvent { .. }
        | DashboardEvent::Diagnosis { .. }
        | DashboardEvent::ExperimentWinnersUpdated { .. }
        | DashboardEvent::CFactorTrendUpdated { .. }
        | DashboardEvent::EpisodeRecorded { .. }
        | DashboardEvent::TaskOutputAppended { .. }
        | DashboardEvent::CascadeRouterUpdated { .. }
        | DashboardEvent::GateThresholdsUpdated { .. }
        | DashboardEvent::MarketplaceJobsUpdated { .. }
        | DashboardEvent::AtelierPrdsUpdated { .. }
        | DashboardEvent::KnowledgeEntriesUpdated { .. }
        | DashboardEvent::EfficiencyTrendUpdated { .. }
        | DashboardEvent::IsfrRateComputed { .. }
        | DashboardEvent::IsfrSourceHealthChanged { .. }
        | DashboardEvent::IsfrKeeperStateChanged { .. }
        | DashboardEvent::FeedTick { .. }
        | DashboardEvent::FeedAgentOnline { .. }
        | DashboardEvent::FeedAgentOffline { .. }
        | DashboardEvent::ChainBlock { .. }
        | DashboardEvent::ChainTx { .. }
        | DashboardEvent::ChainContractEvent { .. } => return None,
    };

    let line = if pfx.is_empty() {
        format!("{icon} {msg}")
    } else {
        format!("{pfx} {icon} {msg}")
    };

    if color {
        // Colorize by icon type.
        let colored = match icon {
            "+" => format!("\x1b[32m{line}\x1b[0m"),
            "x" => format!("\x1b[31m{line}\x1b[0m"),
            ">" => format!("\x1b[33m{line}\x1b[0m"),
            "|" => format!("\x1b[2m{line}\x1b[0m"),
            _ => line,
        };
        Some(colored)
    } else {
        Some(line)
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Apply the 50-line truncation rule: if lines exceed the threshold,
/// show the first `AGENT_OUTPUT_HEAD_LINES`, an omission notice, and
/// the last `AGENT_OUTPUT_TAIL_LINES`.
pub(crate) fn format_truncated_lines(lines: &[&str]) -> Vec<String> {
    if lines.len() <= AGENT_OUTPUT_TRUNCATE_THRESHOLD {
        return lines.iter().map(|s| (*s).to_string()).collect();
    }

    let omitted = lines.len() - AGENT_OUTPUT_HEAD_LINES - AGENT_OUTPUT_TAIL_LINES;
    let mut result: Vec<String> = lines[..AGENT_OUTPUT_HEAD_LINES]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    result.push(format!("... ({omitted} lines omitted)"));
    result.extend(
        lines[lines.len() - AGENT_OUTPUT_TAIL_LINES..]
            .iter()
            .map(|s| (*s).to_string()),
    );
    result
}

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
        sink.token_usage(
            "plan-1",
            "task-1",
            TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
                cache_write_tokens: 5,
            },
        );
        sink.agent_turn_completed(
            "plan-1",
            "task-1",
            Some(0.01),
            false,
            "claude-sonnet-4-6",
            100,
            50,
        );
        sink.gate_result(
            "plan-1",
            "task-1",
            &GateResultSummary {
                rung: 0,
                passed: true,
                gate_name: "compile".to_string(),
                summary: "ok".to_string(),
                duration_ms: 1200,
            },
        );
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

    // ─── FormattedStderrSink tests ───────────────────────────────────

    #[test]
    fn formatted_sink_in_arc_does_not_panic() {
        let sink: Arc<dyn RunOutputSink> = Arc::new(FormattedStderrSink::new(false));
        sink.task_started("plan-1", "task-1", "implementer", "Build feature X", 1);
        sink.agent_started("plan-1", "task-1", "claude", "claude-sonnet-4-6", Some(1234));
        sink.agent_text_delta("plan-1", "task-1", "hello ");
        sink.agent_text_delta("plan-1", "task-1", "world\n");
        sink.flush_agent_text("plan-1", "task-1");
        sink.tool_call("plan-1", "task-1", "tc-1", "Read");
        sink.tool_output("plan-1", "task-1", "tc-1", "file contents...");
        sink.token_usage(
            "plan-1",
            "task-1",
            TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_tokens: 10,
                cache_write_tokens: 5,
            },
        );
        sink.agent_turn_completed("plan-1", "task-1", Some(0.01), false, "sonnet", 100, 50);
        sink.agent_error("plan-1", "task-1", "test error");
        sink.gate_result(
            "plan-1",
            "task-1",
            &GateResultSummary {
                rung: 0,
                passed: true,
                gate_name: "compile".to_string(),
                summary: "ok".to_string(),
                duration_ms: 1200,
            },
        );
        sink.gate_result(
            "plan-1",
            "task-1",
            &GateResultSummary {
                rung: 1,
                passed: false,
                gate_name: "test".to_string(),
                summary: "2 test failures".to_string(),
                duration_ms: 3400,
            },
        );
        sink.gate_retry("plan-1", "task-1", 2, 5000);
        sink.warm_cache_started();
        sink.warm_cache_completed(1500);
        sink.diff_block("plan-1", "task-1", &[]);
        sink.task_completed("plan-1", "task-1", 3, 10, 5000);
        sink.task_failed("plan-1", "task-1", "compile error");
        sink.plan_summary("plan-1", 9, 1, 60000);
    }

    #[test]
    fn formatted_sink_debug_name() {
        let sink = FormattedStderrSink::new(false);
        assert_eq!(format!("{sink:?}"), "FormattedStderrSink");
    }

    #[test]
    fn formatted_sink_prefix_formats() {
        assert_eq!(FormattedStderrSink::prefix("p", "t"), "[p/t]");
        assert_eq!(FormattedStderrSink::prefix("p", ""), "[p]");
        assert_eq!(FormattedStderrSink::prefix("", "t"), "[t]");
        assert_eq!(FormattedStderrSink::prefix("", ""), "");
    }

    // ─── Truncation tests ────────────────────────────────────────────

    #[test]
    fn truncation_short_input_unchanged() {
        let lines: Vec<&str> = (0..10).map(|i| match i {
            0 => "line-0",
            1 => "line-1",
            2 => "line-2",
            3 => "line-3",
            4 => "line-4",
            5 => "line-5",
            6 => "line-6",
            7 => "line-7",
            8 => "line-8",
            _ => "line-9",
        }).collect();
        let result = format_truncated_lines(&lines);
        assert_eq!(result.len(), 10);
        assert_eq!(result[0], "line-0");
        assert_eq!(result[9], "line-9");
    }

    #[test]
    fn truncation_at_threshold_unchanged() {
        let strs: Vec<String> = (0..50).map(|i| format!("line-{i}")).collect();
        let lines: Vec<&str> = strs.iter().map(|s| s.as_str()).collect();
        let result = format_truncated_lines(&lines);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn truncation_over_threshold_truncates() {
        let strs: Vec<String> = (0..60).map(|i| format!("line-{i}")).collect();
        let lines: Vec<&str> = strs.iter().map(|s| s.as_str()).collect();
        let result = format_truncated_lines(&lines);
        // 10 head + 1 omission + 10 tail = 21
        assert_eq!(result.len(), 21);
        assert_eq!(result[0], "line-0");
        assert_eq!(result[9], "line-9");
        assert!(result[10].contains("40 lines omitted"));
        assert_eq!(result[11], "line-50");
        assert_eq!(result[20], "line-59");
    }

    // ─── format_dashboard_event tests ────────────────────────────────

    #[test]
    fn format_event_plan_started() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::PlanStarted {
            plan_id: "my-plan".to_string(),
        };
        let line = format_dashboard_event(&event, false).unwrap();
        assert!(line.contains("[my-plan]"));
        assert!(line.contains("Plan started"));
    }

    #[test]
    fn format_event_gate_result_pass() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::GateResult {
            plan_id: "p".to_string(),
            task_id: "t".to_string(),
            gate: "compile".to_string(),
            passed: true,
        };
        let line = format_dashboard_event(&event, false).unwrap();
        assert!(line.contains("[p/t]"));
        assert!(line.contains("Gate passed: compile"));
    }

    #[test]
    fn format_event_gate_result_fail() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::GateResult {
            plan_id: "p".to_string(),
            task_id: "t".to_string(),
            gate: "test".to_string(),
            passed: false,
        };
        let line = format_dashboard_event(&event, false).unwrap();
        assert!(line.contains("Gate failed: test"));
    }

    #[test]
    fn format_event_color_has_ansi() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::PlanCompleted {
            plan_id: "p".to_string(),
            success: true,
        };
        let colored = format_dashboard_event(&event, true).unwrap();
        assert!(colored.contains("\x1b[32m"), "expected green ANSI");
        let plain = format_dashboard_event(&event, false).unwrap();
        assert!(!plain.contains("\x1b["), "expected no ANSI escapes");
    }

    #[test]
    fn format_event_skips_bulk_events() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::CascadeRouterUpdated {
            snapshot_json: "{}".to_string(),
        };
        assert!(format_dashboard_event(&event, false).is_none());
    }

    #[test]
    fn format_event_error() {
        use roko_core::dashboard_snapshot::DashboardEvent;
        let event = DashboardEvent::Error {
            message: "something broke".to_string(),
        };
        let line = format_dashboard_event(&event, false).unwrap();
        assert!(line.contains("Error: something broke"));
    }
}
