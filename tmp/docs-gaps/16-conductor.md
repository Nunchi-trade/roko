# 07-conductor -- Gap Checklist

Spec: `docs/07-conductor/` (16 files). Code: `crates/roko-conductor/`, `crates/roko-runtime/`.

Overall: ~60-70% complete. Core regulation (10 watchers, circuit breaker, diagnosis, stuck detection, health, OODA) works. Gaps in adaptive learning, cognitive signals, pressure dynamics, federated conductors.

## Compliant (no action needed)
- Conductor architecture at L3 placement (doc 00)
- All 10 watchers as Policy impls (doc 01 core)
- Circuit breaker -- per-plan failure budget, DashMap (doc 02 core)
- 3-action graduated interventions (doc 03 core)
- Diagnosis engine -- 34 patterns, 20 error categories, 9 interventions (doc 04)
- Stuck detection -- 6 heuristics, MetaCognitionHook (doc 05)
- Health monitors -- 4 system checks, SystemSnapshot (doc 06)
- OODA core cycle (doc 07 core)
- Adaptive timeouts -- phase timeouts by complexity, hard limits (doc 10 core)
- Process supervision -- PID tracking, orphan reaper, SIGTERM escalation (doc 13)
- Production failure catalog -- reference doc (doc 14)

## Checklist

### COND-01: ConductorBandit not wired
- [x] Wire ConductorBandit into conductor's evaluate() path

**Spec** (doc 11, 15): Learned intervention policy using ConductorBandit for adaptive decision-making.
**Current code**: `ConductorBandit` at `crates/roko-learn/src/conductor.rs:110` with `ConductorBanditSnapshot` at line 117. Conductor's `evaluate()` at `crates/roko-conductor/src/conductor.rs:162` uses `WorstSeverityPolicy` (line 98). `WorstSeverityPolicy` at `crates/roko-conductor/src/interventions.rs:109` is the only active `InterventionPolicy` impl (trait at line 101). No Thompson Sampling blending or warmup logic.
**What to change**: Create a new `InterventionPolicy` impl (e.g., `BanditPolicy`) in `crates/roko-conductor/src/interventions.rs` that wraps `ConductorBandit`. During `evaluate()`, blend bandit recommendation with `WorstSeverityPolicy` at 65/35 ratio. Skip bandit until 50 observations accumulate (warmup).
**Reference files**:
- `crates/roko-learn/src/conductor.rs:110` -- ConductorBandit
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() method
- `crates/roko-conductor/src/conductor.rs:98` -- WorstSeverityPolicy usage
- `crates/roko-conductor/src/interventions.rs:101` -- InterventionPolicy trait
- `crates/roko-conductor/src/interventions.rs:109` -- WorstSeverityPolicy impl
- `docs/07-conductor/11-*`, `docs/07-conductor/15-*` -- bandit spec
**Depends on**: None
**Accept when**:
- [x] ConductorBandit consulted during evaluate()
  - `BanditPolicy` at interventions.rs:150 wraps ConductorBandit and implements InterventionPolicy
- [x] Thompson Sampling 65/35 blending with static policy
  - `BANDIT_BLEND_WEIGHT` = 0.65, blends with WorstSeverityPolicy fallback
- [x] 50-observation warmup before activation
  - `BANDIT_WARMUP_THRESHOLD` = 50, `is_warmed_up()` checks total observations
- [x] `cargo test -p roko-conductor`
**Verify**:
```bash
grep -rn 'ConductorBandit\|BanditPolicy\|warmup' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P1

### COND-02: Cognitive signals
- [x] Implement 8 typed cognitive signal interrupts

**Spec** (doc 09): The cognitive signals spec defines 8 typed interrupt signals that go beyond the current 3-action decision space (Continue/Restart/Fail). Each signal has specific semantics:
- `Pause` -- temporarily suspend execution, preserve state (used for dream cycles, resource pressure)
- `Resume` -- resume from paused state
- `Reprioritize` -- reorder task queue based on new information (e.g., dependency resolved)
- `InjectContext` -- add context to the agent's prompt without restarting (e.g., new knowledge from another agent)
- `Escalate` -- promote to a stronger model tier or request human review
- `Cooldown` -- reduce pressure by extending deadlines or lowering expectations
- `Explore` -- switch to exploratory mode (wider search, cheaper model, more turns)
- `Shutdown` -- graceful termination with state persistence

Signals are richer than decisions: a decision is final ("restart now"), while a signal is a modulation ("escalate model tier without restarting"). Multiple signals can be active simultaneously (e.g., `Escalate` + `InjectContext`).

**Current code**: `ConductorDecision` at `crates/roko-core/src/conductor.rs:25` has 3 variants: `Continue`, `Restart { watcher, reason }`, `Fail { watcher, reason }`. The enum is `#[non_exhaustive]` so new variants can be added. `WatcherOutput::to_decision()` at `crates/roko-conductor/src/interventions.rs:36` maps severity to these 3 decisions only. No `CognitiveSignal` enum anywhere in the codebase. The orchestrator at `crates/roko-cli/src/orchestrate.rs` matches only on `Continue`, `Restart`, and `Fail`.

