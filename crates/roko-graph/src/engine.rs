//! Graph execution engine with fan-out/fan-in parallelism.
//!
//! The engine performs a topological sort of the graph, groups nodes by depth
//! (nodes at the same depth have no dependencies between them), and executes
//! each depth level in parallel using a [`tokio::task::JoinSet`].
//!
//! Fan-out: one node feeds multiple downstream nodes (multiple edges from one source).
//! Fan-in: multiple nodes must complete before a downstream node starts (multiple edges to one target).
//! Conditional edges are evaluated after a node completes; edges whose conditions
//! are not met are pruned, potentially skipping downstream nodes.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use tokio::task::JoinSet;
use tracing::{debug, info, warn};

use crate::budget::BudgetTracker;
use crate::condition;
use crate::error::{GraphError, Result};
use crate::types::{Edge, GraphDef, GraphResult, GraphStatus, Node, NodeOutput, NodeStatus};

/// A cell executor function: given a node and its inputs, produce an output.
/// This is the pluggable execution strategy — callers provide their own
/// cell resolution logic.
pub type CellExecutor = Arc<
    dyn Fn(Node, Vec<NodeOutput>) -> futures::future::BoxFuture<'static, NodeOutput> + Send + Sync,
>;

/// The graph execution engine.
pub struct GraphEngine {
    /// The graph definition to execute.
    graph: GraphDef,
    /// Budget tracker for resource limits.
    budget: Arc<BudgetTracker>,
    /// Cell executor function.
    executor: CellExecutor,
}

impl GraphEngine {
    /// Create a new engine for the given graph definition.
    pub fn new(graph: GraphDef, executor: CellExecutor) -> Self {
        let budget = Arc::new(BudgetTracker::from_config(&graph.config));
        Self {
            graph,
            budget,
            executor,
        }
    }

    /// Create with explicit budget limits (useful for testing).
    pub fn with_budget(graph: GraphDef, executor: CellExecutor, budget: BudgetTracker) -> Self {
        Self {
            graph,
            budget: Arc::new(budget),
            executor,
        }
    }

    /// Execute the graph, returning results for all nodes.
    ///
    /// Nodes at the same topological depth are executed in parallel (fan-out).
    /// A node only starts once all its upstream dependencies have completed (fan-in).
    /// Conditional edges are evaluated after each node completes.
    pub async fn execute(&self) -> Result<GraphResult> {
        let start = Instant::now();

        // Validate the graph structure.
        self.validate()?;

        // Compute topological levels (groups of nodes that can run in parallel).
        let levels = self.topological_levels()?;
        let max_par = self.graph.config.max_parallelism;

        // Track outputs per node for condition evaluation and input passing.
        let mut outputs: HashMap<String, NodeOutput> = HashMap::new();
        let mut all_outputs: Vec<NodeOutput> = Vec::new();

        // Track which nodes are "reachable" (not pruned by conditions).
        let mut reachable: HashSet<String> =
            self.graph.nodes.iter().map(|n| n.id.clone()).collect();

        for level in &levels {
            // Before executing this level, check budget.
            if let Err(e) = self.budget.check() {
                // Skip all remaining nodes with BudgetExceeded.
                let reason = e.to_string();
                for node_id in level {
                    if reachable.contains(node_id) {
                        let out = NodeOutput::skipped(node_id, &reason);
                        outputs.insert(node_id.clone(), out.clone());
                        all_outputs.push(out);
                    }
                }
                continue;
            }

            // Determine which nodes in this level are actually runnable.
            let runnable: Vec<&str> = level
                .iter()
                .filter(|id| reachable.contains(id.as_str()))
                .map(String::as_str)
                .collect();

            if runnable.is_empty() {
                continue;
            }

            // Execute nodes in parallel, respecting max_parallelism.
            let chunk_results = self.execute_level(&runnable, &outputs, max_par).await;

            // Process results: record budget, evaluate outgoing conditions.
            for output in chunk_results {
                self.budget.record(
                    &output.node_id,
                    output.tokens_used,
                    output.cost_usd,
                    output.duration,
                );

                // Evaluate outgoing edges for condition pruning.
                self.evaluate_outgoing_edges(&output, &mut reachable);

                outputs.insert(output.node_id.clone(), output.clone());
                all_outputs.push(output);
            }
        }

        // Determine overall status.
        let status = self.determine_status(&all_outputs);
        let total_duration = start.elapsed();

        Ok(GraphResult {
            graph_name: self.graph.name.clone(),
            node_outputs: all_outputs,
            status,
            total_tokens: self.budget.tokens_used(),
            total_cost_usd: self.budget.cost_usd(),
            total_duration,
        })
    }

