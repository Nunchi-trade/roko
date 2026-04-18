//! `TaskMetric` — the raw data foundation for Roko's continuous-tuning loops.
//!
//! Every gate verdict produces one [`TaskMetric`] record. Records are appended
//! to `.roko/runs/{run_id}/metrics.jsonl` by the persistence layer
//! (`roko-fs::metrics::MetricsLog`) and consumed by the five optimization
//! loops described in `tmp/roko-progress/roko-continuous-tuning.md`.
//!
//! The join key across every record is [`ConfigHash`]: a 16-char BLAKE3
//! prefix of the run's canonical `RokoConfig`. All analysis slices by
//! `config_hash` — that is the primitive operation that makes all five
//! loops work.
//!
//! # Discipline
//!
//! Three rules for this layer, from §1 of `roko-continuous-tuning.md`:
//!
//! 1. **Ship this first.** No loop is worth anything without clean metric
//!    data. Get `TaskMetric` emitting on every gate before starting any
//!    optimizer.
//! 2. **Record 20+ plans before drawing conclusions.** Then compute the
//!    four headline metrics via [`compute_headlines`]. That is your
//!    baseline.
//! 3. **Never mutate records.** Metrics are immutable; `.jsonl` is
//!    append-only. Bad data stays and is filtered, never rewritten.

use serde::{Deserialize, Serialize};

/// 16-char BLAKE3 prefix over a canonical-JSON-serialized config.
///
/// Stable across machines (no timestamps, no process-specific state in
/// the hash input). This is the **join key** across every `TaskMetric`
/// record — all analysis compares configs by their hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConfigHash(pub String);

impl ConfigHash {
    /// Compute the hash of any serializable value.
    ///
    /// The input is first serialized to **canonical JSON** (sorted keys,
    /// no whitespace variability), then BLAKE3-hashed, then the first
    /// 8 bytes are hex-encoded giving a 16-char string.
    ///
    /// # Errors
    ///
    /// Returns an error if the value cannot be serialized to JSON. In
    /// practice this only happens for types with custom serializers
    /// that fail — plain config structs never fail.
    pub fn of<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        // Round-trip through serde_json::Value to get deterministic key
        // ordering. serde_json::to_string alone preserves struct-field
        // order, but fails for HashMap<String, _>. Going through Value
        // and then re-serializing with sort_keys gives a canonical form.
        let v = serde_json::to_value(value)?;
        let canonical = canonical_json(&v);
        let digest = blake3::hash(canonical.as_bytes());
        let bytes = digest.as_bytes();
        let hex = hex_prefix(&bytes[..8]);
        Ok(Self(hex))
    }

    /// Borrow the inner 16-char hex string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConfigHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ConfigHash {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Render bytes as a hex-encoded string.
fn hex_prefix(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Serialize a `serde_json::Value` with sorted object keys.
///
/// `serde_json` preserves insertion order for `Map<String, Value>`
/// (since it uses a `BTreeMap` by default — wait, it uses `BTreeMap`
/// ONLY with the `preserve_order` feature **off**). We guarantee sort
/// order regardless by walking the value and rebuilding with a sorted
/// `BTreeMap`.
fn canonical_json(v: &serde_json::Value) -> String {
    use serde_json::Value;
    fn walk(v: &Value, out: &mut String) {
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => {
                // serde_json handles escaping — cheapest to call it.
                out.push_str(&serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()));
            }
            Value::Array(arr) => {
                out.push('[');
                for (i, item) in arr.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    walk(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into()));
                    out.push(':');
                    walk(&map[*k], out);
                }
                out.push('}');
            }
        }
    }
    let mut s = String::new();
    walk(v, &mut s);
    s
}

