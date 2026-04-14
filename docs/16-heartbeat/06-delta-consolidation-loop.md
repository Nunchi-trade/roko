# Delta: The Consolidation Loop (~Hours)

> The slowest cognitive frequency — offline learning, dream replay, knowledge promotion, and meta-cognition during idle time.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md), [05-theta-reflective-loop.md](./05-theta-reflective-loop.md)
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §Dreams, `refactoring-prd/09-innovations.md` §V, legacy `bardo-backup/prd/01-golem/18-cortical-state.md` §Three Concurrent Scales, `implementation-plans/12a-cognitive-layer.md` §I3

---

## Abstract

Delta is sleep. When the agent has nothing to do — no active tasks, no pending observations, no urgent gamma work — it enters delta consolidation: the offline learning phase that turns individual experiences into durable knowledge. Delta corresponds to biological slow-wave sleep (0.5-4 Hz delta oscillations), where the brain consolidates episodic memories into semantic knowledge, prunes irrelevant connections, and generates novel hypotheses through dream replay (McClelland et al. 1995, "Why there are complementary learning systems in the hippocampus and neocortex", Psychological Review 102(3)).

Delta is where the agent gets smarter without doing any task work. It replays prioritized episodes using the Mattar-Daw utility formula, generates counterfactual scenarios using Boden's three creativity modes and Pearl's structural causal models, promotes knowledge across tiers (Transient → Working → Consolidated → Persistent), compiles top heuristics into reusable playbook rules, optimizes model routing weights, and performs meta-cognition ("Am I getting better overall?").

Critically, delta is **non-blocking**. If a new task arrives during delta processing, the dream is paused via `CognitiveSignal::Pause`, the dream state is serialized to disk, and gamma takes over immediately. The dream resumes when the agent goes idle again. This ensures that delta never delays reactive work — it runs only when there is genuinely nothing else to do.

This document specifies the delta loop: its trigger conditions, three-phase dream cycle, knowledge tier promotion, playbook compilation, routing optimization, and the meta-cognition assessment.

---

## Trigger Conditions

Delta fires when any of the following conditions are met:

