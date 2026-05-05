//! Integration tests for fan-out/fan-in parallelism and conditional edges.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use roko_graph::budget::{BudgetLimits, BudgetTracker};
use roko_graph::condition::{CompareOp, EdgeCondition};
use roko_graph::engine::{CellExecutor, GraphEngine};
use roko_graph::types::*;
use serde_json::json;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn passthrough_executor() -> CellExecutor {
    Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move { NodeOutput::success(&node.id, json!({"cell_type": node.cell_type})) })
    })
}

fn timed_executor(delay_ms: u64) -> CellExecutor {
    Arc::new(move |node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            NodeOutput::success(&node.id, json!({}))
        })
    })
}

fn node(id: &str, cell_type: &str) -> Node {
    Node {
        id: id.into(),
        name: id.into(),
        cell_type: cell_type.into(),
        config: HashMap::new(),
    }
}

fn edge(from: &str, to: &str) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        condition: EdgeCondition::Always,
    }
}

fn conditional_edge(from: &str, to: &str, condition: EdgeCondition) -> Edge {
    Edge {
        from: from.into(),
        to: to.into(),
        condition,
    }
}

// ─── Fan-Out Tests ──────────────────────────────────────────────────────────

#[tokio::test]
async fn fan_out_three_branches_execute_in_parallel() {
    // Source -> (A, B, C) all independent.
    let graph = GraphDef {
        name: "fan-out-3".into(),
        description: "".into(),
        nodes: vec![
            node("source", "test"),
            node("branch_a", "test"),
            node("branch_b", "test"),
            node("branch_c", "test"),
        ],
        edges: vec![
            edge("source", "branch_a"),
            edge("source", "branch_b"),
            edge("source", "branch_c"),
        ],
        config: GraphConfig::default(),
    };

    let start = Instant::now();
    // Each branch takes 50ms. If parallel, total should be ~100ms (source + branches).
    let engine = GraphEngine::new(graph, timed_executor(50));
    let result = engine.execute().await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(result.status, GraphStatus::Success);
    assert_eq!(result.node_outputs.len(), 4);
    // Should complete in roughly 100ms (2 levels of 50ms), not 200ms (serial).
    assert!(
        elapsed < Duration::from_millis(250),
        "elapsed {elapsed:?} suggests branches ran serially"
    );
}

#[tokio::test]
async fn fan_in_waits_for_all_branches() {
    // A, B, C -> Merge (fan-in from 3 sources).
    let graph = GraphDef {
        name: "fan-in-3".into(),
        description: "".into(),
        nodes: vec![
            node("a", "test"),
            node("b", "test"),
            node("c", "test"),
            node("merge", "test"),
        ],
        edges: vec![edge("a", "merge"), edge("b", "merge"), edge("c", "merge")],
        config: GraphConfig::default(),
    };

    // Executor that records input count for merge node.
    let executor: CellExecutor = Arc::new(|node: Node, inputs: Vec<NodeOutput>| {
        Box::pin(async move { NodeOutput::success(&node.id, json!({"input_count": inputs.len()})) })
    });

    let engine = GraphEngine::new(graph, executor);
    let result = engine.execute().await.unwrap();

    let merge_out = result
        .node_outputs
        .iter()
        .find(|o| o.node_id == "merge")
        .unwrap();
    assert_eq!(merge_out.data["input_count"], 3);
}

#[tokio::test]
async fn diamond_dag_fan_out_then_fan_in() {
    // Classic diamond: A -> (B, C) -> D
    let graph = GraphDef {
        name: "diamond".into(),
        description: "".into(),
        nodes: vec![
            node("a", "source"),
            node("b", "branch"),
            node("c", "branch"),
            node("d", "sink"),
        ],
        edges: vec![
            edge("a", "b"),
            edge("a", "c"),
            edge("b", "d"),
            edge("c", "d"),
        ],
        config: GraphConfig::default(),
    };

    let counter = Arc::new(AtomicUsize::new(0));
    let ctr = counter.clone();
    let executor: CellExecutor = Arc::new(move |node: Node, inputs: Vec<NodeOutput>| {
        let ctr = ctr.clone();
        Box::pin(async move {
            let order = ctr.fetch_add(1, Ordering::SeqCst);
            NodeOutput::success(
                &node.id,
                json!({"order": order, "input_count": inputs.len()}),
            )
        })
    });

    let engine = GraphEngine::new(graph, executor);
    let result = engine.execute().await.unwrap();

    assert_eq!(result.status, GraphStatus::Success);
    assert_eq!(result.node_outputs.len(), 4);

    // D must execute last and receive 2 inputs.
    let d_out = result
        .node_outputs
        .iter()
        .find(|o| o.node_id == "d")
        .unwrap();
    assert_eq!(d_out.data["input_count"], 2);

    // A must execute first.
    let a_out = result
        .node_outputs
        .iter()
        .find(|o| o.node_id == "a")
        .unwrap();
    assert_eq!(a_out.data["order"], 0);
}

