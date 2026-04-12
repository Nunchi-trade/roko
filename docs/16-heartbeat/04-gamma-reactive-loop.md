# Gamma: The Reactive Loop (~5-15s)

> The fastest cognitive frequency — perception, gating, and action at the speed of environmental change.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md), [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md)
**Key sources**: `refactoring-prd/02-five-layers.md` §Adaptive Clock, legacy `bardo-backup/prd/01-golem/02-heartbeat.md` §S3, `bardo-backup/prd/01-golem/18-cortical-state.md` §Adaptive Clock, `implementation-plans/12a-cognitive-layer.md` §I1

---

## Abstract

Gamma is the heartbeat. Every 5-15 seconds, a Roko agent executes one complete pass through the universal Synapse loop — the 9-step PERCEIVE → EVALUATE → ATTEND → INTEGRATE → ACT → VERIFY → PERSIST → ADAPT → META-COGNIZE pipeline described in [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md). Gamma is where things actually happen: probes fire, observations are scored, the tier gate decides whether to invoke an LLM, and actions are taken if warranted.

The name "Gamma" comes from EEG frequency bands. Biological gamma oscillations (30-100 Hz) are associated with sensory processing, attention binding, and active perception (Buzsáki 2006, "Rhythms of the Brain", Oxford University Press). Roko's gamma loop serves the same function at a longer timescale: it is the agent's fast reactive perception, the constant sweep of its environment that keeps it aware of what is happening right now.

The critical property of gamma is that **~80% of gamma ticks cost nothing**. The 16 T0 probes (see [09-16-t0-probes.md](./09-16-t0-probes.md)) run as pure functions with zero LLM cost. Only when probes detect an anomaly — a prediction error exceeding the adaptive threshold — does the tick escalate to T1 or T2 and invoke an LLM. This makes high-frequency perception economically viable. An agent ticking at 10-second intervals executes ~8,640 ticks per day. At $0.10 per tick (T2 cost), that would be $864/day — prohibitive. With 80% T0 suppression, the effective daily cost drops to $2-50 depending on domain volatility.

This document specifies the gamma loop in detail: its internal structure, adaptive interval computation, relationship to the existing `roko-cli/src/orchestrate.rs` orchestration loop, the CorticalState perception surface it writes to, and the DecisionCycleRecord it produces.

---

## The Gamma Tick: Full 9-Step Execution

Each gamma tick runs the complete Synapse loop. The steps are not all equally expensive — the first four (PERCEIVE through ATTEND) always execute, while steps 5-8 (INTEGRATE through PERSIST) are conditional on the tier gate decision.

### Step 1: PERCEIVE — `Substrate.query()`

The tick begins by reading the environment. In Roko's Synapse Architecture, this maps to `Substrate.query()` at L0 Runtime. The Substrate provides raw access to stored Engrams and external state.

What PERCEIVE does on each gamma tick:

1. **Run T0 probes**: All 16 deterministic probes execute as pure functions (see [09-16-t0-probes.md](./09-16-t0-probes.md)). Each probe returns a scalar in [0.0, 1.0]. No LLM, no network call for domain-agnostic probes. Domain probes may make lightweight reads (RPC calls for chain, filesystem stats for coding).
2. **Fetch current observations**: Read domain-specific state — market prices for chain agents, build status for coding agents, research corpus updates for research agents.
3. **Detect regime changes**: Compare current observation to the previous tick's state. Has the regime shifted? (Trending → volatile, stable → crisis, etc.)
4. **Read coordination signals**: Check the pheromone field (mesh coordination signals from peer agents) for threat, opportunity, and wisdom pheromones.

The output is an observation Engram — a structured record of everything the agent perceived on this tick.

