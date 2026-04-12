//! Reusable widgets for the dashboard TUI.

// Shared palette — must come before widget modules that depend on it.
pub mod rosedust;

pub mod agent_grid;
pub mod agent_output;
pub mod agent_pool;
pub mod braille;
pub mod branch_tree;
pub mod command_output;
pub mod context_gauge;
pub mod diff_panel;
pub mod error_digest;
pub mod header_bar;
pub mod parallel_pool;
pub mod phase_bar;
pub mod phase_compact;
pub mod phase_timeline;
pub mod plan_list;
pub mod plan_tree;
pub mod scrollbar;
pub mod status_badge;
pub mod status_bar;
pub mod sys_metrics;
pub mod tab_bar;
pub mod task_progress;
pub mod token_bar;
pub mod token_sparkline;
pub mod wave_bar;
pub mod wave_progress;
