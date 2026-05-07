//! Canonical metric schema shared across Roko observability surfaces.

use crate::obs::metrics::MetricKind;

/// Current metric schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Common `status` label key.
pub const LABEL_STATUS: &str = "status";
/// Common `role` label key.
pub const LABEL_ROLE: &str = "role";
/// Common `tool` label key.
pub const LABEL_TOOL: &str = "tool";
/// Common `outcome` label key.
pub const LABEL_OUTCOME: &str = "outcome";
/// Common `gate` label key.
pub const LABEL_GATE: &str = "gate";
/// Common `verdict` label key.
pub const LABEL_VERDICT: &str = "verdict";
/// Common `backend` label key.
pub const LABEL_BACKEND: &str = "backend";
/// Common `provider` label key.
pub const LABEL_PROVIDER: &str = "provider";
/// Common `model` label key.
pub const LABEL_MODEL: &str = "model";
/// Common `direction` label key.
pub const LABEL_DIRECTION: &str = "direction";
/// Common `error_type` label key.
pub const LABEL_ERROR_TYPE: &str = "error_type";

/// Total LLM calls, by provider, model, and status.
pub const ROKO_LLM_CALLS_TOTAL: &str = "roko_llm_calls_total";
/// Total LLM errors, by provider, model, and error type.
pub const ROKO_LLM_ERRORS_TOTAL: &str = "roko_llm_errors_total";
/// LLM time-to-first-token in seconds, by provider and model.
pub const ROKO_LLM_TTFT_SECONDS: &str = "roko_llm_ttft_seconds";
/// LLM total request duration in seconds, by provider and model.
pub const ROKO_LLM_REQUEST_DURATION_SECONDS: &str = "roko_llm_request_duration_seconds";
/// Context window utilization in basis points, by provider and model.
pub const ROKO_CONTEXT_UTILIZATION: &str = "roko_context_utilization";
/// Output token throughput for the latest call (tokens/sec gauge), by provider and model.
pub const ROKO_TOKEN_THROUGHPUT_PER_SECOND: &str = "roko_token_throughput_per_second";

/// Total number of plans observed, by status.
pub const ROKO_PLANS_TOTAL: &str = "roko_plans_total";
/// Total number of tasks observed, by status and role.
pub const ROKO_TASKS_TOTAL: &str = "roko_tasks_total";
/// Total number of tool calls, by tool and outcome.
pub const ROKO_TOOL_CALLS_TOTAL: &str = "roko_tool_calls_total";
/// Verify verdicts, by gate and verdict.
pub const ROKO_GATE_VERDICTS_TOTAL: &str = "roko_gate_verdicts_total";
/// Agent turn duration in seconds, by backend and role.
pub const ROKO_AGENT_DURATION_SECONDS: &str = "roko_agent_duration_seconds";
/// LLM tokens consumed or produced, by provider, model, and direction.
pub const ROKO_LLM_TOKENS_TOTAL: &str = "roko_llm_tokens_total";
/// Cumulative LLM spend in USD, by provider and model.
pub const ROKO_LLM_COST_USD_TOTAL: &str = "roko_llm_cost_usd_total";
/// Total HTTP requests handled by an agent sidecar.
pub const ROKO_AGENT_SERVER_REQUESTS_TOTAL: &str = "roko_agent_server_requests_total";
/// Total message-bearing requests handled by an agent sidecar.
pub const ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL: &str =
    "roko_agent_server_message_requests_total";

/// Static descriptor for one canonical metric family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDescriptor {
    /// Prometheus-compatible metric family name.
    pub name: &'static str,
    /// Human-readable help text.
    pub help: &'static str,
    /// Metric family kind.
    pub kind: MetricKind,
    /// Supported label keys for the family.
    pub labels: &'static [&'static str],
}

/// Trait describing a concrete metric schema version.
pub trait MetricSchema {
    /// Returns the monotonically increasing schema version.
    fn schema_version() -> u32;

    /// Returns every canonical metric family in the schema.
    fn metrics() -> &'static [MetricDescriptor];
}

/// The canonical metric schema for `roko-core`, `roko-serve`, and sidecars.
pub struct CanonicalMetricSchema;

impl MetricSchema for CanonicalMetricSchema {
    fn schema_version() -> u32 {
        SCHEMA_VERSION
    }

