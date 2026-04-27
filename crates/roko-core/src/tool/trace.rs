//! Execution traces and failure traces (§36.o, §36.p; parity items 36.96–36.110).
//!
//! **Why traces are a day-one feature**: Meta-Harness (Stanford, March 2026)
//! demonstrated that an agentic outer loop reading **10 million tokens of
//! prior trace history** can rewrite its own harness to deliver a 6× gain.
//! DSPy's GEPA optimizer (97.8% on information extraction) works by
//! evolutionary optimization over **serialized trajectories**. ToolRL and
//! Self-Challenging Agents both require structured traces to compute
//! fine-grained rewards.
//!
//! Roko therefore emits a full [`ToolTrace`] for **every** tool call from
//! v1 — not "later when we add learning". Traces persist to
//! `.roko/traces/{yyyy-mm-dd}/{trace_id}.jsonl` and become the substrate
//! for the continuous-tuning loops (§35 Loop F/G/H) described in
//! `roko-continuous-tuning.md`.
//!
//! # Types
//!
//! - [`TraceId`] — random 128-bit identity
//! - [`ToolTrace`] — complete record of one tool call
//! - [`ToolTraceEvent`] — fine-grained events (14+ kinds)
//! - [`TraceStep`] — coarse phase classification for `FailureTrace`
//! - [`CancelSource`] — attribution for [`ToolTraceEvent::Cancellation`]
//! - [`FailureKind`] — structured root cause (14 variants)
//! - [`FailureTrace`] — typed failure with evidence
//! - [`ToolOutcome`] — terminal reward/latency/cost record
//! - [`TraceSink`] — async-agnostic sink trait
//! - [`TraceBuilder`] — ergonomic RAII trace assembly
//!
//! # Invariants
//!
//! 1. Every tool call produces **exactly one** `ToolTrace`.
//! 2. Every failed call produces **exactly one** `FailureTrace`, linked
//!    by `trace_id`.
//! 3. Events within a trace are ordered by `at_ms` (monotonic within a
//!    process).
//! 4. `TraceBuilder::finish` is idempotent; `Drop` calls it once if not
//!    already called (RAII safety net against panics).

use serde::{Deserialize, Serialize};

use super::call::ToolError;
use super::format::ToolFormat;
use crate::AgentRole;

// ─── TraceId ──────────────────────────────────────────────────────────────

/// 128-bit random trace identifier.
///
/// Intentionally simple: callers provide the bytes (typically from a UUID
/// v4 or similar). Serialization is hex so traces are grep-able in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TraceId(pub [u8; 16]);

impl TraceId {
    /// Construct from raw bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Construct from a hex string (32 chars). Returns `None` on invalid
    /// length or non-hex characters.
    #[must_use]
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 32 {
            return None;
        }
        let mut out = [0u8; 16];
        for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
            let hi = hex_val(chunk[0])?;
            let lo = hex_val(chunk[1])?;
            out[i] = (hi << 4) | lo;
        }
        Some(Self(out))
    }

    /// Render as a 32-char lowercase hex string.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut s = String::with_capacity(32);
        for b in self.0 {
            s.push(hex_char(b >> 4));
            s.push(hex_char(b & 0xF));
        }
        s
    }
}

const fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + nibble - 10) as char,
        _ => '?',
    }
}

const fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

impl std::fmt::Display for TraceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for TraceId {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for TraceId {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        Self::from_hex(&s).ok_or_else(|| serde::de::Error::custom("invalid TraceId hex"))
    }
}

// ─── FailureKind ──────────────────────────────────────────────────────────

/// Structured root cause of a failed tool call.
///
/// Required for ToolRL-style fine-grained rewards — coarse answer-matching
/// fails for tool use (Qian et al., NeurIPS 2025). Each variant is one
/// decision the downstream optimizer can key off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureKind {
    /// LLM emitted syntactically invalid JSON (parser failure).
    MalformedJson,
    /// Tool name not present in the registry.
    UnknownTool,
    /// A required argument was missing.
    MissingRequired,
    /// An argument had the wrong type (string vs number, etc.).
    TypeMismatch,
    /// An argument value was outside the accepted domain (e.g. enum miss).
    OutOfDomain,
    /// LLM invented an argument not in the schema.
    HallucinatedParam,
    /// Role lacked a required [`crate::tool::ToolPermission`] flag.
    PermissionDenied,
    /// Path argument escaped the worktree sandbox.
    PathEscape,
    /// Handler exceeded its timeout budget.
    Timeout,
    /// Cancel token fired before handler completed.
    Cancelled,
    /// Handler raised a tool-specific error.
    ToolHandlerError,
    /// Multi-turn loop exhausted its iteration cap.
    LoopExhausted,
    /// JSON-schema validation rejected the call.
    SchemaInvalid,
    /// Ollama dropped the tool call on streaming (issues #9632/#12557).
    StreamDropped,
}

