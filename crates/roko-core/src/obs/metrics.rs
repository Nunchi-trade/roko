//! Prometheus-flavoured metric primitives (§40.1 + §40.2).
//!
//! This module produces metric strings in the [Prometheus text exposition
//! format](https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format).
//! It does **not** serve HTTP; embedding binaries (gateway, mirage-rs,
//! roko-cli) call [`MetricRegistry::render_prometheus`] and plug the result
//! into their own axum / hyper route.
//!
//! Manual implementation (no `prometheus` crate dep) keeps roko-core's
//! dependency surface small. The text format is genuinely simple:
//!
//! ```text
//! # HELP name help text
//! # TYPE name counter|gauge|histogram
//! name{label="value",...} number
//! ```

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::obs::histograms::{Histogram, HistogramSnapshot, escape_help, format_f64};
use crate::obs::schema;

// ─── Primitives: Counter + Gauge ─────────────────────────────────────

/// Monotonically increasing `u64` counter. Cheap, lock-free, clonable
/// handle (`Arc` inside).
#[derive(Debug, Clone)]
pub struct Counter {
    inner: Arc<AtomicU64>,
}

impl Counter {
    /// Construct a new counter initialised to zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Increment by 1.
    pub fn inc(&self) {
        self.inner.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by `n`.
    pub fn inc_by(&self, n: u64) {
        self.inner.fetch_add(n, Ordering::Relaxed);
    }

    /// Current counter value.
    #[must_use]
    pub fn get(&self) -> u64 {
        self.inner.load(Ordering::Relaxed)
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

/// Signed gauge (can go up and down). Stored as `i64` atomics for `add`
/// and `sub`; `set` takes a signed integer.
#[derive(Debug, Clone)]
pub struct Gauge {
    inner: Arc<AtomicI64>,
}

impl Gauge {
    /// Construct a new gauge initialised to zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicI64::new(0)),
        }
    }

    /// Set the gauge to `v`.
    pub fn set(&self, v: i64) {
        self.inner.store(v, Ordering::Relaxed);
    }

    /// Add `n` to the gauge.
    pub fn add(&self, n: i64) {
        self.inner.fetch_add(n, Ordering::Relaxed);
    }

    /// Subtract `n` from the gauge.
    pub fn sub(&self, n: i64) {
        self.inner.fetch_sub(n, Ordering::Relaxed);
    }

    /// Current gauge value.
    #[must_use]
    pub fn get(&self) -> i64 {
        self.inner.load(Ordering::Relaxed)
    }
}

impl Default for Gauge {
    fn default() -> Self {
        Self::new()
    }
}

// ─── LabelSet ────────────────────────────────────────────────────────

/// A stable-sorted set of label key/value pairs used for metric identity
/// and Prometheus line formatting.
///
/// Label values are escaped per Prometheus spec (backslash, double-quote,
/// newline). Keys are assumed to already be valid Prometheus label names
/// (`[a-zA-Z_][a-zA-Z0-9_]*`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct LabelSet {
    pairs: Vec<(&'static str, String)>,
}

impl LabelSet {
    /// Construct an empty label set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct from a slice of static-key pairs. Duplicate keys are
    /// resolved to the last-supplied value (matching `HashMap` semantics).
    #[must_use]
    pub fn from_pairs(pairs: &[(&'static str, &str)]) -> Self {
        let mut out = Self::new();
        for (k, v) in pairs {
            out.insert(k, (*v).to_string());
        }
        out
    }

    /// Insert / replace a label. Keeps the internal vec stable-sorted.
    pub fn insert(&mut self, key: &'static str, value: String) {
        if let Some(slot) = self.pairs.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = value;
        } else {
            self.pairs.push((key, value));
            self.pairs.sort_by_key(|(k, _)| *k);
        }
    }

    /// Number of labels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    /// True if no labels are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// Borrow the sorted pairs.
    #[must_use]
    pub fn pairs(&self) -> &[(&'static str, String)] {
        &self.pairs
    }

    /// Render the inside of the Prometheus label-set (without the
    /// enclosing curly braces). Returns an empty string when there are
    /// no labels.
    #[must_use]
    pub fn render_inner(&self) -> String {
        let mut out = String::new();
        for (i, (k, v)) in self.pairs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(k);
            out.push_str("=\"");
            out.push_str(&escape_label_value(v));
            out.push('"');
        }
        out
    }

    /// Render as `{k1="v1",k2="v2"}`; empty string if the set is empty.
    #[must_use]
    pub fn render_braced(&self) -> String {
        if self.pairs.is_empty() {
            String::new()
        } else {
            format!("{{{}}}", self.render_inner())
        }
    }
}

fn escape_label_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

// ─── MetricKind ──────────────────────────────────────────────────────

/// The three metric kinds exposed to Prometheus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MetricKind {
    /// Monotonically increasing counter.
    Counter,
    /// Signed up/down gauge.
    Gauge,
    /// Observation histogram.
    Histogram,
}

impl MetricKind {
    const fn type_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::Gauge => "gauge",
            Self::Histogram => "histogram",
        }
    }
}

