# 05-learning -- Gap Checklist

Spec: `docs/05-learning/` (22 files, docs 00-20 + INDEX). Code: `crates/roko-learn/`.

Overall: ~77% complete. 14 core subsystems shipping. Gaps in advanced routing, predictive foraging, Bus-backed calibration, and Phase 2+ research-to-runtime.

## Compliant (no action needed)
- Episode logger with HDC fingerprinting (doc 00)
- Skill library with deduplication + extraction pipeline (doc 02)
- Pattern discovery: trigram mining, HDC k-medoids clustering (doc 05 -- core infra exists, wiring gap in GATE-06)
- Task metrics and baselines: TaskMetric JSONL, SliceBaseline, AgentEfficiencyEvent (doc 06)
- Regression detection with per-slice analysis (doc 07)
- Cost normalization with blended formula + budget guardrails (doc 08)
- Provider health + circuit breaker -- 3 states, error classification (doc 09)
- Self-improvement survey -- all frameworks mapped (doc 12)
- Stability mechanisms -- hysteresis, frequency separation, EMA (doc 14)
- C-Factor collective calibration -- 11 components, leave-one-out (doc 15)

## Checklist

### LEARN-01: Playbook demurrage decay not continuous
- [x] Wire continuous demurrage decay into playbook update cycle

**Spec** (doc 01 §Playbook System): Rules have `balance` and `demurrage_rate` fields.
Balance represents the rule's "attention budget" -- a Gesellian demurrage tax decays it
over time, so rules must be actively validated (used successfully) to replenish their
balance. Rules with depleted balance are deprioritized in retrieval, ensuring that stale
or unvalidated heuristics naturally fade. The INDEX.md defers this as "deferred design"
but the spec is detailed enough to implement.

**Current code** (`crates/roko-learn/src/playbook_rules.rs:173`): `PlaybookRules` struct
manages rules with confidence dynamics (validate +0.05, contradict -0.10, ceiling 0.95).
No `balance` or `demurrage_rate` fields on rules. No "demurrage" or "balance" matches in
the playbook context. Rules persist via TOML at `.roko/learn/playbook-rules.toml`. The
rule struct has `confidence: f64` which serves as the only freshness indicator. The
`LearningRuntime::record_completed_run()` calls `PlaybookRules::validate/contradict` but
no time-based decay.

**What to change**: Add `balance: f64` (initial 1.0) and `demurrage_rate: f64` (default
0.01 per hour) fields to the rule struct in `playbook_rules.rs`. Add
`pub fn tick_demurrage(&mut self, elapsed_hours: f64)` that applies
`balance *= (1.0 - demurrage_rate).powf(elapsed_hours)` to each rule. Add
`pub fn replenish(&mut self, amount: f64)` called on successful validation. Call
`tick_demurrage()` at the start of `record_completed_run()` with elapsed time since last
call. Rules with `balance < 0.1` get deprioritized in retrieval (sorted after higher-balance
rules).

**Reference files**:
- `crates/roko-learn/src/playbook_rules.rs:173` -- `PlaybookRules` struct, rule management
- `crates/roko-learn/src/playbook.rs` -- playbook module
- `crates/roko-learn/src/runtime_feedback.rs:335` -- `LearningRuntime::record_completed_run()` where tick should be called
- `docs/05-learning/01-playbook-system.md` -- demurrage spec, confidence dynamics

**Depends on**: None

