//! Integration tests for §36.b built-in tool handlers (shipped subset).
//!
//! Each handler is exercised through [`ToolHandler::execute`] with a
//! real tempdir as the worktree, so the path-sandbox checks run against
//! a genuine filesystem.

use roko_core::tool::{ToolCall, ToolContext, ToolError, ToolResult};
use roko_std::tool::handler_for;

fn call_with_args(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall::at(format!("{name}-test"), name, args, 0)
}

fn assert_ok_text(result: ToolResult, substr: &str) {
    match result {
        ToolResult::Ok { content, .. } => {
            assert!(
                content.contains(substr),
                "content={content} (want substring {substr})"
            );
        }
        ToolResult::Err(e) => panic!("expected Ok, got Err: {e}"),
    }
}

#[tokio::test]
async fn read_file_returns_contents() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("hello.txt"), "Hello, roko!")
        .await
        .expect("write");

    let handler = handler_for("read_file").expect("read_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args("read_file", serde_json::json!({ "path": "hello.txt" }));
    let result = handler.execute(call, &ctx).await;
    assert_ok_text(result, "Hello, roko!");
}

#[tokio::test]
async fn read_file_rejects_path_escape() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("read_file").expect("read_file");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("read_file", serde_json::json!({ "path": "../etc/passwd" }));
    let result = handler.execute(call, &ctx).await;
    assert!(matches!(
        result,
        ToolResult::Err(ToolError::PathOutsideWorktree(_))
    ));
}

#[tokio::test]
async fn read_file_rejects_missing_path_arg() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("read_file").expect("read_file");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("read_file", serde_json::json!({}));
    let result = handler.execute(call, &ctx).await;
    assert!(matches!(
        result,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn write_file_creates_and_reads_back() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let handler = handler_for("write_file").expect("write_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "write_file",
        serde_json::json!({ "path": "out.txt", "content": "payload" }),
    );
    let result = handler.execute(call, &ctx).await;
    assert_ok_text(result, "wrote 7 bytes");

    let back = tokio::fs::read_to_string(worktree.join("out.txt"))
        .await
        .expect("read");
    assert_eq!(back, "payload");
}

#[tokio::test]
async fn write_file_creates_parent_dirs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let handler = handler_for("write_file").expect("write_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "write_file",
        serde_json::json!({ "path": "a/b/c/out.txt", "content": "x" }),
    );
    let result = handler.execute(call, &ctx).await;
    assert!(matches!(result, ToolResult::Ok { .. }));
    assert!(worktree.join("a/b/c/out.txt").is_file());
}

#[tokio::test]
async fn edit_file_replaces_single_occurrence() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("file.txt"), "alpha beta gamma")
        .await
        .expect("write");
    let handler = handler_for("edit_file").expect("edit_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "edit_file",
        serde_json::json!({
            "path": "file.txt",
            "old_string": "beta",
            "new_string": "delta",
        }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Ok { .. }
    ));
    let after = tokio::fs::read_to_string(worktree.join("file.txt"))
        .await
        .expect("read");
    assert_eq!(after, "alpha delta gamma");
}

#[tokio::test]
async fn edit_file_errors_on_ambiguous_match() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("file.txt"), "x x x")
        .await
        .expect("write");
    let handler = handler_for("edit_file").expect("edit_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "edit_file",
        serde_json::json!({
            "path": "file.txt",
            "old_string": "x",
            "new_string": "y",
        }),
    );
    let result = handler.execute(call, &ctx).await;
    assert!(matches!(result, ToolResult::Err(ToolError::Other(_))));
}

#[tokio::test]
async fn edit_file_replace_all_works() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("file.txt"), "x x x")
        .await
        .expect("write");
    let handler = handler_for("edit_file").expect("edit_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "edit_file",
        serde_json::json!({
            "path": "file.txt",
            "old_string": "x",
            "new_string": "y",
            "replace_all": true,
        }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Ok { .. }
    ));
    let after = tokio::fs::read_to_string(worktree.join("file.txt"))
        .await
        .expect("read");
    assert_eq!(after, "y y y");
}

#[tokio::test]
async fn edit_file_errors_when_old_string_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("file.txt"), "hello")
        .await
        .expect("write");
    let handler = handler_for("edit_file").expect("edit_file");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "edit_file",
        serde_json::json!({
            "path": "file.txt",
            "old_string": "missing",
            "new_string": "x",
        }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
}

