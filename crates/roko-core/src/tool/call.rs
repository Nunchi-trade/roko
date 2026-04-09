//! Inbound tool invocations ([`ToolCall`]) and outbound results
//! ([`ToolResult`], [`ToolError`], [`Artifact`]).
//!
//! These types are **wire-compatible** with OpenAI / Anthropic "tool use"
//! shapes: every LLM backend ultimately emits a `tool_calls[]` array and
//! consumes a `tool_results[]` array. The canonical types here are the
//! superset that every translator (§36.c) marshals to/from.
//!
//! # Lifecycle
//!
//! 1. LLM emits a `tool_use` block → translator parses into [`ToolCall`]
//! 2. Dispatcher validates args against [`crate::tool::ToolDef::parameters`]
//! 3. Dispatcher looks up a [`crate::tool::ToolHandler`] and invokes it
//! 4. Handler returns [`ToolResult`] (either a content payload + artifacts
//!    or a typed [`ToolError`])
//! 5. Translator serializes [`ToolResult`] back into a `tool_result` block
//!    for the next LLM turn

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Body;

// ─── ToolCall ─────────────────────────────────────────────────────────────

/// An inbound tool invocation, parsed from an LLM response.
///
/// The `id` is whatever stable identifier the backend uses to correlate
/// the call with its result (Anthropic: `id`, OpenAI: `id`, Ollama:
/// `id`). Roko does not generate this — it comes from the LLM.
#[allow(clippy::derive_partial_eq_without_eq)] // arguments: serde_json::Value isn't Eq
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Stable id assigned by the emitting backend; pairs a call with its result.
    pub id: String,
    /// Canonical snake_case tool name (translators normalize on the way in).
    pub name: String,
    /// Tool arguments — validated against `ToolDef.parameters` at dispatch.
    pub arguments: serde_json::Value,
    /// Unix-millis timestamp when the dispatcher received the call.
    pub request_ts_ms: i64,
}

impl ToolCall {
    /// Construct a new tool call, stamping `request_ts_ms` with "now".
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
            request_ts_ms: chrono::Utc::now().timestamp_millis(),
        }
    }

    /// Construct a tool call with an explicit timestamp (useful for tests
    /// and for replaying recorded calls).
    #[must_use]
    pub fn at(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
        request_ts_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
            request_ts_ms,
        }
    }
}

// ─── Artifact ─────────────────────────────────────────────────────────────

/// A named artifact produced alongside a textual tool result.
///
/// Artifacts let a tool return structured or binary output (a generated
/// file, a diff, a rendered image) without inflating the main `content`
/// string. Translators that don't support side-channel artifacts flatten
/// them into the content payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// Logical name (filename, id, label).
    pub name: String,
    /// IANA MIME type (`text/plain`, `application/json`, `image/png`, …).
    pub mime_type: String,
    /// Artifact payload.
    pub body: Body,
}

impl Artifact {
    /// Construct an artifact.
    #[must_use]
    pub fn new(name: impl Into<String>, mime_type: impl Into<String>, body: Body) -> Self {
        Self {
            name: name.into(),
            mime_type: mime_type.into(),
            body,
        }
    }
}

// ─── ToolResult ───────────────────────────────────────────────────────────

/// The result of executing a [`ToolCall`].
///
/// The `Ok` variant mirrors OpenAI / Anthropic "tool result content":
/// a text payload plus zero or more artifacts. `is_structured` signals
/// that `content` is a JSON document (the translator may pass it through
/// without re-wrapping).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ToolResult {
    /// Successful execution with a textual payload and optional artifacts.
    Ok {
        /// Primary text (or JSON, if `is_structured`) returned to the LLM.
        content: String,
        /// If `true`, `content` is a JSON document rather than plain text.
        is_structured: bool,
        /// Side-channel artifacts (files, images, diffs).
        artifacts: Vec<Artifact>,
    },
    /// Execution failed with a typed error.
    Err(ToolError),
}

impl ToolResult {
    /// Construct a plain-text `Ok` result with no artifacts.
    #[must_use]
    pub fn text(content: impl Into<String>) -> Self {
        Self::Ok {
            content: content.into(),
            is_structured: false,
            artifacts: Vec::new(),
        }
    }

    /// Construct a structured (JSON) `Ok` result with no artifacts.
    #[must_use]
    pub fn structured(content: impl Into<String>) -> Self {
        Self::Ok {
            content: content.into(),
            is_structured: true,
            artifacts: Vec::new(),
        }
    }

    /// Construct an `Ok` result with artifacts.
    #[must_use]
    pub fn with_artifacts(content: impl Into<String>, artifacts: Vec<Artifact>) -> Self {
        Self::Ok {
            content: content.into(),
            is_structured: false,
            artifacts,
        }
    }

    /// Wrap a [`ToolError`] as a failing result.
    #[must_use]
    pub const fn err(err: ToolError) -> Self {
        Self::Err(err)
    }

    /// Returns true iff this is an `Ok` result.
    #[must_use]
    pub const fn is_ok(&self) -> bool {
        matches!(self, Self::Ok { .. })
    }

    /// Returns true iff this is an `Err` result.
    #[must_use]
    pub const fn is_err(&self) -> bool {
        matches!(self, Self::Err(_))
    }
}

