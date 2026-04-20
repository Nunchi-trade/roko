# 04-verification -- Gap Checklist

Spec: `docs/04-verification/` (15 files, docs 00-12 + 15 + INDEX). Code: `crates/roko-gate/`, `crates/roko-core/src/verdict.rs`.

Overall: ~61% of designed features implemented. Core gate pipeline ~100%. Advanced learning/SPC/evaluation lifecycle ~0-30%.

## Compliant (no action needed)
- Gate trait definition (doc 00)
- All 11+ gate implementations (doc 01) -- actually 14 in code
- 7-rung selector with complexity escalation (doc 02)
- Core sequential pipeline with short-circuit (doc 03)
- Artifact store with BLAKE3 content-addressing (doc 04)
- Ratcheting -- monotonic rung tracking, regression prevention (doc 05)
- Agent feedback classification -- noise detection, error/warning/suggestion buckets (doc 08)

## Checklist

### GATE-01: SPC extensions for adaptive thresholds
- [x] Add CUSUM, EWMA control charts, BOCPD to threshold adaptation

**Spec** (doc 06 §11-15 Adaptive Thresholds): Three Statistical Process Control extensions
beyond the current EMA:
1. **CUSUM (Cumulative Sum)**: Detects sustained shifts in gate pass rates. Accumulates
   deviations from target; when cumulative sum exceeds threshold `h`, signals a shift.
   Useful for catching gradual degradation (e.g., pass rate drifting from 85% to 70% over
   50 tasks).
2. **EWMA Control Chart**: Exponentially weighted moving average with formal UCL/LCL
   (Upper/Lower Control Limits) at mean +/- L*sigma/sqrt(lambda/(2-lambda)). More sensitive
   to small shifts than standard Shewhart charts.
3. **BOCPD (Bayesian Online Change Point Detection)**: Detects abrupt regime changes (e.g.,
   a model update causes sudden behavior shift). Maintains run-length distribution and
   signals when posterior probability of recent change point exceeds threshold.

**Current code** (`crates/roko-gate/src/adaptive_threshold.rs:47`): `AdaptiveThresholds`
struct tracks per-rung pass rates via core EMA (alpha=0.1) and manages retry budgets. No
SPC detectors. `EwmaState` at `crates/roko-learn/src/anomaly.rs:152` provides a reusable
EWMA primitive with `update()` and `current()` methods but is not imported into roko-gate.

**What to change**: Add a new `crates/roko-gate/src/spc.rs` module with:
- `pub struct CusumDetector { cumsum: f64, target: f64, threshold_h: f64, drift_k: f64 }`
  with `update(observation: f64) -> bool` returning true on shift
- `pub struct EwmaControlChart { ewma: f64, lambda: f64, sigma: f64, L: f64 }`
  with `update(observation: f64) -> ControlStatus` (InControl/Warning/OutOfControl)
- `pub struct BocpdDetector { run_lengths: Vec<f64>, hazard_rate: f64 }`
  with `update(observation: f64) -> Option<ChangePoint>`
Wire into `AdaptiveThresholds`: after each rung update, feed the pass/fail observation to
all three detectors. If any signals an anomaly, emit a conductor alert.

**Reference files**:
- `crates/roko-gate/src/adaptive_threshold.rs:47` -- `AdaptiveThresholds` (wire detectors here)
- `crates/roko-learn/src/anomaly.rs:152` -- `EwmaState` primitive (reuse for EWMA chart)
- `docs/04-verification/06-adaptive-thresholds.md` -- §11-15 SPC spec with formulas

**Depends on**: None

**Accept when**:
- [x] `CusumDetector` struct with `update()` returning shift detection
- [x] `EwmaControlChart` with formal UCL/LCL limits
- [x] `BocpdDetector` with run-length distribution and change point detection
- [x] Detectors wired into `AdaptiveThresholds` per-rung updates
- [x] `cargo test -p roko-gate` passes

**Verify**:
```bash
grep -rn 'CusumDetector\|BocpdDetector\|EwmaControlChart' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-gate
```

**Priority**: P2

### GATE-02: Process reward models
- [x] Implement Promise + Progress scoring from gate verdicts

