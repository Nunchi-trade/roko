//! Golden-file tests for the 16 built-in tool definitions.
//!
//! Each test verifies that a tool's [`ToolDef`] has:
//! - A non-empty canonical name
//! - A non-empty description
//! - A schema whose top-level JSON value has `"type": "object"`
//!
//! These are definition-level golden tests — they do NOT execute handlers.

use roko_core::tool::ToolRegistry;
use roko_std::{ROKO_BUILTIN_TOOLS, StaticToolRegistry};

/// Helper: look up a tool by name and assert its definition is well-formed.
fn assert_golden(name: &str) {
    let reg = StaticToolRegistry::new();
    let def = reg
        .get(name)
        .unwrap_or_else(|| panic!("tool `{name}` must exist in StaticToolRegistry"));

    // Name is non-empty and matches the lookup key.
    assert!(!def.name.is_empty(), "`{name}` has empty name");
    assert_eq!(def.name, name, "name mismatch for `{name}`");

    // Description is non-empty.
    assert!(
        !def.description.is_empty(),
        "`{name}` has empty description"
    );

    // Schema has `"type": "object"`.
    let schema_val = def.parameters.as_value();
    assert!(
        schema_val.is_object(),
        "`{name}` schema is not a JSON object"
    );
    assert_eq!(
        schema_val.get("type").and_then(serde_json::Value::as_str),
        Some("object"),
        "`{name}` schema missing `\"type\": \"object\"`"
    );
}

// ── One test per tool ──────────────────────────────────────────────────

#[test]
fn golden_read_file() {
    assert_golden("read_file");
}

#[test]
fn golden_write_file() {
    assert_golden("write_file");
}

#[test]
fn golden_edit_file() {
    assert_golden("edit_file");
}

#[test]
fn golden_multi_edit() {
    assert_golden("multi_edit");
}

#[test]
fn golden_glob() {
    assert_golden("glob");
}

#[test]
fn golden_grep() {
    assert_golden("grep");
}

#[test]
fn golden_bash() {
    assert_golden("bash");
}

#[test]
fn golden_ls() {
    assert_golden("ls");
}

#[test]
fn golden_web_fetch() {
    assert_golden("web_fetch");
}

#[test]
fn golden_web_search() {
    assert_golden("web_search");
}

#[test]
fn golden_notebook_edit() {
    assert_golden("notebook_edit");
}

#[test]
fn golden_todo_write() {
    assert_golden("todo_write");
}

#[test]
fn golden_task() {
    assert_golden("task");
}

#[test]
fn golden_exit_plan_mode() {
    assert_golden("exit_plan_mode");
}

#[test]
fn golden_apply_patch() {
    assert_golden("apply_patch");
}

#[test]
fn golden_run_tests() {
    assert_golden("run_tests");
}

// ── Aggregate checks ──────────────────────────────────────────────────

#[test]
fn golden_all_16_present() {
    assert_eq!(ROKO_BUILTIN_TOOLS.len(), 16);
    let expected = [
        "read_file",
        "write_file",
        "edit_file",
        "multi_edit",
        "glob",
        "grep",
        "bash",
        "ls",
        "web_fetch",
        "web_search",
        "notebook_edit",
        "todo_write",
        "task",
        "exit_plan_mode",
        "apply_patch",
        "run_tests",
    ];
    for name in expected {
        assert_golden(name);
    }
}