    /// Execute a single level of nodes in parallel, respecting max_parallelism.
    async fn execute_level(
        &self,
        node_ids: &[&str],
        prior_outputs: &HashMap<String, NodeOutput>,
        max_parallelism: usize,
    ) -> Vec<NodeOutput> {
        let mut results = Vec::with_capacity(node_ids.len());

        // Process in chunks of max_parallelism.
        for chunk in node_ids.chunks(max_parallelism) {
            let mut join_set: JoinSet<NodeOutput> = JoinSet::new();

            for &node_id in chunk {
                let node = self
                    .graph
                    .nodes
                    .iter()
                    .find(|n| n.id == node_id)
                    .cloned()
                    .expect("node validated to exist");

                // Gather inputs from upstream nodes (fan-in).
                let inputs = self.gather_inputs(node_id, prior_outputs);
                let executor = self.executor.clone();

                join_set.spawn(async move { (executor)(node, inputs).await });
            }

            // Collect all results from this chunk.
            while let Some(result) = join_set.join_next().await {
                match result {
                    Ok(output) => results.push(output),
                    Err(join_err) => {
                        // JoinError means the task panicked or was cancelled.
                        warn!("node task failed: {join_err}");
                        results.push(NodeOutput::failed("unknown", join_err.to_string()));
                    }
                }
            }
        }

        results
    }

    /// Gather input outputs from upstream nodes (fan-in support).
    fn gather_inputs(
        &self,
        node_id: &str,
        prior_outputs: &HashMap<String, NodeOutput>,
    ) -> Vec<NodeOutput> {
        self.graph
            .edges
            .iter()
            .filter(|e| e.to == node_id)
            .filter_map(|e| prior_outputs.get(&e.from))
            .cloned()
            .collect()
    }

    /// After a node completes, evaluate outgoing conditional edges.
    /// If a condition is not met, remove the target from the reachable set
    /// (unless another edge still reaches it).
    fn evaluate_outgoing_edges(&self, output: &NodeOutput, reachable: &mut HashSet<String>) {
        // Find all outgoing edges from this node.
        let outgoing: Vec<&Edge> = self
            .graph
            .edges
            .iter()
            .filter(|e| e.from == output.node_id)
            .collect();

        for edge in &outgoing {
            if !condition::evaluate(&edge.condition, output) {
                debug!(
                    from = %output.node_id,
                    to = %edge.to,
                    "conditional edge not satisfied, checking if target has other sources"
                );

                // Only prune target if ALL incoming edges to it are unsatisfied.
                let target_still_reachable = self
                    .graph
                    .edges
                    .iter()
                    .filter(|e| e.to == edge.to && e.from != output.node_id)
                    .any(|_other| {
                        // Another edge still reaches this target.
                        true
                    });

                // For edges from THIS node that failed: check if this is the only
                // path to the target. If the target has no other satisfied incoming
                // edges, it becomes unreachable.
                if !target_still_reachable {
                    info!(node = %edge.to, "node pruned: no satisfied incoming edges");
                    reachable.remove(&edge.to);
                }
            }
        }
    }

    /// Validate graph structure: all edges reference valid nodes, no cycles.
    fn validate(&self) -> Result<()> {
        if self.graph.nodes.is_empty() {
            return Err(GraphError::InvalidGraph {
                reason: "graph has no nodes".into(),
            });
        }

        let node_ids: HashSet<&str> = self.graph.nodes.iter().map(|n| n.id.as_str()).collect();

        // Check all edges reference valid nodes.
        for edge in &self.graph.edges {
            if !node_ids.contains(edge.from.as_str()) {
                return Err(GraphError::InvalidEdge {
                    node_id: edge.from.clone(),
                });
            }
            if !node_ids.contains(edge.to.as_str()) {
                return Err(GraphError::InvalidEdge {
                    node_id: edge.to.clone(),
                });
            }
        }

        // Check for cycles via topological sort (Kahn's algorithm).
        let _ = self.topological_levels()?;

        Ok(())
    }

