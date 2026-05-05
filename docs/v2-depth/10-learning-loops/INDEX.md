# 10-learning-loops -- Depth Index

Depth for [07-LEARNING.md](../../unified/07-LEARNING.md)

---

## Source docs (23)

### Episodes and playbooks

| Source doc | Status |
|---|---|
| `docs/05-learning/00-episode-logger.md` | Done |
| `docs/05-learning/01-playbook-system.md` | Done |
| `docs/05-learning/02-skill-library-voyager.md` | Done |

### Bandit algorithms and routing

| Source doc | Status |
|---|---|
| `docs/05-learning/03-bandits-ucb-thompson-linucb.md` | Done |
| `docs/05-learning/04-cascade-router.md` | Done |
| `docs/05-learning/11-thompson-sampling-drift.md` | Done |

### Metrics, baselines, and detection

| Source doc | Status |
|---|---|
| `docs/05-learning/05-pattern-discovery-trigram.md` | Done |
| `docs/05-learning/06-task-metrics-and-baselines.md` | Done |
| `docs/05-learning/07-regression-detection.md` | Done |
| `docs/05-learning/08-cost-normalization.md` | Done |
| `docs/05-learning/10-pareto-frontier-pruning.md` | Done |

### Provider health

| Source doc | Status |
|---|---|
| `docs/05-learning/09-provider-health-circuit-breaker.md` | Done |

### Self-improvement and cybernetics

| Source doc | Status |
|---|---|
| `docs/05-learning/12-self-improvement-frameworks.md` | Done |
| `docs/05-learning/13-8-missing-feedback-loops.md` | Done |
| `docs/05-learning/14-stability-mechanisms.md` | Done |
| `docs/05-learning/15-collective-calibration-31x.md` | Done |
| `docs/05-learning/16-predictive-foraging.md` | Done |
| `docs/05-learning/17-adas-and-autocatalytic.md` | Done |
| `docs/05-learning/18-self-learning-cybernetic-loops.md` | Done |
| `docs/05-learning/19-heuristics-worldviews-and-falsifiers.md` | Done |
| `docs/05-learning/20-research-to-runtime.md` | Done |

### Collective intelligence

| Source doc | Status |
|---|---|
| `docs/00-architecture/14-c-factor-collective-intelligence.md` | Done |
| `docs/00-architecture/16-autocatalytic-and-cybernetics.md` | Done |

---

## Depth docs

| Depth doc | Source | What it adds |
|---|---|---|
| [c-factor-as-lens.md](c-factor-as-lens.md) | `14-c-factor-collective-intelligence.md` | C-factor as a Lens Cell with five sub-lenses, WisdomGate Verify Cell, anti-groupthink React Cells, Goodhart defense |
| [autocatalytic-compounding.md](autocatalytic-compounding.md) | `16-autocatalytic-and-cybernetics.md` | Seven compounding Loops, feedback graph, Kauffman autocatalytic condition, anti-metric Verify Cells, single points of failure |
| [episodes-and-playbooks.md](episodes-and-playbooks.md) | `00-episode-logger.md`, `01-playbook-system.md`, `02-skill-library-voyager.md` | Episode Store Cell (tiered JSONL, HDC fingerprinting, importance scoring), playbook rules as Heuristic Signals (demurrage-driven retention, 0.95 confidence ceiling, predict-publish-correct calibration), Voyager-style skill library as Memory (monotonic growth, template injection), four-tier learning hierarchy, mori-diffs reality gaps |
| [bandit-routing-and-cascade.md](bandit-routing-and-cascade.md) | `03-bandits-ucb-thompson-linucb.md`, `04-cascade-router.md`, `05-pattern-discovery-trigram.md` | Three-stage cascade (static -> confidence -> LinUCB), three bandit algorithms (UCB1, LinUCB, Track-and-Stop), 18-dimensional context vector, Pareto frontier pre-filtering, pattern discovery via trigram mining and HDC clustering, router calibration (Platt scaling, ECE), lookahead routing for KV cache reuse, C-Factor routing bias |
| [metrics-baselines-and-regression.md](metrics-baselines-and-regression.md) | `06-task-metrics-and-baselines.md`, `07-regression-detection.md`, `08-cost-normalization.md` | TaskMetric Score Cells, per-slice baseline Lens Cells, regression detection Verify Cell (15% pass rate / 20% cost thresholds), cost normalization (3:1 blended formula), budget guardrails (per-task/session/day), efficiency grading (A-D), Lens-to-Loop pipeline for adaptive gate thresholds |
| [provider-health-and-pareto.md](provider-health-and-pareto.md) | `09-provider-health-circuit-breaker.md`, `10-pareto-frontier-pruning.md` | Three-state circuit breaker React Cell (closed/open/half-open), error classification with tailored cooldowns, exponential backoff, Pareto dominance pruning of model candidates, anomaly detection (prompt loops, cost spikes, quality degradation), Health-Routing-Health cybernetic loop |
| [drift-and-stability.md](drift-and-stability.md) | `11-thompson-sampling-drift.md`, `14-stability-mechanisms.md` | Thompson Sampling with discount as a Loop (gamma=0.995, adaptive discount, abrupt drift reset), stability mechanisms (hysteresis 10% threshold, frequency separation 4-tier hierarchy, EMA damping), compound stability condition, anti-pattern traps (model lock-in, cost death spiral, threshold collapse), Ashby/Beer/Good Regulator foundations |
| [self-improvement-frameworks.md](self-improvement-frameworks.md) | `12-self-improvement-frameworks.md`, `17-adas-and-autocatalytic.md`, `18-self-learning-cybernetic-loops.md`, `20-research-to-runtime.md` | Five-level improvement stack (parameters -> strategies -> representations -> architecture -> meta), academic framework mapping (Reflexion, ExpeL, DSPy, ADAS), Variance Inequality bound, constitutional constraints, gate gaming detection, predict-publish-correct doctrine for all operators, research-to-runtime pipeline (Paper -> Claim -> Heuristic -> Trial -> Calibration), improvement measurement (four key metrics, holdout experiments, monotonicity), compound improvement math, velocity limits |
| [missing-loops-and-calibration.md](missing-loops-and-calibration.md) | `13-8-missing-feedback-loops.md`, `15-collective-calibration-31x.md`, `16-predictive-foraging.md` | Eight cybernetic feedback Loops (Health->Routing, Conductor->Routing, Section->Scaffold, Failure->Replan, Skills->Prompts, Cost->Routing, Latency->Reward, Experiments->Static), cross-Loop interaction matrix, interaction-aware scheduling (4 priority tiers), collective calibration 31.6x heuristic (CLT upper bound, 3-10x practical), C-Factor components and leave-one-out contributions, predictive foraging (4 prediction types, Brier score, arithmetic corrector, per-category correction), higher-order learning hierarchy |
| [heuristics-and-falsifiers.md](heuristics-and-falsifiers.md) | `19-heuristics-worldviews-and-falsifiers.md`, `16-predictive-foraging.md` | Heuristic Signal kind (claim + preconditions + prediction + calibration + receipts), Predicate surface for matching and falsification, heuristic lifecycle (birth/test/adjust/retire), Wilson confidence interval for prompt weighting, worldviews as co-citation clusters, dissonance detection for active learning, MVT foraging cutoff, AntiKnowledge from refuted heuristics, inspection CLI surface |
