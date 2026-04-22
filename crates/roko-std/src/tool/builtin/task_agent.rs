//! `task` -- delegate a sub-task to a specialized sub-agent (meta-tool).
//!
//! Note: the module is named `task_agent` to avoid namespace clash with
//! the `task` concept used elsewhere in the orchestrator; the canonical
//! tool name exposed to the LLM remains `"task"` (Claude aliases it to
//! `Agent` -- see [`roko_core::tool::aliases`]).
//!
//! Category: [`ToolCategory::Meta`]. Permission: read + write (the
//! spawned sub-agent inherits these at minimum). Concurrency:
//! [`ToolConcurrency::Parallel`]. Idempotent: no.
//!
//! # Sub-agent dispatch
//!
//! Since `roko-std` cannot depend on `roko-cli` or `roko-agent` (that
//! would introduce a circular dependency), sub-agent dispatch uses a
//! trait-based injection pattern:
//!
//! 1. [`SubAgentDispatcher`] defines the async interface for spawning a
//!    one-shot agent.
//! 2. At startup, `roko-cli` calls [`set_sub_agent_dispatcher`] to inject
//!    a concrete implementation backed by the real agent spawn machinery.
//! 3. At execute time, [`Handler`] reads the global dispatcher via
//!    [`get_sub_agent_dispatcher`]. If none was injected, it returns a
//!    clear error.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};
use std::sync::{Arc, OnceLock};

use super::sandbox::require_string;

/// Canonical `snake_case` name (aliases to Claude's `Agent`).
pub const NAME: &str = "task";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Launch a specialized sub-agent to handle a focused task \
    autonomously and return its result.";

// ── SubAgentDispatcher trait ─────────────────────────────────────────────

/// Trait for dispatching a one-shot sub-agent.
///
/// Implementations live in higher-level crates (e.g. `roko-cli`) and are
/// injected at runtime via [`set_sub_agent_dispatcher`].
#[async_trait]
pub trait SubAgentDispatcher: Send + Sync {
    /// Spawn a sub-agent of the given type with the given prompt.
    ///
    /// `subagent_type` is a hint (e.g. `"explorer"`, `"coder"`,
    /// `"researcher"`) that the implementation can use to select the
    /// model, system prompt, or tool set.
    ///
    /// `prompt` is the user-facing instruction for the sub-agent.
    ///
    /// `worktree` is the working directory the sub-agent should operate
    /// in (inherited from the parent agent's context).
    ///
    /// Returns the sub-agent's textual output on success, or a
    /// human-readable error string on failure.
    async fn dispatch(
        &self,
        subagent_type: &str,
        prompt: &str,
        worktree: &std::path::Path,
    ) -> Result<String, String>;
}

/// Global dispatcher slot. Initialized once by the host binary.
static SUB_AGENT_DISPATCHER: OnceLock<Arc<dyn SubAgentDispatcher>> = OnceLock::new();

/// Inject the sub-agent dispatcher. Call once at startup.
///
/// Returns `Err` with the passed-in dispatcher if one was already set
/// (idempotent: the first call wins).
pub fn set_sub_agent_dispatcher(
    dispatcher: Arc<dyn SubAgentDispatcher>,
) -> Result<(), Arc<dyn SubAgentDispatcher>> {
    SUB_AGENT_DISPATCHER.set(dispatcher)
}

/// Retrieve the injected dispatcher, if any.
#[must_use]
pub fn get_sub_agent_dispatcher() -> Option<&'static Arc<dyn SubAgentDispatcher>> {
    SUB_AGENT_DISPATCHER.get()
}

/// Build the [`ToolDef`] for `task`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Meta,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(false)
    .with_timeout_ms(600_000)
}

