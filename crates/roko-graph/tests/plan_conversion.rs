//! Integration tests for plan-to-graph conversion and execution.
//!
//! These tests exercise the full path: build `PlanTaskInfo` -> convert
//! to `Graph` -> validate via `GraphEngine` -> execute via dry-run
//! `TaskExecutorCell`.

use roko_graph::convert::{PlanTaskInfo, plan_to_graph, plan_to_graph_with_endpoints};
use roko_graph::engine::GraphEngine;
use roko_graph::{CellRegistry, default_registry};

/// Helper: create a `PlanTaskInfo` with minimal defaults.
fn task_info(title: &str, depends_on: &[&str]) -> PlanTaskInfo {
    PlanTaskInfo {
        title: title.to_string(),
        description: Some(format!("Test task: {title}")),
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
        full_config_json: serde_json::json!({"title": title}),
    }
}

/// Test 1: Single task converts and executes successfully through the engine.
#[tokio::test]
async fn single_task_round_trip() {
    let tasks = vec![("T1".to_string(), task_info("Build the widget", &[]))];

    let graph = plan_to_graph("test-plan", "/tmp/plan", &tasks, 1).unwrap();
    assert_eq!(graph.node_count(), 1);

    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);

    // Validate before execution.
    let issues = engine.validate();
    assert!(
        issues.is_empty(),
        "unexpected validation issues: {issues:?}"
    );

    // Execute.
    let ctx = roko_graph::CellContext::new();
    let output = engine.execute(&ctx).await.unwrap();
    assert!(output.success);
    assert_eq!(output.node_results.len(), 1);
    assert_eq!(
        output.node_results[0].status,
        roko_graph::NodeStatus::Complete
    );
    assert!(output.node_results[0].output_count > 0);
}

/// Test 2: Linear chain (T1 -> T2 -> T3) executes in correct order.
#[tokio::test]
async fn linear_chain_execution() {
    let tasks = vec![
        ("T1".to_string(), task_info("First", &[])),
        ("T2".to_string(), task_info("Second", &["T1"])),
        ("T3".to_string(), task_info("Third", &["T2"])),
    ];

    let graph = plan_to_graph("chain-plan", "/tmp/chain", &tasks, 1).unwrap();
    assert_eq!(graph.node_count(), 3);
    assert_eq!(graph.edge_count(), 2);

    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);
    let ctx = roko_graph::CellContext::new();
    let output = engine.execute(&ctx).await.unwrap();

    assert!(output.success);
    assert_eq!(output.node_results.len(), 3);

    // All nodes should complete.
    for result in &output.node_results {
        assert_eq!(result.status, roko_graph::NodeStatus::Complete);
    }
}

/// Test 3: Diamond DAG (T1 -> T2, T1 -> T3, T2+T3 -> T4) with entry/exit detection.
#[tokio::test]
async fn diamond_with_endpoints() {
    let tasks = vec![
        ("T1".to_string(), task_info("Root", &[])),
        ("T2".to_string(), task_info("Left", &["T1"])),
        ("T3".to_string(), task_info("Right", &["T1"])),
        ("T4".to_string(), task_info("Join", &["T2", "T3"])),
    ];

    let (graph, entries, exits) =
        plan_to_graph_with_endpoints("diamond", "/tmp/diamond", &tasks, 2).unwrap();

    assert_eq!(graph.node_count(), 4);
    assert_eq!(graph.edge_count(), 4);
    assert_eq!(entries, vec!["T1"]);
    assert_eq!(exits, vec!["T4"]);

    // Execute through the engine.
    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);
    let ctx = roko_graph::CellContext::new();
    let output = engine.execute(&ctx).await.unwrap();

    assert!(output.success);
    assert_eq!(output.node_results.len(), 4);
}

/// Test 4: Cycle detection produces a clear error.
#[test]
fn cycle_detection() {
    let tasks = vec![
        ("A".to_string(), task_info("A depends on B", &["B"])),
        ("B".to_string(), task_info("B depends on A", &["A"])),
    ];

    let result = plan_to_graph("cyclic", "/tmp/cyclic", &tasks, 1);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, roko_graph::GraphError::CycleDetected),
        "expected CycleDetected, got: {err:?}"
    );
}

/// Test 5: Cognitive loop TOML loads and validates through the engine.
#[tokio::test]
async fn cognitive_loop_loads_and_validates() {
    let toml_str = include_str!("../../../examples/graphs/cognitive-loop.toml");
    let graph = roko_graph::loader::load_from_str(toml_str).unwrap();

    assert_eq!(graph.metadata.name, "cognitive-loop");
    assert_eq!(graph.node_count(), 7);

    // The default registry should have all cognitive loop stub cell types.
    let registry = default_registry();
    let engine = GraphEngine::new(graph, registry);

    let issues = engine.validate();
    assert!(
        issues.is_empty(),
        "cognitive loop should validate cleanly: {issues:?}"
    );

    // Execute -- stub cells are passthroughs, so it should succeed.
    let ctx = roko_graph::CellContext::new();
    let output = engine.execute(&ctx).await.unwrap();
    assert!(output.success);
    assert_eq!(output.node_results.len(), 7);
}
