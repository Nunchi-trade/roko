//! Convert a Runner v2 `Plan` (tasks.toml) into a `Graph` for Engine execution.
//!
//! Mapping:
//! - Each `TaskDef` becomes a Node with `cell_type = "task-executor"`
//! - `depends_on` relationships become Edges
//! - Tasks with no incoming edges become entry nodes
//! - Tasks with no outgoing edges become exit nodes
//! - `TaskMeta.max_parallel` is stored in graph metadata labels
//! - `TaskDef.timeout_secs` is stored per-node in the config
//! - All tasks use `ExecutionClass::Activity` (non-deterministic, LLM-dispatched)
//!
//! Cross-plan dependencies (`depends_on_plan`) are outside the scope of a
//! single Graph. They are logged as warnings and skipped.

use std::collections::HashSet;

use tracing::warn;

use crate::types::{Edge, Graph, GraphError, GraphMetadata, Node};

/// Convert a loaded Runner v2 plan into a Graph ready for Engine execution.
///
/// The resulting graph uses `cell_type = "task-executor"` for every node.
/// The `task-executor` cell must be registered in the Engine's CellRegistry
/// (see `default_registry()` in `crates/roko-graph/src/engine.rs`).
///
/// # Errors
///
/// Returns an error if:
/// - A task's `depends_on` references a task ID not present in this plan
/// - The resulting graph contains a cycle (detected by `topo::topological_order`)
/// - Two tasks share the same ID
pub fn plan_to_graph(
    plan_id: &str,
    plan_dir: &str,
    tasks: &[(String, PlanTaskInfo)],
    max_parallel: u32,
) -> Result<Graph, GraphError> {
    // Build the set of all known task IDs for dependency validation.
    let known_ids: HashSet<&str> = tasks.iter().map(|(id, _)| id.as_str()).collect();

    let metadata = GraphMetadata {
        name: plan_id.to_string(),
        description: Some(format!("Converted from plan '{plan_id}' tasks.toml")),
        version: None,
        labels: {
            let mut labels = std::collections::HashMap::new();
            labels.insert("source".to_string(), "plan-converter".to_string());
            labels.insert("plan_id".to_string(), plan_id.to_string());
            labels.insert("plan_dir".to_string(), plan_dir.to_string());
            labels.insert("max_parallel".to_string(), max_parallel.to_string());
            labels
        },
    };

    let mut graph = Graph::new(metadata);

    // Phase 1: Add all nodes.
    for (id, info) in tasks {
        let config = build_node_config(plan_id, plan_dir, info);
        let node = Node {
            id: id.clone(),
            cell_type: "task-executor".to_string(),
            config,
            inputs: vec![],
            outputs: vec![],
        };
        graph.add_node(node)?;
    }

    // Phase 2: Add edges from depends_on relationships.
    for (id, info) in tasks {
        for dep in &info.depends_on {
            if known_ids.contains(dep.as_str()) {
                let edge = Edge {
                    from: dep.clone(),
                    to: id.clone(),
                    condition: None,
                };
                graph.add_edge(edge)?;
            } else {
                return Err(GraphError::InvalidGraph {
                    reason: format!(
                        "task '{}' in plan '{}' depends on '{}', which does not exist in this plan",
                        id, plan_id, dep
                    ),
                });
            }
        }

        // Log warnings for cross-plan dependencies (skipped).
        for cross_dep in &info.depends_on_plan {
            warn!(
                plan_id = %plan_id,
                task_id = %id,
                cross_plan_dep = %cross_dep,
                "skipping cross-plan dependency (not supported in single-graph conversion)"
            );
        }
    }

    // Phase 3: Validate the graph (cycle detection).
    crate::topo::topological_order(&graph)?;

    Ok(graph)
}