    fn metrics() -> &'static [MetricDescriptor] {
        CANONICAL_METRICS
    }
}

/// Canonical descriptor for `roko_plans_total`.
pub const ROKO_PLANS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_PLANS_TOTAL,
    help: "Total number of plans observed, by status",
    kind: MetricKind::Counter,
    labels: &[LABEL_STATUS],
};

/// Canonical descriptor for `roko_tasks_total`.
pub const ROKO_TASKS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_TASKS_TOTAL,
    help: "Total number of tasks observed, by status and role",
    kind: MetricKind::Counter,
    labels: &[LABEL_STATUS, LABEL_ROLE],
};

/// Canonical descriptor for `roko_tool_calls_total`.
pub const ROKO_TOOL_CALLS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_TOOL_CALLS_TOTAL,
    help: "Total tool calls, by tool and outcome",
    kind: MetricKind::Counter,
    labels: &[LABEL_TOOL, LABEL_OUTCOME],
};

/// Canonical descriptor for `roko_gate_verdicts_total`.
pub const ROKO_GATE_VERDICTS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_GATE_VERDICTS_TOTAL,
    help: "Verify verdicts, by gate and verdict",
    kind: MetricKind::Counter,
    labels: &[LABEL_GATE, LABEL_VERDICT],
};

/// Canonical descriptor for `roko_agent_duration_seconds`.
pub const ROKO_AGENT_DURATION_SECONDS_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_AGENT_DURATION_SECONDS,
    help: "Agent turn duration in seconds, by backend and role",
    kind: MetricKind::Histogram,
    labels: &[LABEL_BACKEND, LABEL_ROLE],
};

/// Canonical descriptor for `roko_llm_tokens_total`.
pub const ROKO_LLM_TOKENS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_TOKENS_TOTAL,
    help: "LLM tokens consumed/produced, by provider, model, direction",
    kind: MetricKind::Counter,
    labels: &[LABEL_PROVIDER, LABEL_MODEL, LABEL_DIRECTION],
};

/// Canonical descriptor for `roko_llm_cost_usd_total`.
pub const ROKO_LLM_COST_USD_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_COST_USD_TOTAL,
    help: "Cumulative LLM spend in USD, by provider and model",
    kind: MetricKind::Counter,
    labels: &[LABEL_PROVIDER, LABEL_MODEL],
};

/// Canonical descriptor for `roko_agent_server_requests_total`.
pub const ROKO_AGENT_SERVER_REQUESTS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_AGENT_SERVER_REQUESTS_TOTAL,
    help: "Total HTTP requests handled by the agent sidecar",
    kind: MetricKind::Counter,
    labels: &[],
};

/// Canonical descriptor for `roko_agent_server_message_requests_total`.
pub const ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL_DESCRIPTOR: MetricDescriptor =
    MetricDescriptor {
        name: ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL,
        help: "Total message-bearing requests handled by the agent sidecar",
        kind: MetricKind::Counter,
        labels: &[],
    };

/// Canonical descriptor for `roko_llm_calls_total`.
pub const ROKO_LLM_CALLS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_CALLS_TOTAL,
    help: "Total LLM calls by provider, model, and status",
    kind: MetricKind::Counter,
    labels: &[LABEL_PROVIDER, LABEL_MODEL, LABEL_STATUS],
};

/// Canonical descriptor for `roko_llm_errors_total`.
pub const ROKO_LLM_ERRORS_TOTAL_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_ERRORS_TOTAL,
    help: "Total LLM errors by provider, model, and error type",
    kind: MetricKind::Counter,
    labels: &[LABEL_PROVIDER, LABEL_MODEL, LABEL_ERROR_TYPE],
};

/// Canonical descriptor for `roko_llm_ttft_seconds`.
pub const ROKO_LLM_TTFT_SECONDS_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_TTFT_SECONDS,
    help: "LLM time-to-first-token in seconds",
    kind: MetricKind::Histogram,
    labels: &[LABEL_PROVIDER, LABEL_MODEL],
};

/// Canonical descriptor for `roko_llm_request_duration_seconds`.
pub const ROKO_LLM_REQUEST_DURATION_SECONDS_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_LLM_REQUEST_DURATION_SECONDS,
    help: "LLM total request duration in seconds",
    kind: MetricKind::Histogram,
    labels: &[LABEL_PROVIDER, LABEL_MODEL],
};

