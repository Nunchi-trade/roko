//! Error types for the Roko kernel.
//!
//! `RokoError` is the single error type returned by all kernel-level operations.
//! It uses `thiserror` and is `#[non_exhaustive]` so new variants can be added
//! without breaking downstream impls.
//!
//! # Philosophy
//!
//! Anti-pattern: using `anyhow::Error` in public APIs erases error structure
//! and prevents callers from responding intelligently to specific failures.
//! Roko exposes typed errors at crate boundaries.
//!
//! # §41.a — Unified error taxonomy
//!
//! `RokoError` is the root enum spanning every subsystem. Subsystems return
//! their own strongly-typed errors internally, but at crate boundaries they
//! convert into `RokoError`. The [`RokoError::kind`] method returns a stable
//! [`ErrorKind`] discriminant suitable for metrics labels and retry tables.
//! [`RokoError::is_transient`] provides a first-cut classification for retry
//! policies (see §41.6 for the full policy layer).
//!
//! # Submodules
//!
//! - [`retry`] — [`RetryPolicy`](retry::RetryPolicy) and
//!   [`CircuitBreaker`](retry::CircuitBreaker) for transient-error handling.
//! - [`rpc`] — JSON-RPC error code mapping via [`RpcError`](rpc::RpcError).

pub mod retry;
pub mod rpc;

use thiserror::Error;

/// A `Result` alias with `RokoError` as the default error type.
pub type Result<T, E = RokoError> = std::result::Result<T, E>;

/// Kernel-level error type. Returned by trait methods and core operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RokoError {
    /// A substrate failed to store or retrieve a signal.
    #[error("substrate error: {0}")]
    Substrate(String),

    /// Signal not found in the queried substrate.
    #[error("signal not found: {0}")]
    NotFound(crate::ContentHash),

    /// Failed to encode a signal's body (serde error).
    #[error("body encode error: {0}")]
    BodyEncode(String),

    /// Failed to decode a signal's body (wrong type or malformed).
    #[error("body decode error: {0}")]
    BodyDecode(String),

    /// A gate rejected its input with a reason.
    #[error("gate rejected: {0}")]
    Rejected(String),

    /// A budget was exceeded during composition or routing.
    #[error("budget exceeded: {dimension} (used {used}, limit {limit})")]
    BudgetExceeded {
        /// Which budget dimension was exceeded (e.g. "tokens", "signals", "bytes").
        dimension: &'static str,
        /// Amount actually consumed.
        used: usize,
        /// Budget limit.
        limit: usize,
    },

    /// An I/O error from a substrate backed by disk/network.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A JSON serialization/deserialization error not specific to a body.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Generic invalid-input error with human-readable detail.
    #[error("invalid input: {0}")]
    Invalid(String),

    // §41.1 — cross-subsystem variants
    /// Planning/plan-discovery failure.
    #[error("planning error: {0}")]
    Planning(String),

    /// Agent backend failure (Claude/Codex/Cursor/Ollama/OpenAI).
    #[error("agent error ({backend}): {message}")]
    Agent {
        /// Backend identifier (e.g. "claude", "codex", "ollama").
        backend: String,
        /// Human-readable error detail from the backend.
        message: String,
    },

    /// Gate rejected with structured verdict detail (superset of existing `Rejected`).
    #[error("gate error ({gate}): {message}")]
    Gate {
        /// Gate identifier (e.g. "compile", "clippy", "test").
        gate: String,
        /// Verdict detail from the gate.
        message: String,
    },

    /// Tool invocation failure.
    #[error("tool error ({tool}): {message}")]
    Tool {
        /// Tool identifier (e.g. `read_file`, `shell`, `search`).
        tool: String,
        /// Human-readable error detail from the tool handler.
        message: String,
    },

    /// Chain / on-chain interaction failure.
    #[error("chain error: {0}")]
    Chain(String),

    /// Configuration load / parse error.
    #[error("config error: {0}")]
    Config(String),

    /// Transport (HTTP / RPC / WS) failure.
    #[error("transport error: {0}")]
    Transport(String),

    /// User-facing / user-input validation error (not a bug; user must fix).
    #[error("user error: {0}")]
    User(String),

    /// Timeout.
    #[error("timeout after {timeout_ms}ms: {operation}")]
    Timeout {
        /// The operation that timed out.
        operation: String,
        /// The elapsed time budget in milliseconds.
        timeout_ms: u64,
    },

    /// Cancelled by user / token.
    #[error("cancelled: {0}")]
    Cancelled(String),

    /// Permission denied by safety layer.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Rate limit exceeded.
    #[error("rate limit: {0}")]
    RateLimited(String),
}