**Spec** (doc 07 §Process Reward Models): Process rewards are cybernetic signals derived from
gate verdicts at each agent turn (not just final outcome). Two scores: **Promise** predicts
the probability of eventual task success given current trajectory -- computed from ratchet
progression rate, gate pass history, and diff size trends. **Progress** measures trajectory
delta between turns -- computed from rung advancement, error count reduction, and test
coverage increase. Together they enable early termination (low Promise = abandon task) and
intervention (stalling Progress = change model/strategy). The spec references Lightman et al.
2023 (PRM800K) and AgentPRM (arXiv:2502.10325) for per-step reward signals.

**Current code**: No `ProcessRewardModel` struct anywhere in the codebase. The components
that would feed it all exist: `Verdict` at `crates/roko-core/src/verdict.rs:51` (with
`score: f64` and `passed: bool`), `GateRatchet` at `crates/roko-gate/src/ratchet.rs:21`
(monotonic rung tracking with `current_rung()` and `update()`), `GateFeedback` at
`crates/roko-gate/src/feedback.rs:53` (severity classification into error/warning/suggestion
buckets). The retry loop in orchestrate.rs increments iteration count but doesn't compute
any reward signal.

**What to change**: Create `crates/roko-gate/src/process_reward.rs` with:
```rust
pub struct ProcessRewardModel {
    pub history: Vec<TurnSnapshot>,
}
pub struct TurnSnapshot {
    pub rung: u32,
    pub verdicts: Vec<Verdict>,
    pub error_count: u32,
    pub diff_lines: u32,
}
impl ProcessRewardModel {
    pub fn promise(&self) -> f64 { /* ratchet progression rate * historical pass rate */ }
    pub fn progress(&self) -> f64 { /* delta between last two snapshots */ }
    pub fn should_terminate(&self, min_promise: f64) -> bool { /* promise < threshold */ }
}
```
Wire into the orchestrator retry loop at `crates/roko-cli/src/orchestrate.rs` -- after each
gate run, update the PRM, use `should_terminate()` to decide whether to retry or abandon.

**Reference files**:
- `crates/roko-core/src/verdict.rs:51` -- `Verdict` struct with `score`, `passed`, `details`
- `crates/roko-gate/src/ratchet.rs:21` -- `GateRatchet` monotonic rung tracking
- `crates/roko-gate/src/feedback.rs:53` -- `GateFeedback` severity classification
- `crates/roko-gate/src/gate_pipeline.rs:68` -- `GatePipeline` that produces verdicts
- `crates/roko-cli/src/orchestrate.rs` -- retry loop where PRM should drive decisions
- `docs/04-verification/07-process-reward-models.md` -- full PRM spec

**Depends on**: None

**Accept when**:
- [x] `pub struct ProcessRewardModel` in `crates/roko-gate/src/process_reward.rs`
- [x] `promise()` returns f64 prediction of eventual success
- [x] `progress()` returns f64 trajectory delta between turns
- [x] `should_terminate()` drives early abandonment on low Promise
- [ ] Wired into orchestrator retry loop (ProcessRewardModel not referenced in orchestrate.rs)
- [x] `cargo test -p roko-gate`

**Verify**:
```bash
grep -rn 'ProcessRewardModel\|promise\|progress' crates/roko-gate/src/process_reward.rs
grep -rn 'ProcessRewardModel' crates/roko-cli/src/orchestrate.rs
cargo test -p roko-gate
```

**Priority**: P1

### GATE-03: Evaluation lifecycle loops not wired (11 of 14)
- [x] Wire remaining feedback loops across 5 speed tiers

**Spec** (doc 09 §Evaluation Lifecycle): 14 feedback loops across 5 speed tiers. Each loop
has a specific trigger, data source, consumer, and cadence:

| Tier | Speed | Loops |
|------|-------|-------|
| Machine | <100ms | 1: Confidence calibration, 2: Context attribution, 3: Cost-effectiveness |
| Cognitive | 1-10s | 4: Error diagnosis, 5: Gate feedback → agent |
| Consolidation | 1-5min | 6: Gate pipeline (wired), 7: Skill extraction, 8: Retry (wired) |
| Retrospective | 10-30min | 9: Pattern discovery, 10: Playbook promotion, 11: Router (wired) |
| Meta | 1h+ | 12: Regression detection, 13: C-Factor, 14: Experiment evaluation |

