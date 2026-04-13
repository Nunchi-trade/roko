# Conductor learning, federation, and self-healing

> The Conductor is not static. It learns which interventions work,
> federates control across subsystem boundaries, and heals itself
> when its own model drifts from reality.


> **Implementation**: Scaffold

---

## Conductor learning -- from static rules to adaptive policy

### The learning gap

The conductor uses static thresholds. `MAX_GHOST_TURNS=3`.
`WorstSeverityPolicy`. These constants were calibrated from production
batch runs in March-April 2026 and they work for that workload. But
workloads change. Model versions change. Codebase complexity changes.
A threshold that was correct last month may be too strict or too
lenient today.

The learning infrastructure exists. `ConductorBandit` in
`roko-learn/src/conductor.rs` implements a contextual bandit for
intervention selection. The efficiency event pipeline records every
agent turn with 20+ fields of outcome data. The cascade router
already uses Thompson Sampling to learn model-task mappings. The
conductor's decision path does not use any of this.

The gap: the conductor collects data but does not learn from it.
Interventions are rule-driven, not data-driven. Closing this gap
means wiring the bandit into the conductor's `evaluate()` path,
replacing `WorstSeverityPolicy` with a learned policy that falls
back to static rules when confidence is low.

### Contextual bandit for intervention selection

The bandit models intervention selection as a contextual multi-armed
bandit problem. The state captures execution context. The actions are
conductor decisions. The reward reflects whether the intervention
improved the outcome.

**State**: 19-dimensional feature vector extracted from watcher
outputs and execution context:

- Iteration number (how many gate-fail cycles so far)
- Failure count (total failures in this plan attempt)
- Elapsed milliseconds (wall-clock time since task start)
- Accumulated cost in USD
- Model tier (0=haiku, 1=sonnet, 2=opus)
- Task complexity (from TOML frontmatter: 0=trivial, 1=simple, 2=standard, 3=complex)
- Error pattern hash (which error categories have appeared)
- Interaction terms: iteration x failure_count, cost x complexity, elapsed x model_tier

Interaction terms matter because the right intervention depends on
combinations. A high iteration count alone might mean "keep trying."
A high iteration count combined with rising cost means "abort."

**Actions**: Continue, InjectHint, SwitchModel, Restart, Abort.

**Algorithm**: Thompson Sampling blended with a linear context model.
65% Thompson (exploration), 35% linear (exploitation from context
features). The blend prevents the bandit from over-exploiting early
patterns while still using context to make informed decisions.

```rust
/// Learned conductor policy using contextual bandits.
/// Replaces static WorstSeverityPolicy with data-driven decisions.
pub struct LearnedConductorPolicy {
    /// The underlying bandit that selects actions.
    bandit: ConductorBandit,
    /// Minimum confidence before overriding static policy.
    /// Below this, fall back to WorstSeverityPolicy.
    min_confidence: f64,  // default: 0.6
    /// Number of observations before learning activates.
    warmup_observations: usize,  // default: 50
}

impl InterventionPolicy for LearnedConductorPolicy {
    fn evaluate(&self, outputs: &[WatcherOutput], ctx: &Context) -> ConductorDecision {
        // Extract features from watcher outputs and context
        let features = self.extract_features(outputs, ctx);

        if self.bandit.total_observations() < self.warmup_observations {
            // Fall back to static policy during warmup
            return WorstSeverityPolicy.evaluate(outputs, ctx);
        }

        let (action, confidence) = self.bandit.select_with_confidence(&features);

        if confidence < self.min_confidence {
            // Low confidence — use static policy as safety net
            return WorstSeverityPolicy.evaluate(outputs, ctx);
        }

        action.to_conductor_decision(outputs)
    }
}
```

The warmup period (50 observations) prevents the bandit from making
decisions before it has enough data. During warmup, the static policy
runs unchanged. After warmup, the bandit selects actions but defers
to the static policy whenever its confidence falls below 0.6. This
two-tier fallback means the learned policy can only override static
rules when it has both sufficient data and sufficient confidence.

### Reward shaping

Defining good rewards for conductor actions is the hard part. The
naive approach -- reward 1.0 for success, 0.0 for failure -- does
not capture the nuance. A well-timed Abort on a futile plan is a
good outcome. It saves tokens. It frees the executor to work on
plans that can succeed. The reward signal must reflect this.

| Action | Outcome | Reward |
|--------|---------|--------|
| Continue | Next gate passes | 0.9 |
| Continue | Next gate fails | 0.1 |
| Restart | Restarted agent succeeds | 0.8 |
| Restart | Restarted agent also fails | 0.2 |
| Fail | Plan was later retried and failed again | 0.7 (correct fail-fast) |
| Fail | Plan was later retried and succeeded | 0.1 (premature failure) |