/// One record per gate execution. Serialized as a line of `JSONL`.
///
/// Mirrors §1.1 of `roko-continuous-tuning.md`. All five tuning loops
/// slice this table by `config_hash` (the join key) + some subset of
/// `role`/`complexity_band`/`gate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskMetric {
    // ── Identity ─────────────────────────────────────────────────
    /// ISO-8601 UTC timestamp when the gate completed.
    pub timestamp: String,
    /// Git SHA of the Roko binary that produced this record.
    pub run_id: String,
    /// The join key — which `RokoConfig` produced this record.
    pub config_hash: ConfigHash,
    /// Plan this task belongs to (e.g. `"46-reputation-engine"`).
    pub plan_id: String,
    /// Task within the plan (e.g. `"t3"`).
    pub task_id: String,
    /// Iteration number (1 = first attempt).
    pub iteration: u32,

    // ── Routing ──────────────────────────────────────────────────
    /// Agent role that produced the code being gated.
    pub role: String,
    /// Backend that ran the agent (`"claude"` / `"codex"` / `"cursor"` / ...).
    pub backend: String,
    /// Fully resolved model slug (e.g. `"claude-sonnet-4-5"`).
    pub model: String,
    /// Complexity band (`"trivial"` | `"simple"` | `"standard"` | `"complex"`).
    pub complexity_band: String,
    /// Gate that produced this verdict (e.g. `"compile"`, `"test"`).
    pub gate: String,

    // ── Outcome ──────────────────────────────────────────────────
    /// Did the gate pass?
    pub gate_passed: bool,
    /// Wall-clock milliseconds for the gate (not agent) run.
    pub wall_time_ms: u64,

    // ── Cost ─────────────────────────────────────────────────────
    /// LLM input tokens for the agent turn(s) this gate verified.
    pub input_tokens: u64,
    /// LLM output tokens.
    pub output_tokens: u64,
    /// Tokens served from the prefix cache (subset of input).
    pub cached_tokens: u64,
    /// Cost in USD for this task's LLM spend.
    pub cost_usd: f64,

    // ── Context shape ────────────────────────────────────────────
    /// Number of sections included in the final prompt.
    pub sections_included: u32,
    /// Number of sections dropped by budget pressure.
    pub sections_dropped: u32,
    /// Total tokens in the final assembled prompt.
    pub context_tokens: u64,
    /// Fraction of input tokens that hit the prefix cache (`cached/input`).
    pub cache_hit_rate: f64,
}

impl TaskMetric {
    /// Construct a minimal record with defaults. Populate fields after.
    #[must_use]
    pub fn new(
        config_hash: ConfigHash,
        plan_id: impl Into<String>,
        task_id: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: String::new(),
            run_id: String::new(),
            config_hash,
            plan_id: plan_id.into(),
            task_id: task_id.into(),
            iteration: 1,
            role: String::new(),
            backend: String::new(),
            model: String::new(),
            complexity_band: String::new(),
            gate: String::new(),
            gate_passed: false,
            wall_time_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cost_usd: 0.0,
            sections_included: 0,
            sections_dropped: 0,
            context_tokens: 0,
            cache_hit_rate: 0.0,
        }
    }

    /// Serialize to a single JSONL line (no trailing newline).
    ///
    /// # Errors
    ///
    /// Returns an error only if a field's custom serializer fails (never
    /// happens in practice for this struct).
    pub fn to_jsonl(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parse a single JSONL line into a `TaskMetric`.
    ///
    /// # Errors
    ///
    /// Returns an error if the line is not valid JSON or lacks required fields.
    pub fn from_jsonl(line: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(line)
    }
}

/// The four headline metrics (§1.1 of `roko-continuous-tuning.md`).
///
/// Computed from a slice of [`TaskMetric`] records. These are the
/// numbers every tuning loop optimizes against — no loop should
/// introduce a fifth objective.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Headlines {
    /// Fraction of first-attempt gate runs that passed (`[0..1]`).
    ///
    /// This is **the** headline number. Improvements here are usually
    /// real; regressions usually cost real money.
    pub first_attempt_pass_rate: f64,
    /// Average number of iterations per plan (max iteration observed).
    ///
    /// Cost multiplier: if a plan needs 3 iterations, the LLM spend is
    /// ≈3× the baseline.
    pub avg_iterations_per_plan: f64,
    /// Average USD spent per plan.
    pub avg_cost_per_plan: f64,
    /// Average input tokens per gate record (proxy for prompt size).
    pub avg_input_tokens_per_spawn: f64,
    /// Number of distinct plans contributing to these numbers.
    pub n_plans: usize,
    /// Number of records (gate executions) contributing.
    pub n_records: usize,
}

