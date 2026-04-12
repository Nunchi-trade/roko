# Three Cognitive Speeds: Gamma, Theta, Delta

> Named after EEG frequency bands, three concurrent timescales govern all agent cognition — reactive perception, reflective planning, and offline consolidation.

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-coala-9-step-pipeline.md](./00-coala-9-step-pipeline.md)
**Key sources**: `refactoring-prd/01-synapse-architecture.md` §Three Cognitive Speeds, `refactoring-prd/02-five-layers.md` §Adaptive Clock, `implementation-plans/12a-cognitive-layer.md` §I

---

## Abstract

Every Roko agent operates at three timescales simultaneously. These are not sequential phases — they are three concurrent async tasks running in parallel, each processing information at a different temporal grain. The naming follows EEG frequency bands: Gamma (fast reactive), Theta (medium reflective), and Delta (slow consolidation).

The three-speed model draws from Friston's free energy principle (2010), which frames perception as hierarchical prediction at different temporal grains. Clark (2013) extends this into the "predictive brain" framework: biological cognition is nested prediction loops at multiple timescales. Buzsáki's "Rhythms of the Brain" (2006) establishes that oscillatory hierarchies in the brain enable simultaneous processing at different temporal resolutions — fast gamma oscillations (30-100 Hz) ride on top of slower theta oscillations (4-8 Hz), which ride on top of delta oscillations (0.5-4 Hz).

Roko implements this as three concurrent cognitive loops managed by the adaptive clock in `roko-runtime`. All three run simultaneously on separate async tasks. Gamma is the heartbeat. Theta is the breath. Delta is sleep.

---

## The Three Speeds

| Speed | Period | Name | What Happens | Trigger | Cost |
|---|---|---|---|---|---|
| **Gamma** | ~5-15s | Real-time / Reactive | One complete loop tick. Tool calls, LLM inference, verification. The main orchestration loop. | Async event loop (continuous) | T0: $0.00, T1: $0.001-0.003, T2: $0.01-0.25 |
| **Theta** | ~75s (30-120s range) | Reflection / Strategic | Summarize recent work. Update Daimon state. Check predictions. Re-evaluate plan. "Step back and think about the plan." | Episode completion, or every N=5 gamma cycles | T1-T2: $0.01-0.10 |
| **Delta** | Hours (~50 theta cycles) | Consolidation / Offline | Dreams: replay, synthesis, pruning. Knowledge tier promotion. Playbook compilation. Meta-cognition. | Idle detection (no active tasks), or scheduled (nightly) | T0-T1: $0.00-0.01 |

### Why Three Speeds, Not One

A single-speed architecture forces a painful tradeoff: either the agent ticks fast enough to catch urgent events (expensive, ~2,000 ticks/day at ~$0.10/tick = $200/day) or it ticks slowly enough to be cheap (missing time-sensitive signals).

The three-speed model resolves this:
- **Gamma** handles urgent reactive tasks — what is happening RIGHT NOW? Most ticks suppress at T0 ($0.00), making high-frequency perception affordable.
- **Theta** handles strategic reflection — am I on the right track? This fires less frequently but with more cognitive depth.
- **Delta** handles consolidation — what have I learned? This fires during idle time at minimal cost.

The total daily cost with three speeds: ~$2-50 (depending on domain volatility), versus $100-500+ with a single-speed always-on approach.

### Concurrency Model

All three speeds run as separate `tokio` tasks:

```rust
// Conceptual structure (target architecture)
async fn run_agent(config: AgentConfig) {
    let shared_state = Arc::new(RwLock::new(AgentState::new(config)));

    // Gamma: reactive perception + action
    let gamma_handle = tokio::spawn({
        let state = shared_state.clone();
        async move { gamma_loop(state).await }
    });

    // Theta: periodic reflection
    let theta_handle = tokio::spawn({
        let state = shared_state.clone();
        async move { theta_loop(state).await }
    });

    // Delta: consolidation during idle time
    let delta_handle = tokio::spawn({
        let state = shared_state.clone();
        async move { delta_loop(state).await }
    });

    tokio::select! {
        _ = gamma_handle => {},
        _ = theta_handle => {},
        _ = delta_handle => {},
    }
}
```

The three loops share state through `Arc<RwLock<AgentState>>`. Gamma has priority — if a gamma tick and a theta tick collide, gamma runs first. Delta can be preempted by incoming gamma work (task arrives during dreaming → dream is paused via `CognitiveSignal::Pause`, resumed later).

---

## Gamma: The Heartbeat (~5-15s)

Gamma is the main orchestration loop — what most agent frameworks call "the agent." Every gamma tick runs the full 9-step CoALA pipeline (or the Synapse loop equivalent):

