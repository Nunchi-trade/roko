# 05 — Agent

> Agent = Space (owns Bus partition + Store partition) + Extensions + Memory + adaptive clock + vitality. The cognitive loop is a Loop pattern. Cross-cuts (Memory, Daimon, Dreams) are Functor patterns. Every agent is mortal.

**Subsumes**: AgentRuntime, TickPipeline, CorticalState, AdaptiveClock, T0/T1/T2 gating, DomainProfile, AgentMode, Vitality, SomaticMarkers, CognitiveWorkspace, CognitiveEnergy, GoalEmergence, EnergyAffectCoupling.

**Depends on**: [01-SIGNAL](01-SIGNAL.md) (Signal/Pulse duality, demurrage, HDC fingerprints), [02-CELL](02-CELL.md) (9 protocols, predict-publish-correct, Verify redesign, EFE routing), [03-GRAPH](03-GRAPH.md) (Graph, Hot Graph, FanOut), [04-EXECUTION](04-EXECUTION.md) (Engine, Flow, snapshot/resume)

---

## 1. Overview

An **Agent** is the most complex specialization: a Space + Extensions + Memory + adaptive clock + vitality. Every agent -- in-process or remote -- runs the same core loop. The agent's cognitive pipeline is itself a Hot Graph, interpreted by the same Engine that runs all other Graphs.

### Core framing

```
Agent = Space + Extensions + Memory + adaptive clock + vitality
```

| Component | What | Kernel Primitive |
|---|---|---|
| **Space** | Isolation boundary (Bus partition + Store partition) + capability grants | Space pattern |
| **Extensions** | Interceptor Cells across 8 layers | Functor pattern (Signal endofunctors) |
| **Memory** | Store-protocol Cell with demurrage + dreams | Memory specialization ([06-MEMORY](06-MEMORY.md)) |
| **Adaptive clock** | Tick frequency control across 3 timescales | Three nested Hot Graphs |
| **Vitality** | `remaining_budget / initial_budget` | Economic pressure scalar driving behavioral phases |

The cognitive loop is a **Loop pattern** (Graph with feedback edge): the output of REFLECT feeds back into OBSERVE on the next tick. The three cognitive timescales (gamma, theta, delta) are three **Hot Graphs** running concurrently inside a single Agent -- not scheduling hints, not mode flags, but independent Graphs with independent failure isolation, budget accounting, and snapshot/resume.

Cross-cuts -- Memory, Daimon (affect), Dreams, Safety -- are **Functor patterns**: Signal endofunctors that enrich or constrain Signals pre/post a Cell without changing the Graph's topology.

---

## 2. Type-State Lifecycle

Agent states are compile-time enforced. Each state restricts which operations are permitted. Calling a method unavailable in the current state is a type error, not a runtime error.

```rust
pub struct Agent<S: AgentState> {
    pub id: AgentId,
    pub config: AgentConfig,
    pub space: Space,
    pub extensions: ExtensionChain,
    pub memory: MemoryCell,
    pub clock: AdaptiveClock,
    pub vitality: VitalityTracker,
    pub energy: CognitiveEnergy,
    pub cortical: Arc<CorticalState>,
    pub slots: SlotManager,
    _state: PhantomData<S>,
}

pub struct Provisioning;
pub struct Active;
pub struct Dreaming;
pub struct Terminal;

pub trait AgentState: sealed::Sealed {}
impl AgentState for Provisioning {}
impl AgentState for Active {}
impl AgentState for Dreaming {}
impl AgentState for Terminal {}
```

### Transition table

```
Provisioning ──activate()──► Active
Active ──────sleep()───────► Dreaming
Dreaming ────wake()────────► Active
Active ──────terminate()───► Terminal
Dreaming ────terminate()───► Terminal
```

| Transition | Method | Precondition | Side effects |
|---|---|---|---|
| `Provisioning -> Active` | `activate()` | Extensions loaded, Space grants validated, Memory initialized | Emits `AgentStateTransition` Pulse; starts adaptive clock |
| `Active -> Dreaming` | `sleep()` | Sleep pressure threshold met OR idle timeout | Pauses pipeline; triggers dream consolidation ([06-MEMORY](06-MEMORY.md) SS9) |
| `Dreaming -> Active` | `wake()` | Dream cycle complete OR external interrupt | Resumes pipeline; integrates consolidated knowledge |
| `Active -> Terminal` | `terminate()` | Budget exhausted OR explicit shutdown | Knowledge export; Episode flush; Extension shutdown (reverse order) |
| `Dreaming -> Terminal` | `terminate()` | Budget exhausted during dream | Aborts dream; best-effort knowledge flush |

### State-restricted operations

| Operation | Provisioning | Active | Dreaming | Terminal |
|---|---|---|---|---|
| Load Extensions | Yes | No | No | No |
| Run pipeline tick | No | Yes | No | No |
| Execute tool calls | No | Yes | No | No |
| Run dream cycle | No | No | Yes | No |
| Query Memory | No | Yes | Yes (read-only) | No |
| Export knowledge | No | Yes | Yes | Yes (flush) |
| Receive messages | No | Yes | Queued | No |

Attempting an operation in the wrong state is a compile error. The `Agent<Dreaming>` type simply does not have a `tick()` method.

---

## 3. Vitality

Vitality is the economic pressure scalar: `remaining_budget / initial_budget`. It declines monotonically as the agent spends resources, creating five behavioral phases that modulate decision-making. Mortality is a feature, not a bug -- an agent that has never faced resource pressure has never learned to prioritize (cf. Jonas 1966, *The Phenomenon of Life*).

```rust
pub struct VitalityTracker {
    pub initial_budget: Cost,
    pub remaining_budget: Cost,
    pub phase: VitalityPhase,
}

impl VitalityTracker {
    pub fn vitality(&self) -> f64 {
        self.remaining_budget.as_f64() / self.initial_budget.as_f64()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VitalityPhase {
    Thriving,      // 1.0 - 0.7
    Stable,        // 0.7 - 0.4
    Conservation,  // 0.4 - 0.2
    Declining,     // 0.2 - 0.05
    Terminal,      // < 0.05
}
```

### Phase behaviors

| Phase | Vitality | EFE cost term | Compose budget | Verify criteria | Behavioral shift |
|---|---|---|---|---|---|
| **Thriving** | 1.0 - 0.7 | 1.0x (baseline) | Full allocation | Full rigor | Explore freely; invest in learning |
| **Stable** | 0.7 - 0.4 | 1.2x (slight cost pressure) | 90% allocation | Full rigor | Balanced exploration/exploitation |
| **Conservation** | 0.4 - 0.2 | 1.8x (strong cost pressure) | 60% allocation | Relaxed soft criteria | Favor known strategies; reduce exploration; prefer T0/T1 |
| **Declining** | 0.2 - 0.05 | 3.0x (severe cost pressure) | 30% allocation | Minimum viable | Complete current task only; no speculative work; transfer knowledge |
| **Terminal** | < 0.05 | N/A | 0 (sleepwalk) | Skip | Flush episodes; export knowledge; emit farewell Pulse; terminate |

### Phase transitions

Phase transitions emit `AgentPhaseChange` Pulses on the Bus (topic `agent:{id}.phase.changed`). Extensions in L6 Meta can react via the `on_reflect()` hook.

The vitality scalar is readable from `CorticalState` (section 4) as an `AtomicF64`, enabling sub-microsecond concurrent reads from any slot or Extension without locking.

---

## 4. CorticalState

CorticalState is the lock-free atomic shared perception surface. Multiple concurrent slots, Extensions, and Lenses read from CorticalState without synchronization overhead. Writes use atomic operations -- no mutexes, no contention.

```rust
pub struct CorticalState {
    // ── Continuous signals ──────────────────────────────────────
    pub prediction_error: AtomicF64,     // current PE (0.0..=1.0)
    pub vitality: AtomicF64,             // remaining_budget / initial_budget
    pub confidence: AtomicF64,           // agent's self-assessed confidence

    // ── Regime ─────────────────────────────────────────────────
    pub regime: AtomicRegime,            // Calm | Normal | Volatile | Crisis

    // ── Affect ─────────────────────────────────────────────────
    pub affect: AtomicPAD,               // Pleasure / Arousal / Dominance

    // ── Counters ───────────────────────────────────────────────
    pub tick_count: AtomicU64,
    pub episode_count: AtomicU64,
    pub gate_pass_count: AtomicU64,
    pub gate_fail_count: AtomicU64,

    // ── Budget ─────────────────────────────────────────────────
    pub budget_spent: AtomicU64,         // microdollars
    pub budget_remaining: AtomicU64,     // microdollars

    // ── Slot state ─────────────────────────────────────────────
    pub active_slots: AtomicU32,
    pub slot_states: Arc<[AtomicSlotState]>,
}
```

### Atomic types

```rust
pub struct AtomicF64(AtomicU64);        // f64 stored via to_bits/from_bits
pub struct AtomicRegime(AtomicU8);      // Calm=0, Normal=1, Volatile=2, Crisis=3
pub struct AtomicPAD {                   // Pleasure/Arousal/Dominance
    pub pleasure: AtomicF64,             // -1.0..=1.0
    pub arousal: AtomicF64,              // -1.0..=1.0
    pub dominance: AtomicF64,            // -1.0..=1.0
}
```

