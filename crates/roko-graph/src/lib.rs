//! Graph execution engine for Roko.
//!
//! Provides a DAG-based execution engine that orchestrates [`Cell`](roko_core::cell::Cell)
//! instances with support for:
//!
//! - **Fan-out/fan-in**: parallel execution of independent nodes
//! - **Conditional edges**: traverse edges only when conditions are met
//! - **Budget enforcement**: token, cost, and deadline limits
//! - **Agent and Compose cells**: concrete cell implementations for LLM dispatch
//!   and prompt assembly

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::derivable_impls,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::if_not_else,
    clippy::iter_with_drain,
    clippy::map_unwrap_or,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::option_if_let_else,
    clippy::option_option,
    clippy::redundant_closure_for_method_calls,
    clippy::suboptimal_flops,
    clippy::suspicious_operation_groupings,
    clippy::too_many_arguments,
    clippy::unnecessary_wraps,
    clippy::unused_self,
    clippy::unwrap_in_result,
    clippy::unwrap_used,
    clippy::ref_option,
    missing_docs
)]

pub mod budget;
pub mod cells;
pub mod condition;
pub mod engine;
pub mod error;
pub mod types;

pub use budget::BudgetTracker;
pub use condition::EdgeCondition;
pub use engine::GraphEngine;
pub use error::GraphError;
pub use types::{Edge, GraphConfig, GraphDef, GraphResult, Node, NodeOutput, NodeStatus};
