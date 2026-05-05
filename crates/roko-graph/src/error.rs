//! Graph engine errors.

use thiserror::Error;

/// Errors that can occur during graph execution.
#[derive(Debug, Error)]
pub enum GraphError {
    /// A node failed during execution.
    #[error("node '{node_id}' failed: {reason}")]
    NodeFailed { node_id: String, reason: String },

    /// The graph contains a cycle.
    #[error("graph contains a cycle involving node '{node_id}'")]
    CycleDetected { node_id: String },

    /// A referenced node does not exist in the graph.
    #[error("node '{node_id}' not found in graph")]
    NodeNotFound { node_id: String },

    /// An edge references a non-existent source or target.
    #[error("edge references unknown node '{node_id}'")]
    InvalidEdge { node_id: String },

    /// Budget exceeded (tokens, cost, or deadline).
    #[error("budget exceeded: {reason}")]
    BudgetExceeded { reason: String },

    /// A condition expression failed to evaluate.
    #[error("condition evaluation failed for edge {from} -> {to}: {reason}")]
    ConditionError {
        from: String,
        to: String,
        reason: String,
    },

    /// The graph definition is invalid (e.g. no nodes).
    #[error("invalid graph definition: {reason}")]
    InvalidGraph { reason: String },

    /// Underlying roko-core error.
    #[error("core error: {0}")]
    Core(#[from] roko_core::error::RokoError),

    /// Serialization error.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// TOML deserialization error.
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
}

/// Convenience Result type for graph operations.
pub type Result<T> = std::result::Result<T, GraphError>;
