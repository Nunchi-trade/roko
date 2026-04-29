# Yerkes-Dodson Pressure Dynamics

> Moderate pressure maximizes cooperation. Extreme pressure collapses
> cooperative behavior within 5-12 turns. The Conductor's thresholds
> are not timeouts — they are positions on a cooperation curve.


> **Implementation**: Built

---

## The Inverted-U Curve

Robert Yerkes and John Dodson (1908) established that the
relationship between arousal and performance follows an inverted-U
curve: performance increases with arousal up to an optimum, then
declines as arousal becomes excessive. The original experiments
measured maze-learning speed in mice under varying electric shock
intensities. The finding has replicated across species, task types,
and complexity levels for over a century.

The curve has three regimes:

```
Performance
    ^
    |         ****
    |       **    **
    |      *        *
    |     *          *
    |    *            *
    |   *              *
    |  *                *
    | *                  *
    |*                    *
    +-------------------------> Pressure
    Low    Optimal    High

    Zone 1: Under-arousal → drift, exploration, token waste
    Zone 2: Optimal arousal → focused execution, cooperation
    Zone 3: Over-arousal → collapse, minimal-effort responses
```

The shape is consistent but the peak location shifts with task
complexity. Simple tasks peak at higher arousal (more pressure helps
straightforward work). Complex tasks peak at lower arousal (too much
pressure degrades complex reasoning). This complexity interaction is
called the Yerkes-Dodson Law proper.

---

## Yerkes-Dodson in LLM Agent Systems

### Research Evidence

Research on 770,000+ autonomous LLM agents demonstrates that
cooperative behavior follows the same inverted-U pattern with
environmental pressure:

- **Moderate pressure** (iteration limits, timeouts, cost budgets)
  maximizes inter-agent cooperation — agents build on each other's
  work, follow established patterns, and produce complementary
  outputs.

- **Extreme pressure** collapses cooperative behavior within **5-12
  turns**. Under severe iteration limits or aggressive restarts,
  agents shift to minimal-effort strategies: producing the simplest
  possible output that satisfies immediate constraints rather than
  contributing to the broader task. This is the agent equivalent of
  panic — survival over quality.