The "Karpathy property": the system must be able to produce its own training data from
execution traces without human labeling.

**Current code**: Only 3 of 14 loops wired: Loop 6 (gate pipeline at
`crates/roko-gate/src/gate_pipeline.rs:68`), Loop 8 (retry in orchestrate.rs), Loop 11
(CascadeRouter at `crates/roko-learn/src/cascade_router.rs:1006`). The remaining 11 loops
have supporting infrastructure that EXISTS but is not CALLED from orchestrate.rs:
- `CalibrationTracker` at `crates/roko-learn/src/prediction.rs:125` (Loop 1)
- `ContextAssembler` at `crates/roko-neuro/src/context.rs:221` (Loop 2, no attribution output)
- `compute_pareto_frontier()` at `crates/roko-learn/src/pareto.rs:28` (Loop 3)
- Error enrichment at `crates/roko-learn/src/error_enrichment.rs` (Loop 4)
- `GateFeedback` at `crates/roko-gate/src/feedback.rs:53` (Loop 5, not fed back to agent)
- `SkillLibrary` at `crates/roko-learn/src/skill_library.rs:1010` (Loop 7)
- `PatternMiner` at `crates/roko-learn/src/pattern_discovery.rs:99` (Loop 9)
- `PlaybookRules` at `crates/roko-learn/src/playbook_rules.rs:173` (Loop 10)
- Regression detector at `crates/roko-learn/src/regression.rs` (Loop 12)
- C-Factor at `crates/roko-learn/src/c_factor.rs` (Loop 13)
- `ExperimentStore` at `crates/roko-learn/src/prompt_experiment.rs:395` (Loop 14)

**What to change**: In orchestrate.rs, wire each loop at the appropriate cadence:
- After each turn: call `CalibrationTracker::record()`, update cost-effectiveness
- After gate failure: call `error_enrichment::diagnose()`, feed `GateFeedback` back to agent prompt
- After every 5 episodes: call `SkillLibrary::extract_from_episodes()`
- After every 20 episodes: call `PatternMiner::discover()`, promote validated patterns to `PlaybookRules`
- After every plan: update regression detector, C-Factor, evaluate experiments

**Reference files**:
- `crates/roko-cli/src/orchestrate.rs` -- main loop (all wiring happens here)
- `crates/roko-learn/src/prediction.rs:125` -- `CalibrationTracker` (Loop 1)
- `crates/roko-learn/src/pattern_discovery.rs:99` -- `PatternMiner` (Loop 9)
- `crates/roko-learn/src/skill_library.rs:1010` -- `SkillLibrary` (Loop 7)
- `crates/roko-learn/src/error_enrichment.rs` -- error diagnosis (Loop 4)
- `crates/roko-gate/src/feedback.rs:53` -- `GateFeedback` (Loop 5)
- `crates/roko-neuro/src/context.rs:221` -- `ContextAssembler` (Loop 2)
- `docs/04-verification/09-evaluation-lifecycle.md` -- 14-loop spec with cadences

**Depends on**: None

**Accept when**:
- [x] Machine-speed loops wired: calibration tracking (1), cost-effectiveness (3) -- CalibrationTracker loaded at :286, AgentEfficiencyEvent + fleet C-factor at :6407
- [x] Cognitive loops wired: error diagnosis (4), gate feedback → agent prompt (5) -- DiagnosisEngine::diagnose() at :5457/:9426, feedback_for_agent() at :11834
- [x] Consolidation loops wired: skill extraction (7) -- extract_pending_skill() at :14721 calls skill_library.extract_skill()
- [x] Retrospective loops wired: pattern discovery (9), playbook promotion (10) -- pattern_miner().lock().discover() at :9745, playbook.record() at :9937
- [ ] `cargo test --workspace` passes

**Verify**:
```bash
grep -rn 'CalibrationTracker\|pattern_miner\|skill_library\|error_enrichment\|GateFeedback' crates/roko-cli/src/orchestrate.rs
cargo test --workspace
```

**Priority**: P1 (loops 1-5, 7, 9-10), P2 (loops 12-14)

### GATE-04: Advanced gate composition (parallel, voting, fallback)
- [x] Implement parallel, voting, and fallback gate combinators