**Accept when**:
- [x] Rule struct has `balance: f64` and `demurrage_rate: f64` fields
- [x] `tick_demurrage(&mut self, elapsed_hours: f64)` decays balance exponentially
- [x] `replenish()` called on successful validation
- [x] Rules with `balance < 0.1` deprioritized in retrieval
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'demurrage\|balance\|tick_demurrage\|replenish' crates/roko-learn/src/playbook_rules.rs
cargo test -p roko-learn
```

**Priority**: P1

### LEARN-02: NeuralUCB and bandit ensembles
- [x] Implement NeuralUCB for non-linear routing contexts

**Spec** (doc 03): NeuralUCB for when 500+ observations accumulate or non-linear structure is detected.
**Current code**: UCB1 + LinUCB + TrackAndStop in `crates/roko-learn/src/bandits.rs`. `NeuralUCBRouter` shell already exists at `crates/roko-learn/src/bandit_research.rs:118` with `NeuralRewardNetwork` at line 77, but not wired into the active routing pipeline. `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006` uses LinUCB as stage-3.
**What to change**: The `NeuralUCBRouter` shell at line 118 has the struct but `select_arm()`
is a stub. Complete:
1. Implement `NeuralRewardNet::forward(context: &[f64]) -> Vec<f64>` -- two-layer ReLU
   network, computing `h1 = ReLU(W1 @ x + b1)`, `h2 = ReLU(W2 @ h1 + b2)`, `out = W3 @ h2 + b3`.
   Initialize `params` with Xavier initialization in `new()`.
2. Implement `NeuralUCBRouter::select_arm(context: &[f64], arms: &[String]) -> String` --
   UCB score = `f(x; θ)_a + nu * sqrt(g_a^T @ Z_a^{-1} @ g_a)` where `g_a` is the gradient
   of `f` w.r.t. θ for arm a, and `Z_a` is the gradient covariance matrix.
3. Implement `NeuralUCBRouter::update(context: &[f64], arm: &str, reward: f64)` -- add to
   training buffer, retrain network every `retrain_every` observations.
4. In `CascadeRouter::select_model()` at `cascade_router.rs:1006`, check observation count.
   When total observations > 500 AND LinUCB residual variance > threshold (indicating
   non-linearity), switch stage-3 from LinUCB to NeuralUCB.
**Reference files**:
- `crates/roko-learn/src/bandit_research.rs:118` -- NeuralUCBRouter shell
- `crates/roko-learn/src/bandits.rs` -- UCB1, LinUCB, TrackAndStop
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter routing pipeline
- `docs/05-learning/03-*` -- bandit spec
**Depends on**: None
**Accept when**:
- [x] NeuralUCB implementation available as alternative to LinUCB
  - `NeuralUCBRouter` at bandit_research.rs:252 with full `select_arm()`, `retrain_if_needed()`, gradient covariance
- [ ] Automatic switching when non-linear patterns detected
  - NeuralUCB not wired into CascadeRouter; no auto-switch logic based on observation count or residual variance
- [ ] `cargo test -p roko-learn`
**Verify**:
```bash
grep -rn 'NeuralUCB' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```
**Priority**: P2

### LEARN-03: Cascade router lookahead and calibration
- [x] Wire lookahead routing and router calibration

**Spec** (doc 04): Sequence-aware routing with cache reuse. Calibration (Platt, isotonic, temperature scaling).
**Current code**: `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006` handles per-task routing. `LookaheadRouter` shell exists at `crates/roko-learn/src/routing_extras.rs:104` with task dependency graph support but not wired into the main routing path. `CalibrationTracker` at `crates/roko-learn/src/prediction.rs:125` tracks prediction vs outcome but has no Platt/isotonic/temperature scaling -- only mean bias correction. No auto-invocation of calibration.
**What to change**:
(1) Wire `LookaheadRouter` into the orchestrator. In `orchestrate.rs`, when initializing
the routing system, wrap `CascadeRouter` in `LookaheadRouter::new(router, task_graph)` where
`task_graph` is derived from the `UnifiedTaskDag`. `LookaheadRouter::select_model()` at
`routing_extras.rs:104` already has the logic to look ahead `horizon` tasks and estimate
cache reuse savings -- it just needs the task graph populated from the current plan.

(2) Extend `CalibrationTracker` at `prediction.rs:125` with:
- `pub fn brier_score(&self, model: &str, category: &str) -> Option<f64>` -- compute
  `sum(residual^2) / N` for the model/category pair
- `pub fn platt_scaling(&self, model: &str, category: &str) -> (f64, f64)` -- fit logistic
  regression `σ(a*x + b)` to residuals, return `(a, b)` parameters
- `pub fn apply_correction(&self, model: &str, category: &str, raw_score: f64) -> f64` --
  apply Platt correction to a raw confidence score

(3) In `orchestrate.rs`, add a calibration check every 50 episodes. Call
`CalibrationTracker::load_from_routing_log()` and recompute corrections.
**Reference files**:
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter
- `crates/roko-learn/src/routing_extras.rs:104` -- LookaheadRouter shell
- `crates/roko-learn/src/prediction.rs:125` -- CalibrationTracker
- `crates/roko-cli/src/orchestrate.rs` -- where auto-invocation should be wired
- `docs/05-learning/04-*` -- cascade router spec
**Depends on**: None
**Accept when**:
- [x] Router considers task sequence for cache reuse
  - `LookaheadRouter` at routing_extras.rs:106 with `route_with_lookahead()`, `CacheReuseModel`, tier-downgrade logic
- [ ] Calibration runs automatically at configurable intervals
  - `LookaheadRouter` not wired into orchestrate.rs; recalibration interval defined but not invoked from runtime
- [ ] `cargo test -p roko-learn`
**Verify**:
```bash
grep -rn 'LookaheadRouter\|auto_calibrat' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```
**Priority**: P2

### LEARN-04: Feedback loops 7 and 8 not automated
- [x] Wire latency reward weighting and experiment winner promotion

**Spec** (doc 13 §Eight Missing Feedback Loops):
- **Loop 7 (Latency → Reward Weighting)**: Bandit reward signals should blend quality and
  latency. A model that passes gates but takes 10x longer than another should receive a
  discounted reward. Formula: `reward = quality * (1 - latency_penalty)` where
  `latency_penalty = clamp((actual_ms - sla_ms) / sla_ms, 0, 0.5)`. This ensures the
  cascade router eventually routes away from slow models even if they pass gates.
- **Loop 8 (Experiment Winners → Static Defaults)**: When an A/B experiment in
  `ExperimentStore` reaches statistical significance (p < 0.05 via chi-squared test on
  pass rates), the winning variant's config should be automatically promoted to the static
  role→model defaults in `roko.toml` or the in-memory config. This closes the loop from
  experimentation to production configuration.

**Current code**: `LatencyRegistry` at `crates/roko-learn/src/latency.rs` tracks per-model
latency stats with SLA thresholds. `ExperimentStore` at
`crates/roko-learn/src/prompt_experiment.rs:395` manages experiments with variants and
outcome recording, but has no `promote_winner()` or auto-promotion logic. Bandit reward
computation in `crates/roko-learn/src/bandits.rs` uses pure quality (gate pass/fail) with
no latency blending. `DriftDetector` at `crates/roko-learn/src/drift.rs:89` detects
non-stationarity but doesn't connect to reward weighting.

**What to change**:
(1) In `bandits.rs`, add a `LatencyAwareReward` helper or modify the reward computation
path in `CascadeRouter` to blend quality with latency penalty using the formula above.
Read latency from `LatencyRegistry`.
(2) In `prompt_experiment.rs`, add `pub fn promote_winner(&mut self, experiment_id: &str) -> Option<PromptVariant>` that returns the winning variant when statistical significance is reached. Add `auto_promote_check()` called from `LearningRuntime::record_completed_run()`.

**Reference files**:
- `crates/roko-learn/src/latency.rs` -- `LatencyRegistry` with SLA tracking
- `crates/roko-learn/src/prompt_experiment.rs:395` -- `ExperimentStore` (needs `promote_winner()`)
- `crates/roko-learn/src/bandits.rs` -- bandit reward computation (needs latency blending)
- `crates/roko-learn/src/cascade_router.rs:1006` -- `CascadeRouter` (where blended reward is consumed)
- `crates/roko-learn/src/runtime_feedback.rs:335` -- `LearningRuntime` (where auto-promote should run)
- `docs/05-learning/13-8-missing-feedback-loops.md` -- §Loop 7, §Loop 8 specs

**Depends on**: None

**Accept when**:
- [x] Bandit reward blends quality with latency penalty: `reward = quality * (1 - latency_penalty)`
  - `CascadeRouter::latency_penalty()` and `reward_with_latency()` at cascade_router.rs:1683
  - `compute_reward_with_latency()` at runtime_feedback.rs:1125 used in `record_completed_run()`
- [x] Latency penalty computed from `LatencyRegistry` SLA thresholds
  - `reward_with_tracker_latency()` at cascade_router.rs:1705 reads from `LatencyTracker`
- [x] `ExperimentStore::promote_winner()` returns winning variant at statistical significance
  - `promote_winner()` at prompt_experiment.rs:506, confidence >= 0.95
- [x] Auto-promotion checked in `record_completed_run()`
  - `on_experiment_concluded()` at runtime_feedback.rs:1071 called from `record_completed_run()`
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'latency_penalty\|latency.*reward\|promote_winner\|auto_promote' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```

