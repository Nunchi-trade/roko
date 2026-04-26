//! Reusable output primitives for the inline CLI experience.
//!
//! Each primitive is a self-contained data structure + renderer that produces
//! styled ratatui `Line`s. They compose into higher-level experiences:
//!
//! - `roko chat`: StreamingBlock + ToolCallBlock + RunBlock + CostMeter
//! - `roko run`: PredictionBlock + StreamingBlock + GateBlock + RunBlock
//! - `roko audit`: AuditStepBlock + GateBlock + RunBlock
//! - `roko plan run`: ProgressTree + RunBlock per task

pub mod cost_meter;
pub mod run_block;
pub mod streaming;
pub mod tool_call;

pub use cost_meter::CostMeter;
pub use run_block::{KnowledgeInfo, RunBlockData, ToolCallInfo};
pub use streaming::StreamingState;
pub use tool_call::ToolCallBlock;