**Spec** (doc 03 §13 Gate Composition Algebra): Three composition combinators that wrap
inner gates and themselves implement the `Gate` trait:
1. **ParallelGate**: Runs N gates concurrently (tokio::join_all). Aggregates verdicts by
   taking the minimum score. If any gate fails, the aggregate fails. Use case: run
   CompileGate and LintGate simultaneously when they don't interfere.
2. **VotingGate**: Runs M gates, requires N-of-M to pass. Aggregate score = mean of passing
   verdicts. Use case: multiple reviewers must agree (2-of-3 for code review).
3. **FallbackGate**: Tries primary gate; if it fails, tries fallback. First passing verdict
   wins. Use case: try fast integration test, fall back to full test suite on failure.

**Current code** (`crates/roko-gate/src/gate_pipeline.rs:68`): `GatePipeline` runs gates
sequentially with short-circuit on first failure. No parallel execution, no voting, no
fallback. The `Gate` trait at `crates/roko-core/src/traits.rs:118` is
`async fn verify(&self, signal: &Engram, ctx: &GateContext) -> Verdict`. `Verdict` at
`crates/roko-core/src/verdict.rs:51` has `score: f64`, `passed: bool`, `details: String`.

**What to change**: Create `crates/roko-gate/src/composition.rs` with:
```rust
pub struct ParallelGate { gates: Vec<Box<dyn Gate>> }
pub struct VotingGate { gates: Vec<Box<dyn Gate>>, required_passes: usize }
pub struct FallbackGate { primary: Box<dyn Gate>, fallback: Box<dyn Gate> }
```
Each implements `Gate`. Add builder methods: `Gate::parallel(gates)`, `Gate::voting(gates, n)`,
`Gate::fallback(primary, fallback)`. Expose from `gate_pipeline.rs` as alternative strategies.

**Reference files**:
- `crates/roko-gate/src/gate_pipeline.rs:68` -- `GatePipeline` (current sequential)
- `crates/roko-core/src/traits.rs:118` -- `Gate` trait signature
- `crates/roko-core/src/verdict.rs:51` -- `Verdict` struct for aggregation
- `docs/04-verification/03-gate-pipeline.md` -- §13 composition algebra spec

**Depends on**: None

**Accept when**:
- [x] `ParallelGate` runs gates concurrently via `tokio::join_all`, aggregate = min score
- [x] `VotingGate` requires N-of-M pass, aggregate = mean of passing scores
- [x] `FallbackGate` tries primary, falls back on failure, first pass wins
- [x] All three implement `Gate` trait
- [x] `cargo test -p roko-gate` passes

**Verify**:
```bash
grep -rn 'ParallelGate\|VotingGate\|FallbackGate' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-gate
```

**Priority**: P2

### GATE-05: Verdict-as-signal reentry -- downstream consumers
- [x] Wire verdict Engrams to Scorer and Router for downstream learning

**Spec** (doc 15 §Verdicts as Signals): Gate verdicts are first-class Engrams that re-enter
the cognitive loop. Four downstream consumers should react to verdict signals:
1. **Scorer**: Appraise verdict relevance (a compile error on a file the agent just modified
   scores higher than a pre-existing warning). Use `Kind::GateVerdict` to weight future
   routing decisions.
2. **Router**: Use verdict history for model selection (tasks that repeatedly fail compile
   should be routed to stronger models). `CascadeRouter` should query recent verdicts for
   the `(task_type, crate)` pair and adjust its confidence.
3. **Composer**: Inject recent verdicts into agent prompts (already partially done via
   `GateFeedback`).
4. **Dreams**: Replay verdict patterns during consolidation (Phase 2+).

**Current code**: Verdict emission is DONE: `Verdict` at `crates/roko-core/src/verdict.rs:51`,
`Kind::GateVerdict` at `crates/roko-core/src/kind.rs:43`, verdicts emitted as Engrams at
`crates/roko-cli/src/orchestrate.rs:13957` and `crates/roko-cli/src/run.rs:164`. Episodes
log verdicts via `GateVerdict` at `crates/roko-learn/src/episode_logger.rs:90`.