/// Canonical descriptor for `roko_context_utilization`.
pub const ROKO_CONTEXT_UTILIZATION_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_CONTEXT_UTILIZATION,
    help: "Context window utilization in basis points (1 bp = 0.01%)",
    kind: MetricKind::Gauge,
    labels: &[LABEL_PROVIDER, LABEL_MODEL],
};

/// Canonical descriptor for `roko_token_throughput_per_second`.
pub const ROKO_TOKEN_THROUGHPUT_PER_SECOND_DESCRIPTOR: MetricDescriptor = MetricDescriptor {
    name: ROKO_TOKEN_THROUGHPUT_PER_SECOND,
    help: "Output token throughput for the latest call (integer tokens/sec)",
    kind: MetricKind::Gauge,
    labels: &[LABEL_PROVIDER, LABEL_MODEL],
};

/// Full canonical metric surface shared across the core registry and sidecars.
pub const CANONICAL_METRICS: &[MetricDescriptor] = &[
    ROKO_PLANS_TOTAL_DESCRIPTOR,
    ROKO_TASKS_TOTAL_DESCRIPTOR,
    ROKO_TOOL_CALLS_TOTAL_DESCRIPTOR,
    ROKO_GATE_VERDICTS_TOTAL_DESCRIPTOR,
    ROKO_AGENT_DURATION_SECONDS_DESCRIPTOR,
    ROKO_LLM_TOKENS_TOTAL_DESCRIPTOR,
    ROKO_LLM_COST_USD_TOTAL_DESCRIPTOR,
    ROKO_AGENT_SERVER_REQUESTS_TOTAL_DESCRIPTOR,
    ROKO_AGENT_SERVER_MESSAGE_REQUESTS_TOTAL_DESCRIPTOR,
    ROKO_LLM_CALLS_TOTAL_DESCRIPTOR,
    ROKO_LLM_ERRORS_TOTAL_DESCRIPTOR,
    ROKO_LLM_TTFT_SECONDS_DESCRIPTOR,
    ROKO_LLM_REQUEST_DURATION_SECONDS_DESCRIPTOR,
    ROKO_CONTEXT_UTILIZATION_DESCRIPTOR,
    ROKO_TOKEN_THROUGHPUT_PER_SECOND_DESCRIPTOR,
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn canonical_metrics_includes_new_families() {
        let names: Vec<&str> = CANONICAL_METRICS.iter().map(|d| d.name).collect();
        assert!(
            names.contains(&ROKO_LLM_CALLS_TOTAL),
            "missing {ROKO_LLM_CALLS_TOTAL}"
        );
        assert!(
            names.contains(&ROKO_LLM_ERRORS_TOTAL),
            "missing {ROKO_LLM_ERRORS_TOTAL}"
        );
        assert!(
            names.contains(&ROKO_LLM_TTFT_SECONDS),
            "missing {ROKO_LLM_TTFT_SECONDS}"
        );
        assert!(
            names.contains(&ROKO_LLM_REQUEST_DURATION_SECONDS),
            "missing {ROKO_LLM_REQUEST_DURATION_SECONDS}"
        );
        assert!(
            names.contains(&ROKO_CONTEXT_UTILIZATION),
            "missing {ROKO_CONTEXT_UTILIZATION}"
        );
        assert!(
            names.contains(&ROKO_TOKEN_THROUGHPUT_PER_SECOND),
            "missing {ROKO_TOKEN_THROUGHPUT_PER_SECOND}"
        );
        assert!(
            names.contains(&ROKO_GATE_VERDICTS_TOTAL),
            "missing {ROKO_GATE_VERDICTS_TOTAL}"
        );
    }

    #[test]
    fn canonical_metrics_count_is_15() {
        assert_eq!(CANONICAL_METRICS.len(), 15, "expected 15 canonical metrics");
    }

    #[test]
    fn descriptor_labels_are_valid() {
        for desc in CANONICAL_METRICS {
            let mut seen = BTreeSet::new();
            for label in desc.labels {
                assert!(
                    !label.trim().is_empty(),
                    "metric {name} has an empty label",
                    name = desc.name
                );
                assert!(
                    seen.insert(label),
                    "metric {name} repeats label {label}",
                    name = desc.name
                );
            }
        }
    }
}
