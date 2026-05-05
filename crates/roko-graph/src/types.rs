//! Core types for graph definitions and execution results.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::condition::EdgeCondition;

/// A node in the execution graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier for this node.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// The cell type to instantiate (e.g. "agent", "compose", "gate").
    pub cell_type: String,
    /// Configuration passed to the cell (type-specific).
    #[serde(default)]
    pub config: HashMap<String, toml::Value>,
}

/// An edge connecting two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Condition under which this edge is traversed.
    #[serde(default = "EdgeCondition::always")]
    pub condition: EdgeCondition,
}

/// Complete graph definition: nodes + edges + configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDef {
    /// Human-readable name for this graph.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: String,
    /// Nodes in the graph.
    pub nodes: Vec<Node>,
    /// Edges between nodes.
    pub edges: Vec<Edge>,
    /// Execution configuration.
    #[serde(default)]
    pub config: GraphConfig,
}

/// Execution configuration for a graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphConfig {
    /// Maximum total tokens allowed across all node executions.
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Maximum total cost in USD allowed.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Maximum wall-clock time for the entire graph execution.
    #[serde(default, with = "optional_duration_secs")]
    pub deadline: Option<Duration>,
    /// Maximum number of nodes to execute in parallel.
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: usize,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_tokens: None,
            max_cost_usd: None,
            deadline: None,
            max_parallelism: default_max_parallelism(),
        }
    }
}

fn default_max_parallelism() -> usize {
    8
}

/// Status of a node after execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    /// Not yet executed.
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Success,
    /// Failed with an error.
    Failed { reason: String },
    /// Skipped (condition not met or budget exceeded).
    Skipped { reason: String },
}

impl NodeStatus {
    /// Returns true if the node completed successfully.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns true if the node failed.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// Output produced by a single node execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// The node's ID.
    pub node_id: String,
    /// Execution status.
    pub status: NodeStatus,
    /// Output data (cell-type-specific JSON).
    #[serde(default)]
    pub data: serde_json::Value,
    /// Tokens consumed by this node.
    #[serde(default)]
    pub tokens_used: u64,
    /// Cost in USD for this node.
    #[serde(default)]
    pub cost_usd: f64,
    /// Wall-clock duration for this node.
    #[serde(default, with = "duration_millis")]
    pub duration: Duration,
}

impl NodeOutput {
    /// Create a successful output with data.
    #[must_use]
    pub fn success(node_id: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeStatus::Success,
            data,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }

    /// Create a failed output.
    #[must_use]
    pub fn failed(node_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeStatus::Failed {
                reason: reason.into(),
            },
            data: serde_json::Value::Null,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }

    /// Create a skipped output.
    #[must_use]
    pub fn skipped(node_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeStatus::Skipped {
                reason: reason.into(),
            },
            data: serde_json::Value::Null,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }
}

/// Result of executing an entire graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphResult {
    /// Name of the graph that was executed.
    pub graph_name: String,
    /// Per-node outputs in execution order.
    pub node_outputs: Vec<NodeOutput>,
    /// Overall status.
    pub status: GraphStatus,
    /// Total tokens consumed.
    pub total_tokens: u64,
    /// Total cost in USD.
    pub total_cost_usd: f64,
    /// Total wall-clock time.
    #[serde(with = "duration_millis")]
    pub total_duration: Duration,
}

/// Overall graph execution status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphStatus {
    /// All nodes completed successfully.
    Success,
    /// At least one node failed.
    PartialFailure,
    /// Execution was cut short due to budget limits.
    BudgetExceeded { reason: String },
    /// Execution was cancelled.
    Cancelled,
}

// ─── Serde helpers for Duration ─────────────────────────────────────────────

mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(dur: &Duration, s: S) -> Result<S::Ok, S::Error> {
        dur.as_millis().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(d)?;
        Ok(Duration::from_millis(ms))
    }
}

mod optional_duration_secs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(dur: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        dur.map(|d| d.as_secs()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt = Option::<u64>::deserialize(d)?;
        Ok(opt.map(Duration::from_secs))
    }
}