impl FailureKind {
    /// Short identifier for logs / metrics keys.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedJson => "malformed_json",
            Self::UnknownTool => "unknown_tool",
            Self::MissingRequired => "missing_required",
            Self::TypeMismatch => "type_mismatch",
            Self::OutOfDomain => "out_of_domain",
            Self::HallucinatedParam => "hallucinated_param",
            Self::PermissionDenied => "permission_denied",
            Self::PathEscape => "path_escape",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::ToolHandlerError => "tool_handler_error",
            Self::LoopExhausted => "loop_exhausted",
            Self::SchemaInvalid => "schema_invalid",
            Self::StreamDropped => "stream_dropped",
        }
    }

    /// Is this failure recoverable by retrying the same call with a
    /// different format? (`MalformedJson`, `SchemaInvalid`, `StreamDropped`.)
    #[must_use]
    pub const fn retry_with_format_change(self) -> bool {
        matches!(
            self,
            Self::MalformedJson | Self::SchemaInvalid | Self::StreamDropped
        )
    }

    /// Is this failure attributable to the model's output quality
    /// (vs environment/sandbox/timeout)?
    #[must_use]
    pub const fn is_model_error(self) -> bool {
        matches!(
            self,
            Self::MalformedJson
                | Self::UnknownTool
                | Self::MissingRequired
                | Self::TypeMismatch
                | Self::OutOfDomain
                | Self::HallucinatedParam
                | Self::SchemaInvalid
        )
    }
}

// ─── classify_tool_error ──────────────────────────────────────────────────

/// Map a [`ToolError`] variant to the corresponding [`FailureKind`].
///
/// This is the single authoritative mapping used by the dispatcher to emit
/// [`FailureTrace`]s: call `classify_tool_error(&err)` to obtain the
/// structured root cause without duplicating match logic.
#[must_use]
pub const fn classify_tool_error(error: &ToolError) -> FailureKind {
    match error {
        ToolError::SchemaInvalid(_) => FailureKind::SchemaInvalid,
        ToolError::PermissionDenied(_)
        | ToolError::CommandNotAllowed(_)
        | ToolError::NetworkBlocked(_) => FailureKind::PermissionDenied,
        ToolError::Timeout { .. } => FailureKind::Timeout,
        ToolError::Cancelled => FailureKind::Cancelled,
        ToolError::PathOutsideWorktree(_) => FailureKind::PathEscape,
        ToolError::HandlerPanic(_) | ToolError::Other(_) => FailureKind::ToolHandlerError,
    }
}

// ─── TraceStep ────────────────────────────────────────────────────────────

/// Coarse phase of the dispatch pipeline — used by [`FailureTrace`] to
/// point at where in the pipeline the failure occurred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TraceStep {
    /// Prompt assembly + tool-registry subset selection.
    Prompt,
    /// LLM completion (the raw model call).
    Emit,
    /// Per-format parsing of the LLM response into `ToolCall[]`.
    Parse,
    /// JSON-schema validation of arguments.
    Validate,
    /// Permission check against the role's [`ToolPermissions`](crate::ToolPermissions).
    Permissions,
    /// Dispatcher lookup + timeout/cancel setup.
    Dispatch,
    /// Handler execution.
    Execute,
    /// Result shaping, truncation, artifact attachment.
    Result,
}

// ─── CancelSource ─────────────────────────────────────────────────────────

/// Who fired the cancel token that ended a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelSource {
    /// Conductor watcher (§11) intervention.
    Conductor,
    /// Per-tool timeout expired.
    Timeout,
    /// User-initiated abort (Ctrl-C or TUI abort).
    UserAbort,
    /// Model capability downgrade (e.g. Ollama version regression).
    CapabilityLost,
    /// Circuit breaker tripped.
    CircuitBreaker,
}

// ─── ToolOutcome ──────────────────────────────────────────────────────────

/// Terminal record of one completed tool call — the reward signal for the
/// [`crate::tool::FormatBandit`] and the DSPy / ToolRL optimizers.
///
/// `reward` composes success with normalized latency, cost, and recovery
/// attempts. See [`crate::tool::metrics::compute_reward`] for the default
/// composition; overrides are allowed via config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutcome {
    /// Did the call ultimately produce a successful [`crate::ToolResult::Ok`]?
    pub success: bool,
    /// End-to-end wall-clock latency (prompt → final result).
    pub latency_ms: u64,
    /// Total cost in USD, summed over all LLM turns in this call.
    pub cost_usd: f32,
    /// Number of retries/demotions before the terminal result.
    pub recovery_attempts: u8,
    /// Structured failure kind if `!success`.
    pub failure: Option<FailureKind>,
    /// Composite reward in `[0, 1]` for bandit feedback.
    pub reward: f32,
}

impl ToolOutcome {
    /// Construct a successful outcome with zero-cost defaults.
    #[must_use]
    pub const fn success(latency_ms: u64, cost_usd: f32) -> Self {
        Self {
            success: true,
            latency_ms,
            cost_usd,
            recovery_attempts: 0,
            failure: None,
            reward: 1.0,
        }
    }

    /// Construct a failing outcome from a [`FailureKind`].
    #[must_use]
    pub const fn failure(kind: FailureKind, latency_ms: u64, cost_usd: f32) -> Self {
        Self {
            success: false,
            latency_ms,
            cost_usd,
            recovery_attempts: 0,
            failure: Some(kind),
            reward: 0.0,
        }
    }

