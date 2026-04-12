# 05 — Learning

> **Crate:** `roko-learn` · **Path:** `crates/roko-learn/src/`
> **Persistence root:** `.roko/learn/`
> **Entry point:** `LearningRuntime` in `runtime_feedback.rs`

---

## Overview

The learning subsystem turns every agent execution into training data. Each agent turn produces an episode, each episode updates baselines, each baseline informs routing, each routing decision produces a new episode — closing the loop. The compound effect of 11+ interconnected learning subsystems operating simultaneously is that Roko improves autonomously: better prompts, cheaper model routing, fewer repeated mistakes, monotonically growing capabilities.

The subsystem is organized around three tiers of memory (episodes → patterns → playbook rules), three bandit algorithms for online decision-making (UCB1, LinUCB, Track-and-Stop), a three-stage cascade router for model selection (Static → Confidence → UCB), and eight cybernetic feedback loops that connect the subsystems into a self-regulating whole.

---

## Sub-Documents

### Core Data Infrastructure

| # | Document | What it covers |
|---|----------|---------------|
| [00](00-episode-logger.md) | **Episode Logger** | Append-only JSONL episode log, HDC fingerprinting, crash-safe writes, tolerant reader. The foundational data substrate for all learning. |
| [01](01-playbook-system.md) | **Playbook System** | Playbook rules with globset trigger matching, bounded confidence dynamics (validate +0.05, contradict −0.10, ceiling 0.95), TOML persistence. Three-tier memory: episodes → patterns → rules. |
| [02](02-skill-library-voyager.md) | **Skill Library (Voyager)** | Voyager-style skill accumulation (Wang et al. 2023). Monotonically growing library of reusable capabilities with prompt templates, tool dependencies, usage telemetry, and deduplication. |

### Bandit Algorithms

| # | Document | What it covers |
|---|----------|---------------|
| [03](03-bandits-ucb-thompson-linucb.md) | **Bandits: UCB1, Thompson, LinUCB** | UCB1 (Auer et al. 2002), LinUCB 18-dim contextual bandit (Li et al. 2010), Track-and-Stop best-arm identification (Garivier & Kaufmann 2016), BanditBank keyed collections. |
| [04](04-cascade-router.md) | **Cascade Router** | Three-stage model routing: Static (<50 obs) → Confidence (50-200) → UCB (>200). CascadeModel with primary + fallback. Provider health filtering, Pareto pruning, C-Factor bias. |

### Metrics and Monitoring

| # | Document | What it covers |
|---|----------|---------------|
| [05](05-pattern-discovery-trigram.md) | **Pattern Discovery (Trigram)** | Trigram mining across episodes via EpisodeView trait. HDC k-medoids clustering for cross-episode consolidation. Operates every 20 episodes (slowest learning loop). |
| [06](06-task-metrics-and-baselines.md) | **Task Metrics and Baselines** | TaskMetric JSONL writer, per-(role, complexity) SliceBaseline computation, AgentEfficiencyEvent with 20+ fields, A-D prompt grading, four key self-improvement metrics. |
| [07](07-regression-detection.md) | **Regression Detection** | Compare current batch against historical baseline. Thresholds: pass rate drop >15% (Alert), cost increase >20% (Alert), duration +30% (Warning), iterations +25% (Warning). Per-slice analysis. |

### Cost and Provider Management

| # | Document | What it covers |
|---|----------|---------------|
| [08](08-cost-normalization.md) | **Cost Normalization** | CostTable, blended cost formula (3:1 input:output, Artificial Analysis methodology), multi-level budget guardrails (80% downgrade, 95% block, 100% hard stop), CostsLog append-only persistence. |
| [09](09-provider-health-circuit-breaker.md) | **Provider Health / Circuit Breaker** | Three-state circuit breaker (Closed → Open → Half-Open), error classification (RateLimit, AuthFailure, Timeout, ServerError, ContentPolicy, ContextOverflow), error-specific cooldowns, EWMA anomaly detection. |
| [10](10-pareto-frontier-pruning.md) | **Pareto Frontier Pruning** | Non-dominated set computation over (pass_rate, cost_per_success). Pruned models excluded from bandit candidate set. Recomputed every 50 observations. |

### Advanced Algorithms

| # | Document | What it covers |
|---|----------|---------------|
| [11](11-thompson-sampling-drift.md) | **Thompson Sampling with Drift** | Bayesian bandit with discount factor γ for non-stationary environments. Beta distribution per arm, discounted updates, drift detection and arm reset. |
| [12](12-self-improvement-frameworks.md) | **Self-Improvement Frameworks** | Survey: Reflexion (Shinn et al. 2023), ExpeL (Zhao et al. 2023), DSPy (Khattab et al. 2023), RouteLLM (ICLR 2025), FrugalGPT (arXiv:2305.05176), AutoMix (NeurIPS 2024), Karpathy autoresearch. External verifier requirement. |

### Cybernetic Architecture

