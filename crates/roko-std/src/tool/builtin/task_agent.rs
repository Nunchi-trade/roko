//! `task` — delegate a sub-task to a specialized sub-agent (meta-tool).
//!
//! Note: the module is named `task_agent` to avoid namespace clash with
//! the `task` concept used elsewhere in the orchestrator; the canonical
//! tool name exposed to the LLM remains `"task"` (Claude aliases it to
//! `Agent` — see [`roko_core::tool::aliases`]).
//!
//! Category: [`ToolCategory::Meta`]. Permission: read + write (the
//! spawned sub-agent inherits these at minimum). Concurrency:
//! [`ToolConcurrency::Parallel`]. Idempotent: no.
//!
//! # Day-one stub
//!
//! Sub-agent dispatch (spawn + supervise + harvest result) will land
//! with the `roko-orchestrator` crate in a later milestone. For now
//! this handler validates the arguments and returns a structured
//! "not yet wired" error so plan authors can still list `task` as a
//! permitted tool and receive a clean failure signal.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::require_string;

/// Canonical `snake_case` name (aliases to Claude's `Agent`).
pub const NAME: &str = "task";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Launch a specialized sub-agent to handle a focused task \
    autonomously and return its result.";

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

/// Handler for `task` (§36.25) — day-one stub.
///
/// Validates the two load-bearing arguments (`subagent_type`,
/// `prompt`) and reports a structured "dispatch not wired" error.
/// The full implementation hooks into the orchestrator's
/// supervisor when that crate lands.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let subagent_type = match require_string(&call.arguments, "subagent_type") {
            Ok(s) => s,
            Err(e) => return ToolResult::Err(e),
        };
        if subagent_type.trim().is_empty() {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "task: `subagent_type` must be non-empty".into(),
            ));
        }
        if let Err(e) = require_string(&call.arguments, "prompt") {
            return ToolResult::Err(e);
        }
        ToolResult::Err(ToolError::Other(
            "task: subagent dispatch not yet wired (day-one stub)".into(),
        ))
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
    async fn valid_args_return_not_wired_stub() {
        let ctx = testing_ctx();
        let call = ToolCall::new(
            "c",
            NAME,
            serde_json::json!({ "subagent_type": "explorer", "prompt": "map the repo" }),
        );
        let res = Handler.execute(call, &ctx).await;
        match res {
            ToolResult::Err(ToolError::Other(msg)) => assert!(msg.contains("not yet wired")),
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn handler_name_matches_tool_def() {
        assert_eq!(Handler.name(), NAME);
    }
}