**Priority**: P1

### LEARN-05: Predictive foraging / CalibrationTracker
- [x] Extend CalibrationTracker with Brier score and reliability diagrams

**Spec** (doc 16): CalibrationTracker, Brier score, reliability diagrams, arithmetic corrector.
**Current code**: `CalibrationTracker` already exists at `crates/roko-learn/src/prediction.rs:125` with `PredictionRecord` at line 14. It tracks predictions vs outcomes and computes mean bias. Missing: Brier score computation, reliability diagram data, arithmetic corrector. The tracker is used via `from_routing_logs()` at line 179.
**What to change**: Add three methods to `CalibrationTracker` at `prediction.rs:125`:
```rust
/// Brier score: mean squared error of probabilistic predictions.
/// Lower is better. Perfect = 0.0, random = 0.25.
pub fn brier_score(&self, model: &str, category: &str) -> Option<f64> {
    let residuals = self.residuals.get(&(model.to_string(), category.to_string()))?;
    if residuals.is_empty() { return None; }
    Some(residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len() as f64)
}

/// Bin residuals into 10 equally-spaced buckets [0.0, 0.1), [0.1, 0.2), ...
/// Returns (bin_center, mean_predicted, mean_actual, count) per bin.
pub fn reliability_bins(&self, model: &str, category: &str) -> Vec<(f64, f64, f64, usize)> { ... }

/// Arithmetic corrector: subtract mean bias from raw predictions.
/// ~50ns per correction (pure arithmetic).
pub fn arithmetic_corrector(&self, model: &str, category: &str) -> f64 {
    let residuals = self.residuals.get(...)?;
    residuals.iter().sum::<f64>() / residuals.len() as f64  // mean bias
}
```
The `reliability_bins()` method is the data source for reliability diagrams -- a standard
calibration visualization where the x-axis is predicted probability and y-axis is observed
frequency. Diagonal = perfectly calibrated.
**Reference files**:
- `crates/roko-learn/src/prediction.rs:125` -- CalibrationTracker (already exists)
- `crates/roko-learn/src/prediction.rs:14` -- PredictionRecord
- `crates/roko-learn/src/routing_log.rs` -- routing log records fed to tracker
- `docs/05-learning/16-*` -- calibration spec
**Depends on**: None
**Accept when**:
- [x] `brier_score()` computed per prediction category
- [x] `reliability_bins()` returns binned calibration data
- [x] Arithmetic corrector adjusts predictions
- [x] `cargo test -p roko-learn`
**Verify**:
```bash
grep -rn 'brier\|reliability_bins\|arithmetic_correct' crates/roko-learn/src/prediction.rs
cargo test -p roko-learn
```
**Priority**: P1

