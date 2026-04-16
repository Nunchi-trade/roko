# Section E: Feedback Loops & Integration (Tier 1M)

Source: `tmp/integrate-prds/09-REFACTORING-PRD-ADDITIONS.md`
These are the 8 cybernetic feedback loops that make the system self-improving.

---

## E.01 — Health → Routing

**Status**: NOT DONE
**Priority**: P0
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/provider_health.rs` — Health tracking (exists)
- `crates/roko-learn/src/cascade_router.rs` — Add health filtering
- `crates/roko-cli/src/orchestrate.rs` — Wire health check before routing

### Context

Unhealthy providers (repeated timeouts, 5xx errors) should be automatically filtered from CascadeRouter candidates. `provider_health.rs` already tracks health state. CascadeRouter already exists. The missing piece is connecting them: check health before routing.

### Implementation details

1. In `provider_health.rs`, ensure `ProviderHealth` exposes `fn is_healthy(&self, provider: &str) -> bool`
2. In `cascade_router.rs`, add `fn filter_unhealthy(&self, candidates: &[ModelCandidate], health: &ProviderHealth) -> Vec<ModelCandidate>`:
   - Remove candidates whose provider is marked unhealthy
   - If ALL candidates are unhealthy, return the least-unhealthy one (never return empty)
3. In `orchestrate.rs`, before calling `CascadeRouter::select()`:
   - Load `ProviderHealth` state
   - Call `filter_unhealthy()` on candidates
   - Pass filtered list to router

### Verify command

```bash
cargo build -p roko-cli 2>&1 | tail -5
cargo test -p roko-learn --lib -- provider_health 2>&1 | tail -10
```

---

## E.02 — Conductor → Routing

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~50
**Dependencies**: None

### Files to modify

- `crates/roko-conductor/src/conductor.rs` — Emit routing bias signals
- `crates/roko-learn/src/cascade_router.rs` — Accept conductor bias
- `crates/roko-cli/src/orchestrate.rs` — Wire conductor feedback into routing

### Context

Conductor watchers detect live load pressure and model failures. This information should bias routing — e.g., if a model just failed, deprioritize it; if load is high, prefer cheaper models.

### Implementation details

1. Define `RoutingBias` struct in `roko-conductor`:
   ```rust
   pub struct RoutingBias {
       pub deprioritize: Vec<String>,  // model IDs to avoid
       pub prefer_cheaper: bool,       // true when load/cost pressure detected
       pub reason: String,
   }
   ```
2. Add `fn routing_bias(&self) -> RoutingBias` to `Conductor`
3. In `cascade_router.rs`, add `fn apply_bias(&self, candidates: &mut [ModelCandidate], bias: &RoutingBias)`:
   - Reduce score of deprioritized models
   - When `prefer_cheaper`, boost cheaper tier candidates
4. Wire in `orchestrate.rs`: query conductor for bias before routing

### Verify command

```bash
cargo build -p roko-conductor -p roko-learn -p roko-cli 2>&1 | tail -5
cargo test -p roko-conductor --lib -- routing_bias 2>&1 | tail -10
```

---

## E.03 — Section → Scaffold (Prompt Section Lift)

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/section_effect.rs` — Section lift tracking (exists)
- `crates/roko-compose/src/system_prompt_builder.rs` — Apply lift weights at assembly time

### Context

`section_effect.rs` already tracks prompt section lift (which sections correlate with success). The missing piece is feeding those weights back into live prompt assembly so high-lift sections get priority budget and low-lift sections get trimmed.

### Implementation details

1. In `section_effect.rs`, ensure `SectionEffect` exposes `fn lift_weights(&self) -> HashMap<String, f64>`
2. In `system_prompt_builder.rs`, during assembly:
   - Load lift weights from `.roko/learn/section-effects.json`
   - Multiply section token budgets by lift weight (normalized so total budget unchanged)
   - Sections with lift < 0.5 get budget reduced by 50%
   - Sections with lift > 1.5 get budget increased by 50%
3. Log which sections were boosted/reduced to `.roko/learn/section-tuning.jsonl`

### Verify command

```bash
cargo build -p roko-compose -p roko-learn 2>&1 | tail -5
cargo test -p roko-compose --lib -- section 2>&1 | tail -10
```

---

## E.04 — Failure → Replanning

**Status**: PARTIAL
**Priority**: P0
**Estimated LOC**: ~60
**Dependencies**: None

### Files to modify

- `crates/roko-orchestrator/src/replan.rs` — Replan strategy implementation (exists, variants are NOT IMPLEMENTED)
- `crates/roko-orchestrator/src/executor/mod.rs` — Wire replanning on gate failure
- `crates/roko-cli/src/orchestrate.rs` — Handle replan results

### Context

`ReplanStrategy` enum exists with 3 variants (`Decompose`, `RetryWithEscalation`, `RegeneratePlan`) but all are marked NOT IMPLEMENTED. Gate failures currently retry or skip, but never re-plan. The system should be able to split a failing task, escalate the model, or regenerate the entire plan.

### Implementation details

1. Implement `ReplanStrategy::RetryWithEscalation` in `replan.rs`:
   - On gate failure, escalate model tier (haiku → sonnet → opus)
   - Modify task's `model_hint` in the DAG
   - Re-dispatch with higher tier
2. Implement `ReplanStrategy::Decompose`:
   - Call planner agent with failing task + error context
   - Agent generates 2-3 sub-tasks that replace the failing task in the DAG
   - Insert sub-tasks with correct dependencies
3. Implement `ReplanStrategy::RegeneratePlan`:
   - On 3+ consecutive failures in same plan
   - Call `prd plan` with failure context appended
   - Replace entire plan DAG