### Read cost

All reads are single atomic loads -- no CAS loops, no spinlocks. On x86-64 with `Ordering::Relaxed`, a CorticalState read completes in under 1 microsecond. This matters because the cognitive pipeline reads CorticalState on every tick, and multiple concurrent slots read it simultaneously.

### Write protocol

Only the pipeline's owner thread writes to CorticalState. Slots and Extensions read only. This single-writer / multiple-reader pattern eliminates write contention entirely.

---

## 5. Multi-Slot State

An Agent manages N named concurrent slots, each executing an independent task. Slots share the agent's global budget, Memory, CorticalState, and Extension chain, but maintain per-slot task assignment, scratchpad, and capability guards.

```rust
pub struct SlotManager {
    pub slots: Vec<Slot>,
    pub max_slots: usize,
    pub global_budget: Arc<VitalityTracker>,
    pub global_memory: Arc<MemoryCell>,
    pub global_cortical: Arc<CorticalState>,
}

pub struct Slot {
    pub name: SlotName,
    pub state: SlotState,
    pub task: Option<TaskAssignment>,
    pub scratchpad: Value,
    pub capabilities: CapabilitySet,
    pub local_context: Vec<Signal>,
}

#[derive(Debug, Clone, Copy)]
pub enum SlotState {
    Idle,
    Active,
    Blocked { reason: String },
    Completed,
}
```

### Slot budget sharing

All slots draw from the same global budget. The `VitalityTracker.remaining_budget` is backed by an `AtomicU64` (microdollars) to enable contention-free concurrent spending from multiple slots.

**CAS commit protocol**: When a slot spends budget, it uses compare-and-swap (CAS) to atomically deduct:

```rust
loop {
    let current = remaining_budget.load(Ordering::Acquire);
    let cost_micros = cost.to_microdollars();
    if cost_micros > current {
        return Err(BudgetExhausted { spent: current, limit: initial_budget });
    }
    match remaining_budget.compare_exchange_weak(
        current,
        current - cost_micros,
        Ordering::AcqRel,
        Ordering::Relaxed,
    ) {
        Ok(_) => break,   // committed
        Err(_) => continue, // another slot spent concurrently, retry
    }
}
```

This ensures no slot can overdraw the budget even under concurrent spending. The CAS loop is bounded (typically 1-2 iterations) because contention is low -- slots spend at human timescales, not nanosecond timescales.

When one slot's spending causes a phase transition (e.g., Stable -> Conservation), all slots observe the new phase via CorticalState.

### Slot capability guards

Each slot inherits the agent's Space grants but may have additional per-slot restrictions. A slot assigned to "read documentation" has `{read_file, web_search}` capabilities. A slot assigned to "write code" has `{read_file, write_file, execute_command}` capabilities. Capability intersection is fail-closed -- a slot never has more capabilities than the agent's Space grants.

---

## 6. Three Agent Modes

```rust
#[derive(Debug, Clone, Copy)]
pub enum AgentMode {
    /// Single-shot: receive task, execute, terminate.
    /// No persistent state between invocations.
    Ephemeral,

    /// Long-running: maintain state across tasks.
    /// Sleep/wake cycle. Knowledge persists.
    Persistent,

    /// Event-driven: subscribe to Bus topics, react to Pulses.
    /// Minimal compute between events.
    Reactive,
}
```

| Mode | Clock | Memory | Vitality | Use case |
|---|---|---|---|---|
| **Ephemeral** | Fixed gamma | Session-only | Single task budget | One-shot code tasks, research queries |
| **Persistent** | Adaptive (Gamma/Theta/Delta) | Durable across sessions | Cumulative budget | Long-running development agents |
| **Reactive** | Event-driven (fires on Pulse) | Durable | Per-event micro-budget | Monitoring, CI/CD watchers, alert handlers |

---

## 7. Three Cognitive Timescales as Nested Hot Graphs

The three cognitive speeds -- gamma (reactive, ~5-15s), theta (reflective, ~75s), delta (consolidation, hours) -- are three distinct Hot Graphs running concurrently inside a single Agent. Each is a resident Graph that re-fires on its own clock, interpreted by the same Engine that runs task plans.

```
Agent
  |
  +-- gamma_loop: Hot Graph, adaptive clock (100ms - 500ms base)
  |     fires: every tick
  |     cost: $0 for ~80% of ticks (T0 short-circuit)
  |     role: perception, reflexes, fast action
  |
  +-- theta_loop: Hot Graph, adaptive clock (500ms - 16s base)
  |     fires: every N gamma ticks (adaptive)
  |     cost: T1 or T2 per tick
  |     role: planning, evaluation, strategy adjustment
  |
  +-- delta_loop: Hot Graph, adaptive clock (60s - 600s base)
        fires: on idle or scheduled
        cost: T2 per tick
        role: dream consolidation, knowledge synthesis, pruning
```

### Why three loops instead of one with a scheduler

A single loop with a "which timescale this tick?" decision conflates two concerns: clock frequency and cognitive purpose. Separating them gives:

1. **Independent failure isolation**: A delta dream that crashes does not stop gamma perception.
2. **Independent budget accounting**: Gamma ticks are nearly free; theta and delta draw from different budget pools.
3. **Independent snapshot/resume**: Each loop has its own FlowSnapshot. Resuming gamma does not require replaying delta's dream state.
4. **Regime-independent delta**: The adaptive clock modulates gamma and theta by regime (Calm slows gamma by 4x, Crisis speeds it by 4x). Delta is less affected (0.5x in Crisis, 1.0x elsewhere). Separate loops make this natural.

### Adaptive Clock

```rust
pub struct AdaptiveClock {
    pub gamma: Duration,      // fast: perception + reflexes (100ms - 500ms)
    pub theta: Duration,      // medium: planning + execution (500ms - 16s)
    pub delta: Duration,      // slow: consolidation + reflection (60s - 600s)
    pub regime: Regime,
    pub hysteresis_counter: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    Calm,       // environment stable, low PE
    Normal,     // baseline
    Volatile,   // high PE, frequent changes
    Crisis,     // critical failure or budget emergency
}
```

### Regime-based clock adjustment

| Regime | Gamma multiplier | Theta multiplier | Delta multiplier | Effect |
|---|---|---|---|---|
| **Calm** | 4.0x (slower) | 2.0x (slower) | 1.0x | Conserve resources; environment is predictable |
| **Normal** | 1.0x (baseline) | 1.0x | 1.0x | Standard operating tempo |
| **Volatile** | 0.5x (faster) | 0.5x (faster) | 1.0x | Increase perceptual acuity; detect changes quickly |
| **Crisis** | 0.25x (fastest) | 0.25x (fastest) | 0.5x (faster) | Maximum responsiveness; consolidate faster |

### Regime transitions

Regime transitions require **3-tick hysteresis** to prevent oscillation. The system must observe 3 consecutive ticks of regime-qualifying conditions before transitioning. This prevents a single PE spike from triggering Crisis mode.

```rust
impl AdaptiveClock {
    pub fn evaluate_regime(&mut self, pe: f64, vitality: f64) -> Option<Regime> {
        let candidate = match (pe, vitality) {
            (pe, _) if pe > 0.7 => Regime::Crisis,
            (pe, _) if pe > 0.4 => Regime::Volatile,
            (pe, v) if pe < 0.1 && v > 0.5 => Regime::Calm,
            _ => Regime::Normal,
        };

        if candidate == self.regime {
            self.hysteresis_counter = 0;
            return None;
        }

        self.hysteresis_counter += 1;
        if self.hysteresis_counter >= 3 {
            self.hysteresis_counter = 0;
            let old = self.regime;
            self.regime = candidate;
            Some(candidate) // emit AgentRegimeChange Pulse
        } else {
            None
        }
    }
}
```

### Theta cadence adjustment

Theta cadence shortens under stress:
- **Stalling** (completion rate below 25%): theta interval x 0.5
- **Anxious** (low confidence + high arousal + low dominance): theta interval x 0.66

```rust
impl AdaptiveClock {
    fn theta_interval_adjusted(&self, ctx: &ScheduleContext) -> Duration {
        let base = self.theta;

        // Stalling: reflect sooner
        if ctx.completion_rate <= 0.25 {
            return base.mul_f64(0.5);
        }

        // Anxious: reflect sooner
        if ctx.confidence < 0.3
            && (ctx.arousal > 0.25 || ctx.dominance < -0.1)
        {
            return base.mul_f64(0.66);
        }

        base
    }
}
```

---

## 8. The 9-Step Pipeline as Hot Graph

The Agent's pipeline is a Hot Graph -- it stays resident and re-fires every tick. The same execution Engine that runs task Graphs interprets this pipeline.

