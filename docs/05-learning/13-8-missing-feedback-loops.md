# Eight Missing Cybernetic Feedback Loops

> **Implementation plan:** `modelrouting/17-meta-learning-and-corrections.md` (tasks 2O.01–2O.13)
> **PRD source:** `refactoring-prd/07-implementation-priorities.md` (Tier 1M table)
> **Theoretical basis:** Ashby's Law of Requisite Variety, Beer's Viable System Model, Good Regulator Theorem
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md), [14-stability-mechanisms](14-stability-mechanisms.md)


> **Implementation**: Shipping

---

## Purpose

The Roko learning system was built in layers: episodes first, then patterns, then bandits, then routing. Each layer works, but they don't yet talk to each other. The eight missing feedback loops are the inter-layer connections that close the cybernetic circuit — signals that flow from one subsystem's output to another subsystem's input, creating the self-regulating behavior that distinguishes a learning system from a collection of independent optimizers.

These eight loops are the organizing concept for Tier 1M (missing) in the implementation priority roadmap. Each loop has a clear source (where the signal originates), target (where it should flow), and mechanism (how the signal is transformed into action).

---

## The Eight Loops

```
┌─────────────────────────────────────────────────────────────────────┐
│                    EIGHT FEEDBACK LOOPS                              │
│                                                                     │
│  1. Health → Routing     Provider circuit breaker → candidate set   │
│  2. Conductor → Routing  System load signals → routing bias         │
│  3. Section → Scaffold   Section effectiveness → prompt weights     │
│  4. Failure → Replanning Gate failures → plan revision              │
│  5. Skills → Prompts     Skill library → prompt injection           │
│  6. Cost → Routing       Budget pressure → model selection          │
│  7. Latency → Reward     Response latency → bandit reward signal    │
│  8. Experiments → Static Experiment winners → static routing table  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Loop 1: Health → Routing

**Source:** `ProviderHealthRegistry` (circuit breaker state per provider)
**Target:** `CascadeRouter::select()` (candidate model filtering)
**Mechanism:** Before scoring candidates, filter out models whose provider circuit breaker is Open.

```
ProviderHealthRegistry::is_available("anthropic") → false (circuit Open)
    │
    ▼
CascadeRouter excludes all anthropic models from candidate set
    │
    ▼
Routes to openrouter or other available provider
```

**Status:** Wired. The cascade router calls `is_available()` during candidate scoring.

**Impact:** Prevents routing to degraded providers, reducing retry waste and improving first-attempt pass rates.

See: [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)

---

## Loop 2: Conductor → Routing

**Source:** Conductor subsystem (system load, resource utilization, queue depth)
**Target:** `CascadeRouter::select()` (routing bias)
**Mechanism:** When system load is high, bias toward cheaper/faster models. When load is low, allow more expensive/thorough models.

```
Conductor::system_load() → 0.85 (high load)
    │
    ▼
CascadeRouter bias: PreferCheaper
    │
    ▼
Routes to faster model to reduce queue pressure
```

**Status:** Partially wired. The C-Factor provides an aggregate performance signal that influences routing bias, but direct conductor→routing wiring (queue depth, active agent count) is not yet implemented.

**Impact:** Prevents resource exhaustion during high-load periods by dynamically adjusting quality-cost tradeoffs.

See: [07-conductor](../07-conductor/INDEX.md)

---

## Loop 3: Section → Scaffold

**Source:** `PromptSectionMeta` from efficiency events (per-section token attribution + gate outcomes)
**Target:** Prompt composer section weights (priority values used during context assembly)
**Mechanism:** Track which prompt sections correlate with gate passes. Increase weight of sections that correlate with success; decrease weight of sections that consume tokens without contributing to outcomes.

```
PromptSectionMeta { name: "workspace_map", tokens: 2000, was_truncated: false }
    │ + gate outcome: pass
    │
    ▼
Section effectiveness tracker:
    workspace_map: included in 50 turns, 35 passed (70% when included)
    workspace_map: excluded in 20 turns, 18 passed (90% when excluded)
    → workspace_map may be HURTING pass rate. Lower its priority.
