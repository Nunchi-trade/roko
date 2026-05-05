//! Integration tests for condition evaluation, budget tracking, and TOML loading.
//!
//! These tests exercise the wired modules: `condition`, `budget`, `types`
//! (NodeOutput, GraphConfig), and the existing `engine`/`loader` integration.

use std::time::Duration;

use roko_graph::budget::{BudgetLimits, BudgetTracker};
use roko_graph::condition::{CompareOp, Condition, evaluate};
use roko_graph::types::{GraphConfig, NodeOutput};
use serde_json::json;

// ─── Condition evaluation tests ─────────────────────────────────────────────

#[test]
fn condition_always_returns_true() {
    let output = NodeOutput::success("n1", json!({}));
    assert!(evaluate(&Condition::Always, &output));
}

#[test]
fn condition_on_success_with_success() {
    let output = NodeOutput::success("n1", json!({"result": "ok"}));
    assert!(evaluate(&Condition::OnSuccess, &output));
}

#[test]
fn condition_on_success_with_failure() {
    let output = NodeOutput::failed("n1", "boom");
    assert!(!evaluate(&Condition::OnSuccess, &output));
}

#[test]
fn condition_on_failure_with_failure() {
    let output = NodeOutput::failed("n1", "boom");
    assert!(evaluate(&Condition::OnFailure, &output));
}

#[test]
fn condition_on_failure_with_success() {
    let output = NodeOutput::success("n1", json!({}));
    assert!(!evaluate(&Condition::OnFailure, &output));
}

#[test]
fn condition_when_eq_string() {
    let output = NodeOutput::success("n1", json!({"status": "pass"}));
    let cond = Condition::when("status", CompareOp::Eq, toml::Value::String("pass".into()));
    assert!(evaluate(&cond, &output));
}

#[test]
fn condition_when_gte_numeric() {
    let output = NodeOutput::success("n1", json!({"score": 90}));
    let cond = Condition::when("score", CompareOp::Gte, toml::Value::Integer(80));
    assert!(evaluate(&cond, &output));
}

#[test]
fn condition_when_lt_numeric_false() {
    let output = NodeOutput::success("n1", json!({"score": 90}));
    let cond = Condition::when("score", CompareOp::Lt, toml::Value::Integer(80));
    assert!(!evaluate(&cond, &output));
}

#[test]
fn condition_when_nested_field() {
    let output = NodeOutput::success("n1", json!({"result": {"status": "complete"}}));
    let cond = Condition::when(
        "result.status",
        CompareOp::Eq,
        toml::Value::String("complete".into()),
    );
    assert!(evaluate(&cond, &output));
}

#[test]
fn condition_when_contains_string() {
    let output = NodeOutput::success("n1", json!({"message": "all tests passed"}));
    let cond = Condition::when(
        "message",
        CompareOp::Contains,
        toml::Value::String("tests passed".into()),
    );
    assert!(evaluate(&cond, &output));
}

#[test]
fn condition_when_missing_field_returns_false() {
    let output = NodeOutput::success("n1", json!({"other": 42}));
    let cond = Condition::when("missing", CompareOp::Eq, toml::Value::Integer(42));
    assert!(!evaluate(&cond, &output));
}

#[test]
fn skipped_node_is_neither_success_nor_failure() {
    let output = NodeOutput::skipped("n1", "budget exceeded");
    assert!(!evaluate(&Condition::OnSuccess, &output));
    assert!(!evaluate(&Condition::OnFailure, &output));
}

#[test]
fn condition_serde_roundtrip() {
    let cond = Condition::when("score", CompareOp::Gte, toml::Value::Integer(90));
    let serialized = serde_json::to_string(&cond).unwrap();
    let parsed: Condition = serde_json::from_str(&serialized).unwrap();
    assert_eq!(cond, parsed);
}

// ─── Budget tracker tests ───────────────────────────────────────────────────

#[test]
fn budget_tracker_starts_empty() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: Some(1000),
        max_cost_usd: Some(1.0),
        deadline: None,
    });
    assert_eq!(tracker.tokens_used(), 0);
    assert!((tracker.cost_usd()).abs() < f64::EPSILON);
    assert!(tracker.check().is_ok());
}

#[test]
fn budget_token_limit_exceeded() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: Some(100),
        max_cost_usd: None,
        deadline: None,
    });
    tracker.record("n1", 80, 0.01, Duration::from_millis(100));
    assert!(tracker.check().is_ok());

    tracker.record("n2", 30, 0.01, Duration::from_millis(50));
    assert!(tracker.check().is_err());
}