1. **Idle detection**: No active tasks for >5 minutes. The agent has completed all pending work and is waiting for new tasks. This is the most common trigger.
2. **Scheduled time**: A configured consolidation schedule (e.g., nightly at 02:00 UTC) fires regardless of current activity, though it will wait for the current gamma tick to complete.
3. **Episode count threshold**: The number of episodes since the last delta cycle exceeds a threshold (~50 episodes, or approximately 50 theta cycles' worth of work). This prevents the agent from accumulating unbounded unprocessed experience.
4. **Explicit command**: `roko dream run` CLI command triggers delta manually. Useful for development and testing.

```rust
/// Determine whether to enter delta consolidation.
fn should_enter_delta(
    idle_duration: Duration,
    episodes_since_last_delta: usize,
    scheduled_delta_time: Option<SystemTime>,
    explicit_trigger: bool,
) -> bool {
    explicit_trigger
        || idle_duration > Duration::from_secs(300)  // 5 minutes idle
        || episodes_since_last_delta >= 50
        || scheduled_delta_time.map_or(false, |t| SystemTime::now() >= t)
}
```

---

## The Three-Phase Dream Cycle

Delta consolidation follows a three-phase cycle modeled on the biological sleep cycle. Each phase serves a distinct cognitive function.

### Phase 1: NREM Replay (8-15 minutes)

Non-REM replay (corresponding to biological slow-wave sleep) selects episodes for replay using the **Mattar-Daw utility formula** (Mattar & Daw 2018, "Prioritized memory access explains planning and hippocampal replay", Nature Neuroscience 21):

```
Utility(episode) = Gain × Need × (1 / spacing_penalty)

where:
  Gain     = magnitude of prediction error in this episode
  Need     = how frequently similar situations arise (frequency in recent history)
  spacing_penalty = time since last replay (Ebbinghaus spacing effect)
```

Episodes with high utility are replayed first. This is NOT random or chronological replay — it is **prioritized** replay that focuses on the most informative experiences. An episode where the agent made a large prediction error in a commonly-occurring situation is replayed before an episode where it performed well in a rare situation.

**Perturbed replay (30% of replays)**: To stress-test learned knowledge, 30% of NREM replays inject perturbations — simulated adversarial conditions:
- 2× slippage (for chain domain)
- 5× gas costs (for chain domain)
- Correlation shifts (for any domain)
- Build environment changes (for coding domain)
- Data source unavailability (for research domain)

PAD modulates selection: when the agent's affect state shows anxiety (low pleasure, high arousal), warning episodes receive a 2× weight multiplier, ensuring the agent processes threatening experiences more thoroughly during anxious periods.

```rust
/// Phase 1: NREM Replay — prioritized episode review.
///
/// Uses Mattar-Daw utility formula to select and replay episodes.
/// 30% of replays are perturbed for robustness testing.
///
/// Model class: Haiku-class (T1), cost ~$0.001-0.003 per replay.
async fn nrem_replay(
    episodes: &[Episode],
    neuro: &mut NeuroStore,
    calibration: &CalibrationTracker,
    affect: &PadVector,
) -> Result<NremOutput> {
    // Compute utility for each unprocessed episode
    let mut scored: Vec<(usize, f64)> = episodes.iter()
        .enumerate()
        .map(|(i, ep)| {
            let gain = ep.prediction_error.abs() as f64;
            let need = ep.situation_frequency as f64;
            let spacing = (SystemTime::now()
                .duration_since(ep.last_replayed)
                .unwrap_or_default()
                .as_secs() as f64)
                .max(1.0);
            let utility = gain * need / spacing.sqrt();

            // PAD modulation: anxious → weight warnings 2×
            let anxiety_boost = if affect.pleasure < -0.2 && affect.arousal > 0.3 {
                if ep.contains_warnings { 2.0 } else { 1.0 }
            } else {
                1.0
            };

            (i, utility * anxiety_boost)
        })
        .collect();

    // Sort by utility descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    // Replay top episodes (budget: 8-15 minutes)
    let mut output = NremOutput::new();
    let replay_budget = Duration::from_secs(12 * 60);  // 12 min default
    let start = Instant::now();

    for (idx, _utility) in scored {
        if start.elapsed() > replay_budget {
            break;
        }

        let episode = &episodes[idx];

        // 30% perturbed replay
        let perturbed = rand::random::<f32>() < 0.30;
        let replay = if perturbed {
            replay_with_perturbation(episode, neuro).await?
        } else {
            replay_faithful(episode, neuro).await?
        };

        output.replayed.push(replay);
    }

    Ok(output)
}
```

**Cost**: NREM replay uses Haiku-class models (T1). Each replay costs $0.001-0.003. A typical NREM phase replays 20-40 episodes, costing $0.02-0.12 total.

### Phase 2: REM Imagination (5-15 minutes)

REM (Rapid Eye Movement) imagination generates counterfactual scenarios using **Boden's three creativity modes** (Boden 2004, "The Creative Mind: Myths and Mechanisms"):

1. **Combinational creativity**: Recombine existing knowledge fragments into novel combinations. Take two successful strategies from different domains and merge them.
2. **Exploratory creativity**: Traverse the boundaries of the known strategy space. What happens at the extreme edges of current heuristics?
3. **Transformational creativity**: Break constraints to discover entirely new strategy regions. Violate an assumption and see what emerges.

These modes are implemented via **Pearl's structural causal models** (Pearl 2009, "Causality: Models, Reasoning, and Inference"). The agent builds a causal graph of its domain, then generates counterfactuals by intervening on causal variables — "What would have happened if I had chosen a different strategy here?"

**Emotional depotentiation**: During REM, the emotional charge on highly charged memories is reduced by 0.3-0.5 per cycle (Walker & van der Helm 2009, "Overnight therapy? The role of sleep in emotional brain processing", Psychological Bulletin 135(5)). This prevents panic lock-in — an agent that had a catastrophic failure won't remain permanently traumatized. The factual content of the memory is preserved; the emotional intensity is gradually reduced.

**HDC counterfactual synthesis**: Using HDC (Hyperdimensional Computing) permutation operations (Kanerva 2009, Cognitive Computation 1(2)), the agent generates novel knowledge combinations in nanoseconds — far faster than LLM-based generation. Bundle multiple insights, permute their positional roles, and check if the resulting vector has high Hamming similarity to any known successful pattern.

```rust
/// Phase 2: REM Imagination — counterfactual generation.
///
/// Uses Boden's creativity modes + Pearl's causal models to
/// generate novel hypotheses. Emotional depotentiation reduces
/// the intensity of highly charged memories.
///
/// Model class: Sonnet-class (T1-T2), cost ~$0.01-0.05 per scenario.
async fn rem_imagination(
    nrem_output: &NremOutput,
    neuro: &NeuroStore,
    causal_graph: &CausalGraph,
    daimon: &mut DaimonState,
) -> Result<RemOutput> {
    let mut output = RemOutput::new();

    // Combinational: recombine knowledge fragments
    let combinations = hdc_recombine(
        &neuro.recent_insights(20),
        &neuro.recent_heuristics(20),
    );
    output.novel_combinations.extend(combinations);

    // Exploratory: traverse strategy boundaries
    let boundary_scenarios = explore_boundaries(
        causal_graph,
        &nrem_output.replayed,
    ).await?;
    output.boundary_explorations.extend(boundary_scenarios);

    // Transformational: break constraints
    let broken_constraints = transform_constraints(
        causal_graph,
        &neuro.current_assumptions(),
    ).await?;
    output.constraint_breaks.extend(broken_constraints);

    // Emotional depotentiation on high-intensity memories
    for replay in &nrem_output.replayed {
        if replay.emotional_intensity > 0.7 {
            daimon.depotentiate(
                &replay.episode_hash,
                0.3,  // reduce intensity by 0.3 per cycle
            );
        }
    }

    Ok(output)
}
```

**Cost**: REM uses Sonnet-class models (T1-T2). Each scenario costs $0.01-0.05. A typical REM phase generates 5-15 scenarios, costing $0.05-0.20 total.

### Phase 3: Integration & Staging (5-10 minutes)

Dream outputs enter a **staging buffer** at 0.20-0.30 confidence. This is the critical safety valve: nothing generated during dreams is immediately trusted. Novel hypotheses, counterfactual insights, and recombined strategies all start at low confidence and must be validated through live execution before promotion.

```
Dream output (confidence 0.20-0.30)
    ↓ staging buffer
Live validation during subsequent gamma/theta ticks
    ↓ if validated, confidence rises
Promotion to Working memory at 0.50 confidence
    ↓ continued validation
Promotion to Consolidated memory at 0.70 confidence
    ↓ extensive validation
Promotion to Persistent memory at 0.90 confidence
```

Only hypotheses reaching 0.70 confidence through live validation are promoted to permanent memory. This prevents hallucinated insights from corrupting the knowledge base — the dream engine can freely generate speculative content, knowing that the validation pipeline will filter out noise.

**Cost**: Integration and staging are pure computation (T0). No LLM calls. $0.00.

---

## Knowledge Tier Promotion

Delta performs bulk knowledge tier promotion based on validation history. The four tiers correspond to increasing durability and decay resistance:

| Tier | Decay Multiplier | What It Means | Promotion Threshold |
|---|---|---|---|
| **Transient** | 0.1× base half-life | Fresh, unvalidated. May be noise. | (initial state) |
| **Working** | 0.5× base half-life | Used successfully once or twice. | confidence ≥ 0.50, used ≥ 2 times |
| **Consolidated** | 1.0× base half-life | Repeatedly validated. Reliable. | confidence ≥ 0.70, used ≥ 5 times |
| **Persistent** | 5.0× base half-life | Core knowledge. Very slow decay. | confidence ≥ 0.90, used ≥ 10 times |

The base half-life depends on the knowledge type (from Neuro specification):
- Insight: 48 hours base × tier multiplier
- Heuristic: 96 hours base × tier multiplier
- Warning: 24 hours base × tier multiplier
- CausalLink: 168 hours base × tier multiplier
- StrategyFragment: 72 hours base × tier multiplier
- AntiKnowledge: 12 hours base × tier multiplier (decays fastest — negative results lose relevance quickly)

During delta, every knowledge entry is evaluated for tier promotion or demotion:
- Entries with high confidence + frequent use → promote
- Entries with declining confidence or no recent use → demote (or let natural decay handle it)

This implements the **Complementary Learning Systems** framework (McClelland et al. 1995): fast episodic memory (gamma/theta) consolidates into slow semantic memory (delta) during "sleep."

---

## Playbook Compilation

Delta mines the episode history for patterns that can be compiled into reusable **playbook rules**. A playbook rule is a heuristic that was discovered through experience and validated through repeated use:

```rust
pub struct PlaybookRule {
    pub id: ContentHash,
    pub condition: String,      // When does this rule apply?
    pub action: String,         // What should the agent do?
    pub confidence: f32,        // How well-validated is this rule?
    pub source_episodes: Vec<ContentHash>,  // Which episodes generated it?
    pub success_rate: f32,      // Historical success rate
    pub times_applied: u32,     // How often has it been used?
}
```

Playbook compilation uses the `PatternMiner` in `roko-learn` to discover trigram patterns across episodes — sequences of (situation, action, outcome) that recur with positive outcomes. The top patterns (highest success rate × frequency) become playbook rules.

Playbook rules are particularly valuable because they enable T0 processing: when a gamma tick detects a situation that matches a playbook rule's condition, the agent can act without invoking an LLM, keeping the tick at T0 cost ($0.00).

---

## Routing Optimization

Delta updates the `CascadeRouter` weights based on which models performed best during recent gamma/theta cycles. The router tracks per-model outcomes (success rate, latency, cost) and adjusts the LinUCB exploration/exploitation balance:

- Models with high success + low cost → increased selection weight
- Models with low success or high cost → decreased selection weight
- Models with insufficient observations → maintained exploration weight (the UCB1 exploration bonus ensures under-sampled models aren't permanently discarded)

This optimization happens entirely within `roko-learn/src/cascade_router.rs` and costs nothing (T0 computation, no LLM).

---

## Prompt Optimization

Delta analyzes which context sections led to better outcomes during recent tasks. The `ExperimentStore` in `.roko/learn/experiments.json` tracks A/B prompt experiments:

- Which prompt templates produced higher gate pass rates?
- Which context sections (Neuro entries, playbook rules, iteration memory) were present in successful ticks vs. failed ticks?
- Which section ordering produced the best results?

Findings are stored as prompt-optimization Engrams that the Composer uses in subsequent gamma ticks to improve context assembly.

---

## Meta-Cognition

The final delta activity is meta-cognition: "Am I getting better overall?" This is a high-level self-assessment that examines trends across multiple delta cycles:

- **Performance trajectory**: Is the gate pass rate improving over time?
- **Cost efficiency**: Is cost per successful outcome decreasing?
- **Knowledge growth**: Is the Neuro store growing in Consolidated + Persistent entries?
- **Calibration quality**: Is prediction accuracy improving?
- **Behavioral health**: Is the Daimon cycling through healthy states (Engaged, Exploring, Focused) or stuck in unhealthy ones (Struggling, Coasting)?

Meta-cognition produces a `MetaCognitionReport` Engram that theta can read on the next cycle. If the report indicates systemic issues (e.g., "accuracy declining over 3 delta cycles despite high work volume"), it triggers a higher-level intervention: possibly re-examining the entire approach, requesting human review, or switching to a fundamentally different strategy.

---

## Non-Blocking Architecture

Delta must never delay gamma. The implementation uses Tokio's cooperative cancellation:

```rust
/// Delta loop: runs as a separate tokio task.
/// Can be paused at any time when gamma work arrives.
async fn delta_loop(
    state: Arc<RwLock<AgentState>>,
    cancel: CancellationToken,
) {
    loop {
        // Wait for idle or trigger
        tokio::select! {
            _ = wait_for_delta_trigger(&state) => {},
            _ = cancel.cancelled() => break,
        }

        // Run the three-phase dream cycle
        let dream_state = DreamState::new();
        let result = tokio::select! {
            r = run_dream_cycle(&state, &dream_state) => r,
            _ = gamma_work_arrived(&state) => {
                // Pause dream, serialize state
                dream_state.serialize_to_disk().await?;
                state.write().await.emit_signal(CognitiveSignal::Pause);
                continue;  // Resume next idle period
            }
        };

        // Process results
        if let Ok(output) = result {
            apply_dream_output(&state, &output).await;
        }
    }
}
```

When a new task arrives during dreaming:
1. The `CognitiveSignal::Pause` is emitted.
2. The dream state (current phase, replayed episodes, generated hypotheses) is serialized to disk.
3. Gamma takes over immediately — zero latency impact.
4. When the agent goes idle again, the dream state is deserialized and the cycle resumes from where it left off.

---

## Cost Summary

| Delta Phase | Model Class | Duration | Cost |
|---|---|---|---|
| NREM Replay | Haiku-class (T1) | 8-15 min | $0.02-0.12 |
| REM Imagination | Sonnet-class (T1-T2) | 5-15 min | $0.05-0.20 |
| Integration/Staging | Pure computation (T0) | 5-10 min | $0.00 |
| Knowledge Promotion | Pure computation (T0) | 1-2 min | $0.00 |
| Playbook Compilation | Pure computation (T0) | 1-2 min | $0.00 |
| Routing Optimization | Pure computation (T0) | <1 min | $0.00 |
| Meta-Cognition | Haiku-class (T1) | 1-2 min | $0.001-0.005 |
| **Total per Delta cycle** | | **~25-45 min** | **~$0.07-0.33** |

Impact on throughput: ~0% when tasks are available. Dreams only run when there is nothing else to do. The agent loses no productive time to consolidation.

---

## Academic Foundations

- **McClelland et al. 1995** — "Why there are complementary learning systems in the hippocampus and neocortex" (Psychological Review 102(3)). Fast episodic memory consolidates into slow semantic memory during sleep.
- **Mattar & Daw 2018** — "Prioritized memory access explains planning and hippocampal replay" (Nature Neuroscience 21). Utility-based prioritization for replay.
- **Boden 2004** — "The Creative Mind: Myths and Mechanisms" (2nd ed., Routledge). Three creativity modes: combinational, exploratory, transformational.
- **Pearl 2009** — "Causality: Models, Reasoning, and Inference" (2nd ed., Cambridge University Press). Structural causal models for counterfactual generation.
- **Walker & van der Helm 2009** — "Overnight therapy?" (Psychological Bulletin 135(5)). Emotional depotentiation during REM sleep.
- **Kanerva 2009** — "Hyperdimensional computing" (Cognitive Computation 1(2)). HDC vectors for knowledge recombination.
- **Ebbinghaus 1885** — "Über das Gedächtnis" (On Memory). Forgetting curves and spacing effects for replay scheduling.
- **Wilson & McNaughton 1994** — "Reactivation of hippocampal ensemble memories during sleep" (Science 265). Neural replay during sleep consolidation.
- **Buzsáki 2006** — "Rhythms of the Brain" (Oxford University Press). Delta oscillations (0.5-4 Hz) and sleep consolidation.
- **Lacaux et al. 2021** — "Sleep onset is a creative sweet spot" (Science Advances 7(50)). N1 hypnagogia and creative insight.

---

## Current Status and Gaps

**What exists:**
- Episode logging to `.roko/episodes.jsonl` provides raw material for replay.
- `PatternMiner` in `roko-learn` discovers trigram patterns.
- `KMedoids` clustering in `roko-learn` groups episodes.
- `CascadeRouter` persistence enables routing optimization.
- `ExperimentStore` tracks prompt A/B experiments.
- `roko-dreams` crate exists as a scaffold (not yet implemented).

**What is missing (Implementation Plan 12a §I3):**
- **I3**: Delta loop triggering dreams, playbook compilation, meta-cognition.
- NREM replay with Mattar-Daw utility formula.
- REM imagination with Boden's creativity modes.
- Integration staging buffer with 0.20-0.30 confidence threshold.
- Knowledge tier bulk promotion during delta.
- Emotional depotentiation mechanism.
- HDC counterfactual synthesis.
- Non-blocking dream pause/resume with state serialization.
- Meta-cognition assessment report.
- `roko dream run` CLI command wiring.

---

## Cross-References

- See [03-three-cognitive-speeds.md](./03-three-cognitive-speeds.md) for the three-speed hierarchy
- See [05-theta-reflective-loop.md](./05-theta-reflective-loop.md) for the reflective loop that feeds delta
- See [07-adaptive-clock.md](./07-adaptive-clock.md) for the clock managing delta scheduling
- See topic [10-dreams](../10-dreams/INDEX.md) for the full dream engine specification
- See topic [06-neuro](../06-neuro/INDEX.md) for knowledge tiers and tier progression
- See topic [09-daimon](../09-daimon/INDEX.md) for affect state and emotional depotentiation
- See topic [05-learning](../05-learning/INDEX.md) for episodes, playbooks, and pattern mining