Missing: (1) No Scorer implementation reads `Kind::GateVerdict` engrams -- the `Scorer` trait
at `crates/roko-core/src/traits.rs:95` has no verdict-aware implementation. (2) `CascadeRouter`
at `crates/roko-learn/src/cascade_router.rs:1006` does not query verdict history when selecting
models -- it uses bandit rewards derived from final task outcomes, not per-gate verdicts.

**What to change**:
(1) Add a `VerdictAwareScorer` in `crates/roko-learn/src/` (or `crates/roko-core/src/`) that
implements the `Scorer` trait and reads `Kind::GateVerdict` engrams from the Substrate. Score
= recency_weight * severity_weight * relevance_to_current_task.
(2) In `CascadeRouter::select_model()`, query the episode log for recent `GateVerdict` entries
matching the current `(task_type, crate)` pair. If the model that produced the last attempt
has a streak of >2 compile failures, penalize its bandit reward by 0.5x.

**Reference files**:
- `crates/roko-core/src/verdict.rs:51` -- `Verdict` struct with `score: f64`, `passed: bool`, `details: String`
- `crates/roko-core/src/kind.rs:43` -- `Kind::GateVerdict` variant
- `crates/roko-core/src/traits.rs:95` -- `Scorer` trait signature
- `crates/roko-cli/src/orchestrate.rs:13957` -- where verdicts become engrams (already wired)
- `crates/roko-learn/src/cascade_router.rs:1006` -- `CascadeRouter` (add verdict history query)
- `crates/roko-learn/src/episode_logger.rs:90` -- episode log with `GateVerdict` entries
- `docs/04-verification/15-verdicts-as-signals.md` -- full spec for verdict re-entry

**Depends on**: None

**Accept when**:
- [x] Verdicts emitted as Engrams with `Kind::GateVerdict` (ALREADY DONE)
- [x] `VerdictAwareScorer` implements `Scorer` and weights verdict signals by recency and severity
- [x] `CascadeRouter` queries verdict history for the current `(task_type, crate)` pair
- [x] Model with streak of compile failures receives penalized reward
- [x] `cargo test --workspace` passes

**Verify**:
```bash
grep -rn 'Kind::GateVerdict' crates/roko-cli/src/orchestrate.rs
grep -rn 'VerdictAwareScorer\|verdict.*history\|verdict.*router' crates/roko-learn/src/ --include='*.rs'
cargo test --workspace
```

**Priority**: P1

### GATE-06: EvoSkills learning loop
- [x] Wire skill extraction from episodes with adversarial validation

**Spec** (doc 11 §EvoSkills): Three-tier learning hierarchy:
1. **Episodes** → raw execution traces (turns, tool calls, gate results)
2. **Patterns** → recurring structures discovered via trigram mining across episodes
3. **Playbook rules** → validated patterns promoted to reusable heuristics

The critical missing piece is adversarial surrogate verification: when a pattern is
discovered (e.g., "for Rust compile errors, read the error first, then edit the file"),
a DIFFERENT model than the one that generated the pattern must validate it by running a
task using the pattern and demonstrating improved outcomes. This prevents self-reinforcing
errors where a model learns from its own mistakes.

**Current code**: All three tiers have infrastructure:
- `EpisodeLogger` at `crates/roko-learn/src/episode_logger.rs` -- writes episodes to
  `.roko/learn/episodes.jsonl`
- `PatternMiner` at `crates/roko-learn/src/pattern_discovery.rs:99` with `discover()` at
  line 188 -- mines trigram patterns from episodes via `EpisodeView` trait
- `CrossEpisodeConsolidator` at `crates/roko-learn/src/pattern_discovery.rs:291` -- clusters
  patterns across episodes using HDC k-medoids
- `SkillLibrary` at `crates/roko-learn/src/skill_library.rs:1010` -- stores validated skills
- `PlaybookRules` at `crates/roko-learn/src/playbook_rules.rs:173` -- stores promoted rules
  with confidence dynamics
- `LearningRuntime::record_completed_run()` at `crates/roko-learn/src/runtime_feedback.rs:335`
  -- already calls `PatternMiner::ingest_episode()` and `SkillLibrary::record_use()`

Missing: (1) no periodic trigger for `PatternMiner::discover()` -- `ingest_episode()` is
called but `discover()` never is, (2) no adversarial validation step between pattern
discovery and playbook promotion, (3) `CrossEpisodeConsolidator` not called from any
runtime path.

