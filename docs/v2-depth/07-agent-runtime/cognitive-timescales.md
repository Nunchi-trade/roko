# Cognitive Timescales

> Depth for [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md). Redesigns three-cognitive-speeds and dual-process theory as three concurrent Hot Graphs, derives the T0/T1/T2 cascade from Expected Free Energy, and specifies the 16 T0 probes as concrete Cells.

---

## 1. Three Timescales, Three Hot Graphs

The three cognitive speeds -- gamma (reactive, ~5-15s), theta (reflective, ~75s), delta (consolidation, hours) -- are not scheduling hints. They are three distinct Hot Graphs running concurrently inside a single Agent. Each is a resident Graph that re-fires on its own clock, interpreted by the same Engine that runs task plans. See [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md) for Engine semantics.

```
Agent
  |
  +-- gamma_loop: Hot Graph, adaptive clock (100ms - 500ms base)
  |     fires: every tick
  |     cost: $0 for ~80% of ticks (T0 short-circuit)
  |     role: perception, reflexes, fast action
  |
  +-- theta_loop: Hot Graph, adaptive clock (500ms - 16s base)
  |     fires: every N gamma ticks (adaptive)
  |     cost: T1 or T2 per tick
  |     role: planning, evaluation, strategy adjustment
  |
  +-- delta_loop: Hot Graph, adaptive clock (60s - 600s base)
        fires: on idle or scheduled
        cost: T2 per tick
        role: dream consolidation, knowledge synthesis, pruning
```

### Why three loops instead of one with a scheduler

A single loop with a "which timescale this tick?" decision conflates two concerns: clock frequency and cognitive purpose. Separating them gives:

1. **Independent failure isolation**: A delta dream that crashes does not stop gamma perception.
2. **Independent budget accounting**: Gamma ticks are nearly free; theta and delta draw from different budget pools.
3. **Independent snapshot/resume**: Each loop has its own FlowSnapshot. Resuming gamma does not require replaying delta's dream state.
4. **Regime-independent delta**: The adaptive clock modulates gamma and theta by regime (Calm slows gamma by 4x, Crisis speeds it by 4x). Delta is less affected (0.5x in Crisis, 1.0x elsewhere). Separate loops make this natural.

---

## 2. The EFE Cascade: Cheap Probes First

Expected Free Energy (Friston 2010) provides the principled routing criterion for T0/T1/T2 tier selection. The key insight: uncertainty drives compute allocation. When nothing is surprising, burn zero dollars. When everything is surprising, invoke the most capable model.

### 2.1 EFE Formula

```
EFE(tier) = -epistemic_value(tier) - pragmatic_value(tier) + cost(tier) + regime_penalty(tier)
```

Lower EFE is better. The system selects `argmin(EFE)` across the three tiers.

| Component | What it measures | Effect on tier selection |
|---|---|---|
| `epistemic_value` | Expected information gain from acting at this tier | High -> favor this tier (we learn something) |
| `pragmatic_value` | Expected goal advancement from acting at this tier | High -> favor this tier (we achieve something) |
| `cost` | Resource expenditure at this tier | High -> disfavor this tier (it is expensive) |
| `regime_penalty` | Regime-conditioned adjustment | Crisis: cost weighted higher, epistemic value boosted |

### 2.2 Why EFE Naturally Produces the T0 -> T1 -> T2 Cascade

The cascade emerges from the cost structure:

```
T0 cost = $0.000  (pure Rust, no LLM)
T1 cost = $0.001  (Haiku-class model)
T2 cost = $0.100  (Opus-class model)
```

When nothing is surprising:
- Epistemic value is low across all tiers (nothing to learn).
- Pragmatic value is moderate at T0 (reflex can handle it).
- Cost dominates: T0 wins because $0 < $0.001 < $0.100.

When something is mildly surprising:
- Epistemic value is moderate at T1 (quick analysis reveals what changed).
- Pragmatic value is low at T0 (reflex does not cover this case).
- T1 wins: moderate information gain at moderate cost.

