//! Core graph types: `Graph`, `Node`, `Edge`, `NodeId`, `EdgeCondition`, `GraphMetadata`,
//! `NodeOutput`, `NodeOutputStatus`, `GraphConfig`.

use std::collections::HashMap;
use std::time::Duration;

use indexmap::IndexMap;
use petgraph::graph::DiGraph;
use serde::{Deserialize, Serialize};

/// Unique identifier for a node within a graph.
pub type NodeId = String;

/// Condition that gates execution along an edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum EdgeCondition {
    /// Edge fires only if the source node succeeded.
    Success,
    /// Edge fires only if the source node failed.
    Failure,
    /// Edge fires only if the named output equals the given value.
    OutputEquals {
        /// Output key to check.
        key: String,
        /// Expected value.
        value: String,
    },
    /// Edge always fires (unconditional dependency).
    Always,
}

/// Metadata associated with a graph definition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphMetadata {
    /// Human-readable name of the graph.
    pub name: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional version string.
    #[serde(default)]
    pub version: Option<String>,
    /// Arbitrary key-value annotations.
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

/// Return an empty TOML table (used as default for node config).
fn default_config() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

/// A node in the execution graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Unique identifier within this graph.
    pub id: NodeId,
    /// Cell type name used to look up the factory in `CellRegistry`.
    pub cell_type: String,
    /// Configuration passed to the cell factory function.
    #[serde(default = "default_config")]
    pub config: toml::Value,
    /// Named inputs this node consumes (from upstream edges).
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Named outputs this node produces (for downstream edges).
    #[serde(default)]
    pub outputs: Vec<String>,
}

/// An edge connecting two nodes in the execution graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// Source node ID.
    pub from: NodeId,
    /// Target node ID.
    pub to: NodeId,
    /// Optional condition for this edge to fire.
    #[serde(default)]
    pub condition: Option<EdgeCondition>,
}

/// Index type used by petgraph for node indices.
pub type GraphNodeIdx = petgraph::graph::NodeIndex;

/// A directed acyclic graph of execution nodes, backed by petgraph.
#[derive(Debug, Clone)]
pub struct Graph {
    /// Graph metadata (name, description, labels).
    pub metadata: GraphMetadata,
    /// The underlying petgraph directed graph.
    pub inner: DiGraph<Node, Edge>,
    /// Mapping from `NodeId` (string) to petgraph node index.
    pub node_map: IndexMap<NodeId, GraphNodeIdx>,
}

impl Graph {
    /// Create a new empty graph with the given metadata.
    #[must_use]
    pub fn new(metadata: GraphMetadata) -> Self {
        Self {
            metadata,
            inner: DiGraph::new(),
            node_map: IndexMap::new(),
        }
    }

    /// Add a node to the graph. Returns the petgraph index.
    ///
    /// # Errors
    /// Returns an error if a node with the same ID already exists.
    pub fn add_node(&mut self, node: Node) -> Result<GraphNodeIdx, GraphError> {
        if self.node_map.contains_key(&node.id) {
            return Err(GraphError::DuplicateNode(node.id));
        }
        let id = node.id.clone();
        let idx = self.inner.add_node(node);
        self.node_map.insert(id, idx);
        Ok(idx)
    }

    /// Add an edge between two existing nodes.
    ///
    /// # Errors
    /// Returns an error if either node ID is not found.
    pub fn add_edge(&mut self, edge: Edge) -> Result<(), GraphError> {
        let from_idx = self
            .node_map
            .get(&edge.from)
            .copied()
            .ok_or_else(|| GraphError::NodeNotFound(edge.from.clone()))?;
        let to_idx = self
            .node_map
            .get(&edge.to)
            .copied()
            .ok_or_else(|| GraphError::NodeNotFound(edge.to.clone()))?;
        self.inner.add_edge(from_idx, to_idx, edge);
        Ok(())
    }

    /// Get a node by its string ID.
    #[must_use]
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.node_map.get(id).map(|idx| &self.inner[*idx])
    }

    /// Return the number of nodes in the graph.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.inner.node_count()
    }

    /// Return the number of edges in the graph.
    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.inner.edge_count()
    }
}

