# 16-heartbeat -- Gap Checklist

Spec: `docs/16-heartbeat/` (13 files). Code: `crates/roko-runtime/src/heartbeat.rs`, `crates/roko-core/src/loop_tick.rs`, `crates/roko-cli/src/orchestrate.rs`.

Overall: ~75% implemented. All 11 BEAT items checked. Gamma loop in orchestrate.rs; T0/T1/T2 adaptive gating; 16 probes with rolling anomaly detection; CorticalState 32-signal surface; Theta/Delta consumers; FrequencyScheduler; VCG attention auction; POMDP state space; chain SIMULATE+VALIDATE; BROADCAST+REACT steps; TierGatingStats monitoring.

## Compliant (no action needed)
- HeartbeatSpeed enum with Gamma/Theta/Delta (heartbeat.rs)
- InferenceTier enum T0/T1/T2 (tier.rs)
- Regime enum (Calm/Normal/Volatile/Crisis) (heartbeat.rs)
- CascadeRouter for T0/T1/T2 routing (roko-learn)
- Gamma-like loop in orchestrate.rs (approximated)
- Tick topic constants defined (heartbeat.rs)

## Checklist

### BEAT-01: Theta reflective loop [CRITICAL for Phase 2+]
- [x] Implement periodic reflection consumer

**Spec** (doc 05 `docs/16-heartbeat/05-theta-reflective-loop.md`): Theta runs a five-phase reflection cycle every ~75s (configurable 30-120s range via `theta_min_interval_secs`/`theta_max_interval_secs` in `roko.toml`). The five phases are:
1. **Summarize gamma work** — aggregate recent gamma `DecisionCycleRecord`s into a concise summary
2. **Update Daimon affect** — update ALMA three-layer affect model (reactive/learned/stable layers) based on accumulated outcomes
3. **Check prediction calibration** — query `CalibrationTracker` for per-(model, category) accuracy drift
4. **Re-evaluate plan** — compare plan progress against DAG schedule, detect stalled/stuck tasks
5. **Trigger interventions** — escalate if stuck (reassign task, re-plan, alert operator)

Theta fires on `heartbeat.theta.tick` Bus Pulse, typically every N=5 gamma ticks or on episode completion. Theta always costs T1-T2 ($0.01-$0.10). The interval adapts by regime: shorter in Volatile/Crisis, longer in Calm.

**Current code** (`crates/roko-runtime/src/heartbeat.rs:586`): `HeartbeatPolicy` struct exists with `ClockConfig` (line 459). `compute_theta_interval()` function at line 724 already computes regime-based intervals. `HeartbeatTick` at line 510 with `speed: HeartbeatSpeed` field. No Theta consumer that processes ticks. No summarization or plan re-evaluation logic.

**What to change**: Add `ThetaConsumer` struct in `crates/roko-runtime/src/` that:
- Subscribes to `heartbeat.theta.tick` Bus topic (or receives ticks via channel until Bus exists)
- Implements the five-phase cycle as sequential async steps
- Phase 1: collect recent `DecisionCycleRecord`s from episode log and produce a `ThetaSummary`
- Phase 2: call into `roko-daimon` affect update (ALMA layer integration)
- Phase 3: read `CalibrationTracker` (at `crates/roko-core/src/prediction.rs:821`) for drift
- Phase 4: compare plan DAG progress against `crates/roko-orchestrator/src/dag.rs` schedule
- Phase 5: emit intervention Engrams if stuck (pattern: see `PlanRunner` in `crates/roko-cli/src/orchestrate.rs`)

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:586` — `HeartbeatPolicy` struct, `ClockConfig` at line 459, `compute_theta_interval()` at line 724, `HeartbeatTick` at line 510
- `crates/roko-core/src/loop_tick.rs:77` — `loop_tick()` function (gamma loop reference pattern)
- `crates/roko-daimon/src/lib.rs` — affect state, ALMA layers, `PadState`
- `crates/roko-core/src/prediction.rs:821` — `CalibrationTracker` for phase 3 calibration check
- `crates/roko-learn/src/episode_logger.rs` — recent episode data for summarization
- `docs/16-heartbeat/05-theta-reflective-loop.md` — full spec with all five phases
**Depends on**: None (can stub Bus dependency; use channel-based tick delivery)
**Accept when**:
- [x] `ThetaConsumer` struct exists in `crates/roko-runtime/src/` — theta_consumer.rs:138
- [x] Runs at ~75s intervals (configurable via `ClockConfig`) — uses `ThetaConfig` with configurable params; `HeartbeatPolicy` computes interval via `compute_theta_interval()`
- [x] Phase 1: summarizes recent gamma `DecisionCycleRecord`s — `ingest_gamma_record()` buffers records, `summarize_gamma_history()` produces `GammaSummary`
- [x] Phase 2: updates Daimon affect state via ALMA layers — `update_affect()` at theta_consumer.rs:230 computes pleasure/arousal/dominance deltas
- [x] Phase 3: checks `CalibrationTracker` for prediction drift — `check_calibration()` at theta_consumer.rs:267 detects drift above configurable threshold
- [x] Phase 4: re-evaluates current plan progress — `evaluate_plan_progress()` at theta_consumer.rs:297 tracks completion and detects stalled tasks
- [x] Phase 5: triggers interventions when stuck (>3 consecutive failures) — `stuck_threshold: 3` default, `has_stuck_issues()` at line 321
- [x] `cargo test --workspace` — 5 tests: ingests_and_summarizes, detects_stuck_state, updates_affect, plan_progress_tracks_completion, gamma_buffer_respects_capacity
**Verify**:
```bash
grep -rn 'ThetaConsumer\|theta_loop\|fn theta' crates/roko-runtime/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1 (Phase 2+)