When something is very surprising or high-stakes:
- Epistemic value is high at T2 (deep analysis needed to understand).
- Pragmatic value is high at T2 (only a capable model can solve this).
- Cost is dominated by value: T2 wins despite being 100x more expensive.

```rust
fn select_tier(
    probes: &T0ProbeResults,
    cortical: &CorticalState,
    vitality: f64,
    regime: Regime,
) -> CognitiveTier {
    // T0 evaluation: all probes quiet?
    if probes.all_quiet() {
        return CognitiveTier::T0Reflex;
    }

    // Compute EFE for T1 and T2
    let surprise = probes.max_surprise();
    let stakes = probes.max_stakes();

    let efe_t1 = EFEEvaluation {
        tier: CognitiveTier::T1Deliberate,
        epistemic_value: surprise * 0.6,     // T1 partially resolves uncertainty
        pragmatic_value: (1.0 - stakes) * 0.5, // T1 handles moderate-stakes work
        cost: 0.001 * vitality_cost_multiplier(vitality),
        regime_penalty: regime_penalty(regime, CognitiveTier::T1Deliberate),
    };

    let efe_t2 = EFEEvaluation {
        tier: CognitiveTier::T2Reflective,
        epistemic_value: surprise * 1.0,     // T2 fully resolves uncertainty
        pragmatic_value: stakes * 0.9,       // T2 handles high-stakes work
        cost: 0.100 * vitality_cost_multiplier(vitality),
        regime_penalty: regime_penalty(regime, CognitiveTier::T2Reflective),
    };

    // Select minimum EFE
    if efe_t1.total() < efe_t2.total() {
        CognitiveTier::T1Deliberate
    } else {
        CognitiveTier::T2Reflective
    }
}

/// Vitality-based cost multiplier. Low vitality makes cost weigh more.
fn vitality_cost_multiplier(vitality: f64) -> f64 {
    match vitality {
        v if v > 0.7 => 1.0,    // Thriving: baseline cost
        v if v > 0.4 => 1.2,    // Stable: slight cost pressure
        v if v > 0.2 => 1.8,    // Conservation: strong cost pressure
        v if v > 0.05 => 3.0,   // Declining: severe cost pressure
        _ => f64::INFINITY,      // Terminal: no LLM calls
    }
}

/// Regime-based penalty. Adjusts the EFE landscape per regime.
fn regime_penalty(regime: Regime, tier: CognitiveTier) -> f64 {
    match (regime, tier) {
        // Crisis: penalize expensive tiers heavily, but boost epistemic value
        (Regime::Crisis, CognitiveTier::T2Reflective) => 0.5,
        (Regime::Crisis, CognitiveTier::T1Deliberate) => 0.1,
        // Volatile: boost epistemic value (seek information)
        (Regime::Volatile, _) => -0.2,
        // Calm: penalize less (we have resources to spare)
        (Regime::Calm, _) => -0.1,
        _ => 0.0,
    }
}
```

### 2.3 The Approximate EFE

Computing exact EFE over a full generative model is intractable. Roko approximates it from four signals that are already available on the CorticalState:

1. **Prediction accuracy** from the calibration stream: declining accuracy -> high epistemic value.
2. **Confidence from Score**: low confidence on recent outputs -> high uncertainty.
3. **Novelty from Score**: high novelty in observations -> high epistemic value.
4. **Daimon arousal**: high arousal (the agent is "surprised") -> escalate.

These signals combine without requiring explicit EFE computation. The approximation works because each signal is a partial derivative of the true EFE surface: prediction error approximates epistemic value, confidence approximates pragmatic value certainty, and arousal is a somatic integration of both.

---

## 3. The 16 T0 Probes as Concrete Cells

T0 probes are zero-LLM diagnostic checks that run at gamma frequency. Each probe is a Cell with typed I/O, implementing the Observe protocol. When any probe reports surprise above threshold, the tick escalates to T1 or T2.

