//! Conditional edge evaluation for graph traversal.
//!
//! Edges can be annotated with conditions that determine whether
//! a downstream node should be activated based on the upstream
//! node's output.

use serde::{Deserialize, Serialize};

use crate::types::NodeOutput;

/// Comparison operators for `When` conditions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CompareOp {
    /// Field equals value.
    Eq,
    /// Field does not equal value.
    Ne,
    /// Field is greater than value (numeric).
    Gt,
    /// Field is greater than or equal to value (numeric).
    Gte,
    /// Field is less than value (numeric).
    Lt,
    /// Field is less than or equal to value (numeric).
    Lte,
    /// Field contains value (string substring or array element).
    Contains,
}

/// Condition that must be satisfied for an edge to be traversed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EdgeCondition {
    /// Always traverse this edge (default).
    Always,
    /// Only traverse if the source node succeeded.
    OnSuccess,
    /// Only traverse if the source node failed.
    OnFailure,
    /// Traverse if a field in the node's output data matches a condition.
    When {
        /// JSON pointer path into the node output data (e.g. "score" or "result.status").
        field: String,
        /// Comparison operator.
        op: CompareOp,
        /// Value to compare against.
        value: toml::Value,
    },
}

impl EdgeCondition {
    /// Create a default `Always` condition.
    #[must_use]
    pub fn always() -> Self {
        Self::Always
    }

    /// Create an `OnSuccess` condition.
    #[must_use]
    pub fn on_success() -> Self {
        Self::OnSuccess
    }

    /// Create an `OnFailure` condition.
    #[must_use]
    pub fn on_failure() -> Self {
        Self::OnFailure
    }

    /// Create a `When` condition.
    #[must_use]
    pub fn when(field: impl Into<String>, op: CompareOp, value: toml::Value) -> Self {
        Self::When {
            field: field.into(),
            op,
            value,
        }
    }
}

impl Default for EdgeCondition {
    fn default() -> Self {
        Self::Always
    }
}

/// Evaluate a condition against a node's output.
///
/// Returns `true` if the edge should be traversed, `false` otherwise.
pub fn evaluate(condition: &EdgeCondition, node_output: &NodeOutput) -> bool {
    match condition {
        EdgeCondition::Always => true,
        EdgeCondition::OnSuccess => node_output.status.is_success(),
        EdgeCondition::OnFailure => node_output.status.is_failed(),
        EdgeCondition::When { field, op, value } => evaluate_when(field, op, value, node_output),
    }
}

/// Evaluate a `When` condition by extracting a field from the node output
/// and comparing it against the expected value.
fn evaluate_when(field: &str, op: &CompareOp, expected: &toml::Value, output: &NodeOutput) -> bool {
    // Navigate into the output data using dot-separated field path.
    let actual = resolve_field(&output.data, field);
    let Some(actual) = actual else {
        return false;
    };

    compare_values(op, actual, expected)
}