| # | Document | What it covers |
|---|----------|---------------|
| [13](13-8-missing-feedback-loops.md) | **Eight Missing Feedback Loops** | Health→Routing, Conductor→Routing, Section→Scaffold, Failure→Replanning, Skills→Prompts, Cost→Routing, Latency→Reward, Experiments→Static. Status of each loop. |
| [14](14-stability-mechanisms.md) | **Stability Mechanisms** | Hysteresis (10% score delta to switch), frequency separation (every 1/5/20/50 episodes), EMA damping, anti-patterns (lock-in, explosion, death spiral, collapse). |
| [15](15-collective-calibration-31x.md) | **Collective Calibration (31.6×)** | CLT-inspired heuristic `accuracy(t) = 1 − 1/√(N×t)`. Explicit caveats (independence, stationarity, aggregation). C-Factor composite metric with 11 components and leave-one-out agent contributions. |
| [16](16-predictive-foraging.md) | **Predictive Foraging** | Falsifiable predictions (duration, complexity, gate outcome, conflict). CalibrationTracker, arithmetic corrector (~50ns). Brier score calibration metric, reliability diagrams. |
| [17](17-adas-and-autocatalytic.md) | **ADAS and Autocatalytic Thesis** | ADAS meta-architecture search (Hu et al. ICLR 2025, +14% ARC). EvoSkills (Chen et al. 2023). Autocatalytic sets (Kauffman 1993). Compound math: 0.9⁴ = 0.656. Ten flywheel mechanisms. Empirical testability via C-Factor trend. |

---

## LearningRuntime: The Integration Hub

All learning subsystems are coordinated through `LearningRuntime` in `runtime_feedback.rs`. A single method — `record_completed_run(CompletedRunInput)` — updates every subsystem in a consistent order:

```
CompletedRunInput
    │
    ├── 1. EpisodeLogger::append()           → episodes.jsonl
    ├── 2. CostsLog::append()                → costs.jsonl
    ├── 3. PlaybookStore::record_outcome()   → playbooks/*.json
    ├── 4. PlaybookRules::validate/contradict → playbook-rules.toml
    ├── 5. SkillLibrary::record_use()        → skills.json
    ├── 6. TaskMetric → regression history   → task-metrics.jsonl
    ├── 7. ExperimentStore::record_outcome() → experiments.json
    ├── 8. PatternMiner::ingest_episode()    → (in-memory)
    ├── 9. CascadeRouter::update()           → cascade-router.json
    └── 10. CFactor::compute()               → c-factor.jsonl
```

### Persistence Layout

```
.roko/learn/
├── episodes.jsonl         ← append-only episode log
├── costs.jsonl            ← append-only cost records
├── task-metrics.jsonl     ← append-only task metrics
├── efficiency.jsonl       ← append-only efficiency events
├── c-factor.jsonl         ← append-only C-Factor snapshots
├── skills.json            ← skill library (atomic write)
├── cascade-router.json    ← cascade router state (atomic write)
├── experiments.json       ← experiment store (atomic write)
├── gate-thresholds.json   ← adaptive gate thresholds (atomic write)
├── playbook-rules.toml    ← playbook rules (atomic write)
└── playbooks/             ← per-playbook JSON files
    ├── pb-001.json
    ├── pb-002.json
    └── ...
```

---

## Cross-References to Other Topics

| Topic | Relationship |
|-------|-------------|
| [00-architecture](../00-architecture/INDEX.md) | Engram/Signal data model that episodes extend |
| [02-agents](../02-agents/INDEX.md) | Agent dispatch produces the episodes that learning consumes |
| [03-composition](../03-composition/INDEX.md) | Prompt assembly uses skills and playbook rules from learning |
| [04-verification](../04-verification/INDEX.md) | Gate pipeline produces GateVerdict records consumed by learning |
| [07-conductor](../07-conductor/INDEX.md) | Conductor load signals feed into feedback loop 2 |
| [16-heartbeat](../16-heartbeat/INDEX.md) | Dashboard surfaces C-Factor, predictions, regression alerts |

---

## Key Academic Citations

