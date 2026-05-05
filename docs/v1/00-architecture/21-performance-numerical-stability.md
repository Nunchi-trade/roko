# Performance and numerical stability

> Cross-cutting -- All Layers
> Status: **Specification** -- guidelines for implementation
> Canonical source: various crates (see per-section references)
> Refinement alignment: telemetry and observability budgets in this chapter track [tmp/refinements/33-observability-telemetry.md](../../tmp/refinements/33-observability-telemetry.md).

> **Implementation**: Specified

---

## Purpose

Roko performs numerical computation throughout: decay curves, scoring, HDC similarity, bandit arm selection, EMA threshold adaptation, cost normalization, and observability aggregation. This document specifies the time/space complexity of each algorithm, precision targets, normalization rules, instrumentation overhead budgets, and strategies for handling NaN/Inf/underflow without making the telemetry surface itself distort the system being measured.

Terminology in this chapter follows the current kernel vocabulary in [01-naming-and-glossary.md](01-naming-and-glossary.md), especially `Engram`, `Pulse`, `Bus`, and `Topic`.

---

## 1. Precision targets

### 1.1 f32 vs f64 decision matrix

| Domain | Type | Rationale |
|---|---|---|
| Decay weights | f32 | Range [0.0, 1.0], 7 decimal digits sufficient |
| Score axes | f32 | Range [-1.0, 1.0], 7 decimal digits sufficient |
| PAD vector | f32 | Range [-1.0, 1.0], psychometric precision |
| HDC vectors | u64 bitfield | Binary (0/1 per dimension); no floating point |
| Cost tracking | f64 | USD amounts accumulate; f32 loses cents past $16,777 |
| Bandit arm parameters | f64 | UCB/Thompson precision matters for convergence |
| EMA thresholds | f64 | Small alpha values (0.05) compound rounding errors in f32 |
| Timestamps | i64 | Millisecond Unix timestamps; u64 would also work |
| Token counts | usize | Platform-native integer; no floating point |
| Metric counters | u64 | Monotonic counts should not round through floating-point |
| Histogram bucket bounds | f64 | Stable bucket boundaries must survive repeated serialization |
| Trace/log queue watermarks | usize | Queue depth is an integer occupancy, not an estimate |

Rule of thumb: use f32 for per-Engram or per-Pulse fields stored at high volume, f64 for aggregate statistics and telemetry values that accumulate over time, and integers for counters, token totals, timestamps, and queue depths.

### 1.2 Serialization precision

When serializing f32/f64 to JSON:

```rust
// Avoid: serde default produces 0.30000001192092896 for 0.3_f32
// Use: round to meaningful precision before serialization
fn round_f32(v: f32, decimals: u32) -> f32 {
    let factor = 10_f32.powi(decimals as i32);
    (v * factor).round() / factor
}
```

Target precision by domain:

| Domain | Decimal places | Example |
|---|---|---|
| Decay weight | 6 | 0.707107 |
| Score axis | 4 | 0.8500 |
| Cost (USD) | 4 | 12.3456 |
| EMA threshold | 6 | 0.654321 |
| Exploration rate | 4 | 0.1000 |
| c-factor / calibration gauges | 4 | 0.8125 |
| Demurrage ratio | 6 | 0.002500 |

### 1.3 Telemetry precision and exposition rules

Observability is part of the runtime budget, not an afterthought. The numbers emitted by logs, metrics, traces, replay, and StateHub projections need stable units and bounded precision so operators do not confuse rendering artifacts for system behavior.

| Surface | Internal representation | Exposed representation | Precision rule |
|---|---|---|---|
| Monotonic counters | `u64` | integer sample | Never round through `f32`; reset only on process restart |
| Duration and latency histograms | `u64` ns or ms samples, `f64` bucket bounds | Prometheus/OpenTelemetry seconds | Convert units once at exposition boundaries; keep fixed bucket families |
| Cost metrics | `f64` USD | decimal | Keep 4 decimal places in logs and projections; aggregate from raw token pricing |
| Calibration and c-factor gauges | `f64` | decimal | Round to 4 decimals for presentation, keep full precision in memory |
| Demurrage and decay ratios | `f64` | decimal | Round to 6 decimals for presentation |
| Trace timestamps | `i64` or `u64` monotonic clock samples | RFC3339 plus monotonic delta | Wall clock is for display; ordering and latency derive from monotonic time |

Telemetry-specific rules:

1. Percentile dashboards derive from histograms or StateHub projections, not from ad hoc rounded gauges.
2. Histogram bucket boundaries are fixed at startup; do not synthesize buckets from live data.
3. Durations in Prometheus-compatible metrics are exposed in seconds even if the runtime stores them in milliseconds or nanoseconds.

---

## 2. Algorithm complexity

### 2.1 Decay

| Operation | Time | Space | Notes |
|---|---|---|---|
| `Decay::apply(age_ms)` | O(1) | O(1) | Single `powf` or `exp` call |
| `Decay::is_alive(age_ms, threshold)` | O(1) | O(1) | Comparison after `apply` |
| `Substrate::prune(threshold)` | O(n) | O(1) | Linear scan, in-place removal |

### 2.2 Scoring

| Operation | Time | Space | Notes |
|---|---|---|---|
| `Score::effective()` | O(1) | O(1) | Weighted sum of 7 axes |
| `Scorer::score(datum)` | O(k) | O(1) | k = number of scoring rules |
| Batch score all data | O(n * k) | O(n) | n = Engrams or Pulses, k rules per item |

### 2.3 HDC vectors

| Operation | Time | Space | Notes |
|---|---|---|---|
| Bind (XOR) | O(d/64) | O(d/64) | d = dimensionality (10,240 bits = 160 u64s) |
| Bundle (majority) | O(d * m) | O(d/64) | m = number of vectors to bundle |
| Hamming distance | O(d/64) | O(1) | `popcnt` on XOR result |
| Nearest neighbor (brute) | O(n * d/64) | O(1) | n = number of stored vectors |

For d = 10,240 and n = 10,000 knowledge entries: nearest-neighbor search takes ~10,000 * 160 = 1.6M `popcnt` operations. On a modern CPU (~10 billion ops/sec), this completes in ~0.16ms. No index structure is needed below 100,000 entries.

### 2.4 Bandit selection

| Operation | Time | Space | Notes |
|---|---|---|---|
| UCB1 select | O(a) | O(a) | a = number of arms |
| Thompson sampling (Beta) | O(a) | O(a) | One Beta sample per arm |
| LinUCB select | O(a * d^2) | O(a * d^2) | d = context feature dimension |
| Bandit update | O(d^2) | O(1) | Matrix update for LinUCB |

For LinUCB with a = 10 arms and d = 8 features: select takes ~640 multiply-adds. Negligible.

### 2.5 EMA threshold adaptation

| Operation | Time | Space | Notes |
|---|---|---|---|
| EMA update | O(1) | O(1) | `new = alpha * sample + (1 - alpha) * old` |
| Read current threshold | O(1) | O(1) | Single field access |
| Persist to JSON | O(r) | O(r) | r = number of rungs (6) |

### 2.6 Cascade router

| Operation | Time | Space | Notes |
|---|---|---|---|
| Candidate scoring | O(c) | O(c) | c = candidate models |
| Static table lookup | O(1) | O(r * k) | r = roles, k = complexity bands |
| Bandit arm selection | O(c) | O(c) | Falls through to bandit |
| Full route decision | O(c) | O(c) | Dominated by candidate scoring |

### 2.7 Prompt assembly

| Operation | Time | Space | Notes |
|---|---|---|---|
| Section collection | O(s) | O(s) | s = number of sections (~9) |
| Priority sort | O(s log s) | O(1) | In-place sort |
| Budget allocation | O(s) | O(s) | Single pass after sort |
| Token counting | O(t) | O(1) | t = total tokens in content |
| Full assembly | O(s log s + t) | O(s + t) | Dominated by token counting |

### 2.8 Observability and replay

| Operation | Time | Space | Notes |
|---|---|---|---|
| Counter increment | O(1) | O(1) | Atomic add or thread-local batch |
| Histogram observe | O(b) | O(1) | b = bucket search cost; keep small fixed families |
| Span start + finish | O(a) | O(a) | a = span attributes copied into the sink envelope |
| Structured log enqueue | O(1) amortized | O(1) | Formatting happens off the hot path when possible |
| Replay reconstruction | O(e + p) | O(e + p) | e = Engrams, p = Pulses in the retained episode window |

Instrumentation must remain cheaper than the operator it measures. Log and trace emission therefore enqueue into bounded buffers and return quickly; exporter I/O, formatting, and downstream network stalls are not allowed to block step execution on the seven-step loop.

---

## 3. Normalization rules

### 3.1 Score normalization

All seven Score axes are in [-1.0, 1.0]. The `effective()` method produces a weighted sum:

```rust
pub fn effective(&self) -> f32 {
    const W: [f32; 7] = [0.25, 0.20, 0.15, 0.15, 0.10, 0.10, 0.05];
    let axes = [
        self.relevance, self.confidence, self.urgency,
        self.novelty, self.salience, self.coherence, self.surprise,
    ];
    axes.iter().zip(W.iter()).map(|(a, w)| a * w).sum::<f32>().clamp(-1.0, 1.0)
}
```

The weights sum to 1.0, so `effective()` is also in [-1.0, 1.0].

### 3.2 Cost normalization

Costs are normalized to USD. Provider-specific token pricing is converted at query time:

```
normalized_cost_usd = input_tokens * price_per_input_token
                    + output_tokens * price_per_output_token
```

Prices are stored as f64 in units of dollars per token (not per million tokens) to avoid off-by-1000 errors.

### 3.3 Latency normalization

Latencies are stored in milliseconds (u64). EWMA is computed in f64:

```
ewma_ms = alpha * sample_ms + (1.0 - alpha) * ewma_ms
```

Default alpha = 0.1 (recent samples weighted at 10%).

### 3.4 Reward normalization

The routing reward value is a weighted sum of three components, each normalized to [0.0, 1.0]:

```
quality = gate_pass_rate            (already in [0, 1])
cost    = 1.0 - (actual / budget)   (inverted: cheaper = higher)
latency = 1.0 - (actual / sla)     (inverted: faster = higher)

reward = quality_weight * quality + cost_weight * cost + latency_weight * latency
```

Clamped to [0.0, 1.0] after computation.

### 3.5 Telemetry normalization and label discipline

Observability numbers are only comparable when units and labels remain stable across subsystems.

| Concern | Rule |
|---|---|
| Duration units | Store runtime samples in ns or ms, expose metrics in seconds, and label projections with explicit units |
| Counters | Monotonic only; retries and replays must not silently double-count historical work |
| Histograms | Use fixed bucket families; do not create per-tenant or per-model bucket layouts at runtime |
| Labels | Keep labels low-cardinality: `gate`, `topic`, `kind`, `role`, `status`, `cohort`, `budget_scope` are acceptable |
| High-cardinality identifiers | `content_hash`, `engram_hash`, `trace_id`, `session_id`, `agent_id`, `principal_id`, and raw prompts belong in logs, traces, or replay, not metric labels |
| Series budget | Target fewer than 10,000 active series per process and fewer than 100 live values per label dimension in steady state |

Cardinality is a performance constraint, not just an operations concern. A metric that explodes label space can dominate memory, scrape time, and query latency faster than the operator path it was supposed to illuminate.

---

## 4. NaN/Inf/underflow handling

### 4.1 Sources of NaN

| Operation | NaN source | Mitigation |
|---|---|---|
| `0.0 / 0.0` | Division by zero | Check denominator before division |
| `(-1.0_f32).sqrt()` | Negative sqrt | Never occurs in this codebase (all inputs are non-negative) |
| `f32::INFINITY - f32::INFINITY` | Indeterminate | Avoid unbounded accumulation |
| `exp(large_positive)` | Overflow to Inf, then Inf * 0 = NaN | Clamp exponent input |

### 4.2 Sources of Inf

| Operation | Inf source | Mitigation |
|---|---|---|
| `exp(710.0_f64)` | f64 overflow | Clamp input to `exp()` at 700.0 |
| `1.0 / 0.0` | Division by zero | Check denominator; return default |
| `powf(0.5, 0.0 / 0.0)` | NaN propagation from half_life_ms=0 | Guard: `if half_life_ms == 0 { return 0.0; }` |

### 4.3 Underflow

Ebbinghaus decay with small strength and large age produces values below f32 epsilon:

```
exp(-age / (0.01 * scale)) where age >> scale
```

This underflows to 0.0, which is the correct behavior -- a fully decayed Engram has zero weight.

### 4.4 Defensive pattern

Every function that produces a float should use this pattern:

```rust
fn safe_compute(input: f32) -> f32 {
    let result = /* computation */;
    if result.is_nan() || result.is_infinite() {
        // Log the anomaly for debugging
        tracing::warn!(input, result = %result, "numerical anomaly, clamping");
        return DEFAULT_VALUE;
    }
    result.clamp(MIN, MAX)
}
```

Apply this at computation boundaries, not at every intermediate step. Clamping intermediate results can mask bugs.

### 4.5 Telemetry failure modes