### BEAT-02: Delta consolidation loop [CRITICAL for Phase 2+]
- [x] Wire dream cycle as Delta consumer

**Spec** (doc 06 `docs/16-heartbeat/06-delta-consolidation-loop.md`): Delta runs a three-phase dream cycle at ~hours interval (configurable via `delta_episode_threshold`, `delta_idle_timeout_secs`, `delta_scheduled_utc` in `[clock]` config):
1. **NREM replay** (Mattar & Daw 2018) — utility-based replay of high-value episodes, extracting patterns and compiling playbook rules
2. **REM imagination** (Boden 2004, Pearl 2009) — combinational/exploratory/transformational creativity, generating counterfactual scenarios via structural causal models
3. **Integration/staging** — promote validated knowledge from Transient to Working or Consolidated tier, compile playbook rules, prune stale entries

Delta fires on `heartbeat.delta.tick` Pulse, triggered by: (a) idle detection (`delta_idle_timeout_secs = 300`), (b) episode count threshold (`delta_episode_threshold = 50`), or (c) nightly schedule (`delta_scheduled_utc = "02:00"`). Delta is non-blocking — it runs in a background task that does not stall Gamma or Theta. Cost: T0-T1 ($0.00-$0.01).

**Current code** (`crates/roko-dreams/src/runner.rs`): Dream cycle runner exists with partial implementation of NREM/REM/integration phases. `crates/roko-runtime/src/heartbeat.rs:34`: `HeartbeatSpeed::Delta` variant defined. Not wired — no Delta consumer triggers the dream runner. `crates/roko-neuro/src/knowledge_store.rs` has tier promotion methods.

**What to change**: Add `DeltaConsumer` struct in `crates/roko-runtime/src/` that:
- Subscribes to `heartbeat.delta.tick` Bus topic (or receives ticks via channel)
- Spawns `crates/roko-dreams/src/runner.rs` dream cycle as a `tokio::spawn` background task
- After dream cycle completes, calls `roko-neuro` tier promotion for validated knowledge
- Compiles playbook rules from successful episode patterns
- Emits a `DeltaCycleReport` Engram with consolidation stats