/// Compute the four headline metrics over a slice of records.
///
/// Mirrors the Python reference in §1.1:
/// - `first_attempt_pass_rate`: mean of `gate_passed` over records with `iteration == 1`
/// - `avg_iterations_per_plan`: mean of `max(iteration)` per `plan_id`
/// - `avg_cost_per_plan`: mean of `sum(cost_usd)` per `plan_id`
/// - `avg_input_tokens_per_spawn`: mean of `input_tokens` over all records
///
/// Returns a [`Headlines`] with zeros in all fields if `records` is empty.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn compute_headlines(records: &[TaskMetric]) -> Headlines {
    if records.is_empty() {
        return Headlines {
            first_attempt_pass_rate: 0.0,
            avg_iterations_per_plan: 0.0,
            avg_cost_per_plan: 0.0,
            avg_input_tokens_per_spawn: 0.0,
            n_plans: 0,
            n_records: 0,
        };
    }

    // Pass rate over first-attempt records only.
    let first: Vec<&TaskMetric> = records.iter().filter(|r| r.iteration == 1).collect();
    let pass_rate = if first.is_empty() {
        0.0
    } else {
        first.iter().filter(|r| r.gate_passed).count() as f64 / first.len() as f64
    };

    // Group by plan_id.
    let mut plans: std::collections::BTreeMap<&str, PlanAgg> = std::collections::BTreeMap::new();
    for r in records {
        let agg = plans.entry(r.plan_id.as_str()).or_default();
        agg.max_iter = agg.max_iter.max(r.iteration);
        agg.total_cost += r.cost_usd;
    }

    let n_plans = plans.len();
    let avg_iters =
        plans.values().map(|a| f64::from(a.max_iter)).sum::<f64>() / n_plans.max(1) as f64;
    let avg_cost = plans.values().map(|a| a.total_cost).sum::<f64>() / n_plans.max(1) as f64;

    let avg_input =
        records.iter().map(|r| r.input_tokens as f64).sum::<f64>() / records.len() as f64;

    Headlines {
        first_attempt_pass_rate: pass_rate,
        avg_iterations_per_plan: avg_iters,
        avg_cost_per_plan: avg_cost,
        avg_input_tokens_per_spawn: avg_input,
        n_plans,
        n_records: records.len(),
    }
}