impl RokoError {
    /// Construct a body-encode error from any displayable cause.
    #[must_use]
    pub fn body_encode(err: impl std::fmt::Display) -> Self {
        Self::BodyEncode(err.to_string())
    }

    /// Construct a body-decode error from any displayable cause.
    #[must_use]
    pub fn body_decode(err: impl std::fmt::Display) -> Self {
        Self::BodyDecode(err.to_string())
    }

    /// Construct a substrate error from any displayable cause.
    #[must_use]
    pub fn substrate(err: impl std::fmt::Display) -> Self {
        Self::Substrate(err.to_string())
    }

    /// Construct an invalid-input error.
    #[must_use]
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::Invalid(msg.into())
    }

    // §41.1 — constructor helpers for cross-subsystem variants

    /// Construct an agent-backend error.
    #[must_use]
    pub fn agent(backend: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Agent {
            backend: backend.into(),
            message: message.into(),
        }
    }

    /// Construct a gate error with a structured verdict message.
    #[must_use]
    pub fn gate(gate: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Gate {
            gate: gate.into(),
            message: message.into(),
        }
    }

    /// Construct a tool invocation error.
    #[must_use]
    pub fn tool(tool: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Tool {
            tool: tool.into(),
            message: message.into(),
        }
    }

    /// Construct a chain/on-chain interaction error from any displayable cause.
    #[must_use]
    pub fn chain(err: impl std::fmt::Display) -> Self {
        Self::Chain(err.to_string())
    }

    /// Construct a configuration error from any displayable cause.
    #[must_use]
    pub fn config(err: impl std::fmt::Display) -> Self {
        Self::Config(err.to_string())
    }

    /// Construct a transport error (HTTP/RPC/WS) from any displayable cause.
    #[must_use]
    pub fn transport(err: impl std::fmt::Display) -> Self {
        Self::Transport(err.to_string())
    }

    /// Construct a user-facing / validation error.
    #[must_use]
    pub fn user(msg: impl Into<String>) -> Self {
        Self::User(msg.into())
    }

    /// Construct a timeout error.
    #[must_use]
    pub fn timeout(op: impl Into<String>, ms: u64) -> Self {
        Self::Timeout {
            operation: op.into(),
            timeout_ms: ms,
        }
    }

    /// Construct a cancellation error with a human-readable reason.
    #[must_use]
    pub fn cancelled(reason: impl Into<String>) -> Self {
        Self::Cancelled(reason.into())
    }

    /// Construct a permission-denied error.
    #[must_use]
    pub fn permission_denied(msg: impl Into<String>) -> Self {
        Self::PermissionDenied(msg.into())
    }

    /// Construct a rate-limit error.
    #[must_use]
    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self::RateLimited(msg.into())
    }

    /// Construct a planning error from any displayable cause.
    #[must_use]
    pub fn planning(err: impl std::fmt::Display) -> Self {
        Self::Planning(err.to_string())
    }

    /// Returns a stable discriminant suitable for metrics and retry tables.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::Substrate(_) => ErrorKind::Substrate,
            Self::NotFound(_) => ErrorKind::NotFound,
            Self::BodyEncode(_) => ErrorKind::BodyEncode,
            Self::BodyDecode(_) => ErrorKind::BodyDecode,
            Self::Rejected(_) => ErrorKind::Rejected,
            Self::BudgetExceeded { .. } => ErrorKind::BudgetExceeded,
            Self::Io(_) => ErrorKind::Io,
            Self::Json(_) => ErrorKind::Json,
            Self::Invalid(_) => ErrorKind::Invalid,
            Self::Planning(_) => ErrorKind::Planning,
            Self::Agent { .. } => ErrorKind::Agent,
            Self::Gate { .. } => ErrorKind::Gate,
            Self::Tool { .. } => ErrorKind::Tool,
            Self::Chain(_) => ErrorKind::Chain,
            Self::Config(_) => ErrorKind::Config,
            Self::Transport(_) => ErrorKind::Transport,
            Self::User(_) => ErrorKind::User,
            Self::Timeout { .. } => ErrorKind::Timeout,
            Self::Cancelled(_) => ErrorKind::Cancelled,
            Self::PermissionDenied(_) => ErrorKind::PermissionDenied,
            Self::RateLimited(_) => ErrorKind::RateLimited,
        }
    }

    /// Indicates whether retrying this error could succeed.
    ///
    /// - **Transient**: `Timeout`, `RateLimited`, `Io`, `Transport`, `Chain`,
    ///   `Substrate`, `Agent` -- retry may succeed under a backoff policy.
    /// - **Permanent**: `NotFound`, `Rejected`, `Invalid`, `User`,
    ///   `PermissionDenied`, `BudgetExceeded`, `BodyEncode`, `BodyDecode`,
    ///   `Json`, `Config`, `Planning`, `Gate`, `Tool`, `Cancelled` -- retry
    ///   will not succeed without caller intervention.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        self.kind().is_transient()
    }

    /// Returns the recommended retry policy for this error, or `None` if the
    /// error is permanent and should not be retried.
    ///
    /// Transient errors each get a tuned policy:
    /// - `RateLimited`: aggressive backoff (longer base, 5 attempts)
    /// - `Timeout`: moderate backoff (3 attempts)
    /// - `Io`, `Transport`, `Chain`, `Substrate`: standard backoff (3 attempts)
    /// - `Agent`: standard backoff (3 attempts)
    #[must_use]
    pub fn retry_policy(&self) -> Option<retry::RetryPolicy> {
        self.kind().retry_policy()
    }

    /// Maps this error to a log level string suitable for structured logging.
    ///
    /// - `"error"` -- permanent failures, permission denied, budget exceeded
    /// - `"warn"`  -- transient failures that will be retried
    /// - `"info"`  -- cancellations, user errors (expected flow)
    #[must_use]
    pub const fn log_level(&self) -> &'static str {
        self.kind().log_level()
    }
}

