//! [`MockToolDispatcher`] — records tool calls and returns canned results.
//!
//! Used for testing orchestration layers, replay harnesses, and any code
//! that dispatches [`ToolCall`]s without running real side-effects.
//!
//! # Example
//!
//! ```
//! use roko_core::tool::{ToolCall, ToolResult};
//! use roko_std::tool::mock_dispatcher::MockToolDispatcher;
//!
//! let mut mock = MockToolDispatcher::new();
//! mock.expect("read_file", ToolResult::text("fn main() {}"));
//!
//! let call = ToolCall::new("c1", "read_file", serde_json::json!({"path": "main.rs"}));
//! let result = mock.dispatch(call);
//! assert!(result.is_ok());
//! mock.assert_called("read_file", 1);
//! ```

use parking_lot::Mutex;
use std::collections::HashMap;

use roko_core::tool::{ToolCall, ToolError, ToolResult};

/// A mock dispatcher that records every [`ToolCall`] and returns
/// pre-configured canned [`ToolResult`]s.
///
/// Thread-safe: interior mutability via [`parking_lot::Mutex`].
#[derive(Debug, Default)]
pub struct MockToolDispatcher {
    /// Canned results keyed by tool name. When multiple results are
    /// registered for the same name they are returned FIFO; when the
    /// queue is exhausted the last result is reused.
    expectations: Mutex<HashMap<String, Vec<ToolResult>>>,
    /// Every call dispatched, in order.
    recorded: Mutex<Vec<ToolCall>>,
}

impl MockToolDispatcher {
    /// Construct an empty mock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a canned result for `tool_name`.
    ///
    /// If called multiple times for the same name the results are queued
    /// and returned in FIFO order. Once the queue is exhausted the last
    /// result is cloned for every subsequent call.
    pub fn expect(&self, tool_name: &str, result: ToolResult) {
        self.expectations
            .lock()
            .entry(tool_name.to_owned())
            .or_default()
            .push(result);
    }

    /// Dispatch a [`ToolCall`]: record it and return the canned result.
    ///
    /// If no expectation was registered for `call.name` the dispatcher
    /// returns a [`ToolError::Other`] describing the missing expectation.
    pub fn dispatch(&self, call: ToolCall) -> ToolResult {
        let result = {
            let mut exps = self.expectations.lock();
            match exps.get_mut(&call.name) {
                Some(queue) if queue.len() > 1 => queue.remove(0),
                Some(queue) if !queue.is_empty() => queue[0].clone(),
                _ => ToolResult::err(ToolError::Other(format!(
                    "MockToolDispatcher: no expectation registered for `{}`",
                    call.name
                ))),
            }
        };
        self.recorded.lock().push(call);
        result
    }

    /// Return a snapshot of all recorded calls.
    #[must_use]
    pub fn calls(&self) -> Vec<ToolCall> {
        self.recorded.lock().clone()
    }

    /// Assert that `tool_name` was called exactly `times` times.
    ///
    /// # Panics
    ///
    /// Panics if the count does not match.
    pub fn assert_called(&self, tool_name: &str, times: usize) {
        let actual = self
            .recorded
            .lock()
            .iter()
            .filter(|c| c.name == tool_name)
            .count();
        assert_eq!(
            actual, times,
            "expected `{tool_name}` to be called {times} time(s), but was called {actual} time(s)"
        );
    }

    /// Clear all recorded calls and expectations.
    pub fn reset(&self) {
        self.expectations.lock().clear();
        self.recorded.lock().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_call(name: &str) -> ToolCall {
        ToolCall::new(format!("id-{name}"), name, serde_json::json!({}))
    }

    #[test]
    fn dispatch_returns_canned_result() {
        let mock = MockToolDispatcher::new();
        mock.expect("read_file", ToolResult::text("contents"));
        let result = mock.dispatch(make_call("read_file"));
        assert!(result.is_ok());
        if let ToolResult::Ok { content, .. } = result {
            assert_eq!(content, "contents");
        }
    }

    #[test]
    fn dispatch_without_expectation_returns_error() {
        let mock = MockToolDispatcher::new();
        let result = mock.dispatch(make_call("unknown_tool"));
        assert!(result.is_err());
    }

    #[test]
    fn calls_records_dispatch_order() {
        let mock = MockToolDispatcher::new();
        mock.expect("bash", ToolResult::text("ok"));
        mock.expect("grep", ToolResult::text("ok"));
        mock.dispatch(make_call("bash"));
        mock.dispatch(make_call("grep"));
        mock.dispatch(make_call("bash"));
        let calls = mock.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[1].name, "grep");
        assert_eq!(calls[2].name, "bash");
    }

    #[test]
    fn assert_called_passes_on_correct_count() {
        let mock = MockToolDispatcher::new();
        mock.expect("ls", ToolResult::text("ok"));
        mock.dispatch(make_call("ls"));
        mock.dispatch(make_call("ls"));
        mock.assert_called("ls", 2);
    }

    #[test]
    #[should_panic(expected = "expected `ls` to be called 5 time(s)")]
    fn assert_called_panics_on_wrong_count() {
        let mock = MockToolDispatcher::new();
        mock.expect("ls", ToolResult::text("ok"));
        mock.dispatch(make_call("ls"));
        mock.assert_called("ls", 5);
    }

    #[test]
    fn reset_clears_everything() {
        let mock = MockToolDispatcher::new();
        mock.expect("bash", ToolResult::text("ok"));
        mock.dispatch(make_call("bash"));
        assert_eq!(mock.calls().len(), 1);
        mock.reset();
        assert!(mock.calls().is_empty());
        // After reset, dispatch without expectation → error.
        let result = mock.dispatch(make_call("bash"));
        assert!(result.is_err());
    }

    #[test]
    fn multiple_expectations_consumed_fifo() {
        let mock = MockToolDispatcher::new();
        mock.expect("bash", ToolResult::text("first"));
        mock.expect("bash", ToolResult::text("second"));
        mock.expect("bash", ToolResult::text("third"));

        let r1 = mock.dispatch(make_call("bash"));
        let r2 = mock.dispatch(make_call("bash"));
        let r3 = mock.dispatch(make_call("bash"));
        // After queue exhausted, last one is reused.
        let r4 = mock.dispatch(make_call("bash"));

        if let ToolResult::Ok { content, .. } = r1 {
            assert_eq!(content, "first");
        }
        if let ToolResult::Ok { content, .. } = r2 {
            assert_eq!(content, "second");
        }
        if let ToolResult::Ok { content, .. } = r3 {
            assert_eq!(content, "third");
        }
        if let ToolResult::Ok { content, .. } = r4 {
            assert_eq!(content, "third");
        }
    }

    #[test]
    fn assert_called_zero_for_uncalled_tool() {
        let mock = MockToolDispatcher::new();
        mock.assert_called("never_called", 0);
    }

    #[test]
    fn structured_result_roundtrips() {
        let mock = MockToolDispatcher::new();
        mock.expect("grep", ToolResult::structured(r#"{"matches":3}"#));
        let result = mock.dispatch(make_call("grep"));
        assert!(result.is_ok());
        if let ToolResult::Ok {
            content,
            is_structured,
            ..
        } = result
        {
            assert!(is_structured);
            assert_eq!(content, r#"{"matches":3}"#);
        }
    }
}
