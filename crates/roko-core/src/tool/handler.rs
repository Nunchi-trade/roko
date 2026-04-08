//! Tool handler trait ([`ToolHandler`]) and runtime context
//! ([`ToolContext`]) plus the small trait-object abstractions the
//! context depends on ([`AuditSink`], [`CancelToken`]).
//!
//! Handlers are async; the dispatcher is free to run peers concurrently
//! (§36.41) or serially depending on
//! [`ToolConcurrency`](crate::tool::ToolConcurrency).
//!
//! `AuditSink` / `CancelToken` are deliberately trait-objects rather
//! than concrete tokio types so `roko-core` stays runtime-agnostic. The
//! real wiring lives in `roko-agent`'s dispatcher: tokio `mpsc::Sender`
//! and `tokio_util::sync::CancellationToken` just implement these
//! traits.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::call::{ToolCall, ToolResult};
use super::def::ToolPermission;
use super::metrics::{MetricsSink, NoopMetricsSink};
use super::trace::{NoopTraceSink, TraceSink};
use crate::Signal;

// ─── AuditSink ────────────────────────────────────────────────────────────

/// Sink for audit signals emitted during tool execution.
///
/// Every executed [`ToolCall`] should produce at least one
/// `Signal<Kind::ToolInvocation>` (§36.44) on this sink. Implementations
/// may fan out, buffer, or drop; they must not block the caller.
pub trait AuditSink: Send + Sync {
    /// Publish a signal. Must not block; impls buffer if downstream is slow.
    fn emit(&self, signal: Signal);
}

/// No-op [`AuditSink`] — drops every signal. Used in tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn emit(&self, _signal: Signal) {}
}

// ─── CancelToken ──────────────────────────────────────────────────────────

/// Cancellation signal propagated to long-running tool handlers.
///
/// The conductor (or a user-initiated abort) toggles the token;
/// well-behaved handlers poll [`Self::is_cancelled`] at loop boundaries
/// or check it in futures that support cooperative cancellation.
pub trait CancelToken: Send + Sync {
    /// Returns `true` once cancellation has been requested.
    fn is_cancelled(&self) -> bool;
}

/// Token that never fires. Used in tests and for "no cancellation" contexts.
#[derive(Debug, Clone, Copy, Default)]
pub struct NeverCancel;

impl CancelToken for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Token backed by an atomic bool — flip once, stay tripped.
///
/// Useful for single-call orchestration where an owner decides to abort
/// after dispatching. Clone via `Arc` to share state across threads.
#[derive(Debug, Default)]
pub struct AtomicCancel {
    flag: std::sync::atomic::AtomicBool,
}

