//! Concrete cell implementations for the graph engine.
//!
//! - [`AgentCell`]: wraps LLM agent dispatch (send prompt, get response).
//! - [`ComposeCell`]: runs prompt assembly using roko-compose templates.
//! - [`GraduationCell`]: promotes qualifying Bus Pulses to durable Signals.
//! - [`TaskExecutorCell`]: stub cell for plan-to-graph converted tasks.
//! - [`PassthroughCell`]: stub cell that passes input through (placeholder for cognitive loop cells).

pub mod agent;
pub mod compose;
pub mod graduation;
pub mod stubs;
pub mod task_executor;

pub use agent::{AgentCell, AgentCellConfig};
pub use compose::{ComposeCell, ComposeCellConfig};
pub use graduation::GraduationCell;
pub use stubs::PassthroughCell;
pub use task_executor::TaskExecutorCell;