4. Wire into executor: after gate failure, check failure count → select strategy → execute
5. In `orchestrate.rs`: handle `ReplanResult` (new tasks, modified DAG, regenerated plan)

### Verify command

```bash
cargo build -p roko-orchestrator -p roko-cli 2>&1 | tail -5
cargo test -p roko-orchestrator --lib -- replan 2>&1 | tail -10
```

---

## E.05 — Skills → Prompts

**Status**: PARTIAL
**Priority**: P1
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/skill_library.rs` — Skill retrieval by task category (exists)
- `crates/roko-compose/src/system_prompt_builder.rs` — Inject skills into prompt

### Context

`SkillLibrary` exists and extracts skills from successful task completions. The missing piece is injecting task-relevant skills into the system prompt at dispatch time so agents benefit from previously learned techniques.

### Implementation details

1. In `skill_library.rs`, ensure `SkillLibrary` exposes:
   - `fn relevant_skills(&self, task_category: &str, max: usize) -> Vec<Skill>`
   - Skills ranked by success rate and recency
2. In `system_prompt_builder.rs`:
   - Add `with_skills(skills: &[Skill])` method
   - Format skills as a `## Relevant Techniques` section
   - Each skill: title, when to use, how to apply, success rate
   - Insert after task brief but before anti-patterns
3. Budget: skills section gets max 500 tokens (trim oldest if over budget)

### Verify command

```bash
cargo build -p roko-compose -p roko-learn 2>&1 | tail -5
cargo test -p roko-compose --lib -- skill 2>&1 | tail -10
```

---

## E.06 — Cost → Routing

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~40
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/costs_log.rs` — Cost tracking (exists)
- `crates/roko-learn/src/cascade_router.rs` — Add cost pressure signal
- `crates/roko-cli/src/orchestrate.rs` — Wire cost check before routing

### Context

When cost spikes (e.g., expensive model used too often), routing should automatically force cheaper tiers until budget stabilizes. Cost tracking exists in `costs_log.rs` but doesn't influence routing.

### Implementation details

1. In `costs_log.rs`, add `fn recent_cost_rate(&self, window: Duration) -> f64`:
   - Sum costs in last N minutes
   - Return dollars-per-minute rate
2. In `costs_log.rs`, add `fn is_cost_spike(&self, threshold: f64) -> bool`:
   - True when recent cost rate > threshold (configurable, default 0.50 USD/min)
3. In `cascade_router.rs`, add `fn apply_cost_pressure(&self, candidates: &mut [ModelCandidate], spike: bool)`:
   - When spike: filter out T2 (opus) candidates, prefer T0 (haiku)
   - When not spike: no change
4. Wire in `orchestrate.rs`: check cost spike before routing, pass to `apply_cost_pressure()`

### Verify command

```bash
cargo build -p roko-learn -p roko-cli 2>&1 | tail -5
cargo test -p roko-learn --lib -- cost 2>&1 | tail -10
```

---

## E.07 — Latency → Reward

**Status**: NOT DONE
**Priority**: P1
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/cascade_router.rs` — Use actual latency in reward signal
- `crates/roko-learn/src/latency.rs` — Latency tracking (exists)

### Context

CascadeRouter currently uses static latency estimates for model routing reward. Should use actual measured latency so the router learns which models are fast/slow in practice.

### Implementation details

1. In `latency.rs`, ensure `LatencyTracker` exposes:
   - `fn record(&mut self, model: &str, latency_ms: u64)`
   - `fn mean_latency(&self, model: &str) -> Option<f64>`
   - `fn p95_latency(&self, model: &str) -> Option<f64>`
2. In `cascade_router.rs`:
   - Replace static latency estimate with `LatencyTracker::mean_latency()` when available
   - Incorporate latency into reward: `reward = quality_score - latency_penalty`
   - `latency_penalty = 0.1 * (actual_ms / expected_ms - 1.0).max(0.0)` — penalize slower than expected
3. Persist latency data alongside router state in `.roko/learn/cascade-router.json`

### Verify command

```bash
cargo build -p roko-learn 2>&1 | tail -5
cargo test -p roko-learn --lib -- latency 2>&1 | tail -10
```

---

## E.08 — Experiments → Static

**Status**: NOT DONE
**Priority**: P2
**Estimated LOC**: ~30
**Dependencies**: None

### Files to modify

- `crates/roko-learn/src/prompt_experiment.rs` — Experiment conclusion logic (exists)
- `crates/roko-learn/src/cascade_router.rs` — Accept static overrides
- `crates/roko-cli/src/orchestrate.rs` — Apply concluded experiments at startup

### Context

`ExperimentStore` in `.roko/learn/experiments.json` runs A/B experiments on prompts and routing. When an experiment concludes with a winner, that winner should be persisted as the new default so future runs don't re-experiment.

### Implementation details

1. In `prompt_experiment.rs`, add `fn concluded_winners(&self) -> Vec<ExperimentWinner>`:
   - `ExperimentWinner { experiment_id, parameter, winning_value, confidence }`
   - Only return experiments with confidence >= 0.95
2. Add `fn apply_winners(&self, winners: &[ExperimentWinner])` to `prompt_experiment.rs`:
   - Write winners to `.roko/learn/static-overrides.json`
   - Format: `{ "parameter_name": "winning_value", ... }`
3. In `cascade_router.rs`, add `fn load_static_overrides(&mut self, path: &Path)`:
   - Read static overrides and apply as default routing weights
4. In `orchestrate.rs` at startup:
   - Load experiments, call `concluded_winners()`
   - If any new winners, call `apply_winners()` and `load_static_overrides()`

### Verify command

```bash
cargo build -p roko-learn -p roko-cli 2>&1 | tail -5
cargo test -p roko-learn --lib -- experiment 2>&1 | tail -10
```