```
Step 1: OBSERVE     ──► Gather observations from environment + Bus + pheromones
Step 2: RETRIEVE    ──► Query Memory, assemble context via CognitiveWorkspace
Step 3: ANALYZE     ──► Score observations, compute prediction error
Step 4: GATE        ──► EFE evaluation: T0/T1/T2 tier selection
Step 5: SIMULATE    ──► Generate candidate actions (LLM inference at selected tier)
Step 6: VALIDATE    ──► Pre-action verification (verify_pre)
Step 7: EXECUTE     ──► Run the selected action (tool call, code generation, etc.)
Step 8: VERIFY      ──► Post-action verification (verify_post), continuous reward
Step 9: REFLECT     ──► Log episode, update CorticalState, reinforce/demote knowledge
```

### Extension hook points

Each pipeline step has associated Extension hooks (Functor pattern -- Signals flow through interceptors without changing the Graph topology).

| Step | Extension Layer | Hooks |
|---|---|---|
| 1. Observe | L1 Perception (Pulse) | `on_observe`, `filter_input` |
| 2. Retrieve | L2 Memory (Signal) | `on_retrieve`, `on_store` |
| 3. Analyze | (no hooks) | Internal scoring |
| 4. Gate | L3 Cognition (Signal) | `on_gate` |
| 5. Simulate | L3 Cognition (Signal) | `pre_inference`, `post_inference` |
| 6. Validate | (Verify protocol) | `verify_pre` |
| 7. Execute | L4 Action (Signal) | `pre_action`, `post_action`, `on_tool_call` |
| 8. Verify | (Verify protocol) | `verify_post` |
| 9. Reflect | L6 Meta (Signal) | `on_reflect`, `on_cost_update` |

---

## 9. EFE Gating: Dual-Process Routing

Expected Free Energy (Friston 2006) replaces static prediction-error thresholds for T0/T1/T2 tier selection. The three tiers map to dual-process theory: T0 (System 1 reflexes), T1/T2 (System 2 deliberation at two depth levels). EFE subsumes UCB/epsilon-greedy by decomposing the value of each candidate into three terms.

### The three inference tiers

| Tier | Name | What Runs | Latency | Cost | When |
|---|---|---|---|---|---|
| T0 | Reflex | Pattern-match Cells (no LLM) | <50ms | ~$0 | Known patterns, cache hits, heuristic matches |
| T1 | Fast | Small/cached model Cells (Haiku-class) | 1-5s | $0.001-0.01 | Moderate complexity, familiar domains |
| T2 | Deep | Full model Cells (Opus-class) | 10-120s | $0.01-1.00 | Novel problems, high-stakes decisions |

### EFE formula

```
EFE(tier) = -epistemic_value(tier) - pragmatic_value(tier) + cost(tier) + regime_penalty(tier)
```

Lower EFE is better. The system selects `argmin(EFE)` across the three tiers.

| Component | What it measures | Effect on tier selection |
|---|---|---|
| `epistemic_value` | Expected information gain from acting at this tier | High -> favor this tier (we learn something) |
| `pragmatic_value` | Expected goal advancement from acting at this tier | High -> favor this tier (we achieve something) |
| `cost` | Resource expenditure at this tier | High -> disfavor this tier (it is expensive) |
| `regime_penalty` | Regime-conditioned adjustment | Crisis: cost weighted higher, epistemic value boosted |

### Why EFE naturally produces the T0 -> T1 -> T2 cascade

The cascade emerges from the cost structure:

```
T0 cost = $0.000  (pure Rust, no LLM)
T1 cost = $0.001  (Haiku-class model)
T2 cost = $0.100  (Opus-class model)
```

When nothing is surprising:
- Epistemic value is low across all tiers (nothing to learn).
- Pragmatic value is moderate at T0 (reflex can handle it).
- Cost dominates: T0 wins because $0 < $0.001 < $0.100.

When mildly surprising:
- Epistemic value is moderate at T1 (quick analysis reveals what changed).
- Pragmatic value is low at T0 (reflex does not cover this case).
- T1 wins: moderate information gain at moderate cost.

When very surprising or high-stakes:
- Epistemic value is high at T2 (deep analysis needed to understand).
- Pragmatic value is high at T2 (only a capable model can solve this).
- Cost is dominated by value: T2 wins despite being 100x more expensive.

### EFE tier selection implementation

```rust
pub struct EFEEvaluation {
    pub tier: CognitiveTier,
    pub epistemic_value: f64,    // expected information gain
    pub pragmatic_value: f64,    // expected goal advancement
    pub cost: f64,               // resource expenditure
    pub regime_penalty: f64,     // regime-conditioned adjustment
}

#[derive(Debug, Clone, Copy)]
pub enum CognitiveTier {
    /// T0: Pure Rust pattern matching. No LLM call. ~80% of ticks.
    T0Reflex,
    /// T1: Lightweight model (Haiku-class). Fast inference.
    T1Deliberate,
    /// T2: Full model (Opus-class). Deep reasoning.
    T2Reflective,
}

fn select_tier(
    probes: &T0ProbeResults,
    cortical: &CorticalState,
    vitality: f64,
    regime: Regime,
) -> CognitiveTier {
    // T0 evaluation: all probes quiet?
    if probes.all_quiet() {
        return CognitiveTier::T0Reflex;
    }

    // Compute EFE for T1 and T2
    let surprise = probes.max_surprise();
    let stakes = probes.max_stakes();

    let efe_t1 = EFEEvaluation {
        tier: CognitiveTier::T1Deliberate,
        epistemic_value: surprise * 0.6,     // T1 partially resolves uncertainty
        pragmatic_value: (1.0 - stakes) * 0.5, // T1 handles moderate-stakes work
        cost: 0.001 * vitality_cost_multiplier(vitality),
        regime_penalty: regime_penalty(regime, CognitiveTier::T1Deliberate),
    };

    let efe_t2 = EFEEvaluation {
        tier: CognitiveTier::T2Reflective,
        epistemic_value: surprise * 1.0,     // T2 fully resolves uncertainty
        pragmatic_value: stakes * 0.9,       // T2 handles high-stakes work
        cost: 0.100 * vitality_cost_multiplier(vitality),
        regime_penalty: regime_penalty(regime, CognitiveTier::T2Reflective),
    };

    // Select minimum EFE
    if efe_t1.total() < efe_t2.total() {
        CognitiveTier::T1Deliberate
    } else {
        CognitiveTier::T2Reflective
    }
}

/// Vitality-based cost multiplier. Low vitality makes cost weigh more.
fn vitality_cost_multiplier(vitality: f64) -> f64 {
    match vitality {
        v if v > 0.7 => 1.0,    // Thriving: baseline cost
        v if v > 0.4 => 1.2,    // Stable: slight cost pressure
        v if v > 0.2 => 1.8,    // Conservation: strong cost pressure
        v if v > 0.05 => 3.0,   // Declining: severe cost pressure
        _ => f64::INFINITY,      // Terminal: no LLM calls
    }
}

/// Regime-based penalty. Adjusts the EFE landscape per regime.
fn regime_penalty(regime: Regime, tier: CognitiveTier) -> f64 {
    match (regime, tier) {
        // Crisis: penalize expensive tiers heavily, but boost epistemic value
        (Regime::Crisis, CognitiveTier::T2Reflective) => 0.5,
        (Regime::Crisis, CognitiveTier::T1Deliberate) => 0.1,
        // Volatile: boost epistemic value (seek information)
        (Regime::Volatile, _) => -0.2,
        // Calm: penalize less (we have resources to spare)
        (Regime::Calm, _) => -0.1,
        _ => 0.0,
    }
}
```

### Regime conditioning on EFE

| Regime | Effect on EFE landscape |
|---|---|
| **Calm** | Pragmatic value weighted higher; cost weighted lower. Favor goal advancement. |
| **Normal** | Balanced weights. Baseline. |
| **Volatile** | Epistemic value weighted higher. Seek information to resolve uncertainty. |
| **Crisis** | Cost weighted much higher. Epistemic value boosted. Avoid expensive mistakes while maximizing learning. |

### The approximate EFE

Computing exact EFE over a full generative model is intractable. Roko approximates it from four signals already available on the CorticalState:

1. **Prediction accuracy** from the calibration stream: declining accuracy -> high epistemic value.
2. **Confidence from Score**: low confidence on recent outputs -> high uncertainty.
3. **Novelty from Score**: high novelty in observations -> high epistemic value.
4. **Daimon arousal**: high arousal (the agent is "surprised") -> escalate.

These signals combine without requiring explicit EFE computation. The approximation works because each signal is a partial derivative of the true EFE surface: prediction error approximates epistemic value, confidence approximates pragmatic value certainty, and arousal is a somatic integration of both.

### Why EFE > LinUCB

| Property | LinUCB | EFE |
|---|---|---|
| Exploration | Bonus from confidence interval width | Explicit epistemic term (KL divergence) |
| Context | Linear features only | Full context via CorticalState |
| Regime awareness | None (manual override) | Regime -> prior shifts (crisis amplifies pragmatic) |
| Cost-awareness | None | Explicit cost term in objective |
| Composability | Standalone | EFE is additive across cascaded decisions |

### Progressive cascade emergence

EFE naturally produces cascading behavior without hard-coded rules:

1. **Novel task**: High epistemic value for T2 (never tried) -> T2 selected -> learns.
2. **Familiar task**: T0 reflex has strong pragmatic record -> T0 selected -> fast.
3. **Moderate task**: T1 has some history, T2 expensive -> T1 selected -> balanced.

The cascade "learns itself" through the predict-publish-correct loop on Bus:
- `prediction.route.{cell_id}` published before selection.
- `outcome.route.{cell_id}` published after Verify verdict.
- CalibrationReact joins and updates per-candidate EFE priors.

---

## 10. The 16 T0 Probes

T0 probes are zero-LLM diagnostic checks that run at gamma frequency. Each probe is a Cell with typed I/O, implementing the Observe protocol. When any probe reports surprise above threshold, the tick escalates to T1 or T2.

### Probe Cell interface

```rust
/// A T0 probe. Checks one aspect of the environment.
/// Returns a ProbeResult with surprise level and optional evidence.
trait T0Probe: Cell {
    /// The surprise threshold above which this probe triggers escalation.
    fn threshold(&self) -> f32;

    /// The stakes level of what this probe checks.
    /// High-stakes probes can trigger direct T2 escalation.
    fn stakes(&self) -> f32;
}

struct ProbeResult {
    /// 0.0 = no change, 1.0 = maximum surprise.
    surprise: f32,

    /// Optional evidence for downstream analysis.
    evidence: Option<Value>,

    /// Whether this probe recommends escalation.
    escalate: bool,
}
```

### The 16 probes

| # | Probe | Input | Output | Cost |
|---|---|---|---|---|
| 1 | `ConfigChangedProbe` | `last_config_hash` | surprise 1.0 if changed | O(1) hash comparison |
| 2 | `GateFailedRecentlyProbe` | gate verdict stream from Bus | surprise 0.8 if recent failure | O(1) counter check |
| 3 | `FileModifiedProbe` | watched file paths + last mtimes | surprise 0.6 per changed file | O(k) stat calls, k < 20 |
| 4 | `TestCountDeltaProbe` | cached test count | surprise 0.7 if count changed | O(1) comparison |
| 5 | `CompileErrorNewProbe` | cached compile error count | surprise 1.0 if new errors | O(1) comparison |
| 6 | `BudgetThresholdProbe` | CorticalState.budget_remaining | surprise scales with proximity | O(1) atomic read |
| 7 | `ConfidenceDroppingProbe` | confidence ring buffer (last N) | surprise = slope magnitude if negative | O(N), N ~ 10 |
| 8 | `PredictionViolationProbe` | prediction/outcome pairs | surprise = PE magnitude | O(1) per pair |
| 9 | `ToolHealthDegradedProbe` | tool response time EWMAs | surprise if EWMA > 2x baseline | O(t), t < 20 |
| 10 | `PheromoneDetectedProbe` | pheromone Pulses from Bus | surprise = intensity of strongest | O(p), p ~ 0 |
| 11 | `TaskDeadlineNearProbe` | task deadline + current time | surprise scales with closeness | O(1) time comparison |
| 12 | `IdleTimeoutProbe` | time since last non-T0 tick | escalate to Delta if idle > threshold | O(1) time comparison |
| 13 | `KnowledgeStaleProbe` | freshness timestamps on key Memory entries | surprise if past freshness window | O(k), k < 50 |
| 14 | `DependencyChangedProbe` | upstream task completion Pulses | surprise 0.5 per newly completed dep | O(d), d < 10 |
| 15 | `MetricAnomalyProbe` | tracked metrics + running mean/variance | surprise if outside 2-sigma bounds | O(m), m < 30 |
| 16 | `HeartbeatTimeoutProbe` | time since last heartbeat emission | always emits heartbeat; surprise if overdue | O(1) + Pulse emission |

### Gamma probe Graph

All 16 probes execute as a FanOut in the gamma loop's ASSESS step. They run in parallel (each is independent), and results are aggregated.

```toml
[graph]
name = "gamma-probe-fanout"
version = "1.0.0"

# Entry node: distribute cortical snapshot to all probes
[[graph.nodes]]
id = "distribute"
cell = "roko.internal.signal_copy"

# 16 probe nodes (parallel)
[[graph.nodes]]
id = "probe_config_changed"
cell = "roko.probe.config_changed"

[[graph.nodes]]
id = "probe_gate_failed"
cell = "roko.probe.gate_failed_recently"

# ... probes 3-16 follow the same pattern ...

[[graph.nodes]]
id = "probe_heartbeat"
cell = "roko.probe.heartbeat_timeout"

# Aggregation node: merge results, determine escalation
[[graph.nodes]]
id = "aggregate"
cell = "roko.probe.aggregate"

# FanOut edges from distribute to all probes, FanIn to aggregate
```

### Probe aggregation

```rust
struct ProbeAggregateCell;

impl Cell for ProbeAggregateCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let results: Vec<ProbeResult> = input.iter()
            .filter_map(|s| ProbeResult::from_signal(s).ok())
            .collect();

        let max_surprise = results.iter()
            .map(|r| r.surprise)
            .fold(0.0_f32, f32::max);

        let max_stakes = results.iter()
            .map(|r| r.stakes)
            .fold(0.0_f32, f32::max);

        let any_escalate = results.iter().any(|r| r.escalate);

        let aggregate = T0ProbeResults {
            probe_count: results.len(),
            max_surprise,
            max_stakes,
            triggered: results.iter().filter(|r| r.escalate).count(),
            all_quiet: !any_escalate,
        };

        Ok(aggregate.into_signals())
    }
}
```

Total cost of the 16-probe FanOut: approximately 20-50 microseconds. This is the cost of ~80% of gamma ticks. The remaining ~20% escalate to T1/T2 at $0.001-$0.100 per call.

---

## 11. Gamma, Theta, Delta Loop Details

### 11.1 Gamma loop: the agent's heartbeat

The gamma loop fires every 100ms-500ms (base), adjusted by regime. Each tick runs:

1. **T0 probes** (16 Cells in parallel, ~50us)
2. **EFE evaluation** (if any probe triggered)
3. **T1 or T2 execution** (if EFE selects a non-T0 tier)
4. **CorticalState update** (atomic writes)

```rust
// Gamma loop per-tick pseudocode
async fn gamma_tick(agent: &Agent<Active>) -> Result<()> {
    // Step 1: Run T0 probes
    let probe_results = agent.gamma_graph
        .execute_subgraph("gamma-probe-fanout", agent.cortical_snapshot())
        .await?;

    // Step 2: If all quiet, short-circuit
    if probe_results.all_quiet() {
        agent.cortical.tick_count.fetch_add(1, Ordering::Relaxed);
        agent.bus.publish(Pulse::tick_completed(agent.id, "gamma", "t0")).await?;
        return Ok(());
    }

    // Step 3: EFE tier selection
    let tier = select_tier(
        &probe_results,
        &agent.cortical,
        agent.vitality.vitality(),
        agent.clock.regime,
    );

    // Step 4: Execute at selected tier
    match tier {
        CognitiveTier::T1Deliberate => {
            let result = agent.dispatch_t1(&probe_results).await?;
            agent.verify_and_persist(result).await?;
        }
        CognitiveTier::T2Reflective => {
            let result = agent.dispatch_t2(&probe_results).await?;
            agent.verify_and_persist(result).await?;
        }
        _ => unreachable!("T0 handled above"),
    }

    // Step 5: Update CorticalState
    agent.cortical.tick_count.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
```

### 11.2 Theta loop: reflection

The theta loop fires every 500ms-16s (base). It runs a full reflective cycle:

1. **SENSE**: Gather recent gamma tick summaries, Bus Pulses, and task status.
2. **ASSESS**: What has changed since the last theta tick? Is the current approach working?
3. **COMPOSE**: Assemble a reflective prompt with recent episodes and predictions.
4. **ACT**: T1 or T2 inference for summarization, replanning, or strategy adjustment.
5. **VERIFY**: Check that the replan is coherent and within budget.
6. **PERSIST/BROADCAST**: Write updated strategy; publish replan Pulses.
7. **REACT**: Update Daimon PAD, promote/demote Memory entries, adjust confidence.