### 3.1 Probe Cell Interface

```rust
/// A T0 probe. Checks one aspect of the environment.
/// Returns a ProbeResult with surprise level and optional evidence.
trait T0Probe: Cell {
    /// The surprise threshold above which this probe triggers escalation.
    fn threshold(&self) -> f32;

    /// The stakes level of what this probe checks.
    /// High-stakes probes can trigger direct T2 escalation.
    fn stakes(&self) -> f32;
}

struct ProbeResult {
    /// 0.0 = no change, 1.0 = maximum surprise.
    surprise: f32,

    /// Optional evidence for downstream analysis.
    evidence: Option<Value>,

    /// Whether this probe recommends escalation.
    escalate: bool,
}
```

### 3.2 The 16 Probes

Each probe is a concrete Cell. Here they are with typed I/O.

```rust
// ── Probe 1: config_changed ─────────────────────────────────────
// Input: last_config_hash (from previous tick)
// Output: ProbeResult { surprise: 1.0 if changed, 0.0 if same }
// Cost: O(1) hash comparison
struct ConfigChangedProbe {
    config_path: PathBuf,
    last_hash: AtomicU64,
}

// ── Probe 2: gate_failed_recently ───────────────────────────────
// Input: gate_verdict_stream (from Bus)
// Output: ProbeResult { surprise: 0.8 if recent failure, 0.0 if clean }
// Cost: O(1) counter check
struct GateFailedRecentlyProbe {
    window_ticks: u32,
}

// ── Probe 3: file_modified ──────────────────────────────────────
// Input: watched file paths + last modification timestamps
// Output: ProbeResult { surprise: 0.6 per changed file }
// Cost: O(k) stat calls, k = watched files (typically < 20)
struct FileModifiedProbe {
    watched: Vec<PathBuf>,
    last_mtimes: DashMap<PathBuf, SystemTime>,
}

// ── Probe 4: test_count_delta ───────────────────────────────────
// Input: cached test count from last scan
// Output: ProbeResult { surprise: 0.7 if count changed }
// Cost: O(1) comparison against cached value
struct TestCountDeltaProbe {
    last_count: AtomicUsize,
}

// ── Probe 5: compile_error_new ──────────────────────────────────
// Input: cached compile error count
// Output: ProbeResult { surprise: 1.0 if new errors appeared }
// Cost: O(1) comparison
struct CompileErrorNewProbe {
    last_error_count: AtomicUsize,
}

// ── Probe 6: budget_threshold ───────────────────────────────────
// Input: CorticalState.budget_remaining
// Output: ProbeResult { surprise scales with proximity to threshold }
// Cost: O(1) atomic read
struct BudgetThresholdProbe {
    warn_threshold: f64,
}

// ── Probe 7: confidence_dropping ────────────────────────────────
// Input: confidence history (ring buffer of last N values)
// Output: ProbeResult { surprise: slope magnitude if negative trend }
// Cost: O(N) where N = window size (typically 10)
struct ConfidenceDroppingProbe {
    history: RingBuffer<f64>,
    window: usize,
}

// ── Probe 8: prediction_violation ───────────────────────────────
// Input: prediction/outcome pairs from calibration stream
// Output: ProbeResult { surprise: prediction error magnitude }
// Cost: O(1) per pair
struct PredictionViolationProbe {
    error_threshold: f64,
}

// ── Probe 9: tool_health_degraded ───────────────────────────────
// Input: tool response time EWMAs
// Output: ProbeResult { surprise if any tool's EWMA > 2x baseline }
// Cost: O(t) where t = number of tools (typically < 20)
struct ToolHealthDegradedProbe {
    baselines: DashMap<String, f64>,
    multiplier: f64,
}

// ── Probe 10: pheromone_detected ────────────────────────────────
// Input: pheromone Pulses from Bus since last tick
// Output: ProbeResult { surprise: intensity of strongest pheromone }
// Cost: O(p) where p = new pheromone Pulses (typically 0)
struct PheromoneDetectedProbe;

// ── Probe 11: task_deadline_near ────────────────────────────────
// Input: current task deadline + current time
// Output: ProbeResult { surprise scales with closeness to deadline }
// Cost: O(1) time comparison
struct TaskDeadlineNearProbe {
    urgency_window: Duration,
}

// ── Probe 12: idle_timeout ──────────────────────────────────────
// Input: time since last non-T0 tick
// Output: ProbeResult { escalate to Delta if idle > threshold }
// Cost: O(1) time comparison
struct IdleTimeoutProbe {
    threshold: Duration,
}

// ── Probe 13: knowledge_stale ───────────────────────────────────
// Input: freshness timestamps on key Memory entries
// Output: ProbeResult { surprise if key knowledge past freshness window }
// Cost: O(k) where k = tracked knowledge entries (typically < 50)
struct KnowledgeStaleProbe {
    freshness_window: Duration,
}

// ── Probe 14: dependency_changed ────────────────────────────────
// Input: upstream task completion Pulses from Bus
// Output: ProbeResult { surprise: 0.5 per newly completed dependency }
// Cost: O(d) where d = dependency count (typically < 10)
struct DependencyChangedProbe;

// ── Probe 15: metric_anomaly ────────────────────────────────────
// Input: tracked metrics + their running mean/variance
// Output: ProbeResult { surprise if any metric outside 2-sigma bounds }
// Cost: O(m) where m = tracked metrics (typically < 30)
struct MetricAnomalyProbe {
    sigma_threshold: f64,
    stats: DashMap<String, RunningStats>,
}

// ── Probe 16: heartbeat_timeout ─────────────────────────────────
// Input: time since last heartbeat emission
// Output: ProbeResult { always emits heartbeat; surprise if overdue }
// Cost: O(1) time comparison + heartbeat Pulse emission
struct HeartbeatTimeoutProbe {
    interval: Duration,
}
```