Three design decisions in this reward table:

**Continue-pass gets 0.9, not 1.0.** Reserving 1.0 prevents reward
saturation. The bandit can always find room to improve.

**Fail-correct gets 0.7.** A correct Abort is valuable but not as
valuable as a successful Continue or Restart. The system should prefer
actions that lead to success over actions that correctly predict
failure. But correct failure prediction still earns substantial
reward because it saves tokens and wall-clock time.

**Fail-premature gets 0.1, not 0.0.** The plan was recoverable but
the conductor gave up too early. This is the worst outcome -- the
system spent tokens on a failed attempt and then spent more tokens
on a successful retry that should have been the first attempt's
continuation. The low reward (but not zero) prevents the bandit
from completely avoiding Fail actions.

The Restart-fail reward (0.2) is higher than Continue-fail (0.1)
because a restart at least attempted a different strategy. Rewarding
attempted recovery over passive continuation encourages the bandit
to try restarts when it detects problems, even if restarts do not
always succeed.

### Online learning loop

The learning loop closes within the conductor's evaluation cycle:

```
Agent turn completes
    -> Conductor evaluates (bandit selects action)
    -> Action executed (Continue/Restart/Fail)
    -> Outcome observed (next gate result)
    -> Bandit updated with (state, action, reward)
    -> Policy improves
```

The delay between action and reward varies by action type. Continue
rewards arrive on the next turn (fast feedback). Restart rewards
arrive after the restarted agent completes (slower). Fail rewards
arrive only when the plan is retried (possibly never, if the circuit
breaker trips). This variable delay means the bandit must handle
sparse and delayed rewards for Fail actions. The implementation
queues pending rewards and resolves them when outcomes become
available.

---

## Conductor federation -- multi-level control

### Four-level federation architecture

The current conductor operates at a single level: per-task. It
watches one agent executing one task and decides Continue, Restart,
or Fail. But orchestration happens at multiple levels simultaneously.
A plan contains many tasks. A batch contains many plans. A session
contains many batches. Each level has its own signals, its own
failure modes, and its own intervention options.

Federation puts a conductor at each level:

```
+----------------------------------------------------+
|  L4: Fleet Conductor (cross-plan, per-batch)       |
|  Scope: All plans in a session                      |
|  Signals: Plan outcomes, fleet-level metrics        |
|  Actions: Router policy updates, global budgets     |
|                                                      |
|  +----------------------------------------------+  |
|  |  L3: Plan Conductor (per-plan)               |  |
|  |  Scope: All tasks in one plan                 |  |
|  |  Signals: Task outcomes, plan-level cost       |  |
|  |  Actions: Resource reallocation, priority      |  |
|  |                                                |  |
|  |  +----------------------------------------+  |  |
|  |  |  L2: Task Conductor (per-task)         |  |  |
|  |  |  Current roko-conductor                |  |  |
|  |  |  10 watchers + circuit breaker         |  |  |
|  |  |  Continue / Restart / Fail              |  |  |
|  |  |                                        |  |  |
|  |  |  +--------------------------------+   |  |  |
|  |  |  |  L1: Turn Conductor            |   |  |  |
|  |  |  |  AnomalyDetector               |   |  |  |
|  |  |  |  Prompt loop, cost spike       |   |  |  |
|  |  |  +--------------------------------+   |  |  |
|  |  +----------------------------------------+  |  |
|  +----------------------------------------------+  |
+----------------------------------------------------+
```

**L1 (Turn)** operates at the granularity of a single agent turn.
The `AnomalyDetector` already does this -- it checks prompt hashes,
cost spikes, and quality degradation before each turn. L1 catches
problems before they become multi-turn patterns.

**L2 (Task)** is the current conductor. Ten watchers, one circuit
breaker, one intervention policy. It observes multi-turn patterns
within a single task and decides whether to continue, restart, or
fail. This is what `roko-conductor` implements today.

**L3 (Plan)** observes all tasks within one plan. It sees patterns
that L2 cannot: task A failed with a compile error, task B depends
on the code that A was supposed to write, so B will also fail.
L3 can reallocate resources (assign a stronger model to critical-path
tasks) or reprioritize (skip optional tasks when the budget runs low).

**L4 (Fleet)** observes all plans in a session. It sees cross-plan
patterns: three authentication-related plans failed this batch, which
suggests a systemic issue (maybe a dependency changed). L4 can update
router policies globally, adjust session-level budgets, or halt entire
categories of work.

### Conductor trait at each level

