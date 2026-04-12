# Eight Missing Cybernetic Feedback Loops

> **Implementation plan:** `modelrouting/17-meta-learning-and-corrections.md` (tasks 2O.01–2O.13)
> **PRD source:** `refactoring-prd/07-implementation-priorities.md` (Tier 1M table)
> **Theoretical basis:** Ashby's Law of Requisite Variety, Beer's Viable System Model, Good Regulator Theorem
> **Cross-references:** [04-cascade-router](04-cascade-router.md), [09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md), [14-stability-mechanisms](14-stability-mechanisms.md)

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

## Relationship to Other Documents

- **[04-cascade-router](04-cascade-router.md)** — The primary target for loops 1, 2, 6, 7, 8.
- **[09-provider-health-circuit-breaker](09-provider-health-circuit-breaker.md)** — Source for loop 1.
- **[08-cost-normalization](08-cost-normalization.md)** — Source for loop 6.
- **[02-skill-library-voyager](02-skill-library-voyager.md)** — Source for loop 5.
- **[14-stability-mechanisms](14-stability-mechanisms.md)** — Prevents these loops from oscillating.
- **[15-collective-calibration-31x](15-collective-calibration-31x.md)** — Measures the aggregate effect of all loops.
