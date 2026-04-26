//! Reusable output primitives for the inline CLI experience.
//!
//! Each primitive is a self-contained data + renderer producing styled
//! ratatui `Line`s. They compose into higher-level experiences:
//!
//! - `roko chat`: StreamingBlock + ToolCallBlock + RunBlock + CostMeter
//! - `roko run`: RunBlock + GateBlock + SessionSummary
//! - `roko plan run`: ProgressTree + RunBlock + CostWaterfall + ReplanBlock
//! - `roko audit`: GateBlock + ErrorBlock + DiffBlock

pub mod cost_meter;
pub mod cost_waterfall;
pub mod diff_block;
pub mod error_block;
pub mod gate_block;
pub mod progress_tree;
pub mod replan_block;
pub mod run_block;
pub mod session_summary;
pub mod streaming;
pub mod tool_call;

pub use cost_meter::CostMeter;
pub use cost_waterfall::{CostWaterfallData, WaterfallEntry};
pub use diff_block::{DiffBlockData, DiffEntry};
pub use error_block::{ErrorBlockData, ErrorSeverity, RetryInfo};
pub use gate_block::{GateBlockData, GateRung, GateStatus};
pub use progress_tree::{ProgressTreeData, TaskProgress, TreeTask, TreeWave};
pub use replan_block::ReplanBlockData;
pub use run_block::{KnowledgeInfo, RunBlockData, ToolCallInfo};
pub use session_summary::SessionSummaryData;
pub use streaming::StreamingState;
pub use tool_call::ToolCallBlock;