**What to change**:
1. In `LearningRuntime::record_completed_run()`, add an episode counter. After every 20
   episodes, call `PatternMiner::discover()` to extract candidate patterns.
2. For each candidate pattern with support >= 5 episodes, create an adversarial validation
   task: select a different model (via `CascadeRouter` with forced model exclusion), run a
   representative task using the pattern as a playbook rule, check if the outcome improves.
3. Patterns that pass adversarial validation get promoted to `PlaybookRules` via
   `PlaybookRules::add_rule()` with initial confidence 0.50.
4. Wire `CrossEpisodeConsolidator` into the same periodic trigger.

**Reference files**:
- `crates/roko-learn/src/pattern_discovery.rs:99` -- `PatternMiner` struct, `discover()` at line 188
- `crates/roko-learn/src/pattern_discovery.rs:291` -- `CrossEpisodeConsolidator` (not wired)
- `crates/roko-learn/src/skill_library.rs:1010` -- `SkillLibrary` (skill storage)
- `crates/roko-learn/src/playbook_rules.rs:173` -- `PlaybookRules` (promotion target)
- `crates/roko-learn/src/runtime_feedback.rs:335` -- `LearningRuntime` (periodic trigger site)
- `crates/roko-learn/src/cascade_router.rs:1006` -- `CascadeRouter` (for adversarial model selection)
- `docs/04-verification/11-evoskills.md` -- EvoSkills spec

**Depends on**: None

**Accept when**:
- [x] `PatternMiner::discover()` called every 20 episodes from `LearningRuntime` -- pattern_discovery_every_n: 20 at runtime_feedback.rs:215, ingest at :893; discover() also called from orchestrate.rs:9745
- [ ] Patterns with support >= 5 trigger adversarial validation
- [ ] Adversarial validation uses a different model than the pattern source
- [ ] Validated patterns promoted to `PlaybookRules` with confidence 0.50
- [ ] `cargo test --workspace` passes

**Verify**:
```bash
grep -rn 'discover()\|adversarial\|promote.*playbook' crates/roko-learn/src/ --include='*.rs'
grep -rn 'discover\|pattern_miner' crates/roko-learn/src/runtime_feedback.rs
cargo test --workspace
```

**Priority**: P2

### GATE-07: Forensic replay API
- [x] Implement causal chain reconstruction from content-addressed artifacts

**Spec** (doc 12 §Forensic AI): Every gate verdict is linked to its source artifacts via
content-addressed hashes. A forensic replay reconstructs the causal chain for any task:
which agent produced which output, which gate verified it, what the verdict was, and what
evidence (compiler output, test results, diff) supports the verdict. This enables:
- Post-hoc auditing of any task outcome
- Regulatory compliance for safety-critical applications
- Root cause analysis when regressions appear

**Current code**: The building blocks exist but are not connected:
- `ArtifactStore` at `crates/roko-gate/src/artifact_store.rs` -- BLAKE3 content-addressed
  artifact storage with `store()`, `retrieve()`, `verify_integrity()` methods
- `EpisodeLogger` at `crates/roko-learn/src/episode_logger.rs` -- episode records with
  turn-by-turn agent actions and gate results
- Signal log at `.roko/signals.jsonl` -- append-only Engram log
- `ContentHash` type at `crates/roko-core/src/hash.rs` -- BLAKE3 hash wrapper
- `EventLog` at `crates/roko-orchestrator/src/event_log.rs` -- hash-chained event log with
  integrity verification
- Replay module at `crates/roko-dreams/src/replay.rs` -- dream replay (different purpose:
  consolidation playback, not forensic)

No `replay_task()` function that walks the content-hash links to reconstruct the full
causal chain.