```rust
// Conceptual: the PERCEIVE step in Synapse terms
async fn perceive(
    substrate: &dyn Substrate,
    probes: &[Box<dyn Probe>],
    state: &AgentState,
) -> Result<Observation> {
    // Run all T0 probes in parallel (they're pure functions)
    let probe_results: Vec<ProbeResult> = probes
        .iter()
        .map(|p| p.evaluate(&state.engine_state))
        .collect();

    // Fetch domain-specific observations via Substrate
    let domain_obs = substrate.query(
        &Query::current_state(),
        &state.context,
    ).await?;

    // Detect regime changes
    let regime = detect_regime(&probe_results, &state.previous_regime);

    // Read pheromone field (coordination signals from mesh peers)
    let pheromones = substrate.query(
        &Query::pheromones(state.active_domains()),
        &state.context,
    ).await?;

    Ok(Observation {
        tick: state.current_tick,
        probes: probe_results,
        domain_state: domain_obs,
        regime,
        pheromones,
        anomalies: probe_results.iter()
            .filter(|p| p.is_anomalous())
            .cloned()
            .collect(),
    })
}
```

### Step 2: EVALUATE — `Scorer.score()`

Score retrieved Engrams by relevance, recency, emotional congruence, and confidence. This maps to `Scorer.score()` at L2 Scaffold.

The scoring function combines four factors (following Bower 1981 for mood-congruent memory):

```
score = w_recency × recency(Ebbinghaus)
      + w_importance × quality(confidence × validation_ratio)
      + w_relevance × cosine_similarity(query, entry)
      + w_emotional × PAD_cosine(current_mood, entry_affect)
```

The fourth factor — emotional congruence — implements Bower's (1981) mood-congruent memory: the agent's current emotional state (read from CorticalState via the Daimon cross-cut) biases which memories surface. An agent experiencing low pleasure (failing predictions) retrieves warnings and past failures. An agent experiencing high dominance (improving accuracy) retrieves validated heuristics and successful strategies.

Every 100 ticks, **contrarian retrieval** forces mood-opposite entries to prevent rumination loops. This ensures the agent considers counterarguments even when its affect state would otherwise bias retrieval toward confirming its current trajectory.

### Step 3: ATTEND — `Router.select()`

The tier gate decision. This is where the System 1 / System 2 split happens. The Router selects the cognitive tier (T0, T1, or T2) based on the prediction error computed from probe results.

```rust
// The ATTEND step: which cognitive tier handles this tick?
fn attend(
    router: &dyn Router,
    observation: &Observation,
    state: &AgentState,
) -> InferenceTier {
    // Compute aggregate prediction error from probes
    let prediction_error = compute_prediction_error(
        &observation.probes,
        &state.predictions,
        &observation.regime,
    );

    // Compute adaptive threshold (modulated by affect and context)
    let threshold = compute_adaptive_threshold(state);

    // Route via the CascadeRouter
    if prediction_error < threshold {
        InferenceTier::T0  // Suppress. ~80% of ticks.
    } else if prediction_error < threshold * 2.0 {
        InferenceTier::T1  // Fast model. ~15% of ticks.
    } else {
        InferenceTier::T2  // Full model. ~5% of ticks.
    }
}
```

The adaptive threshold considers:
- **Affect state**: Low dominance (agent feels uncertain) → lower threshold → escalate more readily to LLM deliberation.
- **Arousal**: High arousal (surprising recent events) → lower threshold → pay more attention.
- **Resource constraints**: Approaching budget ceiling → higher threshold → be more conservative about LLM calls.
- **Strategy confidence**: High confidence in current plan → higher threshold → coast on existing heuristics.

See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the full dual-process gating specification.

### Step 4: INTEGRATE — `Composer.compose()`

If the tier is T1 or T2, assemble the context window for the LLM call. The Composer takes scored, routed Engrams and assembles them under a token budget constraint. This maps to `Composer.compose()` at L2 Scaffold.

For T0 ticks, this step is skipped entirely — no context assembly needed because no LLM will be called.

For T1 ticks, the Composer assembles a **Focused** context (reduced set: current observation, top-5 retrieved entries, active positions, critical warnings). Token budget: ~4,000 tokens.

