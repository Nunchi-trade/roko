//! Filesystem-backed [`Substrate`](roko_core::Substrate).
//!
//! `FileSubstrate` persists signals to an append-only JSONL log under a
//! directory (typically `.roko/signals/`). It keeps an in-memory index of
//! all live signals for fast querying, and rebuilds that index from the log
//! on startup.
//!
//! # Why JSONL + in-memory?
//!
//! - **Append-only** writes are crash-safe: if the process dies mid-write,
//!   worst case is a partial last line that we skip on replay.
//! - **JSONL** is human-readable, grep-able, diff-able — helpful for debugging
//!   and for users inspecting their `.roko/` directory.
//! - **In-memory index** gives us the same query latency as `MemorySubstrate`.
//!   Memory cost is low: tens of MB per million signals.
//!
//! When workload grows beyond in-memory capacity, we can swap in a different
//! backend (`SQLite`, `sled`) behind the same `Substrate` trait — the callers
//! won't change.

#![allow(clippy::module_name_repetitions)]

pub mod archive;
pub mod file_substrate;
pub mod gc;
pub mod layout;
pub mod metrics;
pub mod tool_audit;
pub mod trace_sink;

pub use archive::{ArchiveEntry, ArchiveKind, ArchiveStats, Archiver};
pub use file_substrate::FileSubstrate;
pub use gc::{GcCandidate, GcEngine, GcReport, RetentionPolicy};
pub use layout::{LayoutVersion, RokoLayout};
pub use metrics::MetricsLog;
pub use tool_audit::ToolAuditLog;
pub use trace_sink::JsonlTraceSink;
