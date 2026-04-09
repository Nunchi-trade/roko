//! MCP tool-to-[`ToolDef`] converter (SS36.59).
//!
//! Converts an [`McpToolDef`] received from an MCP server into a
//! [`roko_core::tool::ToolDef`] that plugs directly into any
//! [`ToolRegistry`].

use roko_core::tool::{ToolCategory, ToolConcurrency, ToolDef, ToolPermission, ToolSchema};

use super::client::McpToolDef;

/// Convert an MCP tool definition into a Roko [`ToolDef`].
///
/// The tool name is prefixed with `{server_prefix}__` to avoid collisions
/// when multiple MCP servers expose identically-named tools.
///
/// MCP tools are categorised as [`ToolCategory::Mcp`] and granted
/// read-only permissions by default (callers can upgrade permissions
/// after conversion if needed).
#[must_use]
pub fn mcp_to_tool_def(mcp_tool: &McpToolDef, server_prefix: &str) -> ToolDef {
    let prefixed_name = format!("{server_prefix}__{}", mcp_tool.name);

    let description = mcp_tool
        .description
        .clone()
        .unwrap_or_else(|| format!("MCP tool: {}", mcp_tool.name));

    let schema = mcp_tool
        .input_schema
        .as_ref()
        .map_or_else(ToolSchema::any_object, |v| {
            ToolSchema::from_value(v.clone())
        });

    ToolDef {
        name: prefixed_name,
        description,
        parameters: schema,
        category: ToolCategory::Mcp,
        permission: ToolPermission::read_only(),
        timeout_ms: 60_000,
        concurrency: ToolConcurrency::Parallel,
        idempotent: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_mcp_tool() -> McpToolDef {
        McpToolDef {
            name: "read_file".to_string(),
            description: Some("Read a file from disk".to_string()),
            input_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"}
                },
                "required": ["path"]
            })),
        }
    }

    #[test]
    fn mcp_to_tool_def_prefixes_name() {
        let mcp = sample_mcp_tool();
        let def = mcp_to_tool_def(&mcp, "filesystem");
        assert_eq!(def.name, "filesystem__read_file");
    }

    #[test]
    fn mcp_to_tool_def_copies_description() {
        let mcp = sample_mcp_tool();
        let def = mcp_to_tool_def(&mcp, "fs");
        assert_eq!(def.description, "Read a file from disk");
    }

    #[test]
    fn mcp_to_tool_def_maps_input_schema() {
        let mcp = sample_mcp_tool();
        let def = mcp_to_tool_def(&mcp, "fs");
        let schema = def.parameters.as_value();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["path"].is_object());
    }

    #[test]
    fn mcp_to_tool_def_missing_description_uses_fallback() {
        let mcp = McpToolDef {
            name: "search".to_string(),
            description: None,
            input_schema: None,
        };
        let def = mcp_to_tool_def(&mcp, "code");
        assert_eq!(def.description, "MCP tool: search");
    }

    #[test]
    fn mcp_to_tool_def_missing_schema_uses_any_object() {
        let mcp = McpToolDef {
            name: "list".to_string(),
            description: Some("List items".to_string()),
            input_schema: None,
        };
        let def = mcp_to_tool_def(&mcp, "srv");
        let schema = def.parameters.as_value();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], true);
    }

    #[test]
    fn mcp_to_tool_def_sets_mcp_category() {
        let mcp = sample_mcp_tool();
        let def = mcp_to_tool_def(&mcp, "fs");
        assert_eq!(def.category, ToolCategory::Mcp);
    }

    #[test]
    fn mcp_to_tool_def_defaults() {
        let mcp = sample_mcp_tool();
        let def = mcp_to_tool_def(&mcp, "fs");
        assert_eq!(def.timeout_ms, 60_000);
        assert_eq!(def.concurrency, ToolConcurrency::Parallel);
        assert!(!def.idempotent);
        assert!(def.permission.read);
        assert!(!def.permission.write);
    }
}
