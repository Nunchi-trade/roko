# 06 — Adaptive Gate Thresholds

> **Layer**: L3 Harness — Verification
> **Crate**: `roko-gate` (`crates/roko-gate/src/adaptive_threshold.rs`)
> **Status**: Implemented (215 lines), persists to `.roko/learn/gate-thresholds.json`


> **Implementation**: Shipping

---

## 1. Overview

Adaptive thresholds tune verification behavior based on historical pass rates. They use
exponential moving averages (EMA) per gate rung to track how often each rung passes, and
from that, derive two advisory signals:

1. **Retry budget**: How many retries should a rung get? High pass rate → fewer retries.
   Low pass rate → more retries.
2. **Skip advisory**: Should a rung be skipped? If it has passed 20+ times consecutively,
   it's probably always passing and could be skipped to save time.

Both signals are advisory — the orchestrator may override them. But they provide a
data-driven default that adapts to the project's actual verification characteristics.

> **Citation**: crates/roko-gate/src/adaptive_threshold.rs — Full implementation.

---

## 2. Per-Rung Statistics

```rust
pub struct RungStats {
    pub ema_pass_rate: f64,       // Exponential moving average of pass rate [0.0, 1.0]
    pub total_observations: u64,  // Total gate runs for this rung
    pub consecutive_passes: u32,  // Consecutive passes (reset on any failure)
}
```

Each rung gets its own `RungStats`. A fresh rung starts with:
- `ema_pass_rate = 0.5` (neutral prior — no assumption about pass/fail tendency)
- `total_observations = 0`
- `consecutive_passes = 0`

---

## 3. The EMA Algorithm

### 3.1 Update Rule

```rust
pub fn update(&mut self, rung: u32, passed: bool) {
    let stats = self.rungs.entry(rung).or_default();
    let value = if passed { 1.0 } else { 0.0 };

    if stats.total_observations == 0 {
        stats.ema_pass_rate = value;  // First observation sets the rate directly
    } else {
        stats.ema_pass_rate = EMA_ALPHA.mul_add(value, (1.0 - EMA_ALPHA) * stats.ema_pass_rate);
    }

    stats.total_observations += 1;

    if passed {
        stats.consecutive_passes += 1;
    } else {
        stats.consecutive_passes = 0;
    }
}
```

### 3.2 Why EMA?

An exponential moving average with α = 0.1 means:
- Recent observations weigh more than old ones
- The effective memory is ~1/α ≈ 10 observations
- Gradual changes in pass rate are tracked smoothly

This is important because gate pass rates change over time:
- A new project with many issues has low pass rates initially
- As issues are fixed, pass rates climb
- A major refactor temporarily drops pass rates before they recover

A simple average (total passes / total observations) would be slow to respond to these
shifts. The EMA adapts within ~10 observations.

### 3.3 The α Parameter

`EMA_ALPHA = 0.1` is the decay constant. Higher α means more responsive (recent data
dominates), lower α means more stable (historical data dominates).

| α | Effective window | Behavior |
|---|---|---|
| 0.01 | ~100 observations | Very stable, slow to adapt |
| 0.1 | ~10 observations | Balanced (current default) |
| 0.3 | ~3 observations | Responsive, potentially noisy |

The choice of 0.1 balances responsiveness with stability. A gate that fails once
shouldn't immediately triple the retry budget, but a gate that fails 5 times in a row
should.

> **Citation**: bardo-backup/prd/16-testing/07-fast-feedback-loops.md — Fast feedback
> loops using EMA-based calibration.

---

## 4. Retry Budget Suggestion

```rust
pub fn suggested_max_retries(&self, rung: u32) -> u32 {
    let Some(stats) = self.rungs.get(&rung) else {
        return 3; // Default for unknown rungs
    };

    if stats.total_observations < 5 {
        return 3; // Not enough data
    }

    // Map pass rate to retries: high pass → low retries, low pass → high retries
    let retries = stats.ema_pass_rate.mul_add(-range, max).round() as u32;
    retries.clamp(MIN_RETRIES, MAX_RETRIES)
}
```

The mapping is linear:
- Pass rate 1.0 → 1 retry (it almost always passes; one attempt is enough)
- Pass rate 0.5 → 3 retries (coin flip; give it a few tries)
- Pass rate 0.0 → 5 retries (it almost never passes; maximize attempts)

### Constants

| Constant | Value | Purpose |
|---|---|---|
| `MIN_RETRIES` | 1 | Floor: always try at least once |
| `MAX_RETRIES` | 5 | Ceiling: don't waste resources endlessly |

### Cold Start

For unknown rungs or rungs with fewer than 5 observations, the default is 3 retries.
This avoids extreme behavior early in a project's lifecycle.

---

## 5. Skip Advisory

```rust
pub fn should_skip_rung(&self, rung: u32) -> bool {
    self.rungs
        .get(&rung)
        .is_some_and(|s| s.consecutive_passes >= SKIP_STREAK_THRESHOLD)
}
```

If a rung has passed `SKIP_STREAK_THRESHOLD` (20) consecutive times, the system
suggests it can be skipped. This advisory is not enforced — the orchestrator decides
whether to honor it.

### Why 20?

Twenty consecutive passes means the gate hasn't failed in at least 20 task executions.
For a gate like Compile, this suggests the project's code generation is reliable enough
that compile failures are rare. Running the gate still costs time (seconds to minutes),
and if the system has high confidence the gate will pass, skipping it saves that time.

