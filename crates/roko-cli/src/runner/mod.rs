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
pub mod merge;
pub mod persist;
pub mod plan_loader;
pub mod projection;
pub mod resume;
pub mod state;
pub mod task_dag;
pub mod tui_bridge;
pub mod types;

// Re-export the primary entry points.
pub use event_loop::{PlanReport, RunReport, run};
pub use plan_loader::{Plan, load_plan, load_plans};
pub use types::RunConfig;