```toml
[graph]
name = "cognitive-loop-theta"
version = "1.0.0"
hot = true
clock = { kind = "adaptive", timescale = "theta" }

[graph.policy]
max_parallelism = 1
failure_strategy = "retry_with_escalation"
snapshot_interval_secs = 600
budget_scope = "agent"

[[graph.nodes]]
id = "sense"
cell = "roko.cognitive.theta_sense"
execution_class = "workflow"
# Gathers: recent gamma tick summaries, Bus Pulses, task status

[[graph.nodes]]
id = "assess"
cell = "roko.cognitive.theta_assess"
execution_class = "workflow"
# Evaluates: is current approach working? Completion rate, gate pass trend

[[graph.nodes]]
id = "compose"
cell = "roko.cognitive.theta_compose"
execution_class = "workflow"
# Assembles: reflective prompt with recent episodes, predictions, strategy

[[graph.nodes]]
id = "act"
cell = "roko.cognitive.theta_act"
execution_class = "activity"
# T1 or T2 inference for summarization, replanning, strategy adjustment

[[graph.nodes]]
id = "verify"
cell = "roko.cognitive.theta_verify"
execution_class = "activity"
# Checks: replan is coherent and within budget

[[graph.nodes]]
id = "persist_broadcast"
cell = "roko.cognitive.theta_persist"
execution_class = "activity"
# Writes: updated strategy; publishes replan Pulses

[[graph.nodes]]
id = "react"
cell = "roko.cognitive.theta_react"
execution_class = "workflow"
# Updates: Daimon PAD, Memory promotion/demotion, confidence adjustment

# Sequential edges
[[graph.edges]]
from = "sense"
to = "assess"

[[graph.edges]]
from = "assess"
to = "compose"

[[graph.edges]]
from = "compose"
to = "act"

[[graph.edges]]
from = "act"
to = "verify"

[[graph.edges]]
from = "verify"
to = "persist_broadcast"

[[graph.edges]]
from = "persist_broadcast"
to = "react"

# Feedback edge
[[graph.edges]]
from = "react"
to = "sense"
kind = "feedback"
```

### 11.3 Delta loop: consolidation

The delta loop fires during idle periods or on a scheduled basis. It runs the Dream consolidation cycle as a sub-Graph:

```
NREM Replay
  -> Replay recent episodes, weighted by prediction error
  -> Extract patterns into candidate heuristics

REM Imagination
  -> HDC recombination: combine knowledge vectors from different domains
  -> Counterfactual generation: "what if?" questions about past episodes
  -> Emotional depotentiation: reduce affective charge of negative experiences

Integration Staging
  -> Validate dream outputs against existing Memory
  -> Promote to Memory if confidence exceeds threshold
  -> Emit consolidation Pulses for other agents
```

The delta loop uses T2 exclusively -- deep reasoning for synthesis and cross-domain insight. See [06-MEMORY](06-MEMORY.md) SS9 for the full 4-phase dream cycle.

---

## 12. T0 Reflex Store

T0 is the zero-cost tier: pure Rust pattern matching, no LLM call. Reflexes are `condition -> action` rules promoted from T2 successes.

```rust
pub struct ReflexStore {
    pub rules: Vec<ReflexRule>,
    pub max_rules: usize,               // default: 200
}

pub struct ReflexRule {
    pub condition: Predicate,
    pub action: Action,
    pub promoted_from: SignalRef,        // episode where pattern was learned
    pub success_count: u64,
    pub failure_count: u64,
    pub last_used: DateTime<Utc>,
}
```

**Promotion**: When a T2 action pattern succeeds 5+ times with >90% gate pass rate, L1 parameter tuning can promote it to a T0 reflex rule.

**Demotion**: When a reflex rule fails a gate, it is demoted -- removed from the T0 store and the tick falls through to T1/T2 evaluation. Demoted rules are logged as negative calibration receipts.

**Cap**: Maximum 200 rules. When full, the least-recently-used rule with the lowest success rate is evicted.

---

## 13. Somatic Markers and PAD Affect

Somatic markers (Damasio 1994) encode the Agent's affective state as a PAD model (Pleasure/Arousal/Dominance), modulated by prospect theory (Tversky & Kahneman 1992) with loss aversion parameter lambda=2.25. Somatic markers influence decision-making by biasing risk tolerance, context allocation, and exploration/exploitation balance.

### PAD affect model

```rust
pub struct PADState {
    pub pleasure: f64,       // -1.0..=1.0 (negative = displeasure)
    pub arousal: f64,        // -1.0..=1.0 (negative = calm, positive = alert)
    pub dominance: f64,      // -1.0..=1.0 (negative = submissive, positive = in-control)
}
```

PAD values are derived from recent experience: gate passes increase pleasure, unexpected failures increase arousal and decrease dominance, budget pressure decreases dominance.

### Prospect theory integration

Kahneman-Tversky prospect theory (1992) models loss aversion: losses hurt more than equivalent gains feel good. The lambda=2.25 asymmetry means a $1 loss has 2.25x the psychological impact of a $1 gain.

```rust
pub fn prospect_value(outcome: f64, reference: f64) -> f64 {
    let delta = outcome - reference;
    if delta >= 0.0 {
        delta.powf(0.88)                     // diminishing sensitivity to gains
    } else {
        -2.25 * (-delta).powf(0.88)         // loss aversion: lambda = 2.25
    }
}
```

This biases Conservation and Declining phase agents toward safe, known strategies -- the prospect of further loss outweighs potential gain from exploration.

### k-d tree spatial index

Somatic markers from past episodes are stored with their PAD coordinates in a k-d tree for sub-100 microsecond retrieval. When the agent faces a decision, it retrieves the K nearest somatic markers in PAD space to recall how similar situations felt.

```rust
pub struct SomaticMarkerStore {
    pub index: KdTree<f64, SignalRef, 3>,  // 3D: pleasure, arousal, dominance
    pub markers: Vec<SomaticMarker>,
}

pub struct SomaticMarker {
    pub pad: PADState,
    pub episode_ref: SignalRef,
    pub outcome: f64,              // continuous reward from Verify
    pub context_hash: ContentHash, // HDC fingerprint of the decision context
}
```

### 15% mandatory contrarian retrieval

To prevent affective lock-in -- where the agent only retrieves markers confirming its current emotional state -- 15% of somatic marker retrievals are mandatory contrarian: they retrieve markers from the opposite PAD quadrant. If the agent is in a negative-pleasure state, 15% of retrieved markers come from positive-pleasure episodes. This breaks echo chambers in affective decision-making.

### Six behavioral states

| State | PAD Region | Risk Tolerance | Context Allocation | Exploration |
|---|---|---|---|---|
| **Confident** | +P, -A, +D | High | Broad (try new approaches) | High |
| **Cautious** | -P, +A, -D | Low | Narrow (stick to known) | Low |
| **Curious** | +P, +A, +D | Medium-high | Broad (seek novelty) | Very high |
| **Anxious** | -P, +A, -D | Very low | Very narrow | Minimal |
| **Bored** | -P, -A, +D | Medium | Broad (seek stimulation) | High |
| **Focused** | +P, -A, +D | Medium | Narrow (deep on current) | Low |

### Affect-modulated routing

The PAD vector modulates EFE computation:

```rust
fn modulated_efe(base_efe: f64, pad: &PadVector, regime: Regime) -> f64 {
    let risk_aversion = 1.0 + (1.0 - pad.dominance).max(0.0) * 0.5;
    let urgency_boost = pad.arousal.max(0.0) * 0.3;
    let exploration_damping = if pad.pleasure < -0.3 { 0.5 } else { 1.0 };

    let pragmatic = base_efe.pragmatic * risk_aversion;
    let epistemic = base_efe.epistemic * exploration_damping;
    let cost = base_efe.cost * (1.0 + urgency_boost);

    -pragmatic - epistemic + cost
}
```

Low dominance -> more risk-averse (prefer known routes).
Low pleasure -> dampen exploration (stick with what works).
High arousal -> amplify cost sensitivity (urgency increases).

### Temperament: PAD modulation of timescales

The Daimon's PAD vector modulates how the three timescales interact. This creates emergent "temperament" without explicit personality programming.

| PAD State | Gamma Effect | Theta Effect | Delta Effect |
|---|---|---|---|
| High confidence + low arousal | Higher T0/T1 threshold (coast longer) | Longer theta interval (fewer reflections) | Normal delta schedule |
| Low confidence + high arousal | Lower escalation threshold | Shorter theta interval (0.66x) | Delta deferred (active work takes priority) |
| High dominance | More willing to act on T1 without T2 | Confident replans at theta speed | Normal |
| Low dominance | Requires T2 for significant actions | Cautious replans; may escalate to T2 | Normal |
| Low arousal + neutral pleasure | Extended gamma; T0-heavy | Extended theta interval | Triggers delta (consolidate during boredom) |

The PAD vector is not a dial that an operator sets. It is computed from recent outcomes: gate passes increase pleasure, unexpected failures increase arousal and decrease dominance. The behavior emerges from the feedback between outcomes and timescale modulation.

---

## 14. Cognitive Energy

Every Agent maintains a cognitive energy pool alongside its budget. Budget tracks money (tokens x price); energy tracks computational capacity (ability to sustain complex reasoning). They deplete independently.

```rust
pub struct CognitiveEnergy {
    pub current: f64,          // 0.0..=1.0
    pub max: f64,              // 1.0 (calibrated per agent)
    pub depletion_rate: f64,   // per-operation cost varies by tier
    pub recovery_mode: RecoveryMode,
    pub fatigue_penalty: f64,  // accumulates, reduces effective capacity
}

pub enum RecoveryMode {
    Gamma,   // micro-recovery between turns (~5% restoration)
    Theta,   // medium recovery during idle (~20% restoration)
    Delta,   // full recovery during dream cycle (~80% restoration)
}
```

### Depletion functions

