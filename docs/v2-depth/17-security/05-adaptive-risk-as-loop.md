# Adaptive Risk as Loop

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Expresses adaptive risk control, loop detection, and temporal logic monitoring as Loops with predict-publish-correct. The 5-layer runtime risk model is a Loop where each cycle refines thresholds. Circuit breaker is a React Cell. LTL/CTL monitors compile into Buchi automata run as Verify Cells.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, Pulse, Bus), [02-CELL](../../unified/02-CELL.md) (React protocol, Verify protocol, predict-publish-correct), [03-GRAPH](../../unified/03-GRAPH.md) (Loop pattern, feedback edges), [07-LEARNING](../../unified/07-LEARNING.md) (adaptive thresholds, EMA), [16-SECURITY](../../unified/16-SECURITY.md) (recursive safety monitoring, rate limits, quality bounds)

---

## 1. Risk Control as a Loop, Not a Wall

Static safety checks are walls: they block or allow, once, at a fixed threshold. The problem with walls is that the world changes -- models improve, attack surfaces shift, and fixed thresholds either block too much (over-conservative) or too little (under-conservative).

Roko's risk control is a **Loop** (see [03-GRAPH.md](../../unified/03-GRAPH.md) for the pattern definition): a Graph with a feedback edge from output back to input. Each cycle of the Loop:

1. **Predicts** the risk of the proposed action (pre-execution).
2. **Publishes** the prediction as a Pulse on the Bus.
3. **Observes** the outcome (post-execution Gate verdicts, incidents, rejections).
4. **Corrects** the risk model by adjusting thresholds, tightening or loosening guardrails.

This is predict-publish-correct (see [02-CELL.md](../../unified/02-CELL.md)) applied to safety. The risk model learns from every action, every failure, every incident. Thresholds are not configured once -- they evolve.

---

## 2. The Five-Layer Risk Control Model

Five layers of runtime risk control, each operating as a T0 deterministic check (no LLM calls, zero inference cost per tick). The LLM proposes actions; the risk engine disposes.

| Layer | Name | Cell Type | What It Does | Feedback Mechanism |
|---|---|---|---|---|
| 1 | **Hard Shields** | Verify | Immutable constraints that cannot be overridden | None -- these never change at runtime |
| 2 | **Position Sizing** | Verify | Kelly-criterion allocation with confidence modulation | Outcome-adjusted confidence |
| 3 | **Adaptive Guardrails** | Verify + React | Bayesian trust expansion/contraction | Beta-Binomial posterior update |
| 4 | **Health Observation** | Observe (Lens) | Anomaly detection, health scoring | EMA on health metrics |
| 5 | **Domain Threat Detection** | Verify | Domain-specific threats (MEV, supply chain, etc.) | Pattern library from immune memory |

### Layer 1: Hard Shields (Immutable Verify Cell)

Hard shields are constraints that never change at runtime. They are the bedrock of the risk model.

```rust
/// Layer 1: Hard Shields. Immutable constraints.
/// These are Verify Cells with no feedback edge -- they never adapt.
pub struct HardShieldCell {
    /// General-purpose shields
    pub max_files_per_task: u32,         // default: 50
    pub max_cost_per_task_usd: f64,      // default: 10.0
    pub forbidden_paths: Vec<PathPattern>,
    pub forbidden_commands: Vec<String>,

    /// Chain-domain shields (if applicable)
    pub max_position_usd: Option<f64>,
    pub max_single_trade_usd: Option<f64>,
    pub emergency_stop_drawdown: Option<f64>,  // e.g., -5% triggers halt
}

impl VerifyCell for HardShieldCell {
    fn name(&self) -> &str { "hard-shields" }

    async fn verify(
        &self,
        action: &Signal,
        _ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        // Check each immutable constraint
        if let Some(file_count) = extract_file_count(action) {
            if file_count > self.max_files_per_task {
                return Ok(Verdict::reject(format!(
                    "task touches {} files, limit is {}",
                    file_count, self.max_files_per_task
                )));
            }
        }

        if let Some(cost) = extract_estimated_cost(action) {
            if cost > self.max_cost_per_task_usd {
                return Ok(Verdict::reject(format!(
                    "estimated cost ${:.2} exceeds limit ${:.2}",
                    cost, self.max_cost_per_task_usd
                )));
            }
        }

        // ... additional immutable checks ...

        Ok(Verdict::pass(1.0, Evidence::HardShield))
    }
}
```

