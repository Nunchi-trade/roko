//! Graph execution engine: sequential topological execution of cell DAGs.
//!
//! The `GraphEngine` takes a `Graph` and a `CellRegistry`, topologically sorts
//! the nodes, and executes each cell sequentially. Outputs from upstream nodes
//! are passed as inputs to downstream nodes via an internal context map.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::cell::{Cell, CellContext};
use crate::registry::CellRegistry;
use crate::topo::topological_order;
use crate::types::{Graph, GraphError, NodeId};

/// Status of a node during graph execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Not yet started.
    Pending,
    /// Currently executing.
    Running,
    /// Completed successfully.
    Complete,
    /// Failed during execution.
    Failed,
    /// Skipped because an upstream node failed.
    Skipped,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Complete => write!(f, "complete"),
            Self::Failed => write!(f, "FAILED"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// Execution result for a single node.
#[derive(Debug, Clone)]
pub struct NodeResult {
    /// Node identifier.
    pub node_id: NodeId,
    /// Cell type that was executed.
    pub cell_type: String,
    /// Final status after execution.
    pub status: NodeStatus,
    /// Wall-clock duration of execution (zero for skipped nodes).
    pub duration: Duration,
    /// Error message if status is Failed.
    pub error: Option<String>,
    /// Number of output engrams produced.
    pub output_count: usize,
}

/// Output of a full graph execution.
#[derive(Debug, Clone)]
pub struct GraphOutput {
    /// Name of the graph that was executed.
    pub graph_name: String,
    /// Whether the entire graph completed successfully (all nodes Complete).
    pub success: bool,
    /// Per-node execution results in topological order.
    pub node_results: Vec<NodeResult>,
    /// Total wall-clock duration for the full graph execution.
    pub total_duration: Duration,
}

impl GraphOutput {
    /// Return a human-readable summary of the graph execution.
    #[must_use]
    pub fn summary(&self) -> String {
        use std::fmt::Write;

        let mut s = String::new();
        let _ = writeln!(s, "Graph: {}", self.graph_name);
        let _ = writeln!(
            s,
            "Status: {}",
            if self.success { "SUCCESS" } else { "FAILED" }
        );
        let _ = writeln!(s, "Duration: {:?}", self.total_duration);
        let _ = writeln!(s, "Nodes: {}", self.node_results.len());
        s.push('\n');
        for result in &self.node_results {
            let dur = if result.duration > Duration::ZERO {
                format!(" ({:?})", result.duration)
            } else {
                String::new()
            };
            let _ = writeln!(
                s,
                "  [{:>8}] {} ({}){}",
                result.status, result.node_id, result.cell_type, dur
            );
            if let Some(err) = &result.error {
                let _ = writeln!(s, "             error: {err}");
            }
        }
        s
    }
}

/// The graph execution engine. Holds a graph and registry, executes nodes
/// sequentially in topological order.
pub struct GraphEngine {
    graph: Graph,
    registry: CellRegistry,
}

impl GraphEngine {
    /// Create a new engine for the given graph and cell registry.
    #[must_use]
    pub const fn new(graph: Graph, registry: CellRegistry) -> Self {
        Self { graph, registry }
    }

    /// Execute the graph sequentially in topological order.
    ///
    /// Each node is instantiated from the registry, executed with inputs from
    /// upstream nodes, and its outputs are stored for downstream consumption.
    /// If a node fails, all its transitive dependents are marked as Skipped.
    ///
    /// # Errors
    /// Returns `GraphError::CycleDetected` if the graph contains a cycle, or
    /// `GraphError::UnknownCellType` if a node references an unregistered cell type.
    pub async fn execute(&self, ctx: &CellContext) -> Result<GraphOutput, GraphError> {
        let start = Instant::now();

        // 1. Topological sort
        let order = topological_order(&self.graph)?;

        // 2. Track outputs per node and failed-set for skip propagation
        let mut outputs: HashMap<NodeId, Vec<roko_core::Engram>> = HashMap::new();
        let mut failed_nodes: HashSet<NodeId> = HashSet::new();
        let mut results: Vec<NodeResult> = Vec::with_capacity(order.len());

        // 3. Execute each node in order
        for node_id in &order {
            // SAFETY: topological_order only returns IDs that are in the graph.
            let Some(node) = self.graph.get_node(node_id) else {
                continue;
            };

            // Check if any upstream dependency failed -> skip
            if self.has_failed_ancestor(node_id, &failed_nodes) {
                results.push(NodeResult {
                    node_id: node_id.clone(),
                    cell_type: node.cell_type.clone(),
                    status: NodeStatus::Skipped,
                    duration: Duration::ZERO,
                    error: Some("upstream dependency failed".to_string()),
                    output_count: 0,
                });
                failed_nodes.insert(node_id.clone());
                continue;
            }

            // Instantiate cell from registry
            let cell: Box<dyn Cell> = self.registry.create(&node.cell_type, node.config.clone())?;

            // Gather inputs from upstream nodes
            let input = self.gather_inputs(node_id, &outputs);

            info!(node_id = %node_id, cell_type = %node.cell_type, "executing node");
            let node_start = Instant::now();

            // Execute the cell
            match cell.execute(input, ctx).await {
                Ok(output_engrams) => {
                    let duration = node_start.elapsed();
                    let count = output_engrams.len();
                    info!(
                        node_id = %node_id,
                        outputs = count,
                        duration_ms = duration.as_millis(),
                        "node complete"
                    );
                    outputs.insert(node_id.clone(), output_engrams);
                    results.push(NodeResult {
                        node_id: node_id.clone(),
                        cell_type: node.cell_type.clone(),
                        status: NodeStatus::Complete,
                        duration,
                        error: None,
                        output_count: count,
                    });
                }
                Err(e) => {
                    let duration = node_start.elapsed();
                    let msg = e.to_string();
                    warn!(
                        node_id = %node_id,
                        error = %msg,
                        duration_ms = duration.as_millis(),
                        "node failed"
                    );
                    failed_nodes.insert(node_id.clone());
                    results.push(NodeResult {
                        node_id: node_id.clone(),
                        cell_type: node.cell_type.clone(),
                        status: NodeStatus::Failed,
                        duration,
                        error: Some(msg),
                        output_count: 0,
                    });
                }
            }
        }

        let total_duration = start.elapsed();
        let success = results.iter().all(|r| r.status == NodeStatus::Complete);

        Ok(GraphOutput {
            graph_name: self.graph.metadata.name.clone(),
            success,
            node_results: results,
            total_duration,
        })
    }

    /// Validate the graph without executing: check for cycles, unknown cell types,
    /// and unresolved edge references.
    ///
    /// # Errors
    /// Returns a list of validation issues.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        // Check for cycles
        if topological_order(&self.graph).is_err() {
            issues.push("graph contains a cycle".to_string());
        }

        // Check all node cell types are registered
        for (node_id, idx) in &self.graph.node_map {
            let node = &self.graph.inner[*idx];
            if !self.registry.contains(&node.cell_type) {
                issues.push(format!(
                    "node '{}' references unknown cell type '{}'",
                    node_id, node.cell_type
                ));
            }
        }

        issues
    }

    /// Check if a node has any failed ancestor in the DAG.
    fn has_failed_ancestor(&self, node_id: &str, failed: &HashSet<NodeId>) -> bool {
        use petgraph::Direction;

        let Some(&idx) = self.graph.node_map.get(node_id) else {
            return false;
        };

        // Check all incoming neighbors (direct parents)
        for pred_idx in self
            .graph
            .inner
            .neighbors_directed(idx, Direction::Incoming)
        {
            let pred_id = &self.graph.inner[pred_idx].id;
            if failed.contains(pred_id) {
                return true;
            }
        }
        false
    }

    /// Gather output engrams from all upstream (predecessor) nodes as input.
    fn gather_inputs(
        &self,
        node_id: &str,
        outputs: &HashMap<NodeId, Vec<roko_core::Engram>>,
    ) -> Vec<roko_core::Engram> {
        use petgraph::Direction;

        let Some(&idx) = self.graph.node_map.get(node_id) else {
            return vec![];
        };

        let mut input = Vec::new();
        for pred_idx in self
            .graph
            .inner
            .neighbors_directed(idx, Direction::Incoming)
        {
            let pred_id = &self.graph.inner[pred_idx].id;
            if let Some(engrams) = outputs.get(pred_id) {
                input.extend(engrams.iter().cloned());
            }
        }
        input
    }
}