/// Resolve a dot-separated field path into a JSON value.
fn resolve_field<'a>(data: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = data;
    for segment in path.split('.') {
        match current {
            serde_json::Value::Object(map) => {
                current = map.get(segment)?;
            }
            serde_json::Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// Compare an actual JSON value against an expected TOML value using the given operator.
fn compare_values(op: &CompareOp, actual: &serde_json::Value, expected: &toml::Value) -> bool {
    match op {
        CompareOp::Eq => values_equal(actual, expected),
        CompareOp::Ne => !values_equal(actual, expected),
        CompareOp::Gt => numeric_cmp(actual, expected).is_some_and(|ord| ord.is_gt()),
        CompareOp::Gte => numeric_cmp(actual, expected).is_some_and(|ord| ord.is_ge()),
        CompareOp::Lt => numeric_cmp(actual, expected).is_some_and(|ord| ord.is_lt()),
        CompareOp::Lte => numeric_cmp(actual, expected).is_some_and(|ord| ord.is_le()),
        CompareOp::Contains => contains_check(actual, expected),
    }
}

/// Check equality between a JSON value and a TOML value.
fn values_equal(json: &serde_json::Value, toml_val: &toml::Value) -> bool {
    match json {
        serde_json::Value::String(a) => toml_val.as_str().is_some_and(|b| a == b),
        serde_json::Value::Number(a) => {
            if let Some(ti) = toml_val.as_integer() {
                a.as_i64().is_some_and(|n| n == ti)
            } else if let Some(tf) = toml_val.as_float() {
                a.as_f64().is_some_and(|n| (n - tf).abs() < f64::EPSILON)
            } else {
                false
            }
        }
        serde_json::Value::Bool(a) => toml_val.as_bool().is_some_and(|b| *a == b),
        _ => false,
    }
}

/// Numeric comparison between JSON and TOML values.
fn numeric_cmp(json: &serde_json::Value, toml_val: &toml::Value) -> Option<std::cmp::Ordering> {
    let actual_f64 = json.as_f64()?;
    let expected_f64 = toml_val
        .as_integer()
        .map(|i| i as f64)
        .or_else(|| toml_val.as_float())?;
    actual_f64.partial_cmp(&expected_f64)
}

/// Check if `actual` contains `expected` (string substring or array element).
fn contains_check(actual: &serde_json::Value, expected: &toml::Value) -> bool {
    match actual {
        serde_json::Value::String(haystack) => expected
            .as_str()
            .is_some_and(|needle| haystack.contains(needle)),
        serde_json::Value::Array(arr) => arr.iter().any(|item| values_equal(item, expected)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn always_evaluates_true() {
        let output = NodeOutput::success("n1", json!({}));
        assert!(evaluate(&EdgeCondition::Always, &output));
    }

    #[test]
    fn on_success_with_success() {
        let output = NodeOutput::success("n1", json!({"result": "ok"}));
        assert!(evaluate(&EdgeCondition::OnSuccess, &output));
    }

    #[test]
    fn on_success_with_failure() {
        let output = NodeOutput::failed("n1", "boom");
        assert!(!evaluate(&EdgeCondition::OnSuccess, &output));
    }

    #[test]
    fn on_failure_with_failure() {
        let output = NodeOutput::failed("n1", "boom");
        assert!(evaluate(&EdgeCondition::OnFailure, &output));
    }

    #[test]
    fn on_failure_with_success() {
        let output = NodeOutput::success("n1", json!({}));
        assert!(!evaluate(&EdgeCondition::OnFailure, &output));
    }

    #[test]
    fn when_eq_string() {
        let output = NodeOutput::success("n1", json!({"status": "pass"}));
        let cond = EdgeCondition::when("status", CompareOp::Eq, toml::Value::String("pass".into()));
        assert!(evaluate(&cond, &output));
    }

    #[test]
    fn when_gt_numeric() {
        let output = NodeOutput::success("n1", json!({"score": 85}));
        let cond = EdgeCondition::when("score", CompareOp::Gt, toml::Value::Integer(70.into()));
        assert!(evaluate(&cond, &output));
    }

    #[test]
    fn when_nested_field() {
        let output = NodeOutput::success("n1", json!({"result": {"status": "complete"}}));
        let cond = EdgeCondition::when(
            "result.status",
            CompareOp::Eq,
            toml::Value::String("complete".into()),
        );
        assert!(evaluate(&cond, &output));
    }

    #[test]
    fn when_contains_string() {
        let output = NodeOutput::success("n1", json!({"message": "all tests passed"}));
        let cond = EdgeCondition::when(
            "message",
            CompareOp::Contains,
            toml::Value::String("tests passed".into()),
        );
        assert!(evaluate(&cond, &output));
    }

    #[test]
    fn when_missing_field_returns_false() {
        let output = NodeOutput::success("n1", json!({"other": 42}));
        let cond = EdgeCondition::when("missing", CompareOp::Eq, toml::Value::Integer(42.into()));
        assert!(!evaluate(&cond, &output));
    }

    #[test]
    fn skipped_node_is_neither_success_nor_failure() {
        let output = NodeOutput::skipped("n1", "budget exceeded");
        assert!(!evaluate(&EdgeCondition::OnSuccess, &output));
        assert!(!evaluate(&EdgeCondition::OnFailure, &output));
    }

    #[test]
    fn condition_serde_roundtrip() {
        let cond = EdgeCondition::when("score", CompareOp::Gte, toml::Value::Integer(90.into()));
        let json = serde_json::to_string(&cond).unwrap();
        let parsed: EdgeCondition = serde_json::from_str(&json).unwrap();
        assert_eq!(cond, parsed);
    }
}