Hard shields have no feedback edge. They are the one layer that does not learn, does not adapt, and does not change without a configuration update and restart. This is intentional: the hard floor must not drift.

### Layer 3: Adaptive Guardrails (The Core Loop)

Adaptive guardrails are the heart of the risk Loop. They use a Beta-Binomial model to track trust per (agent, capability) pair. Trust starts neutral and moves based on outcomes.

```rust
/// Layer 3: Adaptive Guardrails.
/// Trust expands with successful outcomes, contracts with failures.
/// Uses a Beta-Binomial posterior: Beta(alpha, beta) where
///   alpha = successes + prior_alpha
///   beta  = failures  + prior_beta
pub struct AdaptiveGuardrailCell {
    /// Trust state per (agent, capability) pair.
    trust: HashMap<(AgentId, Capability), BetaDistribution>,
    /// Minimum trust required to permit autonomous action.
    min_trust_threshold: f64,     // default: 0.4
    /// Trust below this triggers escalation (human review).
    escalation_threshold: f64,    // default: 0.2
}

#[derive(Clone)]
pub struct BetaDistribution {
    pub alpha: f64,  // successes + prior
    pub beta: f64,   // failures + prior
}

impl BetaDistribution {
    /// Neutral prior: Beta(2, 2) -- slight skepticism.
    pub fn neutral() -> Self {
        Self { alpha: 2.0, beta: 2.0 }
    }

    /// Mean of the Beta distribution: alpha / (alpha + beta).
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Update with a success (Gate pass).
    pub fn record_success(&mut self) {
        self.alpha += 1.0;
    }

    /// Update with a failure (Gate fail, incident, rejection).
    pub fn record_failure(&mut self) {
        self.beta += 1.0;
    }
}

impl VerifyCell for AdaptiveGuardrailCell {
    fn name(&self) -> &str { "adaptive-guardrails" }

    async fn verify(
        &self,
        action: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        let agent_id = extract_agent_id(action)?;
        let capability = extract_required_capability(action)?;
        let key = (agent_id, capability);

        let trust = self.trust.get(&key)
            .cloned()
            .unwrap_or_else(BetaDistribution::neutral);

        let trust_mean = trust.mean();

        if trust_mean < self.escalation_threshold {
            // Trust too low -- escalate to human review
            return Ok(Verdict::escalate(format!(
                "trust {:.2} below escalation threshold {:.2} for {:?}",
                trust_mean, self.escalation_threshold, key,
            )));
        }

        if trust_mean < self.min_trust_threshold {
            // Trust below autonomous threshold -- add extra verification
            return Ok(Verdict::pass_with_condition(
                trust_mean,
                Evidence::AdaptiveTrust { trust_mean },
                Condition::RequirePostVerification,
            ));
        }

        // Trust sufficient -- permit autonomous action
        Ok(Verdict::pass(trust_mean, Evidence::AdaptiveTrust { trust_mean }))
    }
}
```

### The Feedback Edge

The Loop's feedback edge connects post-execution outcomes back to the adaptive guardrails:

```toml
# The adaptive risk Loop
[graph]
id = "adaptive-risk-loop"
description = "Risk control with predict-publish-correct feedback"
pattern = "Loop"

[[graph.cells]]
id = "risk-predict"
protocol = "Verify"
description = "Predict action risk using current trust model"

[[graph.cells]]
id = "risk-observe"
protocol = "Observe"
description = "Observe execution outcome (Gate verdicts, incidents)"

[[graph.cells]]
id = "risk-correct"
protocol = "React"
description = "Update trust model based on observed outcomes"

# Forward path: predict -> execute -> observe
[[graph.edges]]
from = "risk-predict.permit"
to = "risk-observe.in"

# Feedback edge: observe outcomes -> correct trust -> predict
[[graph.edges]]
from = "risk-observe.outcomes"
to = "risk-correct.in"

[[graph.edges]]
from = "risk-correct.updated_trust"
to = "risk-predict.trust_model"
```