### Why Advisory Only?

Even a gate with 100 consecutive passes can fail unexpectedly (new dependency breaks,
CI environment changes, agent produces unusually complex code). Making the skip advisory
rather than mandatory means:
- The orchestrator can skip the gate 90% of the time for speed
- Every Nth run (e.g., every 5th), it still runs the gate to check
- A failure resets the consecutive pass counter, re-enabling the gate

This "mostly skip, periodically verify" pattern is common in testing infrastructure
(see: test quarantine systems, flaky test skip-and-retry).

> **Citation**: bardo-backup/prd/16-testing/09-evaluation-map.md — 14 feedback loops
> across 5 speed tiers, including machine-speed confidence calibration.

---

## 6. Persistence

```rust
pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
    let json = serde_json::to_string_pretty(self)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn load_or_new(path: &Path) -> Self {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}
```

### Atomic Write

The `save()` method uses the atomic write pattern: write to a temporary file, then
rename. This ensures the file is never in a half-written state. If the process crashes
between `write` and `rename`, the old file remains intact.

### Graceful Degradation

`load_or_new()` returns a fresh `AdaptiveThresholds` if the file is missing or corrupt.
This means the system always starts correctly — it just loses its historical data on
corruption.

### Storage Location

The thresholds persist to `.roko/learn/gate-thresholds.json`. This is the learning
subsystem's data directory, alongside:
- `.roko/learn/cascade-router.json` (model routing state)
- `.roko/learn/experiments.json` (prompt experiment state)
- `.roko/learn/efficiency.jsonl` (per-turn efficiency events)

> **Citation**: CLAUDE.md — "Adaptive gate thresholds: EMA per rung in
> `.roko/learn/gate-thresholds.json`."

---

## 7. Interaction with Other Components

### 7.1 Rung Selector

The rung selector (see [02-6-rung-selector.md](./02-6-rung-selector.md)) determines
which rungs to run based on static complexity. The adaptive thresholds refine this:

```
Static selection: complexity → rungs [0, 1, 2, 3]
Adaptive refinement:
  Rung 0: 25 consecutive passes → skip advisory
  Rung 1: 3 consecutive passes → run normally
  Rung 2: 12 consecutive passes → run normally
  Rung 3: not tracked yet → run with default retries
Final: rungs [1, 2, 3] with rung 0 skipped
```

### 7.2 Retry Logic

The orchestrator's retry loop consults `suggested_max_retries()`:

```rust
let max_retries = thresholds.suggested_max_retries(current_rung);
for attempt in 0..max_retries {
    let verdict = pipeline.verify(signal, ctx).await;
    if verdict.passed { break; }
    // ... escalate, adjust prompt, retry
}
```

### 7.3 Gate Pipeline Feedback

After each pipeline execution, the orchestrator updates the thresholds:

```rust
for (rung, verdict) in rung_verdicts {
    thresholds.update(rung as u32, verdict.passed);
}
thresholds.save(&thresholds_path)?;
```

This closes the feedback loop: gate outcomes → EMA update → retry budget adjustment →
different gate behavior on next execution.

---

## 8. Reporting

```rust
pub fn rung_stats(&self, rung: u32) -> Option<&RungStats> {
    self.rungs.get(&rung)
}

pub fn all_rungs(&self) -> impl Iterator<Item = (&u32, &RungStats)> {
    self.rungs.iter()
}
```

These methods enable the dashboard and status commands to display per-rung health:

```
Gate Thresholds:
  Rung 0 (Compile):  98.2% pass rate, 142 observations, 31 consecutive passes [SKIP ADVISORY]
  Rung 1 (Lint):     87.5% pass rate, 130 observations, 8 consecutive passes
  Rung 2 (Test):     72.1% pass rate, 118 observations, 3 consecutive passes
  Rung 3 (Symbol):   95.0% pass rate, 45 observations, 15 consecutive passes
```

---

## 9. Relationship to the GVU Framework

The adaptive thresholds are a practical implementation of the GVU framework's guidance
on verification investment. The framework proves that stronger verifiers yield better
self-improvement. The thresholds operationalize this by:

1. **Allocating more retries** to gates with low pass rates (investing more in
   verification where it's most needed).
2. **Reducing retries** for gates with high pass rates (not wasting resources on
   verification that's already reliable).
3. **Skipping gates** that are essentially always passing (redirecting verification
   budget to where it matters).

This is adaptive resource allocation for verification — a concrete instance of the GVU
insight that verification quality matters more than generation quality.

> **Citation**: Song et al. (ICLR 2025) — GVU framework, verification-first investment
> strategy.

---

## 10. Testing

| Test | Property |
|---|---|
| `new_rung_starts_neutral` | Unknown rung → default 3 retries, no skip |
| `high_pass_rate_reduces_retries` | ~100% pass rate → 1 retry |
| `low_pass_rate_increases_retries` | ~0% pass rate → 5 retries |
| `consecutive_passes_trigger_skip` | 20 consecutive → skip advisory |
| `failure_resets_skip_streak` | One failure → no skip advisory |
| `round_trip_persistence` | Save/load preserves state |

> **Citation**: crates/roko-gate/src/adaptive_threshold.rs — Tests section.
