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

**Status:** Wired for the live orchestration path. Composed prompts now emit per-section inclusion/drop metadata into efficiency events, `LearningRuntime` persists a section-effectiveness registry, and the next prompt build/feedforward pass reweights section priorities from those learned lift signals. The remaining gap is broader coverage outside the current orchestrator path plus more expressive weighting than the current priority-step adjustments.

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
| 3 | Section → Scaffold | PromptSectionMeta | Composer section weights | **Wired (live orchestration path)** |
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

## Detailed Data Flow Specifications

Each feedback loop has precise data flow requirements, latency constraints, and failure mode characteristics. This section formalizes these for implementation reference.

### Loop 1: Health → Routing — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  Provider Response    │────►│  ProviderHealthRegistry │────►│  CascadeRouter   │
│  (success/failure)    │     │  record_success/failure │     │  select()        │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
      Source type:                Transform:                   Sink type:
      ProviderResponse            ErrorClassifier →            is_available() →
      { status_code,              CircuitState update          bool filter on
        latency_ms,               (Closed/Open/HalfOpen)       candidate set
        error: Option }
```

**Latency requirement:** Real-time (< 1ms). Circuit state check is a HashMap lookup.

**Failure mode if loop breaks:** Router sends requests to a degraded provider → timeouts → wasted budget → cascading failures as the provider's queue backs up. Recovery: manual provider blacklist in roko.toml.

```rust
// Source type
pub struct ProviderResponse {
    pub provider_id: String,
    pub model: String,
    pub status_code: u16,
    pub latency_ms: u64,
    pub error: Option<ProviderError>,
    pub timestamp: DateTime<Utc>,
}

// Transform function
fn health_to_routing_transform(response: &ProviderResponse) -> CircuitAction {
    match response.error {
        None => CircuitAction::RecordSuccess,
        Some(ref err) => {
            let class = ErrorClassifier::classify(err);
            let cooldown = CooldownPolicy::for_class(&class);
            CircuitAction::RecordFailure { class, cooldown }
        }
    }
}

// Sink: CascadeRouter reads circuit state during select()
fn is_available(provider: &str, registry: &ProviderHealthRegistry) -> bool {
    matches!(registry.state(provider), CircuitState::Closed | CircuitState::HalfOpen)
}
```

---

### Loop 2: Conductor → Routing — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  SystemLoadSnapshot   │────►│  Load Threshold Check   │────►│  CascadeRouter   │
│  (cpu, mem, agents,   │     │  active_agents >=       │     │  routing bias    │
│   queue_depth)        │     │  max_agents * 0.8?      │     │  adjustment      │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Batch (every 5 episodes). System load changes slowly relative to task execution.

**Failure mode if loop breaks:** System routes to expensive models during high load → resource exhaustion → agent spawn failures → plan stalls. Recovery: manual cost ceiling in roko.toml.

```rust
// Source type (already exists in roko-conductor)
pub struct SystemLoadSnapshot {
    pub cpu_load: f32,
    pub memory_pct: f32,
    pub active_agents: u32,
    pub queue_depth: u32,
    pub timestamp: DateTime<Utc>,
}

// Transform function
fn conductor_to_routing_transform(
    load: &SystemLoadSnapshot,
    config: &ConductorConfig,
) -> RoutingBiasAdjustment {
    let agent_utilization = load.active_agents as f64 / config.max_agents as f64;
    let memory_pressure = load.memory_pct as f64 / 100.0;

    if agent_utilization > 0.8 || memory_pressure > 0.85 {
        RoutingBiasAdjustment::PreferCheaper { cost_weight_multiplier: 1.5 }
    } else if agent_utilization < 0.3 && memory_pressure < 0.50 {
        RoutingBiasAdjustment::AllowExpensive { quality_weight_multiplier: 1.2 }
    } else {
        RoutingBiasAdjustment::Neutral
    }
}