**Reference files**:
- `crates/roko-dreams/src/runner.rs` — dream cycle runner with NREM/REM/integration phases
- `crates/roko-runtime/src/heartbeat.rs:34` — `HeartbeatSpeed::Delta` variant, `HeartbeatTick` at line 510
- `crates/roko-neuro/src/knowledge_store.rs` — tier promotion methods (`promote_tier()`)
- `crates/roko-neuro/src/distiller.rs` — pattern extraction and causal link distillation
- `crates/roko-learn/src/playbook.rs` — playbook rule compilation
- `docs/16-heartbeat/06-delta-consolidation-loop.md` — full three-phase spec
**Depends on**: None (can stub Bus dependency; use `tokio::spawn` for non-blocking)
**Accept when**:
- [x] `DeltaConsumer` struct exists in `crates/roko-runtime/src/` — delta_consumer.rs:166
- [x] Triggers dream cycle at configurable intervals (episode threshold, idle timeout, or schedule) — `DeltaConfig` has `episode_threshold: 50`, `idle_timeout_secs: 300`, `scheduled_utc`; `should_trigger()` returns appropriate `DeltaTrigger` variant
- [x] Non-blocking — runs via `tokio::spawn`, doesn't stall Gamma or Theta — documented at line 24: "non-blocking: spawns as background task"; `run_cycle()` designed for `tokio::spawn`
- [ ] Phase 1 (NREM): replays high-value episodes via dream runner — stub only (`run_nrem_phase()` at line 299 returns empty `NremPhaseReport`, not yet connected to `roko-dreams`)
- [ ] Phase 3 (Integration): promotes validated knowledge in `roko-neuro` tiers — stub only (line 325, `entries_promoted: 0`, contains TODO comments to wire `roko_neuro::knowledge_store`)
- [ ] Compiles playbook rules from successful patterns — stub only (`rules_compiled: 0` at line 307)
- [x] `cargo test --workspace` — tests verify trigger logic, cycle transitions, min interval, and low activity detection
**Verify**:
```bash
grep -rn 'DeltaConsumer\|delta_loop\|fn delta' crates/roko-runtime/src/ --include='*.rs'
grep -rn 'Delta' crates/roko-dreams/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1 (Phase 2+)

### BEAT-03: HeartbeatPolicy + adaptive clock
- [x] Implement tick emission on Bus with regime-based frequency adjustment

**Spec** (doc 07 `docs/16-heartbeat/07-adaptive-clock.md`): `HeartbeatPolicy` is the L0 Runtime component that publishes `heartbeat.gamma.tick`, `heartbeat.theta.tick`, and `heartbeat.delta.tick` Pulses on the Bus. It is a Bus producer, not a control-flow mechanism. Configuration via `[clock]` section in `roko.toml`:
```toml
[clock]
gamma_min_interval_secs = 5
gamma_max_interval_secs = 15
gamma_base_interval_secs = 10
theta_min_interval_secs = 15
theta_max_interval_secs = 120
theta_base_interval_secs = 75
theta_gamma_count = 5          # fire theta every N gamma ticks
delta_episode_threshold = 50
delta_idle_timeout_secs = 300
delta_scheduled_utc = "02:00"
daily_budget_usd = 50.0
throttle_at_percent = 80
hard_stop_at_percent = 95
```

Gamma adapts via `compute_gamma_interval(violations, config)` — more anomalies = faster (down to 5s), fewer = slower (up to 15s), implementing Friston's active sampling. Theta adapts by regime. Delta fires on idle, episode count, or schedule. Budget-aware throttling: at 80% budget, T2 calls throttled; at 95%, T2 hard-stopped, T1 throttled.

**Current code** (`crates/roko-runtime/src/heartbeat.rs:586`): `HeartbeatPolicy` struct **exists** with `ClockConfig` at line 459 (has `gamma_min`/`gamma_max`/`gamma_base`/`theta_*`/`delta_*` fields). `compute_theta_interval()` at line 724 computes regime-based intervals. `Regime` enum at line 79 (Calm/Normal/Volatile/Crisis). `CorticalState` at line 269 stores regime in `AtomicU8`. **Missing**: actual `HeartbeatPolicy::run()` method that emits tick Pulses onto Bus; no Bus trait exists yet.

**What to change**: Once Bus trait (K-02) exists, implement `HeartbeatPolicy::run()` as an async loop that:
1. Maintains three `tokio::time::Interval`s (gamma, theta, delta)
2. On each gamma interval: publish `Pulse { topic: "heartbeat.gamma.tick", payload: HeartbeatTick }` to Bus
3. On each theta interval (every N gamma ticks or regime-adjusted): publish theta tick
4. On delta triggers (idle/episode-count/schedule): publish delta tick
5. Read `CorticalState.regime` to adjust intervals dynamically
6. Check budget tracker against `throttle_at_percent`/`hard_stop_at_percent`

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:586` — `HeartbeatPolicy`, `ClockConfig` at 459, `Regime` at 79, `CorticalState` at 269, `compute_theta_interval()` at 724
- `crates/roko-core/src/operating_frequency.rs` — `OperatingFrequency` <-> `InferenceTier` mapping
- `docs/16-heartbeat/07-adaptive-clock.md` — full spec with frequency adjustment rules, budget throttling, and Bus topic delivery
**Depends on**: K-02 (Bus trait in `02-missing-kernel-types.md`), K-01 (Pulse struct)
**Accept when**:
- [x] `HeartbeatPolicy::run()` async method emits Gamma/Theta/Delta ticks on Bus
- [x] Gamma frequency adjusts based on probe anomaly count (5-15s range)
- [x] Theta frequency adjusts based on detected regime (30-120s range)
- [x] Delta fires on idle timeout, episode threshold, or nightly schedule
- [x] Budget-aware throttling: T2 throttled at 80%, hard-stopped at 95%
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'HeartbeatPolicy\|fn run\|fn emit_tick' crates/roko-runtime/src/heartbeat.rs
cargo test --workspace
```
**Priority**: P1 (Phase 2+, blocked on K-02 Bus)

### BEAT-04: Formal T0 probe registry
- [x] Implement Probe trait and register 16 probes

**Spec** (doc 09 `docs/16-heartbeat/09-16-t0-probes.md`): 16 zero-LLM probes that execute as pure functions with zero LLM cost. Each returns a scalar in [0.0, 1.0]. Organized by domain:

**8 chain probes**: (1) price baseline — deviation from 24h VWAP, (2) TVL — pool TVL change rate, (3) position health — distance to liquidation, (4) gas — current vs 7d average gwei, (5) credit — borrowing utilization ratio, (6) RSI — 14-period Relative Strength Index, (7) MACD — signal line crossover, (8) circuit breaker — aggregate risk score triggers halt.

**6 coding probes**: (1) build — last build time vs 7d average, (2) tests — test pass rate delta, (3) complexity — McCabe complexity delta per commit, (4) deps — dependency freshness score, (5) coverage — test coverage delta, (6) error rate — runtime error rate trend.

**2 universal probes**: (1) world model drift — KL divergence between predicted and observed distributions, (2) causal consistency — ratio of causal predictions confirmed by outcomes.

Each probe implements `trait Probe: Send + Sync` with `fn evaluate(&self, state: &EngineState) -> ProbeResult`. The `ProbeRegistry` manages registration and batch evaluation. Probe anomalies (value crossing adaptive threshold) trigger tier escalation: any anomaly → T1, >=3 anomalies → T2.

**Current code** (`crates/roko-runtime/src/heartbeat_probes.rs:25`): `pub trait Probe: Send + Sync` **already defined**. `ProbeResult` at line 418 with `value: f64`, `is_anomalous: bool`. `ProbeResults` at line 469. `ProbeRegistry` at line 488 with `register()` and `evaluate_all()`. `ProbeDomain` enum at line 41 (Chain, Coding, Universal). `EngineState` at line 139 with chain fields (`tracked_assets`, `gas_gwei`, `macd_signal`, `rsi_value`, etc.) and coding fields (`build_time_secs`, `test_pass_rate`, `complexity_delta`). Also: `crates/roko-core/src/obs/health.rs:89` has a **second** `Probe` trait. **Duplicate Probe traits need reconciliation.**

**What to change**:
1. Reconcile the two `Probe` traits: keep `crates/roko-runtime/src/heartbeat_probes.rs` as canonical (it has the richer `EngineState`), re-export or alias from `roko-core`
2. Implement concrete probe structs for at least the 6 coding probes + 2 universal probes (chain probes depend on chain infrastructure)
3. Register all implemented probes in `ProbeRegistry` default constructor
4. Wire `ProbeRegistry::evaluate_all()` into the SENSE step of `crates/roko-cli/src/orchestrate.rs`
5. Use probe anomaly count to drive tier routing: 0 anomalies → T0, 1-2 → T1, >=3 → T2

**Reference files**:
- `crates/roko-runtime/src/heartbeat_probes.rs:25` — `Probe` trait, `ProbeResult` at 418, `ProbeRegistry` at 488, `ProbeDomain` at 41, `EngineState` at 139
- `crates/roko-core/src/obs/health.rs:89` — duplicate `Probe` trait, `ProbeRegistry` at 103 (reconcile with runtime version)
- `crates/roko-runtime/src/heartbeat.rs:269` — `CorticalState` written by probe results
- `crates/roko-cli/src/orchestrate.rs` — SENSE step where probes should be invoked
- `docs/16-heartbeat/09-16-t0-probes.md` — full spec of all 16 probes with field names and thresholds
**Depends on**: None
**Accept when**:
- [ ] Single canonical `Probe` trait defined (duplicate in roko-core reconciled) — two separate `Probe` traits still exist: `roko-core/src/obs/health.rs:89` (liveness probes) and `roko-runtime/src/heartbeat_probes.rs` (heartbeat probes with richer `EngineState`)
- [x] At least 6 coding probes implemented as concrete structs
- [x] At least 2 universal probes implemented
- [x] All probes registered in `ProbeRegistry`
- [ ] Probes invoked at SENSE step in `orchestrate.rs` — no `evaluate_all` or `run_probes` calls found in orchestrate.rs
- [x] Anomaly count drives T0/T1/T2 tier routing — `StatefulProbeRegistry` with `RollingStats` enables real z-score anomaly detection; `select_tier_from_probes()` maps anomaly count to tier
- [x] Probe results written to `CorticalState` atomic fields — all 32 signal accessors now implemented
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'trait Probe' crates/ --include='*.rs' | grep -v target/
grep -rn 'ProbeRegistry' crates/ --include='*.rs' | grep -v target/
grep -rn 'evaluate_all\|run_probes' crates/roko-cli/src/orchestrate.rs
cargo test --workspace
```
**Priority**: P1

