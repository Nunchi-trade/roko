//! The `Agent` trait and `AgentResult` type.

use crate::usage::Usage;
use async_trait::async_trait;
use roko_core::{Context, Engram};

/// The result of running an agent once.
#[derive(Clone, Debug)]
pub struct AgentResult {
    /// The primary output signal. Typically `Engram<Kind::AgentOutput>` containing
    /// the agent's final response.
    pub output: Engram,

    /// Intermediate signals emitted during the run (stream messages, tool calls,
    /// diff updates, errors). These are ordered chronologically.
    pub trace: Vec<Engram>,

    /// Token usage + cost.
    pub usage: Usage,

    /// Whether the agent ran successfully (non-zero exit / connection errors = false).
    pub success: bool,
}

impl AgentResult {
    /// Construct a successful result with just an output signal.
    #[must_use]
    pub const fn ok(output: Engram) -> Self {
        Self {
            output,
            trace: Vec::new(),
            usage: Usage::zero(),
            success: true,
        }
    }

    /// Construct a failed result with an output signal describing the failure.
    #[must_use]
    pub const fn fail(output: Engram) -> Self {
        Self {
            output,
            trace: Vec::new(),
            usage: Usage::zero(),
            success: false,
        }
    }

    /// Attach trace signals.
    #[must_use]
    pub fn with_trace(mut self, trace: Vec<Engram>) -> Self {
        self.trace = trace;
        self
    }

    /// Attach usage metrics.
    #[must_use]
    pub const fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    /// All signals produced by this run (trace + output), in chronological order.
    #[must_use]
    pub fn all_signals(&self) -> Vec<Engram> {
        let mut v = self.trace.clone();
        v.push(self.output.clone());
        v
    }
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
/// orchestrator work doesn't call `Agent` directly; a Policy decides when
/// to run an agent and the runtime dispatches it.
///
/// # Example
///
/// ```ignore
/// let agent = ExecAgent::new("echo", vec![]);
/// let prompt = Engram::builder(Kind::Prompt).body(Body::text("hello")).build();
/// let result = agent.run(&prompt, &Context::now()).await;
/// assert!(result.success);
/// ```
#[async_trait]
pub trait Agent: Send + Sync {
    /// Run the agent against the input signal.
    ///
    /// The `input` is typically a `Engram<Kind::Prompt>`, but agents may
    /// accept any kind (e.g. a `Engram<Kind::Task>` for task-aware agents).
    ///
    /// Returns an [`AgentResult`] with the primary output, trace of
    /// intermediate signals, and usage metrics.
    async fn run(&self, input: &Engram, ctx: &Context) -> AgentResult;

    /// Human-readable name for logs/metrics.
    fn name(&self) -> &str;

    /// Does this agent emit a streaming trace (many signals), or a single output?
    fn supports_streaming(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    #[test]
    fn agent_result_ok_sets_success() {
        let out = Engram::builder(Kind::AgentOutput)
            .body(Body::text("ok"))
            .build();
        let r = AgentResult::ok(out);
        assert!(r.success);
        assert!(r.trace.is_empty());
    }

    #[test]
    fn agent_result_fail_sets_success_false() {
        let out = Engram::builder(Kind::AgentOutput)
            .body(Body::text("boom"))
            .build();
        let r = AgentResult::fail(out);
        assert!(!r.success);
    }

    #[test]
    fn all_signals_is_trace_then_output() {
        let trace1 = Engram::builder(Kind::AgentMessage)
            .body(Body::text("1"))
            .build();
        let trace2 = Engram::builder(Kind::AgentMessage)
            .body(Body::text("2"))
            .build();
        let out = Engram::builder(Kind::AgentOutput)
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
        let out = Engram::builder(Kind::AgentOutput)
            .body(Body::text("x"))
            .build();
        let r = AgentResult::ok(out).with_trace(vec![]).with_usage(Usage {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        });
        assert_eq!(r.usage.input_tokens, 100);
    }
}
