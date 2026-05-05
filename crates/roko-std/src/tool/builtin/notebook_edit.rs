//! `notebook_edit` — edit cells in a Jupyter notebook (`.ipynb`).
//!
//! Category: [`ToolCategory::Notebook`]. Permission: read + write.
//! Concurrency: [`ToolConcurrency::Serial`]. Idempotent: no.

use async_trait::async_trait;
use roko_core::tool::{
    ToolCall, ToolCategory, ToolConcurrency, ToolContext, ToolDef, ToolError, ToolHandler,
    ToolPermission, ToolResult, ToolSchema,
};

use super::sandbox::{require_string, require_within_worktree};

/// Canonical `snake_case` name.
pub const NAME: &str = "notebook_edit";

/// Human-readable description sent to the LLM.
pub const DESCRIPTION: &str = "Edit, insert, or delete a cell in a Jupyter notebook.";

/// Build the [`ToolDef`] for `notebook_edit`.
#[must_use]
pub fn tool_def() -> ToolDef {
    ToolDef::new(
        NAME,
        DESCRIPTION,
        ToolCategory::Notebook,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Relative path to the .ipynb notebook file."
            },
            "cell_index": {
                "type": "integer",
                "description": "0-based cell index to operate on."
            },
            "mode": {
                "type": "string",
                "enum": ["edit", "insert", "delete"],
                "description": "Operation mode (default: 'edit')."
            },
            "source": {
                "type": "string",
                "description": "New cell source content (required for 'edit' and 'insert' modes)."
            },
            "cell_type": {
                "type": "string",
                "enum": ["code", "markdown"],
                "description": "Cell type for 'insert' mode (default: 'code')."
            }
        },
        "required": ["path", "cell_index"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_idempotent(false)
    .with_timeout_ms(30_000)
}

/// Handler for `notebook_edit` (§36.24).
///
/// Arguments:
/// - `path` — required, path to the `.ipynb` file
/// - `cell_index` — required integer, 0-based index
/// - `mode` — `"edit"` (default), `"insert"`, or `"delete"`
/// - `source` — required for `edit`/`insert`; new source lines (string)
/// - `cell_type` — for `insert`; `"code"` (default) or `"markdown"`
#[derive(Debug, Clone, Copy, Default)]
pub struct Handler;

#[async_trait]
impl ToolHandler for Handler {
    fn name(&self) -> &str {
        NAME
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(&self, call: ToolCall, ctx: &ToolContext) -> ToolResult {
        let rel_path = match require_string(&call.arguments, "path") {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let idx = match call
            .arguments
            .get("cell_index")
            .and_then(serde_json::Value::as_u64)
        {
            Some(i) => usize::try_from(i).unwrap_or(usize::MAX),
            None => {
                return ToolResult::Err(ToolError::SchemaInvalid(
                    "notebook_edit: missing required integer `cell_index`".into(),
                ));
            }
        };
        let mode = call
            .arguments
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("edit");
        let absolute = match require_within_worktree(ctx.worktree(), &rel_path) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let raw = match tokio::fs::read_to_string(&absolute).await {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "notebook_edit: failed to read {}: {e}",
                    absolute.display()
                )));
            }
        };
        let mut nb: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "notebook_edit: invalid notebook JSON: {e}"
                )));
            }
        };
        let Some(cells) = nb
            .get_mut("cells")
            .and_then(serde_json::Value::as_array_mut)
        else {
            return ToolResult::Err(ToolError::Other(
                "notebook_edit: notebook has no `cells` array".into(),
            ));
        };
        match mode {
            "edit" => {
                let source = match require_string(&call.arguments, "source") {
                    Ok(s) => s,
                    Err(e) => return ToolResult::Err(e),
                };
                let Some(target) = cells.get_mut(idx) else {
                    return ToolResult::Err(ToolError::Other(format!(
                        "notebook_edit: cell_index {idx} out of range"
                    )));
                };
                target["source"] = serde_json::Value::String(source);
            }
            "insert" => {
                let source = match require_string(&call.arguments, "source") {
                    Ok(s) => s,
                    Err(e) => return ToolResult::Err(e),
                };
                let cell_type = call
                    .arguments
                    .get("cell_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("code");
                if cell_type != "code" && cell_type != "markdown" {
                    return ToolResult::Err(ToolError::SchemaInvalid(format!(
                        "notebook_edit: cell_type must be code|markdown, got {cell_type}"
                    )));
                }
                let new_cell = serde_json::json!({
                    "cell_type": cell_type,
                    "metadata": {},
                    "source": source,
                    "outputs": [],
                    "execution_count": serde_json::Value::Null,
                });
                if idx > cells.len() {
                    return ToolResult::Err(ToolError::Other(format!(
                        "notebook_edit: insert index {idx} out of range (len={})",
                        cells.len()
                    )));
                }
                cells.insert(idx, new_cell);
            }
            "delete" => {
                if idx >= cells.len() {
                    return ToolResult::Err(ToolError::Other(format!(
                        "notebook_edit: delete index {idx} out of range (len={})",
                        cells.len()
                    )));
                }
                cells.remove(idx);
            }
            other => {
                return ToolResult::Err(ToolError::SchemaInvalid(format!(
                    "notebook_edit: mode must be edit|insert|delete, got {other}"
                )));
            }
        }
        let serialized = match serde_json::to_string_pretty(&nb) {
            Ok(s) => s,
            Err(e) => {
                return ToolResult::Err(ToolError::Other(format!(
                    "notebook_edit: serialize failed: {e}"
                )));
            }
        };
        match tokio::fs::write(&absolute, serialized.as_bytes()).await {
            Ok(()) => ToolResult::text(format!("{mode} cell {idx} in {}", absolute.display())),
            Err(e) => ToolResult::Err(ToolError::Other(format!(
                "notebook_edit: write failed ({}): {e}",
                absolute.display()
            ))),
        }
    }
}
