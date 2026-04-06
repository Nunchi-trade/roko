//! JSON-RPC error code mapping.
//!
//! # §41.14 -- RPC error codes
//!
//! Maps [`RokoError`](super::RokoError) variants to JSON-RPC 2.0 error codes
//! for wire-level transport. Standard codes follow the JSON-RPC spec; custom
//! codes in the -32000..-32099 range encode Roko-specific failure categories.

use super::RokoError;

/// JSON-RPC error with code, message, and optional structured data.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RpcError {
    /// Numeric JSON-RPC error code.
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured error data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// Standard JSON-RPC 2.0 error codes.

/// Parse error -- invalid JSON was received.
pub const PARSE_ERROR: i64 = -32700;
/// Invalid request -- the JSON sent is not a valid request object.
pub const INVALID_REQUEST: i64 = -32600;
/// Method not found -- the method does not exist or is not available.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Invalid params -- invalid method parameters.
pub const INVALID_PARAMS: i64 = -32602;
/// Internal error -- internal JSON-RPC error.
pub const INTERNAL_ERROR: i64 = -32603;

// Custom Roko error codes (-32000 .. -32099).

/// Agent backend failure (Claude/Codex/etc.).
pub const AGENT_FAILURE: i64 = -32000;
/// Gate verification failure.
pub const GATE_FAILURE: i64 = -32001;
/// Operation timed out.
pub const TIMEOUT: i64 = -32002;
/// Budget exceeded.
pub const BUDGET_EXCEEDED: i64 = -32003;

impl RpcError {
    /// Create a new RPC error.
    #[must_use]
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    /// Create a new RPC error with structured data.
    #[must_use]
    pub fn with_data(
        code: i64,
        message: impl Into<String>,
        data: serde_json::Value,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            data: Some(data),
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

/// Convert a [`RokoError`] into a wire-level [`RpcError`].
///
/// # Code mapping
///
/// | `RokoError` variant | Code | Category |
/// |---|---|---|
/// | `Json` | -32700 | Parse error |
/// | `Invalid`, `User` | -32602 | Invalid params |
/// | `NotFound` | -32601 | Method not found |
/// | `Agent` | -32000 | Agent failure |
/// | `Gate`, `Rejected` | -32001 | Gate failure |
/// | `Timeout` | -32002 | Timeout |
/// | `BudgetExceeded` | -32003 | Budget exceeded |
/// | everything else | -32603 | Internal error |
#[must_use]
pub fn to_rpc_error(error: &RokoError) -> RpcError {
    let (code, message) = match error {
        // Standard: parse error
        RokoError::Json(_) => (PARSE_ERROR, format!("{error}")),

        // Standard: invalid params
        RokoError::Invalid(_) | RokoError::User(_) => {
            (INVALID_PARAMS, format!("{error}"))
        }

        // Standard: method not found (signal lookup miss)
        RokoError::NotFound(_) => (METHOD_NOT_FOUND, format!("{error}")),

        // Custom: agent failure
        RokoError::Agent { .. } => (AGENT_FAILURE, format!("{error}")),

        // Custom: gate failure
        RokoError::Gate { .. } | RokoError::Rejected(_) => {
            (GATE_FAILURE, format!("{error}"))
        }

        // Custom: timeout
        RokoError::Timeout { .. } => (TIMEOUT, format!("{error}")),

        // Custom: budget exceeded
        RokoError::BudgetExceeded { .. } => (BUDGET_EXCEEDED, format!("{error}")),

        // Everything else: internal error
        _ => (INTERNAL_ERROR, format!("{error}")),
    };

    RpcError::new(code, message)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_json_error_maps_to_parse_error() {
        let err = RokoError::Json(
            serde_json::from_str::<serde_json::Value>("{ bad").unwrap_err(),
        );
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, PARSE_ERROR);
    }

    #[test]
    fn rpc_invalid_maps_to_invalid_params() {
        let err = RokoError::invalid("bad field");
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, INVALID_PARAMS);
    }

    #[test]
    fn rpc_user_error_maps_to_invalid_params() {
        let err = RokoError::user("please fix");
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, INVALID_PARAMS);
    }

    #[test]
    fn rpc_not_found_maps_to_method_not_found() {
        let err = RokoError::NotFound(crate::ContentHash([0; 32]));
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, METHOD_NOT_FOUND);
    }

    #[test]
    fn rpc_agent_maps_to_agent_failure() {
        let err = RokoError::agent("claude", "rate-limited");
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, AGENT_FAILURE);
        assert!(rpc.message.contains("claude"));
    }

    #[test]
    fn rpc_gate_maps_to_gate_failure() {
        let err = RokoError::gate("compile", "failed");
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, GATE_FAILURE);
    }

    #[test]
    fn rpc_rejected_maps_to_gate_failure() {
        let err = RokoError::Rejected("nope".into());
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, GATE_FAILURE);
    }

    #[test]
    fn rpc_timeout_maps_to_timeout() {
        let err = RokoError::timeout("compile", 30_000);
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, TIMEOUT);
        assert!(rpc.message.contains("30000"));
    }

    #[test]
    fn rpc_budget_exceeded_maps_to_budget_code() {
        let err = RokoError::BudgetExceeded {
            dimension: "tokens",
            used: 100,
            limit: 50,
        };
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, BUDGET_EXCEEDED);
    }

    #[test]
    fn rpc_transport_maps_to_internal() {
        let err = RokoError::transport("dns failure");
        let rpc = to_rpc_error(&err);
        assert_eq!(rpc.code, INTERNAL_ERROR);
    }

    #[test]
    fn rpc_error_display() {
        let rpc = RpcError::new(-32603, "something broke");
        let msg = format!("{rpc}");
        assert!(msg.contains("-32603"));
        assert!(msg.contains("something broke"));
    }

    #[test]
    fn rpc_error_with_data() {
        let rpc = RpcError::with_data(
            -32000,
            "agent down",
            serde_json::json!({"backend": "claude"}),
        );
        assert_eq!(rpc.code, -32000);
        assert!(rpc.data.is_some());
        let data = rpc.data.unwrap();
        assert_eq!(data["backend"], "claude");
    }

    #[test]
    fn rpc_error_json_roundtrip() {
        let original = RpcError::with_data(
            AGENT_FAILURE,
            "agent error",
            serde_json::json!({"retry": true}),
        );
        let json = serde_json::to_string(&original).unwrap();
        let back: RpcError = serde_json::from_str(&json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn rpc_error_without_data_omits_field() {
        let rpc = RpcError::new(-32603, "internal");
        let json = serde_json::to_string(&rpc).unwrap();
        assert!(!json.contains("data"));
    }
}