/// Handler for `task` (section 36.25).
///
/// Validates the `subagent_type` and `prompt` arguments, then delegates
/// to the injected [`SubAgentDispatcher`]. If no dispatcher has been
/// injected, returns a clear error.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        // ── argument validation ──────────────────────────────────────────
        let subagent_type = match require_string(&call.arguments, "subagent_type") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        if subagent_type.trim().is_empty() {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "task: `subagent_type` must be non-empty".into(),
            ));
        }
        let prompt = match require_string(&call.arguments, "prompt") {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        if prompt.trim().is_empty() {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "task: `prompt` must be non-empty".into(),
            ));
        }

        // ── dispatch ─────────────────────────────────────────────────────
        let Some(dispatcher) = get_sub_agent_dispatcher() else {
            return ToolResult::Err(ToolError::Other(
                "task: no sub-agent dispatcher has been registered. \
                 The host binary must call `set_sub_agent_dispatcher()` at startup \
                 to enable sub-agent spawning."
                    .into(),
            ));
        };

        let worktree = ctx.worktree().to_path_buf();
        match dispatcher
            .dispatch(&subagent_type, &prompt, &worktree)
            .await
        {
            Ok(output) => ToolResult::text(output),
            Err(err) => ToolResult::Err(ToolError::Other(format!(
                "task: sub-agent ({subagent_type}) failed: {err}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::ToolContext;

    fn testing_ctx() -> ToolContext {
        ToolContext::testing("/tmp/work")
    }

    #[tokio::test]
    async fn missing_subagent_type_is_schema_invalid() {
        let ctx = testing_ctx();
        let call = ToolCall::new("c", NAME, serde_json::json!({ "prompt": "do the thing" }));
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn missing_prompt_is_schema_invalid() {
        let ctx = testing_ctx();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "subagent_type": "explorer" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn blank_subagent_type_is_rejected() {
        let ctx = testing_ctx();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "subagent_type": "  ", "prompt": "x" }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn blank_prompt_is_rejected() {
        let ctx = testing_ctx();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "subagent_type": "explorer", "prompt": "  " }),
        );
        let res = Handler.execute(call, &ctx).await;
        assert!(matches!(res, ToolResult::Err(ToolError::SchemaInvalid(_))));
    }

    #[tokio::test]
    async fn no_dispatcher_returns_clear_error() {
        // In tests, the global dispatcher is typically not set (OnceLock
        // may have been set by a parallel test; if so, this test becomes
        // a successful dispatch test instead -- but in a fresh process
        // the OnceLock is empty).
        //
        // We can't un-set a OnceLock, so we just verify the error
        // message pattern when no dispatcher is available.
        if get_sub_agent_dispatcher().is_none() {
            let ctx = testing_ctx();
            let call = ToolCall::new(
                "c",
                NAME,
                serde_json::json!({ "subagent_type": "explorer", "prompt": "map the repo" }),
            );
            let res = Handler.execute(call, &ctx).await;
            match res {
                ToolResult::Err(ToolError::Other(msg)) => {
                    assert!(
                        msg.contains("no sub-agent dispatcher"),
                        "expected clear error about missing dispatcher, got: {msg}"
                    );
                }
                other => panic!("expected Other error about missing dispatcher, got: {other:?}"),
            }
        }
    }

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }

    // ── SubAgentDispatcher trait is object-safe ──────────────────────────

    struct MockDispatcher {
        response: Result<String, String>,
    }

    #[async_trait]
    impl SubAgentDispatcher for MockDispatcher {
        async fn dispatch(
            &self,
            _subagent_type: &str,
            _prompt: &str,
            _worktree: &std::path::Path,
        ) -> Result<String, String> {
            self.response.clone()
        }
    }

    #[test]
    fn sub_agent_dispatcher_is_object_safe() {
        // Compile-time verification that the trait is object-safe.
        let _: Arc<dyn SubAgentDispatcher> = Arc::new(MockDispatcher {
            response: Ok("done".into()),
        });
    }

    #[tokio::test]
    async fn mock_dispatcher_returns_ok() {
        let mock = MockDispatcher {
            response: Ok("sub-agent output".into()),
        };
        let result = mock
            .dispatch("explorer", "map the repo", std::path::Path::new("/tmp"))
            .await;
        assert_eq!(result, Ok("sub-agent output".into()));
    }

    #[tokio::test]
    async fn mock_dispatcher_returns_err() {
        let mock = MockDispatcher {
            response: Err("agent crashed".into()),
        };
        let result = mock
            .dispatch("coder", "fix the bug", std::path::Path::new("/tmp"))
            .await;
        assert_eq!(result, Err("agent crashed".into()));
    }
}