### 3.3 The Gamma Probe Graph

All 16 probes execute as a FanOut in the gamma loop's ASSESS step. They run in parallel (each is independent), and the results are aggregated.

```toml
[graph]
name = "gamma-probe-fanout"
version = "1.0.0"

# ── Entry node: distribute cortical snapshot to all probes ────────
[[graph.nodes]]
id = "distribute"
cell = "roko.internal.signal_copy"

# ── 16 probe nodes (parallel) ────────────────────────────────────
[[graph.nodes]]
id = "probe_config_changed"
cell = "roko.probe.config_changed"

[[graph.nodes]]
id = "probe_gate_failed"
cell = "roko.probe.gate_failed_recently"

[[graph.nodes]]
id = "probe_file_modified"
cell = "roko.probe.file_modified"

# ... probes 4-16 follow the same pattern ...

[[graph.nodes]]
id = "probe_heartbeat"
cell = "roko.probe.heartbeat_timeout"

# ── Aggregation node: merge results, determine escalation ────────
[[graph.nodes]]
id = "aggregate"
cell = "roko.probe.aggregate"

# ── FanOut edges from distribute to all probes ────────────────────
[[graph.edges]]
from = "distribute"
to = "probe_config_changed"

[[graph.edges]]
from = "distribute"
to = "probe_gate_failed"

[[graph.edges]]
from = "distribute"
to = "probe_file_modified"

# ... 13 more edges ...

[[graph.edges]]
from = "distribute"
to = "probe_heartbeat"

# ── FanIn edges from all probes to aggregate ──────────────────────
[[graph.edges]]
from = "probe_config_changed"
to = "aggregate"

[[graph.edges]]
from = "probe_gate_failed"
to = "aggregate"

# ... 14 more edges ...

[[graph.edges]]
from = "probe_heartbeat"
to = "aggregate"
```

### 3.4 Probe Aggregation