// ─── MetricRegistry ──────────────────────────────────────────────────

/// A registered metric family (all shapes share `name` + `help`).
struct Family {
    name: String,
    help: String,
    kind: MetricKind,
    entries: Vec<Entry>,
}

struct Entry {
    labels: LabelSet,
    variant: Variant,
}

enum Variant {
    Counter(Counter),
    Gauge(Gauge),
    Histogram(Arc<Histogram>),
}

/// Thread-safe metric registry.
///
/// Registration takes the write-lock; metric mutation is lock-free
/// (counters/gauges use atomics, histograms use their own CAS loop).
/// `render_prometheus` takes the read-lock.
pub struct MetricRegistry {
    inner: RwLock<Vec<Family>>,
}

impl MetricRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
        }
    }

    /// Register or look-up a counter. Idempotent: a second call with the
    /// same `(name, labels)` returns a handle to the existing counter.
    /// `help` from the first registration wins.
    pub fn register_counter(&self, name: &str, help: &str, labels: LabelSet) -> Counter {
        let mut families = self.inner.write();
        let family = get_or_insert_family(&mut families, name, help, MetricKind::Counter);
        for entry in &family.entries {
            if entry.labels == labels {
                if let Variant::Counter(c) = &entry.variant {
                    let handle = c.clone();
                    drop(families);
                    return handle;
                }
            }
        }
        let c = Counter::new();
        family.entries.push(Entry {
            labels,
            variant: Variant::Counter(c.clone()),
        });
        drop(families);
        c
    }

    /// Register or look-up a gauge. Same idempotence contract as
    /// [`Self::register_counter`].
    pub fn register_gauge(&self, name: &str, help: &str, labels: LabelSet) -> Gauge {
        let mut families = self.inner.write();
        let family = get_or_insert_family(&mut families, name, help, MetricKind::Gauge);
        for entry in &family.entries {
            if entry.labels == labels {
                if let Variant::Gauge(g) = &entry.variant {
                    let handle = g.clone();
                    drop(families);
                    return handle;
                }
            }
        }
        let g = Gauge::new();
        family.entries.push(Entry {
            labels,
            variant: Variant::Gauge(g.clone()),
        });
        drop(families);
        g
    }

    /// Register or look-up a histogram.
    ///
    /// If an existing histogram with the same `(name, labels)` is found,
    /// the supplied `buckets` argument is ignored — the original boundaries
    /// win. This matches the Prometheus client-library contract.
    pub fn register_histogram(
        &self,
        name: &str,
        help: &str,
        labels: LabelSet,
        buckets: Vec<f64>,
    ) -> Arc<Histogram> {
        let mut families = self.inner.write();
        let family = get_or_insert_family(&mut families, name, help, MetricKind::Histogram);
        for entry in &family.entries {
            if entry.labels == labels {
                if let Variant::Histogram(h) = &entry.variant {
                    let handle = Arc::clone(h);
                    drop(families);
                    return handle;
                }
            }
        }
        let h = Arc::new(Histogram::new(buckets));
        family.entries.push(Entry {
            labels,
            variant: Variant::Histogram(Arc::clone(&h)),
        });
        drop(families);
        h
    }

    /// Look up an existing counter without registering.
    #[must_use]
    pub fn get_counter(&self, name: &str, labels: &LabelSet) -> Option<Counter> {
        let families = self.inner.read();
        let mut out = None;
        for family in families.iter() {
            if family.name == name && family.kind == MetricKind::Counter {
                for entry in &family.entries {
                    if &entry.labels == labels {
                        if let Variant::Counter(c) = &entry.variant {
                            out = Some(c.clone());
                            break;
                        }
                    }
                }
            }
        }
        drop(families);
        out
    }

    /// Look up an existing gauge without registering.
    #[must_use]
    pub fn get_gauge(&self, name: &str, labels: &LabelSet) -> Option<Gauge> {
        let families = self.inner.read();
        let mut out = None;
        for family in families.iter() {
            if family.name == name && family.kind == MetricKind::Gauge {
                for entry in &family.entries {
                    if &entry.labels == labels {
                        if let Variant::Gauge(g) = &entry.variant {
                            out = Some(g.clone());
                            break;
                        }
                    }
                }
            }
        }
        drop(families);
        out
    }

    /// Look up an existing histogram without registering.
    #[must_use]
    pub fn get_histogram(&self, name: &str, labels: &LabelSet) -> Option<Arc<Histogram>> {
        let families = self.inner.read();
        let mut out = None;
        for family in families.iter() {
            if family.name == name && family.kind == MetricKind::Histogram {
                for entry in &family.entries {
                    if &entry.labels == labels {
                        if let Variant::Histogram(h) = &entry.variant {
                            out = Some(Arc::clone(h));
                            break;
                        }
                    }
                }
            }
        }
        drop(families);
        out
    }

    /// Number of registered metric families.
    #[must_use]
    pub fn family_count(&self) -> usize {
        let families = self.inner.read();
        let n = families.len();
        drop(families);
        n
    }

    /// Render the entire registry in Prometheus text-exposition format.
    ///
    /// Output groups every family under a single `# HELP` / `# TYPE`
    /// pair and emits one line per (family, labels) entry.
    #[must_use]
    pub fn render_prometheus(&self) -> String {
        let families = self.inner.read();
        let mut out = String::new();
        for family in families.iter() {
            let _ = writeln!(out, "# HELP {} {}", family.name, escape_help(&family.help));
            let _ = writeln!(out, "# TYPE {} {}", family.name, family.kind.type_str());
            for entry in &family.entries {
                match &entry.variant {
                    Variant::Counter(c) => {
                        let lbl = entry.labels.render_braced();
                        let _ = writeln!(out, "{}{lbl} {}", family.name, c.get());
                    }
                    Variant::Gauge(g) => {
                        let lbl = entry.labels.render_braced();
                        let _ = writeln!(out, "{}{lbl} {}", family.name, g.get());
                    }
                    Variant::Histogram(h) => {
                        out.push_str(&render_histogram_lines(&family.name, h, &entry.labels));
                    }
                }
            }
        }
        drop(families);
        out
    }

    /// Serialize every metric as a [`MetricSnapshot`] list (JSON-friendly).
    #[must_use]
    pub fn snapshot(&self) -> Vec<MetricSnapshot> {
        let families = self.inner.read();
        let mut out = Vec::new();
        for family in families.iter() {
            for entry in &family.entries {
                let value = match &entry.variant {
                    Variant::Counter(c) => MetricValue::Counter(c.get()),
                    Variant::Gauge(g) => MetricValue::Gauge(g.get()),
                    Variant::Histogram(h) => MetricValue::Histogram(h.snapshot()),
                };
                out.push(MetricSnapshot {
                    name: family.name.clone(),
                    help: family.help.clone(),
                    kind: family.kind,
                    labels: entry
                        .labels
                        .pairs()
                        .iter()
                        .map(|(k, v)| ((*k).to_string(), v.clone()))
                        .collect(),
                    value,
                });
            }
        }
        drop(families);
        out
    }
}

