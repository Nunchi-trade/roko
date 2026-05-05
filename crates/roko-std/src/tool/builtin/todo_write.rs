//! `todo_write` — manage the agent's per-turn todo list.
//!
//! Category: [`ToolCategory::Meta`]. Permission: none (meta-tool that
//! only mutates the agent's local plan; no filesystem/exec/network).
//! Concurrency: [`ToolConcurrency::Serial`]. Idempotent: yes.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

/// Canonical `snake_case` name.
pub const NAME: &str = "todo_write";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Manage the agent's per-turn todo list: create, update, \
    or complete todo items to track plan progress.";

/// Build the [`ToolDef`] for `todo_write`.
#[must_use]
pub fn tool_def() -> ToolDef {
    // `todo_write` mutates only in-memory agent state (the todo list),
    // so it intentionally requires *no* capability flags — any role can
    // call it regardless of its `ToolPermissions`.
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Meta,
        ToolPermission::default(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "todos": {
                "type": "array",
                "description": "List of todo items representing the agent's current plan.",
                "items": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "Description of the todo item." },
                        "status": { "type": "string", "enum": ["pending", "in_progress", "done"], "description": "Item status." }
                    },
                    "required": ["content"]
                }
            }
        },
        "required": ["todos"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(true)
    .with_timeout_ms(5_000)
}

/// Handler for `todo_write` (§36.25).
///
/// Validates the `todos` argument is an array of objects with at least a
/// `content` field, then echoes the count back to the caller. The actual
/// per-session todo-list state is owned by the agent runtime; this
/// handler is a **stateless** validator that lets the LLM express its
/// plan structure without any server-side persistence.
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    async fn execute(&self, call: ToolCall, _ctx: &ToolContext) -> ToolResult {
        let Some(todos) = call
            .arguments
            .get("todos")
            .and_then(serde_json::Value::as_array)
        else {
            return ToolResult::Err(ToolError::SchemaInvalid(
                "todo_write: missing required array argument `todos`".into(),
            ));
        };
        let mut count = 0usize;
        for (i, item) in todos.iter().enumerate() {
            let content = item.get("content").and_then(serde_json::Value::as_str);
            if content.is_none() {
                return ToolResult::Err(ToolError::SchemaInvalid(format!(
                    "todo_write: todos[{i}].content must be a string"
                )));
            }
            count += 1;
        }
        ToolResult::text(format!("accepted {count} todo items"))
    }
}