All four levels implement the same trait. Federation is achieved
through composition -- each level reads signals from the level below
and emits signals for the level above. No special hierarchy protocol
is needed. The signal stream is the communication channel.

```rust
/// All conductor levels implement the same trait.
/// Federation is achieved through composition, not hierarchy.
pub trait ConductorLevel: Send + Sync {
    /// The scope of signals this conductor observes.
    fn scope(&self) -> ConductorScope;

    /// Evaluate the signal stream and produce decisions.
    fn evaluate(&self, stream: &[Signal], ctx: &Context) -> Vec<ConductorDecision>;

    /// Accept parameter updates from the level above.
    fn accept_parameters(&mut self, params: &ParameterUpdate);

    /// Emit observations for the level above.
    fn emit_observations(&self) -> Vec<Signal>;
}

pub enum ConductorScope {
    Turn,    // L1: per-agent-turn signals
    Task,    // L2: per-task signals (current conductor)
    Plan,    // L3: per-plan signals
    Fleet,   // L4: cross-plan signals
}
```

The `accept_parameters` method is the downward channel. L4 can push
budget constraints to L3 ("this plan gets at most $5 more"). L3 can
push model selection to L2 ("use opus for this task"). L2 can push
prompt modifications to L1 ("add this hint to the next prompt").

The `emit_observations` method is the upward channel. L1 emits
anomaly signals. L2 emits intervention signals. L3 emits plan
progress signals. L4 emits fleet health signals. Each level
consumes the level below's observations as part of its own signal
stream.

### Communication via signal stream

All conductors communicate through the same signal stream that the
rest of the system uses. No side channels, no special-purpose
message queues. The signal stream is the universal bus.

Signal tags encode the level and type:

- L1 emits `conductor.anomaly.prompt_loop`, `conductor.anomaly.cost_spike`
- L2 reads L1 signals and emits `conductor.intervention.restart`, `conductor.intervention.fail`
- L3 reads L2 signals and emits `conductor.plan.budget_realloc`, `conductor.plan.reprioritize`
- L4 reads L3 signals and emits `conductor.fleet.policy_update`, `conductor.fleet.budget_adjust`

Each level filters the stream by tag prefix. L2 reads all signals
tagged `conductor.anomaly.*`. L3 reads all signals tagged
`conductor.intervention.*`. The filtering is cheap -- a prefix match
on the tag string. The signal stream's append-only JSONL format means
each level reads the full stream and filters in memory.

This design avoids the distributed systems problem of conductor-to-conductor
coordination. There is no coordinator. There is no leader election.
There is a shared log, and each conductor reads the portion relevant
to its scope.

### VSM mapping

Each federation level maps to a system in Beer's Viable System Model.
This mapping is not decorative -- it constrains what each level is
allowed to do and prevents scope creep between levels.

| Level | VSM System | Function |
|-------|-----------|----------|
| L1 (Turn) | System 2 | Coordination -- prevent oscillations within a turn |
| L2 (Task) | System 3 | Control -- internal oversight of task execution |
| L3 (Plan) | System 3* | Audit -- independent check of plan progress |
| L4 (Fleet) | System 4 | Intelligence -- scanning cross-plan patterns for adaptation |

**System 2 (L1)** dampens oscillations. The anomaly detector prevents
prompt loops and cost spikes -- these are oscillatory failure modes
where the system repeats or escalates without bound. S2's job is
stability within a turn.

**System 3 (L2)** provides internal oversight. The 10-watcher
ensemble monitors ongoing execution and intervenes when behavior
diverges from the self-model. S3's job is performance within a task.

**System 3* (L3)** is the audit function. It checks that L2's
interventions are producing good outcomes at the plan level. If L2
keeps restarting a task but the plan is not converging, L3 intervenes
at a higher level (reallocate, reprioritize, or fail the plan). S3*'s
job is accountability across tasks.

**System 4 (L4)** scans the environment for adaptation opportunities.
Cross-plan patterns reveal systemic issues (a model version regresses
on a class of tasks) or systemic opportunities (a model version
excels at a new class of tasks). S4's job is adaptation across plans.

System 5 (policy) is not a conductor level -- it is the human
operator who sets the constraints within which all four levels
operate. The `roko.toml` configuration, the plan definitions, the
acceptance criteria: these are System 5.

---

## Self-healing conductor

### Conductor failure modes

The conductor can fail. Its thresholds can drift. Its model can go
stale. Its watchers can develop blind spots. Its circuit breaker can
get stuck. A conductor that cannot detect its own failures is a
liability -- it gives false confidence that the system is regulated
when it is not.

