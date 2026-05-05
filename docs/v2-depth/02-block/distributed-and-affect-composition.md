# Distributed and Affect-Modulated Composition

> Depth for [02-CELL.md](../../unified/02-CELL.md). How multi-agent context engineering scales the Compose protocol across agent collectives, and how the Daimon's PAD affect state acts as an endofunctor that biases what Signals the Compose Cell assembles.

---

## Overview

Two problems sit at the boundary of the Compose protocol:

1. **Distributed context engineering**: when multiple agents work in parallel on a plan, how do you manage context across them? What each agent sees must be coordinated without creating shared mutable state.
2. **Affect-modulated retrieval**: the Daimon's PAD (Pleasure-Arousal-Dominance) vector biases which Signals are surfaced and scored. An anxious agent automatically receives cautionary context; a confident agent receives exploratory context.

Both are expressed as the same structural pattern: a **Functor** (see [00-INDEX.md](../../unified/00-INDEX.md) "Functor"). An endofunctor `F: Signal -> Signal` enriches or biases Signals before or after the Compose Cell runs, without changing the Compose Graph's topology. The Daimon's affect is one such functor; distributed context strategies are others.

---

## Part I: Distributed Context Engineering

### 1. The Four Strategies

Andrej Karpathy (2025) articulated context engineering as the real skill in building LLM applications. The framework defines four fundamental operations that form a complete basis for context management at any scale:

| Strategy | Definition | Roko Expression |
|---|---|---|
| **Write** | Generate context that does not yet exist | Enrichment pipeline creates 13 artifact types. Knowledge Store distills episodes into insights. SystemPromptBuilder generates affect guidance from PAD state. Each of these is a Cell with Compose or Store protocol. |
| **Select** | Choose which existing Signals to include | EFE Score Cell ranks candidates (see [active-inference-context-selection.md](active-inference-context-selection.md)). VCG auctioneer allocates budget (see [vcg-attention-auction.md](vcg-attention-auction.md)). MVT Route Cell decides when to stop searching. |
| **Compress** | Reduce Signal size preserving semantics | ContextAssembler summarizes lower-ranked chunks. History compaction summarizes old turns. PromptBudget truncates low-priority sections. Each is a Compose Cell that maps Signal -> smaller Signal. |
| **Isolate** | Separate context into non-interfering channels | Each agent session starts cold — no shared conversation history. Cache layers separate stable prefix from volatile suffix. Git worktrees isolate filesystem views. Each is a Space boundary. |

Select is highest-leverage: including the wrong 1,000 tokens is worse than no context at all (Joren et al., ICLR 2025). The empirical data is unambiguous.

### 2. Three Levels of Context Engineering