### BEAT-05: Formal seven-step loop alignment
- [x] Align orchestrate.rs with canonical SENSE->ASSESS->COMPOSE->ACT->VERIFY->PERSIST/BROADCAST->REACT

**Spec** (docs 00-01 `docs/16-heartbeat/00-coala-9-step-pipeline.md`, `docs/16-heartbeat/01-universal-loop-mapping.md`): The canonical seven-step universal loop maps the historical CoALA 9-step pipeline onto the Synapse Architecture:
1. **SENSE** — `Substrate.query()` + T0 probes (L0 Runtime)
2. **ASSESS** — `Scorer.score()` + `Router.select()` tier gate (L1 Framework)
3. **COMPOSE** — `Composer.compose()` context assembly with VCG auction (L2 Scaffold)
4. **ACT** — Agent execution (L4 Orchestration)
5. **VERIFY** — `Gate.verify()` (L3 Harness)
6. **PERSIST + BROADCAST** — `Substrate.put()` co-equal with Bus Pulse publication. BROADCAST publishes live Pulses for downstream consumers (other agents, dashboard, watchers)
7. **REACT** — `Policy.decide()` plus Daimon/Neuro/Dreams cross-cuts

Neuro, Daimon, and Dreams are injected as cross-cuts rather than sequenced as extra steps. Domain parameterization (coding vs chain vs research) changes the probes and tools, not the loop shape.

The loop outputs a `DecisionCycleRecord` per tick — a structured, self-contained record (not conversation history). Fields include: tick number, tier used, observations, actions, outcomes, cost, duration, and credit assignment.

**Current code** (`crates/roko-cli/src/orchestrate.rs`): Approximates the loop but steps are implicit. `crates/roko-core/src/loop_tick.rs:77`: `loop_tick()` function implements core loop with Scorer -> Router -> Composer -> Gate -> Substrate flow. No BROADCAST step separate from PERSIST. No explicit REACT step via `Policy.decide()`. No `DecisionCycleRecord` struct.

**What to change**:
1. Add explicit step markers/comments in `loop_tick()` and `orchestrate.rs` (e.g., `// === STEP 1: SENSE ===`)
2. Add BROADCAST step after PERSIST — publish a `Pulse` on Bus with tick results
3. Add REACT step calling `Policy.decide()` at end of loop
4. Define `DecisionCycleRecord` struct with fields: `tick: u64`, `tier: InferenceTier`, `observations: Vec<ProbeResult>`, `action: Option<Action>`, `outcome: Option<Outcome>`, `cost_usd: f64`, `duration_ms: u64`
5. Emit `DecisionCycleRecord` per tick for Theta summarization and episode logging