| Citation | Used In | Contribution |
|----------|---------|-------------|
| Auer, Cesa-Bianchi & Fischer 2002 | [03](03-bandits-ucb-thompson-linucb.md) | UCB1 algorithm |
| Li et al. 2010 | [03](03-bandits-ucb-thompson-linucb.md), [04](04-cascade-router.md) | LinUCB contextual bandit |
| Garivier & Kaufmann 2016 | [03](03-bandits-ucb-thompson-linucb.md) | Track-and-Stop best-arm identification |
| Thompson 1933 | [11](11-thompson-sampling-drift.md) | Thompson Sampling |
| Wang et al. 2023 | [02](02-skill-library-voyager.md) | Voyager skill library |
| Zhao et al. 2023 | [12](12-self-improvement-frameworks.md) | ExpeL experience extraction |
| Shinn et al. 2023 | [12](12-self-improvement-frameworks.md) | Reflexion |
| Khattab et al. 2023 | [12](12-self-improvement-frameworks.md) | DSPy prompt optimization |
| Hu et al. ICLR 2025 | [17](17-adas-and-autocatalytic.md) | ADAS meta-architecture search |
| Chen et al. 2023 | [17](17-adas-and-autocatalytic.md) | EvoSkills |
| Kauffman 1993 | [17](17-adas-and-autocatalytic.md) | Autocatalytic sets |
| Ong et al. ICLR 2025 | [12](12-self-improvement-frameworks.md) | RouteLLM |
| Chen et al. arXiv:2305.05176 | [12](12-self-improvement-frameworks.md) | FrugalGPT |
| Loreto & Tria 2014 | [17](17-adas-and-autocatalytic.md) | Pólya urn model for innovation |
| Huang et al. ICLR 2024 | [12](12-self-improvement-frameworks.md) | External verifier requirement |
| Song et al. ICLR 2025 | [12](12-self-improvement-frameworks.md) | Self-improvement verification |
| Pan et al. ICML 2024 | [12](12-self-improvement-frameworks.md) | Self-improvement limitations |
| Garivier & Moulines 2011 | [11](11-thompson-sampling-drift.md) | Discounted Thompson Sampling |
| Gneiting & Raftery 2007 | [16](16-predictive-foraging.md) | Calibration theory |

---

## Architecture Diagram

```
                    ┌─────────────────────────────┐
                    │       Agent Turn             │
                    │   (orchestrate.rs)           │
                    └──────────┬──────────────────┘
                               │
                               ▼
                    ┌─────────────────────────────┐
                    │    LearningRuntime           │
                    │  record_completed_run()      │
                    └──────────┬──────────────────┘
                               │
           ┌───────────────────┼───────────────────────┐
           │                   │                       │
           ▼                   ▼                       ▼
    ┌──────────────┐   ┌──────────────┐      ┌──────────────┐
    │ EpisodeLogger│   │  CostsLog    │      │ TaskMetrics   │
    │  (JSONL)     │   │  (JSONL)     │      │   (JSONL)     │
    └──────┬───────┘   └──────┬───────┘      └──────┬───────┘
           │                  │                      │
           ▼                  ▼                      ▼
    ┌──────────────┐   ┌──────────────┐      ┌──────────────┐
    │PatternMiner  │   │ CascadeRouter│      │  Regression   │
    │(trigrams)    │   │ (3-stage)    │      │  Detection    │
    └──────┬───────┘   └──────┬───────┘      └──────┬───────┘
           │                  │                      │
           ▼                  ▼                      ▼
    ┌──────────────┐   ┌──────────────┐      ┌──────────────┐
    │PlaybookRules │   │ProviderHealth│      │  C-Factor     │
    │  (TOML)      │   │(CircuitBrkr) │      │  (composite)  │
    └──────────────┘   └──────────────┘      └──────────────┘
           │                  │                      │
           └───────────┬──────┘──────────────────────┘
                       │
                       ▼
              ┌──────────────────┐
              │ SystemPromptBuilder│
              │ (prompt injection) │
              └──────────────────┘
```

---

## Data Flow Summary

| Source | Artifact | Consumers |
|--------|----------|-----------|
| Agent turn | Episode | PatternMiner, CascadeRouter, CFactor, SkillLibrary |
| Gate execution | GateVerdict | Episode (embedded), Regression detector |
| Provider response | CostRecord | CostsLog, CostsDb, BudgetGuardrail |
| Agent turn | TaskMetric | MetricsWriter, Baseline, Regression |
| Agent turn | AgentEfficiencyEvent | Efficiency grading, section effectiveness |
| PatternMiner | Pattern | PlaybookRules (promotion candidate) |
| PlaybookRules | Rule | SystemPromptBuilder (injection) |
| SkillLibrary | Skill | SystemPromptBuilder (injection) |
| CascadeRouter | CascadeModel | Orchestrator (model selection) |
| CFactor | CFactorSnapshot | Dashboard, routing bias |
| ProviderHealth | CircuitState | CascadeRouter (filtering) |
| LatencyRegistry | LatencyStats | CascadeRouter (SLA compliance) |
| ExperimentStore | PromptVariant | SystemPromptBuilder (variant selection) |

---

## Quick Start

To enable learning in a Roko project:

```bash
# Initialize .roko directory (creates .roko/learn/ subdirectory)
cargo run -p roko-cli -- init

# Execute plans — learning subsystems update automatically
cargo run -p roko-cli -- plan run plans/

# View learning status
cargo run -p roko-cli -- dashboard
```

Learning is automatic: every agent turn updates all subsystems through the `LearningRuntime`. No manual configuration is needed beyond `roko init`.

### Inspecting Learning State

```bash
# View episode count and recent episodes
ls -la .roko/learn/episodes.jsonl

# View cost summary
wc -l .roko/learn/costs.jsonl

# View skill library
cat .roko/learn/skills.json | python3 -m json.tool | head -50

# View cascade router state (current stage, observations)
cat .roko/learn/cascade-router.json | python3 -m json.tool | head -20

# View playbook rules
cat .roko/learn/playbook-rules.toml
```