impl Default for MetricRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn get_or_insert_family<'a>(
    families: &'a mut Vec<Family>,
    name: &str,
    help: &str,
    kind: MetricKind,
) -> &'a mut Family {
    // Find-or-push; index-based so we can return a borrow that covers
    // both branches without lifetime juggling.
    let idx = families
        .iter()
        .position(|f| f.name == name)
        .unwrap_or_else(|| {
            families.push(Family {
                name: name.to_string(),
                help: help.to_string(),
                kind,
                entries: Vec::new(),
            });
            families.len() - 1
        });
    &mut families[idx]
}

fn render_histogram_lines(name: &str, h: &Histogram, labels: &LabelSet) -> String {
    let snap = h.snapshot();
    let base = labels.render_inner();
    let sep = if base.is_empty() { "" } else { "," };
    let mut out = String::new();
    for (upper, cum) in snap.buckets.iter().zip(snap.counts.iter()) {
        let _ = writeln!(
            out,
            "{name}_bucket{{{base}{sep}le=\"{}\"}} {cum}",
            format_f64(*upper),
        );
    }
    let inf_cum = snap.counts.last().copied().unwrap_or(0);
    let _ = writeln!(out, "{name}_bucket{{{base}{sep}le=\"+Inf\"}} {inf_cum}");
    if base.is_empty() {
        let _ = writeln!(out, "{name}_sum {}", format_f64(snap.sum));
        let _ = writeln!(out, "{name}_count {}", snap.count);
    } else {
        let _ = writeln!(out, "{name}_sum{{{base}}} {}", format_f64(snap.sum));
        let _ = writeln!(out, "{name}_count{{{base}}} {}", snap.count);
    }
    out
}