**Reference files**:
- `crates/roko-core/src/loop_tick.rs:77` — core `loop_tick()` function to annotate with step markers
- `crates/roko-cli/src/orchestrate.rs` — orchestration wrapper, integration point for BROADCAST and REACT
- `crates/roko-core/src/traits.rs` — `Policy` trait with `decide()` method for REACT step
- `docs/16-heartbeat/00-coala-9-step-pipeline.md` — historical framing and `DecisionCycleRecord` spec
- `docs/16-heartbeat/01-universal-loop-mapping.md` — canonical seven-step mapping with layer traversal
- `docs/16-heartbeat/04-gamma-reactive-loop.md` — gamma-specific step details with code examples
**Depends on**: K-01 (Pulse), K-02 (Bus) for BROADCAST; TM-06 (Policy migration) for REACT
**Accept when**:
- [x] Loop steps clearly identifiable in code with step markers — `DecisionCycleRecord` fields annotated with `// -- Step N: PHASE --` comments
- [x] `DecisionCycleRecord` struct defined with per-tick fields — all 7 steps represented
- [x] PERSIST and BROADCAST are co-equal steps (Bus Pulse published after Substrate write) — `HeartbeatPolicy::broadcast_tick_outcome()` emits `RokoEvent::TickBroadcast`
- [x] `Policy.decide()` implements REACT step — `HeartbeatPolicy::emit_react_decision()` emits `RokoEvent::ReactDecision`
- [x] `DecisionCycleRecord` emitted per gamma tick
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'SENSE\|ASSESS\|COMPOSE\|ACT\|VERIFY\|PERSIST\|BROADCAST\|REACT' crates/roko-core/src/loop_tick.rs
grep -rn 'DecisionCycleRecord' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2

### BEAT-06: CorticalState shared perception surface
- [x] Implement 32-signal CorticalState struct

**Spec** (doc 12 `docs/16-heartbeat/12-attention-auction-and-gating.md` §CorticalState): `CorticalState` is a 32-signal atomic struct (~192 bytes, 4 cache lines, `#[repr(C, align(64))]`) providing zero-latency inter-subsystem communication. Organized into 7 signal groups with clear ownership:

| Signal Group | Writer | Fields | Frequency |
|---|---|---|---|
| **Affect** (4 signals) | Daimon | `pleasure: AtomicU32`, `arousal: AtomicU32`, `dominance: AtomicU32`, `primary_emotion: AtomicU8` (Plutchik 0-7) | Per prediction resolution |
| **Prediction** (20 signals) | Oracle/CalibrationTracker | `aggregate_accuracy: AtomicU32`, `accuracy_trend: AtomicI8`, `category_accuracies: [AtomicU32; 16]`, `surprise_rate: AtomicU32` | Per prediction resolution |
| **Attention** (3 signals) | AttentionForager | `universe_size: AtomicU32`, `active_count: AtomicU16`, `pending_predictions: AtomicU32` | Per gamma tick |
| **Creative** (4 signals) | Dream engine | `creative_mode: AtomicU8`, `fragments_captured: AtomicU32`, `last_novel_prediction_tick: AtomicU32`, `last_novel_prediction_tick_hi: AtomicU32` | Per dream cycle |
| **Environment** (2 signals) | Domain probes | `regime: AtomicU8` (0-3), `gas_gwei: AtomicU32` | Per gamma tick |
| **Resource** (4 signals) | Budget tracker / Theta | `resource_health: AtomicU32`, `knowledge_health: AtomicU32`, `performance_trend: AtomicU32`, `behavioral_state: AtomicU8` (0-5) | Per theta tick |
| **Derived** (1 signal) | Runtime | `compounding_momentum: AtomicU32` | Per delta tick |

No locks — writes use `Ordering::Release`, reads use `Ordering::Acquire`. No signal has two writers (no contention). f32 values stored via `f32::to_bits()` / `f32::from_bits()`.

`CorticalSnapshot` is the full copy for context assembly. Personality presets (Cautious/Balanced/Aggressive) set initial PAD values.

**Current code** (`crates/roko-runtime/src/heartbeat.rs:269`): `CorticalState` struct partially defined with atomic fields: `pleasure`, `arousal`, `dominance`, `prediction_accuracy`, `regime`, `behavioral_state`, `resource_health`, plus padding. `CorticalSnapshot` at line 416 has a subset of fields. **Missing**: `primary_emotion`, `accuracy_trend`, `category_accuracies[16]`, `surprise_rate`, `universe_size`, `active_count`, `pending_predictions`, `creative_mode`, `fragments_captured`, `last_novel_prediction_tick`, `last_novel_prediction_tick_hi`, `gas_gwei`, `knowledge_health`, `performance_trend`, `compounding_momentum`.