pub enum RoutingBiasAdjustment {
    PreferCheaper { cost_weight_multiplier: f64 },
    AllowExpensive { quality_weight_multiplier: f64 },
    Neutral,
}
```

---

### Loop 3: Section → Scaffold — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  PromptSectionMeta +  │────►│  SectionEffectiveness   │────►│  SectionWeights  │
│  gate_passed (from    │     │  Tracker (conditional   │     │  (HashMap<String │
│  efficiency events)   │     │  pass rate analysis)    │     │   , f32>)        │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Batch (every 20 episodes). Section effectiveness needs a meaningful sample before adjusting weights.

**Failure mode if loop breaks:** Wasteful prompt sections consume tokens without contributing to success → inflated costs and potentially confused agents. Recovery: manual section weights in roko.toml prompt configuration.

```rust
// Source type (already exists in efficiency events)
pub struct SectionEffectivenessInput {
    pub section_name: String,
    pub was_included: bool,
    pub tokens_consumed: u64,
    pub gate_passed: bool,
    pub role: String,
    pub complexity_band: String,
}

// Transform: conditional pass rate analysis
pub struct SectionEffectivenessTracker {
    /// Per-section: (included_count, included_pass_count, excluded_count, excluded_pass_count)
    stats: HashMap<String, SectionStats>,
}

pub struct SectionStats {
    pub included_count: u32,
    pub included_pass_count: u32,
    pub excluded_count: u32,
    pub excluded_pass_count: u32,
}

impl SectionStats {
    fn effectiveness_delta(&self) -> f64 {
        let included_rate = self.included_pass_count as f64 / self.included_count.max(1) as f64;
        let excluded_rate = self.excluded_pass_count as f64 / self.excluded_count.max(1) as f64;
        included_rate - excluded_rate
        // Positive = section helps, Negative = section hurts
    }
}

fn section_to_scaffold_transform(
    tracker: &SectionEffectivenessTracker,
    min_samples: u32,  // default: 50
    significance_delta: f64,  // default: 0.05
) -> HashMap<String, f32> {
    let mut weights = HashMap::new();
    for (name, stats) in &tracker.stats {
        if stats.included_count + stats.excluded_count < min_samples {
            weights.insert(name.clone(), 1.0); // Not enough data, neutral weight
            continue;
        }
        let delta = stats.effectiveness_delta();
        if delta > significance_delta {
            weights.insert(name.clone(), 1.0 + delta as f32); // Boost helpful sections
        } else if delta < -significance_delta {
            weights.insert(name.clone(), (1.0 + delta as f32).max(0.1)); // Reduce harmful sections
        } else {
            weights.insert(name.clone(), 1.0); // Neutral
        }
    }
    weights
}

// Sink: SectionWeights in roko-compose
pub struct SectionWeights {
    pub weights: HashMap<String, f32>,
    pub computed_at: DateTime<Utc>,
    pub episode_count: usize,
}
```

---

### Loop 4: Failure → Replanning — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  Consecutive gate     │────►│  Failure Analyzer       │────►│  Plan Generator  │
│  failures for task    │     │  (pattern detection,    │     │  (decompose into │
│  (Vec<GateVerdict>)   │     │   root cause grouping)  │     │   subtasks)      │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Per-task (triggered after max_iterations failures). Must complete before plan executor moves to next task.

**Failure mode if loop breaks:** System retries the same failing task indefinitely → budget burn on intractable tasks → plan stalls. Recovery: manual task skip or plan abort.

```rust
// Source type
pub struct FailureSequence {
    pub task_id: String,
    pub plan_id: String,
    pub failures: Vec<GateVerdict>,
    pub total_cost_burned: f64,
    pub models_tried: Vec<String>,
}

// Transform: failure analysis
pub struct FailureAnalysis {
    /// Are all failures the same error? (systematic issue)
    pub is_repeated_error: bool,
    /// Dominant error signature (if repeated).
    pub dominant_signature: Option<String>,
    /// Did model escalation help? (opus failed same as haiku = not a model issue)
    pub model_escalation_helped: bool,
    /// Recommended action.
    pub recommendation: FailureRecommendation,
}