```

**Status:** Data collection wired (efficiency events capture section metadata). The feedback path from section effectiveness to composer weights is defined in implementation plan 2J.07 but not yet wired.

**Impact:** This is the highest-leverage self-improvement loop. Adaptive context assembly can reduce prompt size by 30-50% while improving pass rates, because sections that waste the agent's attention budget are demoted.

See: [03-composition](../03-composition/INDEX.md) for the prompt assembly pipeline.

---

## Loop 4: Failure → Replanning

**Source:** Gate failure patterns (repeated failures on the same task, regression alerts)
**Target:** Plan generator (re-decompose the failing task)
**Mechanism:** When a task fails N consecutive times, trigger replanning: break the task into smaller subtasks, change the approach, or escalate to a human review.

```
Task T3: fail (iteration 1) → fail (iteration 2) → fail (iteration 3)
    │
    ▼
Failure→Replanning trigger:
    ├── Analyze failure pattern (same error? different errors?)
    ├── Generate alternative decomposition
    └── Create new subtasks T3a, T3b, T3c
```

**Status:** Not wired. The orchestrator currently retries tasks with the same decomposition. The replanning feedback path requires integration with the plan generator (`roko prd plan`).

**Impact:** Prevents the system from burning budget on intractable tasks. Replanning turns a hard task into multiple easier tasks that may succeed individually.

---

## Loop 5: Skills → Prompts

**Source:** `SkillLibrary` (accumulated skills with confidence scores)
**Target:** Prompt composer (skill injection into agent prompts)
**Mechanism:** When a new task matches a skill's trigger pattern (file paths, task category, tags), inject the skill's prompt template into the agent's system prompt.

```
New task: modify crates/roko-core/src/config/schema.rs
    │
    ▼
SkillLibrary::search_by_files(["crates/roko-core/src/config/schema.rs"])
    │
    ▼
Match: skill "config_schema_extension" (confidence: 0.87)
    │
    ▼
Inject into prompt:
    "Recommended approach (from 12 successful similar tasks):
     1. Add new field to the config struct
     2. Add serde default annotation
     3. Update the TOML schema documentation
     4. Add a test in config_tests.rs"
```

**Status:** Partially wired. The skill library stores and retrieves skills. The injection path from skill library to prompt composer requires integration with the `SystemPromptBuilder`.

**Impact:** Reduces iterations by providing agents with proven approaches. The 100th modification to a crate is dramatically cheaper than the 1st because the skill library has accumulated the crate's patterns.

See: [02-skill-library-voyager](02-skill-library-voyager.md)

---

## Loop 6: Cost → Routing

**Source:** Budget guardrails (per-task, per-session, per-day cost tracking)
**Target:** `CascadeRouter::select()` (model tier bias)
**Mechanism:** When spending approaches budget limits, force the router to select cheaper models.

```
Session cost: $45.60 / $50.00 limit (91.2%)
    │
    ▼
BudgetGuardrail::check() → BudgetAction::Block
    │
    ▼
CascadeRouter: only consider models cheaper than $0.10/M tokens
```

**Status:** Cost tracking wired (CostsLog, CostRecord). Budget guardrail enforcement is defined in implementation plan 2H.05–2H.10 but not yet integrated into the cascade router's selection path.

**Impact:** Prevents cost overruns by dynamically adjusting the quality-cost tradeoff. The system degrades gracefully (cheaper models, lower quality) rather than halting entirely.

See: [08-cost-normalization](08-cost-normalization.md)

---

## Loop 7: Latency → Reward

**Source:** `LatencyRegistry` (per-model, per-provider latency statistics)
**Target:** Bandit reward signal (used to update LinUCB/UCB1 arms)
**Mechanism:** Include latency as a component of the reward signal, so the bandit learns to avoid slow models when latency SLAs are tight.

```
Model A: pass_rate=0.90, avg_latency=2000ms, latency_sla=1500ms
Model B: pass_rate=0.85, avg_latency=800ms, latency_sla=1500ms
    │
    ▼