// ─── ToolError ────────────────────────────────────────────────────────────

/// Typed failure modes for tool dispatch and execution.
///
/// All variants are serializable and round-trip through serde so a
/// failing result can be written to the signal log (§36.44) without
/// loss of structure. Serialization uses externally-tagged enum form so
/// each variant is a self-describing JSON object.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolError {
    /// The role's [`ToolPermissions`](crate::ToolPermissions) did not grant
    /// every flag the tool requires. `String` carries the missing flag(s).
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Arguments failed JSON-schema validation before dispatch.
    #[error("schema validation failed: {0}")]
    SchemaInvalid(String),

    /// The handler panicked (caught via `catch_unwind`).
    #[error("handler panicked: {0}")]
    HandlerPanic(String),

    /// The call exceeded its timeout budget.
    #[error("timed out after {after_ms} ms")]
    Timeout {
        /// How long the call ran before cancellation, in milliseconds.
        after_ms: u64,
    },

    /// A path argument pointed outside the worktree sandbox.
    #[error("path outside worktree: {0}")]
    PathOutsideWorktree(PathBuf),

    /// A shell command was blocked by the bash allowlist/blocklist.
    #[error("command not allowed: {0}")]
    CommandNotAllowed(String),

    /// A network destination was blocked by the domain allowlist.
    #[error("network destination blocked: {0}")]
    NetworkBlocked(String),

    /// The dispatcher's [`crate::tool::CancelToken`] fired before the
    /// handler completed.
    #[error("tool call cancelled")]
    Cancelled,

    /// Catch-all for tool-specific failures.
    #[error("tool failure: {0}")]
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_new_stamps_timestamp() {
        let before = chrono::Utc::now().timestamp_millis();
        let call = ToolCall::new("call-1", "read_file", serde_json::json!({ "path": "x.rs" }));
        let after = chrono::Utc::now().timestamp_millis();
        assert!(call.request_ts_ms >= before);
        assert!(call.request_ts_ms <= after);
        assert_eq!(call.id, "call-1");
        assert_eq!(call.name, "read_file");
    }

    #[test]
    fn tool_call_at_uses_explicit_timestamp() {
        let call = ToolCall::at("x", "bash", serde_json::json!({}), 1_700_000_000_000);
        assert_eq!(call.request_ts_ms, 1_700_000_000_000);
    }

    #[test]
    fn tool_call_serde_roundtrip() {
        let call = ToolCall::at(
            "abc",
            "grep",
            serde_json::json!({"pattern": "foo"}),
            1_700_000_000_000,
        );
        let json = serde_json::to_string(&call).unwrap();
        let decoded: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, call);
    }

    #[test]
    fn tool_result_text_has_no_artifacts() {
        let r = ToolResult::text("hello");
        match r {
            ToolResult::Ok {
                content,
                is_structured,
                artifacts,
            } => {
                assert_eq!(content, "hello");
                assert!(!is_structured);
                assert!(artifacts.is_empty());
            }
            ToolResult::Err(_) => panic!("expected Ok"),
        }
    }

    #[test]
    fn tool_result_structured_sets_flag() {
        let r = ToolResult::structured(r#"{"x":1}"#);
        assert!(matches!(
            r,
            ToolResult::Ok {
                is_structured: true,
                ..
            }
        ));
    }

    #[test]
    fn tool_result_is_ok_is_err() {
        assert!(ToolResult::text("x").is_ok());
        assert!(!ToolResult::text("x").is_err());
        assert!(ToolResult::err(ToolError::Cancelled).is_err());
        assert!(!ToolResult::err(ToolError::Cancelled).is_ok());
    }

    #[test]
    fn artifact_roundtrips_through_serde() {
        let a = Artifact::new("diff.txt", "text/plain", Body::text("hunk"));
        let json = serde_json::to_string(&a).unwrap();
        let decoded: Artifact = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, a);
    }

    #[test]
    fn tool_result_with_artifacts_roundtrips() {
        let artifacts = vec![Artifact::new(
            "a.json",
            "application/json",
            Body::text("{}"),
        )];
        let r = ToolResult::with_artifacts("done", artifacts);
        let json = serde_json::to_string(&r).unwrap();
        let decoded: ToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, r);
    }

    #[test]
    fn tool_error_display_is_useful() {
        let e = ToolError::PermissionDenied("needs write".into());
        assert!(format!("{e}").contains("permission denied"));
        let e = ToolError::Timeout { after_ms: 1_234 };
        assert!(format!("{e}").contains("1234"));
    }

    #[test]
    fn tool_error_serde_roundtrip_each_variant() {
        let errors = [
            ToolError::PermissionDenied("need write".into()),
            ToolError::SchemaInvalid("missing field".into()),
            ToolError::HandlerPanic("unwrap".into()),
            ToolError::Timeout { after_ms: 5_000 },
            ToolError::PathOutsideWorktree(PathBuf::from("/etc/passwd")),
            ToolError::CommandNotAllowed("git push".into()),
            ToolError::NetworkBlocked("evil.example.com".into()),
            ToolError::Cancelled,
            ToolError::Other("boom".into()),
        ];
        for e in errors {
            let json = serde_json::to_string(&e).unwrap();
            let decoded: ToolError = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, e);
        }
    }
}