/// Build the default cell registry with standard gate and utility cells.
///
/// Registered cell types:
/// - `gate.compile` -- `CompileGate` (cargo check)
/// - `gate.test` -- `TestGate` (cargo test)
/// - `gate.clippy` -- `ClippyGate` (cargo clippy)
/// - `noop` -- `NoopCell` (passes input through unchanged, useful for testing)
#[must_use]
pub fn default_registry() -> CellRegistry {
    let mut registry = CellRegistry::new();

    registry.register("gate.compile", |_config| {
        Box::new(ShellCell::new(
            "gate.compile",
            "CompileGate",
            "cargo",
            &["check", "--workspace"],
        ))
    });

    registry.register("gate.test", |_config| {
        Box::new(ShellCell::new(
            "gate.test",
            "TestGate",
            "cargo",
            &["test", "--workspace"],
        ))
    });

    registry.register("gate.clippy", |_config| {
        Box::new(ShellCell::new(
            "gate.clippy",
            "ClippyGate",
            "cargo",
            &["clippy", "--workspace", "--no-deps", "--", "-D", "warnings"],
        ))
    });

    registry.register("noop", |_config| Box::new(NoopCell::default()));

    registry.register("score", |_config| {
        Box::new(NoopCell::with_id_and_name("score", "ScoreCell"))
    });

    registry.register("compose", |_config| {
        Box::new(NoopCell::with_id_and_name("compose", "ComposeCell"))
    });

    registry.register("act", |_config| {
        Box::new(NoopCell::with_id_and_name("act", "ActCell"))
    });

    registry
}

