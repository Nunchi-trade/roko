//! Reusable agent-session policy and resume metadata.
//!
//! These types keep warm-agent reuse explicit. A warmed instance may only be
//! selected for a new task when the caller supplies a matching request; old
//! prompts or context fingerprints are never implicitly carried forward.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use roko_core::ContentHash;
use serde::{Deserialize, Serialize};

/// Scope in which a warmed agent/session may be reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReuseScope {
    /// Reuse is disabled.
    Disabled,
    /// Reuse is valid only for the exact task id.
    Task,
    /// Reuse is valid within the same plan id.
    Plan,
    /// Reuse is valid for a caller-defined session id.
    Session,
}

/// Opt-in policy attached to a warm agent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarmReusePolicy {
    /// Stable policy id for audit/debugging.
    pub policy_id: String,
    /// The maximum scope where reuse is allowed.
    pub scope: ReuseScope,
    /// Optional maximum idle age before this warm entry is stale.
    pub max_idle_ms: Option<u64>,
    /// Plan id this warmed session is bound to.
    pub plan_id: Option<String>,
    /// Task id this warmed session is bound to.
    pub task_id: Option<String>,
    /// Caller-defined session id this warmed session is bound to.
    pub session_id: Option<String>,
    /// Prompt-policy fingerprint used when the session was warmed.
    pub prompt_policy_fingerprint: Option<String>,
    /// Context fingerprint used when the session was warmed.
    pub context_fingerprint: Option<String>,
    /// Whether context from the warmed session may carry into the next task.
    pub allow_context_carryover: bool,
}

impl WarmReusePolicy {
    /// A policy for stateless warm entries that carry no context.
    #[must_use]
    pub fn stateless(policy_id: impl Into<String>) -> Self {
        Self {
            policy_id: policy_id.into(),
            scope: ReuseScope::Session,
            max_idle_ms: None,
            plan_id: None,
            task_id: None,
            session_id: None,
            prompt_policy_fingerprint: None,
            context_fingerprint: None,
            allow_context_carryover: false,
        }
    }

    /// A disabled policy. This is useful for preserving legacy warm-pool API
    /// behavior while making production reuse call a checked path.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            policy_id: "disabled".to_string(),
            scope: ReuseScope::Disabled,
            max_idle_ms: Some(0),
            plan_id: None,
            task_id: None,
            session_id: None,
            prompt_policy_fingerprint: None,
            context_fingerprint: None,
            allow_context_carryover: false,
        }
    }

    /// Bind this policy to a plan.
    #[must_use]
    pub fn for_plan(mut self, plan_id: impl Into<String>) -> Self {
        self.plan_id = Some(plan_id.into());
        self
    }

    /// Bind this policy to a task.
    #[must_use]
    pub fn for_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    /// Bind this policy to a session.
    #[must_use]
    pub fn for_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Attach prompt and context fingerprints.
    #[must_use]
    pub fn with_fingerprints(
        mut self,
        prompt_policy_fingerprint: Option<String>,
        context_fingerprint: Option<String>,
    ) -> Self {
        self.prompt_policy_fingerprint = prompt_policy_fingerprint;
        self.context_fingerprint = context_fingerprint;
        self
    }

    /// Set the maximum idle age for this policy.
    #[must_use]
    pub fn with_max_idle(mut self, max_idle: Duration) -> Self {
        self.max_idle_ms = Some(u64::try_from(max_idle.as_millis()).unwrap_or(u64::MAX));
        self
    }

    /// Allow explicit context carryover.
    #[must_use]
    pub fn allow_context_carryover(mut self, allow: bool) -> Self {
        self.allow_context_carryover = allow;
        self
    }

    /// Validate whether this policy can satisfy `request` at `now`.
    #[must_use]
    pub fn allows(&self, request: &WarmReuseRequest, warmed_at: Instant, now: Instant) -> bool {
        if self.scope == ReuseScope::Disabled {
            return false;
        }
        if let Some(max_idle_ms) = self.max_idle_ms {
            let age_ms =
                u64::try_from(now.duration_since(warmed_at).as_millis()).unwrap_or(u64::MAX);
            if age_ms > max_idle_ms {
                return false;
            }
        }
        if self.prompt_policy_fingerprint != request.prompt_policy_fingerprint {
            return false;
        }
        if self.context_fingerprint != request.context_fingerprint {
            return false;
        }
        if request.context_fingerprint.is_some() && !self.allow_context_carryover {
            return false;
        }

        match self.scope {
            ReuseScope::Disabled => false,
            ReuseScope::Task => {
                self.plan_id.as_deref() == request.plan_id.as_deref()
                    && self.task_id.as_deref() == request.task_id.as_deref()
            }
            ReuseScope::Plan => self.plan_id.as_deref() == request.plan_id.as_deref(),
            ReuseScope::Session => self.session_id.as_deref() == request.session_id.as_deref(),
        }
    }
}

/// Request supplied by the scheduler when selecting a warm entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WarmReuseRequest {
    /// Plan id for scope checks.
    pub plan_id: Option<String>,
    /// Task id for scope checks.
    pub task_id: Option<String>,
    /// Caller-defined session id for scope checks.
    pub session_id: Option<String>,
    /// Required prompt-policy fingerprint.
    pub prompt_policy_fingerprint: Option<String>,
    /// Required context fingerprint.
    pub context_fingerprint: Option<String>,
}

