//! JSONL replay test for tool calls.
//!
//! [`ReplayEntry`] captures a tool name, input arguments, and expected
//! output. [`replay_from_entries`] replays the entries against a
//! [`MockToolDispatcher`] and verifies outputs match.

use roko_core::tool::{ToolCall, ToolResult};
use roko_std::MockToolDispatcher;
use serde::{Deserialize, Serialize};

/// A single replay entry: tool name, input arguments, expected output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayEntry {
    /// Canonical tool name.
    pub tool_name: String,
    /// Input arguments as a JSON value.
    pub input: serde_json::Value,
    /// Expected text content of the tool result.
    pub expected_output: String,
}

impl ReplayEntry {
    /// Construct a replay entry.
    #[must_use]
    fn new(tool_name: &str, input: serde_json::Value, expected_output: &str) -> Self {
        Self {
            tool_name: tool_name.to_owned(),
            input,
            expected_output: expected_output.to_owned(),
        }
    }
}

/// Replay a sequence of [`ReplayEntry`] values against a
/// [`MockToolDispatcher`], verifying that each dispatched result matches
/// the expected output.
///
/// All expectations are registered up-front (FIFO per tool name) before
/// any calls are dispatched, so repeated calls to the same tool consume
/// results in the expected order.
///
/// # Panics
///
/// Panics if any dispatched result does not match its expected output.
fn replay_from_entries(entries: &[ReplayEntry], dispatcher: &MockToolDispatcher) {
    // Phase 1: register all expectations up-front.
    for entry in entries {
        dispatcher.expect(&entry.tool_name, ToolResult::text(&entry.expected_output));
    }

    // Phase 2: dispatch each call and verify results.
    for (i, entry) in entries.iter().enumerate() {
        let call = ToolCall::new(format!("replay-{i}"), &entry.tool_name, entry.input.clone());
        let result = dispatcher.dispatch(call);
        match result {
            ToolResult::Ok { content, .. } => {
                assert_eq!(
                    content, entry.expected_output,
                    "replay entry {i} (`{}`): output mismatch",
                    entry.tool_name
                );
            }
            ToolResult::Err(e) => {
                panic!(
                    "replay entry {i} (`{}`): expected Ok, got Err({e})",
                    entry.tool_name
                );
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[test]
fn replay_single_entry() {
    let mock = MockToolDispatcher::new();
    let entries = vec![ReplayEntry::new(
        "read_file",
        serde_json::json!({"path": "main.rs"}),
        "fn main() {}",
    )];
    replay_from_entries(&entries, &mock);
    mock.assert_called("read_file", 1);
}

#[test]
fn replay_multiple_entries() {
    let mock = MockToolDispatcher::new();
    let entries = vec![
        ReplayEntry::new(
            "read_file",
            serde_json::json!({"path": "lib.rs"}),
            "pub fn greet() {}",
        ),
        ReplayEntry::new(
            "bash",
            serde_json::json!({"command": "cargo test"}),
            "test result: ok. 0 passed",
        ),
        ReplayEntry::new(
            "grep",
            serde_json::json!({"pattern": "fn main", "path": "."}),
            "src/main.rs:1:fn main() {}",
        ),
    ];
    replay_from_entries(&entries, &mock);
    mock.assert_called("read_file", 1);
    mock.assert_called("bash", 1);
    mock.assert_called("grep", 1);
    assert_eq!(mock.calls().len(), 3);
}

#[test]
fn replay_preserves_call_order() {
    let mock = MockToolDispatcher::new();
    let entries = vec![
        ReplayEntry::new(
            "glob",
            serde_json::json!({"pattern": "*.rs"}),
            "main.rs\nlib.rs",
        ),
        ReplayEntry::new("ls", serde_json::json!({"path": "."}), "src/\nCargo.toml"),
        ReplayEntry::new("read_file", serde_json::json!({"path": "a.rs"}), "// a"),
    ];
    replay_from_entries(&entries, &mock);
    let calls = mock.calls();
    assert_eq!(calls[0].name, "glob");
    assert_eq!(calls[1].name, "ls");
    assert_eq!(calls[2].name, "read_file");
}

#[test]
fn replay_entry_serializes_to_jsonl() {
    let entry = ReplayEntry::new("bash", serde_json::json!({"command": "echo hi"}), "hi");
    let json = serde_json::to_string(&entry).expect("serialize");
    let decoded: ReplayEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.tool_name, "bash");
    assert_eq!(decoded.expected_output, "hi");
}

#[test]
fn replay_multiple_jsonl_lines() {
    // Simulate reading a JSONL buffer line by line.
    let lines = [
        r#"{"tool_name":"read_file","input":{"path":"x.rs"},"expected_output":"code"}"#,
        r#"{"tool_name":"bash","input":{"command":"ls"},"expected_output":"Cargo.toml"}"#,
    ];
    let entries: Vec<ReplayEntry> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("parse JSONL line"))
        .collect();
    assert_eq!(entries.len(), 2);

    let mock = MockToolDispatcher::new();
    replay_from_entries(&entries, &mock);
    mock.assert_called("read_file", 1);
    mock.assert_called("bash", 1);
}

#[test]
fn replay_same_tool_multiple_times() {
    let mock = MockToolDispatcher::new();
    let entries = vec![
        ReplayEntry::new("read_file", serde_json::json!({"path": "a.rs"}), "aaa"),
        ReplayEntry::new("read_file", serde_json::json!({"path": "b.rs"}), "bbb"),
    ];
    replay_from_entries(&entries, &mock);
    mock.assert_called("read_file", 2);
}