```rust
struct ProbeAggregateCell;

impl Cell for ProbeAggregateCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let results: Vec<ProbeResult> = input.iter()
            .filter_map(|s| ProbeResult::from_signal(s).ok())
            .collect();

        let max_surprise = results.iter()
            .map(|r| r.surprise)
            .fold(0.0_f32, f32::max);

        let max_stakes = results.iter()
            .map(|r| r.stakes)
            .fold(0.0_f32, f32::max);

        let any_escalate = results.iter().any(|r| r.escalate);

        let aggregate = T0ProbeResults {
            probe_count: results.len(),
            max_surprise,
            max_stakes,
            triggered: results.iter().filter(|r| r.escalate).count(),
            all_quiet: !any_escalate,
        };

        Ok(aggregate.into_signals())
    }
}
```

The total cost of the 16-probe FanOut: approximately 20-50 microseconds. This is the cost of ~80% of gamma ticks. The remaining ~20% escalate to T1/T2 at $0.001-$0.100 per call.

---

## 4. Gamma, Theta, Delta Loop Details

### 4.1 Gamma Loop: The Agent's Heartbeat

The gamma loop fires every 100ms-500ms (base), adjusted by regime. Each tick runs:

1. **T0 probes** (16 Cells in parallel, ~50us)
2. **EFE evaluation** (if any probe triggered)
3. **T1 or T2 execution** (if EFE selects a non-T0 tier)
4. **CorticalState update** (atomic writes)

```rust
// Gamma loop per-tick pseudocode
async fn gamma_tick(agent: &Agent<Active>) -> Result<()> {
    // Step 1: Run T0 probes
    let probe_results = agent.gamma_graph
        .execute_subgraph("gamma-probe-fanout", agent.cortical_snapshot())
        .await?;

    // Step 2: If all quiet, short-circuit
    if probe_results.all_quiet() {
        agent.cortical.tick_count.fetch_add(1, Ordering::Relaxed);
        agent.bus.publish(Pulse::tick_completed(agent.id, "gamma", "t0")).await?;
        return Ok(());
    }

    // Step 3: EFE tier selection
    let tier = select_tier(
        &probe_results,
        &agent.cortical,
        agent.vitality.vitality(),
        agent.clock.regime,
    );

    // Step 4: Execute at selected tier
    match tier {
        CognitiveTier::T1Deliberate => {
            let result = agent.dispatch_t1(&probe_results).await?;
            agent.verify_and_persist(result).await?;
        }
        CognitiveTier::T2Reflective => {
            let result = agent.dispatch_t2(&probe_results).await?;
            agent.verify_and_persist(result).await?;
        }
        _ => unreachable!("T0 handled above"),
    }

    // Step 5: Update CorticalState
    agent.cortical.tick_count.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
```

### 4.2 Theta Loop: Reflection

The theta loop fires every 500ms-16s (base), adjustable. It runs the full 7-step cognitive loop but with a reflective focus:

1. **SENSE**: Gather recent gamma tick summaries, Bus Pulses, and task status.
2. **ASSESS**: What has changed since the last theta tick? Is the current approach working?
3. **COMPOSE**: Assemble a reflective prompt with recent episodes and predictions.
4. **ACT**: T1 or T2 inference for summarization, replanning, or strategy adjustment.
5. **VERIFY**: Check that the replan is coherent and within budget.
6. **PERSIST/BROADCAST**: Write updated strategy; publish replan Pulses.
7. **REACT**: Update Daimon PAD, promote/demote Memory entries, adjust confidence.

Theta cadence shortens under stress:
- **Stalling** (completion rate below 25%): theta interval x 0.5
- **Anxious** (low confidence + high arousal + low dominance): theta interval x 0.66

```rust
impl AdaptiveClock {
    fn theta_interval_adjusted(&self, ctx: &ScheduleContext) -> Duration {
        let base = self.theta;

        // Stalling: reflect sooner
        if ctx.completion_rate <= 0.25 {
            return base.mul_f64(0.5);
        }

        // Anxious: reflect sooner
        if ctx.confidence < 0.3
            && (ctx.arousal > 0.25 || ctx.dominance < -0.1)
        {
            return base.mul_f64(0.66);
        }

        base
    }
}
```