#[derive(Default)]
struct PlanAgg {
    max_iter: u32,
    total_cost: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn config_hash_is_16_chars() {
        let h = ConfigHash::of(&"hello").unwrap();
        assert_eq!(h.as_str().len(), 16);
        assert!(h.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn config_hash_is_stable_across_field_order() {
        // Two maps with the same logical content but different insertion
        // order must produce the same hash.
        let mut a: BTreeMap<String, i32> = BTreeMap::new();
        a.insert("foo".into(), 1);
        a.insert("bar".into(), 2);

        let mut b: BTreeMap<String, i32> = BTreeMap::new();
        b.insert("bar".into(), 2);
        b.insert("foo".into(), 1);

        assert_eq!(ConfigHash::of(&a).unwrap(), ConfigHash::of(&b).unwrap());
    }

    #[test]
    fn config_hash_differs_on_content_change() {
        let h1 = ConfigHash::of(&vec![1, 2, 3]).unwrap();
        let h2 = ConfigHash::of(&vec![1, 2, 4]).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn config_hash_nested_object_stable() {
        #[derive(Serialize)]
        struct Cfg {
            a: i32,
            b: BTreeMap<String, f64>,
        }
        let c = Cfg {
            a: 1,
            b: {
                let mut m = BTreeMap::new();
                m.insert("x".into(), 1.5);
                m.insert("y".into(), 2.5);
                m
            },
        };
        // Hash must be stable across calls.
        assert_eq!(ConfigHash::of(&c).unwrap(), ConfigHash::of(&c).unwrap());
    }

    #[test]
    fn task_metric_jsonl_roundtrip() {
        let mut m = TaskMetric::new(ConfigHash::from("abcd1234".to_string()), "plan-46", "t3");
        m.iteration = 2;
        m.gate_passed = true;
        m.input_tokens = 1500;
        m.cost_usd = 0.042;
        let line = m.to_jsonl().unwrap();
        let parsed = TaskMetric::from_jsonl(&line).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn headlines_empty_records_returns_zeros() {
        let h = compute_headlines(&[]);
        assert_eq!(h.first_attempt_pass_rate, 0.0);
        assert_eq!(h.avg_iterations_per_plan, 0.0);
        assert_eq!(h.n_plans, 0);
        assert_eq!(h.n_records, 0);
    }

    #[test]
    fn headlines_single_plan_single_iter() {
        let hash = ConfigHash::from("h1".to_string());
        let mut r1 = TaskMetric::new(hash.clone(), "p1", "t1");
        r1.iteration = 1;
        r1.gate_passed = true;
        r1.input_tokens = 1000;
        r1.cost_usd = 0.10;

        let h = compute_headlines(&[r1]);
        assert_eq!(h.first_attempt_pass_rate, 1.0);
        assert_eq!(h.avg_iterations_per_plan, 1.0);
        assert_eq!(h.avg_cost_per_plan, 0.10);
        assert_eq!(h.avg_input_tokens_per_spawn, 1000.0);
        assert_eq!(h.n_plans, 1);
    }

    #[test]
    fn headlines_uses_first_attempt_for_pass_rate() {
        let hash = ConfigHash::from("h".to_string());
        // First attempt fails, second attempt passes — pass_rate should be 0%.
        let mut r1 = TaskMetric::new(hash.clone(), "p1", "t1");
        r1.iteration = 1;
        r1.gate_passed = false;
        let mut r2 = TaskMetric::new(hash.clone(), "p1", "t1");
        r2.iteration = 2;
        r2.gate_passed = true;

        let h = compute_headlines(&[r1, r2]);
        assert_eq!(h.first_attempt_pass_rate, 0.0);
    }

    #[test]
    fn headlines_aggregates_cost_per_plan() {
        let hash = ConfigHash::from("h".to_string());
        // Two tasks in plan A with costs 0.5 + 1.5 = 2.0
        // One task in plan B with cost 1.0
        // avg_cost_per_plan = (2.0 + 1.0) / 2 = 1.5
        let mut r1 = TaskMetric::new(hash.clone(), "A", "t1");
        r1.iteration = 1;
        r1.cost_usd = 0.5;
        let mut r2 = TaskMetric::new(hash.clone(), "A", "t2");
        r2.iteration = 1;
        r2.cost_usd = 1.5;
        let mut r3 = TaskMetric::new(hash.clone(), "B", "t1");
        r3.iteration = 1;
        r3.cost_usd = 1.0;

        let h = compute_headlines(&[r1, r2, r3]);
        assert!((h.avg_cost_per_plan - 1.5).abs() < 1e-9);
        assert_eq!(h.n_plans, 2);
    }

    #[test]
    fn headlines_uses_max_iteration_per_plan() {
        let hash = ConfigHash::from("h".to_string());
        // Plan A: iterations 1, 2, 3 → max=3
        // Plan B: iteration 1 → max=1
        // avg_iterations_per_plan = (3+1)/2 = 2.0
        let mut r1 = TaskMetric::new(hash.clone(), "A", "t1");
        r1.iteration = 1;
        let mut r2 = TaskMetric::new(hash.clone(), "A", "t1");
        r2.iteration = 2;
        let mut r3 = TaskMetric::new(hash.clone(), "A", "t1");
        r3.iteration = 3;
        let mut r4 = TaskMetric::new(hash.clone(), "B", "t1");
        r4.iteration = 1;

        let h = compute_headlines(&[r1, r2, r3, r4]);
        assert!((h.avg_iterations_per_plan - 2.0).abs() < 1e-9);
    }

    #[test]
    fn headlines_pass_rate_across_plans() {
        let hash = ConfigHash::from("h".to_string());
        // 4 first-attempt records: 3 pass, 1 fail → 75%
        let make = |plan: &str, passed: bool| {
            let mut r = TaskMetric::new(hash.clone(), plan, "t1");
            r.iteration = 1;
            r.gate_passed = passed;
            r
        };
        let records = vec![
            make("A", true),
            make("B", true),
            make("C", false),
            make("D", true),
        ];
        let h = compute_headlines(&records);
        assert!((h.first_attempt_pass_rate - 0.75).abs() < 1e-9);
    }

    #[test]
    fn canonical_json_handles_nested_arrays() {
        let v = serde_json::json!({
            "b": [3, 1, 2],
            "a": 1,
        });
        let out = canonical_json(&v);
        // Keys sorted: a before b. Arrays preserve order.
        assert_eq!(out, r#"{"a":1,"b":[3,1,2]}"#);
    }

    #[test]
    fn canonical_json_escapes_strings() {
        let v = serde_json::json!({"s": "a\"b"});
        let out = canonical_json(&v);
        assert_eq!(out, r#"{"s":"a\"b"}"#);
    }
}