/// Minimal task information needed by the converter.
///
/// This is a crate-boundary-safe struct that does not depend on roko-cli types.
/// Callers construct this from their `TaskDef` or equivalent.
#[derive(Debug, Clone)]
pub struct PlanTaskInfo {
    /// Task title.
    pub title: String,
    /// Task description.
    pub description: Option<String>,
    /// Role (implementer, researcher, etc.).
    pub role: Option<String>,
    /// Complexity tier.
    pub tier: String,
    /// Model hint for task dispatch.
    pub model_hint: Option<String>,
    /// Files this task modifies.
    pub files: Vec<String>,
    /// Task IDs this task depends on (same plan).
    pub depends_on: Vec<String>,
    /// Plan IDs this task depends on (cross-plan, skipped).
    pub depends_on_plan: Vec<String>,
    /// Per-task timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Work domain.
    pub domain: Option<String>,
    /// Definition order index.
    pub sequence: usize,
    /// Full serialized task config as JSON (for TaskExecutorCell).
    pub full_config_json: serde_json::Value,
}

/// Build the TOML config value for a graph node from task info.
fn build_node_config(plan_id: &str, plan_dir: &str, info: &PlanTaskInfo) -> toml::Value {
    // Store task metadata as a TOML table that TaskExecutorCell can deserialize.
    let mut table = toml::map::Map::new();
    table.insert(
        "plan_id".to_string(),
        toml::Value::String(plan_id.to_string()),
    );
    table.insert(
        "plan_dir".to_string(),
        toml::Value::String(plan_dir.to_string()),
    );
    table.insert("title".to_string(), toml::Value::String(info.title.clone()));
    if let Some(ref desc) = info.description {
        table.insert("description".to_string(), toml::Value::String(desc.clone()));
    }
    if let Some(ref role) = info.role {
        table.insert("role".to_string(), toml::Value::String(role.clone()));
    }
    table.insert("tier".to_string(), toml::Value::String(info.tier.clone()));
    if let Some(ref hint) = info.model_hint {
        table.insert("model_hint".to_string(), toml::Value::String(hint.clone()));
    }
    if let Some(ref domain) = info.domain {
        table.insert("domain".to_string(), toml::Value::String(domain.clone()));
    }
    table.insert(
        "timeout_secs".to_string(),
        toml::Value::Integer(i64::from(info.timeout_secs.min(i64::MAX as u64) as u32)),
    );
    table.insert(
        "max_retries".to_string(),
        toml::Value::Integer(i64::from(info.max_retries)),
    );
    table.insert(
        "sequence".to_string(),
        toml::Value::Integer(info.sequence as i64),
    );

    // Store full task config JSON as a string for TaskExecutorCell deserialization.
    table.insert(
        "task_def_json".to_string(),
        toml::Value::String(info.full_config_json.to_string()),
    );

    // Store files as a TOML array.
    let files_arr: Vec<toml::Value> = info
        .files
        .iter()
        .map(|f| toml::Value::String(f.clone()))
        .collect();
    table.insert("files".to_string(), toml::Value::Array(files_arr));

    toml::Value::Table(table)
}