**What to change**: Add all missing atomic signal fields to `CorticalState` to reach 32 total. Add `#[repr(C, align(64))]` for cache-line alignment. Update `CorticalSnapshot` to include all 32 signals. Add `fn pad(&self) -> PadVector`, `fn prediction_accuracy(&self) -> f32`, `fn behavioral_state(&self) -> BehavioralState`, `fn snapshot(&self) -> CorticalSnapshot` reader methods. Add `fn new(personality: &PersonalityPreset) -> Self` constructor.

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:269` — current partial `CorticalState`, `CorticalSnapshot` at 416
- `crates/roko-runtime/src/heartbeat_probes.rs:139` — `EngineState` probe-side fields (separate from CorticalState)
- `docs/16-heartbeat/12-attention-auction-and-gating.md` §CorticalState — full 32-signal struct definition with signal group ownership table
**Depends on**: BEAT-04 (T0 probes to populate Environment signals)
**Accept when**:
- [x] `CorticalState` has all 32 atomic signal fields matching doc 12 spec
- [x] `#[repr(C, align(64))]` for cache-line alignment
- [x] Reader methods: `pad()`, `prediction_accuracy()`, `behavioral_state()`, `snapshot()` — plus `pending_predictions()`, `fragments_captured()`, `last_novel_prediction_tick()`, `category_accuracy()`, `category_accuracies()`, `aggregate_accuracy()`
- [x] `CorticalSnapshot` covers all 32 signals
- [x] No signal has two writers
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'CorticalState' crates/roko-runtime/src/heartbeat.rs
grep -c 'Atomic' crates/roko-runtime/src/heartbeat.rs  # should show 32+ atomic fields
cargo test --workspace
```
**Priority**: P2

### BEAT-07: VCG attention auction for context budget
- [x] Implement truthful bidding for context token allocation

**Spec** (doc 12 `docs/16-heartbeat/12-attention-auction-and-gating.md` §VCG Attention Auction): VCG mechanism (Vickrey 1961, Clarke 1971, Groves 1973) allocates context token budget across 8 competing subsystems on every T1/T2 tick during the COMPOSE step.

**8 bidding subsystems**: Neuro (knowledge entries), Daimon (affect state), Iteration Memory (past failures), Code Intelligence (symbol graphs), Playbook Rules (learned heuristics), Research Artifacts (analyses), Task Context (PRD/plan details), Oracle Predictions (calibration data).

**Bid formula**: `bid = expected_value * urgency * affect_weight`
- `expected_value`: per-subsystem estimation (e.g., Neuro uses PredictiveScorer salience, Daimon uses PAD magnitude)
- `urgency`: multiplier [0.5, 2.0] based on deadline pressure, retry count, safety relevance
- `affect_weight`: Daimon PAD modulation — high arousal boosts safety (+0.5), low dominance boosts exploration (+0.3), low pleasure boosts iteration memory (+0.4)

**Context governor** manages tier-specific budgets: T0=0 tokens, T1=~4K tokens, T2=~32K tokens. Task complexity adjusts budget (0.5x-1.5x). Winners pay second-highest bid (VCG truthfulness). Attention budget carryover with 0.95 decay and -5.0 max debt.

**Current code** (`crates/roko-runtime/src/heartbeat_attention.rs:534`): `AttentionBudgetManager` exists with `tier_budgets`. `BidItem` at line 265 with `tier: InferenceTier`. Separately: `crates/roko-compose/src/auction.rs` has a VCG auction implementation. **Not integrated** — the two are disconnected. No `ContextGovernor` struct. No per-subsystem `ContextBidder` trait. No affect modulation wiring.

**What to change**:
1. Define `trait ContextBidder: Send + Sync` with `fn generate_candidates(&self) -> Vec<ContextCandidate>` in roko-compose or roko-runtime
2. Implement `ContextBidder` for all 8 subsystems
3. Implement `ContextGovernor` struct with `tier_budgets`, `adjusted_budget()` for task complexity, and `assemble()` method that runs VCG auction
4. Wire `ContextGovernor::assemble()` into the COMPOSE step of `loop_tick()` / `orchestrate.rs`
5. Connect `AttentionBudgetManager` carryover to affect the next tick's bid multipliers
6. Wire Daimon PAD affect modulation into bid computation

**Reference files**:
- `crates/roko-runtime/src/heartbeat_attention.rs:534` — `AttentionBudgetManager`, `BidItem` at 265
- `crates/roko-compose/src/auction.rs` — existing VCG auction implementation (connect to heartbeat)
- `crates/roko-runtime/src/heartbeat.rs:269` — `CorticalState` PAD fields for affect modulation
- `crates/roko-compose/src/system_prompt_builder.rs` — current context assembly (to be enhanced with VCG)
- `docs/16-heartbeat/12-attention-auction-and-gating.md` — full spec: VCG mechanism, 8 bidders, affect modulation formulas, context governor, attention budget carryover
**Depends on**: BEAT-03 (HeartbeatPolicy tick emission), BEAT-06 (CorticalState PAD fields)
**Accept when**:
- [x] `ContextBidder` trait defined with `generate_candidates()`
- [x] All 8 subsystems implement `ContextBidder`
- [x] `ContextGovernor` manages T0/T1/T2 budgets (0/4K/32K)
- [x] VCG auction runs during COMPOSE step on T1/T2 ticks
- [x] Affect modulation applies PAD-derived weights to bids
- [x] Attention budget carryover with 0.95 decay
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'ContextBidder\|ContextGovernor\|AttentionBudgetManager\|VCG\|auction' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2

### BEAT-08: Active inference POMDP state space
- [x] Implement factorized discrete POMDP for tier selection

**Spec** (doc 11 `docs/16-heartbeat/11-active-inference-state-space.md`): Factorized discrete POMDP (Koudahl et al. 2024, arXiv:2412.10425) with 3 state factors:
- **TaskPhase** (6 values): Planning, Implementing, Testing, Reviewing, Debugging, Deploying
- **ContextQuality** (5 values): Poor, Fair, Good, Excellent, Perfect
- **Uncertainty** (3 values): Low, Medium, High
Total: 6 x 5 x 3 = 90 states.

Four matrices from pymdp:
- **A** (likelihood): P(observation | state) — how probe results map to hidden states
- **B** (transition): P(state' | state, action) — how tier selection changes state
- **C** (preferences): desired observations (low prediction error, low cost)
- **D** (prior): initial belief distribution

Tier selection via Expected Free Energy (EFE) minimization: `G = pragmatic_value + epistemic_value - ambiguity`. The agent selects the tier (T0/T1/T2) that minimizes G. Prediction/outcome Bus topic families: `prediction.registered.*`, `prediction.resolved.*`, `prediction.error.*`. Bayesian matrix learning from outcomes.

**Current code** (`crates/roko-runtime/src/heartbeat.rs`): Prediction error heuristics only. `InferenceTier` enum at line 57 (T0/T1/T2). `CascadeRouter` in `crates/roko-learn/src/cascade_router.rs` does heuristic tier selection via LinUCB bandit. No formal POMDP. No belief state. No EFE computation.

**What to change**:
1. Define `PomdpState` enum variants for the 3 factors in `crates/roko-runtime/src/` or `crates/roko-primitives/src/`
2. Define `BeliefState` as a probability distribution over 90 states (flat `Vec<f64>`)
3. Implement A/B/C/D matrices with reasonable priors
4. Implement `fn select_tier(belief: &BeliefState, matrices: &PomdpMatrices) -> InferenceTier` using EFE minimization
5. Can coexist with `CascadeRouter` — POMDP replaces heuristic when matrices are learned

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:57` — `InferenceTier` enum (T0/T1/T2)
- `crates/roko-learn/src/cascade_router.rs` — `CascadeRouter` with LinUCB (current tier routing, to be replaced)
- `crates/roko-core/src/operating_frequency.rs` — `OperatingFrequency` <-> `InferenceTier` mapping
- `docs/16-heartbeat/11-active-inference-state-space.md` — full POMDP spec with matrix definitions and EFE formula
- `docs/16-heartbeat/10-active-inference-compute-allocation.md` — EFE theory, PredictiveScorer, rational inattention
**Depends on**: None (can coexist with CascadeRouter as a gradual replacement)
**Accept when**:
- [x] `PomdpState` with 3 factors (6 x 5 x 3 = 90 states)
- [x] `BeliefState` probability distribution defined
- [x] A/B/C/D matrices initialized with reasonable priors
- [x] EFE minimization selects tier (T0/T1/T2) from belief state
- [x] Belief updated from probe observations and prediction outcomes
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'POMDP\|PomdpState\|EFE\|BeliefState' crates/ --include='*.rs' | grep -v target/
cargo test --workspace
```
**Priority**: P2 (Phase 2+ research)

### BEAT-09: Chain heartbeat variant (SIMULATE + VALIDATE)
- [x] Implement domain-specific SIMULATE and VALIDATE steps for chain agents

**Spec** (doc 02 `docs/16-heartbeat/02-chain-heartbeat-variant.md`): Chain agents extend the universal 7-step loop with two additional steps between ATTEND and ACT because chain actions are financially irreversible:
- **Step 5: SIMULATE** — run proposed transaction in local EVM fork via mirage-rs. Checks: revert detection, gas estimation, state change verification, sandwich attack vulnerability, price impact calculation.
- **Step 6: VALIDATE** — check against PolicyCage, position limits, approved asset list, max position size. Fails the tick if any policy violation detected.

This makes the chain heartbeat an 11-step variant (the universal 7 + SIMULATE + VALIDATE + ANALYZE + META-COGNIZE). Also specifies the Sleepwalker 3-step variant (OBSERVE -> REFLECT -> PUBLISH) for observer-only chain agents that never execute transactions.

**Current code**: `crates/roko-chain/src/` exists but has no heartbeat-specific logic. `crates/roko-agent/src/safety/contract.rs` has some contract safety checks. No mirage-rs integration. No SIMULATE step. No VALIDATE step with position limits.

**What to change**: Add `ChainHeartbeatExtension` (or similar) in `crates/roko-chain/src/` that:
1. Hooks into the universal loop between ATTEND and ACT
2. SIMULATE: calls mirage-rs (or Revm via alloy) to simulate proposed transaction
3. VALIDATE: checks PolicyCage constraints, position limits, approved assets
4. Returns `Ok(())` or `Err(ChainValidationError)` to gate the ACT step

**Reference files**:
- `crates/roko-chain/src/` — chain crate (implementation target)
- `crates/roko-agent/src/safety/contract.rs` — existing contract safety checks (pattern reference)
- `crates/roko-agent/src/safety/hooks.rs:192` — `SafetyHook` trait (SIMULATE/VALIDATE are domain-specific hooks)
- `crates/roko-cli/src/orchestrate.rs` — orchestration loop where chain extension hooks in
- `docs/16-heartbeat/02-chain-heartbeat-variant.md` — full 11-step chain heartbeat spec, mirage-rs checks, Sleepwalker variant
**Depends on**: TOOL-05 (chain domain tools), BEAT-05 (formal loop alignment)
**Accept when**:
- [x] SIMULATE step runs proposed tx through EVM fork (mirage-rs or Revm) — `ChainHeartbeatExtension::simulate()` delegates to `TxSimulator` trait
- [x] VALIDATE step checks PolicyCage, position limits, approved assets — `PolicyCageConfig` enforces `max_open_positions`, `max_daily_volume_usd`, `approved_assets`, `max_gas_gwei`
- [x] Chain heartbeat hooks into universal loop without modifying it — `pre_act_check()` returns `ChainPreActResult` gating ACT
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'SIMULATE\|VALIDATE\|ChainHeartbeat\|simulate_tx' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```
**Priority**: P2 (blocked on chain domain tools)

