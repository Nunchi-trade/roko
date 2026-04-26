//! Inline terminal rendering engine.
//!
//! This module provides a Claude Code-like CLI experience using ratatui's
//! `Viewport::Inline`. A fixed-height viewport stays at the bottom of the
//! terminal for live content (streaming, spinners, status bars), while
//! completed blocks are pushed into terminal scrollback via `insert_before`.
//!
//! # Architecture
//!
//! ```text
//! Terminal scrollback (grows upward via insert_before)
//! ┌──────────────────────────────────────────────────┐
//! │ ◆ agent  auditor@v1 · eid://roko/auditor.v1     │ ← RunBlock
//! │ │ predict  $0.043 · route: haiku                 │
//! │ │ ▸ ReadFile src/auth.rs (0.2s)                  │ ← ToolCallBlock
//! │ │ actual   $0.031 (-28%)                         │
//! │ └ deposited 2 engrams                            │
//! ├──────────────────────────────────────────────────┤
//! │ ◌ Thinking... (2.3s)                             │ ← StreamingState
//! │ │ The analysis shows...█                         │    (live viewport)
//! │ $0.018 · 2,341 tokens · haiku                    │ ← status bar
//! └──────────────────────────────────────────────────┘
//! ```
//!
//! # Modules
//!
//! - [`primitives`] — Reusable data+rendering blocks (RunBlock, ToolCallBlock, etc.)
//! - [`styled`] — Low-level styled line builders (section_start, gates_line, etc.)
//! - [`symbols`] — Clack-style glyphs (◆│└✔✖⚠→)
//! - [`terminal`] — InlineTerminal wrapper (viewport + insert_before)
//! - [`markdown`] — Markdown → styled ratatui Text (static + streaming)
//! - [`agent_events`] — Typed event stream from agent WebSocket/SSE

pub mod agent_events;
pub mod markdown;
pub mod primitives;
pub mod styled;
pub mod symbols;
pub mod terminal;

pub use agent_events::{AgentEvent, AgentEventStream};
pub use primitives::{CostMeter, KnowledgeInfo, RunBlockData, StreamingState, ToolCallBlock, ToolCallInfo};
pub use terminal::{InlineTerminal, should_use_inline};