Four failure modes, each with a distinct symptom, detection method,
and recovery path:

| Failure | Symptom | Detection | Recovery |
|---------|---------|-----------|----------|
| Threshold drift | Good plans get killed (false positives) | Intervention effectiveness drops below 50% | Bayesian threshold adaptation |
| Model staleness | Conductor interventions have no effect | Restart success rate unchanged from continue | Re-calibrate from recent efficiency events |
| Watcher blindness | New failure mode not caught by any watcher | Plans fail without intervention | Unclassified error clustering in efficiency logs |
| Circuit breaker stuck | Plans permanently tripped that should retry | Tripped plans with changed environment | Auto-probe after sleep window (half-open state) |

**Threshold drift** is the most common failure. Model versions change.
Codebase complexity changes. A threshold calibrated for Sonnet 3.5 may
be too strict for Sonnet 4 (which fails less often) or too lenient for
a smaller model (which fails more often). Detection: track the ratio of
interventions that improve outcomes. When this ratio falls below 50%,
the conductor is doing more harm than good.

**Model staleness** is subtler. The conductor intervenes (restarts an
agent), but the restarted agent fails at the same rate as the original.
The intervention has no effect. This means the conductor's model of
"what went wrong" is no longer accurate -- the restart does not address
the actual failure mode. Detection: compare restart success rate against
continue success rate. If they are statistically indistinguishable, the
restart is not helping.

**Watcher blindness** occurs when a new failure pattern emerges that
no watcher detects. The system has a type of error that causes plan
failure but does not trigger any conductor intervention. The plan
fails silently. Detection: look for plans that failed without any
conductor intervention in their history. If the ratio of
unintercepted failures rises, the watchers have a blind spot.

**Circuit breaker stuck** happens when the environment changes but
the breaker does not re-probe. A plan that failed twice because of
a provider outage should be retried after the outage resolves. The
current breaker does not probe -- it is permanently tripped until
a human resets it. Detection: check tripped plans against changed
conditions (provider health recovered, dependency updated, model
version changed).

### Recovery-oriented computing applied

The self-healing conductor borrows four principles from Patterson
et al.'s Recovery-Oriented Computing:

**Make restart cheap.** A conductor threshold reset is cheap: update
a constant, no process restart needed. The conductor can recalibrate
its thresholds without interrupting ongoing execution. This is
analogous to the micro-reboot principle -- fix the smallest unit
possible.

**Test recovery paths.** The self-model accuracy metrics
(`SelfModelAccuracy` from 08-good-regulator-self-model.md) validate
that recovery mechanisms work. If intervention effectiveness drops,
the system knows its recovery path (restart) is not effective. This
is continuous validation, not post-hoc testing.

**Micro-reboots.** Reset individual watcher thresholds without
resetting the entire conductor. If the ghost-turn watcher is too
aggressive, recalibrate that one threshold. The other nine watchers
continue with their existing calibration.

**Survivor functions.** Conductor state -- circuit breaker records,
watcher history, bandit weights -- persists through process restarts
via `.roko/state/`. A crashed orchestrator resumes with the
conductor's learned state intact. The conductor does not start from
zero on every restart.

```rust
/// Self-healing conductor that detects and repairs its own model drift.
pub struct SelfHealingConductor {
    /// The underlying conductor with all watchers and policies.
    inner: Conductor,
    /// Self-model accuracy tracker.
    accuracy: SelfModelAccuracy,
    /// Threshold learner for adaptive calibration.
    threshold_learner: ThresholdLearner,
    /// Minimum accuracy before triggering self-repair.
    min_accuracy: f64,  // default: 0.5
    /// Interval between self-assessments.
    self_check_interval: Duration,  // default: 300s (5 min)
}

impl SelfHealingConductor {
    pub fn self_assess(&mut self) -> Option<SelfRepairAction> {
        // Check intervention effectiveness
        if self.accuracy.intervention_effectiveness < self.min_accuracy {
            return Some(SelfRepairAction::RecalibrateThresholds);
        }
        // Check for undetected failures
        if self.accuracy.stuck_detection_precision < 0.3 {
            return Some(SelfRepairAction::ExpandStuckHeuristics);
        }
        // Check for watcher blindness (plans failing without conductor intervention)
        if self.accuracy.undetected_failure_rate() > 0.2 {
            return Some(SelfRepairAction::AddNewWatcher);
        }
        None
    }
}

pub enum SelfRepairAction {
    RecalibrateThresholds,
    ExpandStuckHeuristics,
    AddNewWatcher,
    ResetCircuitBreakers,
    RetrainBandit,
}
```