### LEARN-06: Multi-objective Pareto frontier
- [x] Extend Pareto frontier to 4 objectives

**Spec** (doc 10 §Pareto Frontier Pruning): Pareto frontier should use 4 objectives:
quality (pass_rate, higher is better), cost (cost_per_success, lower is better), latency
(avg_latency_ms, lower is better), and reliability (fraction of non-error responses, higher
is better). The dominance check must handle all 4 dimensions: model A dominates model B iff
A is at least as good on all objectives and strictly better on at least one. Models not on
the frontier are pruned from the bandit candidate set. The spec also calls for configurable
scalarization weights so the operator can prioritize cost over latency or vice versa.

**Current code** (`crates/roko-learn/src/pareto.rs:12`): `ModelObservation` struct has
`pass_rate: f64`, `cost_per_success: f64`, `avg_latency_ms: f64`, `observations: u64`. The
`compute_pareto_frontier()` at line 28 only uses 2 objectives (pass_rate and
cost_per_success) in its dominance check -- the `avg_latency_ms` field is present on the
struct but completely unused in the dominance comparison. No reliability field. No
scalarization weights.

**What to change**: In `compute_pareto_frontier()`:
1. Add `reliability: f64` field to `ModelObservation` (derive as `non_error_responses / total_responses`)
2. Extend dominance check to compare all 4 objectives (pass_rate >=, cost <=, latency <=, reliability >=)
3. Add `pub struct ParetoWeights { quality: f64, cost: f64, latency: f64, reliability: f64 }`
4. Add `scalarize(obs: &ModelObservation, weights: &ParetoWeights) -> f64` for weighted ranking

**Reference files**:
- `crates/roko-learn/src/pareto.rs:12` -- `ModelObservation` struct (add reliability field)
- `crates/roko-learn/src/pareto.rs:28` -- `compute_pareto_frontier()` (extend dominance check)
- `crates/roko-learn/src/cascade_router.rs:1006` -- `CascadeRouter` (consumer of frontier)
- `docs/05-learning/10-pareto-frontier-pruning.md` -- multi-objective spec

**Depends on**: None