// ─── Conditional Edge Tests ─────────────────────────────────────────────────

#[tokio::test]
async fn conditional_on_success_prunes_failure_branch() {
    let graph = GraphDef {
        name: "conditional-branch".into(),
        description: "".into(),
        nodes: vec![
            node("check", "gate"),
            node("success_path", "handler"),
            node("failure_path", "handler"),
        ],
        edges: vec![
            conditional_edge("check", "success_path", EdgeCondition::OnSuccess),
            conditional_edge("check", "failure_path", EdgeCondition::OnFailure),
        ],
        config: GraphConfig::default(),
    };

    // Check node succeeds.
    let engine = GraphEngine::new(graph, passthrough_executor());
    let result = engine.execute().await.unwrap();

    let executed_ids: Vec<&str> = result
        .node_outputs
        .iter()
        .map(|o| o.node_id.as_str())
        .collect();

    assert!(executed_ids.contains(&"check"));
    assert!(executed_ids.contains(&"success_path"));
    assert!(!executed_ids.contains(&"failure_path"));
}

#[tokio::test]
async fn conditional_on_failure_runs_failure_branch() {
    let graph = GraphDef {
        name: "failure-branch".into(),
        description: "".into(),
        nodes: vec![
            node("check", "gate"),
            node("success_path", "handler"),
            node("failure_path", "handler"),
        ],
        edges: vec![
            conditional_edge("check", "success_path", EdgeCondition::OnSuccess),
            conditional_edge("check", "failure_path", EdgeCondition::OnFailure),
        ],
        config: GraphConfig::default(),
    };

    // Executor where "check" fails.
    let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            if node.id == "check" {
                NodeOutput::failed(&node.id, "gate check failed")
            } else {
                NodeOutput::success(&node.id, json!({}))
            }
        })
    });

    let engine = GraphEngine::new(graph, executor);
    let result = engine.execute().await.unwrap();

    let executed_ids: Vec<&str> = result
        .node_outputs
        .iter()
        .map(|o| o.node_id.as_str())
        .collect();

    assert!(executed_ids.contains(&"check"));
    assert!(!executed_ids.contains(&"success_path"));
    assert!(executed_ids.contains(&"failure_path"));
}

#[tokio::test]
async fn conditional_when_expression_routes_by_score() {
    let graph = GraphDef {
        name: "score-routing".into(),
        description: "".into(),
        nodes: vec![
            node("eval", "scorer"),
            node("high_quality", "handler"),
            node("low_quality", "handler"),
        ],
        edges: vec![
            conditional_edge(
                "eval",
                "high_quality",
                EdgeCondition::when("score", CompareOp::Gte, toml::Value::Integer(80.into())),
            ),
            conditional_edge(
                "eval",
                "low_quality",
                EdgeCondition::when("score", CompareOp::Lt, toml::Value::Integer(80.into())),
            ),
        ],
        config: GraphConfig::default(),
    };

    // Eval produces score=90 (high quality).
    let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            if node.id == "eval" {
                NodeOutput::success(&node.id, json!({"score": 90}))
            } else {
                NodeOutput::success(&node.id, json!({}))
            }
        })
    });

    let engine = GraphEngine::new(graph, executor);
    let result = engine.execute().await.unwrap();

    let executed_ids: Vec<&str> = result
        .node_outputs
        .iter()
        .map(|o| o.node_id.as_str())
        .collect();

    assert!(executed_ids.contains(&"eval"));
    assert!(executed_ids.contains(&"high_quality"));
    assert!(!executed_ids.contains(&"low_quality"));
}

// ─── Budget Enforcement Tests ───────────────────────────────────────────────

#[tokio::test]
async fn budget_token_limit_stops_execution() {
    let graph = GraphDef {
        name: "budget-test".into(),
        description: "".into(),
        nodes: vec![node("a", "test"), node("b", "test"), node("c", "test")],
        edges: vec![edge("a", "b"), edge("b", "c")],
        config: GraphConfig::default(),
    };

    let budget = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: Some(50),
        max_cost_usd: None,
        deadline: None,
    });

    // Each node uses 30 tokens.
    let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            let mut out = NodeOutput::success(&node.id, json!({}));
            out.tokens_used = 30;
            out
        })
    });

    let engine = GraphEngine::with_budget(graph, executor, budget);
    let result = engine.execute().await.unwrap();

    assert!(matches!(result.status, GraphStatus::BudgetExceeded { .. }));

    // First node (30 tokens) succeeds; remaining exceed 50-token limit.
    let a_out = result
        .node_outputs
        .iter()
        .find(|o| o.node_id == "a")
        .unwrap();
    assert!(a_out.status.is_success());

    // At least one subsequent node should be skipped.
    let has_skipped = result
        .node_outputs
        .iter()
        .any(|o| matches!(o.status, NodeStatus::Skipped { .. }));
    assert!(has_skipped);
}