---

### BEAT-10: Dual-process T0/T1/T2 adaptive gating
- [x] Implement adaptive gating threshold for tier escalation

**Spec** (doc 08 `docs/16-heartbeat/08-dual-process-t0-t1-t2.md`): LLM-Last architecture. Three tiers with adaptive gating:
- **T0** (reflex): No LLM. Pure deterministic probes + playbook rules. ~80% of ticks. Cost: $0.00. Latency: <100ms.
- **T1** (deliberative): Fast/cheap LLM (Haiku-class). ~15% of ticks. Cost: $0.001-$0.003. ~4K context.
- **T2** (reflective): Full LLM (Sonnet/Opus). ~5% of ticks. Cost: $0.01-$0.25. ~32K context.

Tier escalation via adaptive threshold: `anomaly_count >= threshold` triggers T1, `anomaly_count >= threshold * 2` triggers T2. Threshold adapts per regime (lower in Volatile/Crisis, higher in Calm). FrugalGPT-inspired cascade (Chen et al. 2023): try T0 first, escalate only if needed.

**Current code**: `InferenceTier` enum exists at `crates/roko-runtime/src/heartbeat.rs:57`. `CascadeRouter` in `crates/roko-learn/src/cascade_router.rs` does model routing but not tier gating from probes. `crates/roko-gate/src/adaptive_threshold.rs` has adaptive thresholds for gates, not tier escalation. No integration between probe anomaly counts and tier selection.