1. **PERCEIVE**: Run T0 probes, fetch environment state.
2. **EVALUATE**: Score retrieved knowledge.
3. **ATTEND**: Gate decision (T0/T1/T2).
4. **INTEGRATE**: Assemble context window (if T1/T2).
5. **ACT**: Call LLM, produce output (if T1/T2).
6. **VERIFY**: Check against ground truth (if acted).
7. **PERSIST**: Store output with lineage.
8. **ADAPT**: Fire policies, log episode.
9. **META-COGNIZE**: Update Daimon state.

**Adaptive interval**: Gamma accelerates when the environment is volatile (more issues detected → faster ticks, down to 5s) and slows when calm (fewer anomalies → slower ticks, up to 15s).

```rust
// Adaptive gamma interval
fn compute_gamma_interval(violations: &[Violation]) -> Duration {
    Duration::from_secs(15)
        .mul_f64(1.0 / (1.0 + violations.len() as f64 * 0.3))
        .max(Duration::from_secs(5))
}
```

**Cost structure**: ~80% of gamma ticks suppress at T0 ($0.00). ~15% escalate to T1 ($0.001-0.003). ~5% reach T2 ($0.01-0.25). This makes high-frequency gamma perception economically viable.

See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for the full Gamma specification.

---

## Theta: The Breath (~75s)

Theta fires periodically — every 5 gamma cycles or upon episode completion (whichever comes first). It is the agent's "step back and think about the plan" moment.

**What Theta does:**
1. **Summarize recent gamma work**: What happened in the last 5 ticks? Any patterns?
2. **Update Daimon state**: Compute aggregate affect from recent outcomes. Am I succeeding? Struggling? Exploring?
3. **Check predictions**: How accurate were my predictions over the last theta cycle? Calibration check.
4. **Re-evaluate plan**: Is the current plan still the best approach? Should I re-plan?
5. **Trigger interventions**: If stuck (>3 retries on same task), suggest escalation.

**Adaptive interval**: Theta adjusts based on regime multipliers. During volatile periods (chain domain: high market volatility; coding domain: many build failures), theta accelerates toward 30s. During calm periods, theta stretches to 120s.

| Domain Regime | Theta Interval | Rationale |
|---|---|---|
| Calm / stable | ~120s | Little changes; less frequent reflection needed |
| Normal | ~75s | Standard reflection cadence |
| Volatile / troubled | ~30s | Rapid changes require frequent re-evaluation |

**Cost structure**: Theta always invokes at least T1 (it needs LLM reasoning to reflect). Most theta cycles use T1 ($0.005-0.01). Complex situations escalate to T2 ($0.03-0.10).

See [05-theta-reflective-loop.md](./05-theta-reflective-loop.md) for the full Theta specification.

---

## Delta: Sleep (~Hours)

Delta runs during agent idle time — when no active tasks are queued and the gamma loop has nothing to do. It is the agent's "offline learning" phase, corresponding to biological sleep consolidation.

**What Delta does:**
1. **Dream replay**: Review accumulated episodes using the Mattar-Daw utility formula (see topic [10-dreams](../10-dreams/INDEX.md)).
2. **Knowledge consolidation**: Promote knowledge tiers (Transient → Working → Consolidated → Persistent) based on validation history.
3. **Pattern discovery**: Mine trigram patterns across episodes via `roko-learn::PatternMiner`.
4. **Hypothesis generation**: HDC bundling of related insights produces novel knowledge combinations.
5. **Playbook compilation**: Top heuristics compiled into reusable action rules.
6. **Routing optimization**: Update CascadeRouter weights based on which models performed best.
7. **Prompt optimization**: Determine which context sections led to better outcomes.
8. **Meta-cognition**: "Am I getting better overall? What should I focus on improving?"

**Trigger conditions**: Delta fires when:
- No active tasks for >5 minutes (idle detection).
- Scheduled time (nightly consolidation cron).
- Episode count since last delta exceeds threshold (~50 episodes).
- Explicit `roko dream run` CLI command.

**Non-blocking**: Dreams do not block the agent. If a new task arrives during delta processing, the dream is paused (`CognitiveSignal::Pause`), the dream state is serialized, and gamma takes over. The dream resumes when the agent goes idle again.

**Cost structure**: Delta uses cheap models for most operations. NREM replay uses Haiku-class (T1, $0.001-0.003). REM imagination uses Sonnet-class (T1-T2, $0.01-0.05). Integration and staging are pure computation (T0, $0.00). Impact on throughput: ~0% when tasks are available (dreams only run when there is nothing else to do).

| Delta Phase | Model Class | Duration | Cost |
|---|---|---|---|
| NREM Replay | Haiku-class (T1) | 8-15 min | $0.01-0.05 |
| REM Imagination | Sonnet-class (T1-T2) | 5-15 min | $0.05-0.20 |
| Integration/Staging | Pure computation (T0) | 5-10 min | $0.00 |
| **Total per Delta cycle** | | **~30 min** | **~$0.06-0.25** |

See [06-delta-consolidation-loop.md](./06-delta-consolidation-loop.md) for the full Delta specification.