Reward adjustment:
    Model A: base_reward=1.0, latency_penalty=0.30 → adjusted=0.70
    Model B: base_reward=1.0, latency_penalty=0.00 → adjusted=1.00
    │
    ▼
Bandit learns to prefer Model B when latency SLA is tight
```

**Status:** Latency tracking wired (`LatencyRegistry` with EWMA per model/provider, p50/p95/p99 percentiles). The reward adjustment path is defined in implementation plan 2G.14 but not yet integrated into the bandit update function.

**Impact:** Prevents the bandit from selecting high-latency models that violate SLAs. Particularly important for interactive use cases where the human is waiting for results.

---

## Loop 8: Experiments → Static

**Source:** `ExperimentStore` (concluded experiments with identified winners)
**Target:** Static routing table (stage-1 defaults)
**Mechanism:** When a prompt experiment concludes with a clear winner, update the static configuration to use the winning variant. When a model experiment identifies the best model for a (role, complexity) pair, update the static routing table.

```
Experiment "system-prompt-v2": winner = variant "concise" (89% pass rate vs 72% baseline)
    │
    ▼
Update static config:
    system_prompt_section.constraints = "concise" variant text
    │
    ▼
All future tasks use the winning variant by default
```

**Status:** Experiment store wired (tracks variants, outcomes, UCB1 selection). The promotion path from concluded experiments to static configuration is defined in implementation plan 2O.12 but not yet automated.

**Impact:** Experiment results are currently ephemeral — they influence routing while the experiment is running, but the winner isn't persisted into the static config. Closing this loop makes experiment improvements permanent.

---

## Summary Table

| # | Loop | Source | Target | Status |
|---|------|--------|--------|--------|
| 1 | Health → Routing | ProviderHealthRegistry | CascadeRouter candidate filter | **Wired** |
| 2 | Conductor → Routing | Conductor load signals | CascadeRouter bias | Partial (via C-Factor) |
| 3 | Section → Scaffold | PromptSectionMeta | Composer section weights | Data collection wired |
| 4 | Failure → Replanning | Gate failure patterns | Plan generator | Not wired |
| 5 | Skills → Prompts | SkillLibrary | SystemPromptBuilder | Partial |
| 6 | Cost → Routing | Budget guardrails | CascadeRouter model tier | Data collection wired |
| 7 | Latency → Reward | LatencyRegistry | Bandit reward signal | Data collection wired |
| 8 | Experiments → Static | ExperimentStore | Static config | Not wired |

---

## Cybernetic Theory

These eight loops implement the core principle of cybernetics: **negative feedback for stability** (Ashby's Law of Requisite Variety). Each loop detects a deviation from desired behavior and applies a corrective signal:

- Provider health degrades → route away (loop 1)
- System overloaded → use cheaper models (loop 2)
- Section wasteful → reduce its weight (loop 3)
- Task intractable → decompose differently (loop 4)
- Skill available → inject it (loop 5)
- Budget exhausted → downgrade quality (loop 6)
- Latency excessive → penalize slow models (loop 7)
- Experiment concluded → lock in winner (loop 8)

The compound effect of all eight loops operating simultaneously is that the system converges toward an optimal operating point without manual tuning. See [14-stability-mechanisms](14-stability-mechanisms.md) for how oscillation is prevented, and [17-adas-and-autocatalytic](17-adas-and-autocatalytic.md) for why the compound improvement rate can be super-linear.

---

## Wiring recipes

Each unwired loop has a concrete implementation path. This section specifies the exact source/target structs, code paths, and estimated LOC for each.

### Recipe: Loop 2 -- Conductor -> Routing

```
Source struct:  roko-conductor::SystemLoadSnapshot
  Fields: cpu_load: f32, memory_pct: f32, active_agents: u32, queue_depth: u32
  File: crates/roko-conductor/src/load.rs (exists)

Target struct:  roko-learn::CascadeRouter
  Method: select(&self, task: &TaskSignal, ctx: &RoutingContext) -> ModelChoice
  File: crates/roko-learn/src/cascade_router.rs