impl AtomicCancel {
    /// Construct a fresh, un-tripped token.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            flag: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Trip the token. Subsequent [`Self::is_cancelled`] calls return true.
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl CancelToken for AtomicCancel {
    fn is_cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// ─── ToolContext ──────────────────────────────────────────────────────────

/// Runtime context threaded through every [`ToolHandler::execute`] call.
///
/// The dispatcher constructs one [`ToolContext`] per call, populating it
/// from the active worktree, the role's granted permissions, and the
/// conductor's audit/cancel/trace/metrics machinery.
///
/// # Why sinks are trait objects
///
/// `audit_sink`, `trace_sink`, and `metrics_sink` are `Arc<dyn _>` trait
/// objects so `roko-core` remains runtime-agnostic — the real tokio-backed
/// sinks live in `roko-agent` / `roko-std` / `roko-fs`. Tests use the
/// `Noop*` variants from this crate.
pub struct ToolContext {
    /// Root of the worktree this tool call may touch.
    ///
    /// Safety (§36.46) canonicalizes every path argument under this
    /// root; writes outside it produce
    /// [`ToolError::PathOutsideWorktree`](crate::tool::ToolError::PathOutsideWorktree).
    pub worktree_path: PathBuf,
    /// Wall-clock budget for this call (from
    /// [`ToolDef::timeout_ms`](crate::tool::ToolDef::timeout_ms)).
    pub timeout: Duration,
    /// Capability flags granted to this handler (the intersection of the
    /// role's [`ToolPermissions`](crate::ToolPermissions) and any
    /// per-call restrictions).
    pub capabilities: ToolPermission,
    /// Where to publish audit signals (coarse-grained orchestration events).
    pub audit_sink: Arc<dyn AuditSink>,
    /// Where to publish execution trace events (fine-grained per-call timelines).
    pub trace_sink: Arc<dyn TraceSink>,
    /// Where to publish aggregated evaluation metrics.
    pub metrics_sink: Arc<dyn MetricsSink>,
    /// Cancellation signal from the conductor.
    pub cancel_token: Arc<dyn CancelToken>,
}

impl ToolContext {
    /// Construct a context explicitly.
    #[must_use]
    pub fn new(
        worktree_path: impl Into<PathBuf>,
        timeout: Duration,
        capabilities: ToolPermission,
        audit_sink: Arc<dyn AuditSink>,
        trace_sink: Arc<dyn TraceSink>,
        metrics_sink: Arc<dyn MetricsSink>,
        cancel_token: Arc<dyn CancelToken>,
    ) -> Self {
        Self {
            worktree_path: worktree_path.into(),
            timeout,
            capabilities,
            audit_sink,
            trace_sink,
            metrics_sink,
            cancel_token,
        }
    }

    /// Construct a context suitable for tests:
    /// 60-second timeout, full capabilities, all no-op sinks, never-cancel token.
    #[must_use]
    pub fn testing(worktree_path: impl Into<PathBuf>) -> Self {
        Self {
            worktree_path: worktree_path.into(),
            timeout: Duration::from_secs(60),
            capabilities: ToolPermission {
                read: true,
                write: true,
                exec: true,
                git: true,
                network: false,
            },
            audit_sink: Arc::new(NoopAuditSink),
            trace_sink: Arc::new(NoopTraceSink),
            metrics_sink: Arc::new(NoopMetricsSink),
            cancel_token: Arc::new(NeverCancel),
        }
    }

    /// Replace the trace sink (builder-style for test setup).
    #[must_use]
    pub fn with_trace_sink(mut self, sink: Arc<dyn TraceSink>) -> Self {
        self.trace_sink = sink;
        self
    }

    /// Replace the metrics sink (builder-style for test setup).
    #[must_use]
    pub fn with_metrics_sink(mut self, sink: Arc<dyn MetricsSink>) -> Self {
        self.metrics_sink = sink;
        self
    }

    /// Replace the audit sink (builder-style for test setup).
    #[must_use]
    pub fn with_audit_sink(mut self, sink: Arc<dyn AuditSink>) -> Self {
        self.audit_sink = sink;
        self
    }

    /// Replace the cancel token (builder-style for test setup).
    #[must_use]
    pub fn with_cancel_token(mut self, token: Arc<dyn CancelToken>) -> Self {
        self.cancel_token = token;
        self
    }

    /// Short-cut: is the context's [`CancelToken`] tripped?
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// The worktree as a [`Path`].
    #[must_use]
    pub fn worktree(&self) -> &Path {
        &self.worktree_path
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolContext")
            .field("worktree_path", &self.worktree_path)
            .field("timeout", &self.timeout)
            .field("capabilities", &self.capabilities)
            .field("audit_sink", &"Arc<dyn AuditSink>")
            .field("trace_sink", &"Arc<dyn TraceSink>")
            .field("metrics_sink", &"Arc<dyn MetricsSink>")
            .field("cancel_token", &"Arc<dyn CancelToken>")
            .finish()
    }
}

// ─── ToolHandler ──────────────────────────────────────────────────────────

/// The async executor for a single tool.
///
/// Every built-in tool (§36.b) is an implementor of [`ToolHandler`];
/// MCP-backed and plugin tools are too. The dispatcher (§36.d) resolves
/// a [`ToolCall`] to a handler via the registry, then calls
/// [`Self::execute`].
#[async_trait]
pub trait ToolHandler: Send + Sync {
    /// Canonical snake_case name — matches `ToolDef::name`.
    fn name(&self) -> &str;

    /// Execute the tool. The dispatcher is responsible for wrapping this
    /// in a timeout and cancellation race; handlers should poll
    /// [`ToolContext::is_cancelled`] at cheap boundaries for prompt exits.
    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_audit_sink_accepts_signals() {
        let sink = NoopAuditSink;
        // Construct a minimal signal via the builder (body can be empty).
        let signal = Signal::builder(crate::Kind::Task).build();
        sink.emit(signal);
    }

    #[test]
    fn never_cancel_never_fires() {
        let c = NeverCancel;
        assert!(!c.is_cancelled());
    }

    #[test]
    fn atomic_cancel_flips_once() {
        let c = AtomicCancel::new();
        assert!(!c.is_cancelled());
        c.cancel();
        assert!(c.is_cancelled());
        // Stays tripped.
        assert!(c.is_cancelled());
    }

    #[test]
    fn context_testing_has_full_local_capabilities() {
        let ctx = ToolContext::testing("/tmp/work");
        assert!(ctx.capabilities.read);
        assert!(ctx.capabilities.write);
        assert!(ctx.capabilities.exec);
        assert!(ctx.capabilities.git);
        assert!(!ctx.capabilities.network);
        assert_eq!(ctx.worktree(), Path::new("/tmp/work"));
        assert_eq!(ctx.timeout, Duration::from_secs(60));
        assert!(!ctx.is_cancelled());
    }

    #[test]
    fn context_new_is_explicit() {
        let c = AtomicCancel::new();
        c.cancel();
        let ctx = ToolContext::new(
            "/tmp/w",
            Duration::from_secs(5),
            ToolPermission::read_only(),
            Arc::new(NoopAuditSink),
            Arc::new(NoopTraceSink),
            Arc::new(NoopMetricsSink),
            Arc::new(c),
        );
        assert!(ctx.is_cancelled());
        assert_eq!(ctx.timeout, Duration::from_secs(5));
    }

    #[test]
    fn context_testing_has_all_sinks_wired() {
        let ctx = ToolContext::testing("/tmp/sinks");
        // Just verify the builder chain doesn't panic and sinks are wired.
        let ctx = ctx
            .with_trace_sink(Arc::new(NoopTraceSink))
            .with_metrics_sink(Arc::new(NoopMetricsSink))
            .with_audit_sink(Arc::new(NoopAuditSink))
            .with_cancel_token(Arc::new(NeverCancel));
        assert!(!ctx.is_cancelled());
    }

    #[test]
    fn context_debug_does_not_blow_up() {
        let ctx = ToolContext::testing("/tmp/debug");
        let s = format!("{ctx:?}");
        assert!(s.contains("ToolContext"));
        assert!(s.contains("/tmp/debug"));
    }

    // Compile-time check that a ToolHandler impl is accepted by
    // `Arc<dyn ToolHandler>` (object-safe) and is Send + Sync.
    struct EchoHandler;

    #[async_trait]
    impl ToolHandler for EchoHandler {
        #[allow(clippy::unnecessary_literal_bound)]
        fn name(&self) -> &str {
            "echo"
        }

        async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
            ToolResult::text(call.arguments.to_string())
        }
    }

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn tool_handler_is_object_safe_and_thread_safe() {
        let handler: Arc<dyn ToolHandler> = Arc::new(EchoHandler);
        assert_eq!(handler.name(), "echo");
        assert_send_sync::<EchoHandler>();
    }
}