#[tokio::test]
async fn ls_lists_entries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("a.txt"), "aaa")
        .await
        .expect("write");
    tokio::fs::create_dir(worktree.join("sub"))
        .await
        .expect("mkdir");
    let handler = handler_for("ls").expect("ls");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args("ls", serde_json::json!({}));
    let result = handler.execute(call, &ctx).await;
    match result {
        ToolResult::Ok { content, .. } => {
            assert!(content.contains("a.txt"), "listing={content}");
            assert!(content.contains("sub"), "listing={content}");
        }
        ToolResult::Err(e) => panic!("ls failed: {e}"),
    }
}

#[tokio::test]
async fn todo_write_accepts_valid_todos() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("todo_write").expect("todo_write");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args(
        "todo_write",
        serde_json::json!({
            "todos": [
                { "content": "write tests", "status": "pending" },
                { "content": "ship feature", "status": "in_progress" },
            ]
        }),
    );
    assert_ok_text(handler.execute(call, &ctx).await, "accepted 2 todo items");
}

#[tokio::test]
async fn todo_write_rejects_non_array() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("todo_write").expect("todo_write");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("todo_write", serde_json::json!({ "todos": "not array" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn todo_write_rejects_item_missing_content() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("todo_write").expect("todo_write");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args(
        "todo_write",
        serde_json::json!({ "todos": [{"status": "pending"}] }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn exit_plan_mode_returns_structured_payload() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("exit_plan_mode").expect("exit_plan_mode");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args(
        "exit_plan_mode",
        serde_json::json!({ "plan": "1. do X\n2. do Y" }),
    );
    match handler.execute(call, &ctx).await {
        ToolResult::Ok {
            content,
            is_structured,
            ..
        } => {
            assert!(is_structured);
            assert!(content.contains("plan_submitted"));
            assert!(content.contains("plan_length_chars"));
        }
        ToolResult::Err(e) => panic!("exit_plan_mode failed: {e}"),
    }
}

#[tokio::test]
async fn exit_plan_mode_rejects_missing_plan() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("exit_plan_mode").expect("exit_plan_mode");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("exit_plan_mode", serde_json::json!({}));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn unknown_handler_returns_none() {
    assert!(handler_for("no_such_tool").is_none());
}

#[tokio::test]
async fn multi_edit_applies_sequential_edits_atomically() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("file.txt"), "alpha beta gamma delta")
        .await
        .expect("write");
    let handler = handler_for("multi_edit").expect("multi_edit");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "multi_edit",
        serde_json::json!({
            "path": "file.txt",
            "edits": [
                { "old_string": "alpha", "new_string": "a" },
                { "old_string": "beta", "new_string": "b" },
                { "old_string": "gamma", "new_string": "g" },
            ]
        }),
    );
    assert_ok_text(handler.execute(call, &ctx).await, "applied 3 edits");
    let after = tokio::fs::read_to_string(worktree.join("file.txt"))
        .await
        .expect("read");
    assert_eq!(after, "a b g delta");
}

#[tokio::test]
async fn multi_edit_aborts_on_failing_edit_preserving_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let original = "alpha beta gamma";
    tokio::fs::write(worktree.join("file.txt"), original)
        .await
        .expect("write");
    let handler = handler_for("multi_edit").expect("multi_edit");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "multi_edit",
        serde_json::json!({
            "path": "file.txt",
            "edits": [
                { "old_string": "alpha", "new_string": "a" },
                { "old_string": "not_here", "new_string": "x" },  // fails
            ]
        }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
    // File should be unchanged.
    let after = tokio::fs::read_to_string(worktree.join("file.txt"))
        .await
        .expect("read");
    assert_eq!(after, original);
}

#[tokio::test]
async fn multi_edit_requires_edits_array() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("multi_edit").expect("multi_edit");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("multi_edit", serde_json::json!({ "path": "x" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn glob_finds_matching_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::create_dir(worktree.join("src"))
        .await
        .expect("mkdir");
    tokio::fs::write(worktree.join("src/main.rs"), "a")
        .await
        .expect("write");
    tokio::fs::write(worktree.join("src/lib.rs"), "b")
        .await
        .expect("write");
    tokio::fs::write(worktree.join("README.md"), "c")
        .await
        .expect("write");

    let handler = handler_for("glob").expect("glob");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args("glob", serde_json::json!({ "pattern": "**/*.rs" }));
    let result = handler.execute(call, &ctx).await;
    match result {
        ToolResult::Ok { content, .. } => {
            assert!(content.contains("src/main.rs"), "got: {content}");
            assert!(content.contains("src/lib.rs"), "got: {content}");
            assert!(!content.contains("README.md"), "got: {content}");
        }
        ToolResult::Err(e) => panic!("glob failed: {e}"),
    }
}

#[tokio::test]
async fn glob_requires_pattern() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("glob").expect("glob");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("glob", serde_json::json!({}));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::SchemaInvalid(_))
    ));
}