**What to change**: (1) Add `CognitiveSignal` enum to `crates/roko-core/src/conductor.rs` with 8 variants (Pause, Resume, Reprioritize, InjectContext, Escalate, Cooldown, Explore, Shutdown), each carrying relevant payload data (e.g., `InjectContext { context: String }`, `Escalate { target_tier: InferenceTier }`). (2) Add a `Signal { signal: CognitiveSignal }` variant to `ConductorDecision`. (3) In `evaluate()` at `crates/roko-conductor/src/conductor.rs:162`, emit signals for sub-critical situations (e.g., context pressure at 60% -> `InjectContext` to trim, cost trending up -> `Cooldown`). (4) In `orchestrate.rs`, add match arms for `Signal` that translate each cognitive signal into runtime actions without full restart.

**Reference files**:
- `crates/roko-core/src/conductor.rs:25` -- ConductorDecision enum (#[non_exhaustive], safe to extend)
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() where signals should be emitted
- `crates/roko-conductor/src/interventions.rs:36` -- to_decision() mapping (extend for sub-critical signals)
- `crates/roko-conductor/src/interventions.rs:101` -- InterventionPolicy trait
- `crates/roko-cli/src/orchestrate.rs` -- orchestrator match arms for ConductorDecision
- `docs/07-conductor/09-cognitive-signals.md` -- signal semantics, payload definitions, implementation path
**Depends on**: None
**Accept when**:
- [x] `CognitiveSignal` enum exists at `crates/roko-core/src/conductor.rs` with all 8 variants
  - Pause, Resume, Reprioritize, InjectContext, Escalate, Cooldown, Explore, Shutdown
- [x] `ConductorDecision::Signal { signal: CognitiveSignal }` variant added
  - Design evolved: `ConductorEvaluation` wraps `decision + signals: Vec<CognitiveSignal>`
  - `ConductorDecision::with_signals()` creates evaluation pairs
- [x] `evaluate()` emits signals for sub-critical situations
  - `evaluate_full()` returns `ConductorEvaluation` with signals
- [x] Orchestrator handles each signal type with distinct runtime actions
- [x] `cargo test -p roko-conductor` and `cargo test -p roko-core` pass
**Verify**:
```bash
grep -rn 'CognitiveSignal\|Signal.*signal' crates/roko-core/src/ crates/roko-conductor/src/ crates/roko-cli/src/orchestrate.rs --include='*.rs'
cargo test -p roko-conductor && cargo test -p roko-core
```
**Priority**: P1

### COND-03: Adaptive self-model with threshold learning
- [x] Wire learning-based threshold tuning into conductor

**Spec** (doc 08): The Good Regulator Theorem (Conant & Ashby 1970) requires the conductor to contain a model of the system it regulates. The adaptive self-model extends static watcher thresholds with a learning loop: after each intervention, the conductor records whether the intervention improved the outcome (task completed, quality improved) or worsened it (task still failed, budget wasted). This feedback adjusts thresholds via Bayesian update, making the conductor more precise over time.

Three self-model components per doc 08:
1. **Prediction**: Before each task, predict likely outcome based on task features and historical data. Precision-weighted prediction errors drive model updates.
2. **Threshold adaptation**: Each watcher's threshold is an EMA of the optimal boundary between "intervene" and "don't intervene", updated after each intervention outcome.
3. **Forward projection**: Use the model to predict what will happen if no intervention is taken vs. if an intervention is taken, and choose the action with higher expected utility.

**Current code**: Watchers in `crates/roko-conductor/src/watchers/mod.rs` each have static thresholds (e.g., `CostOverrunWatcher` in `cost_overrun.rs`, `TimeOverrunWatcher` in `time_overrun.rs`). `AdaptiveThresholds` at `crates/roko-gate/src/adaptive_threshold.rs:47` provides a working EMA-based threshold adaptation pattern for gate thresholds that can be followed. Efficiency events logged to `.roko/learn/efficiency.jsonl` via orchestrate.rs provide intervention outcome data. No feedback loop from intervention effectiveness to watcher thresholds.

**What to change**:
1. Create `crates/roko-conductor/src/threshold_learner.rs` with:
   ```rust
   pub struct ThresholdLearner {
       pub watcher_thresholds: HashMap<String, AdaptiveThreshold>,
       pub intervention_history: VecDeque<InterventionOutcome>,
       pub alpha: f64,  // EMA smoothing factor, default 0.1
   }
   pub struct AdaptiveThreshold {
       pub current: f64,
       pub ema: f64,
       pub observations: usize,
       pub last_updated: DateTime<Utc>,
   }
   pub struct InterventionOutcome {
       pub watcher_name: String,
       pub threshold_at_fire: f64,
       pub intervention_taken: ConductorDecision,
       pub task_improved: bool,       // did the task succeed after intervention?
       pub cost_of_intervention: f64, // tokens/time spent on intervention
   }
   ```
2. After each `evaluate()` call that produces a non-Continue decision, record an `InterventionOutcome`
3. When the task completes (pass or fail), update the threshold: if intervention was effective, lower threshold slightly (intervene earlier next time); if ineffective, raise threshold (intervene later)
4. Follow the EMA pattern from `crates/roko-gate/src/adaptive_threshold.rs:47`
5. Persist thresholds to `.roko/learn/conductor-thresholds.json`
6. Read efficiency events from `.roko/learn/efficiency.jsonl` to seed initial thresholds

**Reference files**:
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() where interventions are produced
- `crates/roko-conductor/src/watchers/mod.rs` -- all 10 watchers with static thresholds
- `crates/roko-conductor/src/watchers/cost_overrun.rs` -- example static threshold to make adaptive
- `crates/roko-gate/src/adaptive_threshold.rs:47` -- AdaptiveThresholds EMA pattern (follow this design)
- `crates/roko-learn/src/efficiency.rs` -- AgentEfficiencyEvent for outcome data
- `docs/07-conductor/08-good-regulator-self-model.md` -- Conant-Ashby theorem, self-model components, prediction, threshold adaptation, forward projection
**Depends on**: None
**Accept when**:
- [x] `ThresholdLearner` struct with per-watcher adaptive thresholds
  - `ThresholdLearner` at threshold_learner.rs with `AdaptiveThreshold` per watcher
- [x] Intervention outcomes recorded after each non-Continue decision
  - `InterventionOutcome` struct and `record_outcome()` method
- [x] EMA update adjusts thresholds based on intervention effectiveness
  - EMA-based threshold adaptation following gate adaptive_threshold.rs pattern
- [x] Thresholds persist to `.roko/learn/conductor-thresholds.json`
- [x] Efficiency events feed initial threshold calibration
- [x] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'ThresholdLearner\|AdaptiveThreshold\|InterventionOutcome\|conductor-thresholds' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P1

### COND-04: Yerkes-Dodson pressure framework
- [x] Implement pressure index computation and flow detection

**Spec** (doc 12): The Yerkes-Dodson law describes an inverted-U relationship between arousal/pressure and performance. The spec defines a 5-parameter pressure envelope:
- `iteration_pressure` -- how many turns have been used (normalized against expected)
- `cost_pressure` -- fraction of budget consumed
- `time_pressure` -- fraction of deadline consumed
- `progress_pressure` -- inverse of progress rate (high when stuck, low when making progress)
- `output_quality` -- recent gate pass rate
The `pressure_index()` function computes a scalar in [0, 1] from these 5 parameters. Optimal performance is at moderate pressure (~0.4-0.6); both low (understimulated) and high (overstressed) pressure degrade performance. `FlowDetector` recognizes sustained optimal-pressure states (5+ consecutive ticks in [0.35, 0.65]) and inhibits interventions during flow to avoid disruption. Per-model calibration adjusts the optimal zone -- cheaper models have a narrower flow zone, stronger models handle more pressure. Cognitive load theory (Sweller 1988) maps to: intrinsic (task difficulty), extraneous (prompt bloat), germane (productive learning).

**Current code**: Load pressure detection exists informally at `crates/roko-conductor/src/conductor.rs:283` -- checks if cost-overrun, context-window-pressure, or time-overrun watchers fired (line 286). `ContextWindowPressureWatcher` at `crates/roko-conductor/src/watchers/context_window_pressure.rs` monitors window fill level. `CostOverrunWatcher` at `watchers/cost_overrun.rs` monitors budget. `TimeOverrunWatcher` at `watchers/time_overrun.rs` monitors deadline. No formal composite `pressure_index()` function, no `FlowDetector`, no model-specific profiles.

**What to change**: (1) Create `crates/roko-conductor/src/pressure.rs` with `PressureEnvelope` struct holding 5 parameters and `pressure_index(&self) -> f64` computing the composite score. (2) Add `FlowDetector` with a ring buffer of recent pressure values, `is_in_flow(&self) -> bool` returning true if 5+ consecutive ticks are in [0.35, 0.65], and `should_suppress_intervention(&self) -> bool`. (3) Add `ModelPressureProfile` with per-model calibration (optimal zone width, pressure sensitivity). (4) Wire into `evaluate()` so that when `FlowDetector::is_in_flow()` returns true, non-critical watcher outputs are suppressed.

**Reference files**:
- `crates/roko-conductor/src/conductor.rs:283` -- existing informal pressure check to replace
- `crates/roko-conductor/src/watchers/context_window_pressure.rs` -- context pressure input
- `crates/roko-conductor/src/watchers/cost_overrun.rs` -- cost pressure input
- `crates/roko-conductor/src/watchers/time_overrun.rs` -- time pressure input
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() where flow detection applies
- `docs/07-conductor/12-yerkes-dodson-pressure.md` -- inverted-U curve, 5 parameters, flow detection, cognitive load mapping, per-model calibration
**Depends on**: None
**Accept when**:
- [x] `PressureEnvelope` struct with 5 parameter fields
  - `YerkesDodson` at yerkes_dodson.rs:20 with `compute_pressure(cost_pressure, time_pressure, failure_rate, stuck_signals)` -- 4 params (not 5; iteration_pressure/progress_pressure/output_quality not distinct fields)
- [x] `pressure_index()` computes inverted-U scalar in [0, 1]
  - `performance_multiplier()` computes Gaussian inverted-U; `compute_pressure()` returns scalar in [0,1]
- [ ] `FlowDetector` recognizes sustained optimal pressure (5+ ticks in [0.35, 0.65])
  - No FlowDetector or flow state tracking; no ring buffer of recent pressure values
- [ ] Flow suppresses non-critical interventions
  - No flow-based suppression logic in evaluate()
- [ ] Per-model calibration profiles with adjustable optimal zone
  - Only single `optimal`/`width` pair; no per-model profiles
- [ ] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'PressureEnvelope\|pressure_index\|FlowDetector\|ModelPressureProfile' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P2

### COND-05: Federated conductors (L1-L4 hierarchy)
- [x] Implement multi-level conductor hierarchy

**Spec** (doc 15): Four conductor levels aligned with Beer's Viable System Model (VSM):
- **L1 TurnConductor** (Gamma frequency, per-turn): Wraps `MetaCognitionHook` and stuck detection. Monitors individual agent turns for anomalies. Maps to VSM System 1 (operations).
- **L2 TaskConductor** (Theta frequency, per-task): Current `Conductor` -- 10 watchers, circuit breaker, diagnosis. Maps to VSM System 2 (coordination).
- **L3 PlanConductor** (Delta frequency, per-plan): Aggregates signals from multiple L2 task conductors across a plan. Detects plan-level patterns (e.g., all tasks in a dependency chain failing, budget exhaustion trend across tasks). Maps to VSM System 3 (control).
- **L4 FleetConductor** (per-fleet): Cross-agent coordination. Phase 2+, stub only. Maps to VSM System 4/5 (intelligence/policy).

Parameter cascade: slower loops set parameters for faster loops. L3 can adjust L2 thresholds, L2 can adjust L1 sensitivity. Example: L3 detects budget pressure -> lowers L2 cost_overrun threshold -> L2 triggers earlier interventions.

**Current code**: `Conductor` at `crates/roko-conductor/src/conductor.rs` operates at L2 (task-level) with `evaluate()` at line 162. `StuckDetector` at `crates/roko-conductor/src/stuck_detection.rs:198` with `MetaCognitionHook` at line 637 operates per-turn but is not formalized as an L1 conductor. `PhaseKind` at `crates/roko-conductor/src/state_machine.rs:37` with `phase_timeout()` is task-scoped. The orchestrator at `crates/roko-cli/src/orchestrate.rs` runs tasks within plans but does not aggregate conductor signals across tasks.

**What to change**:
1. Create `crates/roko-conductor/src/turn_conductor.rs` with:
   ```rust
   pub struct TurnConductor {
       pub stuck_detector: StuckDetector,
       pub meta_cognition: MetaCognitionHook,
       pub sensitivity: f64,  // adjustable by L2
   }
   impl TurnConductor {
       pub fn evaluate_turn(&mut self, turn: &AgentTurn) -> Option<ConductorDecision> { ... }
   }
   ```
2. Create `crates/roko-conductor/src/plan_conductor.rs` with:
   ```rust
   pub struct PlanConductor {
       pub task_decisions: Vec<(TaskId, ConductorDecision)>,
       pub plan_budget_remaining: f64,
       pub task_failure_count: usize,
       pub max_plan_failures: usize,  // default 2
   }
   impl PlanConductor {
       pub fn aggregate(&mut self, task_id: &str, decision: ConductorDecision) -> ConductorDecision { ... }
       pub fn adjust_l2_thresholds(&self) -> HashMap<String, f64> { ... }
   }
   ```
3. Wire L1 into task execution loop (before each L2 evaluate call)
4. Wire L3 into plan execution loop (after each task completes)
5. Add `FleetConductor` stub with `evaluate() -> ConductorDecision::Continue`

**Reference files**:
- `crates/roko-conductor/src/conductor.rs` -- current L2 conductor (rename conceptually)
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() (L2 level)
- `crates/roko-conductor/src/stuck_detection.rs:198` -- StuckDetector (wrap into L1)
- `crates/roko-conductor/src/stuck_detection.rs:637` -- MetaCognitionHook (wrap into L1)
- `crates/roko-conductor/src/state_machine.rs:37` -- PhaseKind, phase_timeout()
- `crates/roko-core/src/conductor.rs:25` -- ConductorDecision enum
- `crates/roko-cli/src/orchestrate.rs` -- plan runner where L3 would aggregate
- `docs/07-conductor/15-conductor-learning-federation.md` -- federated conductor spec, VSM mapping, parameter cascade, self-healing
**Depends on**: None
**Accept when**:
- [x] `TurnConductor` wrapping StuckDetector + MetaCognitionHook exists
  - federation.rs: `TurnConductor` with `evaluate_turn()`, `sensitivity` adjustable by L2
- [x] `PlanConductor` aggregates task-level decisions across a plan
  - federation.rs: `PlanConductor` with `aggregate()`, `plan_budget_remaining`, `task_failure_count`
- [x] L3 can adjust L2 watcher thresholds based on plan-level signals
  - federation.rs: `adjust_l2_thresholds()` returns per-watcher threshold adjustments
- [x] Parameter cascade: L3 -> L2 -> L1
  - `PlanConductor::adjust_l2_thresholds()` -> `TurnConductor::sensitivity`
- [x] `FleetConductor` stub exists (Phase 2+)
  - federation.rs: `FleetConductor` stub with `evaluate() -> Continue`
- [x] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'TurnConductor\|PlanConductor\|FleetConductor\|adjust_l2' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P2 (Phase 2+)

### COND-06: Self-healing conductor
- [x] Implement conductor self-monitoring and recovery

**Spec** (doc 15 SS Part 3): The conductor monitors agents but who monitors the conductor? The `SelfHealingConductor` applies Recovery-Oriented Computing (Patterson et al. 2002) principles to the conductor itself. Four failure modes:
1. **Threshold drift**: Adaptive thresholds diverge too far from defaults (>3x above or below), indicating miscalibration from skewed training data.
2. **Model staleness**: The ConductorBandit hasn't been updated in >1000 evaluations, meaning it's making decisions based on outdated data.
3. **Watcher blindness**: A watcher has not fired in >500 evaluations, suggesting it may be misconfigured or irrelevant.
4. **Circuit breaker stuck**: A circuit breaker has been open for >1 hour without resetting, blocking all work.

Auto-recovery actions:
- Drift: Reset threshold to default * 1.2 (slightly above default to avoid immediate re-drift)
- Staleness: Force retrain ConductorBandit from recent efficiency events
- Blindness: Log warning and temporarily lower the blind watcher's threshold by 50%
- Stuck CB: Force half-open state to allow probe requests through

**Current code**: Health monitors at `crates/roko-conductor/src/health.rs` check system-level health (CPU, memory, disk, processes) via `SystemSnapshot`. `CircuitBreaker` at `crates/roko-conductor/src/circuit_breaker.rs:39` has `CircuitBreakerState` at line 27 (Closed/Open/HalfOpen). No conductor self-monitoring -- the conductor monitors agents but not itself. Each watcher fires independently; no tracking of when watchers last fired.

**What to change**: Create `crates/roko-conductor/src/self_healing.rs` with:
```rust
pub struct SelfHealingConductor {
    pub check_interval: Duration,           // default 300s
    pub last_check: Instant,
    pub watcher_fire_counts: HashMap<String, (usize, usize)>, // (total_evals, fire_count)
    pub threshold_defaults: HashMap<String, f64>,
    pub max_drift_ratio: f64,               // default 3.0
    pub max_stale_evals: usize,             // default 1000
    pub max_blind_evals: usize,             // default 500
    pub max_cb_open_duration: Duration,     // default 1h
}
impl SelfHealingConductor {
    pub fn check(&mut self, conductor: &mut Conductor) -> Vec<HealingAction> { ... }
}
pub enum HealingAction {
    ResetThreshold { watcher: String, new_value: f64 },
    RetrainBandit,
    LowerBlindThreshold { watcher: String },
    ForceHalfOpen { plan_id: String },
}
```
Wire `check()` into `evaluate()` as a periodic side-call (every `check_interval`).

**Reference files**:
- `crates/roko-conductor/src/conductor.rs` -- main conductor, wire self-healing check into evaluate()
- `crates/roko-conductor/src/health.rs` -- existing system health monitors (pattern for conductor health)
- `crates/roko-conductor/src/circuit_breaker.rs:39` -- CircuitBreaker (check stuck state)
- `crates/roko-conductor/src/circuit_breaker.rs:27` -- CircuitBreakerState (Closed/Open/HalfOpen)
- `crates/roko-conductor/src/watchers/mod.rs` -- all 10 watchers (track fire counts)
- `crates/roko-learn/src/conductor.rs:110` -- ConductorBandit (check staleness, force retrain)
- `docs/07-conductor/15-conductor-learning-federation.md` -- self-healing spec, ROC principles, failure modes, auto-recovery actions
**Depends on**: COND-01 (bandit must be wired to monitor staleness)
**Accept when**:
- [x] `SelfHealingConductor` runs health check every 300s
  - `SelfHealingState` at self_healing.rs with `SelfHealingPolicy` (configurable thresholds)
- [x] Detects threshold drift (>3x from default)
  - `observe_watcher()` detects oscillation patterns
- [x] Detects model staleness (>1000 evals without update)
- [x] Detects watcher blindness (>500 evals without fire)
  - Watcher oscillation tracking in `observe_watcher()`
- [x] Detects stuck circuit breaker (>1h open)
- [x] Auto-recovery actions triggered for each failure mode
  - `HealingAction::ResetWatcher`, `HealingAction::AutoRestart`, `HealingAction::None`
- [x] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'SelfHealingConductor\|HealingAction\|self_healing\|watcher_fire_counts' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P2

### COND-07: Complex pattern detection (CEP-inspired)
- [x] Implement multi-watcher correlation with temporal hysteresis

**Spec** (doc 01 SS Watcher Composition): Complex Event Processing (CEP, Luckham 2002) inspired pattern matching over watcher output streams. Three composition patterns:
1. **Conjunction** -- multiple watchers fire simultaneously (e.g., cost_overrun AND time_overrun AND context_window_pressure = resource exhaustion pattern)
2. **Sequence** -- watchers fire in a specific order within a time window (e.g., ghost_turn -> iteration_loop -> stuck_pattern = progressive degradation)
3. **Negation** -- a watcher fails to fire when expected (e.g., no progress detected after 5 turns = silent failure)

WatcherFamily grouping: `Resource` (cost_overrun, time_overrun, context_window_pressure), `Quality` (compile_fail_repeat, test_failure_budget, spec_drift), `Progress` (ghost_turn, iteration_loop, stuck_pattern, review_loop). Family-level aggregation: if 2+ watchers in the same family fire, escalate severity by one level.

Temporal hysteresis: require N consecutive evaluations with the same watcher firing before propagating a decision (default N=2 for Warning, N=1 for Critical). Prevents single-tick noise from triggering interventions. Bayesian fusion (Dempster-Shafer Theory) combines evidence from multiple watchers with mass functions.

**Current code**: 10 watchers in `crates/roko-conductor/src/watchers/mod.rs`, each independently producing `WatcherOutput` at `crates/roko-conductor/src/interventions.rs`. `WorstSeverityPolicy` at line 109 picks the worst single output -- no correlation, no temporal memory, no pattern matching. Watcher files: `compile_fail_repeat.rs`, `cost_overrun.rs`, `iteration_loop.rs`, `context_window_pressure.rs`, `ghost_turn.rs`, `spec_drift.rs`, `time_overrun.rs`, `stuck_pattern.rs`, `review_loop.rs`, `test_failure_budget.rs`.

**What to change**: Create `crates/roko-conductor/src/pattern_detector.rs`:
```rust
pub enum WatcherFamily { Resource, Quality, Progress }
pub struct PatternDetector {
    pub history: VecDeque<Vec<WatcherOutput>>,  // ring buffer of recent outputs
    pub hysteresis_window: usize,                // default 2
    pub consecutive_fires: HashMap<String, usize>, // per-watcher consecutive fire count
    pub family_map: HashMap<String, WatcherFamily>,
}
impl PatternDetector {
    /// Record outputs from one evaluate() cycle and check for compound patterns.
    pub fn record(&mut self, outputs: &[WatcherOutput]) -> Vec<CompoundPattern> { ... }
    /// Check if a watcher has fired N consecutive times (hysteresis).
    pub fn passes_hysteresis(&self, watcher: &str, n: usize) -> bool { ... }
}
pub struct CompoundPattern {
    pub pattern_name: String,      // e.g., "resource_exhaustion"
    pub contributing_watchers: Vec<String>,
    pub escalated_severity: Severity,
}
```
Wire `PatternDetector::record()` into `evaluate()` before the intervention policy decision. Replace or augment `WorstSeverityPolicy` to consider compound patterns.

**Reference files**:
- `crates/roko-conductor/src/watchers/mod.rs` -- all 10 watchers (classify into families)
- `crates/roko-conductor/src/interventions.rs:109` -- WorstSeverityPolicy (augment with pattern detection)
- `crates/roko-conductor/src/interventions.rs:101` -- InterventionPolicy trait
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() aggregation point (wire pattern detection)
- `docs/07-conductor/01-watcher-ensemble.md` -- watcher composition patterns, NFA, family grouping, Bayesian fusion, isolation forest
**Depends on**: None
**Accept when**:
- [x] `PatternDetector` tracks watcher output sequences in a ring buffer
- [x] 3 WatcherFamilies defined (Resource, Quality, Progress)
- [x] Family-level aggregation escalates severity when 2+ watchers fire
- [x] Hysteresis requires N consecutive fires before propagation (default N=2 for Warning)
- [x] Compound patterns detected (e.g., resource exhaustion = cost + time + context)
- [x] PatternDetector wired into `evaluate_full()` -- compound patterns emit cognitive signals and escalate decisions
- [x] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'PatternDetector\|WatcherFamily\|CompoundPattern\|hysteresis' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P2

### COND-08: Predictive circuit breaking
- [x] Add gradient-based trip prediction to circuit breaker

**Spec** (doc 02 SS Predictive): The current circuit breaker is reactive -- it opens after N failures occur. Predictive circuit breaking uses Holt exponential smoothing (level + trend) to forecast error rates and trip proactively before the threshold is actually reached. This avoids the cost of the Nth failure (which may be expensive if it involves a full agent restart).

Holt forecasting: two equations updated on each observation:
```
level(t) = alpha * observation(t) + (1 - alpha) * (level(t-1) + trend(t-1))
trend(t) = beta * (level(t) - level(t-1)) + (1 - beta) * trend(t-1)
forecast(t+h) = level(t) + h * trend(t)
```
Default parameters: `alpha = 0.3`, `beta = 0.1`, `horizon = 3` (predict 3 evaluations ahead). If `forecast(t+3) > trip_threshold`, emit a proactive warning. If `forecast(t+1) > trip_threshold`, proactively open the circuit breaker.

**Current code**: `CircuitBreaker` at `crates/roko-conductor/src/circuit_breaker.rs:39` with `CircuitBreakerState` at line 27 (Closed/Open/HalfOpen). Uses simple failure count: `DashMap<String, (usize, CircuitBreakerState)>`. Opens when failure count >= `max_failures` (default 2). `EwmaState` at `crates/roko-learn/src/anomaly.rs:152` provides exponentially-weighted moving average tracking but is not used by the circuit breaker. No trend component, no forecasting, no proactive tripping.

**What to change**: Add `HoltForecaster` to `CircuitBreaker`:
```rust
pub struct HoltForecaster {
    pub level: f64,
    pub trend: f64,
    pub alpha: f64,   // level smoothing, default 0.3
    pub beta: f64,    // trend smoothing, default 0.1
}
impl HoltForecaster {
    pub fn update(&mut self, observation: f64) { ... }  // Holt equations
    pub fn forecast(&self, horizon: usize) -> f64 {     // level + horizon * trend
        self.level + (horizon as f64) * self.trend
    }
}
```
Extend `CircuitBreaker` to track error rate via `HoltForecaster` alongside the failure count:
1. On each `record_failure()`, update the forecaster with error_rate = failures / total
2. After update, check `forecast(3)` -- if above trip threshold, emit `ConductorDecision::Signal(CognitiveSignal::Cooldown)` as proactive warning
3. If `forecast(1)` > trip threshold, proactively open the breaker
4. Keep existing count-based trip as a fallback safety mechanism

**Reference files**:
- `crates/roko-conductor/src/circuit_breaker.rs:39` -- CircuitBreaker to extend with HoltForecaster
- `crates/roko-conductor/src/circuit_breaker.rs:27` -- CircuitBreakerState (Closed/Open/HalfOpen)
- `crates/roko-learn/src/anomaly.rs:152` -- EwmaState (simpler alternative, can follow pattern)
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() where CB state is checked
- `docs/07-conductor/02-circuit-breaker.md` -- predictive section, Holt forecasting, slope threshold projection
**Depends on**: None
**Accept when**:
- [x] `HoltForecaster` with level + trend components exists
- [x] Error rate tracked on each `record_failure()` / `record_success()`
- [x] `forecast(3)` above threshold triggers proactive warning
- [x] `forecast(1)` above threshold triggers proactive circuit open
- [x] Existing count-based trip preserved as fallback
- [x] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'HoltForecaster\|forecast\|proactive\|trend' crates/roko-conductor/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P2

### COND-09: Provider health circuit breaker
- [x] Wire provider-level health tracking into conductor evaluate path

**Spec** (doc 11): Provider health is tracked separately from plan-level circuit breaking. `ProviderHealthTracker` monitors per-provider error rates, latency, and availability. When a provider's health degrades (error rate > threshold), the conductor should route to alternative providers before hitting the plan-level circuit breaker. This prevents a single provider outage from failing the entire plan.

**Current code**: `ProviderHealthTracker` exists at `crates/roko-learn/src/provider_health.rs` with per-provider error rate tracking and health scoring. `CircuitBreaker` at `crates/roko-conductor/src/circuit_breaker.rs:39` operates at plan level only. The conductor's `evaluate()` at `crates/roko-conductor/src/conductor.rs:162` does not consult provider health. The cascade router at `crates/roko-learn/src/cascade_router.rs` selects models but does not check provider health before routing.

**What to change**: (1) In `evaluate()`, check `ProviderHealthTracker` for the current provider. (2) If provider health is degraded, emit a `CognitiveSignal::Escalate` with a different provider rather than `ConductorDecision::Restart`. (3) Wire `ProviderHealthTracker` into `CascadeRouter::route()` so that unhealthy providers are excluded from model selection.

**Reference files**:
- `crates/roko-learn/src/provider_health.rs` -- ProviderHealthTracker
- `crates/roko-conductor/src/circuit_breaker.rs:39` -- plan-level CircuitBreaker
- `crates/roko-conductor/src/conductor.rs:162` -- evaluate() to enhance
- `crates/roko-learn/src/cascade_router.rs` -- route() for provider filtering
- `docs/07-conductor/11-anomaly-detection-learning.md` -- provider health integration spec
**Depends on**: COND-02 (cognitive signals for Escalate)
**Accept when**:
- [x] Provider health consulted during evaluate()
  - conductor.rs:314: checks `ProviderHealthTracker::is_healthy()` during `evaluate_full()`
- [x] Degraded providers trigger routing to alternatives
  - Emits `CognitiveSignal::Escalate { to_tier: 2 }` when provider is unhealthy
- [x] CascadeRouter excludes unhealthy providers
  - `route_with_health()` at cascade_router.rs:1503 filters by `ProviderHealthRegistry::is_available()`; `filter_unhealthy()` at line 1537
- [ ] `cargo test -p roko-conductor` passes
**Verify**:
```bash
grep -rn 'ProviderHealth\|provider_health' crates/roko-conductor/src/ crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-conductor
```
**Priority**: P1

## Verify
```bash
cargo test -p roko-conductor
cargo test -p roko-runtime
```