/// Errors specific to graph construction, validation, and execution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// A node with this ID already exists.
    #[error("duplicate node: `{0}`")]
    DuplicateNode(NodeId),
    /// Referenced node ID does not exist in the graph.
    #[error("node not found: `{0}`")]
    NodeNotFound(NodeId),
    /// The graph contains a cycle and cannot be topologically sorted.
    #[error("cycle detected in graph")]
    CycleDetected,
    /// TOML parsing or schema error.
    #[error("loader error: {0}")]
    LoaderError(String),
    /// Cell type not found in registry.
    #[error("unknown cell type: `{0}`")]
    UnknownCellType(String),
    /// A node failed during execution.
    #[error("node '{node_id}' failed: {reason}")]
    NodeFailed {
        /// The node that failed.
        node_id: String,
        /// Description of the failure.
        reason: String,
    },
    /// An edge references a non-existent source or target.
    #[error("edge references unknown node '{node_id}'")]
    InvalidEdge {
        /// The referenced node ID.
        node_id: String,
    },
    /// Budget exceeded (tokens, cost, or deadline).
    #[error("budget exceeded: {reason}")]
    BudgetExceeded {
        /// Description of which limit was breached.
        reason: String,
    },
    /// A condition expression failed to evaluate.
    #[error("condition evaluation failed for edge {from} -> {to}: {reason}")]
    ConditionError {
        /// Source node.
        from: String,
        /// Target node.
        to: String,
        /// What went wrong.
        reason: String,
    },
    /// The graph definition is invalid.
    #[error("invalid graph definition: {reason}")]
    InvalidGraph {
        /// What is wrong with the graph.
        reason: String,
    },
}

// ─── Node output types (used by condition evaluation & cell modules) ─────────

/// Status of a node's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeOutputStatus {
    /// Node completed successfully.
    Success,
    /// Node failed during execution.
    Failed,
    /// Node was skipped (e.g., budget exceeded, upstream failure).
    Skipped,
}

impl NodeOutputStatus {
    /// Returns `true` if this status represents success.
    #[must_use]
    pub fn is_success(self) -> bool {
        matches!(self, Self::Success)
    }

    /// Returns `true` if this status represents failure.
    #[must_use]
    pub fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    /// Returns `true` if this status represents a skipped node.
    #[must_use]
    pub fn is_skipped(self) -> bool {
        matches!(self, Self::Skipped)
    }
}

/// Output produced by a single node execution, used for condition evaluation
/// and inter-cell communication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// The node that produced this output.
    pub node_id: NodeId,
    /// Execution status.
    pub status: NodeOutputStatus,
    /// Structured output data (JSON).
    pub data: serde_json::Value,
    /// Error message if status is `Failed` or `Skipped`.
    pub error: Option<String>,
    /// Tokens consumed during execution.
    #[serde(default)]
    pub tokens_used: u64,
    /// Estimated cost in USD.
    #[serde(default)]
    pub cost_usd: f64,
    /// Wall-clock duration of execution.
    #[serde(default, with = "duration_millis")]
    pub duration: Duration,
}

impl NodeOutput {
    /// Create a successful output with the given data.
    pub fn success(node_id: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeOutputStatus::Success,
            data,
            error: None,
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }

    /// Create a failed output with the given error message.
    pub fn failed(node_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeOutputStatus::Failed,
            data: serde_json::Value::Null,
            error: Some(error.into()),
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }

    /// Create a skipped output with the given reason.
    pub fn skipped(node_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            status: NodeOutputStatus::Skipped,
            data: serde_json::Value::Null,
            error: Some(reason.into()),
            tokens_used: 0,
            cost_usd: 0.0,
            duration: Duration::ZERO,
        }
    }
}

