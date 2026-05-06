//! Runner v2 — event-driven plan executor with streaming agent output.
//!
//! This module replaces the batch-only `orchestrate.rs` plan runner with
//! a streaming architecture:
//!
//! - Agent output is parsed line-by-line from `--output-format stream-json`
//! - State is flushed to disk after every task completion
//! - TUI receives real-time updates via `StateHub`
//! - Process groups ensure clean agent teardown on Ctrl+C
//!
//! # Config resolution
//!
//! Callers should build [`RunConfig`] from the effective [`RokoConfig`] via
//! [`RunConfig::from_roko_config`] so that timeouts, gates, models, and budget
//! limits all derive from the project config. If `RunConfig.roko_config` is
//! `None` at run start, the event loop falls back to
//! [`roko_core::config::loader::load_config_unified`] using `config.workdir`
//! (ancestor walk + global merge + env overrides). The runner never performs
//! its own ad-hoc project-root resolution.
//!
//! [`RokoConfig`]: roko_core::config::schema::RokoConfig
//!
//! # Usage
//!
//! ```rust,ignore
//! use roko_cli::runner;
//!
//! let plans = runner::plan_loader::load_plans(&plan_dir)?;
//! let report = runner::run(plans, &config, &state_hub, cancel).await?;
//! ```

pub mod agent_events;
pub mod agent_stream;
pub mod event_loop;
pub mod extension_loader;
pub mod gate_dispatch;
pub mod inline_output;
pub mod merge;
pub mod output_sink;
pub mod persist;
pub mod plan_loader;
pub mod projection;
pub mod resume;
pub mod snapshot_writer;
pub mod sse_stream;
pub mod state;
pub mod task_dag;
pub mod tui_bridge;
pub mod types;

// Re-export the primary entry points.
pub use event_loop::{PlanReport, RunReport, run};
pub use plan_loader::{Plan, load_plan, load_plans, scaffold_missing_crates};
pub use sse_stream::SseStreamClient;
pub use types::RunConfig;