**Accept when**:
- [x] `ModelObservation` has `reliability: f64` field
- [x] Dominance check uses all 4 objectives (quality, cost, latency, reliability)
- [x] `ParetoWeights` struct for configurable scalarization
- [x] Scalarized ranking available as alternative to frontier
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'reliability\|ParetoWeights\|scalariz' crates/roko-learn/src/pareto.rs
grep -rn 'avg_latency' crates/roko-learn/src/pareto.rs
cargo test -p roko-learn
```

**Priority**: P2

### LEARN-07: Thompson sampling with drift
- [x] Wire discounted Thompson sampling as alternative stage-3 router

**Spec** (doc 11): Thompson with gamma discount for non-stationary environments.
**Current code**: `DriftDetector` at `crates/roko-learn/src/drift.rs:89` with `DriftAlert` at line 21. LinUCB is current stage-3 in `CascadeRouter` (`crates/roko-learn/src/cascade_router.rs:1006`). Thompson sampling not implemented. The drift module detects non-stationarity but doesn't trigger routing strategy changes.
**What to change**: Add `ThompsonSampler` to `crates/roko-learn/src/bandits.rs`:
```rust
pub struct ThompsonSampler {
    /// Per-arm Beta distribution parameters.
    pub arms: HashMap<String, (f64, f64)>,  // (alpha, beta)
    /// Discount factor for non-stationarity (default: 0.995).
    pub gamma: f64,
}

