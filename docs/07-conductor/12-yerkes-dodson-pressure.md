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