/// Convenience function: convert plan data and return entry/exit node IDs alongside the graph.
pub fn plan_to_graph_with_endpoints(
    plan_id: &str,
    plan_dir: &str,
    tasks: &[(String, PlanTaskInfo)],
    max_parallel: u32,
) -> Result<(Graph, Vec<String>, Vec<String>), GraphError> {
    let graph = plan_to_graph(plan_id, plan_dir, tasks, max_parallel)?;
    let entries = crate::topo::root_nodes(&graph);
    let exits = crate::topo::leaf_nodes(&graph);
    Ok((graph, entries, exits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_task(id: &str, depends_on: &[&str]) -> (String, PlanTaskInfo) {
        (
            id.to_string(),
            PlanTaskInfo {
                title: format!("Task {id}"),
                description: None,
                role: Some("implementer".to_string()),
                tier: "mechanical".to_string(),
                model_hint: None,
                files: vec![],
                depends_on: depends_on.iter().map(|s| s.to_string()).collect(),
                depends_on_plan: vec![],
                timeout_secs: 300,
                max_retries: 2,
                domain: None,
                sequence: 0,
                full_config_json: json!({"id": id, "title": format!("Task {id}")}),
            },
        )
    }

    #[test]
    fn convert_single_task() {
        let tasks = vec![make_task("T1", &[])];
        let graph = plan_to_graph("test-plan", "/tmp/plan", &tasks, 1).unwrap();
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.metadata.name, "test-plan");

        let node = graph.get_node("T1").unwrap();
        assert_eq!(node.cell_type, "task-executor");
    }

    #[test]
    fn convert_linear_chain() {
        let tasks = vec![
            make_task("T1", &[]),
            make_task("T2", &["T1"]),
            make_task("T3", &["T2"]),
        ];
        let graph = plan_to_graph("chain", "/tmp", &tasks, 1).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn convert_diamond_dependencies() {
        let tasks = vec![
            make_task("T1", &[]),
            make_task("T2", &["T1"]),
            make_task("T3", &["T1"]),
            make_task("T4", &["T2", "T3"]),
        ];
        let (graph, entries, exits) =
            plan_to_graph_with_endpoints("diamond", "/tmp", &tasks, 2).unwrap();
        assert_eq!(graph.node_count(), 4);
        assert_eq!(graph.edge_count(), 4);
        assert_eq!(entries, vec!["T1"]);
        assert_eq!(exits, vec!["T4"]);
    }

    #[test]
    fn convert_missing_dependency_errors() {
        let tasks = vec![make_task("T1", &["T_MISSING"])];
        let result = plan_to_graph("bad", "/tmp", &tasks, 1);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("T_MISSING"));
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn convert_cycle_detected() {
        let tasks = vec![make_task("T1", &["T2"]), make_task("T2", &["T1"])];
        let result = plan_to_graph("cyclic", "/tmp", &tasks, 1);
        assert!(matches!(result, Err(GraphError::CycleDetected)));
    }

    #[test]
    fn convert_parallel_tasks_no_deps() {
        let tasks = vec![
            make_task("T1", &[]),
            make_task("T2", &[]),
            make_task("T3", &[]),
        ];
        let (graph, entries, exits) =
            plan_to_graph_with_endpoints("parallel", "/tmp", &tasks, 3).unwrap();
        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(entries.len(), 3);
        assert_eq!(exits.len(), 3);
    }

    #[test]
    fn node_config_contains_task_metadata() {
        let tasks = vec![(
            "T1".to_string(),
            PlanTaskInfo {
                title: "Build feature".to_string(),
                description: Some("A detailed description".to_string()),
                role: Some("implementer".to_string()),
                tier: "focused".to_string(),
                model_hint: Some("claude-sonnet-4-20250514".to_string()),
                files: vec!["src/lib.rs".to_string()],
                depends_on: vec![],
                depends_on_plan: vec![],
                timeout_secs: 600,
                max_retries: 3,
                domain: Some("coding".to_string()),
                sequence: 0,
                full_config_json: json!({"id": "T1"}),
            },
        )];
        let graph = plan_to_graph("meta-test", "/work", &tasks, 1).unwrap();
        let node = graph.get_node("T1").unwrap();

        // Verify the config table has expected keys.
        let table = node.config.as_table().unwrap();
        assert_eq!(table["plan_id"].as_str().unwrap(), "meta-test");
        assert_eq!(table["plan_dir"].as_str().unwrap(), "/work");
        assert_eq!(table["title"].as_str().unwrap(), "Build feature");
        assert_eq!(table["role"].as_str().unwrap(), "implementer");
        assert_eq!(table["tier"].as_str().unwrap(), "focused");
        assert_eq!(
            table["model_hint"].as_str().unwrap(),
            "claude-sonnet-4-20250514"
        );
        assert_eq!(table["domain"].as_str().unwrap(), "coding");
    }

    #[test]
    fn cross_plan_deps_are_skipped() {
        let tasks = vec![(
            "T1".to_string(),
            PlanTaskInfo {
                title: "Task with cross-plan dep".to_string(),
                description: None,
                role: None,
                tier: "mechanical".to_string(),
                model_hint: None,
                files: vec![],
                depends_on: vec![],
                depends_on_plan: vec!["other-plan".to_string()],
                timeout_secs: 300,
                max_retries: 2,
                domain: None,
                sequence: 0,
                full_config_json: json!({}),
            },
        )];
        // Should succeed despite cross-plan dep (logged as warning, not an error).
        let graph = plan_to_graph("cross", "/tmp", &tasks, 1).unwrap();
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn duplicate_task_id_errors() {
        let tasks = vec![make_task("T1", &[]), make_task("T1", &[])];
        let result = plan_to_graph("dup", "/tmp", &tasks, 1);
        assert!(matches!(result, Err(GraphError::DuplicateNode(_))));
    }
}