#[tokio::test]
async fn bash_executes_and_captures_stdout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("bash").expect("bash");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("bash", serde_json::json!({ "command": "echo hello" }));
    assert_ok_text(handler.execute(call, &ctx).await, "hello");
}

#[tokio::test]
async fn bash_rejects_blocklisted_command() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("bash").expect("bash");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("bash", serde_json::json!({ "command": "rm -rf /" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::CommandNotAllowed(_))
    ));
}

#[tokio::test]
async fn bash_requires_exec_capability() {
    use roko_core::tool::trace::NoopTraceSink;
    use roko_core::tool::{NeverCancel, NoopAuditSink, NoopMetricsSink, ToolPermission};
    use std::sync::Arc;
    use std::time::Duration;
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("bash").expect("bash");
    let ctx = ToolContext::new(
        tmp.path(),
        Duration::from_secs(5),
        ToolPermission::read_only(),
        Arc::new(NoopAuditSink),
        Arc::new(NoopTraceSink),
        Arc::new(NoopMetricsSink),
        Arc::new(NeverCancel),
    );
    let call = call_with_args("bash", serde_json::json!({ "command": "echo x" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::PermissionDenied(_))
    ));
}

#[tokio::test]
async fn bash_nonzero_exit_returns_err() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("bash").expect("bash");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("bash", serde_json::json!({ "command": "false" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
}

#[tokio::test]
async fn run_tests_rejects_unknown_build_system() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let handler = handler_for("run_tests").expect("run_tests");
    let ctx = ToolContext::testing(tmp.path());
    let call = call_with_args("run_tests", serde_json::json!({ "build": "xyz" }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
}

#[tokio::test]
async fn grep_finds_matching_lines() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("a.txt"), "alpha\nbeta\ngamma\n")
        .await
        .expect("write");
    tokio::fs::write(worktree.join("b.txt"), "nothing\nbeta\n")
        .await
        .expect("write");
    let handler = handler_for("grep").expect("grep");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args("grep", serde_json::json!({ "pattern": "beta" }));
    match handler.execute(call, &ctx).await {
        ToolResult::Ok { content, .. } => {
            assert!(content.contains("a.txt:2:beta"), "got: {content}");
            assert!(content.contains("b.txt:2:beta"), "got: {content}");
            assert!(!content.contains("alpha"));
        }
        ToolResult::Err(e) => panic!("grep failed: {e}"),
    }
}

#[tokio::test]
async fn grep_files_with_matches_mode() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("x.rs"), "fn main() {}")
        .await
        .expect("write");
    tokio::fs::write(worktree.join("y.rs"), "fn helper() {}")
        .await
        .expect("write");
    let handler = handler_for("grep").expect("grep");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "grep",
        serde_json::json!({ "pattern": "fn ", "mode": "files_with_matches" }),
    );
    match handler.execute(call, &ctx).await {
        ToolResult::Ok { content, .. } => {
            assert!(content.contains("x.rs"));
            assert!(content.contains("y.rs"));
            // Should not include line numbers in files_with_matches mode.
            assert!(!content.contains(":1:"));
        }
        ToolResult::Err(e) => panic!("grep failed: {e}"),
    }
}

#[tokio::test]
async fn grep_count_mode() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("x.txt"), "a\nb\na\na\n")
        .await
        .expect("write");
    let handler = handler_for("grep").expect("grep");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "grep",
        serde_json::json!({ "pattern": "a", "mode": "count" }),
    );
    match handler.execute(call, &ctx).await {
        ToolResult::Ok { content, .. } => {
            assert!(content.contains("x.txt:3"), "got: {content}");
        }
        ToolResult::Err(e) => panic!("grep failed: {e}"),
    }
}