Wiring path:
  1. Add RoutingContext.system_load: Option<SystemLoadSnapshot>
  2. In orchestrate.rs, read SystemLoadSnapshot before calling CascadeRouter::select()
  3. In CascadeRouter::select(), when system_load.active_agents >= conductor.max_agents * 0.8:
     bias = RoutingBias::PreferCheaper
  4. Apply bias as a cost_weight multiplier (1.5x) during candidate scoring

Estimated LOC: ~45
  - 10 lines: RoutingContext field addition
  - 15 lines: load snapshot read in orchestrate.rs
  - 20 lines: bias application in cascade_router.rs
```

### Recipe: Loop 3 -- Section -> Scaffold

```
Source struct:  roko-learn::EfficiencyEvent
  Fields: section_meta: Vec<PromptSectionMeta>, gate_passed: bool
  File: crates/roko-learn/src/efficiency.rs

Target struct:  roko-compose::SectionWeights
  Fields: weights: HashMap<String, f32>   (section name -> priority modifier)
  File: crates/roko-compose/src/budget.rs (new struct)

Wiring path:
  1. Create SectionWeights struct in roko-compose::budget
  2. Add SectionEffectivenessTracker to roko-learn:
     - Tracks (section_name, included, gate_passed) tuples
     - Computes per-section pass-rate-when-included vs pass-rate-when-excluded
     - Emits weight adjustments when delta > 5% over 50+ samples
  3. In orchestrate.rs, load SectionWeights from .roko/learn/section-weights.json
  4. Pass SectionWeights to SystemPromptBuilder
  5. In budget allocation, multiply section max_tokens by weight modifier

Estimated LOC: ~120
  - 25 lines: SectionWeights struct + serde
  - 50 lines: SectionEffectivenessTracker (accumulator + computation)
  - 15 lines: persistence to/from JSON
  - 15 lines: orchestrate.rs loading + passing
  - 15 lines: budget.rs integration
```

### Recipe: Loop 4 -- Failure -> Replanning

```
Source struct:  GateVerdict (from roko-learn::episode_logger)
  Fields: gate: String, passed: bool, signature: Option<String>
  File: crates/roko-learn/src/episode_logger.rs

Target:  roko-cli::prd::plan (the plan generation subcommand)
  File: crates/roko-cli/src/prd.rs

Wiring path:
  1. In orchestrate.rs, track consecutive failures per task:
     task_failures: HashMap<TaskId, Vec<GateVerdict>>
  2. When task_failures[task_id].len() >= gates.max_iterations:
     a. Analyze failure signatures (same error repeated? different errors each time?)
     b. Generate a replanning prompt with failure context
     c. Call the plan generation agent with the failing task + failure analysis
     d. Replace the failing task with the generated subtasks in the plan DAG
  3. The new subtasks inherit the original task's dependencies

Estimated LOC: ~80
  - 20 lines: failure tracking in orchestrate.rs
  - 30 lines: failure analysis (signature grouping, pattern detection)
  - 30 lines: replanning agent dispatch + subtask insertion
```

### Recipe: Loop 5 -- Skills -> Prompts

```
Source struct:  roko-learn::SkillEntry
  Fields: trigger: SkillTrigger, template: String, confidence: f32
  File: crates/roko-learn/src/skill_library.rs

Target struct:  roko-compose::SystemPromptBuilder
  Method: add_skill_section(skills: &[SkillEntry])
  File: crates/roko-compose/src/system_prompt_builder.rs

Wiring path:
  1. In orchestrate.rs, before building the system prompt:
     let skills = skill_library.search_by_task(&task);
  2. Filter to skills with confidence >= 0.5
  3. Call system_prompt_builder.add_skill_section(skills)
  4. The skill section is priority 3 (Medium), max_tokens 500

Estimated LOC: ~55
  - 10 lines: skill search call in orchestrate.rs
  - 20 lines: add_skill_section method in SystemPromptBuilder
  - 15 lines: skill section template formatting
  - 10 lines: budget allocation entry for skill section