| Operation | Energy Cost | Rationale |
|---|---|---|
| T0 reflex | 0.01 | Near-zero: pattern match is cheap |
| T1 fast inference | 0.05 | Moderate: small model invocation |
| T2 deep inference | 0.15 | High: full reasoning chain |
| Tool execution | 0.03 | I/O bound, not compute bound |
| Context assembly (VCG) | 0.02 | One-time per turn |
| Gate verification | 0.04 | Verification is computationally lighter than generation |
| Dream consolidation | -0.30 | Net positive: recovery during consolidation |

### Fatigue accumulation

Sustained high-tier usage causes fatigue that does not recover with micro-pauses:

```
fatigue(t+1) = fatigue(t) * decay + cost(operation) * intensity_factor
effective_energy = current_energy - fatigue_penalty
```

Where `decay = 0.95` (slow bleed) and `intensity_factor = 1.0` for T0, `2.0` for T1, `4.0` for T2. Only Delta recovery (dream cycle) resets fatigue to zero.

### Five energy zones

Energy zones constrain what the agent can do, independent of budget:

| Zone | Energy Range | Tier Access | Max Active Goals | Strategy |
|---|---|---|---|---|
| **Peak** | 0.8-1.0 | T0/T1/T2 | Unlimited | Explore, take risks |
| **Normal** | 0.5-0.8 | T0/T1/T2 | 5 | Balanced |
| **Conserving** | 0.3-0.5 | T0/T1 only | 3 | Prioritize, defer T2 |
| **LowPower** | 0.1-0.3 | T0 only | 1 | Exploit known patterns |
| **Critical** | <0.1 | T0 only | 0 | Flush state, request dream |

### Energy-vitality coupling

Energy zones interact with vitality (budget) phases multiplicatively:

```rust
fn effective_tier_access(energy: &CognitiveEnergy, vitality: &Vitality) -> TierAccess {
    let energy_zone = energy.zone();
    let budget_phase = vitality.phase();

    match (energy_zone, budget_phase) {
        (Peak, Thriving) => TierAccess::Full,           // everything available
        (_, Terminal) => TierAccess::None,               // budget gone = stop
        (Critical, _) => TierAccess::T0Only,             // too tired for LLM
        (LowPower, Conservation) => TierAccess::T0Only,  // both constrained
        (Conserving, Declining) => TierAccess::T0T1,     // belt and suspenders
        _ => TierAccess::from_energy(energy_zone),       // energy dominates
    }
}
```

### Bidirectional energy-affect coupling

Energy and SomaticState (PAD vector) influence each other. This is a **Loop pattern** where energy changes affect, and affect changes energy dynamics.

#### Energy -> Affect

| Energy State | Affect Shift | Mechanism |
|---|---|---|
| Low energy | Pleasure down, Dominance down | Agent "feels" depleted, less confident |
| Critical energy | Arousal up | Urgency signal, needs recovery |
| Post-recovery | Pleasure up, Dominance up | Restored capacity = restored confidence |
| Fatigue spike | Arousal down | Numbing effect of sustained high load |

#### Affect -> Energy

| Affect State | Energy Effect | Mechanism |
|---|---|---|
| High arousal | Burn rate x1.3 | Stress accelerates depletion |
| Low pleasure (sustained) | Recovery rate x0.7 | Depression impairs recovery |
| High dominance | Fatigue resistance x1.2 | Confidence sustains effort |
| Success streak | Micro-recovery boost x1.5 | Momentum aids restoration |

```rust
fn energy_tick(energy: &mut CognitiveEnergy, pad: &PadVector, dt: Duration) {
    // Affect modulates burn rate
    let arousal_factor = 1.0 + pad.arousal.max(0.0) * 0.3;
    energy.current -= energy.depletion_rate * dt.as_secs_f64() * arousal_factor;

    // Affect modulates recovery
    let recovery_factor = if pad.pleasure < -0.3 { 0.7 } else { 1.0 };
    let dominance_factor = 1.0 + pad.dominance.max(0.0) * 0.2;
    energy.current += energy.recovery_rate() * dt.as_secs_f64()
        * recovery_factor * dominance_factor;

    energy.current = energy.current.clamp(0.0, energy.max);
}
```

### Three recovery modes

| Mode | Trigger | Duration | Restores | Resets Fatigue? |
|---|---|---|---|---|
| Gamma | Between turns | ~5s | 5% energy | No |
| Theta | Idle period > 60s | ~75s | 20% energy | Partially (50%) |
| Delta | Dream cycle | Minutes-hours | 80% energy | Yes (full reset) |

Delta recovery is the only way to fully clear fatigue. This creates natural pressure for dream cycles -- agents that skip dreams accumulate fatigue until they can only run T0 reflexes.

### Energy-affect Loop

```rust
pub struct EnergyAffectLoop {
    // Energy -> Affect
    energy_to_pleasure: f64,        // 0.3
    energy_to_dominance: f64,       // 0.2
    critical_energy_arousal: f64,   // 0.4

    // Affect -> Energy
    pleasure_cost_discount: f64,    // 0.15
    arousal_cost_premium: f64,      // 0.1
    dominance_recovery_bonus: f64,  // 0.2
}

impl EnergyAffectLoop {
    /// Energy -> Affect: compute PAD delta from energy state.
    pub fn energy_to_pad(&self, energy: &CognitiveEnergy) -> PadDelta {
        let fraction = energy.current / energy.max_energy.max(f64::EPSILON);

        let pleasure = if fraction < 0.3 {
            -self.energy_to_pleasure * (1.0 - fraction / 0.3)
        } else { 0.0 };

        let dominance = if fraction < 0.4 {
            -self.energy_to_dominance * (1.0 - fraction / 0.4)
        } else { 0.0 };

        let arousal = if fraction < 0.15 {
            self.critical_energy_arousal * (1.0 - fraction / 0.15)
        } else { 0.0 };

        PadDelta { pleasure, arousal, dominance }
    }

    /// Affect -> Energy: modulate energy cost.
    pub fn affect_cost_modifier(&self, pad: &PadVector) -> f64 {
        let pleasure_mod = -self.pleasure_cost_discount * pad.pleasure.clamp(-1.0, 1.0);
        let arousal_mod = self.arousal_cost_premium * pad.arousal.clamp(0.0, 1.0);
        1.0 + pleasure_mod + arousal_mod
    }

    /// Affect -> Energy: modulate recovery rate.
    pub fn affect_recovery_modifier(&self, pad: &PadVector) -> f64 {
        let dominance_mod = self.dominance_recovery_bonus * pad.dominance.clamp(0.0, 1.0);
        1.0 + dominance_mod
    }
}
```

The Loop creates emergent behavior:
- **Virtuous cycle**: agent succeeds -> pleasure rises -> energy cost decreases -> agent can do more -> more successes.
- **Protective cycle**: agent fails repeatedly -> pleasure drops -> energy cost increases -> agent slows down -> forced into Theta/Delta -> consolidation and recovery -> return with better strategy.
- **Stress response**: energy drops to critical -> arousal spikes -> triggers Delta consolidation -> deep recovery.

---

## 15. Emergent Goals

Goals emerge from the intersection of what the agent wants (affect), what it knows (Memory), and what it has done (episodes). Goal emergence is a Cell that runs during Theta reflections.

### Goal emergence Cell

```rust
pub struct GoalEmergenceCell {
    /// Active pattern detectors.
    detectors: Vec<Box<dyn GoalDetector>>,
    /// Minimum intrinsic motivation for a Nascent goal to survive.
    nascent_threshold: f64,    // default: 0.3
    /// Reinforcements required for Nascent -> Candidate promotion.
    reinforcement_threshold: u32,  // default: 3
    /// Maximum concurrent Active goals.
    max_active_goals: usize,   // default: 5
    /// HDC similarity threshold for merging duplicate goals.
    merge_threshold: f64,      // default: 0.85
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergentGoal {
    pub id: GoalId,
    pub description: String,
    pub sources: GoalSources,
    pub state: GoalState,
    pub priority: f64,
    pub intrinsic_motivation: f64,
    /// Expected free energy reduction if achieved.
    pub expected_efe_reduction: f64,
    /// Estimated energy cost to achieve.
    pub estimated_energy_cost: f64,
    pub sub_goals: Vec<GoalId>,
    pub parent: Option<GoalId>,
    pub created_at: SystemTime,
    pub last_evaluated: SystemTime,
    pub reinforcement_count: u32,
}

/// Goal lifecycle states.
///
///  Nascent -> Candidate -> Active -> { Achieved | Abandoned | Merged }
///               ^                          |
///               +--- (re-emerge) ----------+
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalState {
    Nascent,
    Candidate,
    Active,
    Achieved,
    Abandoned,
    Merged { into: GoalId },
}
```

### Five built-in detectors

| Detector | Pattern | Goal |
|---|---|---|
| **KnowledgeGapDetector** | Recent queries returned 0 results for a topic referenced in multiple tasks | "Research [topic] to fill knowledge gap" |
| **QualityDegradationDetector** | Verify pass rate trending downward over recent episodes | "Investigate and fix declining [gate] pass rate" |
| **CuriosityDetector** | Agent has high arousal (Daimon) AND there is knowledge with high novelty that has not been explored | "Investigate [novel topic] -- high potential utility" |
| **SelfMaintenanceDetector** | Consolidated-tier knowledge items unvalidated for > N hours | "Re-validate stale knowledge about [topic]" |
| **FrustrationRecoveryDetector** | Low pleasure (Daimon) AND 3+ consecutive Verify failures on similar tasks | "Change approach for [task pattern] -- current strategy failing" |

