//! Observability primitives: Prometheus text-format metrics, health endpoints,
//! histograms tuned for LLM latency.
//!
//! This module is I/O-free — it produces strings and JSON values. The
//! gateway (or any embedding binary) is responsible for wiring the strings
//! into an HTTP handler. See §40.a + §40.c of the Mori-parity checklist.

pub mod health;
pub mod histograms;
pub mod metrics;
pub mod schema;
pub mod scrub;

pub use health::{
    AlwaysUpProbe, DegradedReason, HealthStatus, NamedProbe, Probe, ProbeRegistry, ReadinessStatus,
};
pub use histograms::{Histogram, HistogramSnapshot, LLM_LATENCY_BUCKETS};
pub use metrics::{
    Counter, Gauge, LabelSet, MetricKind, MetricRegistry, MetricSnapshot, MetricValue,
    STANDARD_METRICS, register_standard_metrics,
};
pub use schema::{CanonicalMetricSchema, MetricDescriptor, MetricSchema, SCHEMA_VERSION};
pub use scrub::{LogScrubber, REDACTED};
