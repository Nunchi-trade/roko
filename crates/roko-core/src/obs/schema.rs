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

/// Total number of plans observed, by status.
pub const ROKO_PLANS_TOTAL: &str = "roko_plans_total";
/// Total number of tasks observed, by status and role.
pub const ROKO_TASKS_TOTAL: &str = "roko_tasks_total";
/// Total number of tool calls, by tool and outcome.
pub const ROKO_TOOL_CALLS_TOTAL: &str = "roko_tool_calls_total";
/// Gate verdicts, by gate and verdict.
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
    help: "Gate verdicts, by gate and verdict",
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
];