**What to change**: Create `crates/roko-gate/src/forensic.rs` with:
```rust
pub struct ForensicReplay {
    artifact_store: ArtifactStore,
    episode_log_path: PathBuf,
    signal_log_path: PathBuf,
}

pub struct CausalChain {
    pub task_id: GlobalTaskId,
    pub agent_model: String,
    pub turns: Vec<TurnRecord>,
    pub verdicts: Vec<(Verdict, ContentHash)>,
    pub artifacts: Vec<(ContentHash, ArtifactMetadata)>,
    pub integrity_verified: bool,
}

impl ForensicReplay {
    pub fn replay_task(&self, task_id: &str) -> Result<CausalChain, ForensicError> { ... }
    pub fn verify_chain_integrity(&self, chain: &CausalChain) -> bool { ... }
}
```
Walk: task_id → find episodes matching task_id → for each episode, collect gate verdict
entries → for each verdict, look up artifact by content hash → verify BLAKE3 hash chain.
Expose via `roko replay --task <id>` CLI subcommand.

**Reference files**:
- `crates/roko-gate/src/artifact_store.rs` -- `ArtifactStore` with BLAKE3 content-addressing
- `crates/roko-learn/src/episode_logger.rs` -- episode records with `GateVerdict` entries
- `crates/roko-core/src/hash.rs` -- `ContentHash` BLAKE3 wrapper
- `crates/roko-orchestrator/src/event_log.rs` -- hash-chained event log for integrity
- `crates/roko-dreams/src/replay.rs` -- dream replay (pattern reference, different purpose)
- `docs/04-verification/12-forensic-ai-causal-replay.md` -- forensic replay spec

**Depends on**: None

**Accept when**:
- [x] `pub struct ForensicReplayBuilder` in `crates/roko-gate/src/forensic.rs`
- [x] `replay_task(task_id)` returns `CausalChain` with verdicts, artifacts, turns
- [x] `verify_chain_integrity()` validates BLAKE3 hash links
- [ ] `roko replay --task <id>` CLI command works (current `roko replay` only walks by hash, not by task-id)
- [x] `cargo test -p roko-gate` passes

**Verify**:
```bash
grep -rn 'ForensicReplay\|replay_task\|CausalChain' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-gate
```

**Priority**: P2

### GATE-08: Multi-gate coordination (Hotelling's T-squared)
- [x] Implement multi-gate joint anomaly detection