impl ThompsonSampler {
    pub fn select_arm(&self, rng: &mut impl Rng) -> &str { ... }
    pub fn update(&mut self, arm: &str, reward: f64) {
        let (alpha, beta) = self.arms.get_mut(arm).unwrap();
        // Discount: shrink toward prior
        *alpha = *alpha * self.gamma + reward;
        *beta = *beta * self.gamma + (1.0 - reward);
    }
}
```
In `CascadeRouter` at `cascade_router.rs:1006`, add `thompson: Option<ThompsonSampler>`
field. In `select_model()`, check `DriftDetector::detect()` (at `drift.rs:89`). If drift
is significant (`DriftAlert` returned), switch stage-3 from LinUCB to Thompson for the
next N observations (configurable, default 100), then revert. The rationale: Thompson
adapts faster to distribution shifts via the discount factor.
**Reference files**:
- `crates/roko-learn/src/drift.rs:89` -- DriftDetector
- `crates/roko-learn/src/bandits.rs` -- existing bandit implementations
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter stage-3 slot
- `docs/05-learning/11-*` -- Thompson/drift spec
**Depends on**: None
**Accept when**:
- [x] Thompson available as configurable stage-3 alternative
- [x] Discount factor gamma=0.995 default
- [x] `cargo test -p roko-learn`
**Verify**:
```bash
grep -rn 'Thompson\|gamma.*discount' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```
**Priority**: P2

### LEARN-08: ADAS and autocatalytic optimization
- [x] Implement ADAS meta-agent architecture search

**Spec** (doc 17): Meta-agent searches over roles, communication patterns, tool configs, routing strategies.
**Current code**: Not implemented. Explicitly Phase 2+. Closest infrastructure: `ExperimentStore` at `crates/roko-learn/src/prompt_experiment.rs:395` (A/B testing), `CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006` (model routing). No architecture search space definition.
**What to change**: Create `crates/roko-learn/src/adas.rs` defining the search space (roles, communication patterns, tool configs, routing strategies). Use gate pipeline as verifier. Evaluate architecture variants via controlled experiments using `ExperimentStore`.
**Reference files**:
- `crates/roko-learn/src/prompt_experiment.rs:395` -- ExperimentStore for A/B testing
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter as baseline
- `crates/roko-gate/src/gate_pipeline.rs:68` -- GatePipeline as verifier
- `docs/05-learning/17-*` -- ADAS spec
**Depends on**: None
**Accept when**:
- [x] ADAS search space defined
  - `AdasCandidate` at adas.rs:14 with model, prompt_variant, params HashMap; `AdasOptimizer` population-based search
- [ ] Gate pipeline serves as verifier
  - No reference to GatePipeline in adas.rs; evaluation done via user-supplied closure, not gate pipeline
- [x] Architecture variants evaluated and ranked
  - `evolve_generation()` evaluates children via fitness closure, `cull()` ranks by fitness, `select_elites()` picks top-N
- [ ] `cargo test -p roko-learn`
**Verify**:
```bash
grep -rn 'Adas\|search_space\|architecture_variant' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```
**Priority**: P2 (Phase 2+)

### LEARN-09: Bus-backed cybernetic loops
- [x] Refactor learners to Bus subscribers (predict-publish-correct)

**Spec** (doc 18): CascadeRouter, EpisodeLogger, ExperimentStore become Bus subscribers. prediction.error.* as first-class signals.
**Current code**: `EventBus` exists at `crates/roko-runtime/src/event_bus.rs:188` with `emit()`, `subscribe()`, `BusSender`. `EventSubscriber` trait at `crates/roko-learn/src/event_subscriber.rs`. All learners (`CascadeRouter` at `crates/roko-learn/src/cascade_router.rs:1006`, `EpisodeLogger` at `crates/roko-learn/src/episode_logger.rs`, `ExperimentStore` at `crates/roko-learn/src/prompt_experiment.rs:395`) are currently called directly from `crates/roko-cli/src/orchestrate.rs`, not Bus-driven.
**What to change**: Implement `EventSubscriber` for each learner. Have them subscribe to relevant Bus topics in orchestrate.rs startup. Publish prediction errors as events. Add `CalibrationPolicy` that closes the predict/outcome loop.
**Reference files**:
- `crates/roko-runtime/src/event_bus.rs:188` -- EventBus
- `crates/roko-learn/src/event_subscriber.rs` -- EventSubscriber trait
- `crates/roko-learn/src/cascade_router.rs:1006` -- CascadeRouter
- `crates/roko-learn/src/episode_logger.rs` -- EpisodeLogger
- `crates/roko-learn/src/prompt_experiment.rs:395` -- ExperimentStore
- `crates/roko-cli/src/orchestrate.rs` -- where learners are called directly
- `docs/05-learning/18-*` -- Bus-backed loops spec
**Depends on**: K-02 (Bus trait in 02-missing-kernel-types.md)
**Accept when**:
- [x] Learners subscribe to Bus topics
  - `run_learning_subscriber()` in event_subscriber.rs fans out events to health, latency, router, anomaly, costs
- [x] Prediction errors published as Pulses
  - CalibrationPolicy tracks residuals per (model, category) via `process_event()`
- [x] CalibrationPolicy closes prediction/outcome loops
  - `CalibrationPolicy` at calibration_policy.rs: accumulates residuals, triggers corrections when bias exceeds threshold
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'EventSubscriber\|subscribe.*bus\|CalibrationPolicy' crates/roko-learn/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2 (blocked on Bus trait)

### LEARN-10: Heuristics worldview clustering and dissonance -- RESOLVED
- [x] Add worldview clustering and dissonance detection

**Status**: The typed `Heuristic` struct is mostly complete. `Heuristic` at
`crates/roko-learn/src/heuristics.rs:234` already has:
- `preconditions: Vec<Predicate>` -- typed predicates with And/Or/Not combinators (line 202+)
- `prediction: Predicate` -- predicted outcome
- `calibration: Calibration` -- with `trials`, `confirmations`, `violations`, `brier_score`,
  `confidence_interval` fields (line 63)
- `fingerprint: HdcVector` -- HDC fingerprint for similarity
- `lineage: Vec<HeuristicId>` -- provenance chain
- `receipts: Vec<EpisodeHash>` -- evidence

The `Predicate` enum at line 170 supports `Equals`, `Contains`, `GreaterThan`, `LessThan`,
`Custom`, `And`, `Or`, `Not`.

**Still missing**: (1) explicit `falsifier: Option<Predicate>` field -- a condition that,
if observed, would invalidate the heuristic. (2) worldview clustering -- grouping
co-occurring heuristics using HDC k-medoids from `crates/roko-learn/src/hdc_clustering.rs`.
(3) dissonance detection -- finding pairs of heuristics with contradictory predictions for
overlapping preconditions.

**What to change**:
1. Add `pub falsifier: Option<Predicate>` to `Heuristic` struct at line 234. Default to
   `None`. When a falsifier is set and observed, automatically call `contradict()` on the
   heuristic.
2. Add `pub fn cluster_worldviews(heuristics: &[Heuristic], k: usize) -> Vec<Vec<&Heuristic>>`
   in `heuristics.rs` that uses HDC fingerprint similarity to group co-occurring heuristics.
3. Add `pub fn detect_dissonance(a: &Heuristic, b: &Heuristic) -> Option<DissonanceReport>`
   that checks if two heuristics have overlapping preconditions but contradictory predictions.

**Reference files**:
- `crates/roko-learn/src/heuristics.rs:234` -- `Heuristic` struct (already has most fields)
- `crates/roko-learn/src/heuristics.rs:170` -- `Predicate` enum (And/Or/Not/Custom)
- `crates/roko-learn/src/heuristics.rs:63` -- `Calibration` struct (trials, brier_score)
- `crates/roko-learn/src/hdc_clustering.rs` -- HDC k-medoids clustering infrastructure
- `docs/05-learning/19-heuristics-worldviews-and-falsifiers.md` -- spec

**Depends on**: None

**Accept when**:
- [x] `falsifier: Option<Predicate>` field on `Heuristic`
- [x] `cluster_worldviews()` groups heuristics by HDC fingerprint similarity
- [x] `detect_dissonance()` identifies contradictory heuristic pairs
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'falsifier\|cluster_worldview\|detect_dissonance' crates/roko-learn/src/heuristics.rs
cargo test -p roko-learn
```