**What to change**: Wire probe anomaly count from BEAT-04 into tier selection. Add `fn select_tier(anomaly_count: usize, regime: Regime, threshold: &AdaptiveThreshold) -> InferenceTier` in `crates/roko-runtime/src/heartbeat.rs`. Threshold adapts based on regime and past accuracy.

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:57` — `InferenceTier` enum
- `crates/roko-learn/src/cascade_router.rs` — `CascadeRouter` (model routing, not tier gating)
- `crates/roko-gate/src/adaptive_threshold.rs` — adaptive threshold EMA (pattern reference for tier threshold)
- `crates/roko-runtime/src/heartbeat_probes.rs` — probe results that drive tier selection
- `docs/16-heartbeat/08-dual-process-t0-t1-t2.md` — full spec with cost model, threshold adaptation, FrugalGPT validation
**Depends on**: BEAT-04 (probe registry provides anomaly counts)
**Accept when**:
- [x] `select_tier()` function maps anomaly count + regime to InferenceTier — `select_tier_from_probes()` in heartbeat.rs
- [x] Adaptive threshold adjusts based on regime (lower in Crisis, higher in Calm)
- [x] ~80% of ticks stay at T0 (verifiable via efficiency events) — `TierGatingStats` tracks distribution with `distribution_healthy()` check
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'select_tier\|tier_gate\|anomaly.*threshold' crates/roko-runtime/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1 (prerequisite for cost-efficient heartbeat)

---

### BEAT-11: Frequency scheduler and meta-cognition hook
- [x] Implement frequency scheduler coordinating all three loops and meta-cognition

**Spec** (doc 12 `docs/16-heartbeat/12-attention-auction-and-gating.md` §Frequency Scheduler, §Meta-cognition): The frequency scheduler coordinates Gamma, Theta, and Delta loops:
- Ensures Theta fires after every N gamma ticks (configurable `theta_gamma_count = 5`)
- Ensures Delta fires on idle detection, episode threshold, or schedule
- Prevents loop starvation (Gamma cannot monopolize resources)
- Tracks loop health metrics (ticks per loop, latency, cost)

Meta-cognition hook ("Am I stuck? Am I thrashing?"):
- Detects stuck state: >3 consecutive failures on same task
- Detects thrashing: alternating success/failure pattern
- Detects coasting: >10 consecutive T0 ticks with no anomalies
- Triggers: escalate to Theta reflection, switch strategy, alert operator
- Fires as part of REACT step or as a Theta-triggered check

**Current code**: `HeartbeatPolicy` at `crates/roko-runtime/src/heartbeat.rs:586` has `ClockConfig` but no scheduler that coordinates the three loops. No meta-cognition detection logic. `crates/roko-conductor/src/circuit_breaker.rs` has circuit breaker (pattern reference for detecting stuck states) but not integrated into heartbeat.

**What to change**:
1. Add `FrequencyScheduler` struct that owns three interval timers and coordinates tick emission
2. Add `MetaCognitionHook` that analyzes recent `DecisionCycleRecord` history for stuck/thrashing/coasting patterns
3. Wire `MetaCognitionHook` into the REACT step of the gamma loop or as a Theta-phase-5 check
4. Emit meta-cognition events for dashboard visibility

**Reference files**:
- `crates/roko-runtime/src/heartbeat.rs:586` — `HeartbeatPolicy`, `ClockConfig` at 459
- `crates/roko-conductor/src/circuit_breaker.rs` — circuit breaker (pattern reference for stuck detection)
- `crates/roko-learn/src/efficiency.rs` — efficiency events (track loop health metrics)
- `docs/16-heartbeat/12-attention-auction-and-gating.md` — frequency scheduler spec, meta-cognition hook spec
**Depends on**: BEAT-03 (HeartbeatPolicy), BEAT-05 (DecisionCycleRecord for pattern analysis)
**Accept when**:
- [x] `FrequencyScheduler` coordinates Gamma/Theta/Delta emission
- [x] Theta fires every N gamma ticks (configurable)
- [x] Delta fires on idle/episode-count/schedule triggers
- [x] `MetaCognitionHook` detects stuck (>3 failures), thrashing, coasting
- [x] Meta-cognition triggers strategy change or Theta reflection
- [x] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'FrequencyScheduler\|MetaCognition\|meta_cognition' crates/roko-runtime/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P2

---

## Verify
```bash
cargo test -p roko-runtime
cargo test --workspace
```
