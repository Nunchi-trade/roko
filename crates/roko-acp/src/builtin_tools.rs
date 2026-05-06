//! ACP builtin tool definitions.
//!
//! Returns [`ToolDef`] entries for the 8 core tools exposed to ACP sessions.
//! These are schema-only definitions — execution is handled elsewhere.

use roko_core::tool::{ToolCategory, ToolConcurrency, ToolDef, ToolPermission, ToolSchema};

/// Returns the 8 builtin tool definitions for ACP sessions.
#[must_use]
pub fn acp_builtin_tools() -> Vec<ToolDef> {
    vec![
        read_file(),
        write_file(),
        edit_file(),
        glob(),
        grep(),
        bash(),
        ls(),
        web_fetch(),
    ]
}

fn read_file() -> ToolDef {
    ToolDef::new(
        "read_file",
        "Read the contents of a UTF-8 file.",
        ToolCategory::Read,
        ToolPermission::read_only(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute or relative file path to read." },
            "lines": { "type": "integer", "description": "Maximum number of lines to return." }
        },
        "required": ["path"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Parallel)
    .with_idempotent(true)
    .with_timeout_ms(30_000)
}

fn write_file() -> ToolDef {
    ToolDef::new(
        "write_file",
        "Write content to a file, creating or overwriting it.",
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "Absolute or relative file path to write." },
            "content": { "type": "string", "description": "Full file content to write." }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_timeout_ms(30_000)
}

fn edit_file() -> ToolDef {
    ToolDef::new(
        "edit_file",
        "Perform a surgical string replacement in a file.",
        ToolCategory::Write,
        ToolPermission::writes(),
    )
    .with_parameters(ToolSchema::from_value(serde_json::json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "File path to edit." },
            "old_string": { "type": "string", "description": "Exact text to find and replace." },
            "new_string": { "type": "string", "description": "Replacement text." }
        },
        "required": ["path", "old_string", "new_string"],
        "additionalProperties": false
    })))
    .with_concurrency(ToolConcurrency::Serial)
    .with_timeout_ms(30_000)
}

fn glob() -> ToolDef {
    ToolDef::new("glob", "Find files matching a glob pattern.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match (e.g. \"**/*.rs\")." },
                "path": { "type": "string", "description": "Directory to search in. Defaults to working directory." }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(30_000)
}

fn grep() -> ToolDef {
    ToolDef::new("grep", "Search file contents using a regex pattern.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression pattern to search for." },
                "path": { "type": "string", "description": "File or directory to search in." },
                "type": { "type": "string", "description": "File type filter (e.g. \"rs\", \"py\", \"js\")." }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(60_000)
}

fn bash() -> ToolDef {
    ToolDef::new("bash", "Execute a shell command and return its output.", ToolCategory::Exec, ToolPermission::executes())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute." },
                "timeout": { "type": "integer", "description": "Timeout in milliseconds (default 120000)." }
            },
            "required": ["command"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Serial)
        .with_timeout_ms(120_000)
}

fn ls() -> ToolDef {
    ToolDef::new("ls", "List directory contents.", ToolCategory::Read, ToolPermission::read_only())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Directory path to list. Defaults to working directory." }
            },
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_idempotent(true)
        .with_timeout_ms(15_000)
}

fn web_fetch() -> ToolDef {
    ToolDef::new("web_fetch", "Fetch content from a URL and process it.", ToolCategory::Network, ToolPermission::networked())
        .with_parameters(ToolSchema::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "URL to fetch content from." },
                "prompt": { "type": "string", "description": "Prompt describing what information to extract." }
            },
            "required": ["url", "prompt"],
            "additionalProperties": false
        })))
        .with_concurrency(ToolConcurrency::Parallel)
        .with_timeout_ms(60_000)
}
