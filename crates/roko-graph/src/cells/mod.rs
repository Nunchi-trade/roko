//! Concrete cell implementations for the graph engine.
//!
//! - [`AgentCell`]: wraps LLM agent dispatch (send prompt, get response).
//! - [`ComposeCell`]: runs prompt assembly using roko-compose templates.

pub mod agent;
pub mod compose;

pub use agent::{AgentCell, AgentCellConfig};
pub use compose::{ComposeCell, ComposeCellConfig};