pub enum FailureRecommendation {
    /// Decompose into smaller subtasks.
    Decompose { suggested_split: Vec<String> },
    /// Change approach entirely (different tool set, different strategy).
    ChangeApproach { reason: String },
    /// Escalate to human review.
    HumanReview { context: String },
    /// Skip task (it may be impossible given current capabilities).
    Skip { reason: String },
}

fn failure_to_replan_transform(seq: &FailureSequence) -> FailureAnalysis {
    let signatures: Vec<_> = seq.failures.iter()
        .filter_map(|v| v.signature.as_ref())
        .collect();

    let is_repeated = signatures.windows(2).all(|w| w[0] == w[1]);
    let model_set: HashSet<_> = seq.models_tried.iter().collect();
    let tried_multiple_models = model_set.len() >= 2;

    let recommendation = if is_repeated && tried_multiple_models {
        // Same error with multiple models = fundamental approach problem
        FailureRecommendation::Decompose {
            suggested_split: suggest_decomposition(&seq.task_id),
        }
    } else if seq.failures.len() > 5 && seq.total_cost_burned > 10.0 {
        // Many failures, high cost = escalate
        FailureRecommendation::HumanReview {
            context: format!("Task {} failed {} times, burned ${:.2}",
                seq.task_id, seq.failures.len(), seq.total_cost_burned),
        }
    } else {
        FailureRecommendation::ChangeApproach {
            reason: "Varied errors suggest the approach needs revision".into(),
        }
    };

    FailureAnalysis {
        is_repeated_error: is_repeated,
        dominant_signature: signatures.first().map(|s| s.to_string()),
        model_escalation_helped: !is_repeated && tried_multiple_models,
        recommendation,
    }
}
```

---

### Loop 5: Skills → Prompts — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  SkillLibrary         │────►│  Skill Matcher          │────►│  SystemPrompt    │
│  (accumulated skills  │     │  (tag + file + HDC      │     │  Builder layer 4 │
│   with confidence)    │     │   similarity search)    │     │  ("skills" sect) │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Per-task (< 5ms). Must complete during prompt assembly before agent dispatch.

**Failure mode if loop breaks:** Agents rediscover solutions that the skill library already contains → wasted iterations → higher cost. Recovery: skills still accumulate but aren't injected; no data loss.

```rust
// Source: SkillLibrary::search_by_task()
pub struct SkillMatch {
    pub skill_name: String,
    pub confidence: f64,
    pub match_type: SkillMatchType,
    pub prompt_template: String,
    pub max_tokens: usize,  // budget for this skill injection
}

pub enum SkillMatchType {
    /// Matched by file path overlap.
    FileMatch { overlap_files: Vec<String> },
    /// Matched by tag overlap.
    TagMatch { matching_tags: Vec<String> },
    /// Matched by HDC similarity to task context.
    HdcSimilarity { similarity: f64 },
}

// Transform: filter and rank
fn skills_to_prompt_transform(
    matches: Vec<SkillMatch>,
    max_skills: usize,  // default: 3
    min_confidence: f64,  // default: 0.50
    max_total_tokens: usize,  // default: 500
) -> Vec<SkillInjection> {
    let mut qualified: Vec<_> = matches.into_iter()
        .filter(|m| m.confidence >= min_confidence)
        .collect();
    qualified.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

    let mut injections = Vec::new();
    let mut token_budget = max_total_tokens;
    for skill in qualified.into_iter().take(max_skills) {
        if skill.max_tokens <= token_budget {
            token_budget -= skill.max_tokens;
            injections.push(SkillInjection {
                skill_name: skill.skill_name,
                template: skill.prompt_template,
                confidence: skill.confidence,
            });
        }
    }
    injections
}