    /// Override the reward (after composition).
    #[must_use]
    pub const fn with_reward(mut self, reward: f32) -> Self {
        self.reward = reward;
        self
    }

    /// Override the recovery-attempts counter.
    #[must_use]
    pub const fn with_recovery_attempts(mut self, attempts: u8) -> Self {
        self.recovery_attempts = attempts;
        self
    }
}

// ─── ToolTraceEvent ───────────────────────────────────────────────────────

/// One fine-grained event in a [`ToolTrace`]. Events are append-only,
/// ordered by `at_ms` (monotonic within a process).
///
/// Adding a new variant: append, don't insert. The enum is
/// `#[non_exhaustive]` so downstream match arms don't need updating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "event", content = "data")]
#[non_exhaustive]
pub enum ToolTraceEvent {
    /// Prompt assembled for the LLM with N tools offered.
    PromptAssembled {
        /// Token count of the assembled prompt.
        token_count: u32,
        /// Canonical names of tools presented to the model.
        tools_offered: Vec<String>,
        /// Event timestamp (ms since epoch).
        at_ms: i64,
    },
    /// LLM returned a completion.
    LlmCompleted {
        /// Wall-clock time of the LLM call.
        wall_ms: u64,
        /// Prompt tokens consumed.
        prompt_tokens: u32,
        /// Completion tokens generated.
        completion_tokens: u32,
        /// USD cost of this LLM turn.
        cost_usd: f32,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Per-format parser extracted tool calls from the completion.
    ToolCallParsed {
        /// Format family parser name (e.g. `"hermes"`, `"gemma4"`).
        parser: String,
        /// Length of the raw text/bytes parsed.
        raw_bytes_len: usize,
        /// Parser wall-clock time.
        parse_ms: u32,
        /// Number of calls extracted.
        calls_extracted: u32,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Argument JSON-schema validation ran.
    ArgsValidated {
        /// Did validation pass?
        ok: bool,
        /// Schema-validator errors (empty on success).
        schema_errors: Vec<String>,
        /// Validator wall-clock time.
        validate_ms: u32,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Permission check ran.
    PermissionsChecked {
        /// Were all required flags granted?
        granted: bool,
        /// Missing flag names (empty on grant).
        missing: Vec<String>,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Handler execution started.
    HandlerStarted {
        /// Canonical tool name.
        handler: String,
        /// Tool category.
        category: super::def::ToolCategory,
        /// Concurrency policy.
        concurrency: super::def::ToolConcurrency,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Handler execution finished.
    HandlerFinished {
        /// Handler wall-clock time.
        exit_ms: u64,
        /// Bytes in the result content.
        bytes_out: usize,
        /// Number of artifacts returned.
        artifacts_count: u32,
        /// Event timestamp.
        at_ms: i64,
    },
    /// The dispatcher retried after a recoverable failure.
    Retry {
        /// Which attempt this is (1-indexed).
        attempt: u8,
        /// Why we're retrying.
        reason: FailureKind,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Format bandit demoted from one arm to another.
    Demotion {
        /// Format before demotion.
        from: ToolFormat,
        /// Format after demotion.
        to: ToolFormat,
        /// Human-readable reason.
        reason: String,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Call was cancelled.
    Cancellation {
        /// Attribution.
        source: CancelSource,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Parallel calls forced to serial due to `ToolConcurrency::Serial`.
    Serialization {
        /// Reason (e.g. tool-side, conflicting exclusive file).
        reason: String,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Streaming was forced off because tools were present.
    StreamCoerced {
        /// Event timestamp.
        at_ms: i64,
    },
    /// Result content was truncated to stay under `max_inline_bytes`.
    Truncation {
        /// Bytes kept.
        kept: usize,
        /// Original bytes.
        total: usize,
        /// Event timestamp.
        at_ms: i64,
    },
    /// MCP server connection was lost.
    McpConnectionLost {
        /// Server identifier.
        server: String,
        /// Event timestamp.
        at_ms: i64,
    },
    /// Custom event for extensions.
    Custom {
        /// Event name.
        name: String,
        /// Arbitrary JSON payload.
        data: serde_json::Value,
        /// Event timestamp.
        at_ms: i64,
    },
}

impl ToolTraceEvent {
    /// Extract the embedded timestamp.
    #[must_use]
    pub const fn at_ms(&self) -> i64 {
        match self {
            Self::PromptAssembled { at_ms, .. }
            | Self::LlmCompleted { at_ms, .. }
            | Self::ToolCallParsed { at_ms, .. }
            | Self::ArgsValidated { at_ms, .. }
            | Self::PermissionsChecked { at_ms, .. }
            | Self::HandlerStarted { at_ms, .. }
            | Self::HandlerFinished { at_ms, .. }
            | Self::Retry { at_ms, .. }
            | Self::Demotion { at_ms, .. }
            | Self::Cancellation { at_ms, .. }
            | Self::Serialization { at_ms, .. }
            | Self::StreamCoerced { at_ms }
            | Self::Truncation { at_ms, .. }
            | Self::McpConnectionLost { at_ms, .. }
            | Self::Custom { at_ms, .. } => *at_ms,
        }
    }

    /// Return a scrubbed clone where string payloads have been passed through
    /// [`LogScrubber::scrub`](crate::obs::scrub::LogScrubber::scrub).
    #[must_use]
    fn scrubbed(&self, scrubber: &crate::obs::scrub::LogScrubber) -> Self {
        match self {
            Self::PromptAssembled {
                token_count,
                tools_offered,
                at_ms,
            } => Self::PromptAssembled {
                token_count: *token_count,
                tools_offered: tools_offered.iter().map(|s| scrubber.scrub(s)).collect(),
                at_ms: *at_ms,
            },
            Self::ToolCallParsed {
                parser,
                raw_bytes_len,
                parse_ms,
                calls_extracted,
                at_ms,
            } => Self::ToolCallParsed {
                parser: scrubber.scrub(parser),
                raw_bytes_len: *raw_bytes_len,
                parse_ms: *parse_ms,
                calls_extracted: *calls_extracted,
                at_ms: *at_ms,
            },
            Self::ArgsValidated {
                ok,
                schema_errors,
                validate_ms,
                at_ms,
            } => Self::ArgsValidated {
                ok: *ok,
                schema_errors: schema_errors.iter().map(|s| scrubber.scrub(s)).collect(),
                validate_ms: *validate_ms,
                at_ms: *at_ms,
            },
            Self::PermissionsChecked {
                granted,
                missing,
                at_ms,
            } => Self::PermissionsChecked {
                granted: *granted,
                missing: missing.iter().map(|s| scrubber.scrub(s)).collect(),
                at_ms: *at_ms,
            },
            Self::HandlerStarted {
                handler,
                category,
                concurrency,
                at_ms,
            } => Self::HandlerStarted {
                handler: scrubber.scrub(handler),
                category: *category,
                concurrency: *concurrency,
                at_ms: *at_ms,
            },
            Self::Demotion {
                from,
                to,
                reason,
                at_ms,
            } => Self::Demotion {
                from: from.clone(),
                to: to.clone(),
                reason: scrubber.scrub(reason),
                at_ms: *at_ms,
            },
            Self::Serialization { reason, at_ms } => Self::Serialization {
                reason: scrubber.scrub(reason),
                at_ms: *at_ms,
            },
            Self::McpConnectionLost { server, at_ms } => Self::McpConnectionLost {
                server: scrubber.scrub(server),
                at_ms: *at_ms,
            },
            Self::Custom { name, data, at_ms } => {
                // Scrub by re-serializing the JSON payload to a string,
                // scrubbing it, then parsing it back. If the round-trip fails
                // (shouldn't for valid JSON), keep the original.
                let scrubbed_data = serde_json::to_string(data)
                    .ok()
                    .map(|s| scrubber.scrub(&s))
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or_else(|| data.clone());
                Self::Custom {
                    name: scrubber.scrub(name),
                    data: scrubbed_data,
                    at_ms: *at_ms,
                }
            }
            // Events with only numeric/copy fields — clone as-is.
            other => other.clone(),
        }
    }
}

// ─── ToolTrace ────────────────────────────────────────────────────────────

/// Complete execution trace of one tool call.
///
/// Persisted to `.roko/traces/{yyyy-mm-dd}/{trace_id}.jsonl` by the
/// `JsonlTraceSink` (in roko-std, §36.99). Aggregated per-tool /
/// per-model / per-role statistics feed the TUI Optimizer view and the
/// continuous-tuning loops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolTrace {
    /// 128-bit trace identity.
    pub trace_id: TraceId,
    /// The inbound [`crate::tool::ToolCall::id`] this trace records.
    pub call_id: String,
    /// The role whose agent emitted the call.
    pub role: AgentRole,
    /// Model slug (as-recorded, unnormalized).
    pub model: String,
    /// Format used for this call (the bandit's chosen arm).
    pub format_used: ToolFormat,
    /// Start of the trace (ms since epoch).
    pub started_at_ms: i64,
    /// End of the trace (ms since epoch).
    pub ended_at_ms: i64,
    /// Ordered event list.
    pub events: Vec<ToolTraceEvent>,
    /// Terminal outcome.
    pub outcome: ToolOutcome,
}

impl ToolTrace {
    /// Trace duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> i64 {
        self.ended_at_ms - self.started_at_ms
    }

    /// Index of the first event matching a predicate.
    #[must_use]
    pub fn find_event_index(&self, pred: impl Fn(&ToolTraceEvent) -> bool) -> Option<usize> {
        self.events.iter().position(pred)
    }

    /// Return a clone of this trace with all event payloads scrubbed through
    /// `scrubber`. String fields that could contain user secrets (tool names,
    /// custom-event payloads, demotion reasons, parser names, schema errors,
    /// etc.) are passed through [`LogScrubber::scrub`](crate::obs::scrub::LogScrubber::scrub).
    ///
    /// Callers should use this before persisting a trace to disk so that
    /// secrets never land in `.roko/traces/`.
    #[must_use]
    pub fn scrubbed(&self, scrubber: &crate::obs::scrub::LogScrubber) -> Self {
        let events = self.events.iter().map(|e| e.scrubbed(scrubber)).collect();
        Self {
            trace_id: self.trace_id,
            call_id: scrubber.scrub(&self.call_id),
            role: self.role,
            model: scrubber.scrub(&self.model),
            format_used: self.format_used.clone(),
            started_at_ms: self.started_at_ms,
            ended_at_ms: self.ended_at_ms,
            events,
            outcome: self.outcome.clone(),
        }
    }
}

// ─── FailureTrace ─────────────────────────────────────────────────────────

/// Typed failure record — produced alongside a failing [`ToolTrace`].
///
/// The `contributing_events` field points at `ToolTrace::events[i]`
/// indices so a downstream analyzer can replay the exact event sequence
/// that produced the failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureTrace {
    /// Back-reference to the parent [`ToolTrace`].
    pub trace_id: TraceId,
    /// Pipeline phase in which the failure occurred.
    pub step: TraceStep,
    /// Classified root cause.
    pub root_cause: FailureKind,
    /// Human-readable evidence (snippet of the failing JSON, stack frame, …).
    pub evidence: String,
    /// Event indices in `ToolTrace::events` that contributed.
    pub contributing_events: Vec<usize>,
    /// Optional cheap-model explanation of the failure (opt-in per §36.108).
    pub model_self_explanation: Option<String>,
}

impl FailureTrace {
    /// Construct a failure trace from its required fields.
    #[must_use]
    pub fn new(
        trace_id: TraceId,
        step: TraceStep,
        root_cause: FailureKind,
        evidence: impl Into<String>,
    ) -> Self {
        Self {
            trace_id,
            step,
            root_cause,
            evidence: evidence.into(),
            contributing_events: Vec::new(),
            model_self_explanation: None,
        }
    }

    /// Append a contributing event index.
    pub fn add_contributing_event(&mut self, idx: usize) {
        self.contributing_events.push(idx);
    }

    /// Attach a model self-explanation.
    #[must_use]
    pub fn with_self_explanation(mut self, s: impl Into<String>) -> Self {
        self.model_self_explanation = Some(s.into());
        self
    }
}

// ─── TraceSink trait ──────────────────────────────────────────────────────

/// Runtime-agnostic sink for trace events + finished traces.
///
/// Implementors: `JsonlTraceSink` (roko-std, persistent),
/// `InMemoryTraceSink` (roko-std, test helper), `NoopTraceSink` (this file).
///
/// Sinks must not block the caller — append and return immediately,
/// buffering downstream if necessary.
pub trait TraceSink: Send + Sync {
    /// Append a single event to an open trace.
    fn append(&self, trace_id: TraceId, event: ToolTraceEvent);
    /// Close an open trace with its finalized snapshot.
    fn finish(&self, trace: ToolTrace);
}

/// No-op sink — drops every event. Used as a default when tracing is
/// disabled or unconfigured.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopTraceSink;

impl TraceSink for NoopTraceSink {
    fn append(&self, _trace_id: TraceId, _event: ToolTraceEvent) {}
    fn finish(&self, _trace: ToolTrace) {}
}

// ─── TraceBuilder (ergonomic assembly) ────────────────────────────────────

/// Ergonomic trace-assembly helper with RAII close-on-drop safety.
///
/// `TraceBuilder::finish` is idempotent; `Drop` calls it with the pending
/// snapshot if the caller forgot (or a panic unwound). This is critical
/// for invariant #1: every tool call produces **exactly one** trace,
/// even under failure.
pub struct TraceBuilder {
    trace_id: TraceId,
    call_id: String,
    role: AgentRole,
    model: String,
    format_used: ToolFormat,
    started_at_ms: i64,
    events: Vec<ToolTraceEvent>,
    outcome: Option<ToolOutcome>,
    sink: std::sync::Arc<dyn TraceSink>,
    finished: bool,
}

impl TraceBuilder {
    /// Start a new trace.
    #[must_use]
    pub fn start(
        trace_id: TraceId,
        call_id: impl Into<String>,
        role: AgentRole,
        model: impl Into<String>,
        format_used: ToolFormat,
        started_at_ms: i64,
        sink: std::sync::Arc<dyn TraceSink>,
    ) -> Self {
        Self {
            trace_id,
            call_id: call_id.into(),
            role,
            model: model.into(),
            format_used,
            started_at_ms,
            events: Vec::new(),
            outcome: None,
            sink,
            finished: false,
        }
    }

    /// Append an event. Also streamed to the sink for low-latency
    /// downstream consumers (TUI live view).
    pub fn event(&mut self, event: ToolTraceEvent) {
        self.sink.append(self.trace_id, event.clone());
        self.events.push(event);
    }

    /// Number of events recorded so far.
    #[must_use]
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Attach the terminal outcome (call this before `finish`).
    #[allow(clippy::missing_const_for_fn)] // cannot be const: Option<ToolOutcome> destructor
    pub fn set_outcome(&mut self, outcome: ToolOutcome) {
        self.outcome = Some(outcome);
    }

    /// Finalize the trace and flush to the sink. Idempotent.
    pub fn finish(&mut self, ended_at_ms: i64) {
        if self.finished {
            return;
        }
        let outcome = self.outcome.clone().unwrap_or_else(|| {
            // Panic-safe default: if no outcome was set, assume aborted.
            ToolOutcome::failure(FailureKind::Cancelled, 0, 0.0)
        });
        let trace = ToolTrace {
            trace_id: self.trace_id,
            call_id: std::mem::take(&mut self.call_id),
            role: self.role,
            model: std::mem::take(&mut self.model),
            format_used: self.format_used.clone(),
            started_at_ms: self.started_at_ms,
            ended_at_ms,
            events: std::mem::take(&mut self.events),
            outcome,
        };
        self.sink.finish(trace);
        self.finished = true;
    }
}

impl Drop for TraceBuilder {
    fn drop(&mut self) {
        if !self.finished {
            // RAII safety net: close with now-timestamp.
            let now = chrono::Utc::now().timestamp_millis();
            self.finish(now);
        }
    }
}

#[cfg(test)]
#[allow(clippy::significant_drop_tightening)] // test lock scopes are intentional
mod tests {
    use super::super::def::{ToolCategory, ToolConcurrency};
    use super::*;
    use parking_lot::Mutex;
    use std::sync::Arc;

    fn sample_trace_id() -> TraceId {
        TraceId::from_bytes([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
    }

    #[test]
    fn trace_id_hex_roundtrip() {
        let id = sample_trace_id();
        let hex = id.to_hex();
        assert_eq!(hex, "0102030405060708090a0b0c0d0e0f10");
        let decoded = TraceId::from_hex(&hex).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn trace_id_from_hex_rejects_bad_input() {
        assert!(TraceId::from_hex("short").is_none());
        assert!(TraceId::from_hex("g".repeat(32).as_str()).is_none());
    }

    #[test]
    fn trace_id_serde_roundtrip() {
        let id = sample_trace_id();
        let json = serde_json::to_string(&id).unwrap();
        let decoded: TraceId = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, id);
    }

    #[test]
    fn failure_kind_retry_detection() {
        assert!(FailureKind::MalformedJson.retry_with_format_change());
        assert!(FailureKind::SchemaInvalid.retry_with_format_change());
        assert!(FailureKind::StreamDropped.retry_with_format_change());
        assert!(!FailureKind::PathEscape.retry_with_format_change());
        assert!(!FailureKind::Timeout.retry_with_format_change());
    }

    #[test]
    fn failure_kind_is_model_error_classification() {
        assert!(FailureKind::MalformedJson.is_model_error());
        assert!(FailureKind::HallucinatedParam.is_model_error());
        assert!(!FailureKind::Timeout.is_model_error());
        assert!(!FailureKind::PermissionDenied.is_model_error());
        assert!(!FailureKind::PathEscape.is_model_error());
    }

    #[test]
    fn tool_outcome_success_default() {
        let o = ToolOutcome::success(250, 0.01);
        assert!(o.success);
        assert!(o.failure.is_none());
        assert!((o.reward - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn tool_outcome_failure_default() {
        let o = ToolOutcome::failure(FailureKind::Timeout, 5000, 0.03);
        assert!(!o.success);
        assert_eq!(o.failure, Some(FailureKind::Timeout));
        assert!(o.reward.abs() < f32::EPSILON);
    }

    #[test]
    fn tool_outcome_builders_chain() {
        let o = ToolOutcome::success(100, 0.01)
            .with_reward(0.75)
            .with_recovery_attempts(2);
        assert_eq!(o.recovery_attempts, 2);
        assert!((o.reward - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn event_at_ms_extracts_timestamp() {
        let e = ToolTraceEvent::HandlerFinished {
            exit_ms: 50,
            bytes_out: 100,
            artifacts_count: 0,
            at_ms: 12345,
        };
        assert_eq!(e.at_ms(), 12345);
    }

    #[test]
    fn event_serde_roundtrip() {
        let e = ToolTraceEvent::Demotion {
            from: ToolFormat::HermesJson,
            to: ToolFormat::ReActText,
            reason: "3 malformed JSON in a row".into(),
            at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&e).unwrap();
        let decoded: ToolTraceEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, e);
    }

    #[test]
    fn trace_duration_is_computed() {
        let trace = ToolTrace {
            trace_id: sample_trace_id(),
            call_id: "c".into(),
            role: AgentRole::Implementer,
            model: "claude-sonnet-4-5".into(),
            format_used: ToolFormat::AnthropicBlocks,
            started_at_ms: 1_000,
            ended_at_ms: 1_250,
            events: Vec::new(),
            outcome: ToolOutcome::success(250, 0.01),
        };
        assert_eq!(trace.duration_ms(), 250);
    }

    #[test]
    fn trace_find_event_index_returns_first_match() {
        let trace = ToolTrace {
            trace_id: sample_trace_id(),
            call_id: "c".into(),
            role: AgentRole::Implementer,
            model: "x".into(),
            format_used: ToolFormat::ReActText,
            started_at_ms: 0,
            ended_at_ms: 100,
            events: vec![
                ToolTraceEvent::StreamCoerced { at_ms: 1 },
                ToolTraceEvent::Demotion {
                    from: ToolFormat::HermesJson,
                    to: ToolFormat::ReActText,
                    reason: "r".into(),
                    at_ms: 2,
                },
                ToolTraceEvent::StreamCoerced { at_ms: 3 },
            ],
            outcome: ToolOutcome::success(100, 0.0),
        };
        let idx = trace.find_event_index(|e| matches!(e, ToolTraceEvent::Demotion { .. }));
        assert_eq!(idx, Some(1));
        let none = trace.find_event_index(|e| matches!(e, ToolTraceEvent::Truncation { .. }));
        assert!(none.is_none());
    }

    #[test]
    fn failure_trace_builders_add_and_explain() {
        let mut ft = FailureTrace::new(
            sample_trace_id(),
            TraceStep::Parse,
            FailureKind::MalformedJson,
            "unexpected token `}`",
        );
        ft.add_contributing_event(2);
        ft.add_contributing_event(3);
        let ft = ft.with_self_explanation("I forgot to close the brace");
        assert_eq!(ft.step, TraceStep::Parse);
        assert_eq!(ft.contributing_events, vec![2, 3]);
        assert!(ft.model_self_explanation.is_some());
    }

    #[test]
    fn failure_trace_serde_roundtrip() {
        let ft = FailureTrace::new(
            sample_trace_id(),
            TraceStep::Validate,
            FailureKind::MissingRequired,
            "arg `path` required",
        );
        let json = serde_json::to_string(&ft).unwrap();
        let decoded: FailureTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, ft);
    }

    // Collecting sink used in builder tests.
    #[derive(Default)]
    struct CollectingSink {
        appends: Mutex<Vec<(TraceId, ToolTraceEvent)>>,
        finishes: Mutex<Vec<ToolTrace>>,
    }

    impl TraceSink for CollectingSink {
        fn append(&self, id: TraceId, e: ToolTraceEvent) {
            self.appends.lock().push((id, e));
        }
        fn finish(&self, t: ToolTrace) {
            self.finishes.lock().push(t);
        }
    }

    #[test]
    fn trace_builder_streams_and_finishes() {
        let sink = Arc::new(CollectingSink::default());
        {
            let mut tb = TraceBuilder::start(
                sample_trace_id(),
                "call-1",
                AgentRole::Implementer,
                "claude-sonnet-4-5",
                ToolFormat::AnthropicBlocks,
                1_000,
                sink.clone(),
            );
            tb.event(ToolTraceEvent::HandlerStarted {
                handler: "read_file".into(),
                category: ToolCategory::Read,
                concurrency: ToolConcurrency::Parallel,
                at_ms: 1_005,
            });
            tb.set_outcome(ToolOutcome::success(50, 0.0));
            tb.finish(1_050);
        }

        assert_eq!(sink.appends.lock().len(), 1);
        let finishes = sink.finishes.lock();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0].events.len(), 1);
        assert_eq!(finishes[0].duration_ms(), 50);
    }

    #[test]
    fn trace_builder_finish_is_idempotent() {
        let sink = Arc::new(CollectingSink::default());
        let mut tb = TraceBuilder::start(
            sample_trace_id(),
            "c",
            AgentRole::Auditor,
            "m",
            ToolFormat::ReActText,
            0,
            sink.clone(),
        );
        tb.set_outcome(ToolOutcome::success(1, 0.0));
        tb.finish(1);
        tb.finish(2); // idempotent — no double-emit
        assert_eq!(sink.finishes.lock().len(), 1);
    }

    #[test]
    fn trace_builder_drop_finalizes_unclosed_trace() {
        let sink = Arc::new(CollectingSink::default());
        {
            let mut tb = TraceBuilder::start(
                sample_trace_id(),
                "dropped",
                AgentRole::Auditor,
                "m",
                ToolFormat::ReActText,
                0,
                sink.clone(),
            );
            tb.event(ToolTraceEvent::StreamCoerced { at_ms: 1 });
            // No set_outcome, no finish — Drop should close with
            // ToolOutcome::failure(Cancelled, …).
        }
        let finishes = sink.finishes.lock();
        assert_eq!(finishes.len(), 1);
        assert!(!finishes[0].outcome.success);
        assert_eq!(finishes[0].outcome.failure, Some(FailureKind::Cancelled));
    }

    #[test]
    fn noop_trace_sink_drops_events_silently() {
        let sink = NoopTraceSink;
        sink.append(
            sample_trace_id(),
            ToolTraceEvent::StreamCoerced { at_ms: 1 },
        );
        // No panic, no state change.
    }

    // ─── classify_tool_error tests ───────────────────────────────────────

    #[test]
    fn classify_schema_invalid() {
        let e = ToolError::SchemaInvalid("missing field".into());
        assert_eq!(classify_tool_error(&e), FailureKind::SchemaInvalid);
    }

    #[test]
    fn classify_permission_denied() {
        let e = ToolError::PermissionDenied("needs write".into());
        assert_eq!(classify_tool_error(&e), FailureKind::PermissionDenied);
    }

    #[test]
    fn classify_timeout() {
        let e = ToolError::Timeout { after_ms: 5_000 };
        assert_eq!(classify_tool_error(&e), FailureKind::Timeout);
    }

    #[test]
    fn classify_cancelled() {
        assert_eq!(
            classify_tool_error(&ToolError::Cancelled),
            FailureKind::Cancelled
        );
    }

    #[test]
    fn classify_command_not_allowed() {
        let e = ToolError::CommandNotAllowed("git push".into());
        assert_eq!(classify_tool_error(&e), FailureKind::PermissionDenied);
    }

    #[test]
    fn classify_network_blocked() {
        let e = ToolError::NetworkBlocked("evil.example.com".into());
        assert_eq!(classify_tool_error(&e), FailureKind::PermissionDenied);
    }

    #[test]
    fn classify_path_outside_worktree() {
        let e = ToolError::PathOutsideWorktree(std::path::PathBuf::from("/etc/passwd"));
        assert_eq!(classify_tool_error(&e), FailureKind::PathEscape);
    }

    #[test]
    fn classify_handler_panic() {
        let e = ToolError::HandlerPanic("unwrap on None".into());
        assert_eq!(classify_tool_error(&e), FailureKind::ToolHandlerError);
    }

    #[test]
    fn classify_other() {
        let e = ToolError::Other("boom".into());
        assert_eq!(classify_tool_error(&e), FailureKind::ToolHandlerError);
    }

    #[test]
    fn classify_all_variants_covered() {
        // Exhaustive: every ToolError variant maps to a FailureKind.
        let all_errors = [
            ToolError::PermissionDenied("a".into()),
            ToolError::SchemaInvalid("b".into()),
            ToolError::HandlerPanic("c".into()),
            ToolError::Timeout { after_ms: 1 },
            ToolError::PathOutsideWorktree(std::path::PathBuf::from("/x")),
            ToolError::CommandNotAllowed("d".into()),
            ToolError::NetworkBlocked("e".into()),
            ToolError::Cancelled,
            ToolError::Other("f".into()),
        ];
        for err in &all_errors {
            // Just assert it returns *something* without panicking.
            let _kind = classify_tool_error(err);
        }
    }

    // ─── scrubbed() tests ────────────────────────────────────────────────

    #[test]
    fn scrubbed_redacts_custom_event_payload() {
        use crate::obs::scrub::LogScrubber;

        let scrubber = LogScrubber::new();
        let trace = ToolTrace {
            trace_id: sample_trace_id(),
            call_id: "call-1".into(),
            role: AgentRole::Implementer,
            model: "mock".into(),
            format_used: ToolFormat::AnthropicBlocks,
            started_at_ms: 0,
            ended_at_ms: 10,
            events: vec![ToolTraceEvent::Custom {
                name: "leak".into(),
                data: serde_json::json!({
                    "key": "sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890"
                }),
                at_ms: 1,
            }],
            outcome: ToolOutcome::success(10, 0.0),
        };
        let scrubbed = trace.scrubbed(&scrubber);

        // The secret should be gone from the custom event data.
        let json = serde_json::to_string(&scrubbed.events[0]).unwrap();
        assert!(!json.contains("sk-ant-api03"));
        assert!(json.contains("[REDACTED]"));
    }

    #[test]
    fn scrubbed_redacts_model_field() {
        use crate::obs::scrub::LogScrubber;

        let scrubber = LogScrubber::new();
        let trace = ToolTrace {
            trace_id: sample_trace_id(),
            call_id: "c".into(),
            role: AgentRole::Implementer,
            // Unlikely but tests the path: model field contains a secret.
            model: "sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890".into(),
            format_used: ToolFormat::ReActText,
            started_at_ms: 0,
            ended_at_ms: 10,
            events: Vec::new(),
            outcome: ToolOutcome::success(10, 0.0),
        };
        let scrubbed = trace.scrubbed(&scrubber);
        assert!(!scrubbed.model.contains("sk-ant-api03"));
        assert!(scrubbed.model.contains("[REDACTED]"));
    }

    #[test]
    fn scrubbed_preserves_numeric_events() {
        use crate::obs::scrub::LogScrubber;

        let scrubber = LogScrubber::new();
        let trace = ToolTrace {
            trace_id: sample_trace_id(),
            call_id: "c".into(),
            role: AgentRole::Implementer,
            model: "m".into(),
            format_used: ToolFormat::ReActText,
            started_at_ms: 0,
            ended_at_ms: 10,
            events: vec![
                ToolTraceEvent::StreamCoerced { at_ms: 42 },
                ToolTraceEvent::Truncation {
                    kept: 100,
                    total: 200,
                    at_ms: 43,
                },
            ],
            outcome: ToolOutcome::success(10, 0.0),
        };
        let scrubbed = trace.scrubbed(&scrubber);
        assert_eq!(scrubbed.events.len(), 2);
        assert_eq!(scrubbed.events[0].at_ms(), 42);
        assert_eq!(scrubbed.events[1].at_ms(), 43);
    }
}