// ─── Built-in cell implementations ──────────────────────────────────────────

/// A no-op cell that passes its input through unchanged. Useful for testing
/// and as a placeholder in graph definitions.
struct NoopCell {
    id: &'static str,
    name: &'static str,
}

impl NoopCell {
    const fn with_id_and_name(id: &'static str, name: &'static str) -> Self {
        Self { id, name }
    }
}

impl Default for NoopCell {
    fn default() -> Self {
        Self {
            id: "noop",
            name: "NoopCell",
        }
    }
}

#[async_trait::async_trait]
impl Cell for NoopCell {
    fn cell_id(&self) -> &str {
        self.id
    }
    fn cell_name(&self) -> &str {
        self.name
    }
    fn cell_version(&self) -> crate::cell::CellVersion {
        (0, 1, 0)
    }
    fn protocols(&self) -> &[&str] {
        &[]
    }
    fn estimated_cost(&self) -> Option<f64> {
        None
    }
    fn estimated_duration(&self) -> Option<Duration> {
        Some(Duration::from_millis(1))
    }
    async fn execute(
        &self,
        input: Vec<roko_core::Engram>,
        _ctx: &CellContext,
    ) -> roko_core::error::Result<Vec<roko_core::Engram>> {
        Ok(input)
    }
}

/// A cell that runs a shell command. Used for gate implementations (compile, test, clippy).
/// Succeeds if the command exits with status 0, fails otherwise.
struct ShellCell {
    id: &'static str,
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

impl ShellCell {
    const fn new(
        id: &'static str,
        name: &'static str,
        program: &'static str,
        args: &'static [&'static str],
    ) -> Self {
        Self {
            id,
            name,
            program,
            args,
        }
    }
}