**Spec** (doc 06 §12 Multi-gate Hotelling's T-squared): When multiple gates shift together
(e.g., compile pass rate drops AND lint pass rate drops), this signals a systemic problem
(model degradation, environment issue) rather than a gate-specific issue. Hotelling's
T-squared statistic is the multivariate extension of the t-test: it tests whether a
p-dimensional observation vector differs significantly from the historical mean vector.

Formula: `T² = n × (x̄ - μ)ᵀ × S⁻¹ × (x̄ - μ)` where x̄ is the current gate pass rate
vector, μ is the historical mean, S is the covariance matrix, and n is the sample size.
When `T²` exceeds the chi-squared critical value at p degrees of freedom, signal a joint
anomaly.

**Current code** (`crates/roko-gate/src/gate_pipeline.rs:68`): `GatePipeline` runs gates
independently. Each gate produces its own `Verdict` (`crates/roko-core/src/verdict.rs:51`).
`AdaptiveThresholds` at `crates/roko-gate/src/adaptive_threshold.rs:47` tracks per-rung
EMA but has no cross-gate correlation -- each rung is tracked independently. `EwmaState`
at `crates/roko-learn/src/anomaly.rs:152` provides univariate anomaly detection only.

**What to change**: Create `crates/roko-gate/src/hotelling.rs` with:
```rust
pub struct HotellingDetector {
    pub dimension: usize,                // number of gate types tracked
    pub mean: Vec<f64>,                  // running mean per gate
    pub covariance: Vec<Vec<f64>>,       // p×p covariance matrix
    pub observations: usize,             // total observations
    pub threshold: f64,                  // chi-squared critical value
}

impl HotellingDetector {
    pub fn new(dimension: usize, alpha: f64) -> Self { ... }
    pub fn update(&mut self, gate_pass_rates: &[f64]) { ... }
    pub fn t_squared(&self, current: &[f64]) -> f64 { ... }
    pub fn is_anomalous(&self, current: &[f64]) -> bool {
        self.t_squared(current) > self.threshold
    }
}
```
Wire into `AdaptiveThresholds`: after each complete pipeline run (all rungs evaluated),
collect the pass/fail vector and feed to `HotellingDetector::update()`. If anomalous,
emit a conductor alert via the event bus.

**Reference files**:
- `crates/roko-gate/src/gate_pipeline.rs:68` -- `GatePipeline` (produces per-gate verdicts)
- `crates/roko-gate/src/adaptive_threshold.rs:47` -- `AdaptiveThresholds` (integration point)
- `crates/roko-learn/src/anomaly.rs:152` -- `EwmaState` (univariate reference)
- `docs/04-verification/06-adaptive-thresholds.md` -- §12 Hotelling's T-squared spec

**Depends on**: GATE-01 (SPC foundations)

**Accept when**:
- [x] `pub struct HotellingDetector` with `t_squared()` and `is_anomalous()` methods
- [x] Covariance matrix maintained across pipeline runs
- [x] Joint anomaly detection triggers when T² exceeds chi-squared threshold
- [x] Wired into `AdaptiveThresholds::observe_pipeline()` for joint anomaly detection
- [x] `cargo test -p roko-gate` passes

**Verify**:
```bash
grep -rn 'HotellingDetector\|t_squared\|joint_anomaly' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-gate
```

**Priority**: P2

### GATE-09: Autonomous evaluation generation pipeline
- [x] Wire test generation → validation → registration pipeline before implementation

**Spec** (doc 10 §Autonomous Eval Generation): Before the implementation agent starts, a
separate test-generation agent reads the task spec and generates targeted test cases. These
tests are validated against the current codebase (new functionality tests should FAIL, baseline
tests should PASS). Validated tests are registered with `GeneratedTestGate` (Rung 4). The
implementation agent then produces code; Rung 4 runs generated tests against the new code.
This is automated TDD: the system generates tests, the agent generates the implementation,
the gate verifies alignment. Three strategies: example-based (concrete I/O pairs),
property-based (invariants via proptest), and mutation-based (mutant detection).

**Current code**: `GeneratedTestGate` at `crates/roko-gate/src/generated_test_gate.rs:178`
exists and is wired as Rung 4 at `crates/roko-cli/src/orchestrate.rs:14227`. It can run
pre-registered test artifacts from `ArtifactStore`. `PropertyTestGate` at
`crates/roko-gate/src/property_test_gate.rs:46` exists as Rung 5. However, the **generation
pipeline** is missing -- no separate test-generation agent is spawned before implementation,
no test validation against current code, no test registration flow. The gate infrastructure
exists but is not fed by automated test generation.

**What to change**: Add a test generation phase to the orchestrator. Before dispatching
the implementation agent for a task:
1. Spawn a test-generation agent (role: TestGenerator) with the task spec
2. Collect generated test code
3. Validate: compile and run against current code (expect failures for new features)
4. Register validated tests with `GeneratedArtifactStore`
5. Proceed with implementation agent
6. `GeneratedTestGate` runs registered tests against the agent's output

Wire into `orchestrate.rs` in the `Implementing` → `SpawnAgent` flow, adding a pre-step.

**Reference files**:
- `crates/roko-gate/src/generated_test_gate.rs:178` -- `GeneratedTestGate` (consumer of generated tests)
- `crates/roko-gate/src/generated_test_gate.rs:1` -- `ArtifactStore` for test registration
- `crates/roko-gate/src/property_test_gate.rs:46` -- `PropertyTestGate` (complementary)
- `crates/roko-cli/src/orchestrate.rs:14227` -- where `GeneratedTestGate` is instantiated
- `docs/04-verification/10-autonomous-eval-generation.md` -- full pipeline spec

**Depends on**: None (gate infrastructure already exists)

**Accept when**:
- [ ] Test-generation agent spawned before implementation agent
- [ ] Generated tests validated against current codebase (new = must fail, existing = must pass)
- [ ] Validated tests registered with `GeneratedArtifactStore`
- [ ] `GeneratedTestGate` runs registered tests after implementation
- [ ] `cargo test -p roko-gate` passes
- [ ] `cargo test -p roko-cli` passes

**Verify**:
```bash
grep -rn 'TestGenerator\|test_generation\|pre_implementation' crates/roko-cli/src/orchestrate.rs
grep -rn 'GeneratedArtifactStore\|register_test' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-gate
cargo test -p roko-cli
```

**Priority**: P1

---

## Verify
```bash
cargo test -p roko-gate
cargo test --workspace
```