/// Serde helper: serialize/deserialize `Duration` as milliseconds (u64).
mod duration_millis {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Duration, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_u64(d.as_millis() as u64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Duration, D::Error> {
        let ms = u64::deserialize(de)?;
        Ok(Duration::from_millis(ms))
    }
}

// ─── Graph-level configuration (used by budget tracker) ─────────────────────

/// Graph-level configuration for resource limits and execution policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphConfig {
    /// Maximum total tokens allowed.
    #[serde(default)]
    pub max_tokens: Option<u64>,
    /// Maximum total cost in USD.
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Maximum wall-clock time.
    #[serde(default)]
    pub deadline: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_empty_graph() {
        let meta = GraphMetadata {
            name: "test-graph".to_string(),
            ..Default::default()
        };
        let graph = Graph::new(meta);
        assert_eq!(graph.metadata.name, "test-graph");
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn add_nodes_and_edges() {
        let mut graph = Graph::new(GraphMetadata {
            name: "pipeline".to_string(),
            ..Default::default()
        });

        let compile_node = Node {
            id: "compile".to_string(),
            cell_type: "gate.compile".to_string(),
            config: toml::Value::Table(toml::map::Map::new()),
            inputs: vec![],
            outputs: vec!["success".to_string()],
        };
        let test_node = Node {
            id: "test".to_string(),
            cell_type: "gate.test".to_string(),
            config: toml::Value::Table(toml::map::Map::new()),
            inputs: vec!["compile.success".to_string()],
            outputs: vec![],
        };

        graph.add_node(compile_node).unwrap();
        graph.add_node(test_node).unwrap();

        let edge = Edge {
            from: "compile".to_string(),
            to: "test".to_string(),
            condition: Some(EdgeCondition::Success),
        };
        graph.add_edge(edge).unwrap();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
        assert!(graph.get_node("compile").is_some());
        assert!(graph.get_node("test").is_some());
        assert!(graph.get_node("nonexistent").is_none());
    }

    #[test]
    fn duplicate_node_errors() {
        let mut graph = Graph::new(GraphMetadata::default());
        let node = Node {
            id: "a".to_string(),
            cell_type: "noop".to_string(),
            config: toml::Value::Table(toml::map::Map::new()),
            inputs: vec![],
            outputs: vec![],
        };
        graph.add_node(node.clone()).unwrap();
        let result = graph.add_node(node);
        assert_eq!(result, Err(GraphError::DuplicateNode("a".to_string())));
    }

    #[test]
    fn edge_to_nonexistent_node_errors() {
        let mut graph = Graph::new(GraphMetadata::default());
        let node = Node {
            id: "a".to_string(),
            cell_type: "noop".to_string(),
            config: toml::Value::Table(toml::map::Map::new()),
            inputs: vec![],
            outputs: vec![],
        };
        graph.add_node(node).unwrap();

        let edge = Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            condition: None,
        };
        assert_eq!(
            graph.add_edge(edge),
            Err(GraphError::NodeNotFound("b".to_string()))
        );
    }

    #[test]
    fn edge_condition_serde_roundtrip() {
        let cond = EdgeCondition::OutputEquals {
            key: "status".to_string(),
            value: "ok".to_string(),
        };
        let serialized = toml::to_string(&cond).unwrap();
        let deserialized: EdgeCondition = toml::from_str(&serialized).unwrap();
        assert_eq!(cond, deserialized);
    }

    #[test]
    fn node_output_success() {
        let output = NodeOutput::success("n1", serde_json::json!({"result": "ok"}));
        assert!(output.status.is_success());
        assert!(!output.status.is_failed());
        assert!(!output.status.is_skipped());
        assert_eq!(output.node_id, "n1");
        assert!(output.error.is_none());
    }

    #[test]
    fn node_output_failed() {
        let output = NodeOutput::failed("n1", "boom");
        assert!(output.status.is_failed());
        assert!(!output.status.is_success());
        assert_eq!(output.error.as_deref(), Some("boom"));
    }

    #[test]
    fn node_output_skipped() {
        let output = NodeOutput::skipped("n1", "budget exceeded");
        assert!(output.status.is_skipped());
        assert!(!output.status.is_success());
        assert!(!output.status.is_failed());
    }
}