// ─── Snapshot (JSON-friendly) ────────────────────────────────────────

/// JSON-friendly view of one metric entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricSnapshot {
    /// Metric family name.
    pub name: String,
    /// Free-form help text.
    pub help: String,
    /// Family kind.
    pub kind: MetricKind,
    /// Label pairs (sorted).
    pub labels: Vec<(String, String)>,
    /// The current value.
    pub value: MetricValue,
}

/// The typed value carried by a [`MetricSnapshot`].
///
/// Serialised with an internal `type` tag so downstream consumers can
/// switch on `value.type`. (A `tag = ...` on a newtype variant holding a
/// scalar is not supported by serde; we use adjacent tagging instead.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MetricValue {
    /// Counter observation.
    Counter(u64),
    /// Gauge observation.
    Gauge(i64),
    /// Histogram snapshot.
    Histogram(HistogramSnapshot),
}

// ─── Standard metric catalog (§40.2) ─────────────────────────────────

/// Standard metrics every Roko binary registers (§40.2 + task 096 additions).
///
/// This list is sourced from the canonical schema so core registration and
/// sidecar emission cannot drift on names, help text, or kinds.
pub const STANDARD_METRICS: &[schema::MetricDescriptor] = &[
    schema::ROKO_PLANS_TOTAL_DESCRIPTOR,
    schema::ROKO_TASKS_TOTAL_DESCRIPTOR,
    schema::ROKO_TOOL_CALLS_TOTAL_DESCRIPTOR,
    schema::ROKO_GATE_VERDICTS_TOTAL_DESCRIPTOR,
    schema::ROKO_AGENT_DURATION_SECONDS_DESCRIPTOR,
    schema::ROKO_LLM_TOKENS_TOTAL_DESCRIPTOR,
    schema::ROKO_LLM_COST_USD_TOTAL_DESCRIPTOR,
    schema::ROKO_LLM_CALLS_TOTAL_DESCRIPTOR,
    schema::ROKO_LLM_ERRORS_TOTAL_DESCRIPTOR,
    schema::ROKO_LLM_TTFT_SECONDS_DESCRIPTOR,
    schema::ROKO_LLM_REQUEST_DURATION_SECONDS_DESCRIPTOR,
    schema::ROKO_CONTEXT_UTILIZATION_DESCRIPTOR,
    schema::ROKO_TOKEN_THROUGHPUT_PER_SECOND_DESCRIPTOR,
];

