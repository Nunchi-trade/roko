# Anomaly Detection and Learning Integration

> The Conductor does not exist in isolation. It feeds the learning
> system and is fed by it. Interventions produce data. Data produces
> better interventions.


> **Implementation**: Built

---

## AnomalyDetector

The anomaly detector (`roko-learn/src/anomaly.rs`) provides
statistical anomaly detection complementing the Conductor's
threshold-based watchers:

```rust
pub struct AnomalyDetector {
    prompt_hash_window: VecDeque<u64>,   // last 20 prompt hashes
    cost_ewma: EwmaState,                // exponentially weighted moving average
    quality_history: VecDeque<f64>,      // last 50 quality scores
    session_cost_usd: f64,               // accumulated session cost
    session_start_ms: i64,               // session start time
}

pub enum Anomaly {
    PromptLoop { repeated_count: usize },
    CostSpike { z_score: f64 },
    QualityDegradation { avg_drop: f64 },
    BudgetExhausted { used: f64, limit: f64 },
}
```

### Prompt Loop Detection

Hashes each prompt and tracks in a sliding window of 20. Five
identical hashes trigger `Anomaly::PromptLoop`:

```rust
pub fn check_prompt(&mut self, prompt_hash: u64) -> Option<Anomaly> {
    self.prompt_hash_window.push_back(prompt_hash);
    if self.prompt_hash_window.len() > 20 {
        self.prompt_hash_window.pop_front();
    }

    let count = self.prompt_hash_window.iter()
        .filter(|&&h| h == prompt_hash)
        .count();

    if count >= 5 {
        Some(Anomaly::PromptLoop { repeated_count: count })
    } else {
        None
    }
}
```

This catches a broader class of loops than the stuck-pattern watcher.
The watcher looks at agent output; the anomaly detector looks at
agent input. If the system is sending the same prompt five times, the
agent will produce the same output five times — detecting the loop at
the input level catches it earlier.

### Cost Spike Detection

Uses EWMA (Exponentially Weighted Moving Average) with z-score
anomaly detection:

```rust
impl EwmaState {
    pub fn update(&mut self, value: f64) {
        let diff = value - self.mean;
        self.mean += self.alpha * diff;
        self.variance = (1.0 - self.alpha) * (self.variance + self.alpha * diff * diff);
    }

    pub fn z_score(&self, value: f64) -> f64 {
        let stddev = self.variance.sqrt();
        if stddev < 1e-10 { return 0.0; }
        (value - self.mean) / stddev
    }
}
```

A z-score above 3.0 triggers `Anomaly::CostSpike`. This means the
cost of the current turn is more than 3 standard deviations above
the running average — a sudden 10x cost increase, for example.

### Quality Degradation Detection

Compares recent quality scores (last 5) against earlier scores
(turns 11-20). If the recent average drops more than 0.15 below
the earlier average AND the recent average is below 0.5, the system
is degrading:

```rust
if recent_avg < earlier_avg - 0.15 && recent_avg < 0.5 {
    Some(Anomaly::QualityDegradation { avg_drop: earlier_avg - recent_avg })
}
```

The dual condition prevents false positives: a quality drop from 0.95
to 0.80 does not trigger (recent is still above 0.5), but a drop from
0.7 to 0.4 does (recent is below 0.5 AND the delta exceeds 0.15).

---

## Conductor ↔ Learning System Integration

### Interventions as Learning Signals

Every conductor intervention produces data for the learning system:

```
Conductor fires "compile-fail-repeat" for plan-42
    │
    ▼
AgentEfficiencyEvent {
    agent_id: "agent-7",
    model: "claude-sonnet-4-6",
    outcome: "conductor_intervention",
    gate_errors: [{ category: "TypeMismatch", count: 3 }],
    // ...
}
    │
    ▼
Cascade Router records negative observation:
    model="claude-sonnet-4-6", context=(complex_task, auth_module), reward=low
    │
    ▼
Next similar task routed to claude-opus-4-6 instead
```

The intervention is not just a corrective action — it is a data point.
The learning system uses it to improve future routing decisions.

### Efficiency Events

Every agent turn records an `AgentEfficiencyEvent` with 20+ fields:

```rust
pub struct AgentEfficiencyEvent {
    pub agent_id: String,
    pub role: String,
    pub backend: String,
    pub model: String,
    pub plan_id: String,
    pub task_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub cost_usd_without_cache: f64,
    pub prompt_sections: Vec<String>,
    pub total_prompt_tokens: u64,
    pub system_prompt_tokens: u64,
    pub tools_available: usize,
    pub tools_used: usize,
    pub tool_calls: Vec<String>,
    pub wall_time_ms: u64,
    pub duration_ms: u64,
    pub time_to_first_token_ms: u64,
    pub was_warm_start: bool,
    pub iteration: u32,
    pub gate_passed: bool,
    pub outcome: String,
    pub gate_errors: Vec<String>,
    pub model_used: String,
    pub frequency: OperatingFrequency,
    pub strategy_attempted: String,
    pub timestamp: String,
}
```

The context window pressure watcher reads these events directly:
```rust
if let Ok(event) = signal.body.as_json::<AgentEfficiencyEvent>() {
    if let Some(total) = context_window_tokens(&event.model) {
        return Some((event.total_prompt_tokens as f64, total as f64));
    }
}
```

This shared data format means context pressure monitoring and
efficiency tracking use the same instrumentation — no additional
data collection needed.

### Cascade Router Feedback

The cascade router learns which model-task combinations produce good
outcomes. Conductor interventions are negative outcomes:

| Conductor Event | Router Signal |
|----------------|--------------|
| Continue | Positive (task progressing normally) |
| Restart (compile-fail-repeat) | Negative (model failed on compile errors) |
| Restart (stuck-pattern) | Negative (model got stuck) |
| Restart (ghost-turn) | Strongly negative (model produced nothing) |
| Fail (iteration-loop) | Strongly negative (model did not converge) |

Over time, the router accumulates enough data to route tasks away
from model-context combinations that historically trigger conductor
interventions:

```
Context: { complexity: Complex, category: Auth, file_count: 5 }
    │
    ▼
Router observations:
    claude-sonnet-4-6 + this context → 3 interventions in 5 attempts
    claude-opus-4-6   + this context → 0 interventions in 3 attempts
    │
    ▼
Router routes to opus for this context
```

### Adaptive Gate Thresholds

The adaptive gate threshold system (`roko-gate/src/adaptive_threshold.rs`)
adjusts gate pass criteria based on historical data using exponential
moving averages per gate rung.

The Conductor's interventions provide indirect input to this system:
plans that trigger conductor restarts and then pass gates on the
second attempt produce different gate threshold data than plans that
pass on the first attempt. This difference helps the adaptive threshold
system calibrate its expectations.

---

## Feedback Loops

### Loop 1: Intervention → Routing Improvement

```
Agent fails → Conductor intervenes → Negative routing signal →
Router adjusts model selection → Future agents less likely to fail →
Fewer interventions needed
```

This is a negative feedback loop: interventions produce data that
reduces future interventions. Over many batch runs, the intervention
rate should decrease as the router learns better model-task mappings.

### Loop 2: Threshold → Efficiency Data → Threshold Tuning

```
Threshold fires → Intervention occurs → Efficiency event records
outcome → Threshold effectiveness measured → Threshold adjusted
```

This loop requires the adaptive conductor model (described in
08-good-regulator-self-model.md). Currently, thresholds are static.
The learning infrastructure exists to close this loop.

### Loop 3: Error Classification → Auto-Fix → Pattern Library

```
Error occurs → Diagnosis engine classifies → Auto-fix attempted →
If auto-fix succeeds → Pattern stored in diagnosis engine with
higher confidence → Future similar errors auto-fixed faster
```

Each successful auto-fix strengthens the diagnosis engine's confidence
in that error-to-fix mapping. Over time, the auto-fix success rate
for known error patterns approaches the theoretical maximum.

### Loop 4: Quality Degradation → Model Escalation → Quality Data

```
Quality drops → Anomaly detector fires → Escalate to higher-tier
model → Higher-tier model produces better quality → Quality data
recorded → Router learns tier requirements for this task type
```

The quality degradation detector triggers escalation. The escalated
model's performance provides data for future routing. Eventually, the
router learns to assign the correct tier initially, eliminating the
need for runtime escalation.

---

## Anomaly Detection in the Dispatch Pipeline

The anomaly detector integrates into the agent dispatch pipeline
(before each agent turn):

```rust
// Before each agent turn:
if let Some(anomaly) = anomaly_detector.check_prompt(prompt_hash) {
    match anomaly {
        Anomaly::PromptLoop { .. } => {
            // Abort session — sending the same prompt will produce the same failure
            return Err(DispatchError::PromptLoop);
        }
        _ => {}
    }
}

if let Some(anomaly) = anomaly_detector.check_cost(turn_cost_usd) {
    match anomaly {
        Anomaly::CostSpike { z_score } => {
            // Log warning, consider model downgrade
            tracing::warn!("cost spike z={z_score:.1}, considering model downgrade");
        }
        _ => {}
    }
}

if let Some(anomaly) = anomaly_detector.check_budget(budget_limit_usd) {
    match anomaly {
        Anomaly::BudgetExhausted { used, limit } => {
            // Abort session — budget exceeded
            return Err(DispatchError::BudgetExhausted { used, limit });
        }
        _ => {}
    }
}
```

The anomaly detector runs BEFORE the agent turn, catching problems
at input time rather than output time. This is the "anticipate, don't
react" principle (Design Principle 11) applied to agent dispatch.

---

## Provider Health Integration

The provider health tracker (`roko-learn/src/provider_health.rs`)
provides a separate feedback loop for infrastructure-level anomalies:

```
Provider returns 429 → Health tracker records failure →
3 consecutive failures → Circuit breaker opens →
Router filters out unhealthy provider → Requests routed to
healthy providers → After cooldown → Probe request sent →
If probe succeeds → Circuit breaker closes
```

This is independent of the Conductor's plan-level circuit breaker.
The provider health breaker operates on API call outcomes. The
Conductor's breaker operates on plan-level outcomes. They complement
each other:

| Breaker | Level | What Triggers It | What It Blocks |
|---------|-------|-----------------|----------------|
| Provider health | API call | 3 consecutive provider errors | Requests to that provider |
| Conductor | Plan | 2 plan failures | Retries of that plan |

---

## File Reference

| File | What |
|------|------|
| `crates/roko-learn/src/anomaly.rs` | AnomalyDetector, EWMA, prompt loop, cost spike, quality degradation |
| `crates/roko-learn/src/efficiency.rs` | AgentEfficiencyEvent (shared data format) |
| `crates/roko-learn/src/cascade_router.rs` | Cascade router (consumes intervention signals) |
| `crates/roko-learn/src/provider_health.rs` | Provider health tracker (infrastructure breaker) |
| `crates/roko-gate/src/adaptive_threshold.rs` | Adaptive gate thresholds (EMA per rung) |
| `crates/roko-conductor/src/watchers/context_window_pressure.rs` | Reads AgentEfficiencyEvent for token tracking |