- **Insufficient pressure** produces drift — agents explore
  tangential approaches, repeat themselves, or engage in verbose
  reasoning that burns tokens without advancing the task. Ghost turns
  (Issue #9 in the production failure catalog) are an extreme form of
  under-pressure drift.

### Active Minority Dynamics

In large agent populations, meaningful role differentiation is limited
to active minorities. Most agents converge to generic behavior
regardless of pressure level. This validates the approach of spawning
specialized agents on demand rather than maintaining a large standing
pool. The Conductor does not need to manage pressure across dozens of
simultaneous agents — it manages pressure on the small number of
agents actually doing differentiated work at any moment.

---

## Conductor Thresholds as Pressure Parameters

Every Conductor threshold defines a position on the Yerkes-Dodson
curve. The ensemble of thresholds creates a **pressure envelope**
around the agent:

### Threshold-to-Pressure Mapping

| Threshold | Pressure Type | Too Low | Optimal | Too High |
|-----------|--------------|---------|---------|----------|
| `max_iterations` (3) | Iteration pressure | Agent loops indefinitely | Agent converges in 2-3 attempts | Agent gives up after first failure |
| `cost_limit_usd` ($10) | Budget pressure | Agent uses expensive reasoning freely | Agent balances cost vs. quality | Agent produces minimal output |
| `time_limit` (80%) | Time pressure | Agent takes unlimited time per phase | Agent completes within phase window | Agent rushes, skips verification |
| `stuck_threshold` (4) | Progress pressure | Agent allowed to spin | Agent must show progress each turn | Agent forced into superficial progress |
| `ghost_turn_max` (3) | Output pressure | Agent can produce empty turns | Agent must produce meaningful output | Agent produces any output to avoid detection |

### The Pressure Envelope

The thresholds work together, not independently. An agent experiences
their combined effect as a **pressure envelope**:

```
                            Pressure Envelope
                     ┌─────────────────────────────┐
                     │                             │
    Iteration ─────► │   ┌─────────────────────┐   │
    Pressure         │   │                     │   │
                     │   │    Agent Operating   │   │
    Cost ──────────► │   │       Space          │   │
    Pressure         │   │                     │   │
                     │   │   (Work happens      │   │
    Time ──────────► │   │    in here)          │   │
    Pressure         │   │                     │   │
                     │   └─────────────────────┘   │
    Progress ──────► │                             │
    Pressure         │                             │
                     └─────────────────────────────┘
    Output ────────►
    Pressure

    Tight envelope → High total pressure → Zone 3 risk
    Loose envelope → Low total pressure → Zone 1 risk
```

The operating space shrinks as more thresholds tighten. The
Conductor's current thresholds define a moderate envelope:

- Iteration limit of 3 allows meaningful retry without indefinite
  looping
- Cost limit of $10 provides generous budget for complex tasks while
  capping runaway spending
- Time limit at 80% of phase budget leaves 20% margin for cleanup
- Stuck threshold of 4 identical outputs requires genuine repetition,
  not just similar outputs
- Ghost turn threshold of 3 allows for legitimate warm-up before
  firing

This calibration sits near the peak of the inverted-U for standard
tasks. Complex tasks may need a looser envelope (more iterations,
higher cost budget, longer time). Fast tasks could tolerate a tighter
one.

---

## Complexity-Pressure Interaction

The Yerkes-Dodson Law's most important implication for agent systems
is the complexity interaction: the optimal pressure level depends on
task difficulty.

### Complexity Bands

The state machine (`state_machine.rs`) defines three complexity bands
with different phase timeouts:

| Complexity | Phase Timeout | Iteration Budget | Pressure Level |
|-----------|--------------|-----------------|---------------|
| Complex | 600s | Higher tolerance | Lower pressure (Zone 2 left) |
| Standard | 300s | Default | Moderate pressure (Zone 2 center) |
| Fast | 120s | Lower tolerance | Higher pressure (Zone 2 right) |

These bands implicitly implement the Yerkes-Dodson complexity
interaction:

- **Complex tasks** get more time and iteration room because the peak
  of their inverted-U curve is at lower pressure. A complex
  refactoring that touches 15 files needs room to iterate, backtrack,
  and converge.

- **Fast tasks** get less time because their peak is at higher
  pressure. A simple import fix does not benefit from exploration
  time. Tight constraints focus the agent on the obvious solution.

- **Standard tasks** sit in between. The defaults are calibrated for
  this middle ground.

### The Collapse Window

When pressure exceeds the optimal zone, cooperative behavior collapses
within 5-12 turns. In concrete terms:

```
Turn 1:  Agent attempts task normally
Turn 2:  First failure, agent retries with adjustment
Turn 3:  Conductor restarts (compile-fail-repeat fires)
Turn 4:  Agent attempts again, now with restart pressure
Turn 5:  Second failure, agent begins simplifying approach
Turn 6:  Conductor restarts again
Turn 7:  Agent produces minimal-effort output
Turn 8:  Output barely passes or fails again
...
Turn 12: Agent in full collapse — producing template responses
         that technically satisfy format requirements but contain
         no meaningful implementation
```

The circuit breaker's MAX_PLAN_FAILURES=2 limit catches this
collapse pattern. After 2 plan-level failures, the breaker opens and
prevents further attempts. This is not a simple retry limit — it is
a pressure release valve that prevents the system from pushing agents
past the collapse point.

---

## Cooperation Metrics

### Defining Cooperation

In multi-agent software development, cooperation manifests as agents
building on each other's work rather than clobbering it:

- **Positive cooperation**: Agent B extends Agent A's implementation,
  respects established patterns, follows the code conventions Agent A
  introduced.

- **Negative cooperation**: Agent B rewrites Agent A's work, ignores
  established patterns, introduces conflicting conventions.

- **Neutral**: Agent B works on entirely independent code with no
  interaction with Agent A's output.

### Measurable Cooperation Signals

The system produces several signals that correlate with cooperation
quality:

| Signal | Source | Cooperative | Collapsed |
|--------|--------|-------------|-----------|
| Merge conflict rate | Git merge queue | Low (<5%) | High (>20%) |
| Gate pass on first attempt | Gate pipeline | High (>60%) | Low (<30%) |
| Conductor intervention rate | Conductor events | Low (<10% of turns) | High (>40% of turns) |
| Review approval on first review | Review loop | High (>50%) | Low (<20%) |
| Cost per successful task | Efficiency events | Low and stable | High and rising |
| Token waste ratio | Efficiency events | Low (<20% wasted) | High (>50% wasted) |

### The Feedback Loop

Cooperation metrics close the learning loop:

```
Batch run N:
    Conductor thresholds = [max_iter=3, cost=$10, stuck=4]
    Cooperation metrics:
        merge_conflict_rate = 0.08
        first_pass_gate_rate = 0.55
        intervention_rate = 0.15
    →  Position: slightly left of optimal (could tighten)

Batch run N+1:
    Conductor thresholds = [max_iter=3, cost=$8, stuck=3]
    Cooperation metrics:
        merge_conflict_rate = 0.12
        first_pass_gate_rate = 0.48
        intervention_rate = 0.22
    →  Position: slightly right of optimal (overtightened)

Batch run N+2:
    Conductor thresholds = [max_iter=3, cost=$9, stuck=4]
    Cooperation metrics:
        merge_conflict_rate = 0.06
        first_pass_gate_rate = 0.62
        intervention_rate = 0.11
    →  Position: near optimal
```

Each batch run produces data that refines the next run's pressure
calibration. The cascade router already tracks model-task outcome
data. Extending it to track pressure-cooperation relationships
enables automated Yerkes-Dodson tuning.

---

## Pressure Tuning in Practice

### Static Pressure (Current Implementation)

The current Conductor uses static thresholds. These thresholds were
calibrated through production experience (the 21-failure catalog) and
represent good defaults:

- `MAX_GHOST_TURNS = 3`: Too low (1-2) catches legitimate warm-up.
  Too high (5+) wastes tokens on genuinely stuck agents.

- `MAX_COMPILE_FAILS = 3`: Derived from production observation that
  agents rarely recover after 3 consecutive compile failures on the
  same error pattern. Allowing more attempts pushes into collapse
  territory.

- `MAX_ITERATIONS = 3`: Plan-level retry limit. Combined with the
  circuit breaker's MAX_PLAN_FAILURES=2, this creates a total of
  2×3=6 attempts before permanent failure — just at the edge of the
  collapse window.

- `COST_LIMIT = $10`: Budget pressure. At current model pricing,
  $10 allows approximately 200-300 agent turns with a mid-tier model,
  sufficient for complex tasks without enabling runaway exploration.

### Adaptive Pressure (Design Target)

The adaptive Conductor model (described in
08-good-regulator-self-model.md) would tune pressure dynamically:

1. **Per-task complexity assessment**: Before each task, estimate
   complexity from plan metadata (files to modify, dependency depth,
   error category). Set the pressure envelope accordingly.

2. **Runtime pressure adjustment**: If an agent is making steady
   progress (gate scores improving, test counts increasing), maintain
   or loosen pressure. If progress stalls, increase pressure by
   tightening thresholds.

3. **Cross-batch learning**: Track cooperation metrics across batch
   runs. Adjust the default pressure envelope based on observed
   cooperation peaks for each complexity band.

4. **Model-specific calibration**: Different models have different
   Yerkes-Dodson curves. A high-capability model tolerates more
   pressure before collapse. A smaller model collapses sooner.
   The cascade router's model-outcome data informs per-model
   pressure profiles.

---

## Stigmergy and Pressure

Pierre-Paul Grassé's stigmergy concept (1959) — indirect
coordination through environment modification — intersects with
Yerkes-Dodson pressure in multi-agent development.

Git is stigmergic: each commit is an environmental trace that
influences future agents. Under optimal pressure, agents leave
high-quality traces:

- Clean commits with meaningful messages
- Consistent code patterns that subsequent agents follow
- Established conventions that reduce decision overhead

Under excessive pressure, stigmergic quality degrades:

- Minimal commits with no context
- Ad hoc patterns that subsequent agents cannot follow
- Conflicting conventions that increase future merge conflicts

The stigmergic quality of one batch run's output becomes the
environmental input for the next run. Poor stigmergic quality
compounds: low-quality traces produce confused agents that produce
lower-quality traces. This is a positive feedback loop that the
Conductor's pressure calibration must prevent.

The key property of stigmergic coordination is O(1) cost per agent
— each agent reads the environment independently. This scales
sublinearly compared to O(n²) message-based coordination. But this
scaling advantage depends on trace quality, which depends on
pressure calibration.

---

## The Conductor's Role

The Conductor does not directly tune pressure — its thresholds ARE
the pressure. Every threshold decision is implicitly a position
choice on the Yerkes-Dodson curve:

1. **Watcher thresholds** define sensitivity — how quickly the system
   detects problems. More sensitive = more interventions = more
   pressure.

2. **Intervention severity** defines response magnitude. Warning
   (restart) is moderate pressure. Critical (fail) is maximum
   pressure.

3. **Circuit breaker limits** define persistence — how many times the
   system retries before giving up. More retries = sustained pressure.
   Fewer retries = quick pressure release.

4. **Phase timeouts** define temporal pressure — how long the agent
   has to work. Shorter timeouts = higher time pressure.

The Conductor's design philosophy — **decide, don't nudge** —
reflects Yerkes-Dodson wisdom. A nudge (suggestion to the agent) adds
ambiguous pressure — the agent must interpret the suggestion and
decide how to respond, which itself consumes cognitive resources. A
decision (restart, fail) is unambiguous — the agent gets a clean
slate or the task is done. Clear decisions produce predictable
pressure. Ambiguous nudges produce unpredictable pressure that may
push the agent past the collapse point.

---

## Cross-References

- [01-watcher-ensemble.md](01-watcher-ensemble.md) — Watcher
  thresholds as pressure parameters
- [02-circuit-breaker.md](02-circuit-breaker.md) — Circuit breaker
  as pressure release valve
- [03-graduated-interventions.md](03-graduated-interventions.md) —
  Severity system as pressure magnitude
- [05-stuck-detection.md](05-stuck-detection.md) — Stuck detection
  as progress pressure
- [08-good-regulator-self-model.md](08-good-regulator-self-model.md)
  — Adaptive self-model for pressure tuning
- [10-adaptive-timeouts-state-machine.md](10-adaptive-timeouts-state-machine.md)
  — Complexity bands as Yerkes-Dodson implementation
- [11-anomaly-detection-learning.md](11-anomaly-detection-learning.md)
  — Learning loops that enable pressure optimization
- [14-production-failure-catalog.md](14-production-failure-catalog.md)
  — Production data that calibrated current thresholds

### Citations

- Yerkes, R.M. & Dodson, J.D. (1908). "The relation of strength of
  stimulus to rapidity of habit-formation." *Journal of Comparative
  Neurology and Psychology*, 18, 459-482.
- Grassé, P.P. (1959). "La reconstruction du nid et les coordinations
  interindividuelles chez Bellicositermes natalensis et Cubitermes sp."
  *Insectes Sociaux*, 6(1), 41-80.
- Research on 770,000+ autonomous agents: emergent cooperation
  dynamics, Yerkes-Dodson replication in LLM multi-agent systems.

---

## Pressure Calibration Per Agent Type

Different models have different optimal pressure. The inverted-U
curve shifts left or right depending on model capability, and the
shape of the curve — peak width, collapse steepness — varies too.
Treating all models identically wastes capacity on strong models
and breaks weak ones.

### Model-specific Yerkes-Dodson curves

| Model Tier | Peak Location | Collapse Threshold | Rationale |
|-----------|--------------|-------------------|-----------|
| Opus (Premium) | Higher pressure | Later collapse | Superior reasoning tolerates more constraint |
| Sonnet (Standard) | Moderate pressure | Moderate collapse | Good general-purpose balance |
| Haiku (Fast) | Lower pressure | Earlier collapse | Limited reasoning degrades faster under stress |

The curve shape also differs:

- **Opus**: wider peak (robust over a range of pressures), gradual
  collapse. You can push an Opus agent harder before performance
  degrades, and the degradation is smooth rather than cliff-like.

- **Sonnet**: moderate peak width, moderate collapse steepness. The
  default thresholds in the Conductor are calibrated for this tier.

- **Haiku**: narrow peak (small optimal window), steep collapse. A
  Haiku agent operating even slightly past its optimal pressure
  drops to minimal-effort output fast. The margin for error is
  thin.

### Empirical calibration protocol

To determine optimal pressure per model, the system maintains a
per-model pressure profile learned from execution history:

```rust
/// Per-model pressure profile learned from execution history.
/// Each model has its own Yerkes-Dodson curve parameters.
pub struct ModelPressureProfile {
    /// Model identifier (e.g., "claude-opus-4-6").
    pub model: String,
    /// Estimated optimal pressure level (0.0 to 1.0).
    pub optimal_pressure: f64,
    /// Estimated collapse threshold (pressure above which performance drops sharply).
    pub collapse_threshold: f64,
    /// Confidence in the estimate (number of observations).
    pub observations: usize,
    /// Historical (pressure, performance) pairs for curve fitting.
    pub history: Vec<(f64, f64)>,
}

/// Compute a scalar pressure index from the multi-dimensional pressure envelope.
pub fn pressure_index(
    iteration: u32,
    max_iterations: u32,
    cost_usd: f64,
    cost_budget_usd: f64,
    elapsed_ms: u64,
    timeout_ms: u64,
    stuck_count: u32,
    stuck_threshold: u32,
) -> f64 {
    let iter_pressure = iteration as f64 / max_iterations as f64;
    let cost_pressure = cost_usd / cost_budget_usd;
    let time_pressure = elapsed_ms as f64 / timeout_ms as f64;
    let stuck_pressure = stuck_count as f64 / stuck_threshold as f64;

    // Weighted combination (weights sum to 1.0)
    0.30 * iter_pressure
        + 0.25 * cost_pressure
        + 0.25 * time_pressure
        + 0.20 * stuck_pressure
}
```

The `pressure_index` function collapses the multi-dimensional
pressure envelope into a single scalar. This scalar maps to the
x-axis of the Yerkes-Dodson curve. Comparing it to the model's
`optimal_pressure` and `collapse_threshold` tells the Conductor
whether the agent is in zone 1 (under-pressure), zone 2 (optimal),
or zone 3 (over-pressure).

### Thompson sampling for pressure optimization

Rather than hand-tuning pressure configurations, use bandit
algorithms to find the optimal level per model:

- **Arms**: five discrete pressure configurations — very-loose,
  loose, moderate, tight, very-tight
- **Reward**: gate pass rate weighted by efficiency
  (`pass_rate / cost_usd`)
- **Prior**: discounted Beta distribution to handle
  non-stationarity as models update and codebases evolve

Each arm maps to a concrete `(max_iter, cost_limit, timeout)`
setting:

```rust
/// Thompson Sampling arms for pressure level selection.
/// Each arm represents a discrete pressure configuration.
pub struct PressureBandit {
    /// Per-pressure-level Thompson arm (Beta posterior).
    arms: Vec<PressureArm>,
    /// Discount factor for non-stationarity (default: 0.995).
    discount: f64,
}

pub struct PressureArm {
    pub name: &'static str,
    pub config: PressureConfig,
    pub alpha: f64,  // Beta posterior: successes
    pub beta: f64,   // Beta posterior: failures
}

pub struct PressureConfig {
    pub max_iterations: u32,
    pub cost_budget_usd: f64,
    pub phase_timeout_secs: u64,
    pub stuck_threshold: u32,
    pub ghost_turn_max: u32,
}

/// Default pressure configurations for each arm.
pub const PRESSURE_CONFIGS: &[(&str, PressureConfig)] = &[
    ("very-loose", PressureConfig { max_iterations: 5, cost_budget_usd: 25.0, phase_timeout_secs: 900, stuck_threshold: 6, ghost_turn_max: 5 }),
    ("loose",      PressureConfig { max_iterations: 4, cost_budget_usd: 15.0, phase_timeout_secs: 600, stuck_threshold: 5, ghost_turn_max: 4 }),
    ("moderate",   PressureConfig { max_iterations: 3, cost_budget_usd: 10.0, phase_timeout_secs: 300, stuck_threshold: 4, ghost_turn_max: 3 }),
    ("tight",      PressureConfig { max_iterations: 2, cost_budget_usd: 5.0,  phase_timeout_secs: 180, stuck_threshold: 3, ghost_turn_max: 2 }),
    ("very-tight", PressureConfig { max_iterations: 1, cost_budget_usd: 2.0,  phase_timeout_secs: 120, stuck_threshold: 2, ghost_turn_max: 1 }),
];
```

The discount factor (0.995) means observations from ~200 tasks ago
carry half their original weight. This prevents stale data from
anchoring the bandit on an outdated optimum when the model or
codebase changes.

---

## Pressure-Performance Curve Fitting from Historical Data

The inverted-U is a qualitative shape. To use it for adaptive
pressure control, the system needs a quantitative model fit from
observed (pressure, performance) pairs.

### Curve parameterization

The Yerkes-Dodson curve is modeled as an asymmetric logistic
product:

```
P(x) = P_max * sigmoid(k1 * (x - a_low)) * (1 - sigmoid(k2 * (x - a_high)))
```

Parameters:

- **P_max**: peak performance (observed maximum gate pass rate)
- **a_low**: left threshold (pressure below which performance is
  limited by under-arousal)
- **a_high**: right threshold (pressure above which performance
  collapses)
- **k1**: steepness of the left slope (how fast performance rises
  with pressure)
- **k2**: steepness of the right slope (how fast performance
  collapses)

The curve is typically asymmetric: k2 > k1. Performance collapses
faster than it rises. An agent that took 10 turns of gentle
pressure to reach peak performance can lose that performance in
3 turns of excessive pressure. This asymmetry is why the
Conductor's circuit breaker errs on the side of stopping early
rather than pushing harder.

### Online curve estimation

Full parametric curve fitting requires nonlinear optimization,
which is expensive to run per-agent per-task. Instead, the system
uses binned estimation — a lightweight online method that tracks
the curve shape as data arrives:

```rust
/// Online estimator for Yerkes-Dodson curve parameters.
/// Maintains running estimates of curve shape from streaming (pressure, performance) data.
pub struct YerkesDodsonEstimator {
    /// Binned observations: pressure_bin -> (sum_performance, count).
    bins: Vec<PressureBin>,
    /// Number of bins (default: 10, covering 0.0 to 1.0 pressure range).
    num_bins: usize,
    /// Estimated optimal pressure (argmax of binned performance).
    estimated_optimum: f64,
    /// Estimated peak performance (max of binned performance).
    estimated_peak: f64,
    /// Confidence: total observations across all bins.
    total_observations: usize,
    /// Minimum observations per bin before including in estimate.
    min_bin_count: usize,  // default: 5
}

pub struct PressureBin {
    pub center: f64,       // center of the bin (e.g., 0.05, 0.15, ...)
    pub sum_perf: f64,     // sum of performance observations
    pub count: usize,      // number of observations
}

impl YerkesDodsonEstimator {
    pub fn record(&mut self, pressure: f64, performance: f64) {
        let bin_idx = ((pressure * self.num_bins as f64) as usize).min(self.num_bins - 1);
        self.bins[bin_idx].sum_perf += performance;
        self.bins[bin_idx].count += 1;
        self.total_observations += 1;
        self.reestimate();
    }

    fn reestimate(&mut self) {
        let mut best_perf = 0.0;
        let mut best_pressure = 0.5;
        for bin in &self.bins {
            if bin.count >= self.min_bin_count {
                let avg = bin.sum_perf / bin.count as f64;
                if avg > best_perf {
                    best_perf = avg;
                    best_pressure = bin.center;
                }
            }
        }
        self.estimated_optimum = best_pressure;
        self.estimated_peak = best_perf;
    }

    /// Recommend a pressure level for the next task.
    pub fn recommended_pressure(&self) -> f64 {
        if self.total_observations < 20 {
            0.5  // Default to moderate pressure with insufficient data
        } else {
            self.estimated_optimum
        }
    }
}
```

The estimator defaults to moderate pressure (0.5) until it
accumulates at least 20 observations. Below that threshold, the
binned averages are too noisy to trust. The `min_bin_count` of 5
per bin prevents a single outlier from dominating a bin's estimate.

### Regime shift detection

The Yerkes-Dodson curve is not static. Model updates, codebase
evolution, and task distribution shifts can all change the curve's
shape:

- A model update that improves reasoning shifts the peak rightward
  (the model tolerates more pressure)
- A codebase that grows more complex shifts the peak leftward
  (complex tasks need less pressure)
- A change in task mix (more refactoring, less greenfield) changes
  the curve's width

The system detects these shifts using CUSUM (cumulative sum
control chart) on the residuals between actual performance and
predicted performance at the current pressure level. When the
CUSUM statistic exceeds a threshold, the estimator resets its bin
counts and begins re-estimation. The signal:
"The Yerkes-Dodson curve for this model has shifted —
recalibrating."

### Bayesian confidence bounds

The binned estimator produces point estimates. To know whether
those estimates are reliable, the system computes confidence
intervals from the variance of the binned performance:

- If the 95% confidence interval for the optimal pressure spans
  more than 30% of the pressure range, data is insufficient. Use
  defaults.
- If the 95% CI is narrow (under 10% of the range), the estimate
  is reliable. Use it for adaptive pressure control.

This prevents the system from acting on noisy estimates. With
fewer than ~50 observations spread across pressure levels, the
CI is typically wide enough to trigger the default fallback. The
system earns the right to adapt by accumulating evidence first.

---

## Cognitive Load Theory Mapping

John Sweller's cognitive load theory (1988) partitions the demands
on working memory into three components. The same decomposition
applies to LLM agent context windows — the context window is the
agent's working memory, and it has finite capacity.

### Three load components in LLM context

| Cognitive Load | LLM Agent Equivalent | Source in Roko |
|---------------|---------------------|----------------|
| Intrinsic load | Task complexity: files to modify, dependency depth, domain specificity | Plan metadata, complexity classification |
| Extraneous load | Irrelevant context: stale docs, off-topic examples, verbose error history | Prompt sections with low signal_ratio |
| Germane load | Productive scaffolding: PRD context, error digests, playbook rules, skill hints | InjectContext engrams, high signal_ratio sections |

**Intrinsic load** is fixed by the task itself. A refactoring that
touches 15 files across 4 crates has high intrinsic load. A typo
fix has low intrinsic load. The system cannot reduce intrinsic load
without changing the task.

**Extraneous load** is waste. It occupies context window capacity
without contributing to task completion. Stale documentation, full
error logs from previous attempts (instead of digests), irrelevant
file contents included "for context" — all extraneous.

**Germane load** is productive overhead. PRD context that explains
why the task exists, error digests that summarize what went wrong
on the last attempt, playbook rules that encode lessons from past
failures — this content helps the agent reason better about the
task.

### The saturation constraint

The three loads compete for the same finite resource:

`intrinsic + extraneous + germane <= context_window_capacity`

When intrinsic + extraneous saturates the context, no room remains
for germane scaffolding. The agent has the task and the noise but
none of the helpful context.

The context window pressure watcher (80% threshold) exists to
enforce this constraint. By firing at 80% usage, it preserves 20%
of the window for germane content. The SystemPromptBuilder's
signal_ratio scoring on prompt sections is the mechanism for
deciding what stays (high signal) and what gets cut (low signal).

Reducing extraneous load (dropping verbose error logs, removing
stale docs) creates room for germane load (error digests,
skill suggestions, PRD context). The Conductor's job is not to
minimize total context — it is to maximize the germane-to-extraneous
ratio within the available window.

### Pressure as cognitive overload

Conductor pressure interacts with cognitive load in two ways:

1. **Each restart adds extraneous context.** When the Conductor
   restarts an agent, the new prompt includes error history from
   the previous attempt. This is necessary context — but it is
   also additional load. Three restarts can accumulate enough error
   history to crowd out germane scaffolding. The error digest
   pattern (summarize rather than include raw logs) mitigates this.

2. **Tight time pressure prevents germane processing.** An agent
   under severe time pressure rushes through the prompt, skipping
   the slower reasoning that germane scaffolding enables. The PRD
   context is there, but the agent does not use it because it
   optimizes for speed over understanding.

The Conductor must balance pressure (motivating focus) against
cognitive overload (degrading reasoning). Pressure that eliminates
drift is productive. Pressure that eliminates understanding is
destructive. The difference is whether the pressure reduces
extraneous processing (good) or germane processing (bad).

---

## Flow State Detection

Mihaly Csikszentmihalyi's flow research (1975, 1990) identifies a
psychological state of deep productive engagement. The conditions
for flow map to observable agent behaviors: clear goals, immediate
feedback, and a balance between challenge and skill. When an agent
operates in this zone, interrupting it is costly — rebuilding
context after a restart takes several turns of reduced productivity.

### Flow indicators

Observable signals distinguish flow state from collapse:

| Signal | Flow State | Collapse State |
|--------|-----------|---------------|
| Files changed per turn | Consistent, moderate | Zero or extreme |
| Gate score trajectory | Improving | Flat or declining |
| Tool utilization | Diverse, purposeful | Repetitive or absent |
| Context usage | 40-70% of window | >85% or <20% |
| Cost per meaningful change | Low, stable | High, increasing |

A flow-state agent changes a moderate number of files each turn
(not zero, not everything at once), shows improving gate scores,
uses a variety of tools for different purposes, and consumes a
healthy fraction of its context window. A collapsed agent either
produces nothing or produces frantic changes that fail gates, uses
the same tool repeatedly (or stops using tools), and either floods
its context or barely uses it.

### Flow preservation policy

When the system detects flow, it should reduce intervention
sensitivity to avoid disrupting the productive state:

- If an agent shows flow indicators for 3 or more consecutive
  turns, increase watcher thresholds by 50%
- Rationale: interrupting flow is costly. The agent needs multiple
  turns to rebuild context after any restart. A false positive
  intervention during flow destroys more value than a few extra
  turns of mild drift.
- The threshold increase is temporary and automatically reverts
  when flow indicators stop

```rust
/// Flow state detection and preservation.
/// When an agent shows sustained productive behavior, reduce intervention sensitivity
/// to avoid disrupting the productive state.
pub struct FlowDetector {
    /// Minimum consecutive productive turns to declare flow.
    pub min_flow_turns: usize,  // default: 3
    /// Threshold multiplier when flow is detected (default: 1.5 = 50% more lenient).
    pub flow_threshold_multiplier: f64,
    /// Per-agent flow state tracking.
    agent_flow: HashMap<String, FlowState>,
}

pub struct FlowState {
    pub consecutive_productive_turns: usize,
    pub in_flow: bool,
    pub flow_started_at: Option<Instant>,
}

impl FlowDetector {
    /// Update flow state based on agent's latest turn.
    pub fn update(&mut self, agent_id: &str, turn: &TurnMetrics) {
        let state = self.agent_flow.entry(agent_id.to_string())
            .or_insert(FlowState { consecutive_productive_turns: 0, in_flow: false, flow_started_at: None });

        if turn.is_productive() {
            state.consecutive_productive_turns += 1;
            if state.consecutive_productive_turns >= self.min_flow_turns && !state.in_flow {
                state.in_flow = true;
                state.flow_started_at = Some(Instant::now());
            }
        } else {
            state.consecutive_productive_turns = 0;
            state.in_flow = false;
            state.flow_started_at = None;
        }
    }
}

pub struct TurnMetrics {
    pub files_changed: usize,
    pub gate_score_improved: bool,
    pub tool_calls_diverse: bool,
    pub context_usage_ratio: f64,
}

impl TurnMetrics {
    pub fn is_productive(&self) -> bool {
        self.files_changed > 0
            && self.context_usage_ratio > 0.2
            && self.context_usage_ratio < 0.85
    }
}
```

The `is_productive` check is deliberately conservative. It requires
nonzero file changes and moderate context usage. An agent that
changes files but floods its context (>85%) or barely uses it
(<20%) is not in flow — it is either thrashing or coasting.

The flow detector does not override the circuit breaker. If the
circuit breaker fires (plan-level failure), flow state is
irrelevant — the task has failed. Flow preservation only affects
the watcher thresholds that trigger interventions below the circuit
breaker level.

---

### References

- Yerkes, R.M. & Dodson, J.D. (1908). "The relation of strength
  of stimulus to rapidity of habit-formation." *Journal of
  Comparative Neurology and Psychology*, 18, 459-482.
- Csikszentmihalyi, M. (1975/1990). *Flow: The Psychology of
  Optimal Experience*. Harper & Row.
- Sweller, J. (1988). "Cognitive load during problem solving:
  Effects on learning." *Cognitive Science*, 12(2), 257-285.
- Hanin, Y.L. (2000). "Individual Zones of Optimal Functioning
  (IZOF) model." In Y.L. Hanin (Ed.), *Emotions in Sport*.
  Human Kinetics.
- Thompson, W.R. (1933). "On the likelihood that one unknown
  probability exceeds another in view of the evidence of two
  samples." *Biometrika*, 25(3-4), 285-294.