/// Pre-register every metric in [`STANDARD_METRICS`] with zero labels.
///
/// This gives callers stable handles they can look up via
/// [`MetricRegistry::get_counter`] / [`MetricRegistry::get_histogram`].
/// Callers that need labels (e.g. `status="running"`) should call
/// `register_counter` directly with the appropriate [`LabelSet`] — the
/// family already exists at that point and the registration becomes a
/// pure look-up.
pub fn register_standard_metrics(registry: &MetricRegistry) {
    use crate::obs::histograms::LLM_LATENCY_BUCKETS;
    for descriptor in STANDARD_METRICS {
        match descriptor.kind {
            MetricKind::Counter => {
                let _ =
                    registry.register_counter(descriptor.name, descriptor.help, LabelSet::new());
            }
            MetricKind::Gauge => {
                let _ = registry.register_gauge(descriptor.name, descriptor.help, LabelSet::new());
            }
            MetricKind::Histogram => {
                let _ = registry.register_histogram(
                    descriptor.name,
                    descriptor.help,
                    LabelSet::new(),
                    LLM_LATENCY_BUCKETS.to_vec(),
                );
            }
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    #[test]
    fn counter_inc_inc_by_get() {
        let c = Counter::new();
        assert_eq!(c.get(), 0);
        c.inc();
        c.inc();
        c.inc_by(5);
        assert_eq!(c.get(), 7);
    }

    #[test]
    fn counter_is_concurrent() {
        let c = Counter::new();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let c = c.clone();
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        c.inc();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread joined");
        }
        assert_eq!(c.get(), 4000);
    }

    #[test]
    fn gauge_set_add_sub_get() {
        let g = Gauge::new();
        assert_eq!(g.get(), 0);
        g.set(10);
        assert_eq!(g.get(), 10);
        g.add(5);
        assert_eq!(g.get(), 15);
        g.sub(3);
        assert_eq!(g.get(), 12);
        g.set(-4);
        assert_eq!(g.get(), -4);
    }

    #[test]
    fn label_set_is_sorted() {
        let ls = LabelSet::from_pairs(&[("role", "coder"), ("backend", "claude")]);
        let pairs = ls.pairs();
        assert_eq!(pairs[0].0, "backend");
        assert_eq!(pairs[1].0, "role");
    }

    #[test]
    fn label_set_escapes_values() {
        let ls = LabelSet::from_pairs(&[("k", "a\"b\\c\nd")]);
        let inner = ls.render_inner();
        assert_eq!(inner, r#"k="a\"b\\c\nd""#);
    }

    #[test]
    fn label_set_empty_renders_empty() {
        let ls = LabelSet::new();
        assert_eq!(ls.render_braced(), "");
        assert_eq!(ls.render_inner(), "");
    }

    #[test]
    fn registry_register_counter_is_idempotent() {
        let reg = MetricRegistry::new();
        let a = reg.register_counter("foo_total", "foo count", LabelSet::new());
        let b = reg.register_counter("foo_total", "foo count", LabelSet::new());
        a.inc();
        b.inc_by(2);
        // They share atomic state.
        assert_eq!(a.get(), 3);
        assert_eq!(b.get(), 3);
    }

    #[test]
    fn registry_get_counter_returns_existing() {
        let reg = MetricRegistry::new();
        let labels = LabelSet::from_pairs(&[("status", "ok")]);
        let _ = reg.register_counter("foo_total", "h", labels.clone());
        let c = reg.get_counter("foo_total", &labels).expect("exists");
        c.inc();
        assert_eq!(c.get(), 1);
    }

    #[test]
    fn registry_get_counter_none_when_missing() {
        let reg = MetricRegistry::new();
        assert!(reg.get_counter("nope", &LabelSet::new()).is_none());
    }

    #[test]
    fn registry_render_prometheus_basic() {
        let reg = MetricRegistry::new();
        let c = reg.register_counter("roko_plans_total", "plans by status", LabelSet::new());
        c.inc_by(3);
        let out = reg.render_prometheus();
        assert!(out.contains("# HELP roko_plans_total plans by status\n"));
        assert!(out.contains("# TYPE roko_plans_total counter\n"));
        assert!(out.contains("roko_plans_total 3\n"));
    }

    #[test]
    fn registry_multiple_label_sets_render_as_multiple_lines() {
        let reg = MetricRegistry::new();
        let a = reg.register_counter(
            "roko_plans_total",
            "plans",
            LabelSet::from_pairs(&[("status", "succeeded")]),
        );
        let b = reg.register_counter(
            "roko_plans_total",
            "plans",
            LabelSet::from_pairs(&[("status", "failed")]),
        );
        a.inc_by(7);
        b.inc_by(2);
        let out = reg.render_prometheus();
        // Exactly one HELP / TYPE per family.
        assert_eq!(out.matches("# HELP roko_plans_total").count(), 1);
        assert_eq!(out.matches("# TYPE roko_plans_total").count(), 1);
        assert!(out.contains("roko_plans_total{status=\"succeeded\"} 7\n"));
        assert!(out.contains("roko_plans_total{status=\"failed\"} 2\n"));
    }

    #[test]
    fn registry_histogram_renders_with_buckets_sum_count() {
        let reg = MetricRegistry::new();
        let h = reg.register_histogram(
            "roko_latency_seconds",
            "latency",
            LabelSet::new(),
            vec![0.5, 1.0],
        );
        h.observe(0.25);
        h.observe(0.75);
        h.observe(3.0);
        let out = reg.render_prometheus();
        assert!(out.contains("# TYPE roko_latency_seconds histogram\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"0.5\"} 1\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"1\"} 2\n"));
        assert!(out.contains("roko_latency_seconds_bucket{le=\"+Inf\"} 3\n"));
        assert!(out.contains("roko_latency_seconds_count 3\n"));
        assert!(out.contains("roko_latency_seconds_sum "));
    }

    #[test]
    fn registry_gauge_render() {
        let reg = MetricRegistry::new();
        let g = reg.register_gauge(
            "roko_in_flight",
            "tasks in flight",
            LabelSet::from_pairs(&[("role", "coder")]),
        );
        g.set(4);
        g.sub(1);
        let out = reg.render_prometheus();
        assert!(out.contains("# TYPE roko_in_flight gauge\n"));
        assert!(out.contains("roko_in_flight{role=\"coder\"} 3\n"));
    }

    #[test]
    fn standard_metrics_all_prefixed_roko() {
        for descriptor in STANDARD_METRICS {
            assert!(
                descriptor.name.starts_with("roko_"),
                "expected roko_ prefix, got {}",
                descriptor.name
            );
        }
        assert_eq!(STANDARD_METRICS.len(), 13);
    }

    #[test]
    fn register_standard_metrics_populates_registry() {
        let reg = MetricRegistry::new();
        register_standard_metrics(&reg);
        assert_eq!(reg.family_count(), STANDARD_METRICS.len());
        // spot-check lookups
        assert!(
            reg.get_counter(schema::ROKO_PLANS_TOTAL, &LabelSet::new())
                .is_some()
        );
        assert!(
            reg.get_histogram(schema::ROKO_AGENT_DURATION_SECONDS, &LabelSet::new())
                .is_some()
        );
    }

    #[test]
    fn snapshot_json_roundtrip() {
        let reg = MetricRegistry::new();
        let c = reg.register_counter("c_total", "count", LabelSet::new());
        c.inc_by(5);
        let snap = reg.snapshot();
        let json = serde_json::to_string(&snap).expect("serialise snapshot");
        assert!(json.contains("c_total"));
        let parsed: Vec<MetricSnapshot> =
            serde_json::from_str(&json).expect("deserialise snapshot");
        assert_eq!(parsed.len(), 1);
        match &parsed[0].value {
            MetricValue::Counter(v) => assert_eq!(*v, 5),
            _ => panic!("expected counter"),
        }
    }

    #[test]
    fn registry_is_thread_safe_for_registration() {
        let reg = StdArc::new(MetricRegistry::new());
        let handles: Vec<_> = (0u64..4)
            .map(|i| {
                let reg = StdArc::clone(&reg);
                std::thread::spawn(move || {
                    let labels = LabelSet::from_pairs(&[("status", "ok")]);
                    let c = reg.register_counter("x_total", "x", labels);
                    c.inc_by(i + 1);
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread joined");
        }
        // All 4 threads share one counter: 1+2+3+4 = 10
        let c = reg
            .get_counter("x_total", &LabelSet::from_pairs(&[("status", "ok")]))
            .expect("counter registered by threads");
        assert_eq!(c.get(), 10);
    }

    #[test]
    fn render_prometheus_appends_registered_metrics() {
        let reg = MetricRegistry::new();
        let labels = LabelSet::from_pairs(&[("gate", "compile"), ("verdict", "pass")]);
        let counter = reg.register_counter("roko_gate_verdicts_total", "Total gate verdicts", labels);
        counter.inc_by(3);

        let output = reg.render_prometheus();
        assert!(
            output.contains("roko_gate_verdicts_total"),
            "rendered output should contain the registered metric: {output}"
        );
        assert!(
            output.contains("gate=\"compile\""),
            "rendered output should contain gate label: {output}"
        );
        assert!(
            output.contains("verdict=\"pass\""),
            "rendered output should contain verdict label: {output}"
        );
        assert!(
            output.contains(" 3"),
            "counter value should be 3: {output}"
        );
    }
}