| Level | Scope | Compose Graph Topology |
|---|---|---|
| **Level 1: Local** | Single agent, single task | Pipeline Graph: [Query] -> [Score] -> [Dedup] -> [Budget] -> [Format]. All Cells local to one agent Space. |
| **Level 2: Allocation** | Multiple agents, same plan | Compose Graph adds shared prefix Cell: byte-identical plan context across agents. Role-specific budget routing. Cross-agent iteration memory (gate errors from Agent A inform Agent B's prompt). |
| **Level 3: Network** | Agent collectives sharing knowledge mesh | Stigmergic knowledge accumulation via Bus Pulses. VCG auction across collective. HDC-based retrieval from shared Store. Knowledge distillation chain: episodes -> insights -> heuristics -> playbooks. |

Level 1 is where the system operates today. Level 2 is partially wired (SharedPlanContext, RoleSystemPromptSpec). Level 3 is the target architecture.

### 3. Level 2: Allocation Context Engineering

When 5-20 agents execute a plan in parallel, the orchestrator manages their context as a set of Compose Graphs that share some Cells:

```
Shared prefix (byte-identical plan context)
    |
    +---[Agent A: Implementer]
    |     Role-specific budget: 8K file_context, 1K episodes
    |     Compose Graph: full pipeline with code-heavy bias
    |
    +---[Agent B: Reviewer]
    |     Role-specific budget: 2K file_context, 4K episodes
    |     Compose Graph: full pipeline with episode-heavy bias
    |
    +---[Agent C: Architect]
          Role-specific budget: 4K file_context, 2K research
          Compose Graph: full pipeline with research-heavy bias
```

**Shared Plan Context** is a Compose Cell that produces the identical prefix for all agents. It contains plan metadata, task DAG structure, and shared conventions. This prefix is cache-aligned (see 02-CELL.md CacheLayer) so the LLM provider caches it once across all agent calls in the same plan.

**Cross-Agent Iteration Memory**: when Agent A fails a gate and Agent B picks up a related task, Agent A's failure context (gate errors, prior attempt summary) is written as a Signal to Store and injected into Agent B's prompt. There is no message passing — the Signal is written to disk (Store) and read by the ContextAssembler (Store protocol). This is the "write-for-amnesia" principle: all context is explicit, inspectable, and reproducible.

### 4. The Write-for-Amnesia Principle

Every agent session starts cold. No conversation history. No shared memory. No implicit context. The files on disk are the only truth.

This is an **Isolate** strategy with profound implications:

1. **All context must be explicit.** The agent cannot "remember" what a previous agent did. If needed, it must be written to Store and injected into the prompt.
2. **Enrichment is pre-computation.** The enrichment pipeline creates artifacts BEFORE the agent session starts. The agent reads Signals, not memories.
3. **Iteration memory is structured.** Retry context (gate errors, prior attempt summary) is explicitly written as Signals and injected. The agent does not "recall" failure — it reads about it.
4. **Cross-agent communication is Signal-based.** Agent A's output is a Signal in Store. Agent B's Compose Graph includes Agent A's output by querying Store. No message passing, no shared state.

This makes the system fully inspectable: if an agent produces bad output, you can read its input Signals and see exactly what it saw.

### 5. Evaluation: Meta-Harness Evidence

Lee et al. (2026, arXiv:2603.28052) evaluated coding agents across scaffolds:
- **6x performance gap** from scaffold changes alone (same model)
- **4x fewer input tokens** in the best scaffolds
- Scaffold diversity matters — no single scaffold dominates

This validates the premise: investing in better context engineering (the Compose protocol) produces more improvement than upgrading to a more expensive model.

The CLEAR framework (2025) adds: optimizing for efficacy alone produces systems 4.4-10.8x more expensive than co-optimizing for cost and efficacy. The four strategies naturally co-optimize: Select reduces both cost and noise, Compress reduces cost while preserving quality, Isolate improves reliability, Write invests cost where return is highest.

---

## Part II: Affect-Modulated Retrieval

### 6. The PAD Model

Albert Mehrabian (1996) defined PAD as a three-dimensional emotional space:

```rust
/// Daimon affect state. Three dimensions, each in [-1.0, 1.0].
/// Persisted to .roko/daimon/affect.json and decayed over time.
struct PadState {
    /// Positive: task success, gate passes.
    /// Negative: failures, rejections.
    pleasure: f64,
    /// Positive: time pressure, urgency.
    /// Negative: idle, exploratory mode.
    arousal: f64,
    /// Positive: high confidence, autonomous.
    /// Negative: low confidence, seeking guidance.
    dominance: f64,
}
```

Why PAD and not simple sentiment? PAD captures motivational state:
- A simple positive/negative model treats "confident and exploring" (+P -A +D) the same as "excited and rushing" (+P +A +D). Both are positive. PAD distinguishes them: the first favors novel context, the second favors concise action-oriented context.

### 7. PAD Octants and Context Bias

The three dimensions define eight behavioral states:

| Octant | P | A | D | State | Context Bias |
|---|---|---|---|---|---|
| +P +A +D | + | + | + | Excited | Action-oriented, recent, concise |
| +P +A -D | + | + | - | Surprised | Directive, structured guidance |
| +P -A +D | + | - | + | Confident | Exploratory, novel, cross-domain |
| +P -A -D | + | - | - | Calm | Comprehensive, thorough |
| -P +A +D | - | + | + | Angry | Focused, targeted at error source |
| -P +A -D | - | + | - | Anxious | Cautionary, anti-patterns, warnings |
| -P -A +D | - | - | + | Bored | Stimulating, diverse, challenging |
| -P -A -D | - | - | - | Sad | Supportive, past successes, proven patterns |

### 8. The Daimon as Endofunctor

The affect modulation is a **Functor** pattern: an endofunctor `F: Signal -> Signal` that enriches Signals before the Compose Cell processes them. It does not change the Graph topology — it transforms the data flowing through it.

```rust
/// Functor Cell: modulates Signal scores based on PAD affect state.
/// Sits in the Compose Graph between Score Cell and Budget Cell.
/// Transforms scored Signals: F(Signal) -> Signal with biased scores.
struct AffectFunctor {
    pad: PadState,
}

impl AffectFunctor {
    fn apply(&self, signal: &mut Signal) {
        let scores = &mut signal.scores;

        // Arousal modulation
        if self.pad.arousal >= 0.35 {
            // High arousal: boost recent, action-oriented content
            match signal.kind {
                SignalKind::TaskDescription | SignalKind::CodeContext
                | SignalKind::GateError => {
                    scores.utility *= 1.3;
                }
                SignalKind::Research | SignalKind::CrossPlanContext => {
                    scores.utility *= 0.7;
                }
                _ => {}
            }
        } else if self.pad.arousal <= -0.35 {
            // Low arousal: boost novel, exploratory content
            scores.novelty *= 1.5;
        }

        // Pleasure modulation
        if self.pad.pleasure <= -0.35 {
            // Low pleasure: boost anti-patterns and warnings
            if matches!(signal.kind, SignalKind::AntiKnowledge | SignalKind::Warning) {
                scores.utility *= 1.5;
            }
        }

        // Dominance modulation
        if self.pad.dominance <= -0.35 {
            // Low dominance: boost explanatory content
            if matches!(signal.kind, SignalKind::ArchitectureDoc | SignalKind::Overview) {
                scores.utility *= 1.2;
            }
        }
    }
}
```

### 9. Compose Graph with Affect Functor

The endofunctor inserts between scoring and budgeting:

```
[QueryCell] -> [EfeScoreCell] -> [AffectFunctor] -> [DedupCell] -> [BudgetCell] -> [FormatCell]
                                       ^
                                       |
                                  [PadState from Daimon]
```

The AffectFunctor is a **natural transformation** (see [26-CROSS-CUTS.md](../../unified/26-CROSS-CUTS.md)) — it commutes with other functors. Applying affect modulation before or after deduplication produces the same final ranking. This is because the functor only scales scores, and scaling preserves sort order relative to the dedup threshold.

### 10. Appraisal Triggers — What Updates PAD

PAD is updated by events that change the agent's "emotional" state:

| Event | Pleasure | Arousal | Dominance |
|---|---|---|---|
| Gate pass (first attempt) | +0.2 | -0.1 | +0.1 |
| Gate pass (after retry) | +0.1 | -0.05 | +0.05 |
| Gate failure | -0.2 | +0.15 | -0.1 |
| Consecutive failures (3+) | -0.3 | +0.3 | -0.2 |
| Task completed under budget | +0.15 | -0.1 | +0.15 |
| Task exceeded budget | -0.1 | +0.2 | -0.1 |
| Approaching deadline | 0 | +0.25 | -0.05 |
| Idle (no active tasks) | 0 | -0.2 | 0 |

Each trigger is a **React Cell** — it watches for Pulses (gate verdicts, budget events, deadline signals) on Bus and updates the PadState Signal in Store.

### 11. Decay Toward Baseline

PAD decays toward neutral [0, 0, 0] with a configurable half-life:

```
pad(t) = pad(t-1) * exp(-ln(2) / half_life * dt)
```

Default half-life: 30 minutes. After 30 minutes without new appraisal events, PAD is halved. After ~2 hours, it is approximately zero.

This prevents permanent affect drift: a series of failures creates temporary anxiety that naturally dissipates. Without decay, cumulative negative events would push the agent into permanent pessimism.

### 12. Connection to Somatic Markers

Antonio Damasio (1994) proposed that emotional reactions (somatic markers) guide decision-making by rapidly narrowing the choice field before conscious reasoning. The PAD-modulated retrieval implements a computational analog: before the agent reasons about context (the LLM generation), the affect state has already biased retrieval scores.

This is not a metaphor — it is a functional equivalence. Somatic markers reduce the combinatorial explosion of decision-making by pruning options before deliberation. The Affect Functor reduces the combinatorial explosion of context selection by biasing scores before assembly.

Doya (2002) mapped biological neuromodulators to computational meta-parameters: dopamine -> learning rate, serotonin -> time horizon, noradrenaline -> exploration, acetylcholine -> uncertainty. The PAD dimensions map similarly: pleasure ~ dopamine (reward signal), arousal ~ noradrenaline (action urgency), dominance ~ serotonin (planning horizon).

---

## Behavioral Examples

### Example 1: Anxious Agent (Low P, High A)

Scenario: Three consecutive gate failures on a cross-crate integration task.

PAD: pleasure = -0.45, arousal = 0.50, dominance = -0.25

Effect on Compose Graph output:
- Anti-pattern Signals boosted x1.5 -- common failure modes surface prominently
- Warning Signals prioritized -- "this import path changed in v3"
- Gate error Signals placed at End (high-attention recency position)
- Affect guidance injected by SystemPromptBuilder: "Recent attempts have had issues. Be extra careful."
- CascadeRouter may prefer a more capable model (affect influences Route Cell)

### Example 2: Confident Explorer (High P, Low A)

Scenario: Five consecutive gate passes, no time pressure, idle period.

PAD: pleasure = 0.55, arousal = -0.40, dominance = 0.35

Effect on Compose Graph output:
- Novel content boosted x1.5 -- cross-domain insights, research memos surfaced
- Exploratory guidance -- "Consider multiple approaches before committing"
- Research Signals included (normally medium priority, now boosted)
- Cross-plan context included -- broader architectural awareness
- Exploration-friendly model selection -- may allow more creative approaches

### Example 3: Urgent Executor (Neutral P, High A)

Scenario: Approaching deadline, many tasks remaining.

PAD: pleasure = 0.0, arousal = 0.60, dominance = 0.10

Effect on Compose Graph output:
- Recent Signals boosted x1.5 -- most recent files and gate results
- Action-oriented content prioritized -- task brief, file context, acceptance criteria
- Research Signals suppressed -- no time for exploration
- Affect guidance: "You are under time pressure. Focus on the most impactful changes first."
- CascadeRouter prefers speed over depth

---

## Mori-Diffs Reality

**PadState struct exists** in `crates/roko-compose/src/context_assembler.rs`. Arousal and pleasure modulation are implemented in the `score_chunk` function. The struct is wired as optional into the ContextAssembler.

**Affect guidance in SystemPromptBuilder is implemented** for arousal and pleasure thresholds.

**Not yet implemented**:
- Dominance modulation (PadState has the field; no scoring logic)
- PAD persistence to `.roko/daimon/affect.json` (designed, not wired)
- PAD decay over time (designed, not wired)
- Appraisal triggers (event -> PAD update not wired)
- CascadeRouter affect integration (F8 in implementation plan)

**Distributed context engineering**:
- Level 1 (local) is fully implemented
- Level 2 (allocation) is partially implemented (SharedPlanContext, RoleSystemPromptSpec)
- Level 3 (network) is scaffold only

---

## What This Enables

- **Adaptive context personality** — the same Compose Graph produces different prompts for the same task depending on the agent's recent history. Failures trigger caution; successes enable exploration.
- **Principled isolation** — write-for-amnesia makes every agent session inspectable and reproducible. No hidden state.
- **Scalable multi-agent context** — shared prefixes enable KV cache sharing across agents; role-specific budgets ensure each agent sees what matters for its role.
- **Emotional intelligence analog** — Damasio's somatic markers, implemented computationally, give agents a form of intuition: past failures bias current perception toward caution without explicit rules.

## Feedback Loops

1. **Affect Calibration Loop**: `gate Verdict -> appraisal trigger -> PAD update -> Affect Functor biases retrieval -> agent behavior changes -> next gate Verdict` (Loop pattern)
2. **Affect Decay Loop**: `time passes without events -> PAD decays toward neutral -> less modulation -> default behavior` (continuous decay via timestamp)
3. **Cross-Agent Learning Loop**: `Agent A gate failure -> failure Signal to Store -> Agent B's ContextAssembler retrieves it -> Agent B avoids same mistake` (Level 2 allocation)
4. **Stigmergic Loop**: `Agent A success -> retrieval Pulse on Bus -> Agent B's social foraging boost -> better context for Agent B` (Level 3 network)

## Open Questions

1. **Affect interference with VCG truthfulness**: The Affect Functor biases bids before the auctioneer sees them. Does this compromise VCG's truthfulness guarantee? (Likely yes for strict theory; likely irrelevant in practice since all bidders are software modules, not strategic agents.)
2. **PAD calibration**: The appraisal trigger magnitudes (+0.2, -0.3, etc.) are hand-tuned. Should they be learned from outcome data?
3. **Dominance dimension utility**: Pleasure and arousal have clear behavioral effects. What concrete context-selection decisions should dominance influence beyond explanatory/directive bias?
4. **Level 3 cold start**: When a new agent joins a collective, it has no social foraging signal. How long until social information becomes useful? Is there a bootstrap mechanism?
5. **Affect persistence across plans**: Should PAD state carry over from one `roko plan run` to the next? Current design says yes (persistence file), but consecutive plans may have unrelated tasks where prior affect is misleading.

---

## References

- Karpathy (2025), Context Engineering
- Lee et al. (2026), Meta-Harness: Evaluating Coding Agents Across Scaffolds, arXiv:2603.28052
- CLEAR Framework (2025), Cost/Latency/Efficacy/Assurance/Reliability
- Shahul Es et al. (2024), RAGAS, EACL
- Joren et al. (2025), Sufficient Context, ICLR
- Mehrabian (1996), Pleasure-Arousal-Dominance Model
- Damasio (1994), Descartes' Error: Somatic Marker Hypothesis
- Doya (2002), Metalearning and Neuromodulation
- Plutchik (1980), Emotion: A Psychoevolutionary Synthesis
- Friston (2022), Free Energy Principle — Active Inference + Affect
- Zaharia et al. (2024), The Shift to Compound AI Systems, BAIR
- Contextual Influence Value (2025), Shanghai Jiao Tong University