The feedback React Cell updates the Beta distribution:

```rust
/// React Cell: update trust model based on observed outcomes.
pub struct RiskCorrectCell;

impl Cell for RiskCorrectCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "risk-correct" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut updates = Vec::new();

        for signal in &input {
            let outcome = extract_outcome(signal)?;

            match outcome {
                Outcome::GatePass { agent, capability, .. } => {
                    // Success: trust increases
                    updates.push(Signal::new(Kind::TrustUpdate, TrustUpdate {
                        agent,
                        capability,
                        direction: TrustDirection::Increase,
                    }));
                }
                Outcome::GateFail { agent, capability, .. } |
                Outcome::Incident { agent, capability, .. } => {
                    // Failure: trust decreases
                    updates.push(Signal::new(Kind::TrustUpdate, TrustUpdate {
                        agent,
                        capability,
                        direction: TrustDirection::Decrease,
                    }));
                }
                Outcome::Rejection { agent, capability, head, .. } => {
                    // Corrigibility rejection: trust decreases sharply
                    // Higher-priority head rejections have more impact
                    let impact = match head {
                        CorrigibilityHead::Deference => 3.0,
                        CorrigibilityHead::Switch => 2.5,
                        CorrigibilityHead::Truth => 2.0,
                        CorrigibilityHead::Impact => 1.5,
                        CorrigibilityHead::Task => 1.0,
                    };
                    updates.push(Signal::new(Kind::TrustUpdate, TrustUpdate {
                        agent,
                        capability,
                        direction: TrustDirection::DecreaseBy(impact),
                    }));
                }
            }
        }

        Ok(updates)
    }
}
```

---

## 3. Circuit Breaker as a React Cell

The circuit breaker pattern (from `crates/roko-conductor/`) maps to a React Cell with a three-state machine. It subscribes to outcome Pulses on the Bus and transitions between states based on observed failure rates.

```rust
/// Circuit breaker: React Cell that monitors agent health
/// and intervenes when anomalies are detected.
///
/// States:
///   Closed (normal) -> Half-Open (testing) -> Open (broken)
///
/// Transitions:
///   Closed -> Open:     failure rate exceeds threshold
///   Open -> Half-Open:  cooldown period expires
///   Half-Open -> Closed: test action succeeds
///   Half-Open -> Open:   test action fails
pub struct CircuitBreakerCell {
    state: AtomicU8,  // 0=Closed, 1=HalfOpen, 2=Open
    failure_window: VecDeque<(Instant, bool)>,  // (time, was_success)
    failure_threshold: f64,    // default: 0.5
    window_size: usize,        // default: 10
    cooldown: Duration,        // default: 60s
    last_open: Option<Instant>,
}

impl Cell for CircuitBreakerCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::React] }
    fn name(&self) -> &str { "circuit-breaker" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            let outcome = extract_outcome(signal)?;
            let success = matches!(outcome, Outcome::GatePass { .. });

            // Record outcome
            self.failure_window.push_back((Instant::now(), success));
            while self.failure_window.len() > self.window_size {
                self.failure_window.pop_front();
            }

            // Calculate failure rate
            let failures = self.failure_window.iter()
                .filter(|(_, s)| !s).count();
            let failure_rate = failures as f64 / self.failure_window.len() as f64;

            let current_state = self.state.load(Ordering::Relaxed);

            match current_state {
                0 => { // Closed
                    if failure_rate > self.failure_threshold {
                        self.state.store(2, Ordering::Relaxed);
                        self.last_open = Some(Instant::now());
                        outputs.push(Signal::pulse(
                            Kind::Alert,
                            topic!("safety.circuit_breaker.open"),
                            CircuitBreakerEvent::Opened {
                                failure_rate,
                                threshold: self.failure_threshold,
                            },
                        ));
                    }
                }
                1 => { // Half-Open
                    if success {
                        self.state.store(0, Ordering::Relaxed);
                        outputs.push(Signal::pulse(
                            Kind::Event,
                            topic!("safety.circuit_breaker.closed"),
                            CircuitBreakerEvent::Closed,
                        ));
                    } else {
                        self.state.store(2, Ordering::Relaxed);
                        self.last_open = Some(Instant::now());
                        outputs.push(Signal::pulse(
                            Kind::Alert,
                            topic!("safety.circuit_breaker.reopened"),
                            CircuitBreakerEvent::Reopened,
                        ));
                    }
                }
                2 => { // Open
                    if let Some(opened_at) = self.last_open {
                        if Instant::now().duration_since(opened_at) > self.cooldown {
                            self.state.store(1, Ordering::Relaxed);
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        Ok(outputs)
    }
}
```