    /// Compute topological levels using Kahn's algorithm.
    /// Returns groups of node IDs; nodes within the same group can execute in parallel.
    fn topological_levels(&self) -> Result<Vec<Vec<String>>> {
        let node_ids: Vec<&str> = self.graph.nodes.iter().map(|n| n.id.as_str()).collect();

        // Build adjacency and in-degree.
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();

        for id in &node_ids {
            in_degree.insert(id, 0);
            successors.insert(id, Vec::new());
        }

        for edge in &self.graph.edges {
            *in_degree.get_mut(edge.to.as_str()).unwrap() += 1;
            successors
                .get_mut(edge.from.as_str())
                .unwrap()
                .push(&edge.to);
        }

        let mut levels: Vec<Vec<String>> = Vec::new();
        let mut queue: VecDeque<&str> = VecDeque::new();

        // Start with nodes that have no incoming edges.
        for (&id, &deg) in &in_degree {
            if deg == 0 {
                queue.push_back(id);
            }
        }

        let mut processed = 0usize;

        while !queue.is_empty() {
            // All nodes currently in the queue form one parallel level.
            let level: Vec<String> = queue.drain(..).map(|s| s.to_string()).collect();
            processed += level.len();

            // Decrement in-degree for successors.
            let mut next_queue: VecDeque<&str> = VecDeque::new();
            for node_id in &level {
                if let Some(succs) = successors.get(node_id.as_str()) {
                    for &succ in succs {
                        let deg = in_degree.get_mut(succ).unwrap();
                        *deg -= 1;
                        if *deg == 0 {
                            next_queue.push_back(succ);
                        }
                    }
                }
            }

            levels.push(level);
            queue = next_queue;
        }

        if processed != node_ids.len() {
            // Some nodes were not processed => cycle exists.
            let unprocessed: Vec<String> = node_ids
                .iter()
                .filter(|id| in_degree.get(*id).copied().unwrap_or(0) > 0)
                .map(|s| s.to_string())
                .collect();
            return Err(GraphError::CycleDetected {
                node_id: unprocessed.first().cloned().unwrap_or_default(),
            });
        }

        Ok(levels)
    }

    /// Determine overall graph status from node outputs.
    fn determine_status(&self, outputs: &[NodeOutput]) -> GraphStatus {
        let has_budget_skip = outputs.iter().any(
            |o| matches!(&o.status, NodeStatus::Skipped { reason } if reason.contains("budget")),
        );

        if has_budget_skip {
            return GraphStatus::BudgetExceeded {
                reason: "one or more nodes skipped due to budget limits".into(),
            };
        }

        let has_failure = outputs.iter().any(|o| o.status.is_failed());
        if has_failure {
            return GraphStatus::PartialFailure;
        }

        GraphStatus::Success
    }

    /// Get a reference to the budget tracker.
    #[must_use]
    pub fn budget(&self) -> &BudgetTracker {
        &self.budget
    }

    /// Get the graph definition.
    #[must_use]
    pub fn graph_def(&self) -> &GraphDef {
        &self.graph
    }
}