### 4.3 Delta Loop: Consolidation

The delta loop fires during idle periods or on a scheduled basis (every few hours). It runs the Dream consolidation cycle as a sub-Graph:

```
NREM Replay
  -> Replay recent episodes, weighted by prediction error
  -> Extract patterns into candidate heuristics

REM Imagination
  -> HDC recombination: combine knowledge vectors from different domains
  -> Counterfactual generation: "what if?" questions about past episodes
  -> Emotional depotentiation: reduce affective charge of negative experiences

Integration Staging
  -> Validate dream outputs against existing Memory
  -> Promote to Memory if confidence exceeds threshold
  -> Emit consolidation Pulses for other agents
```

The delta loop uses T2 exclusively -- deep reasoning for synthesis and cross-domain insight.

---

## 5. The Fourth Timescale: Epsilon

The three existing timescales leave a gap: sub-second reactive execution for latency-critical domains (DeFi price monitoring, real-time safety interlocks, infrastructure alerting).

### 5.1 Epsilon: Real-Time (< 100ms)

```
Epsilon  | Period: 10ms - 100ms
         | Role: deterministic state machine execution (no LLM, no Store queries)
         | Cost: $0 (pure Rust only, no I/O)
         | Model: pre-compiled decision tables, not probes
```

Epsilon is not T0. T0 probes check for changes and decide whether to escalate. Epsilon executes pre-compiled actions without decision-making. The difference:

```
T0: "Has the price moved? No -> do nothing. Yes -> escalate to T1."
Epsilon: "Price crossed stop-loss threshold -> execute the hedge NOW."
```

### 5.2 Epsilon Loop as a Hot Graph

```toml
[graph]
name = "epsilon-loop"
version = "1.0.0"
hot = true
clock = { kind = "fixed", period_ms = 50 }  # 20Hz

[graph.policy]
max_parallelism = 1
failure_strategy = "skip"  # Never retry in epsilon; missed tick is lost

[[graph.nodes]]
id = "read_state"
cell = "roko.epsilon.read_cortical"
execution_class = "workflow"

[[graph.nodes]]
id = "evaluate_triggers"
cell = "roko.epsilon.trigger_table"
execution_class = "workflow"

[[graph.nodes]]
id = "execute_action"
cell = "roko.epsilon.deterministic_action"
execution_class = "activity"

[[graph.edges]]
from = "read_state"
to = "evaluate_triggers"

[[graph.edges]]
from = "evaluate_triggers"
to = "execute_action"
condition = "triggers.any_fired"
```

### 5.3 Constraints

Epsilon Cells are severely constrained:
- No LLM calls (latency budget < 100ms total).
- No Store queries (I/O latency is unpredictable).
- No Bus publish during execution (async overhead). Pulses are batched and published after the tick.
- Only pre-compiled decision tables (populated by gamma/theta).
- Epsilon Cells must complete within the tick period or be killed.

### 5.4 Open Design Questions for Epsilon

- Should epsilon ticks update CorticalState? If so, the atomic writes add contention with gamma reads.
- How does epsilon interact with the Verify protocol? Post-hoc verification (verify after the action is taken) is the only option at this latency.
- Should epsilon have its own budget? Or does it draw from the Agent's global budget with a micro-reserve?

---

## 6. Temperament: PAD Modulation of Timescales

The Daimon's PAD (Pleasure-Arousal-Dominance) vector modulates how the three timescales interact. This creates emergent "temperament" without explicit personality programming.