#[tokio::test]
async fn budget_cost_limit_stops_execution() {
    let graph = GraphDef {
        name: "cost-budget".into(),
        description: "".into(),
        nodes: vec![node("a", "test"), node("b", "test")],
        edges: vec![edge("a", "b")],
        config: GraphConfig::default(),
    };

    let budget = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: None,
        max_cost_usd: Some(0.01),
        deadline: None,
    });

    let executor: CellExecutor = Arc::new(|node: Node, _inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            let mut out = NodeOutput::success(&node.id, json!({}));
            out.cost_usd = 0.02; // Exceeds limit after first node.
            out
        })
    });

    let engine = GraphEngine::with_budget(graph, executor, budget);
    let result = engine.execute().await.unwrap();

    assert!(matches!(result.status, GraphStatus::BudgetExceeded { .. }));
}

// ─── TOML Parsing Test ──────────────────────────────────────────────────────

#[test]
fn parse_graph_toml_basic() {
    let toml_str = r#"
name = "test-pipeline"
description = "A simple pipeline"

[[nodes]]
id = "compose"
name = "Prompt Assembly"
cell_type = "compose"

[nodes.config]
template = "Do the thing"

[[nodes]]
id = "agent"
name = "LLM Agent"
cell_type = "agent"

[nodes.config]
model = "claude-sonnet-4-20250514"
provider = "anthropic"

[[edges]]
from = "compose"
to = "agent"

[edges.condition]
type = "always"

[config]
max_tokens = 10000
max_cost_usd = 1.0
deadline = 300
max_parallelism = 4
"#;

    let graph = roko_graph::engine::parse_graph_toml(toml_str).unwrap();
    assert_eq!(graph.name, "test-pipeline");
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.config.max_tokens, Some(10000));
    assert_eq!(graph.config.max_parallelism, 4);
}

// ─── Complex DAG Test ───────────────────────────────────────────────────────

#[tokio::test]
async fn wide_parallel_dag_all_independent() {
    // 8 nodes, no edges = all at level 0, all parallel.
    let nodes: Vec<Node> = (0..8).map(|i| node(&format!("n{i}"), "test")).collect();
    let graph = GraphDef {
        name: "wide".into(),
        description: "".into(),
        nodes,
        edges: vec![],
        config: GraphConfig {
            max_parallelism: 4,
            ..Default::default()
        },
    };

    let start = Instant::now();
    let engine = GraphEngine::new(graph, timed_executor(50));
    let result = engine.execute().await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(result.node_outputs.len(), 8);
    assert_eq!(result.status, GraphStatus::Success);
    // With max_parallelism=4, 8 nodes at 50ms each should take ~100ms (2 chunks).
    assert!(
        elapsed < Duration::from_millis(250),
        "elapsed {elapsed:?} — expected ~100ms for 2 chunks of 4"
    );
}

#[tokio::test]
async fn multi_level_dag_with_mixed_conditions() {
    // Level 0: source
    // Level 1: gate (on_success from source)
    // Level 2: fast_path (when score >= 70 from gate), slow_path (when score < 70)
    // Level 3: merge (from fast_path OR slow_path)
    let graph = GraphDef {
        name: "multi-level".into(),
        description: "".into(),
        nodes: vec![
            node("source", "data"),
            node("gate", "gate"),
            node("fast_path", "handler"),
            node("slow_path", "handler"),
            node("merge", "aggregator"),
        ],
        edges: vec![
            conditional_edge("source", "gate", EdgeCondition::OnSuccess),
            conditional_edge(
                "gate",
                "fast_path",
                EdgeCondition::when("score", CompareOp::Gte, toml::Value::Integer(70.into())),
            ),
            conditional_edge(
                "gate",
                "slow_path",
                EdgeCondition::when("score", CompareOp::Lt, toml::Value::Integer(70.into())),
            ),
            edge("fast_path", "merge"),
            edge("slow_path", "merge"),
        ],
        config: GraphConfig::default(),
    };

    // gate produces score=85, so fast_path runs.
    let executor: CellExecutor = Arc::new(|node: Node, inputs: Vec<NodeOutput>| {
        Box::pin(async move {
            match node.id.as_str() {
                "gate" => NodeOutput::success(&node.id, json!({"score": 85})),
                "merge" => NodeOutput::success(&node.id, json!({"input_count": inputs.len()})),
                _ => NodeOutput::success(&node.id, json!({})),
            }
        })
    });

    let engine = GraphEngine::new(graph, executor);
    let result = engine.execute().await.unwrap();

    let executed_ids: Vec<&str> = result
        .node_outputs
        .iter()
        .map(|o| o.node_id.as_str())
        .collect();

    assert!(executed_ids.contains(&"source"));
    assert!(executed_ids.contains(&"gate"));
    assert!(executed_ids.contains(&"fast_path"));
    assert!(!executed_ids.contains(&"slow_path"));
    // merge gets input from fast_path only.
    assert!(executed_ids.contains(&"merge"));
}