For T2 ticks, the Composer assembles a **Full** context (the complete Cognitive Workspace following Baddeley's 2000 working memory model): all invariants, current strategy, PLAYBOOK heuristics, retrieved episodes and insights, causal graph edges, dream hypotheses, somatic landscape readings, pheromone summary, and conversation tail (if human is chatting). Token budget: ~32,000 tokens.

Context assembly uses the VCG Attention Auction (see [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md)) to allocate tokens optimally across competing subsystems.

### Step 5: ACT — `Agent.execute()`

If T1 or T2: call the LLM with the assembled context. Produce output Engrams (tool calls, text, structured decisions).

For T0 ticks, this step is skipped. The agent produces no output — it simply observed and determined that nothing interesting happened.

The LLM call routes through the `CascadeRouter` in `roko-learn/src/cascade_router.rs`, which selects the specific model within the tier. The cascade uses three stages: Static (< 50 observations, use a fixed routing table), Confidence (50-200 observations, route by confidence score), and UCB1 (> 200 observations, contextual bandit with LinUCB for exploration/exploitation balance).

### Step 6: VERIFY — `Gate.verify()`

If the agent acted, verify the output against ground truth. This maps to `Gate.verify()` at L3 Harness. The Gate returns a `Verdict` directly — not `Result<Verdict>`. A gate failure is not an error; it is a `Verdict { passed: false, ... }`.

Verification is domain-specific:
- **Coding domain**: `CompileGate`, `TestGate`, `ClippyGate` — does the code compile? Do tests pass? Are there warnings?
- **Chain domain**: `TxSimGate` (simulate in mirage-rs), `WalletGate` (position limits), `VerifyChainGate` (on-chain receipt).
- **Research domain**: `LlmJudgeGate` — subjective quality check via a separate LLM call.

The gate pipeline features ratcheting (prevents regression — best-ever results are the floor for future attempts) and adaptive thresholds (EMA-based adjustment per gate rung).

### Step 7: PERSIST — `Substrate.put()`

Store the output Engram with lineage tracking. The Engram's `lineage` field records all parent Engrams that contributed to this output — the observation, the retrieved knowledge entries, the prompt, the gate verdict. This forms the audit DAG that enables forensic replay (see topic [11-safety](../11-safety/INDEX.md)).

### Step 8: ADAPT — `Policy.decide()`

Fire policies based on the outcome. Policies are batch observers that watch Engram streams and emit new Engrams. Common policies:

- **Episode logging**: Record the tick as an episode in `.roko/episodes.jsonl`.
- **Efficiency tracking**: Log per-turn cost, latency, and cache hit rate to `.roko/learn/efficiency.jsonl`.
- **Router feedback**: Feed the outcome (passed/failed, cost, latency) back to the CascadeRouter via `Router.feedback()`.
- **Playbook updates**: If this tick matched a PLAYBOOK rule, update the rule's effectiveness score.

### Step 9: META-COGNIZE — `Daimon.assess()`

Update the Daimon's affect state based on the tick's outcome. The Daimon computes PAD (Pleasure-Arousal-Dominance) deltas from prediction residuals following Barrett's (2017) theory of constructed emotion:

- **Pleasure** = `accuracy - baseline`. Correct predictions → positive. Failures → negative (with 1.6× negativity bias).
- **Arousal** = `|residual|`. Large errors in either direction → high arousal.
- **Dominance** = `trend_direction`. Improving accuracy → high dominance. Declining → low.

The PAD update writes to the CorticalState (see [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md)) for zero-latency reads by other subsystems.

---

## Adaptive Interval Computation

Gamma does not tick at a fixed rate. The interval adapts to environmental volatility:

```rust
/// Compute the gamma tick interval based on recent probe results.
///
/// More anomalies → faster ticks (down to 5s minimum).
/// Fewer anomalies → slower ticks (up to 15s maximum).
///
/// This implements the "Adaptive Clock" from the Friston free energy
/// principle framework: sample the environment more frequently when
/// prediction error is high (Friston 2010).
fn compute_gamma_interval(violations: &[Violation]) -> Duration {
    Duration::from_secs(15)
        .mul_f64(1.0 / (1.0 + violations.len() as f64 * 0.3))
        .max(Duration::from_secs(5))
}
```

At zero violations, the interval is 15 seconds. At 1 violation, it drops to ~11.5 seconds. At 3 violations, it drops to ~7.9 seconds. At 7+ violations, it approaches the 5-second floor.

This adaptive behavior has cost implications. During volatile periods (many anomalies, fast ticks), gamma fires ~720 times per hour. During calm periods (few anomalies, slow ticks), gamma fires ~240 times per hour. The daily cost model adjusts accordingly:

| Regime | Gamma Interval | Ticks/Day | T0 Rate | T1 Calls | T2 Calls | Estimated Daily Cost |
|---|---|---|---|---|---|---|
| Calm | ~15s | ~5,760 | ~90% | ~461 | ~115 | ~$1.00 (with context eng.) |
| Normal | ~10s | ~8,640 | ~80% | ~864 | ~288 | ~$2.50 (with context eng.) |
| Volatile | ~5s | ~17,280 | ~60% | ~1,440 | ~864 | ~$8.00 (with context eng.) |

Without the tier gating system (every tick at T2): 17,280 × $0.10 = **$1,728/day** in the volatile case. Tier gating alone provides a **~35× cost reduction**. Context engineering (caching, prompt cache alignment, tool pruning, multi-model routing via the CascadeRouter) provides an additional **~6× reduction**.

---

## The DecisionCycleRecord

Every gamma tick produces a **DecisionCycleRecord** — a typed, self-contained record of everything that happened during that tick. This is NOT a conversation message. It is a structured data record that serves as:

1. **The unit of dream replay**: During delta consolidation (see [06-delta-consolidation-loop.md](./06-delta-consolidation-loop.md)), episodes are selected for replay using the Mattar-Daw utility formula. The record IS the episode — observation, action, outcome, emotional state, and regime are already structured fields. No extraction step needed.

2. **The unit of credit assignment**: The cybernetic self-tuning loop traces outcomes to the context entries that contributed to the decision. The record's `context_bundle_summary` tells exactly which Neuro entries were in context, and `outcome` tells whether the result was positive.

3. **The unit of resource accounting**: Every tick records cost (inference cost, domain-specific costs like gas for chain), resource consumption, and tier classification.

4. **The source of event fabric events**: Every field maps to a typed event. The rendering layer (TUI, dashboard, API) translates record fields to display events with no additional logic.

```rust
/// The structured output of a single gamma tick.
///
/// This replaces the "conversation message" as the fundamental unit
/// of agent cognition. Structured fields enable direct credit
/// assignment, dream replay, and resource accounting without
/// parsing natural language output.
pub struct DecisionCycleRecord {
    // Identity
    pub tick: u64,
    pub timestamp: SystemTime,
    pub agent_id: AgentId,

    // Step 1: PERCEIVE
    pub observation: Observation,
    pub regime: Regime,
    pub probe_results: Vec<ProbeResult>,
    pub anomalies: Vec<Anomaly>,

    // Step 2-3: EVALUATE + ATTEND
    pub prediction_error: f32,
    pub deliberation_threshold: f32,
    pub tier: InferenceTier,
    pub gating_reason: String,

    // Step 4: INTEGRATE (context assembly)
    pub context_bundle_summary: ContextSummary,
    pub retrieved_entries: Vec<EngramSummary>,
    pub active_interventions: Vec<InterventionSummary>,

    // Step 5: ACT (if T1/T2)
    pub deliberation: Option<DeliberationRecord>,

    // Step 6-7: VERIFY + PERSIST
    pub actions: Vec<ActionRecord>,
    pub outcome: Option<OutcomeRecord>,

    // Step 8: ADAPT
    pub episodes_written: Vec<ContentHash>,
    pub neuro_mutations: Vec<NeuroMutation>,

    // Step 9: META-COGNIZE
    pub pad_before: PadVector,
    pub pad_after: PadVector,
    pub somatic_markers_fired: Vec<SomaticMarkerRef>,
    pub primary_emotion: PlutchikLabel,

    // Cost accounting
    pub inference_cost: f64,
    pub domain_cost: f64,    // e.g., gas for chain
    pub total_cost: f64,
}
```

### Conversation Preserved for the Minority Case

When a human chats with their agent (~20% of ticks), the conversation is stored in a separate JSONL file that follows the session format. The conversation informs the heartbeat — the conversation tail is injected into the Cognitive Workspace as one of its structured categories — but the heartbeat does not become a conversation.

---

## Mapping to Existing Code

The gamma loop maps directly to the existing orchestration in `roko-cli/src/orchestrate.rs`. The current `orchestrate.rs` is effectively a gamma loop — it runs the main tick cycle: discover plans, dispatch agents, run gates, persist results.

| Gamma Step | Current Implementation | Target Crate |
|---|---|---|
| PERCEIVE | Plan discovery + task loading | `roko-runtime` (L0) |
| EVALUATE | (implicit in task selection) | `roko-core::Scorer` (L2) |
| ATTEND | (always T2 currently — no gating) | `bardo-primitives::TierRouter` (L1) |
| INTEGRATE | `RoleSystemPromptSpec` + 6-layer builder | `roko-compose::Composer` (L2) |
| ACT | Agent dispatch via `ClaudeCliBackend` / `ExecAgent` | `roko-agent` (L1) |
| VERIFY | Gate pipeline (compile, test, clippy, diff) | `roko-gate` (L3) |
| PERSIST | FileSubstrate JSONL append | `roko-fs` (L0) |
| ADAPT | Episode logging + efficiency events | `roko-learn` (L3-L4) |
| META-COGNIZE | (not yet implemented) | `roko-daimon` (cross-cut) |

### What Exists

- The orchestration loop in `orchestrate.rs` runs the full tick cycle but does not use adaptive timing — it processes tasks sequentially, not on a timed heartbeat.
- `InferenceTier` enum (T0/T1/T2) and `TierRouter` exist in `bardo-primitives/src/tier.rs` but are not wired into the orchestration loop.
- `CascadeRouter` in `roko-learn/src/cascade_router.rs` implements three-stage model routing but is used for model selection within a tier, not for tier gating itself.

### What is Missing (Implementation Plan 12a §I1)

- **I1**: Formal gamma loop with adaptive interval based on probe anomaly count.
- The existing orchestration loop needs to be refactored into a timed heartbeat with the `compute_gamma_interval()` function controlling the tick rate.
- T0 probe registry needs to be wired — the probe definitions exist conceptually but are not implemented as executable pure functions.
- CorticalState needs to be created and written to on each gamma tick.
- DecisionCycleRecord needs to be produced per tick instead of per-task.

---

## Gamma and the Ten Cognitive Mechanisms

Ten runtime mechanisms from the "Mind" specification (legacy `bardo-backup/prd/01-golem/03-mind.md`) operate alongside gamma, modulating how each tick behaves. They are not gamma steps; they are concurrent processes that inject into gamma:

| Mechanism | What It Does | Where in Gamma |
|---|---|---|
| **AttentionSalience** | Priority queue with decay for observation items | Step 1 (PERCEIVE) — which observations get priority |
| **HabituationMask** | Per-pattern exposure attenuation | Step 1 (PERCEIVE) — attenuate repeated patterns |
| **EventDrivenWakeup** | Condition-based clock interrupts | Before Step 1 — interrupt normal cadence for urgent events |
| **SleepPressure** | Accumulates pressure for dream consolidation | Step 3 (ATTEND) — accumulate toward delta threshold |
| **HomeostasisRegulator** | Proportional control for signal stability | Step 3 (ATTEND) — check signal stability |
| **ContextDelta** | I-frame/P-frame context compression | Step 4 (INTEGRATE) — compress context for LLM calls |
| **EpisodicReplay** | Case-based reasoning for deliberation | Step 4 (INTEGRATE) — inject relevant past episodes |
| **CompensationChain** | Saga-pattern rollback for multi-step actions | Steps 5-7 (ACT through PERSIST) — track rollback points |
| **StateSnapshot** | Content-addressed full agent state checkpoint | Step 9 (META-COGNIZE) — periodic checkpoint |
| **MetricsEmitter** | Wide-event telemetry and W3C tracing | Every step — emit telemetry events |

These mechanisms make gamma adaptive rather than mechanical. Without them, gamma would be a fixed pipeline executing the same steps identically on every tick. With them, gamma adjusts what it pays attention to (AttentionSalience), ignores repeated patterns (HabituationMask), wakes up early for urgent events (EventDrivenWakeup), and tracks whether it's time to dream (SleepPressure).

---

## OODA Loop Correspondence

Boyd's Observe-Orient-Decide-Act (OODA) loop maps directly onto the gamma pipeline, with one critical addition: the REFLECT step closes the learning loop that OODA leaves open.

| OODA Phase | Gamma Step(s) | What Happens |
|---|---|---|
| **Observe** | PERCEIVE | Probes + pheromone field capture environment state |
| **Orient** | EVALUATE + ATTEND | Scoring + prediction error orient the agent |
| **Decide** | ATTEND + INTEGRATE | Tier gate + context assembly select action approach |
| **Act** | ACT + VERIFY + PERSIST | Execution with verification and persistence |
| _(missing in OODA)_ | ADAPT + META-COGNIZE | Episode recording + affect update close the learning loop |

The REFLECT addition (ADAPT + META-COGNIZE) is what makes the gamma loop self-improving. Without it, the loop would execute actions without learning from outcomes. With it, every tick feeds the cybernetic self-tuning loop that improves future ticks.

---

## Academic Foundations

- **Buzsáki 2006** — "Rhythms of the Brain" (Oxford University Press). Gamma oscillations (30-100 Hz) in biological cognition: fast sensory processing and attention binding.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Hierarchical prediction at different temporal grains; adaptive sampling rate based on prediction error.
- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). System 1 (fast, automatic) / System 2 (slow, deliberate) dual-process theory.
- **Bower 1981** — "Mood and memory" (American Psychologist 36(2)). Mood-congruent memory retrieval.
- **Barrett 2017** — "How Emotions Are Made" (Houghton Mifflin). Constructed emotion theory: emotions as summary statistics of prediction errors.
- **Sumers et al. 2023** — CoALA framework (arXiv:2309.02427). Cognitive architectures for language agents.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176). Cascade architectures for LLM cost reduction.
- **Baddeley 2000** — "The episodic buffer" (Trends in Cognitive Sciences 4(11)). Working memory model used for context assembly.
- **Boyd 1987** — OODA loop. Observe-Orient-Decide-Act decision cycle.
- **Mattar & Daw 2018** — "Prioritized memory access explains planning and hippocampal replay" (Nature Neuroscience 21). Utility-based episode selection for replay.