impl WarmReuseRequest {
    /// Build a request scoped to one task.
    #[must_use]
    pub fn task(plan_id: impl Into<String>, task_id: impl Into<String>) -> Self {
        Self {
            plan_id: Some(plan_id.into()),
            task_id: Some(task_id.into()),
            ..Default::default()
        }
    }

    /// Build a request scoped to one session.
    #[must_use]
    pub fn session(session_id: impl Into<String>) -> Self {
        Self {
            session_id: Some(session_id.into()),
            ..Default::default()
        }
    }

    /// Attach required fingerprints.
    #[must_use]
    pub fn with_fingerprints(
        mut self,
        prompt_policy_fingerprint: Option<String>,
        context_fingerprint: Option<String>,
    ) -> Self {
        self.prompt_policy_fingerprint = prompt_policy_fingerprint;
        self.context_fingerprint = context_fingerprint;
        self
    }
}

/// Persistable metadata for one agent invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInvocationSession {
    /// Unique invocation id.
    pub invocation_id: String,
    /// Optional provider/native session id.
    pub provider_session_id: Option<String>,
    /// Backend id, for example `claude_cli` or `openai_compat`.
    pub backend_id: String,
    /// Model slug/key used for the invocation.
    pub model: String,
    /// Role label used for the invocation.
    pub role: String,
    /// Plan id, if this invocation belongs to a plan.
    pub plan_id: Option<String>,
    /// Task id, if this invocation belongs to a task.
    pub task_id: Option<String>,
    /// Prompt fingerprint for resume/reuse validation.
    pub prompt_fingerprint: String,
    /// Context fingerprint for resume/reuse validation.
    pub context_fingerprint: Option<String>,
    /// Warm reuse policy used for the invocation.
    pub reuse_policy: WarmReusePolicy,
    /// Working directory for the invocation.
    pub working_dir: Option<PathBuf>,
    /// Unix milliseconds when the invocation started.
    pub started_at_ms: u64,
    /// Unix milliseconds when the invocation ended.
    pub ended_at_ms: Option<u64>,
    /// Timeout configured for the invocation.
    pub timeout_ms: Option<u64>,
    /// Final state, if known.
    pub state: InvocationState,
}

/// Durable invocation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationState {
    /// Invocation is running or was interrupted before a terminal update.
    InProgress,
    /// Invocation completed successfully.
    Succeeded,
    /// Invocation failed.
    Failed,
    /// Invocation timed out and should not be treated as success.
    TimedOut,
    /// Invocation was cancelled.
    Cancelled,
}

impl InvocationState {
    /// Whether the state is terminal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }

    /// Whether resume can be attempted from this state.
    #[must_use]
    pub const fn is_resumable(self) -> bool {
        matches!(self, Self::InProgress | Self::TimedOut | Self::Cancelled)
    }
}

/// Validate that persisted invocation metadata matches a requested resume.
///
/// This fails closed on model/backend/prompt/context mismatches.
pub fn validate_resume_request(
    persisted: &AgentInvocationSession,
    requested: &AgentInvocationSession,
) -> Result<(), ResumeValidationError> {
    if !persisted.state.is_resumable() {
        return Err(ResumeValidationError::TerminalState(persisted.state));
    }
    if persisted.backend_id != requested.backend_id {
        return Err(ResumeValidationError::BackendMismatch);
    }
    if persisted.model != requested.model {
        return Err(ResumeValidationError::ModelMismatch);
    }
    if persisted.role != requested.role {
        return Err(ResumeValidationError::RoleMismatch);
    }
    if persisted.plan_id != requested.plan_id || persisted.task_id != requested.task_id {
        return Err(ResumeValidationError::ScopeMismatch);
    }
    if persisted.prompt_fingerprint != requested.prompt_fingerprint {
        return Err(ResumeValidationError::PromptMismatch);
    }
    if persisted.context_fingerprint != requested.context_fingerprint {
        return Err(ResumeValidationError::ContextMismatch);
    }
    Ok(())
}

/// Resume validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResumeValidationError {
    /// Persisted state is terminal and should not be resumed.
    #[error("invocation already reached terminal state {0:?}")]
    TerminalState(InvocationState),
    /// Backend changed.
    #[error("backend mismatch")]
    BackendMismatch,
    /// Model changed.
    #[error("model mismatch")]
    ModelMismatch,
    /// Role changed.
    #[error("role mismatch")]
    RoleMismatch,
    /// Plan/task scope changed.
    #[error("scope mismatch")]
    ScopeMismatch,
    /// Prompt fingerprint changed.
    #[error("prompt fingerprint mismatch")]
    PromptMismatch,
    /// Context fingerprint changed.
    #[error("context fingerprint mismatch")]
    ContextMismatch,
}

/// Stable BLAKE3 fingerprint for prompt/context policy material.
#[must_use]
pub fn fingerprint_text(text: &str) -> String {
    ContentHash::of(text.as_bytes()).to_hex()
}