// Sink: injected into SystemPromptBuilder
pub struct SkillInjection {
    pub skill_name: String,
    pub template: String,
    pub confidence: f64,
}
```

---

### Loop 6: Cost → Routing — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  CostsLog (running    │────►│  BudgetGuardrail        │────►│  CascadeRouter   │
│  cost accumulator)    │     │  check() → action       │     │  cost_weight or  │
│                       │     │                          │     │  candidate filter │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Per-task (< 1ms). Budget check is arithmetic comparison.

**Failure mode if loop breaks:** System exceeds budget → unexpected charges → operator loses trust in automated execution. Recovery: hard stop at 100% budget via separate watchdog process.

```rust
// Source: accumulated costs
pub struct BudgetState {
    pub task_cost_usd: f64,
    pub session_cost_usd: f64,
    pub day_cost_usd: f64,
    pub task_limit: f64,
    pub session_limit: f64,
    pub day_limit: f64,
}

// Transform: multi-level budget check
fn cost_to_routing_transform(state: &BudgetState) -> BudgetRoutingAction {
    // Check each level, return most restrictive action
    let task_pct = state.task_cost_usd / state.task_limit;
    let session_pct = state.session_cost_usd / state.session_limit;
    let day_pct = state.day_cost_usd / state.day_limit;

    let max_pct = task_pct.max(session_pct).max(day_pct);

    if max_pct >= 1.0 {
        BudgetRoutingAction::HardStop
    } else if max_pct >= 0.95 {
        BudgetRoutingAction::Block
    } else if max_pct >= 0.80 {
        BudgetRoutingAction::Downgrade {
            max_cost_per_m: 0.50, // Only allow models cheaper than $0.50/M
        }
    } else {
        BudgetRoutingAction::Continue
    }
}

pub enum BudgetRoutingAction {
    Continue,
    Downgrade { max_cost_per_m: f64 },
    Block,
    HardStop,
}
```

---

### Loop 7: Latency → Reward — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  LatencyRegistry      │────►│  Latency Reward         │────►│  Bandit Update   │
│  (EWMA per model,     │     │  Computation (SLA       │     │  (composite      │
│   p50/p95/p99)        │     │   compliance scoring)   │     │   reward signal) │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Per-episode. Latency reward is computed as part of the bandit update.

**Failure mode if loop breaks:** Bandit selects high-quality but slow models → SLA violations → user-facing delays. Recovery: manual latency SLA enforcement in roko.toml routing config.

```rust
// Source: LatencyRegistry state
pub struct LatencyStats {
    pub model: String,
    pub provider: String,
    pub ewma_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub sample_count: u64,
}

// Transform: latency → reward component
fn latency_to_reward_transform(
    stats: &LatencyStats,
    sla_ms: u64,  // from RoutingConfig, default: 5000
) -> f64 {
    let sla = sla_ms as f64;
    if stats.ewma_ms <= sla * 0.5 {
        1.0  // Well within SLA → full reward
    } else if stats.ewma_ms <= sla {
        // Linear decay from 1.0 to 0.5 as latency approaches SLA
        1.0 - 0.5 * ((stats.ewma_ms - sla * 0.5) / (sla * 0.5))
    } else {
        // Beyond SLA → penalty proportional to overshoot
        (0.5 * sla / stats.ewma_ms).max(0.0)
    }
}

// Sink: composite reward for bandit update
fn composite_reward(
    quality: f64,       // gate pass = 1.0, fail = 0.0
    cost_reward: f64,   // 1.0 - normalized_cost
    latency_reward: f64, // from transform above
    weights: &RewardWeights,
) -> f64 {
    weights.quality * quality
        + weights.cost * cost_reward
        + weights.latency * latency_reward
}

pub struct RewardWeights {
    pub quality: f64,   // default: 0.60
    pub cost: f64,      // default: 0.25
    pub latency: f64,   // default: 0.15
}
```

---

### Loop 8: Experiments → Static — Data Flow

```
┌──────────────────────┐     ┌────────────────────────┐     ┌─────────────────┐
│  ExperimentStore      │────►│  Significance Tester    │────►│  Static Config   │
│  (concluded expts     │     │  (chi-squared or        │     │  (roko.toml      │
│   with variant data)  │     │   z-test for winner)    │     │   updates)       │
└──────────────────────┘     └────────────────────────┘     └─────────────────┘
```

**Latency requirement:** Batch (on experiment conclusion, checked every 50 episodes). Config changes need human review.

**Failure mode if loop breaks:** Experiment results are transient — winners aren't persisted, so the system re-runs the same experiments indefinitely. Recovery: manual config update based on experiment logs.

```rust
// Source: ExperimentStore concluded experiments
pub struct ExperimentConclusion {
    pub experiment_id: String,
    pub section_name: String,
    pub winner_variant: String,
    pub winner_pass_rate: f64,
    pub baseline_pass_rate: f64,
    pub delta: f64,
    pub p_value: f64,
    pub sample_size: usize,
}