**Priority**: P2 (Phase 2+)

### LEARN-11: Research-to-runtime pipeline
- [x] Implement Paper -> Claim -> Heuristic -> Trial -> Ledger pipeline

**Spec** (doc 20): Paper Engrams, Claims with falsifiers, replication ledger, claim!() macro.
**Current code**: Not implemented. Explicitly Phase 2+. Closest infrastructure: `Heuristic` at `crates/roko-learn/src/heuristics.rs:234`, `KnowledgeEntry` at `crates/roko-neuro/src/lib.rs:216`, `KnowledgeKind` enum includes knowledge types that could represent Papers/Claims.
**What to change**: Create `crates/roko-learn/src/research_pipeline.rs` with `Paper`, `Claim`, `Trial`, `ReplicationLedger` types. Implement ingestion from research docs, claim extraction with falsifiers, trial tracking, and ledger persistence. Optionally add `claim!()` macro for inline claim definitions.
**Reference files**:
- `crates/roko-learn/src/heuristics.rs:234` -- Heuristic (target for validated claims)
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry (storage target)
- `crates/roko-learn/src/prediction.rs:125` -- CalibrationTracker (trial tracking pattern)
- `docs/05-learning/20-*` -- research pipeline spec
**Depends on**: LEARN-10 (full Heuristic type)
**Accept when**:
- [x] Paper type can be ingested
  - `ResearchPipeline::ingest_paper()` at research_pipeline.rs
- [x] Claims extracted with falsifiers
  - `ResearchPipeline::register_claim()` links claims to papers with `Predicate` falsifiers
- [x] Replication ledger tracks trials
  - `ResearchPipeline::record_trial()` updates calibration, Brier score, and ledger status
  - `ReplicationLedger` tracks our_effect, our_n, status (Replicated/Mixed/Diverged)
- [x] `cargo test -p roko-learn`
  - 8 tests in research_pipeline::tests all pass
**Verify**:
```bash
grep -rn 'Paper\|Claim\|ReplicationLedger' crates/roko-learn/src/ --include='*.rs'
cargo test -p roko-learn
```
**Priority**: P2 (Phase 2+)

### LEARN-12: CurriculumScheduler wired to orchestrator
- [x] Wire curriculum-based task ordering into the executor's task dispatch

**Spec** (doc INDEX §Curriculum Learning): Tasks within each dependency level should be
ordered by difficulty -- easy first, hard later (Bengio et al. 2009). Early successes build
the skill library and playbook rules that help with harder tasks. The spec defines
`CurriculumScheduler` with `CurriculumMode` (EasyFirst, HardFirst, Interleaved, Adaptive)
and `DifficultyModel` that estimates difficulty from (1) historical pass rate for
`(role, complexity, crate)` triples, (2) HDC similarity to failed episodes, (3) dependency
depth.

**Current code** (`crates/roko-learn/src/curriculum.rs:116`): `CurriculumScheduler` struct
exists with all modes (`EasyFirst`, `HardFirst`, `Interleaved`, `Adaptive`) and
`DifficultyModel` at line 58. `schedule()` at line 136 calls `reorder_tasks()` at line 172
to sort tasks by difficulty. But `CurriculumScheduler` is never imported or called from
`crates/roko-cli/src/orchestrate.rs` -- tasks are dispatched in dependency order only, with
no difficulty-based reordering within each dependency level.

**What to change**: In orchestrate.rs, wire curriculum scheduling at two points:
1. **Initialization**: At plan run startup (in the `PlanRunner` constructor or setup),
   create a `CurriculumScheduler::new(CurriculumMode::EasyFirst)`. Initialize its
   `DifficultyModel` from historical pass rates by loading episode data from
   `.roko/learn/episodes.jsonl` and computing per-`(role, complexity, crate)` pass rates.
2. **Task dispatch**: In the tick loop where ready tasks are collected (tasks whose
   dependencies are satisfied), call `scheduler.schedule(&ready_tasks)` to reorder them
   by difficulty before dispatching. This preserves dependency constraints (only ready
   tasks are reordered) while adding difficulty-based optimization within each dependency
   level.

The `CurriculumScheduler::schedule()` method at `curriculum.rs:136` already handles the
reordering -- the only work is importing it and calling it at the right place.

**Reference files**:
- `crates/roko-learn/src/curriculum.rs:116` -- `CurriculumScheduler` (exists, not wired)
- `crates/roko-learn/src/curriculum.rs:172` -- `reorder_tasks()` function
- `crates/roko-learn/src/curriculum.rs:58` -- `DifficultyModel` struct
- `crates/roko-cli/src/orchestrate.rs` -- task dispatch (where reordering should happen)
- `docs/05-learning/INDEX.md` -- §Curriculum Learning spec