#[test]
fn budget_cost_limit_exceeded() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: None,
        max_cost_usd: Some(0.50),
        deadline: None,
    });
    tracker.record("n1", 100, 0.30, Duration::from_millis(100));
    assert!(tracker.check().is_ok());

    tracker.record("n2", 100, 0.25, Duration::from_millis(50));
    assert!(tracker.check().is_err());
}

#[test]
fn budget_no_limits_never_exceeded() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: None,
        max_cost_usd: None,
        deadline: None,
    });
    tracker.record("n1", 999_999, 999.0, Duration::from_secs(9999));
    assert!(tracker.check().is_ok());
}

#[test]
fn budget_remaining_cost_computed_correctly() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: None,
        max_cost_usd: Some(1.0),
        deadline: None,
    });
    tracker.record("n1", 0, 0.35, Duration::ZERO);
    let remaining = tracker.remaining_cost_usd().unwrap();
    assert!((remaining - 0.65).abs() < 0.001);
}

#[test]
fn budget_breakdown_records_all_nodes() {
    let tracker = BudgetTracker::with_limits(BudgetLimits {
        max_tokens: None,
        max_cost_usd: None,
        deadline: None,
    });
    tracker.record("n1", 50, 0.01, Duration::from_millis(10));
    tracker.record("n2", 75, 0.02, Duration::from_millis(20));
    let bd = tracker.breakdown();
    assert_eq!(bd.len(), 2);
    assert_eq!(bd[0].node_id, "n1");
    assert_eq!(bd[1].node_id, "n2");
}

// ─── GraphConfig and NodeOutput tests ───────────────────────────────────────

#[test]
fn graph_config_defaults_are_none() {
    let config = GraphConfig::default();
    assert!(config.max_tokens.is_none());
    assert!(config.max_cost_usd.is_none());
    assert!(config.deadline.is_none());
}

#[test]
fn budget_tracker_from_config() {
    let config = GraphConfig {
        max_tokens: Some(500),
        max_cost_usd: Some(0.10),
        deadline: Some(Duration::from_secs(30)),
    };
    let tracker = BudgetTracker::from_config(&config);
    assert!(tracker.check().is_ok());
    assert_eq!(tracker.tokens_used(), 0);
}

#[test]
fn node_output_success_properties() {
    let output = NodeOutput::success("n1", json!({"result": "ok"}));
    assert!(output.status.is_success());
    assert!(!output.status.is_failed());
    assert!(!output.status.is_skipped());
    assert_eq!(output.node_id, "n1");
    assert!(output.error.is_none());
    assert_eq!(output.data["result"], "ok");
}

#[test]
fn node_output_failed_properties() {
    let output = NodeOutput::failed("n1", "boom");
    assert!(output.status.is_failed());
    assert!(!output.status.is_success());
    assert_eq!(output.error.as_deref(), Some("boom"));
}

#[test]
fn node_output_skipped_properties() {
    let output = NodeOutput::skipped("n1", "budget exceeded");
    assert!(output.status.is_skipped());
    assert!(!output.status.is_success());
    assert!(!output.status.is_failed());
    assert_eq!(output.error.as_deref(), Some("budget exceeded"));
}

// ─── Engine integration with live API ───────────────────────────────────────

#[tokio::test]
async fn engine_runs_linear_graph() {
    use roko_graph::cell::CellContext;
    use roko_graph::engine::{GraphEngine, NodeStatus, default_registry};
    use roko_graph::loader;

    let toml_str = r#"
[graph]
name = "linear"

[[nodes]]
id = "a"
cell_type = "noop"

[[nodes]]
id = "b"
cell_type = "noop"

[[edges]]
from = "a"
to = "b"
"#;
    let graph = loader::load_from_str(toml_str).unwrap();
    let engine = GraphEngine::new(graph, default_registry());
    let ctx = CellContext::new();
    let output = engine.execute(&ctx).await.unwrap();

    assert!(output.success);
    assert_eq!(output.node_results.len(), 2);
    assert!(
        output
            .node_results
            .iter()
            .all(|r| r.status == NodeStatus::Complete)
    );
}

#[tokio::test]
async fn engine_unknown_cell_type_reported() {
    use roko_graph::cell::CellContext;
    use roko_graph::engine::GraphEngine;
    use roko_graph::loader;
    use roko_graph::registry::CellRegistry;

    let toml_str = r#"
[graph]
name = "bad"

[[nodes]]
id = "a"
cell_type = "nonexistent"
"#;
    let graph = loader::load_from_str(toml_str).unwrap();
    let engine = GraphEngine::new(graph, CellRegistry::new());
    let issues = engine.validate();
    assert!(!issues.is_empty());
    assert!(issues[0].contains("nonexistent"));
}