/// Stable discriminant — maps variants to a flat set of error kinds for
/// metrics and retry decisions.
///
/// This enum is `#[non_exhaustive]` so new kinds can be added without breaking
/// downstream matches. It implements `Serialize`/`Deserialize` for transport
/// across crate boundaries (e.g. RPC error mapping, metrics sinks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum ErrorKind {
    /// See [`RokoError::Substrate`].
    Substrate,
    /// See [`RokoError::NotFound`].
    NotFound,
    /// See [`RokoError::BodyEncode`].
    BodyEncode,
    /// See [`RokoError::BodyDecode`].
    BodyDecode,
    /// See [`RokoError::Rejected`].
    Rejected,
    /// See [`RokoError::BudgetExceeded`].
    BudgetExceeded,
    /// See [`RokoError::Io`].
    Io,
    /// See [`RokoError::Json`].
    Json,
    /// See [`RokoError::Invalid`].
    Invalid,
    /// See [`RokoError::Planning`].
    Planning,
    /// See [`RokoError::Agent`].
    Agent,
    /// See [`RokoError::Gate`].
    Gate,
    /// See [`RokoError::Tool`].
    Tool,
    /// See [`RokoError::Chain`].
    Chain,
    /// See [`RokoError::Config`].
    Config,
    /// See [`RokoError::Transport`].
    Transport,
    /// See [`RokoError::User`].
    User,
    /// See [`RokoError::Timeout`].
    Timeout,
    /// See [`RokoError::Cancelled`].
    Cancelled,
    /// See [`RokoError::PermissionDenied`].
    PermissionDenied,
    /// See [`RokoError::RateLimited`].
    RateLimited,
}