| Failure mode | Cause | Mitigation |
|---|---|---|
| Negative rate after restart | Counter reset interpreted as a true decrease | Compute rates over reset-aware counters and annotate process restarts |
| Nonsensical latency histograms | Mixed ms/seconds units across emitters | Convert units once in the sink and test bucket families end to end |
| Cardinality explosion | Unbounded identifiers used as labels | Reject or hash identifiers into logs only; keep metrics on bounded enums |
| Sample bias from sink saturation | Log or trace queues drop under pressure | Expose drop counters and queue depth so operators can discount biased windows |
| False precision in replay | Episode reconstructed from partial Pulse history | Mark measurements as `exact`, `reconstructed`, or `partial` before using them for regression or audit |

---

## 5. Profiling targets

### 5.1 Hot paths

| Path | Budget | Measurement |
|---|---|---|
| `Decay::apply()` | < 10ns | Single function, inline candidate |
| `Score::effective()` | < 50ns | Weighted sum of 7 floats |
| HDC Hamming distance | < 1us | 160 `popcnt` operations |
| Prompt assembly | < 5ms | Token counting dominates |
| Gate execution (compile) | < 60s | External subprocess |
| Gate execution (test) | < 300s | External subprocess |
| Cascade router select | < 100us | Candidate scoring + bandit |
| Episode log write | < 1ms | JSONL append |
| Metric counter increment | < 250ns | No heap allocation on the hot path |
| Histogram observation | < 750ns | Fixed bucket family, thread-local fast path preferred |
| Trace span start + finish | < 10us | Attribute copy included, exporter excluded |
| Structured log enqueue | < 50us | JSON serialization may spill to background worker |
| StateHub telemetry fold | < 1ms per delta | Projection update must not stall Bus consumers |
| Replay scan throughput | > 50,000 records/s | Enough for postmortem and test replay without distorting wall-clock budgets |

Instrumentation budget rule: end-to-end observability overhead should stay below 5% CPU on Gamma-speed interactive turns and below 10% on Theta/Delta batch work, excluding explicitly enabled debug profiling.

### 5.2 Memory budgets

| Component | Target | Notes |
|---|---|---|
| Engram or Pulse header (in memory) | < 1 KB each | Body is the variable part |
| HDC vector | 1.25 KB | 10,240 bits = 1,280 bytes |
| Knowledge entry | < 2 KB | HDC vector + metadata |
| Episode record | < 4 KB | Compressed JSON |
| Config (loaded) | < 64 KB | All structs combined |
| Cascade router state | < 256 KB | Arm parameters for all models |
| Metrics registry working set | < 16 MB/process | Includes active series, bucket state, and registry metadata |
| Log queue | 8-32 MB/process | Bounded; drop low-priority lines before unbounded growth |
| Trace export queue | 8-32 MB/process | Bounded; sustained overflow degrades readiness |
| Replay index cache | < 64 MB/process | Enough to seek episode windows without full scans |

### 5.3 Disk budgets

| File | Growth rate | Target max | Rotation |
|---|---|---|---|
| `engrams.jsonl` | ~1 KB/Engram | 100 MB | Prune by demurrage / decay policy |
| `episodes.jsonl` | ~2 KB/episode | 50 MB | Retain `episode_retention_days` |
| `efficiency.jsonl` | ~500 B/telemetry event | 20 MB | Monthly rotation |
| `cascade-router.json` | Rewritten on update | < 1 MB | Single file, overwritten |
| `gate-thresholds.json` | Rewritten on update | < 10 KB | Single file, overwritten |
| `experiments.json` | Grows with experiments | < 5 MB | Archive concluded experiments |
| `telemetry.log.jsonl` | 1-10 MB/hour | 7 days local, ship downstream | Rotate by size and age |
| `telemetry-spans.otlp` | exporter-dependent | 24 hours local spool max | Best-effort spill only; do not treat as source of truth |
| `replay-index.json` | ~100 B/episode | < 50 MB | Rebuildable from durable episode material |

Retention changes how measurements should be interpreted:

1. Metrics and traces describe the retained window of the downstream stack, not the full lifetime of the deployment.
2. Replay-derived performance analysis must record whether the episode is `exact`, `reconstructed`, or `partial`.
3. `partial` replay windows are valid for qualitative debugging, but they should not feed benchmark baselines, regression gates, or accounting totals.

---

## 6. Benchmark suite

The following benchmarks should run on every CI build:

```rust
#[bench]
fn bench_decay_apply_halflife(b: &mut Bencher) {
    let d = Decay::HalfLife { half_life_ms: 86_400_000 };
    b.iter(|| d.apply(black_box(3_600_000)));
}

#[bench]
fn bench_decay_apply_ebbinghaus(b: &mut Bencher) {
    let d = Decay::Ebbinghaus { strength: 1.0, scale_ms: 86_400_000 };
    b.iter(|| d.apply(black_box(3_600_000)));
}

#[bench]
fn bench_score_effective(b: &mut Bencher) {
    let s = Score { relevance: 0.8, confidence: 0.9, urgency: 0.5,
                    novelty: 0.3, salience: 0.6, coherence: 0.7, surprise: 0.2 };
    b.iter(|| black_box(&s).effective());
}

#[bench]
fn bench_hdc_hamming_10240(b: &mut Bencher) {
    let a = vec![0u64; 160]; // 10,240 bits
    let c = vec![u64::MAX; 160];
    b.iter(|| hamming_distance(black_box(&a), black_box(&c)));
}
```

Add a second benchmark group for observability paths: counter increment throughput, histogram observation cost, span allocation cost, structured log enqueue latency, queue saturation behavior, and replay scan throughput over representative episode sizes.

---

## 7. Error handling

| Condition | Response |
|---|---|
| NaN in Score axis | Clamp to 0.0, log warning |
| Inf in decay weight | Clamp to 1.0, log warning |
| Negative cost | Clamp to 0.0 (negative costs are accounting errors) |
| Latency EWMA overflow | Reset EWMA to current sample |
| HDC vector dimension mismatch | Return max distance (vectors are maximally dissimilar) |
| Token count overflow (usize) | Saturating add; log warning |
| Metric label over cardinality budget | Drop the sample, increment a self-observability counter, log at warn |
| Log queue saturation | Drop `debug`/`info` first, preserve `warn`/`error`, emit dropped-line counter |
| Trace exporter backlog | Bound the queue, drop spans with accounting, mark `/readyz` degraded after threshold |
| Replay window incomplete | Return `partial` measurement status instead of exact aggregates |
| Metrics scrape too slow | Emit scrape-duration metric and treat sustained overruns as an observability fault |

---

## 8. Test criteria

1. `Decay::apply()` produces results within 1e-6 of the mathematical formula for all variants.
2. `Score::effective()` with all axes at 1.0 returns 1.0; all axes at -1.0 returns -1.0.
3. No function in the codebase produces NaN or Inf when given valid inputs (property test with proptest).
4. `Decay::apply()` with `half_life_ms = 0` returns 0.0 (not NaN or Inf).
5. `Decay::Ebbinghaus` with `strength = 0.0` returns 0.0 (not NaN).
6. Cost normalization of 0 input tokens and 0 output tokens returns 0.0 USD.
7. EMA update with alpha = 0.0 returns old value; alpha = 1.0 returns new sample.
8. HDC Hamming distance of identical vectors is 0; of complementary vectors is dimensionality.
9. Benchmark regression: all hot-path benchmarks within 2x of baseline.
10. Metrics expose stable base units: seconds for durations, USD for cost, integers for counters and queue depths.
11. Histogram bucket families are fixed and monotonic; no runtime label value can create new buckets.
12. Metric label validation rejects high-cardinality identifiers such as `trace_id` or `engram_hash`.
13. Log and trace queues remain bounded under sink failure and emit self-observability counters for drops and backlog.
14. Replay-based measurements mark `exact`, `reconstructed`, or `partial` provenance and exclude `partial` windows from regression baselines by default.
15. `/readyz` degrades when telemetry sinks are unavailable beyond threshold, and the degradation is itself observable via metrics and logs.

---

## Cross-References

- [04-decay-variants.md](04-decay-variants.md) -- Decay math
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) -- Score normalization
- [01-naming-and-glossary.md](01-naming-and-glossary.md) -- Current architecture vocabulary for Engram, Pulse, Bus, Topic, replay, and telemetry terms
- [../05-learning/03-bandits-ucb-thompson-linucb.md](../05-learning/03-bandits-ucb-thompson-linucb.md) -- Bandit precision
- [../05-learning/04-cascade-router.md](../05-learning/04-cascade-router.md) -- Router complexity
- [../05-learning/08-cost-normalization.md](../05-learning/08-cost-normalization.md) -- Cost precision
- [../../tmp/refinements/33-observability-telemetry.md](../../tmp/refinements/33-observability-telemetry.md) -- Canonical observability and telemetry refinement propagated into this chapter
- `crates/roko-core/src/decay.rs` -- Decay implementation
- `crates/roko-core/src/engram.rs` -- Engram structure and durable metadata
- `crates/roko-core/src/score.rs` -- Score representation
- `crates/roko-core/src/obs/metrics.rs` -- Metrics sink implementation