The circuit breaker composes with the risk Loop: when the circuit opens, the adaptive guardrails receive a sharp trust decrease for the affected agent. When the circuit closes after recovery, trust begins to rebuild.

---

## 4. Temporal Logic Monitoring as Verify Cells

Temporal logic properties (LTL and CTL) express safety requirements that span sequences of events over time. These are compiled into Buchi automata -- finite state machines that monitor the event stream and detect violations.

### LTL Safety Properties

Safety properties specify what must always be true:

```
G(shell_exec -> pre_check_passed)
    "Every shell execution was preceded by a pre-check"

G(gate_fail -> F(task_paused | task_skipped))
    "Every gate failure is eventually followed by pause or skip"

G(task_started -> F(task_completed | task_failed))
    "Every started task eventually completes or fails" (liveness)

G(secret_read -> audit_logged)
    "Every secret access is immediately logged"
```

### LTL Formulas as Verify Cells

Each LTL formula compiles to a Buchi automaton, which is run as a Verify Cell that monitors the Pulse stream:

```rust
/// A Buchi automaton compiled from an LTL formula.
/// Monitors the event stream and rejects when the automaton
/// enters a state from which no accepting path exists.
pub struct BuchiMonitorCell {
    /// The LTL formula this monitor checks.
    formula: String,
    /// The compiled automaton states.
    states: Vec<BuchiState>,
    /// Current state of the automaton.
    current_state: usize,
    /// Accepting states (the automaton must visit these infinitely often).
    accepting: HashSet<usize>,
}

pub struct BuchiState {
    /// Transitions: (predicate on Pulse) -> next state
    transitions: Vec<(PulsePredicate, usize)>,
}

impl VerifyCell for BuchiMonitorCell {
    fn name(&self) -> &str { "temporal-monitor" }

    async fn verify(
        &self,
        action: &Signal,
        ctx: &CellContext,
    ) -> Result<Verdict, CellError> {
        // Feed the action into the automaton
        let event = signal_to_event(action);
        let state = &self.states[self.current_state];

        // Find matching transition
        let next_state = state.transitions.iter()
            .find(|(pred, _)| pred.matches(&event))
            .map(|(_, next)| *next);

        match next_state {
            Some(next) => {
                self.current_state = next;

                // Check if we've entered a rejecting sink
                if self.is_rejecting_sink(next) {
                    return Ok(Verdict::reject(format!(
                        "temporal property violated: {} (state {})",
                        self.formula, next,
                    )));
                }

                Ok(Verdict::pass(1.0, Evidence::TemporalCheck {
                    formula: self.formula.clone(),
                    state: next,
                }))
            }
            None => {
                // No matching transition -- deadlock in automaton
                Ok(Verdict::reject(format!(
                    "temporal monitor deadlocked on formula: {}",
                    self.formula,
                )))
            }
        }
    }
}
```