### Intrinsic motivation (Score protocol Cell)

Intrinsic motivation scores candidate goals by how "interesting" they are. Combines three factors (Schmidhuber 2010, Colas et al. 2022):

1. **Learning progress**: estimated knowledge gain (information-theoretic).
2. **Competence match**: how well agent skills match goal difficulty (zone of proximal development).
3. **Affect alignment**: how well the goal aligns with current Daimon state.

```
IM = alpha * learning_progress * zpd_score
   + beta  * competence_score
   + gamma * affect_alignment
```

Where `alpha = 0.4`, `beta = 0.35`, `gamma = 0.25`.

### Zone of proximal development (ZPD)

Goals are most motivating when they sit at the competence boundary. The ZPD score (Vygotsky 1978, operationalized by Colas et al. 2022) peaks when the goal is challenging enough to learn from but achievable enough to avoid frustration:

```
zpd_score = exp(-competence_gap^2 / 0.5)
```

Where `competence_gap = |difficulty - competence|`, `difficulty = estimated_energy_cost / max_energy`, and `competence = historical_success_rate`.

```
Motivation(goal)
      |
  1.0 |         /\
      |        /  \
  0.5 |       /    \
      |      /      \
  0.0 |-----/--------\------
      +-----------------------  Difficulty
            |    |    |
          Easy   ZPD   Hard
        (boring)     (frustrating)
```

### Goal selection via EFE (Route protocol)

Goal selection ranks candidates by expected free energy reduction, gated by energy feasibility:

```rust
pub struct GoalSelectionRouteCell;

impl Cell for GoalSelectionRouteCell {
    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let candidates = extract_candidate_goals(&input)?;
        let cortical = ctx.cortical.as_ref().ok_or(CellError::Internal(
            anyhow::anyhow!("GoalSelectionRouteCell requires CorticalState"),
        ))?;
        let energy = cortical.energy();
        let vitality = cortical.vitality.load(Ordering::Relaxed);

        // Filter by energy feasibility: only goals costing < 80% of current energy
        let feasible: Vec<_> = candidates.iter()
            .filter(|g| g.estimated_energy_cost < energy * 0.8)
            .collect();

        // Rank by EFE reduction:
        //   EFE(goal) = -epistemic_value(goal) - pragmatic_value(goal)
        //               + energy_cost_penalty(goal)
        //
        // epistemic_value = learning_progress * zpd_score  (from intrinsic motivation)
        // pragmatic_value = expected_efe_reduction          (from goal struct)
        // energy_cost_penalty = estimated_energy_cost / energy * vitality_cost_multiplier
        let mut ranked: Vec<_> = feasible.iter().map(|g| {
            let epistemic = g.intrinsic_motivation;
            let pragmatic = g.expected_efe_reduction;
            let cost_penalty = (g.estimated_energy_cost / energy.max(f64::EPSILON))
                * vitality_cost_multiplier(vitality);
            let efe = -epistemic - pragmatic + cost_penalty;
            (*g, efe)
        }).collect();
        ranked.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));

        // Select top N where N = zone.max_active_goals()
        let max_goals = energy_zone(energy).max_active_goals();
        let selected: Vec<_> = ranked.iter().take(max_goals).map(|(g, _)| *g).collect();

        Ok(selected.into_signals())
    }
}
```

### File-set overlap detection

Two goals conflict when their file sets overlap AND their success conditions are incompatible (e.g., one requires adding a field, the other requires removing it). File-set overlap is detected by intersecting the `touched_files` globs from each goal's task decomposition:

```rust
fn goals_conflict(a: &EmergentGoal, b: &EmergentGoal) -> bool {
    let a_files: HashSet<_> = a.touched_files().collect();
    let b_files: HashSet<_> = b.touched_files().collect();
    let overlap = a_files.intersection(&b_files).count();
    // Conflict if >30% of either goal's files overlap
    let a_ratio = overlap as f64 / a_files.len().max(1) as f64;
    let b_ratio = overlap as f64 / b_files.len().max(1) as f64;
    a_ratio > 0.3 || b_ratio > 0.3
}
```

### Goal conflict arbitration

When two active goals conflict (overlapping file sets with incompatible success conditions), arbitration resolves by: (1) compare EFE scores, (2) if within 10%: cheaper wins, (3) if tied: older goal wins (more reinforcement = more validated). Losing goals with < 2 reinforcements are abandoned; others are deferred.

### Capacity growth

Over time, successful work and effective consolidation increase the agent's energy capacity. Growth is logarithmic; disuse decay prevents unbounded growth -- use it or lose it.

```rust
pub struct EnergyCapacityModel {
    base_capacity: f64,       // 100.0
    growth_per_delta: f64,    // 0.1
    growth_per_task: f64,     // 0.02
    capacity_ceiling: f64,    // 200.0
    disuse_decay: f64,        // 0.001 per hour
}
```

---

## 16. CognitiveWorkspace (Compose Protocol)

The CognitiveWorkspace assembles context for LLM inference using a VCG auction (Vickrey-Clarke-Groves) with 8+ bidders competing for limited token budget. Section effect tracking via beta-distribution posteriors learns which context sections correlate with gate success.

### VCG auction

```rust
pub struct CognitiveWorkspace {
    pub bidders: Vec<Box<dyn AttentionBidder>>,
    pub token_budget: usize,
    pub section_effects: BTreeMap<String, BetaDistribution>,
}

pub trait AttentionBidder: Send + Sync {
    fn name(&self) -> &str;
    fn bid(&self, context: &TaskContext) -> Vec<ContextBid>;
}

pub struct ContextBid {
    pub section_name: String,
    pub content: String,
    pub token_count: usize,
    pub value: f64,               // bidder's valuation
}
```

### 8+ built-in bidders

| Bidder | What it bids | Value signal |
|---|---|---|
| **NeuroBidder** | Knowledge Signals from Memory store | HDC similarity to task + demurrage balance |
| **TaskBidder** | Task description, dependencies, constraints | Always bids; baseline context |
| **ResearchBidder** | Research artifacts relevant to task | Recency + citation count |
| **HeuristicBidder** | Matched heuristics with calibration scores | Calibration score (Brier) |
| **EpisodeBidder** | Recent relevant episodes | HDC similarity + recency |
| **PheromoneBidder** | Pheromone Pulse summaries for task context | Pheromone intensity |
| **AffectBidder** | Somatic marker summaries | PAD distance to current state |
| **SystemBidder** | System prompt, spec sections, domain profile | Always bids; infrastructure |

### Section effect tracking

Every context section is tracked by a beta-distribution posterior:

```rust
pub struct BetaDistribution {
    pub alpha: f64,     // successes (section present AND gate passed)
    pub beta: f64,      // failures (section present AND gate failed)
}

impl BetaDistribution {
    pub fn mean(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }
}
```

After each gate evaluation, the workspace updates beta distributions for all included sections. Sections with high `mean()` are boosted in future auctions; sections with low `mean()` are penalized.

### Novelty attenuation

```
novelty_weight = 1 / (1 + ln(freq))
```

Where `freq` is the number of times this section has appeared in context. Habituation that never reaches zero -- even a very common section retains a nonzero novelty floor. Prevents prompt space from being dominated by familiar but uninformative sections.

---

## 17. Domain Profiles

Domain profiles are complete cognitive postures -- not just extension lists. A profile configures clock rates, extension chains, Bus subscriptions, context weights, gate pipelines, and infrastructure preferences.

```rust
pub struct DomainProfile {
    pub name: String,

    // Clock
    pub gamma_base: Duration,
    pub theta_base: Duration,
    pub delta_base: Duration,

    // Extensions
    pub extensions: Vec<ExtensionRef>,
    pub disable_extensions: Vec<String>,

    // Bus subscriptions
    pub subscriptions: Vec<TopicFilter>,

    // Context weights
    pub bidder_weights: BTreeMap<String, f64>,

    // Gate pipeline
    pub gate_rungs: Vec<GateRungConfig>,

    // Infrastructure
    pub preferred_models: Vec<String>,
    pub max_budget: Cost,
    pub max_slots: usize,
}
```

### Built-in profiles

| Profile | Clock | Extensions | Gates | Models |
|---|---|---|---|---|
| **coding** | Gamma 200ms, Theta 8s | git, compiler, test-runner, safety | compile, test, clippy, diff | Opus for T2, Sonnet for T1 |
| **research** | Gamma 500ms, Theta 16s | web-search, citation, summarizer, safety | source-quality, coherence | Opus for T2, Sonnet for T1 |
| **trading** | Gamma 100ms, Theta 2s | chain-reader, risk-manager, safety | risk-check, compliance | Sonnet for T1, Haiku for T0 |
| **devops** | Gamma 300ms, Theta 10s | git, compiler, deploy-checker, safety | health-check, rollback-gate | Sonnet for T1 |
| **general** | Gamma 300ms, Theta 10s | safety, cost-tracker | basic-quality | Sonnet for T1 |

