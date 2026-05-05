//! Inline terminal rendering engine.
//!
//! Provides a Claude Code-like CLI experience using ratatui's
//! `Viewport::Inline`. Completed blocks scroll into terminal history,
//! live content stays in a fixed viewport at the bottom.
//!
//! # Modules
//!
//! - [`primitives`] — 11 reusable data+rendering blocks
//! - [`styled`] — Low-level styled line builders
//! - [`symbols`] — Clack-style glyphs
//! - [`terminal`] — InlineTerminal wrapper
//! - [`markdown`] — Markdown → styled ratatui Lines
//! - [`agent_events`] — Typed event stream from agent WebSocket
//! - [`plaintext`] — Non-TTY fallback renderer

pub mod agent_events;
pub mod markdown;
pub mod plaintext;
pub mod primitives;
pub mod styled;
pub mod symbols;
pub mod terminal;

pub use agent_events::{AgentEvent, AgentEventStream};
pub use plaintext::{lines_to_plain, print_plain};
pub use primitives::{
    CostMeter, CostWaterfallData, DiffBlockData, DiffEntry, ErrorBlockData, ErrorSeverity,
    GateBlockData, GateRung, GateStatus, KnowledgeInfo, ProgressTreeData, ReplanBlockData,
    RetryInfo, RunBlockData, SessionSummaryData, StreamingState, TaskProgress, ToolCallBlock,
    ToolCallInfo, TreeTask, TreeWave, WaterfallEntry,
};
pub use terminal::{InlineTerminal, RawModeGuard, should_use_inline};