### Ghost Turn Detection

Ghost turns (empty responses, repeated tool failures, no-progress turns) are a temporal logic property:

```
G(turn_completed -> (output_nonempty | efficiency > threshold))
    "Every completed turn produces non-empty output or meets efficiency"

G(ghost_count > 3 -> F(circuit_open))
    "Three consecutive ghost turns trigger circuit breaker"
```

Ghost turn detection is implemented as a Buchi monitor that counts consecutive empty turns and triggers the circuit breaker when the threshold is exceeded.

---

## 5. Composition with Adaptive Gate Thresholds

The adaptive gate threshold system (already wired at `crates/roko-learn/`, persisted to `.roko/learn/gate-thresholds.json`) is itself a Loop. It tracks Gate pass rates per rung using EMA (Exponential Moving Average) and adjusts thresholds:

```rust
/// Adaptive gate threshold: a Loop that tightens or loosens
/// Gate requirements based on observed pass rates.
pub struct AdaptiveGateThreshold {
    /// EMA of pass rate per Gate rung.
    ema: HashMap<GateRung, f64>,
    /// EMA smoothing factor (default: 0.1).
    alpha: f64,
    /// Target pass rate (default: 0.7).
    target_pass_rate: f64,
}

impl AdaptiveGateThreshold {
    /// Update the EMA with a new observation.
    pub fn update(&mut self, rung: GateRung, passed: bool) {
        let value = if passed { 1.0 } else { 0.0 };
        let current = self.ema.entry(rung).or_insert(self.target_pass_rate);
        *current = self.alpha * value + (1.0 - self.alpha) * *current;
    }

    /// Get the current threshold for a Gate rung.
    /// When EMA drops (more failures), threshold tightens (requires higher confidence).
    /// When EMA rises (more passes), threshold loosens (allows lower confidence).
    pub fn threshold(&self, rung: &GateRung) -> f64 {
        let ema = self.ema.get(rung).copied().unwrap_or(self.target_pass_rate);
        // Invert: low pass rate -> high threshold (stricter)
        1.0 - ema
    }
}
```

This gate threshold Loop composes with the adaptive risk Loop:

```
Risk Loop                           Gate Threshold Loop
  predict -> execute -> observe       gate -> verdict -> EMA update
                    |                              |
                    +-- outcome feeds both loops --+
```

Both Loops share the same outcome stream (Gate verdicts as Pulses on the Bus). An agent that fails Gates frequently sees both its trust decrease (risk Loop) and its Gate thresholds tighten (gate Loop). The two Loops reinforce each other without explicit coupling -- they are connected only through the shared Bus fabric.

---

## 6. Layer 4: Health Observation as Lens

Health observation is a Lens Cell (read-only) that monitors the agent's vital signs without intervening:

```rust
/// Layer 4: Health Observation Lens.
/// Monitors agent health metrics and publishes anomaly Pulses.
pub struct HealthObservationLens {
    /// Efficiency baseline (tokens out / tokens in).
    efficiency_baseline: f64,
    /// Health score weights.
    weights: HealthWeights,
}

pub struct HealthWeights {
    pub gate_pass_rate: f64,     // weight: 0.3
    pub efficiency: f64,          // weight: 0.2
    pub error_rate: f64,          // weight: 0.2
    pub cost_rate: f64,           // weight: 0.15
    pub latency: f64,             // weight: 0.15
}

impl Cell for HealthObservationLens {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn name(&self) -> &str { "health-observation" }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let metrics = aggregate_health_metrics(&input)?;
        let health_score = self.compute_health_score(&metrics);

        let mut outputs = Vec::new();

        // Publish health score as Pulse
        outputs.push(Signal::pulse(
            Kind::Metric,
            topic!("safety.health.score"),
            HealthScore {
                agent_id: metrics.agent_id,
                score: health_score,
                components: metrics,
            },
        ));

        // Detect anomalies
        if health_score < 0.3 {
            outputs.push(Signal::pulse(
                Kind::Alert,
                topic!("safety.health.critical"),
                HealthAlert {
                    agent_id: metrics.agent_id,
                    score: health_score,
                    recommendation: ConductorDecision::Pause,
                },
            ));
        } else if health_score < 0.5 {
            outputs.push(Signal::pulse(
                Kind::Alert,
                topic!("safety.health.warning"),
                HealthAlert {
                    agent_id: metrics.agent_id,
                    score: health_score,
                    recommendation: ConductorDecision::Continue,
                },
            ));
        }

        Ok(outputs)
    }
}
```

