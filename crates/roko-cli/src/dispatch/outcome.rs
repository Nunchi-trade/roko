//! Outcome + error types for [`Dispatcher::dispatch`].
//!
//! Both types are *normalized*: every backend produces an
//! [`AgentOutcome`] with the same shape, so feedback writers
//! ([`runtime_feedback`]) can persist episodes / efficiency events
//! without branching on provider.
//!
//! [`Dispatcher::dispatch`]: super::Dispatcher::dispatch
//! [`runtime_feedback`]: crate::runtime_feedback

use thiserror::Error;

/// Normalized result from a single agent dispatch.
///
/// Every backend (Claude CLI, Anthropic API, Codex, Cursor, OpenAI-compat,
/// Ollama, Gemini, Perplexity) lands here through
/// `AgentDispatcherV2::run_agent_result_bridge` — the runner never sees
/// provider-specific shapes.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentOutcome {
    /// Task id this dispatch was for.
    pub task_id: String,
    /// Plan id (dispatchers operate per-plan, so this is always set).
    pub plan_id: String,
    /// Resolved model slug as the provider reported it (may differ from
    /// the requested slug when a fallback fired).
    pub model: String,
    /// Provider label (`"claude_cli"`, `"anthropic_api"`, ...). Recorded
    /// here so episode / efficiency writers don't have to re-derive it.
    pub provider: String,
    /// Captured agent output (truncated upstream to keep snapshots small).
    pub output: String,
    /// Input tokens consumed by this dispatch.
    pub tokens_in: u64,
    /// Output tokens produced by this dispatch.
    pub tokens_out: u64,
    /// Cumulative cost in USD reported by the provider.
    pub cost_usd: f64,
    /// Wall-clock duration of the dispatch in milliseconds.
    pub duration_ms: u64,
    /// Exit code if the agent ran as a subprocess; `None` for HTTP-only
    /// providers.
    pub exit_code: Option<i32>,
    /// `true` if the provider reported a soft error envelope (e.g. an
    /// HTTP API returned a structured error rather than a transport
    /// failure). Distinct from exit code which surfaces hard failures.
    pub is_error: bool,
}

impl AgentOutcome {
    /// `true` if the dispatch is considered successful for episode
    /// logging purposes (no soft error and a clean exit when applicable).
    #[must_use]
    pub fn succeeded(&self) -> bool {
        if self.is_error {
            return false;
        }
        match self.exit_code {
            None => true, // in-process providers — no exit code
            Some(0) => true,
            Some(_) => false,
        }
    }

    /// Total tokens (input + output). Used by efficiency writers.
    #[must_use]
    pub fn total_tokens(&self) -> u64 {
        self.tokens_in.saturating_add(self.tokens_out)
    }
}

/// Why a dispatch failed before producing an [`AgentOutcome`].
///
/// Failures captured *during* agent execution still produce an outcome
/// (with `is_error = true`); this type covers pre-spawn rejection only.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum DispatchError {
    /// Plan budget was already exhausted; dispatcher refused to spawn.
    #[error("budget exceeded: spent ${spent:.4} of ${limit:.4}")]
    BudgetExceeded {
        /// USD spent on this plan so far.
        spent: f64,
        /// Configured plan-level USD limit.
        limit: f64,
    },
    /// No available model could be selected for this task.
    #[error("no model available: {reason}")]
    NoModelAvailable {
        /// Why selection failed (no router, all candidates filtered, ...).
        reason: String,
    },
    /// Pre-spawn validation rejected the request.
    #[error("pre-validation failed: {reason}")]
    PreValidationFailed {
        /// Specific validation that failed.
        reason: String,
    },
    /// Provider spawn returned an error before any output was produced.
    #[error("agent spawn failed: {0}")]
    SpawnFailed(String),
    /// The dispatch was cancelled by the runner.
    #[error("dispatch cancelled")]
    Cancelled,
    /// Prompt assembly failed.
    #[error("prompt assembly failed: {0}")]
    PromptAssembly(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome() -> AgentOutcome {
        AgentOutcome {
            task_id: "t".into(),
            plan_id: "p".into(),
            model: "claude-sonnet-4-6".into(),
            provider: "claude_cli".into(),
            output: "...".into(),
            tokens_in: 100,
            tokens_out: 50,
            cost_usd: 0.001,
            duration_ms: 42,
            exit_code: Some(0),
            is_error: false,
        }
    }

    #[test]
    fn succeeded_treats_clean_exit_as_success() {
        assert!(outcome().succeeded());
    }

    #[test]
    fn succeeded_rejects_soft_error_envelope() {
        let mut o = outcome();
        o.is_error = true;
        assert!(!o.succeeded());
    }

    #[test]
    fn succeeded_rejects_nonzero_exit() {
        let mut o = outcome();
        o.exit_code = Some(1);
        assert!(!o.succeeded());
    }

    #[test]
    fn succeeded_treats_in_process_provider_as_success_when_clean() {
        let mut o = outcome();
        o.exit_code = None;
        assert!(o.succeeded());
    }

    #[test]
    fn total_tokens_does_not_panic_on_overflow() {
        let mut o = outcome();
        o.tokens_in = u64::MAX;
        o.tokens_out = 100;
        assert_eq!(o.total_tokens(), u64::MAX);
    }

    #[test]
    fn dispatch_error_display_carries_actionable_detail() {
        let err = DispatchError::BudgetExceeded {
            spent: 25.5,
            limit: 20.0,
        };
        let msg = err.to_string();
        assert!(msg.contains("25.5"));
        assert!(msg.contains("20"));
    }
}