#[async_trait::async_trait]
impl Cell for ShellCell {
    fn cell_id(&self) -> &str {
        self.id
    }
    fn cell_name(&self) -> &str {
        self.name
    }
    fn cell_version(&self) -> crate::cell::CellVersion {
        (0, 1, 0)
    }
    fn protocols(&self) -> &[&str] {
        &["Gate"]
    }
    fn estimated_cost(&self) -> Option<f64> {
        None
    }
    fn estimated_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
    async fn execute(
        &self,
        input: Vec<roko_core::Engram>,
        _ctx: &CellContext,
    ) -> roko_core::error::Result<Vec<roko_core::Engram>> {
        let output = tokio::process::Command::new(self.program)
            .args(self.args)
            .output()
            .await
            .map_err(|e| roko_core::error::RokoError::Verify {
                gate: self.name.to_string(),
                message: format!("failed to spawn '{}': {}", self.program, e),
            })?;

        if output.status.success() {
            Ok(input)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let detail = if stderr.is_empty() {
                stdout.to_string()
            } else {
                stderr.to_string()
            };
            // Truncate to avoid massive error messages
            let detail = if detail.len() > 2000 {
                format!("{}...(truncated)", &detail[..2000])
            } else {
                detail
            };
            Err(roko_core::error::RokoError::Verify {
                gate: self.name.to_string(),
                message: format!(
                    "{} exited with code {}: {}",
                    self.program,
                    output.status.code().unwrap_or(-1),
                    detail
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_from_str;

    fn noop_registry() -> CellRegistry {
        let mut r = CellRegistry::new();
        r.register("noop", |_| Box::new(NoopCell::default()));
        r.register("gate.compile", |_| {
            Box::new(NoopCell::with_id_and_name("gate.compile", "CompileGate"))
        });
        r.register("gate.test", |_| {
            Box::new(NoopCell::with_id_and_name("gate.test", "TestGate"))
        });
        r.register("gate.clippy", |_| {
            Box::new(NoopCell::with_id_and_name("gate.clippy", "ClippyGate"))
        });
        r
    }

    #[tokio::test]
    async fn execute_linear_graph() {
        let toml_str = r#"
[graph]
name = "linear"

[[nodes]]
id = "a"
cell_type = "noop"

[[nodes]]
id = "b"
cell_type = "noop"

[[nodes]]
id = "c"
cell_type = "noop"

[[edges]]
from = "a"
to = "b"

[[edges]]
from = "b"
to = "c"
"#;
        let graph = load_from_str(toml_str).unwrap();
        let engine = GraphEngine::new(graph, noop_registry());
        let ctx = CellContext::new();
        let output = engine.execute(&ctx).await.unwrap();

        assert!(output.success);
        assert_eq!(output.node_results.len(), 3);
        assert!(
            output
                .node_results
                .iter()
                .all(|r| r.status == NodeStatus::Complete)
        );
    }

    #[tokio::test]
    async fn execute_single_node() {
        let toml_str = r#"
[graph]
name = "single"

[[nodes]]
id = "only"
cell_type = "noop"
"#;
        let graph = load_from_str(toml_str).unwrap();
        let engine = GraphEngine::new(graph, noop_registry());
        let ctx = CellContext::new();
        let output = engine.execute(&ctx).await.unwrap();

        assert!(output.success);
        assert_eq!(output.node_results.len(), 1);
        assert_eq!(output.node_results[0].status, NodeStatus::Complete);
    }

    #[tokio::test]
    async fn validate_missing_cell_type() {
        let toml_str = r#"
[graph]
name = "bad"

[[nodes]]
id = "a"
cell_type = "nonexistent"
"#;
        let graph = load_from_str(toml_str).unwrap();
        let engine = GraphEngine::new(graph, noop_registry());
        let issues = engine.validate();
        assert!(!issues.is_empty());
        assert!(issues[0].contains("nonexistent"));
    }

    #[tokio::test]
    async fn validate_valid_graph() {
        let toml_str = r#"
[graph]
name = "valid"

[[nodes]]
id = "a"
cell_type = "noop"

[[nodes]]
id = "b"
cell_type = "gate.compile"

[[edges]]
from = "a"
to = "b"
"#;
        let graph = load_from_str(toml_str).unwrap();
        let engine = GraphEngine::new(graph, noop_registry());
        let issues = engine.validate();
        assert!(issues.is_empty());
    }
}