---

## What This Enables

1. **Self-improving safety**: Risk thresholds evolve with experience. Agents that consistently pass Gates earn trust; agents that fail lose it. The system does not need manual threshold tuning.
2. **Fast failure detection**: Circuit breaker and temporal monitors catch runaway behavior in real-time, not after the fact.
3. **Composable risk layers**: Five independent risk layers compose through the shared Bus fabric. Adding a domain-specific risk layer means adding a new Cell, not modifying existing ones.
4. **Temporal awareness**: LTL/CTL monitors detect patterns that point-in-time checks miss: escalation sequences, permission probing, ghost turns, liveness violations.
5. **Hard floor guarantee**: Layer 1 (Hard Shields) provides an immutable safety floor that the adaptive layers cannot erode.

## Feedback Loops

- **L1**: Adaptive guardrails (Beta-Binomial) update after every action outcome. Trust expands with successes, contracts with failures.
- **L2**: Gate thresholds (EMA) adjust after every Gate verdict. Persistent failures tighten the gates.
- **L3**: Circuit breaker state feeds the conductor's diagnosis engine. Open circuits trigger root cause analysis.
- **L4**: Health observation metrics feed dashboard projections and episode logging.
- **Memory**: Trust state persisted to `.roko/learn/` survives restarts. New sessions start from the last known trust levels, not from neutral.

## Open Questions

1. **Trust recovery rate**: How fast should trust recover after a failure? The Beta-Binomial model gives equal weight to successes and failures. Should recovery be slower than decay (asymmetric update) to be conservative?

2. **Cross-agent trust**: Should trust be per-agent or per-model? If an agent using Claude-3.5-sonnet has high trust, does a new agent using the same model inherit it? The current design is per-(agent, capability) pair, which does not share across agents.

3. **Temporal monitor overhead**: Each LTL formula compiles to a Buchi automaton with potentially exponential state space. For complex formulas (nested Until operators), the automaton may be too large. Practical limits on formula complexity need empirical testing.

4. **Adaptive guardrail gaming**: An adversarial agent could build trust with many small successes, then exploit the earned trust for one large malicious action. The hard shields (Layer 1) are the defense, but the gap between the hard floor and the adaptive ceiling could be exploited.

## Implementation Tasks

| Task | File | What |
|---|---|---|
| Implement AdaptiveGuardrailCell | `crates/roko-agent/src/safety/` | Beta-Binomial trust model with Verify protocol |
| Wire circuit breaker as React Cell | `crates/roko-conductor/src/` | Express existing circuit breaker as a Cell conforming to React protocol |
| Persist trust state | `crates/roko-learn/src/` | Save/load Beta-Binomial parameters to `.roko/learn/trust-state.json` |
| Implement BuchiMonitorCell | `crates/roko-gate/src/` | LTL formula compiler and Buchi automaton Verify Cell |
| Add ghost turn detection | `crates/roko-conductor/src/` | Temporal monitor for consecutive empty/no-progress turns |
| Compose risk Loop with gate threshold Loop | `crates/roko-cli/src/orchestrate.rs` | Both Loops subscribe to same Bus topics; outcomes feed both |
| Integration test: trust decay after failures | `crates/roko-agent/tests/` | Simulate 5 failures, verify trust drops below escalation threshold |
| Integration test: circuit breaker recovery | `crates/roko-conductor/tests/` | Open circuit, wait cooldown, verify half-open transition |