/// Parse a graph definition from TOML.
pub fn parse_graph_toml(toml_str: &str) -> Result<GraphDef> {
    let graph: GraphDef = toml::from_str(toml_str)?;
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::condition::EdgeCondition;
    use crate::types::GraphConfig;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Helper: create a simple pass-through executor that marks nodes as successful.
    fn passthrough_executor() -> CellExecutor {
        Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
            Box::pin(
                async move { NodeOutput::success(&node.id, json!({"cell_type": node.cell_type})) },
            )
        })
    }

    /// Helper: create an executor that records execution order.
    fn order_tracking_executor(counter: Arc<AtomicUsize>) -> CellExecutor {
        Arc::new(move |node: Node, _inputs: Vec<NodeOutput>| {
            let counter = counter.clone();
            Box::pin(async move {
                let order = counter.fetch_add(1, Ordering::SeqCst);
                NodeOutput::success(&node.id, json!({"order": order}))
            })
        })
    }

    fn simple_linear_graph() -> GraphDef {
        GraphDef {
            name: "linear".into(),
            description: "A -> B -> C".into(),
            nodes: vec![
                Node {
                    id: "a".into(),
                    name: "Node A".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "b".into(),
                    name: "Node B".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "c".into(),
                    name: "Node C".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
            ],
            edges: vec![
                Edge {
                    from: "a".into(),
                    to: "b".into(),
                    condition: EdgeCondition::Always,
                },
                Edge {
                    from: "b".into(),
                    to: "c".into(),
                    condition: EdgeCondition::Always,
                },
            ],
            config: GraphConfig::default(),
        }
    }

    fn fan_out_graph() -> GraphDef {
        // A fans out to B and C (parallel), then D fans in from B and C.
        GraphDef {
            name: "fan-out-in".into(),
            description: "A -> (B, C) -> D".into(),
            nodes: vec![
                Node {
                    id: "a".into(),
                    name: "Source".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "b".into(),
                    name: "Branch 1".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "c".into(),
                    name: "Branch 2".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "d".into(),
                    name: "Merge".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
            ],
            edges: vec![
                Edge {
                    from: "a".into(),
                    to: "b".into(),
                    condition: EdgeCondition::Always,
                },
                Edge {
                    from: "a".into(),
                    to: "c".into(),
                    condition: EdgeCondition::Always,
                },
                Edge {
                    from: "b".into(),
                    to: "d".into(),
                    condition: EdgeCondition::Always,
                },
                Edge {
                    from: "c".into(),
                    to: "d".into(),
                    condition: EdgeCondition::Always,
                },
            ],
            config: GraphConfig::default(),
        }
    }

    #[tokio::test]
    async fn linear_graph_executes_in_order() {
        let counter = Arc::new(AtomicUsize::new(0));
        let engine = GraphEngine::new(simple_linear_graph(), order_tracking_executor(counter));

        let result = engine.execute().await.unwrap();
        assert_eq!(result.status, GraphStatus::Success);
        assert_eq!(result.node_outputs.len(), 3);

        // Verify execution order: a=0, b=1, c=2.
        let a_order = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "a")
            .unwrap();
        let b_order = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "b")
            .unwrap();
        let c_order = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "c")
            .unwrap();
        assert_eq!(a_order.data["order"], 0);
        assert_eq!(b_order.data["order"], 1);
        assert_eq!(c_order.data["order"], 2);
    }

    #[tokio::test]
    async fn fan_out_executes_in_parallel() {
        let engine = GraphEngine::new(fan_out_graph(), passthrough_executor());
        let result = engine.execute().await.unwrap();

        assert_eq!(result.status, GraphStatus::Success);
        assert_eq!(result.node_outputs.len(), 4);

        // d should come after both b and c.
        let d_idx = result
            .node_outputs
            .iter()
            .position(|o| o.node_id == "d")
            .unwrap();
        let b_idx = result
            .node_outputs
            .iter()
            .position(|o| o.node_id == "b")
            .unwrap();
        let c_idx = result
            .node_outputs
            .iter()
            .position(|o| o.node_id == "c")
            .unwrap();
        assert!(d_idx > b_idx);
        assert!(d_idx > c_idx);
    }

    #[tokio::test]
    async fn fan_in_receives_all_inputs() {
        // Executor that records how many inputs each node received.
        let executor: CellExecutor = Arc::new(|node: Node, inputs: Vec<NodeOutput>| {
            Box::pin(
                async move { NodeOutput::success(&node.id, json!({"input_count": inputs.len()})) },
            )
        });

        let engine = GraphEngine::new(fan_out_graph(), executor);
        let result = engine.execute().await.unwrap();

        // Node D should receive 2 inputs (from B and C).
        let d_output = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "d")
            .unwrap();
        assert_eq!(d_output.data["input_count"], 2);
    }

    #[tokio::test]
    async fn cycle_detected() {
        let graph = GraphDef {
            name: "cyclic".into(),
            description: "".into(),
            nodes: vec![
                Node {
                    id: "a".into(),
                    name: "A".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "b".into(),
                    name: "B".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
            ],
            edges: vec![
                Edge {
                    from: "a".into(),
                    to: "b".into(),
                    condition: EdgeCondition::Always,
                },
                Edge {
                    from: "b".into(),
                    to: "a".into(),
                    condition: EdgeCondition::Always,
                },
            ],
            config: GraphConfig::default(),
        };

        let engine = GraphEngine::new(graph, passthrough_executor());
        let err = engine.execute().await.unwrap_err();
        assert!(matches!(err, GraphError::CycleDetected { .. }));
    }

    #[tokio::test]
    async fn empty_graph_rejected() {
        let graph = GraphDef {
            name: "empty".into(),
            description: "".into(),
            nodes: vec![],
            edges: vec![],
            config: GraphConfig::default(),
        };

        let engine = GraphEngine::new(graph, passthrough_executor());
        let err = engine.execute().await.unwrap_err();
        assert!(matches!(err, GraphError::InvalidGraph { .. }));
    }

    #[tokio::test]
    async fn invalid_edge_rejected() {
        let graph = GraphDef {
            name: "bad-edge".into(),
            description: "".into(),
            nodes: vec![Node {
                id: "a".into(),
                name: "A".into(),
                cell_type: "test".into(),
                config: HashMap::new(),
            }],
            edges: vec![Edge {
                from: "a".into(),
                to: "nonexistent".into(),
                condition: EdgeCondition::Always,
            }],
            config: GraphConfig::default(),
        };

        let engine = GraphEngine::new(graph, passthrough_executor());
        let err = engine.execute().await.unwrap_err();
        assert!(matches!(err, GraphError::InvalidEdge { .. }));
    }

    #[tokio::test]
    async fn conditional_edge_prunes_downstream() {
        // A -> (on_success) -> B, A -> (on_failure) -> C
        // A succeeds, so B runs but C is pruned.
        let graph = GraphDef {
            name: "conditional".into(),
            description: "".into(),
            nodes: vec![
                Node {
                    id: "a".into(),
                    name: "A".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "b".into(),
                    name: "B (success path)".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "c".into(),
                    name: "C (failure path)".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
            ],
            edges: vec![
                Edge {
                    from: "a".into(),
                    to: "b".into(),
                    condition: EdgeCondition::OnSuccess,
                },
                Edge {
                    from: "a".into(),
                    to: "c".into(),
                    condition: EdgeCondition::OnFailure,
                },
            ],
            config: GraphConfig::default(),
        };

        let engine = GraphEngine::new(graph, passthrough_executor());
        let result = engine.execute().await.unwrap();

        // A and B should have run, C should not appear in outputs
        // (it was pruned before execution).
        let node_ids: Vec<&str> = result
            .node_outputs
            .iter()
            .map(|o| o.node_id.as_str())
            .collect();
        assert!(node_ids.contains(&"a"));
        assert!(node_ids.contains(&"b"));
        // C should not be in results since it was pruned.
        assert!(!node_ids.contains(&"c"));
    }

    #[tokio::test]
    async fn budget_skips_remaining_nodes() {
        use crate::budget::BudgetLimits;

        let graph = simple_linear_graph();
        // Set a token limit of 1 — first node will exhaust it.
        let budget = BudgetTracker::with_limits(BudgetLimits {
            max_tokens: Some(1),
            max_cost_usd: None,
            deadline: None,
        });

        // Executor that uses 5 tokens per node.
        let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
            Box::pin(async move {
                let mut out = NodeOutput::success(&node.id, json!({}));
                out.tokens_used = 5;
                out
            })
        });

        let engine = GraphEngine::with_budget(graph, executor, budget);
        let result = engine.execute().await.unwrap();

        assert!(matches!(result.status, GraphStatus::BudgetExceeded { .. }));
        // First node should succeed, remaining should be skipped.
        let a_out = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "a")
            .unwrap();
        assert!(a_out.status.is_success());

        let b_out = result
            .node_outputs
            .iter()
            .find(|o| o.node_id == "b")
            .unwrap();
        assert!(matches!(b_out.status, NodeStatus::Skipped { .. }));
    }

    #[tokio::test]
    async fn max_parallelism_respected() {
        use std::sync::atomic::AtomicUsize;
        use tokio::time::sleep;

        let concurrent = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));

        let conc = concurrent.clone();
        let ms = max_seen.clone();

        // 4 independent nodes, max_parallelism = 2.
        let graph = GraphDef {
            name: "parallel-limited".into(),
            description: "".into(),
            nodes: vec![
                Node {
                    id: "a".into(),
                    name: "A".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "b".into(),
                    name: "B".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "c".into(),
                    name: "C".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
                Node {
                    id: "d".into(),
                    name: "D".into(),
                    cell_type: "test".into(),
                    config: HashMap::new(),
                },
            ],
            edges: vec![],
            config: GraphConfig {
                max_parallelism: 2,
                ..Default::default()
            },
        };

        let executor: CellExecutor = Arc::new(move |node: Node, _| {
            let conc = conc.clone();
            let ms = ms.clone();
            Box::pin(async move {
                let current = conc.fetch_add(1, Ordering::SeqCst) + 1;
                ms.fetch_max(current, Ordering::SeqCst);
                sleep(Duration::from_millis(50)).await;
                conc.fetch_sub(1, Ordering::SeqCst);
                NodeOutput::success(&node.id, json!({}))
            })
        });

        let engine = GraphEngine::new(graph, executor);
        let result = engine.execute().await.unwrap();

        assert_eq!(result.node_outputs.len(), 4);
        assert_eq!(result.status, GraphStatus::Success);
        // Max concurrency should not exceed 2.
        assert!(max_seen.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn node_failure_propagates() {
        let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
            Box::pin(async move {
                if node.id == "b" {
                    NodeOutput::failed(&node.id, "intentional failure")
                } else {
                    NodeOutput::success(&node.id, json!({}))
                }
            })
        });

        let engine = GraphEngine::new(simple_linear_graph(), executor);
        let result = engine.execute().await.unwrap();
        assert_eq!(result.status, GraphStatus::PartialFailure);
    }
}