impl ErrorKind {
    /// A stable string label suitable for metrics (`snake_case`).
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Substrate => "substrate",
            Self::NotFound => "not_found",
            Self::BodyEncode => "body_encode",
            Self::BodyDecode => "body_decode",
            Self::Rejected => "rejected",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Io => "io",
            Self::Json => "json",
            Self::Invalid => "invalid",
            Self::Planning => "planning",
            Self::Agent => "agent",
            Self::Gate => "gate",
            Self::Tool => "tool",
            Self::Chain => "chain",
            Self::Config => "config",
            Self::Transport => "transport",
            Self::User => "user",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::PermissionDenied => "permission_denied",
            Self::RateLimited => "rate_limited",
        }
    }

    /// Whether this kind represents a transient failure that retrying may
    /// resolve. See [`RokoError::is_transient`] for the full contract.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        match self {
            // Transient -- retry under backoff may succeed.
            Self::Timeout
            | Self::RateLimited
            | Self::Io
            | Self::Transport
            | Self::Chain
            | Self::Substrate
            | Self::Agent => true,

            // Permanent -- caller must change input or give up.
            Self::NotFound
            | Self::Rejected
            | Self::Invalid
            | Self::User
            | Self::PermissionDenied
            | Self::BudgetExceeded
            | Self::BodyEncode
            | Self::BodyDecode
            | Self::Json
            | Self::Config
            | Self::Planning
            | Self::Gate
            | Self::Tool
            | Self::Cancelled => false,
        }
    }

    /// Returns the recommended [`RetryPolicy`](retry::RetryPolicy) for this
    /// error kind, or `None` if the error is permanent.
    #[must_use]
    pub fn retry_policy(&self) -> Option<retry::RetryPolicy> {
        if !self.is_transient() {
            return None;
        }
        Some(match self {
            // Rate limits: longer base delay, more attempts.
            Self::RateLimited => retry::RetryPolicy::new(5, 2_000, 60_000, true),
            // Timeouts: moderate.
            Self::Timeout => retry::RetryPolicy::new(3, 1_000, 30_000, true),
            // Everything else transient: standard.
            _ => retry::RetryPolicy::new(3, 500, 15_000, true),
        })
    }

    /// Maps this error kind to a log level string.
    ///
    /// - `"error"` -- permanent failures that indicate bugs or hard stops
    /// - `"warn"`  -- transient failures that will be retried
    /// - `"info"`  -- cancellations, user errors (expected control flow)
    #[must_use]
    pub const fn log_level(&self) -> &'static str {
        match self {
            // Expected flow -- user-initiated or informational.
            Self::Cancelled | Self::User => "info",
            // Transient -- will be retried, warn operators.
            Self::Timeout
            | Self::RateLimited
            | Self::Io
            | Self::Transport
            | Self::Chain
            | Self::Substrate
            | Self::Agent => "warn",
            // Permanent -- something is wrong and needs attention.
            Self::NotFound
            | Self::Rejected
            | Self::Invalid
            | Self::PermissionDenied
            | Self::BudgetExceeded
            | Self::BodyEncode
            | Self::BodyDecode
            | Self::Json
            | Self::Config
            | Self::Planning
            | Self::Gate
            | Self::Tool => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_useful() {
        let e = RokoError::Rejected("no way".into());
        assert!(format!("{e}").contains("no way"));
    }

    #[test]
    fn budget_exceeded_includes_dimension() {
        let e = RokoError::BudgetExceeded {
            dimension: "tokens",
            used: 100,
            limit: 50,
        };
        let msg = format!("{e}");
        assert!(msg.contains("tokens"));
        assert!(msg.contains("100"));
        assert!(msg.contains("50"));
    }

    #[test]
    fn constructors_preserve_message() {
        let e = RokoError::body_decode("bad json");
        assert!(matches!(e, RokoError::BodyDecode(s) if s == "bad json"));
    }

    // §41 tests

    /// Build one example `RokoError` for each `ErrorKind` variant, and assert
    /// that `.kind()` reports the matching discriminant. This is the exhaustive
    /// coverage check called out in the spec.
    #[test]
    fn kind_covers_all_variants() {
        // Force exhaustive matching on `ErrorKind` so that adding a new kind
        // breaks this test until it is covered here.
        fn example(kind: ErrorKind) -> RokoError {
            match kind {
                ErrorKind::Substrate => RokoError::substrate("down"),
                ErrorKind::NotFound => RokoError::NotFound(crate::ContentHash([0; 32])),
                ErrorKind::BodyEncode => RokoError::body_encode("bad"),
                ErrorKind::BodyDecode => RokoError::body_decode("bad"),
                ErrorKind::Rejected => RokoError::Rejected("nope".into()),
                ErrorKind::BudgetExceeded => RokoError::BudgetExceeded {
                    dimension: "tokens",
                    used: 10,
                    limit: 5,
                },
                ErrorKind::Io => {
                    RokoError::Io(std::io::Error::new(std::io::ErrorKind::Other, "oops"))
                }
                ErrorKind::Json => RokoError::Json(
                    serde_json::from_str::<serde_json::Value>("{ bad").unwrap_err(),
                ),
                ErrorKind::Invalid => RokoError::invalid("bad input"),
                ErrorKind::Planning => RokoError::planning("no plan"),
                ErrorKind::Agent => RokoError::agent("claude", "rate-limited"),
                ErrorKind::Gate => RokoError::gate("compile", "failed"),
                ErrorKind::Tool => RokoError::tool("read_file", "not found"),
                ErrorKind::Chain => RokoError::chain("rpc down"),
                ErrorKind::Config => RokoError::config("bad toml"),
                ErrorKind::Transport => RokoError::transport("dns"),
                ErrorKind::User => RokoError::user("please fix"),
                ErrorKind::Timeout => RokoError::timeout("compile", 30_000),
                ErrorKind::Cancelled => RokoError::cancelled("user ctrl-c"),
                ErrorKind::PermissionDenied => RokoError::permission_denied("no fs write"),
                ErrorKind::RateLimited => RokoError::rate_limited("429"),
            }
        }

        let all = [
            ErrorKind::Substrate,
            ErrorKind::NotFound,
            ErrorKind::BodyEncode,
            ErrorKind::BodyDecode,
            ErrorKind::Rejected,
            ErrorKind::BudgetExceeded,
            ErrorKind::Io,
            ErrorKind::Json,
            ErrorKind::Invalid,
            ErrorKind::Planning,
            ErrorKind::Agent,
            ErrorKind::Gate,
            ErrorKind::Tool,
            ErrorKind::Chain,
            ErrorKind::Config,
            ErrorKind::Transport,
            ErrorKind::User,
            ErrorKind::Timeout,
            ErrorKind::Cancelled,
            ErrorKind::PermissionDenied,
            ErrorKind::RateLimited,
        ];

        for kind in all {
            let err = example(kind);
            assert_eq!(err.kind(), kind, "mismatch for {kind:?}");
        }
    }

    #[test]
    fn error_kind_as_str_stable() {
        assert_eq!(ErrorKind::Substrate.as_str(), "substrate");
        assert_eq!(ErrorKind::NotFound.as_str(), "not_found");
        assert_eq!(ErrorKind::BodyEncode.as_str(), "body_encode");
        assert_eq!(ErrorKind::BodyDecode.as_str(), "body_decode");
        assert_eq!(ErrorKind::Rejected.as_str(), "rejected");
        assert_eq!(ErrorKind::BudgetExceeded.as_str(), "budget_exceeded");
        assert_eq!(ErrorKind::Io.as_str(), "io");
        assert_eq!(ErrorKind::Json.as_str(), "json");
        assert_eq!(ErrorKind::Invalid.as_str(), "invalid");
        assert_eq!(ErrorKind::Planning.as_str(), "planning");
        assert_eq!(ErrorKind::Agent.as_str(), "agent");
        assert_eq!(ErrorKind::Gate.as_str(), "gate");
        assert_eq!(ErrorKind::Tool.as_str(), "tool");
        assert_eq!(ErrorKind::Chain.as_str(), "chain");
        assert_eq!(ErrorKind::Config.as_str(), "config");
        assert_eq!(ErrorKind::Transport.as_str(), "transport");
        assert_eq!(ErrorKind::User.as_str(), "user");
        assert_eq!(ErrorKind::Timeout.as_str(), "timeout");
        assert_eq!(ErrorKind::Cancelled.as_str(), "cancelled");
        assert_eq!(ErrorKind::PermissionDenied.as_str(), "permission_denied");
        assert_eq!(ErrorKind::RateLimited.as_str(), "rate_limited");
    }

    #[test]
    fn is_transient_classifies_timeout_as_transient() {
        assert!(RokoError::timeout("compile", 1_000).is_transient());
    }

    #[test]
    fn is_transient_classifies_not_found_as_permanent() {
        let err = RokoError::NotFound(crate::ContentHash([0; 32]));
        assert!(!err.is_transient());
    }

    #[test]
    fn is_transient_classifies_rate_limited_as_transient() {
        assert!(RokoError::rate_limited("429 too many").is_transient());
    }

    #[test]
    fn is_transient_classifies_user_as_permanent() {
        assert!(!RokoError::user("fix your config").is_transient());
    }

    #[test]
    fn constructors_populate_fields() {
        let err = RokoError::agent("claude", "rate-limited");
        assert_eq!(err.kind(), ErrorKind::Agent);
        match err {
            RokoError::Agent { backend, message } => {
                assert_eq!(backend, "claude");
                assert_eq!(message, "rate-limited");
            }
            _ => panic!("wrong variant"),
        }

        let err = RokoError::gate("compile", "boom");
        assert_eq!(err.kind(), ErrorKind::Gate);
        match err {
            RokoError::Gate { gate, message } => {
                assert_eq!(gate, "compile");
                assert_eq!(message, "boom");
            }
            _ => panic!("wrong variant"),
        }

        let err = RokoError::tool("shell", "exit 1");
        assert_eq!(err.kind(), ErrorKind::Tool);
        match err {
            RokoError::Tool { tool, message } => {
                assert_eq!(tool, "shell");
                assert_eq!(message, "exit 1");
            }
            _ => panic!("wrong variant"),
        }

        let err = RokoError::timeout("compile", 5_000);
        assert_eq!(err.kind(), ErrorKind::Timeout);
        match err {
            RokoError::Timeout {
                operation,
                timeout_ms,
            } => {
                assert_eq!(operation, "compile");
                assert_eq!(timeout_ms, 5_000);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn display_contains_component_name() {
        let err = RokoError::agent("claude", "rate-limited");
        let msg = format!("{err}");
        assert!(msg.contains("claude"), "display missing backend: {msg}");
        assert!(msg.contains("rate-limited"), "display missing message: {msg}");

        let err = RokoError::gate("compile", "boom");
        let msg = format!("{err}");
        assert!(msg.contains("compile"), "display missing gate: {msg}");

        let err = RokoError::tool("read_file", "enoent");
        let msg = format!("{err}");
        assert!(msg.contains("read_file"), "display missing tool: {msg}");

        let err = RokoError::timeout("step", 1_234);
        let msg = format!("{err}");
        assert!(msg.contains("1234"), "display missing timeout_ms: {msg}");
        assert!(msg.contains("step"), "display missing operation: {msg}");
    }

    #[test]
    fn json_roundtrip_of_error_kind() {
        let kinds = [
            ErrorKind::Substrate,
            ErrorKind::NotFound,
            ErrorKind::Agent,
            ErrorKind::Gate,
            ErrorKind::Tool,
            ErrorKind::Timeout,
            ErrorKind::RateLimited,
            ErrorKind::PermissionDenied,
        ];
        for kind in kinds {
            let s = serde_json::to_string(&kind).expect("serialize");
            let back: ErrorKind = serde_json::from_str(&s).expect("deserialize");
            assert_eq!(back, kind);
        }
    }

    // §41.6 — retry_policy tests

    #[test]
    fn retry_policy_returns_none_for_permanent_errors() {
        let permanent = [
            ErrorKind::NotFound,
            ErrorKind::Rejected,
            ErrorKind::Invalid,
            ErrorKind::User,
            ErrorKind::PermissionDenied,
            ErrorKind::BudgetExceeded,
            ErrorKind::BodyEncode,
            ErrorKind::BodyDecode,
            ErrorKind::Json,
            ErrorKind::Config,
            ErrorKind::Planning,
            ErrorKind::Gate,
            ErrorKind::Tool,
            ErrorKind::Cancelled,
        ];
        for kind in permanent {
            assert!(
                kind.retry_policy().is_none(),
                "{kind:?} should have no retry policy"
            );
        }
    }

    #[test]
    fn retry_policy_returns_some_for_transient_errors() {
        let transient = [
            ErrorKind::Timeout,
            ErrorKind::RateLimited,
            ErrorKind::Io,
            ErrorKind::Transport,
            ErrorKind::Chain,
            ErrorKind::Substrate,
            ErrorKind::Agent,
        ];
        for kind in transient {
            assert!(
                kind.retry_policy().is_some(),
                "{kind:?} should have a retry policy"
            );
        }
    }

    #[test]
    fn retry_policy_rate_limited_has_higher_attempts() {
        let rl = ErrorKind::RateLimited.retry_policy().unwrap();
        let io = ErrorKind::Io.retry_policy().unwrap();
        assert!(
            rl.max_attempts() > io.max_attempts(),
            "rate-limited should have more attempts than io"
        );
    }

    #[test]
    fn retry_policy_via_roko_error() {
        let err = RokoError::timeout("compile", 5_000);
        assert!(err.retry_policy().is_some());

        let err = RokoError::user("bad input");
        assert!(err.retry_policy().is_none());
    }

    // §41.6 — log_level tests

    #[test]
    fn log_level_classifies_all_kinds() {
        // Transient -> warn
        assert_eq!(ErrorKind::Timeout.log_level(), "warn");
        assert_eq!(ErrorKind::RateLimited.log_level(), "warn");
        assert_eq!(ErrorKind::Io.log_level(), "warn");
        assert_eq!(ErrorKind::Transport.log_level(), "warn");
        assert_eq!(ErrorKind::Chain.log_level(), "warn");
        assert_eq!(ErrorKind::Substrate.log_level(), "warn");
        assert_eq!(ErrorKind::Agent.log_level(), "warn");

        // Expected flow -> info
        assert_eq!(ErrorKind::Cancelled.log_level(), "info");
        assert_eq!(ErrorKind::User.log_level(), "info");

        // Permanent -> error
        assert_eq!(ErrorKind::NotFound.log_level(), "error");
        assert_eq!(ErrorKind::Rejected.log_level(), "error");
        assert_eq!(ErrorKind::BudgetExceeded.log_level(), "error");
        assert_eq!(ErrorKind::Config.log_level(), "error");
        assert_eq!(ErrorKind::PermissionDenied.log_level(), "error");
    }

    #[test]
    fn log_level_via_roko_error() {
        assert_eq!(RokoError::timeout("x", 1).log_level(), "warn");
        assert_eq!(RokoError::cancelled("ctrl-c").log_level(), "info");
        assert_eq!(RokoError::invalid("bad").log_level(), "error");
    }
}