#[tokio::test]
async fn apply_patch_applies_simple_diff() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("note.txt"), "alpha\nbeta\ngamma\n")
        .await
        .expect("write");
    let handler = handler_for("apply_patch").expect("apply_patch");
    let ctx = ToolContext::testing(worktree);
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,3 +1,3 @@\n alpha\n-beta\n+BETA\n gamma\n";
    let call = call_with_args("apply_patch", serde_json::json!({ "patch": patch }));
    assert_ok_text(handler.execute(call, &ctx).await, "patched");
    let after = tokio::fs::read_to_string(worktree.join("note.txt"))
        .await
        .expect("read");
    assert_eq!(after, "alpha\nBETA\ngamma\n");
}

#[tokio::test]
async fn notebook_edit_edits_existing_cell() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let nb = serde_json::json!({
        "cells": [
            { "cell_type": "code", "source": "print(1)", "metadata": {}, "outputs": [], "execution_count": serde_json::Value::Null },
            { "cell_type": "code", "source": "print(2)", "metadata": {}, "outputs": [], "execution_count": serde_json::Value::Null },
        ],
        "metadata": {},
        "nbformat": 4,
        "nbformat_minor": 5,
    });
    tokio::fs::write(
        worktree.join("nb.ipynb"),
        serde_json::to_string(&nb).expect("ser"),
    )
    .await
    .expect("write");
    let handler = handler_for("notebook_edit").expect("notebook_edit");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "notebook_edit",
        serde_json::json!({
            "path": "nb.ipynb",
            "cell_index": 0,
            "mode": "edit",
            "source": "print('hi')",
        }),
    );
    assert_ok_text(handler.execute(call, &ctx).await, "edit cell 0");
    let after: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(worktree.join("nb.ipynb"))
            .await
            .expect("read"),
    )
    .expect("json");
    assert_eq!(after["cells"][0]["source"], "print('hi')");
}

#[tokio::test]
async fn notebook_edit_inserts_and_deletes_cells() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let nb = serde_json::json!({"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 5});
    tokio::fs::write(
        worktree.join("nb.ipynb"),
        serde_json::to_string(&nb).expect("ser"),
    )
    .await
    .expect("write");
    let handler = handler_for("notebook_edit").expect("notebook_edit");
    let ctx = ToolContext::testing(worktree);
    // Insert.
    let call = call_with_args(
        "notebook_edit",
        serde_json::json!({
            "path": "nb.ipynb", "cell_index": 0, "mode": "insert", "source": "x = 1", "cell_type": "code"
        }),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Ok { .. }
    ));
    // Delete.
    let call = call_with_args(
        "notebook_edit",
        serde_json::json!({"path": "nb.ipynb", "cell_index": 0, "mode": "delete"}),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Ok { .. }
    ));
    let after: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(worktree.join("nb.ipynb"))
            .await
            .expect("read"),
    )
    .expect("json");
    assert_eq!(after["cells"].as_array().expect("arr").len(), 0);
}

#[tokio::test]
async fn notebook_edit_rejects_oob_index() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    let nb = serde_json::json!({"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 5});
    tokio::fs::write(
        worktree.join("nb.ipynb"),
        serde_json::to_string(&nb).expect("ser"),
    )
    .await
    .expect("write");
    let handler = handler_for("notebook_edit").expect("notebook_edit");
    let ctx = ToolContext::testing(worktree);
    let call = call_with_args(
        "notebook_edit",
        serde_json::json!({"path": "nb.ipynb", "cell_index": 5, "mode": "edit", "source": "x"}),
    );
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
}

#[tokio::test]
async fn apply_patch_rejects_mismatched_context() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path();
    tokio::fs::write(worktree.join("note.txt"), "alpha\nbeta\n")
        .await
        .expect("write");
    let handler = handler_for("apply_patch").expect("apply_patch");
    let ctx = ToolContext::testing(worktree);
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n WRONG\n-beta\n+B\n";
    let call = call_with_args("apply_patch", serde_json::json!({ "patch": patch }));
    assert!(matches!(
        handler.execute(call, &ctx).await,
        ToolResult::Err(ToolError::Other(_))
    ));
    // Verify file unchanged.
    let after = tokio::fs::read_to_string(worktree.join("note.txt"))
        .await
        .expect("read");
    assert_eq!(after, "alpha\nbeta\n");
}