---

## Current Status and Gaps

**What exists:**
- The orchestration loop in `roko-cli/src/orchestrate.rs` is effectively a gamma loop — it runs plan discovery, agent dispatch, gate verification, and result persistence.
- `InferenceTier` enum (T0/T1/T2) and `TierRouter` in `bardo-primitives/src/tier.rs`.
- `CascadeRouter` three-stage model routing in `roko-learn/src/cascade_router.rs`.
- Episode logging to `.roko/episodes.jsonl`.
- Efficiency event tracking to `.roko/learn/efficiency.jsonl`.

**What is missing:**
- Formal gamma loop with adaptive interval (I1 in implementation-plans/12a-cognitive-layer.md).
- T0 probe registry with executable pure function probes.
- CorticalState shared perception surface.
- DecisionCycleRecord per-tick structured output.
- Tier gating integration — currently all tasks run at T2 (full LLM).
- Affect-modulated threshold computation via Daimon.
- Ten cognitive mechanisms (attention, habituation, sleep pressure, etc.).

---

## Cross-References

- See [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md) for the CoALA framework underlying the 9-step pipeline
- See [01-universal-loop-mapping.md](./01-universal-loop-mapping.md) for the CoALA → Synapse trait mapping
- See [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md) for how gamma relates to theta and delta
- See [05-theta-reflective-loop.md](./05-theta-reflective-loop.md) for the reflective loop that summarizes gamma ticks
- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the tier gating mechanism
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the 16 zero-LLM probes that drive T0 suppression
- See [12-attention-auction-and-gating.md](./12-attention-auction-and-gating.md) for context assembly via VCG auction
- See topic [09-daimon](../09-daimon/INDEX.md) for the affect engine (PAD vectors, somatic markers)
- See topic [05-learning](../05-learning/INDEX.md) for the CascadeRouter and learning feedback loops