---

## 18. TOML Configuration

```toml
[[agents]]
name = "code-agent"
profile = "coding"
mode = "persistent"

[agents.budget]
initial = 10.0                     # USD
warn_threshold = 0.30
sleepwalk_threshold = 0.05

[agents.clock]
gamma_ms = 200
theta_ms = 8000
delta_ms = 300000
hysteresis_ticks = 3

[agents.slots]
max = 3

[agents.extensions]
chain = [
  { name = "git", optional = false },
  { name = "compiler", optional = false },
  { name = "test-runner", optional = false },
]

[agents.memory]
store_path = ".roko/neuro/knowledge.jsonl"
max_entries = 50000
dream_idle_timeout_mins = 5

[agents.compose]
token_budget = 16000
bidder_weights = { neuro = 1.0, task = 1.0, heuristic = 0.8, episode = 0.6 }

[agents.reflexes]
max_rules = 200
promote_threshold = 5
demote_on_failure = true
```

---

## 19. Feedback Loops

| Loop | Timescale | What it observes | What it adjusts |
|---|---|---|---|
| **Probe calibration** | Gamma | Probe false-positive rate (escalation led to T0-level work) | Probe thresholds |
| **EFE adaptation** | Gamma | Prediction error vs tier used | EFE cost and value weights |
| **Theta cadence** | Theta | Completion rate, PAD state | Theta interval (0.5x - 2.0x) |
| **Reflex promotion** | Theta | T2 pattern success rate | T0 reflex store (promote at 5+ successes, >90% pass) |
| **Dream prioritization** | Delta | Prediction error magnitudes from episodes | NREM replay ordering |
| **Regime transitions** | Cross-timescale | PE trend over 3+ ticks | Adaptive clock multipliers |
| **Energy-affect Loop** | Continuous | Energy level vs PAD state | Bidirectional modulation (section 14) |
| **Goal evaluation** | Theta | IM scores, EFE reduction estimates | Goal lifecycle transitions |
| **Capacity growth** | Delta | Successful tasks, effective consolidations | `max_energy` parameter |

---

## 20. Cognitive Architecture References

The three-tier model draws from:

- **ACT-R** (Anderson 2007): Declarative/procedural memory -> Store/Heuristic duality.
- **SOAR** (Laird 2012): Impasse-driven elaboration -> T0 failure triggers T1 escalation.
- **CLARION** (Sun 2002): Implicit/explicit processing -> T0 reflex (implicit) vs T2 reasoning (explicit).
- **Global Workspace Theory** (Baars 1988): Broadcast on Bus = global workspace; CognitiveWorkspace VCG = competitive access.
- **Active Inference** (Friston 2006): Agents minimize prediction error across ALL levels simultaneously. Each tier does not just solve the task -- it reduces the system's overall free energy (uncertainty). This is why EFE works as a unified routing metric.
- **Prospect Theory** (Tversky & Kahneman 1992): Loss aversion with lambda=2.25 shapes risk behavior under resource pressure.
- **Somatic Marker Hypothesis** (Damasio 1994): Emotional fingerprints of past decisions enable sub-millisecond gut-feeling evaluation.
- **Zone of Proximal Development** (Vygotsky 1978; operationalized by Colas et al. 2022): Peak motivation at the competence boundary.
- **Intrinsic Motivation** (Schmidhuber 2010): Learning progress as a reward signal.
- **Stigmergy** (Grasse 1959; Dorigo 1992): Indirect coordination via environment modification.
- **Mortality as motivation** (Jonas 1966, *The Phenomenon of Life*): Resource pressure drives prioritization.

---

## 21. Acceptance Criteria

| # | Criterion | Verification |
|---|---|---|
| AG-1 | `Agent<Provisioning>` cannot call `tick()` (compile error) | Compile check: method does not exist on type |
| AG-2 | `Agent<Active>` can call `tick()`, cannot call `load_extension()` | Compile check |
| AG-3 | `Agent<Dreaming>` runs dream cycle but cannot execute tool calls | Compile check + integration test |
| AG-4 | `Agent<Terminal>` flushes episodes and exports knowledge | Integration test |
| AG-5 | Vitality phases transition at correct thresholds (0.7, 0.4, 0.2, 0.05) | Unit test |
| AG-6 | Conservation phase reduces compose budget to 60% | Integration test: verify token budget |
| AG-7 | Terminal phase triggers sleepwalk (no LLM calls) | Integration test |
| AG-8 | CorticalState reads complete in < 1 microsecond | Benchmark |
| AG-9 | CorticalState single-writer/multi-reader: no data races | `cargo miri test` or equivalent |
| AG-10 | Multi-slot: 3 slots share budget, phase change visible to all | Integration test |
| AG-11 | Slot capability guards enforce fail-closed intersection | Unit test: attempt blocked capability |
| AG-12 | Adaptive clock: 3-tick hysteresis prevents oscillation | Unit test: single PE spike does not trigger regime change |
| AG-13 | Regime Calm slows gamma by 4x | Unit test: compare tick intervals |
| AG-14 | Regime Crisis speeds gamma by 4x (0.25x multiplier) | Unit test |
| AG-15 | EFE selects T0 for pattern-matchable tasks (~80% of ticks) | Integration test with synthetic workload |
| AG-16 | EFE regime conditioning: Crisis biases toward epistemic value | Unit test |
| AG-17 | T0 reflex promoted after 5+ T2 successes at >90% pass rate | Integration test |
| AG-18 | T0 reflex demoted on gate failure | Unit test |
| AG-19 | Reflex store capped at 200 rules with LRU eviction | Unit test |
| AG-20 | Somatic marker retrieval < 100 microseconds via k-d tree | Benchmark |
| AG-21 | 15% mandatory contrarian retrieval from opposite PAD quadrant | Unit test: verify contrarian fraction |
| AG-22 | Prospect theory lambda=2.25 loss aversion modulates risk | Unit test: verify asymmetric valuation |
| AG-23 | VCG auction selects highest-value sections within token budget | Unit test |
| AG-24 | Section effect beta distributions update on gate pass/fail | Integration test |
| AG-25 | Novelty attenuation: freq=10 yields ~0.30 weight | Unit test |
| AG-26 | Pipeline fires as Hot Graph via execution Engine | Integration test: verify tick-driven re-fire |
| AG-27 | Domain profile loads correct extensions, clock rates, gate rungs | Integration test |
| AG-28 | `AgentStateTransition` Pulse emitted on every lifecycle change | Integration test |
| AG-29 | `AgentPhaseChange` Pulse emitted on vitality phase boundary | Integration test |
| AG-30 | `AgentRegimeChange` Pulse emitted on regime transition | Integration test |
| AG-31 | Energy zones constrain tier access correctly | Unit test: Peak allows T2, LowPower T0-only |
| AG-32 | Fatigue accumulates during sustained T2 usage | Integration test: track fatigue over 20 T2 ticks |
| AG-33 | Delta recovery resets fatigue to zero | Integration test: verify post-dream fatigue |
| AG-34 | Energy-affect coupling: low energy reduces pleasure | Unit test: verify PAD delta |
| AG-35 | Energy-affect coupling: high arousal increases burn rate | Unit test: verify energy depletion increase |
| AG-36 | Goal emergence: knowledge gap detector produces Nascent goals | Integration test |
| AG-37 | Intrinsic motivation ZPD score peaks at competence boundary | Unit test: verify Gaussian curve |
| AG-38 | Goal conflict arbitration resolves by EFE -> cost -> age | Unit test |
| AG-39 | Capacity growth: max_energy increases with successful tasks | Unit test |
| AG-40 | 16 T0 probes execute in < 50 microseconds total | Benchmark |

---

## 22. Cross-References

| Topic | Document | Section |
|---|---|---|
| Signal/Pulse duality, demurrage | [01-SIGNAL](01-SIGNAL.md) | -- |
| 9 protocols, predict-publish-correct | [02-CELL](02-CELL.md) | -- |
| Verify redesign (continuous reward) | [02-CELL](02-CELL.md) | -- |
| EFE Route protocol | [02-CELL](02-CELL.md) | -- |
| Graph, Hot Graph, FanOut | [03-GRAPH](03-GRAPH.md) | -- |
| Engine, Flow, snapshot/resume | [04-EXECUTION](04-EXECUTION.md) | -- |
| Memory, demurrage, heuristics, dreams | [06-MEMORY](06-MEMORY.md) | -- |
| Extension system (8 layers, 22 hooks) | [12-EXTENSIONS](12-EXTENSIONS.md) | -- |
| Learning loops (L1-L4) | [07-LEARNING](07-LEARNING.md) | -- |
| CaMeL IFC | [16-SECURITY](16-SECURITY.md) | -- |
| Telemetry: StateHub agent_vitality projection | [15-TELEMETRY](15-TELEMETRY.md) | -- |
| Domain profiles (full spec) | [19-CONFIG](19-CONFIG.md) | -- |
| Surfaces | [20-SURFACES](20-SURFACES.md) | -- |
