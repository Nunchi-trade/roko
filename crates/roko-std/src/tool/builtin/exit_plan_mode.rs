//! `exit_plan_mode` — signal the end of plan-mode with the drafted plan.
//!
//! Category: [`ToolCategory::Meta`]. Permission: none (meta signal).
//! Concurrency: [`ToolConcurrency::Serial`]. Idempotent: yes.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

/// Canonical `snake_case` name.
pub const NAME: &str = "exit_plan_mode";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Exit plan-mode and submit the drafted plan for approval.";

/// Build the [`ToolDef`] for `exit_plan_mode`.
#[must_use]
pub fn tool_def() -> ToolDef {
    // Meta-signal with no capability requirements.
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Meta,
        ToolPermission::default(),
    )
    .with_parameters(ToolSchema::any_object())
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(true)
    .with_timeout_ms(5_000)
}

/// Handler for `exit_plan_mode` (§36.27).
///
/// Validates the `plan` argument exists and echoes a structured JSON
/// result containing the plan text so downstream orchestrator code can
/// extract it from the `ToolTrace`. The conductor owns the decision of
/// whether to transition out of plan mode — this handler only signals.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let plan = match call
            .arguments
            .get("plan")
            .and_then(serde_json::Value::as_str)
        {
            Some(s) => s.to_string(),
            None => {
                return ToolResult::Err(ToolError::SchemaInvalid(
                    "exit_plan_mode: missing required string argument `plan`".into(),
                ));
            }
        };
        let payload = serde_json::json!({
            "status": "plan_submitted",
            "plan_length_chars": plan.len(),
        });
        ToolResult::structured(payload.to_string())
    }
}
