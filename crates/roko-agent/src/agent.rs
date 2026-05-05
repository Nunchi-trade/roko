//! The `Agent` trait and `AgentResult` type.

use crate::streaming::StreamChunk;
use crate::usage::{Usage, UsageObservation};
use async_trait::async_trait;
use roko_core::{Body, ContentHash, Context, Signal, SignalBuilder, Kind};
use tokio::sync::mpsc;

/// The result of running an agent once.
#[derive(Clone, Debug)]
pub struct AgentResult {
    /// The primary output signal. Typically `Signal<Kind::AgentOutput>` containing
    /// the agent's final response.
    pub output: Signal,

    /// Intermediate signals emitted during the run (stream messages, tool calls,
    /// diff updates, errors). These are ordered chronologically.
    pub trace: Vec<Signal>,

    /// Legacy token usage + cost, kept for compatibility.
    pub usage: Usage,

    /// Canonical usage observation with optional provenance.
    pub usage_obs: Option<UsageObservation>,

    /// Whether the agent ran successfully (non-zero exit / connection errors = false).
    pub success: bool,
}

impl AgentResult {
    /// Construct a successful result with just an output signal.
    #[must_use]
    pub const fn ok(output: Signal) -> Self {
        Self {
            output,
            trace: Vec::new(),
            usage: Usage::zero(),
            usage_obs: None,
            success: true,
        }
    }

    /// Construct a failed result with an output signal describing the failure.
    #[must_use]
    pub const fn fail(output: Signal) -> Self {
        Self {
            output,
            trace: Vec::new(),
            usage: Usage::zero(),
            usage_obs: None,
            success: false,
        }
    }

    /// Attach trace signals.
    #[must_use]
    pub fn with_trace(mut self, trace: Vec<Signal>) -> Self {
        self.trace = trace;
        self
    }

    /// Attach legacy usage metrics and mirror them into `usage_obs`.
    #[must_use]
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self.usage_obs = Some(usage.into());
        self
    }

    /// Attach a canonical usage observation and derive the legacy counters.
    #[must_use]
    pub fn with_usage_obs(mut self, usage_obs: UsageObservation) -> Self {
        self.usage = usage_obs.clone().into();
        self.usage_obs = Some(usage_obs);
        self
    }

    /// All signals produced by this run (trace + output), in chronological order.
    #[must_use]
    pub fn all_signals(&self) -> Vec<Signal> {
        let mut v = self.trace.clone();
        v.push(self.output.clone());
        v
    }

    /// All engrams produced by this run (trace + output), in chronological order.
    ///
    /// Compatibility alias for older docs and callers that still use the
    /// `all_engrams` name.
    #[must_use]
    pub fn all_engrams(&self) -> Vec<Signal> {
        self.all_signals()
    }
}

/// Build an output signal that keeps the full upstream lineage from `input`.
///
/// Many runtime wrappers only emit a single final `AgentOutput`, so this helper
/// centralizes the "input lineage + direct parent" propagation rule.
#[must_use]
pub fn derived_output(input: &Signal, kind: Kind, body: Body) -> SignalBuilder {
    Signal::builder(kind).body(body).lineage(
        input
            .lineage
            .iter()
            .copied()
            .chain(std::iter::once(input.id)),
    )
}

/// Return the full upstream lineage for `input`, including the input hash.
#[must_use]
pub fn full_lineage(input: &Signal) -> impl Iterator<Item = ContentHash> + '_ {
    input
        .lineage
        .iter()
        .copied()
        .chain(std::iter::once(input.id))
}

/// An agent: an async executor that takes an input signal (typically a prompt)
/// and produces output signals.
///
/// # Design
///
/// Agents don't fit any of the 6 core traits because they:
/// 1. Are **async** (subprocess, network, LLM API)
/// 2. Have **side effects** (file edits, stdout)
/// 3. Produce **multiple signals** over time (stream)
/// 4. Are **non-deterministic** (LLMs are stochastic)
///
/// Rather than distort another trait, `Agent` is its own capability. Most
/// orchestrator work doesn't call `Agent` directly; a React decides when
/// to run an agent and the runtime dispatches it.
///
/// # Example
///
/// ```ignore
/// let agent = ExecAgent::new("echo", vec![], SafetyLayer::with_defaults());
/// let prompt = Signal::builder(Kind::Prompt).body(Body::text("hello")).build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// assert!(result.success);
/// ```
#[async_trait]
pub trait Agent: Send + Sync {
    /// Run the agent against the input signal.
    ///
    /// The `input` is typically a `Signal<Kind::Prompt>`, but agents may
    /// accept any kind (e.g. a `Signal<Kind::Task>` for task-aware agents).
    ///
    /// Returns an [`AgentResult`] with the primary output, trace of
    /// intermediate signals, and usage metrics.
    async fn run(&self, input: &Signal, ctx: &Context) -> AgentResult;

    /// Human-readable name for logs/metrics.
    fn name(&self) -> &str;

    /// Stable backend identifier for audit and episode logging.
    fn backend_id(&self) -> &'static str {
        "unknown"
    }

    /// Does this agent emit a streaming trace (many signals), or a single output?
    fn supports_streaming(&self) -> bool {
        false
    }

    /// Run the agent with streaming output.
    ///
    /// Agents that support real streaming override this to forward
    /// [`StreamChunk`]s as they arrive from the backend. The default
    /// implementation falls back to [`run`](Self::run) and emits a single
    /// `ContentDelta` with the full output text.
    async fn run_streaming(
        &self,
        input: &Signal,
        ctx: &Context,
        event_tx: mpsc::Sender<StreamChunk>,
    ) -> AgentResult {
        let result = self.run(input, ctx).await;
        if let Ok(text) = result.output.body.as_text() {
            if !text.is_empty() {
                let _ = event_tx.send(StreamChunk::ContentDelta(text.to_string())).await;
            }
        }
        if result.usage.total_tokens() > 0 {
            let _ = event_tx.send(StreamChunk::Usage(result.usage)).await;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    #[test]
    fn agent_result_ok_sets_success() {
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("ok"))
            .build();
        let r = AgentResult::ok(out);
        assert!(r.success);
        assert!(r.trace.is_empty());
    }

    #[test]
    fn agent_result_fail_sets_success_false() {
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("boom"))
            .build();
        let r = AgentResult::fail(out);
        assert!(!r.success);
    }

    #[test]
    fn all_signals_is_trace_then_output() {
        let trace1 = Signal::builder(Kind::AgentMessage)
            .body(Body::text("1"))
            .build();
        let trace2 = Signal::builder(Kind::AgentMessage)
            .body(Body::text("2"))
            .build();
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("done"))
            .build();
        let r = AgentResult::ok(out.clone()).with_trace(vec![trace1.clone(), trace2.clone()]);
        let all = r.all_signals();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, trace1.id);
        assert_eq!(all[1].id, trace2.id);
        assert_eq!(all[2].id, out.id);
    }

    #[test]
    fn builder_chain() {
        let out = Signal::builder(Kind::AgentOutput)
            .body(Body::text("x"))
            .build();
        let r = AgentResult::ok(out).with_trace(vec![]).with_usage(Usage {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        });
        assert_eq!(r.usage.input_tokens, 100);
        assert!(r.usage_obs.is_some());
    }
}
