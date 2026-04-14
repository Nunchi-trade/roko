#![allow(clippy::doc_markdown)]
//! Tool registry, call/result types, and dispatch abstractions (§36.a).
//!
//! Mori delegates tool definitions to the Claude CLI: it just passes
//! `--tools Read,Edit,Bash,…` and lets Claude Code own the tools.
//! That model breaks for **raw-LLM backends** (Ollama, OpenAI API),
//! where there are no built-in tools — Roko must define, execute, and
//! marshal every tool itself.
//!
//! This module provides the canonical tool abstractions used by **all**
//! backends:
//!
//! - **Hosted backends** (Claude CLI, Codex, Cursor) name-alias Roko's
//!   canonical tools into their native tool menus. Roko's definitions
//!   are advisory and used for client-side validation before dispatch.
//! - **Raw backends** (Ollama, OpenAI) use Roko's definitions verbatim
//!   as the `tools[]` field of their API request and run the full
//!   tool-call loop locally.
//!
//! The same 16 canonical tools (§36.b) are built in `roko-std`; their
//! [`ToolDef`]s register into a `StaticToolRegistry` (§36.9) that both
//! backend families consult.
//!
//! # Key types
//!
//! | Type | Purpose |
//! |---|---|
//! | [`ToolDef`] | Name, schema, category, permissions, timeout, concurrency |
//! | [`ToolSchema`] | JSON Schema describing the tool's arguments |
//! | [`ToolCategory`] | Read / Write / Exec / Git / Network / Meta / Notebook / Mcp |
//! | [`ToolPermission`] | Capability flags the tool requires at dispatch |
//! | [`ToolConcurrency`] | Serial vs Parallel (for §36.41 fan-out) |
//! | [`ToolCall`] | Inbound invocation parsed from an LLM response |
//! | [`ToolResult`] | Outbound result (content + artifacts or typed error) |
//! | [`ToolError`] | Typed failure modes (permission, schema, timeout, …) |
//! | [`Artifact`] | Side-channel artifact (file, diff, image) |
//! | [`ToolRegistry`] | Trait: lookup + per-role filtering |
//! | [`VecToolRegistry`] | Trivial `Vec`-backed registry for tests |
//! | [`ToolHandler`] | Async executor trait |
//! | [`ToolContext`] | Runtime capabilities passed to handlers |
//! | [`AuditSink`] | Audit-signal sink trait |
//! | [`CancelToken`] | Cancellation signal trait |
//! | [`aliases`] | Canonical ↔ hosted-backend name mapping |
//! | [`role_allowlist`] | Derive per-role allowlist from [`ToolPermission`] |
//!
//! ## Research-driven adaptive layer (§36.j–u)
//!
//! | Type / module | Purpose |
//! |---|---|
//! | [`ToolFormat`] | `OpenAiJson`, `AnthropicBlocks`, `HermesJson`, `Gemma4Tokens`, `MistralTokens`, `Pythonic`, `QwenXml`, `ReActText`, `JsonMode`, `Custom` |
//! | [`ToolFormatProfile`] + [`profile_for_model`] | Per-model metadata: preferred format, fallback chain, parallel safety, tool-count threshold, stream-disable, tool-call-id length |
//! | [`ToolTrace`] + [`ToolTraceEvent`] | Full execution trace of one tool call (14+ event kinds) |
//! | [`FailureTrace`] + [`FailureKind`] | Structured root cause with evidence + contributing event indices |
//! | [`ToolOutcome`] | Terminal reward/latency/cost record |
//! | [`TraceSink`] + [`TraceBuilder`] | Runtime-agnostic trace sinks + RAII assembly |
//! | [`ToolMetrics`] + [`MetricsSink`] + [`MetricsKey`] | Aggregated PHR/PMR/TSQ/schema/arg/selection metrics |
//! | [`compute_reward`] + [`RewardConfig`] | Composite bandit reward |
//! | [`FormatBandit`] + [`BanditKey`] + [`ArmEntry`] | Adaptive format selection |
//! | [`ProfileBandit`], [`EpsilonGreedyBandit`] | Day-one bandit impls (Track-and-Stop lives in `roko-learn`) |
//! | [`MemoryPointer`] | Large-tool-result pointer for context-pressure mitigation |
//! | [`ToolRelevanceScorer`] + [`KeywordOverlapScorer`] | Progressive tool discovery |

pub mod aliases;
pub mod bandit;
pub mod call;
pub mod def;
pub mod discovery;
pub mod format;
pub mod handler;
pub mod metrics;
pub mod pointer;
pub mod registry;
pub mod relevance;
pub mod role_allowlist;
pub mod trace;

pub use aliases::{ALIASES, ToolAlias};
pub use bandit::{ArmEntry, BanditKey, EpsilonGreedyBandit, FormatBandit, ProfileBandit};
pub use call::{Artifact, ToolCall, ToolError, ToolResult};
pub use def::{ToolCategory, ToolConcurrency, ToolDef, ToolPermission, ToolSchema, ToolSource};
pub use format::{ToolFormat, ToolFormatProfile, profile_for_model};
pub use handler::{
    AtomicCancel, AuditSink, CancelToken, ExternalAction, NeverCancel, NoopAuditSink, ToolContext,
    ToolHandler,
};
pub use metrics::{
    MetricsKey, MetricsSink, NoopMetricsSink, RewardConfig, ToolMetrics, compute_reward,
    galileo_tsq,
};
pub use pointer::MemoryPointer;
pub use registry::{ToolRegistry, VecToolRegistry};
pub use relevance::{KeywordOverlapScorer, ToolRelevanceScorer};
pub use role_allowlist::role_allowlist;
pub use trace::{
    CancelSource, FailureKind, FailureTrace, NoopTraceSink, ToolOutcome, ToolTrace, ToolTraceEvent,
    TraceBuilder, TraceId, TraceSink, TraceStep,
};
