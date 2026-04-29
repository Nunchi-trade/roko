# 28. Funding, Budgets, and Operator Model

> Budgets as a Verify Cell + Loop that constrains agent behavior economically. Multi-level guardrails with graceful degradation. Operator freedom hierarchy (5 levels). Hot-reload config as Trigger Cell. Budget pressure composes with tier routing.

See [02-CELL.md](../../unified/02-CELL.md) for Verify protocol, [03-GRAPH.md](../../unified/03-GRAPH.md) for Loop pattern, [19-CONFIG.md](../../unified/19-CONFIG.md) for config-as-Signal.

---

## 1. Budgets Are Verify Cells

A budget is not just a configuration value. It is a Verify Cell that sits in the gamma flow's Pipeline and can **veto** actions that would exceed cost constraints. The Verify protocol's `verify_pre()` method is the enforcement point:

```rust
/// BudgetGuard: Verify Cell that enforces spending limits.
///
/// Runs in the gamma flow Pipeline between COMPOSE and ACT.
/// Can reject an action before it executes (pre-verification).
/// Can track costs after execution (post-verification).
///
/// Crate: `crates/roko-runtime/src/budget.rs`
pub struct BudgetGuard {
    daily_limit: f64,
    lifetime_limit: Option<f64>,
    per_turn_token_limit: u32,
    tracker: BudgetTracker,
    degradation: Option<DegradationStage>,
}

impl Verify for BudgetGuard {
    fn verify_pre(&self, signal: &Signal) -> Verdict {
        let estimated_cost = signal.field::<f64>("estimated_cost_usd");
        let tier = signal.field::<InferenceTier>("selected_tier");

        // Check per-turn token limit
        let tokens = signal.field::<u32>("context_tokens");
        if tokens > self.per_turn_token_limit {
            return Verdict::reject("Exceeds per-turn token limit");
        }

        // Check if this tier is allowed at current degradation
        if let Some(stage) = &self.degradation {
            match (stage, tier) {
                (DegradationStage::MonitoringOnly, _) => {
                    return Verdict::reject("Budget: monitoring only mode");
                }
                (DegradationStage::ReducedFrequency, InferenceTier::T2) => {
                    return Verdict::reject("Budget: T2 disabled at 90%+");
                }
                (DegradationStage::T0Emphasis, InferenceTier::T2) => {
                    return Verdict::reject("Budget: T2 disabled at 80%+");
                }
                _ => {} // allowed
            }
        }

        // Check daily limit
        if self.tracker.daily_spend() + estimated_cost > self.daily_limit {
            return Verdict::reject("Would exceed daily budget");
        }

        Verdict::pass()
    }

    fn verify_post(&mut self, signal: &Signal) -> Verdict {
        // Record actual cost after execution
        let actual_cost = signal.field::<f64>("actual_cost_usd");
        self.tracker.record(actual_cost);

        // Check if we crossed a degradation threshold
        let fraction = self.tracker.daily_spend() / self.daily_limit;
        self.update_degradation(fraction);

        Verdict::pass()
    }
}
```

---

## 2. Multi-Level Guardrails

Budget enforcement operates at four levels, from most granular to most aggregate:

### Level 1: Per-Turn Token Limit

Enforced by the Compose Cell before context assembly. Default: 8192 tokens. The VCG auction (see [24-attention-auction-and-cortical-state.md](24-attention-auction-and-cortical-state.md)) allocates within this budget.

### Level 2: Per-Hour Inference Rate

A sliding-window rate limiter prevents burst spending:

```rust
/// Sliding-window hourly rate limiter.
/// Max 20% of daily budget in any single hour.
pub struct HourlyRateLimiter {
    window: VecDeque<(Instant, f64)>,  // (timestamp, cost)
    hourly_limit: f64,                  // daily_limit * 0.20
}

impl HourlyRateLimiter {
    pub fn would_exceed(&self, cost: f64) -> bool {
        let hour_ago = Instant::now() - Duration::from_secs(3600);
        let recent_spend: f64 = self.window.iter()
            .filter(|(t, _)| *t > hour_ago)
            .map(|(_, c)| c)
            .sum();
        recent_spend + cost > self.hourly_limit
    }
}
```

### Level 3: Daily Budget Ceiling

The hard daily cap (`max_daily_inference_usd`). When reached, the degradation cascade activates.

### Level 4: Lifetime Budget Cap

Optional absolute spending limit. When reached: permanent pause until operator intervenes.

---

## 3. Graceful Degradation Cascade

