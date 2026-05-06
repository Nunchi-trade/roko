//! Graph engine errors.
//!
//! The canonical error enum lives in [`crate::types::GraphError`].
//! This module re-exports it and provides a convenience `Result` alias.

pub use crate::types::GraphError;

/// Convenience Result type for graph operations.
pub type Result<T> = std::result::Result<T, GraphError>;