---

## The Hierarchy: Gamma Rides on Theta Rides on Delta

The three speeds form a nested hierarchy, exactly like biological brain rhythms (Buzsáki 2006):

```
Delta (~hours):    ┌──────────────────────────────────────────────────┐
                   │  Consolidation: dreams, tier promotion, playbook  │
                   │  Fires: ~50 theta cycles or on idle              │
                   └──────────────────────────────────────────────────┘
                        │ contains ~50 theta cycles
                        ▼
Theta (~75s):      ┌──────┬──────┬──────┬──────┬──────┐
                   │ Refl │ Refl │ Refl │ Refl │ Refl │ ...
                   └──────┴──────┴──────┴──────┴──────┘
                        │ each contains ~5 gamma cycles
                        ▼
Gamma (~10s):      ┌──┬──┬──┬──┬──┬──┬──┬──┬──┬──┐
                   │T0│T0│T1│T0│T0│T0│T0│T2│T0│T0│ ...
                   └──┴──┴──┴──┴──┴──┴──┴──┴─��┴──┘
                   80%     15%           5%
                   free    cheap         full
```

Each higher level integrates information from the level below:
- **Gamma** produces individual tick observations and outcomes.
- **Theta** summarizes gamma ticks into patterns and plan adjustments.
- **Delta** consolidates theta summaries into durable knowledge and behavioral updates.

Information flows both up (gamma → theta → delta) and down (delta produces knowledge that gamma uses; theta adjustments change gamma behavior). This bidirectional flow is what makes the architecture adaptive rather than merely layered.

---

## Mapping to Existing Code

The three-speed model maps directly to existing Roko types:

| Speed | `bardo-primitives` (→ `roko-primitives`) | `roko-learn` | `roko-compose` |
|---|---|---|---|
| Gamma | `InferenceTier::T0/T1/T2` | `CascadeRouter` tier selection | `ContextTier::Surgical/Focused/Full` |
| Theta | (not yet mapped) | `episode_logger` episode boundaries | (not yet mapped) |
| Delta | (not yet mapped) | `PatternMiner`, `KMedoids`, `baseline`, `regression` | (not yet mapped) |

The `InferenceTier` enum in `bardo-primitives/src/tier.rs` directly corresponds to the T0/T1/T2 gating within gamma ticks. The `CascadeRouter` in `roko-learn/src/cascade_router.rs` implements the three-stage routing that drives tier selection. The context tier system in `roko-compose/src/context_provider.rs` (Surgical/Focused/Full) aligns with the cognitive depth at each speed.

---

## Academic Foundations

- **Buzsáki 2006** — "Rhythms of the Brain" (Oxford University Press). Oscillatory hierarchies in biological cognition.
- **Friston 2010** — "The Free-Energy Principle" (Nature Reviews Neuroscience 11(2)). Hierarchical prediction at different temporal grains.
- **Clark 2013** — "Whatever Next?" (Behavioral and Brain Sciences 36(3)). Predictive processing at multiple timescales.
- **Sumers et al. 2023** — CoALA framework (arXiv:2309.02427). Cognitive architecture for language agents.
- **Chen et al. 2023** — FrugalGPT (arXiv:2305.05176). Cost optimization through cascade architectures.
- **McClelland et al. 1995** — Complementary Learning Systems. Fast episodic memory (gamma/theta) consolidates into slow semantic memory (delta) during sleep.

---

## Current Status and Gaps

**What exists:**
- Gamma loop: The orchestration loop in `roko-cli/src/orchestrate.rs` is effectively a gamma loop — it runs the main tick cycle.
- `InferenceTier` T0/T1/T2 and `TierRouter` in `bardo-primitives/src/tier.rs`.
- `CascadeRouter` in `roko-learn/src/cascade_router.rs` for tier selection.

**What is missing (see `12a-cognitive-layer.md` §I):**
- **I1**: Formal gamma loop with adaptive interval.
- **I2**: Theta loop with periodic "step back and think."
- **I3**: Delta loop triggering dreams, playbook compilation, meta-cognition.
- **I4**: Frequency scheduler deciding which loop to run based on context.
- **I5**: Meta-cognition hook ("Am I stuck? Am I thrashing? Should I escalate?").

---

## Cross-References

- See [04-gamma-reactive-loop.md](./04-gamma-reactive-loop.md) for Gamma details
- See [05-theta-reflective-loop.md](./05-theta-reflective-loop.md) for Theta details
- See [06-delta-consolidation-loop.md](./06-delta-consolidation-loop.md) for Delta details
- See [07-adaptive-clock.md](./07-adaptive-clock.md) for the adaptive clock managing all three speeds
- See topic [10-dreams](../10-dreams/INDEX.md) for offline consolidation (Delta phase)
- See topic [09-daimon](../09-daimon/INDEX.md) for affect state updates (Theta phase)