The `self_assess` method runs on a 5-minute interval during batch
execution. It checks three conditions in priority order:

1. **Intervention effectiveness below 50%.** The conductor's
   interventions are failing more often than succeeding. The most
   common cause is threshold drift. Recovery: `RecalibrateThresholds`
   triggers the `ThresholdLearner` (from 08-good-regulator-self-model.md)
   to adjust thresholds based on recent outcome data.

2. **Stuck detection precision below 30%.** The stuck detector is
   flagging agents that are not actually stuck. More than 70% of
   "stuck" detections are false positives. Recovery:
   `ExpandStuckHeuristics` tightens the stuck detection thresholds
   so they trigger less often.

3. **Undetected failure rate above 20%.** More than one in five plan
   failures occurs without any conductor intervention. The watchers
   have a blind spot. Recovery: `AddNewWatcher` clusters unclassified
   errors from the efficiency logs and proposes a new watcher pattern.

The priority order matters. Threshold drift is checked first because
it is the most common failure mode and the cheapest to fix. Watcher
blindness is checked last because it is the hardest to fix -- adding
a new watcher requires identifying the new failure pattern and
implementing a detection heuristic.

### Triple-loop learning

Self-healing operates at three levels of abstraction. Each level
fixes a different class of problem.

```
Loop 1 (Single-loop): Correct errors
    Agent fails -> Conductor restarts -> Agent succeeds
    The system fixes the immediate problem.

Loop 2 (Double-loop): Change the rules
    Conductor thresholds produce too many false positives
    -> ThresholdLearner adjusts thresholds
    -> Future interventions are more accurate
    The system improves its own detection.

Loop 3 (Triple-loop): Change the meta-rules
    The threshold learning rate is too slow (or too fast)
    -> Self-model accuracy metrics detect the meta-problem
    -> Learning parameters are adjusted
    The system improves its own improvement process.
```

**Single-loop** is what the conductor does today. An agent fails. The
conductor detects the failure pattern. It restarts the agent or fails
the plan. The immediate problem is addressed. No learning occurs --
the same threshold, the same watcher, the same intervention. If the
same failure happens again tomorrow, the same intervention fires.

**Double-loop** changes the thresholds. The `ThresholdLearner` tracks
intervention effectiveness per watcher. If the ghost-turn watcher's
interventions succeed 90% of the time, its threshold might be too
lenient (it is only catching the obvious cases). If interventions
succeed 30% of the time, the threshold is too strict (too many false
positives). The learner adjusts the threshold toward the sweet spot.
This changes the conductor's behavior for future similar situations.

**Triple-loop** changes the learning process itself. The self-model
accuracy metrics track whether the double-loop is converging. If
threshold adjustments are oscillating (too strict, then too lenient,
then too strict again), the learning rate is too high. If thresholds
barely move despite clear evidence of drift, the learning rate is
too low. The triple-loop adjusts the learning rate, the discount
factor, and the minimum sample size for the `ThresholdLearner`.

The practical test for whether triple-loop learning is needed: does
intervention effectiveness stabilize after double-loop adjustments?
If it does, double-loop is sufficient. If it oscillates or fails to
converge, the learning parameters themselves need adjustment -- and
that is the triple-loop.

This maps to Argyris and Schon's organizational learning framework:
single-loop corrects deviations within existing norms, double-loop
questions and revises the norms, triple-loop questions the process
by which norms are revised. The conductor implements all three
levels computationally.

---

## File reference

| File | What |
|------|------|
| `crates/roko-learn/src/conductor.rs` | ConductorBandit (built, not wired) |
| `crates/roko-conductor/src/conductor.rs` | Current static conductor |
| `crates/roko-conductor/src/interventions.rs` | InterventionPolicy trait |
| `crates/roko-learn/src/anomaly.rs` | AnomalyDetector (L1 conductor) |
| `crates/roko-learn/src/efficiency.rs` | AgentEfficiencyEvent (reward data) |
| `crates/roko-learn/src/cascade_router.rs` | CascadeRouter (L4 conductor analogue) |

---

## Cross-References

- [02-circuit-breaker.md](02-circuit-breaker.md) -- Circuit breaker, half-open state
- [07-ooda-cybernetic-loop.md](07-ooda-cybernetic-loop.md) -- OODA loop, nested loops, IG&C
- [08-good-regulator-self-model.md](08-good-regulator-self-model.md) -- Self-model, accuracy metrics
- [11-anomaly-detection-learning.md](11-anomaly-detection-learning.md) -- Learning integration, feedback loops
- [12-yerkes-dodson-pressure.md](12-yerkes-dodson-pressure.md) -- Pressure calibration, curve fitting
