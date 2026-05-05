//! TOML loader: parse graph definitions from TOML files into `Graph` structs.

use std::path::Path;

use serde::Deserialize;

use crate::types::{Edge, EdgeCondition, Graph, GraphError, GraphMetadata, Node};

/// Raw TOML representation of a graph file (for deserialization).
#[derive(Debug, Deserialize)]
struct RawGraphFile {
    graph: RawGraphMeta,
    #[serde(default)]
    nodes: Vec<RawNode>,
    #[serde(default)]
    edges: Vec<RawEdge>,
}

/// Raw metadata section.
#[derive(Debug, Deserialize)]
struct RawGraphMeta {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    labels: Option<std::collections::HashMap<String, String>>,
}

/// Raw node definition from TOML.
#[derive(Debug, Deserialize)]
struct RawNode {
    id: String,
    cell_type: String,
    #[serde(default)]
    config: Option<toml::Value>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    outputs: Vec<String>,
}

/// Raw edge definition from TOML.
#[derive(Debug, Deserialize)]
struct RawEdge {
    from: String,
    to: String,
    #[serde(default)]
    condition: Option<RawEdgeCondition>,
}

/// Raw edge condition from TOML.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum RawEdgeCondition {
    #[serde(alias = "success")]
    Success,
    #[serde(alias = "failure")]
    Failure,
    #[serde(alias = "always")]
    Always,
    #[serde(alias = "output_equals")]
    OutputEquals { key: String, value: String },
}

impl From<RawEdgeCondition> for EdgeCondition {
    fn from(raw: RawEdgeCondition) -> Self {
        match raw {
            RawEdgeCondition::Success => EdgeCondition::Success,
            RawEdgeCondition::Failure => EdgeCondition::Failure,
            RawEdgeCondition::Always => EdgeCondition::Always,
            RawEdgeCondition::OutputEquals { key, value } => {
                EdgeCondition::OutputEquals { key, value }
            }
        }
    }
}

/// Load a graph from a TOML string.
///
/// # Errors
/// Returns `GraphError::LoaderError` on parse failures, or `GraphError::DuplicateNode` /
/// `GraphError::NodeNotFound` if the graph definition is inconsistent.
pub fn load_from_str(toml_str: &str) -> Result<Graph, GraphError> {
    let raw: RawGraphFile =
        toml::from_str(toml_str).map_err(|e| GraphError::LoaderError(e.to_string()))?;

    let metadata = GraphMetadata {
        name: raw.graph.name,
        description: raw.graph.description,
        version: raw.graph.version,
        labels: raw.graph.labels.unwrap_or_default(),
    };

    let mut graph = Graph::new(metadata);

    // Add all nodes first.
    for raw_node in raw.nodes {
        let node = Node {
            id: raw_node.id,
            cell_type: raw_node.cell_type,
            config: raw_node.config.unwrap_or(toml::Value::Table(toml::map::Map::new())),
            inputs: raw_node.inputs,
            outputs: raw_node.outputs,
        };
        graph.add_node(node)?;
    }

    // Then add edges (referencing existing nodes).
    for raw_edge in raw.edges {
        let edge = Edge {
            from: raw_edge.from,
            to: raw_edge.to,
            condition: raw_edge.condition.map(Into::into),
        };
        graph.add_edge(edge)?;
    }

    Ok(graph)
}

/// Load a graph from a TOML file on disk.
///
/// # Errors
/// Returns `GraphError::LoaderError` if the file cannot be read or parsed.
pub fn load_from_file(path: &Path) -> Result<Graph, GraphError> {
    let content =
        std::fs::read_to_string(path).map_err(|e| GraphError::LoaderError(e.to_string()))?;
    load_from_str(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_GRAPH: &str = r#"
[graph]
name = "ci-pipeline"
description = "Compile then test"
version = "1.0.0"

[[nodes]]
id = "compile"
cell_type = "gate.compile"
inputs = []
outputs = ["artifact"]
[nodes.config]
workspace = "."

[[nodes]]
id = "test"
cell_type = "gate.test"
inputs = ["compile.artifact"]
outputs = ["report"]
[nodes.config]
timeout_secs = 300

[[edges]]
from = "compile"
to = "test"
[edges.condition]
type = "success"
"#;

    #[test]
    fn load_basic_graph_from_str() {
        let graph = load_from_str(SAMPLE_GRAPH).unwrap();
        assert_eq!(graph.metadata.name, "ci-pipeline");
        assert_eq!(graph.metadata.description.as_deref(), Some("Compile then test"));
        assert_eq!(graph.metadata.version.as_deref(), Some("1.0.0"));
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        let compile = graph.get_node("compile").unwrap();
        assert_eq!(compile.cell_type, "gate.compile");
        assert_eq!(compile.outputs, vec!["artifact"]);

        let test = graph.get_node("test").unwrap();
        assert_eq!(test.cell_type, "gate.test");
        assert_eq!(test.inputs, vec!["compile.artifact"]);
    }

    #[test]
    fn load_graph_without_edges() {
        let toml_str = r#"
[graph]
name = "solo"

[[nodes]]
id = "only"
cell_type = "noop"
"#;
        let graph = load_from_str(toml_str).unwrap();
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn load_graph_edge_to_missing_node_fails() {
        let toml_str = r#"
[graph]
name = "bad"

[[nodes]]
id = "a"
cell_type = "noop"

[[edges]]
from = "a"
to = "b"
"#;
        let result = load_from_str(toml_str);
        assert!(matches!(result, Err(GraphError::NodeNotFound(ref id)) if id == "b"));
    }

    #[test]
    fn load_invalid_toml_fails() {
        let result = load_from_str("not valid toml {{{{");
        assert!(matches!(result, Err(GraphError::LoaderError(_))));
    }

    #[test]
    fn load_from_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.toml");
        std::fs::write(&path, SAMPLE_GRAPH).unwrap();

        let graph = load_from_file(&path).unwrap();
        assert_eq!(graph.metadata.name, "ci-pipeline");
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn load_graph_with_labels() {
        let toml_str = r#"
[graph]
name = "labeled"
[graph.labels]
team = "platform"
priority = "high"

[[nodes]]
id = "a"
cell_type = "noop"
"#;
        let graph = load_from_str(toml_str).unwrap();
        assert_eq!(graph.metadata.labels.get("team").unwrap(), "platform");
        assert_eq!(graph.metadata.labels.get("priority").unwrap(), "high");
    }
}