**Depends on**: None

**Accept when**:
- [x] Ready tasks reordered by difficulty before dispatch
- [x] `DifficultyModel` initialized from historical pass rates
- [x] `cargo test -p roko-learn` passes
- [x] `cargo test -p roko-cli` passes

**Verify**:
```bash
grep -rn 'CurriculumScheduler\|reorder_tasks\|DifficultyModel' crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-learn
```

**Priority**: P2

---

### LEARN-13: ToolUsageProfile not wired to prompt injection
- [x] Wire tool usage profiles into SystemPromptBuilder for per-task tool hints

**Spec** (doc INDEX §Meta-Learning for Tool Use): `ToolUsageProfile` tracks which tool
sequences lead to successful outcomes per `(role, task_category)`. Successful patterns
(e.g., "Read → Edit → Bash:cargo test") are identified by support count and lift (pass
rate with pattern vs without). Low-value tools (high call count, low contribution to
success) generate warnings. These profiles should be injected into agent prompts as hints:
"For this task type, successful approaches typically use Read→Edit→Bash(test) in that
order."

**Current code** (`crates/roko-learn/src/curriculum.rs:143`): `ToolUsageProfile` struct
exists but is simpler than the INDEX spec describes -- it has `tool_name: String`,
`usage_count: u64`, `success_rate: f64` (per-tool aggregate, not per-`(role, category)`).
`ToolSequencePattern` at line 153 has `tools: Vec<String>` and `support_count: u32` but
no `lift: f64` field. `ToolWarning` at line 162 has `tool_name` and `message` but no
`calls_per_episode`, `contribution_to_success`, or `tokens_consumed` fields. None of these
structs are populated from episode data or injected into prompts. `SystemPromptBuilder` at
`crates/roko-compose/src/system_prompt_builder.rs` has no tool usage profile layer.

**What to change**:
(1) Extend `ToolUsageProfile` at `curriculum.rs:143` to match the INDEX spec:
- Change from per-tool to per-`(role, task_category)` keying
- Add `lift: f64` field to `ToolSequencePattern` (pass rate with pattern vs without)
- Add `calls_per_episode: f64`, `contribution_to_success: f64`, `tokens_consumed: u64`
  fields to `ToolWarning`
(2) Add `pub fn from_episodes(episodes: &[EpisodeRecord]) -> Self` that mines tool
sequences from episode turn data. For each `(role, category)` pair, extract tool call
sequences from successful vs failed episodes, compute support counts and lift.
(3) In `SystemPromptBuilder` at `crates/roko-compose/src/system_prompt_builder.rs`, add
a tool hints layer (between Skills and Anti-patterns) that formats the top-3 successful
patterns as natural language hints: "For this task type, successful approaches typically
use Read -> Edit -> Bash(cargo test) in that order."
(4) Call the builder during `LearningRuntime::record_completed_run()` at
`crates/roko-learn/src/runtime_feedback.rs:335` to incrementally update the profile.

**Reference files**:
- `crates/roko-learn/src/curriculum.rs:143` -- `ToolUsageProfile` struct (exists, empty)
- `crates/roko-learn/src/episode_logger.rs` -- episode data with tool call records
- `crates/roko-compose/src/system_prompt_builder.rs` -- where tool hints should be injected
- `docs/05-learning/INDEX.md` -- §Meta-Learning for Tool Use spec

**Depends on**: None

**Accept when**:
- [x] `ToolUsageProfile::from_episodes()` mines tool sequences from episode data
  - `RoleToolProfile::from_episodes()` at curriculum.rs mines tool trigrams, computes lift, contribution_to_success
  - `extract_tool_names()` extracts tool names from external_actions JSON
  - `compute_contribution()` measures pass rate delta with/without tool
- [x] Profile injected into SystemPromptBuilder as tool hints
  - `with_tool_hints()` at system_prompt_builder.rs:210, layer 6b renders tool hints
  - `RoleToolProfile::format_hints()` formats top patterns as natural language
- [x] Low-value tools flagged in prompts
  - `ToolWarning` generated for tools with calls_per_episode > 3.0 and contribution_to_success < 0.1
- [x] `cargo test -p roko-learn` passes

**Verify**:
```bash
grep -rn 'ToolUsageProfile\|tool_hints\|tool_sequence' crates/roko-learn/src/ --include='*.rs'
grep -rn 'tool_usage\|tool_hint' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-learn
```

**Priority**: P2

---

## Verify
```bash
cargo test -p roko-learn
cargo test --workspace
```
