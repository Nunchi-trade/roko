//! Regression tests for the shared observability metric schema.

use roko_core::obs::schema::{self, CanonicalMetricSchema, MetricSchema};

fn agent_server_state_source() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../roko-agent-server/src/state.rs"
    ))
}

fn agent_server_metric_snapshot_source() -> &'static str {
    let source = agent_server_state_source();
    let start = source
        .find("pub fn snapshot(&self) -> serde_json::Value {")
        .expect("find agent-server metric snapshot");
    let end = source[start..]
        .find("fn counter_snapshot(")
        .map(|offset| start + offset)
        .expect("find agent-server counter snapshot helper");
    &source[start..end]
}

fn quoted_literals(source: &str) -> Vec<&str> {
    let mut literals = Vec::new();
    let mut start = None;
    let mut escaped = false;

    for (index, ch) in source.char_indices() {
        if let Some(literal_start) = start {
            if escaped {
                escaped = false;
                continue;
            }

            match ch {
                '\\' => escaped = true,
                '"' => {
                    literals.push(&source[literal_start..index]);
                    start = None;
                }
                _ => {}
            }
        } else if ch == '"' {
            start = Some(index + 1);
        }
    }

    literals
}

fn looks_like_metric_family_name(literal: &str) -> bool {
    let valid_chars = literal
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_');
    let metric_suffix = literal.ends_with("_total")
        || literal.ends_with("_seconds")
        || literal.ends_with("_milliseconds")
        || literal.ends_with("_ms")
        || literal.ends_with("_bytes")
        || literal.ends_with("_usd");

    valid_chars && metric_suffix
}

#[test]
fn canonical_schema_lists_agent_server_metric_families() {
    let names: Vec<_> = CanonicalMetricSchema::metrics()
        .iter()
        .map(|descriptor| descriptor.name)
        .collect();

    assert!(names.contains(&schema::ROKO_AGENT_SERVER_REQUESTS_TOTAL));
    assert!(names.contains(&schema::ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL));
}

#[test]
fn agent_server_metrics_use_canonical_schema_constants() {
    let source = agent_server_metric_snapshot_source();

    assert!(source.contains("ROKO_AGENT_SERVER_REQUESTS_TOTAL_DESCRIPTOR"));
    assert!(source.contains("ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL_DESCRIPTOR"));

    let handwritten_metric_literals: Vec<_> = quoted_literals(source)
        .into_iter()
        .filter(|literal| looks_like_metric_family_name(literal))
        .collect();
    assert!(
        handwritten_metric_literals.is_empty(),
        "raw metric family literals found in agent-server metric snapshot: {handwritten_metric_literals:?}"
    );
}