// Transform: statistical significance test
fn experiments_to_static_transform(
    conclusion: &ExperimentConclusion,
    min_delta: f64,  // default: 0.05 (5% improvement required)
    max_p_value: f64,  // default: 0.05
    min_samples: usize,  // default: 50
) -> Option<ConfigUpdate> {
    if conclusion.delta < min_delta
        || conclusion.p_value > max_p_value
        || conclusion.sample_size < min_samples
    {
        return None; // Not significant enough to promote
    }

    Some(ConfigUpdate {
        key: format!("prompt.{}.variant", conclusion.section_name),
        old_value: "baseline".into(),
        new_value: conclusion.winner_variant.clone(),
        reason: format!(
            "Experiment {} concluded: variant '{}' improved pass rate by {:.1}% (p={:.4}, n={})",
            conclusion.experiment_id,
            conclusion.winner_variant,
            conclusion.delta * 100.0,
            conclusion.p_value,
            conclusion.sample_size,
        ),
        requires_review: true, // Human must approve config changes
    })
}

// Sink: proposed config update
pub struct ConfigUpdate {
    pub key: String,
    pub old_value: String,
    pub new_value: String,
    pub reason: String,
    pub requires_review: bool,
}
```

---

## Cross-Loop Interaction Matrix

The eight loops do not operate independently — they interact. This matrix identifies the key interactions:

| Source Loop | Affected Loop | Interaction |
|-------------|---------------|-------------|
| 1 (Health→Routing) | 6 (Cost→Routing) | Provider failure forces fallback to more expensive provider |
| 2 (Conductor→Routing) | 7 (Latency→Reward) | High system load increases latency, penalizing reward signals |
| 3 (Section→Scaffold) | 5 (Skills→Prompts) | Section weight changes may truncate skill injection section |
| 4 (Failure→Replan) | 6 (Cost→Routing) | Replanning creates new tasks, increasing session cost |
| 6 (Cost→Routing) | 1 (Health→Routing) | Cost-forced downgrade to cheap provider may hit rate limits |
| 7 (Latency→Reward) | 2 (Conductor→Routing) | Latency-optimal routing may increase system load |
| 8 (Experiments→Static) | 3 (Section→Scaffold) | Experiment winner changes section content, resetting effectiveness data |

### Interaction-Aware Scheduling

To prevent cascading oscillation from loop interactions, updates should be scheduled with awareness of dependencies:

```
Priority 1 (every episode): Loop 1 (Health), Loop 6 (Cost)
    → Safety-critical: prevent provider failures and budget overruns

Priority 2 (every 5 episodes): Loop 7 (Latency), Loop 2 (Conductor)
    → Performance: optimize for speed and resource utilization

Priority 3 (every 20 episodes): Loop 3 (Section), Loop 5 (Skills)
    → Learning: adjust prompt composition based on accumulated evidence

Priority 4 (every 50 episodes): Loop 4 (Failure→Replan), Loop 8 (Experiments)
    → Strategic: make structural changes with high confidence requirements
```

This priority ordering ensures that safety-critical loops (health, cost) always run before learning loops (section, skills), preventing a scenario where a learning-driven change causes a safety-critical failure.

---

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The primary target for loops 1, 2, 6, 7, 8.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Source for loop 1.
- **[08-cost-normalization](08-cost-normalization.md)** — Source for loop 6.
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Source for loop 5.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Prevents these loops from oscillating.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Measures the aggregate effect of all loops.
