//! Topological sort: order graph nodes for execution, detect cycles, resolve dependencies.

use petgraph::Direction;
use petgraph::algo::toposort;

use crate::types::{Graph, GraphError, NodeId};

/// Perform a topological sort on the graph, returning node IDs in execution order.
///
/// Nodes with no dependencies come first; nodes that depend on others come later.
///
/// # Errors
/// Returns `GraphError::CycleDetected` if the graph contains a cycle.
pub fn topological_order(graph: &Graph) -> Result<Vec<NodeId>, GraphError> {
    let sorted = toposort(&graph.inner, None).map_err(|_| GraphError::CycleDetected)?;

    Ok(sorted
        .into_iter()
        .map(|idx| graph.inner[idx].id.clone())
        .collect())
}

/// Return the immediate dependencies (predecessors) of a node.
///
/// These are the nodes whose edges point *to* the given node.
#[must_use]
pub fn dependencies(graph: &Graph, node_id: &str) -> Vec<NodeId> {
    let Some(&idx) = graph.node_map.get(node_id) else {
        return vec![];
    };

    graph
        .inner
        .neighbors_directed(idx, Direction::Incoming)
        .map(|pred_idx| graph.inner[pred_idx].id.clone())
        .collect()
}

/// Return the immediate dependents (successors) of a node.
///
/// These are the nodes that depend on the given node.
#[must_use]
pub fn dependents(graph: &Graph, node_id: &str) -> Vec<NodeId> {
    let Some(&idx) = graph.node_map.get(node_id) else {
        return vec![];
    };

    graph
        .inner
        .neighbors_directed(idx, Direction::Outgoing)
        .map(|succ_idx| graph.inner[succ_idx].id.clone())
        .collect()
}

/// Check if the graph is a valid DAG (no cycles).
#[must_use]
pub fn is_dag(graph: &Graph) -> bool {
    toposort(&graph.inner, None).is_ok()
}

/// Return nodes that have no incoming edges (root/entry nodes).
#[must_use]
pub fn root_nodes(graph: &Graph) -> Vec<NodeId> {
    graph
        .node_map
        .iter()
        .filter(|(_, idx)| {
            graph
                .inner
                .neighbors_directed(**idx, Direction::Incoming)
                .next()
                .is_none()
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// Return nodes that have no outgoing edges (leaf/terminal nodes).
#[must_use]
pub fn leaf_nodes(graph: &Graph) -> Vec<NodeId> {
    graph
        .node_map
        .iter()
        .filter(|(_, idx)| {
            graph
                .inner
                .neighbors_directed(**idx, Direction::Outgoing)
                .next()
                .is_none()
        })
        .map(|(id, _)| id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::types::{Edge, GraphMetadata, Node};

    use super::*;

    fn make_node(id: &str) -> Node {
        Node {
            id: id.to_string(),
            cell_type: "noop".to_string(),
            config: toml::Value::Table(toml::map::Map::new()),
            inputs: vec![],
            outputs: vec![],
        }
    }

    fn make_edge(from: &str, to: &str) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            condition: None,
        }
    }

    #[test]
    fn topological_sort_linear_chain() {
        let mut graph = Graph::new(GraphMetadata {
            name: "chain".to_string(),
            ..Default::default()
        });
        graph.add_node(make_node("a")).unwrap();
        graph.add_node(make_node("b")).unwrap();
        graph.add_node(make_node("c")).unwrap();
        graph.add_edge(make_edge("a", "b")).unwrap();
        graph.add_edge(make_edge("b", "c")).unwrap();

        let order = topological_order(&graph).unwrap();
        let pos_a = order.iter().position(|x| x == "a").unwrap();
        let pos_b = order.iter().position(|x| x == "b").unwrap();
        let pos_c = order.iter().position(|x| x == "c").unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn topological_sort_diamond() {
        // a -> b, a -> c, b -> d, c -> d
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("a")).unwrap();
        graph.add_node(make_node("b")).unwrap();
        graph.add_node(make_node("c")).unwrap();
        graph.add_node(make_node("d")).unwrap();
        graph.add_edge(make_edge("a", "b")).unwrap();
        graph.add_edge(make_edge("a", "c")).unwrap();
        graph.add_edge(make_edge("b", "d")).unwrap();
        graph.add_edge(make_edge("c", "d")).unwrap();

        let order = topological_order(&graph).unwrap();
        let pos_a = order.iter().position(|x| x == "a").unwrap();
        let pos_b = order.iter().position(|x| x == "b").unwrap();
        let pos_c = order.iter().position(|x| x == "c").unwrap();
        let pos_d = order.iter().position(|x| x == "d").unwrap();

        assert!(pos_a < pos_b);
        assert!(pos_a < pos_c);
        assert!(pos_b < pos_d);
        assert!(pos_c < pos_d);
    }

    #[test]
    fn topological_sort_detects_cycle() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("a")).unwrap();
        graph.add_node(make_node("b")).unwrap();
        graph.add_edge(make_edge("a", "b")).unwrap();
        graph.add_edge(make_edge("b", "a")).unwrap();

        let result = topological_order(&graph);
        assert_eq!(result, Err(GraphError::CycleDetected));
    }

    #[test]
    fn is_dag_true_for_valid_graph() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("x")).unwrap();
        graph.add_node(make_node("y")).unwrap();
        graph.add_edge(make_edge("x", "y")).unwrap();
        assert!(is_dag(&graph));
    }

    #[test]
    fn is_dag_false_for_cyclic_graph() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("x")).unwrap();
        graph.add_node(make_node("y")).unwrap();
        graph.add_edge(make_edge("x", "y")).unwrap();
        graph.add_edge(make_edge("y", "x")).unwrap();
        assert!(!is_dag(&graph));
    }

    #[test]
    fn dependencies_and_dependents() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("a")).unwrap();
        graph.add_node(make_node("b")).unwrap();
        graph.add_node(make_node("c")).unwrap();
        graph.add_edge(make_edge("a", "b")).unwrap();
        graph.add_edge(make_edge("a", "c")).unwrap();

        let deps_of_b = dependencies(&graph, "b");
        assert_eq!(deps_of_b, vec!["a"]);

        let mut deps_of_a = dependents(&graph, "a");
        deps_of_a.sort();
        assert_eq!(deps_of_a, vec!["b", "c"]);

        // Non-existent node returns empty.
        assert!(dependencies(&graph, "nonexistent").is_empty());
        assert!(dependents(&graph, "nonexistent").is_empty());
    }

    #[test]
    fn root_and_leaf_nodes() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("a")).unwrap();
        graph.add_node(make_node("b")).unwrap();
        graph.add_node(make_node("c")).unwrap();
        graph.add_edge(make_edge("a", "b")).unwrap();
        graph.add_edge(make_edge("b", "c")).unwrap();

        let roots = root_nodes(&graph);
        assert_eq!(roots, vec!["a"]);

        let leaves = leaf_nodes(&graph);
        assert_eq!(leaves, vec!["c"]);
    }

    #[test]
    fn single_node_graph() {
        let mut graph = Graph::new(GraphMetadata::default());
        graph.add_node(make_node("solo")).unwrap();

        let order = topological_order(&graph).unwrap();
        assert_eq!(order, vec!["solo"]);
        assert_eq!(root_nodes(&graph), vec!["solo"]);
        assert_eq!(leaf_nodes(&graph), vec!["solo"]);
        assert!(is_dag(&graph));
    }
}