When budget constraints are hit, the Agent degrades through five stages. Each stage is a more restrictive policy applied to the HeartbeatPolicy and Router:

```rust
/// Degradation stages with concrete effects on the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationStage {
    /// At 70% daily budget: use cheaper models
    ModelDowngrade,
    /// At 80%: maximize T0 suppression
    T0Emphasis,
    /// At 90%: reduce tick frequency 4x
    ReducedFrequency,
    /// At 95%: no actions, only observe
    MonitoringOnly,
    /// At 100%: cognitive loop paused
    BudgetPaused,
}
```

### 3.1 Stage Effects

| Stage | Trigger | Clock Effect | Tier Effect | Action Effect |
|---|---|---|---|---|
| ModelDowngrade | 70% | None | T2 uses Sonnet instead of Opus | Full |
| T0Emphasis | 80% | Theta 2x | T2 disabled except Crisis | Full |
| ReducedFrequency | 90% | All intervals 4x | T0 and T1 only | Full |
| MonitoringOnly | 95% | Gamma at max | T0 only | No actions taken |
| BudgetPaused | 100% | All paused | None | None |

### 3.2 The Degradation Loop

Budget degradation is a Loop: spending triggers degradation, which reduces spending, which eventually recovers the budget (daily reset at midnight UTC):

```
High spending rate
  |
  v
Cross threshold (e.g., 80%)
  |
  v
Apply DegradationStage::T0Emphasis
  |
  v
Spending rate drops (T2 disabled, most ticks free)
  |
  v
Daily budget resets at midnight UTC
  |
  v
Degradation lifted, normal operation resumes
```

### 3.3 Composition with Tier Routing

Budget pressure directly affects the EFE computation (see [25-active-inference-state-space.md](25-active-inference-state-space.md)):

```rust
/// Budget-modulated cost term in EFE.
/// Higher budget pressure -> higher cost penalty -> favor cheaper tiers.
fn budget_adjusted_cost(base_cost: f64, budget_fraction: f64) -> f64 {
    let pressure = if budget_fraction < 0.7 {
        1.0  // no pressure
    } else {
        // Exponential pressure above 70%
        1.0 + ((budget_fraction - 0.7) / 0.3).powi(2) * 10.0
    };
    base_cost * pressure
}

// At 70% budget: T2 cost = $0.10 * 1.0 = $0.10
// At 80% budget: T2 cost = $0.10 * 2.11 = $0.21 (effectively doubles)
// At 90% budget: T2 cost = $0.10 * 5.44 = $0.54 (effectively 5x)
// At 95% budget: T2 cost = $0.10 * 7.94 = $0.79 (massive penalty)
```

This means EFE naturally avoids expensive tiers under budget pressure -- no separate throttling logic needed.

---

## 4. Operator Freedom Hierarchy

The operator has five levels of control, each more disruptive than the last:

### Level 1: Steer (Non-Disruptive)

Edit `STRATEGY.md`. The Agent picks up the new strategy on its next theta tick. No interruption.

```
Operator edits STRATEGY.md
  |
  v
Trigger Cell (inotify/kqueue watch) fires
  |
  v
Strategy reload Pulse published on Bus
  |
  v
Theta flow picks up new strategy on next tick
  |
  v
Agent adapts behavior over next 1-5 ticks
```

### Level 2: Constrain (Bounded Disruption)

Modify hot-reloadable `roko.toml` sections. Takes effect immediately:

```toml
# Hot-reloadable sections:
[inference]       # Model changes: next turn
[budget]          # Limit changes: immediate
[heartbeat]       # Interval changes: next tick
[tools]           # Profile changes: next tool load
```

### Level 3: Pause (Reversible)

```bash
roko agent stop --name X    # Suspend cognitive loop
roko agent start --name X   # Resume from exact state
```

All Hot Flows pause. State preserved in memory. Bus messages buffered.

### Level 4: Restart (State-Preserving)

```bash
roko agent stop --name X && roko agent start --name X
```

Process stops. Neuro and episode state preserved on disk. Agent resumes from last persisted state.

### Level 5: Delete (Irreversible)

```bash
roko delete --backup    # Back up, then terminate
```

The deletion Pipeline runs (see [26-agent-lifecycle-type-state.md](26-agent-lifecycle-type-state.md)). Process terminated. Knowledge preserved in backup.

### 4.1 Operator Levels as Capability Space

Each level maps to a capability on the Agent's Space:

```rust
/// Operator capability levels.
/// Expressed as a bitmask on the Agent's Space access control.
pub struct OperatorCapabilities {
    pub steer: bool,           // L1: edit strategy
    pub constrain: bool,       // L2: edit hot-reload config
    pub pause_resume: bool,    // L3: suspend/resume
    pub restart: bool,         // L4: stop/start
    pub delete: bool,          // L5: terminate
}

// Default operator has all capabilities
impl Default for OperatorCapabilities {
    fn default() -> Self {
        Self { steer: true, constrain: true, pause_resume: true,
               restart: true, delete: true }
    }
}
```

---

## 5. Hot-Reload Config as Trigger Cell

Configuration reload is implemented as a Trigger Cell that watches the filesystem and fires Graph reloads:

```rust
/// ConfigWatcher: Trigger Cell that watches roko.toml and STRATEGY.md.
///
/// Uses inotify (Linux) / kqueue (macOS) for instant detection.
/// On change: validates new config, publishes reload Pulse on Bus.
///
/// Crate: `crates/roko-runtime/src/config_watch.rs`
pub struct ConfigWatcher {
    watcher: RecommendedWatcher,
    watched_paths: Vec<PathBuf>,
}

impl Trigger for ConfigWatcher {
    fn events(&self) -> Vec<EventPattern> {
        vec![
            EventPattern::FileModified("roko.toml"),
            EventPattern::FileModified("STRATEGY.md"),
        ]
    }

    fn fire(&self, event: &Event, bus: &Bus) {
        match event.path.file_name().and_then(|f| f.to_str()) {
            Some("roko.toml") => {
                if let Ok(new_config) = validate_config(&event.path) {
                    bus.publish(Pulse::new("config.toml.reloaded")
                        .with_payload(new_config));
                }
            }
            Some("STRATEGY.md") => {
                if let Ok(strategy) = read_to_string(&event.path) {
                    bus.publish(Pulse::new("config.strategy.reloaded")
                        .with_payload(strategy));
                }
            }
            _ => {}
        }
    }
}
```

### 5.1 Config Validation Before Apply

Changes are validated before application. Invalid configs are rejected with a Pulse on `config.validation.failed`:

```rust
/// Validate proposed config change against safety constraints.
pub fn validate_config(path: &Path) -> Result<ValidatedConfig, ConfigError> {
    let proposed = parse_toml(path)?;

    // Schema validation
    validate_schema(&proposed)?;

    // Semantic validation
    if proposed.budget.max_daily_inference_usd < 0.0 {
        return Err(ConfigError::InvalidValue("budget cannot be negative"));
    }
    if proposed.clock.gamma_min_interval_secs < 1 {
        return Err(ConfigError::InvalidValue("gamma min must be >= 1s"));
    }

    Ok(ValidatedConfig(proposed))
}
```

---

## 6. Four Funding Sources

For Agents that need to sustain themselves economically:

### 6.1 Direct Credit (Operator Funds)

The operator provides a budget via `roko.toml`. The Agent draws from this pool.

### 6.2 x402 Micropayments

Per-request payments via the x402 protocol (Coinbase/Linux Foundation). Each payment extends compute budget:

```rust
/// x402 payment received: extend budget.
pub fn on_x402_payment(&mut self, amount_usd: f64) {
    self.tracker.extend_budget(amount_usd);
    self.bus.publish(Pulse::new("budget.extended")
        .with_field("amount_usd", amount_usd)
        .with_field("source", "x402"));
}
```

### 6.3 Self-Funding Loop

An Agent that earns revenue can fund its own operation:

```
Agent earns revenue (trading profits, service fees, LP fees)
  |
  v
Revenue deposited to agent budget
  |
  v
Agent allocates portion to compute budget
  |
  v
Budget extends -> Agent continues operating
```

Sustainability formula: `daily_cost < daily_revenue * safety_margin`

### 6.4 External Funding (Permissionless)

Anyone can extend any Agent's budget by sending a payment. This enables community-funded Agents.

---

## 7. Cost Tracking and Efficiency Metrics

The BudgetTracker records every expenditure and computes efficiency metrics:

```rust
/// BudgetTracker: real-time cost tracking with daily/lifetime aggregation.
///
/// Writes to CorticalState (budget_fraction, resource_health).
/// Persists to `.roko/learn/efficiency.jsonl`.
pub struct BudgetTracker {
    daily_spend: f64,
    lifetime_spend: f64,
    daily_limit: f64,
    turn_records: Vec<TurnCostRecord>,
}

impl BudgetTracker {
    pub fn daily_fraction(&self) -> f64 {
        self.daily_spend / self.daily_limit
    }

    pub fn efficiency_metrics(&self) -> EfficiencyMetrics {
        let turns = &self.turn_records;
        let t0_rate = turns.iter().filter(|t| t.t0_suppressed).count() as f64
            / turns.len().max(1) as f64;
        let cost_per_gate_pass = self.daily_spend
            / turns.iter().filter(|t| t.gate_passed).count().max(1) as f64;

        EfficiencyMetrics {
            t0_suppression_rate: t0_rate,     // target: > 0.80
            cost_per_gate_pass,               // target: < $0.01
            daily_spend: self.daily_spend,
            projected_monthly: self.daily_spend * 30.0,
        }
    }
}
```

### 7.1 Efficiency Targets

| Metric | Target | What It Means |
|---|---|---|
| T0 suppression rate | > 80% | Most ticks cost nothing |
| Cost per gate pass | < $0.01 | Cheap successful outcomes |
| Model distribution | > 60% Haiku | Mostly cheap models |
| Cache hit rate | > 30% | Reusing computed context |
| Token efficiency | > 0.4 | Output tokens / total tokens |

These metrics feed into the CascadeRouter's cost-quality tradeoff learning.

---

## 8. TOML Budget Configuration

```toml
[budget]
max_daily_inference_usd = 10.0
# max_total_usd = 1000.0        # optional lifetime cap
max_tokens_per_turn = 8192
degradation = "cascade"          # "cascade" | "pause" | "notify-only"

[budget.thresholds]
model_downgrade_at = 0.70
t0_emphasis_at = 0.80
reduced_frequency_at = 0.90
monitoring_only_at = 0.95

[budget.hourly]
max_hourly_fraction = 0.20       # max 20% of daily in any hour
```

---

## What This Enables

- **Economic sustainability**: Budget guardrails prevent runaway costs while preserving Agent autonomy.
- **Graceful degradation**: The Agent dims progressively under budget pressure -- it never goes blind or dies from cost.
- **Composable constraints**: Budget pressure flows through the same EFE mechanism that drives tier routing. No special-case logic.
- **Operator control without proletarianization**: The operator sets bounds (budget, strategy); the Agent develops competence within those bounds.
- **Self-funding viability**: Agents that earn revenue can sustain themselves indefinitely.
- **Hot-reload without restart**: Most configuration changes take effect immediately via Bus Pulses.

## Feedback Loops

1. **Spending -> degradation -> reduced spending -> recovery -> degradation lifted** (the primary budget Loop).
2. **Budget pressure -> EFE cost penalty -> favor cheaper tiers -> lower cost per tick -> budget extends** (integrated via active inference).
3. **Efficiency metrics -> CascadeRouter learning -> better model selection -> lower cost per gate pass** (cross-system Loop).
4. **Config change -> validate -> reload Pulse -> runtime adapts -> operator observes effect -> further tuning** (operator-in-the-loop).

## Open Questions

1. Should the degradation thresholds be learnable (agents that consistently hit 80% might benefit from earlier degradation)?
2. How should multi-Agent budgets work (shared pool across a Group)?
3. Should the self-funding loop have a minimum reserve (never spend more than 80% of revenue)?
4. What is the correct interaction between operator-set limits and EFE-computed cost sensitivity?

## Implementation Tasks

| Task | File Path | Status |
|---|---|---|
| Define `BudgetGuard` Verify Cell | `crates/roko-runtime/src/budget.rs` | Not started |
| Implement `BudgetTracker` with daily/lifetime | `crates/roko-runtime/src/budget.rs` | Partial (efficiency events exist) |
| Implement degradation cascade | `crates/roko-runtime/src/budget.rs` | Not started |
| Wire BudgetGuard into gamma flow Pipeline | `crates/roko-cli/src/orchestrate.rs` | Partial (budget exists in config) |
| Implement `ConfigWatcher` Trigger Cell | `crates/roko-runtime/src/config_watch.rs` | Partial (TUI fs_watch exists) |
| Implement hourly rate limiter | `crates/roko-runtime/src/budget.rs` | Not started |
| Wire budget_fraction into CorticalState | `crates/roko-core/src/cortical.rs` | Not started |
| Integrate budget pressure with EFE cost term | `crates/roko-learn/src/active_inference/efe.rs` | Not started |
| Persist efficiency metrics to .roko/learn/ | `crates/roko-learn/src/efficiency.rs` | Done |