```

### Recipe: Loop 6 -- Cost -> Routing

```
Source struct:  roko-learn::CostsLog
  Fields: records: Vec<CostRecord>
  File: crates/roko-learn/src/costs.rs

Target struct:  roko-learn::CascadeRouter
  File: crates/roko-learn/src/cascade_router.rs

Wiring path:
  1. Add BudgetGuardrail struct to roko-learn:
     - Tracks cumulative cost from CostsLog
     - Compares against budget.max_plan_usd and budget.max_session_usd
     - Returns BudgetAction: Allow, Warn, or Block
  2. In CascadeRouter::select(), call budget_guardrail.check():
     - If Warn: multiply cost_weight by 2.0 (bias toward cheaper models)
     - If Block: filter candidates to only those cheaper than $0.10/M input tokens
  3. In orchestrate.rs, update BudgetGuardrail after each agent turn

Estimated LOC: ~70
  - 30 lines: BudgetGuardrail struct + check()
  - 20 lines: CascadeRouter integration
  - 20 lines: orchestrate.rs update calls
```

### Recipe: Loop 7 -- Latency -> Reward

```
Source struct:  roko-learn::LatencyRegistry
  Fields: ewma: HashMap<String, f64>, percentiles: HashMap<String, Percentiles>
  File: crates/roko-learn/src/latency.rs

Target:  Bandit reward signal computation
  File: crates/roko-learn/src/cascade_router.rs (reward_for method)

Wiring path:
  1. In CascadeRouter::reward_for(), add latency component:
     let latency_ms = latency_registry.ewma_for(model);
     let latency_sla = routing.latency_sla_ms;  // new config field
     let latency_reward = (1.0 - (latency_ms / latency_sla as f64)).clamp(0.0, 1.0);
  2. Incorporate into composite reward:
     reward = quality_weight * quality + cost_weight * cost + latency_weight * latency_reward
  3. Add routing.latency_sla_ms to RoutingConfig (default: 5000ms)

Estimated LOC: ~35
  - 10 lines: latency reward computation
  - 10 lines: composite reward update
  - 15 lines: config field + default
```

### Recipe: Loop 8 -- Experiments -> Static

```
Source struct:  roko-learn::ExperimentStore
  Fields: experiments: HashMap<String, Experiment>
  File: crates/roko-learn/src/experiments.rs

Target:  roko.toml (static configuration)
  File: crates/roko-core/src/config/schema.rs

Wiring path:
  1. Add ExperimentStore::concluded_winners() -> Vec<ExperimentWinner>
     - Returns experiments where one variant has statistically significant advantage
     - Uses chi-squared test or simple threshold (>5% delta, >50 samples)
  2. Add roko config apply-experiments subcommand:
     - Reads concluded winners
     - Updates roko.toml with winning values
     - Archives concluded experiments
  3. Optionally: auto-apply on plan completion (gated by learning.auto_apply_experiments)

Estimated LOC: ~90
  - 30 lines: concluded_winners() method
  - 40 lines: config apply-experiments CLI subcommand
  - 20 lines: TOML update logic
```

### Summary

| Loop | Estimated LOC | Complexity | Dependencies |
|---|---|---|---|
| 2: Conductor -> Routing | ~45 | Low | SystemLoadSnapshot already exists |
| 3: Section -> Scaffold | ~120 | Medium | New SectionEffectivenessTracker |
| 4: Failure -> Replanning | ~80 | Medium | Plan generation agent already exists |
| 5: Skills -> Prompts | ~55 | Low | SkillLibrary and SystemPromptBuilder both exist |
| 6: Cost -> Routing | ~70 | Low | CostsLog already exists |
| 7: Latency -> Reward | ~35 | Low | LatencyRegistry already exists |
| 8: Experiments -> Static | ~90 | Medium | New CLI subcommand |
| **Total** | **~495** | | |

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The primary target for loops 1, 2, 6, 7, 8.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Source for loop 1.
- **[08-cost-normalization](08-cost-normalization.md)** — Source for loop 6.
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Source for loop 5.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Prevents these loops from oscillating.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Measures the aggregate effect of all loops.
