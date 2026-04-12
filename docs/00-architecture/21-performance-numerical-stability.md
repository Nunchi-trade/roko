# Performance and numerical stability

> Cross-cutting -- All Layers
> Status: **Specification** -- guidelines for implementation
> Canonical source: various crates (see per-section references)

---

## Purpose

Roko performs numerical computation throughout: decay curves, scoring, HDC similarity, bandit arm selection, EMA threshold adaptation, cost normalization. This document specifies the time/space complexity of each algorithm, precision targets, normalization rules, and strategies for handling NaN/Inf/underflow.

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

Rule of thumb: use f32 for per-signal fields (stored millions of times), f64 for aggregate statistics (stored once per subsystem).

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
| `Scorer::score(signal)` | O(k) | O(1) | k = number of scoring rules |
| Batch score all signals | O(n * k) | O(n) | n signals, k rules per signal |

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

The routing reward signal is a weighted sum of three components, each normalized to [0.0, 1.0]:

```
quality = gate_pass_rate            (already in [0, 1])
cost    = 1.0 - (actual / budget)   (inverted: cheaper = higher)
latency = 1.0 - (actual / sla)     (inverted: faster = higher)

reward = quality_weight * quality + cost_weight * cost + latency_weight * latency
```

Clamped to [0.0, 1.0] after computation.

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

This underflows to 0.0, which is the correct behavior -- a fully decayed signal has zero weight.

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

### 5.2 Memory budgets

| Component | Target | Notes |
|---|---|---|
| Signal (in memory) | < 1 KB each | Body is the variable part |
| HDC vector | 1.25 KB | 10,240 bits = 1,280 bytes |
| Knowledge entry | < 2 KB | HDC vector + metadata |
| Episode record | < 4 KB | Compressed JSON |
| Config (loaded) | < 64 KB | All structs combined |
| Cascade router state | < 256 KB | Arm parameters for all models |

### 5.3 Disk budgets

| File | Growth rate | Target max | Rotation |
|---|---|---|---|
| `signals.jsonl` | ~1 KB/signal | 100 MB | Prune by decay threshold |
| `episodes.jsonl` | ~2 KB/episode | 50 MB | Retain `episode_retention_days` |
| `efficiency.jsonl` | ~500 B/event | 20 MB | Monthly rotation |
| `cascade-router.json` | Rewritten on update | < 1 MB | Single file, overwritten |
| `gate-thresholds.json` | Rewritten on update | < 10 KB | Single file, overwritten |
| `experiments.json` | Grows with experiments | < 5 MB | Archive concluded experiments |

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

---

## Cross-references

- [04-decay-variants.md](04-decay-variants.md) -- Decay math
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) -- Score normalization
- [../05-learning/03-bandits-ucb-thompson-linucb.md](../05-learning/03-bandits-ucb-thompson-linucb.md) -- Bandit precision
- [../05-learning/04-cascade-router.md](../05-learning/04-cascade-router.md) -- Router complexity
- [../05-learning/08-cost-normalization.md](../05-learning/08-cost-normalization.md) -- Cost precision
- `crates/roko-core/src/decay.rs` -- Decay implementation
- `crates/roko-core/src/signal.rs` -- Signal with Score field