| PAD State | Gamma Effect | Theta Effect | Delta Effect |
|---|---|---|---|
| High confidence + low arousal | Higher T0/T1 threshold (coast longer) | Longer theta interval (fewer reflections) | Normal delta schedule |
| Low confidence + high arousal | Lower escalation threshold | Shorter theta interval (0.66x) | Delta deferred (active work takes priority) |
| High dominance | More willing to act on T1 without T2 | Confident replans at theta speed | Normal |
| Low dominance | Requires T2 for significant actions | Cautious replans; may escalate to T2 | Normal |
| Low arousal + neutral pleasure | Extended gamma; T0-heavy | Extended theta interval | Triggers delta (consolidate during boredom) |

The PAD vector is not a dial that an operator sets. It is computed from recent outcomes: gate passes increase pleasure, unexpected failures increase arousal and decrease dominance. The behavior emerges from the feedback between outcomes and timescale modulation.

---

## 7. What This Enables

- **Zero-cost majority**: ~80% of gamma ticks execute 16 probes in < 50 microseconds and return. No LLM call. $0 cost.
- **Proportional compute**: The EFE cascade invests compute proportionally to difficulty. Routine ticks are free; hard problems get the full model.
- **Emergent temperament**: PAD-modulated timescales create agents with distinct behavioral profiles without explicit personality configuration.
- **Independent failure domains**: A delta dream crash does not stop gamma perception. A theta replan failure does not block gamma action.
- **Real-time extension**: The epsilon timescale extends the architecture to sub-second latency-critical domains without changing the fundamental model.

---

## 8. Feedback Loops

| Loop | Timescale | What it observes | What it adjusts |
|---|---|---|---|
| **Probe calibration** | Gamma | Probe false-positive rate (escalation led to T0-level work) | Probe thresholds |
| **EFE adaptation** | Gamma | Prediction error vs tier used | EFE cost and value weights |
| **Theta cadence** | Theta | Completion rate, PAD state | Theta interval (0.5x - 2.0x) |
| **Reflex promotion** | Theta | T2 pattern success rate | T0 reflex store (promote at 5+ successes, >90% pass) |
| **Dream prioritization** | Delta | Prediction error magnitudes from episodes | NREM replay ordering |
| **Regime transitions** | Cross-timescale | PE trend over 3+ ticks | Adaptive clock multipliers |

---

## 9. Open Questions

1. **Theta-gamma phase coupling**: In neuroscience, theta oscillations modulate gamma amplitude (theta-gamma coupling, Lisman & Jensen 2013). Should theta ticks modulate gamma probe thresholds? If theta determines "we are on the right track," gamma probes could raise their thresholds (coast more). If theta determines "something is wrong," gamma probes lower thresholds (heightened perception).

2. **Delta timing**: Should delta fire on a fixed schedule, or purely on idle detection? Fixed schedules ensure consolidation happens even during sustained busy periods. Idle-triggered ensures consolidation does not interrupt productive work. The current spec says "idle or scheduled" -- is the schedule configurable per-domain?

3. **Multi-agent timescale coordination**: When multiple agents share a Bus, their gamma loops fire independently. Should there be a synchronization mechanism (e.g., a shared gamma clock) to prevent Bus congestion from simultaneous probe execution? Or is eventual consistency sufficient?

4. **Probe composition**: Can probes compose? For example, "compile_error_new AND confidence_dropping" might trigger a different escalation path than either alone. Should there be a probe composition algebra (AND/OR/SEQUENCE)?

5. **Epsilon budget isolation**: If epsilon draws from the Agent's global budget, a misconfigured epsilon trigger table could exhaust the budget before gamma/theta ever fire. Should epsilon have a hard budget cap (e.g., 1% of total)?

---

## Cross-References

- [07-AGENT-RUNTIME.md](../../unified/07-AGENT-RUNTIME.md) SS7-9 -- Adaptive clock, pipeline, EFE gating
- [05-EXECUTION-ENGINE.md](../../unified/05-EXECUTION-ENGINE.md) -- Hot Graph execution, Engine API
- [04-SPECIALIZATIONS.md](../../unified/04-SPECIALIZATIONS.md) SS10 -- Agent definition
- [cross-cut-functors.md](cross-cut-functors.md) -- How Daimon PAD modulates timescales
