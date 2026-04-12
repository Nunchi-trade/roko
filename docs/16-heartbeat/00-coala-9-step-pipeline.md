# CoALA 9-Step Cognitive Pipeline

> The Cognitive Architectures for Language Agents (CoALA) framework provides the organizing taxonomy for every agent's decision cycle in Roko.


> **Implementation**: Specified

**Topic**: [16-heartbeat](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture fundamentals
**Key sources**: Sumers et al. 2023 (arXiv:2309.02427), legacy `bardo-backup/prd/01-golem/02-heartbeat.md`, `refactoring-prd/01-synapse-architecture.md` §3

---

## Abstract

Every Roko agent — coding, chain, research, operations, or custom — executes the same fundamental decision cycle on every tick of its cognitive clock. This cycle is not a conversation turn. It is not a request-response pair. It is a continuous, autonomous loop of observe-decide-act-learn that runs whether or not a human is present.

The organizing framework for this loop is CoALA (Cognitive Architectures for Language Agents), proposed by Sumers, Yao, Narasimhan, and Griffiths in 2023 (arXiv:2309.02427). CoALA draws on decades of cognitive architecture research — Soar (Laird 2012), ACT-R (Anderson 2007), CLARION (Sun et al. 2005) — to formalize what a language agent IS and how its decision cycle should be structured.

Roko adopts the CoALA framework as its primary organizing taxonomy because it provides a rigorous, research-grounded structure for agent cognition that maps cleanly onto the Synapse Architecture's six composable traits. The 9-step pipeline described in this document is the concrete realization of CoALA within Roko's trait-based composition system.

This document describes the 9 steps of the CoALA-derived pipeline as implemented in Roko, explains the academic predecessors that influenced the design, and articulates why CoALA was chosen as the organizing framework over alternatives.

---

## Why Decision Cycles, Not Conversation Turns

### The Conversation Assumption Is Wrong

Most agent frameworks model cognition as a conversation: user says something, agent thinks, agent responds. This model descends from chatbot heritage — LLMs were first deployed as conversational interfaces, and agent frameworks inherited that frame.

For Roko agents, this model is wrong on its face. **80% of heartbeat ticks have no human input.** The agent fires its heartbeat, observes its environment (codebase state, market conditions, research corpus), evaluates whether anything interesting happened, and either acts or moves on. There is no "user message." There is no "response." There is a continuous loop of observe-decide-act-learn running autonomously.

The distinction matters architecturally:

| Aspect | Conversation Turn | Decision Cycle |
|--------|------------------|----------------|
| Trigger | User message | Timer (heartbeat interval) |
| Input | Natural language text | Structured observation (build status, market data, research signal) |
| Output | Natural language response | Typed `DecisionCycleRecord` (structured, self-contained) |
| Storage | Append to growing session | Self-contained record per tick |
| Replay | Parse message history | Structured fields, zero parsing needed |
| Credit assignment | Parse LLM output text | Read outcome + context fields directly |
| Cost | Every turn costs money | ~80% of ticks cost $0.00 (T0 suppression) |

### CoALA Formalizes the Correct Primitive

Sumers et al. (2023, arXiv:2309.02427) proposed CoALA to formalize what a language agent IS, drawing on decades of cognitive architecture research:

> "The agent's decision procedure executes a decision cycle in a loop with the external environment. During each cycle, the agent uses retrieval and reasoning to plan by proposing and evaluating candidate learning or grounding actions. The best action is then selected and executed. An observation may be made, and the cycle begins again."

This maps precisely to Roko's heartbeat. Each tick IS a decision cycle: observe, retrieve from memory, reason about what was observed, decide on an action (or decide to do nothing), execute, observe the outcome, learn.

---

## Academic Predecessors

CoALA does not arise in a vacuum. It synthesizes insights from four major cognitive architecture traditions, each contributing specific mechanisms that appear in Roko's implementation.

### Soar (Laird, Newell, Rosenbloom 1987; Laird 2012)

Soar introduced the concept of a **production system** with a working memory (analogous to Roko's context window), long-term memory (analogous to Neuro knowledge store), and a decision cycle that selects operators based on preferences. Key contributions to Roko:

- **Impasse-driven subgoaling**: When Soar cannot resolve a decision (impasse), it creates a subgoal to resolve the impasse. Roko's tier escalation (T0 → T1 → T2) implements this principle — when deterministic probes cannot resolve the situation (impasse at T0), the agent escalates to analytical reasoning (T1) or deep deliberation (T2).
- **Chunking**: Soar learns new production rules from subgoal resolution. Roko's playbook rule extraction and skill library serve the same function — successful reasoning patterns are compiled into reusable artifacts.
- **Universal subgoaling**: Newell's "Unified Theories of Cognition" (1990) argued that a single architecture should handle all cognitive tasks. Roko's universal loop embodies this — one loop, parameterized by domain, handles all agent types.

### ACT-R (Anderson 1993, 2007)

ACT-R (Adaptive Control of Thought — Rational) provides a modular cognitive architecture with distinct declarative and procedural memory systems. Key contributions to Roko:

- **Activation-based retrieval**: ACT-R retrieves memories based on activation levels that decay with time and increase with use. Roko's Ebbinghaus decay curves in Neuro implement precisely this mechanism — knowledge that is used successfully gains activation (tier promotion), while unused knowledge decays.
- **Rational analysis**: Anderson's rational analysis principle states that cognitive mechanisms should be understood as optimal adaptations to the statistical structure of the environment. Roko's active inference framework (Friston's Free Energy Principle) formalizes this same insight — the agent's behavior should minimize surprise given its model of the environment.
- **Declarative/procedural distinction**: ACT-R separates what you know (declarative) from how you act (procedural). Roko separates Neuro (declarative knowledge: Insights, Heuristics, Warnings) from the Skill Library (procedural knowledge: reusable action sequences).

### CLARION (Sun et al. 2001, 2005)

CLARION (Connectionist Learning with Adaptive Rule Induction ON-line) provides a dual-process architecture with explicit (top-down) and implicit (bottom-up) processing. This is the direct ancestor of Roko's dual-process T0/T1/T2 system:

- **Dual-process cognition**: CLARION's bottom-up implicit processing maps to T0 (fast, heuristic, no LLM). Its top-down explicit processing maps to T2 (slow, deliberate, full LLM). T1 is the intermediate tier that CLARION does not explicitly model.
- **Bottom-up learning**: CLARION extracts explicit rules from implicit knowledge through a process called "rule extraction." Roko's dream consolidation and playbook compilation perform the same function — implicit patterns learned through experience are extracted into explicit heuristics.
- **Motivation subsystem**: CLARION includes a motivational subsystem that modulates cognitive processing. Roko's Daimon (PAD affect engine) serves this role — emotional state modulates tier routing, context bidding, and risk tolerance.

### Global Workspace Theory (Baars 1988, 2005; Baddeley 2000)

Baars' Global Workspace Theory proposes that consciousness arises from a shared workspace where specialized processors compete for access. Baddeley's Working Memory Model refines this with a central executive, episodic buffer, visuospatial sketchpad, and phonological loop. Roko's implementation:

- **Central executive** → Context Governor (allocates attention tokens across subsystems)
- **Episodic buffer** → The assembled context window (integrates information from all sources)
- **Visuospatial sketchpad** → Current observations (environment state, build results, market data)
- **Phonological loop** → Playbook heuristics (rehearsed procedural knowledge — the agent's "inner speech")

The Cognitive Workspace paper (2025, arXiv:2508.13171) validated this approach computationally, demonstrating that active memory management with deliberate information curation achieves 58.6% memory reuse rate compared to 0% for traditional RAG, with 17-18% net efficiency gain.

---

## The 9-Step Pipeline

Each heartbeat tick executes a 9-step pipeline. The steps map to CoALA's decision cycle phases, extended with Roko-specific verification and meta-cognition:

```
Step 1: OBSERVE    — Read environment state, evaluate probes, detect regime changes
Step 2: RETRIEVE   — Pull relevant knowledge from Neuro using multi-factor scoring
Step 3: ANALYZE    — Compute prediction error (how surprising is this observation?)
Step 4: GATE       — Decide cognitive tier (T0: suppress, T1: analyze, T2: deliberate)
Step 5: SIMULATE   — [Domain-specific] Pre-flight verification (mirage-rs for chain, dry-run for code)
Step 6: VALIDATE   — [T1/T2] Check safety constraints, position limits, capability tokens
Step 7: EXECUTE    — [If validated] Execute tool calls with capability authorization
Step 8: VERIFY     — [If acted] Ground truth verification (compiler, test suite, blockchain receipt)
Step 9: REFLECT    — Build DecisionCycleRecord, update affect, fire learning hooks
```

Steps 5-8 are conditional: in a T0 tick (no LLM call), only steps 1-4 and 9 execute. This is what makes ~80% of ticks nearly free ($0.00 inference cost).

### Step 1: OBSERVE

Read the current environment state through deterministic probes. For a coding agent, this means checking build status, test results, complexity metrics, and coverage deltas. For a chain agent, this means reading prices, liquidity, position health, and gas costs. For any agent, the 16 T0 probes (see [09-16-t0-probes.md](./09-16-t0-probes.md)) run at every tick.

Each probe is a pure function: `fn probe(state: &EngineState) -> f32`. No LLM call, no network I/O, no blocking. The observation is the agent's "peripheral vision" — always running, always cheap.

**Synapse trait**: `Substrate.query()` — fetch current state from storage.
**Layer**: L0 Runtime.

### Step 2: RETRIEVE

Pull relevant knowledge from the Neuro store (formerly Grimoire) using multi-factor scoring:

```
score = w_recency × recency(Ebbinghaus_decay)
      + w_importance × quality(confidence × validation_ratio)
      + w_relevance × similarity(query, entry)
      + w_emotional × PAD_cosine(current_mood, entry_affect)
```

The last factor — emotional congruence — implements Bower's (1981) mood-congruent memory: the agent's current emotional state biases which memories surface. An anxious agent retrieves warnings and past failures; a confident agent retrieves successes and validated heuristics. This is not a bug — it is how biological memory works, and it is computationally efficient. Every 100 ticks, mandatory 15% contrarian retrieval forces mood-OPPOSITE entries to prevent rumination.

**Synapse trait**: `Scorer.score()` — evaluate relevance of each retrieved Engram.
**Layer**: L2 Scaffold.

### Step 3: ANALYZE

Compute prediction error: how SURPRISING is this observation compared to what the agent expected? This is the core signal that drives the System 1 / System 2 gating decision in Step 4.

Prediction error is a scalar in [0.0, 1.0] that aggregates weighted sources of surprise. The exact sources depend on the domain:

- **Coding domain**: Build health delta, test regression, complexity drift, coverage change, error rate trend, world model drift, causal consistency.
- **Chain domain**: Price divergence from causal model, regime change detection, position health delta, pheromone field threat intensity, pending interventions, probe anomaly count.
- **Universal**: World model drift (predicted vs. actual state divergence), causal consistency (lineage DAG integrity).

Friston's free-energy principle (2010) provides the theoretical foundation: the brain continuously generates predictions about incoming sensory data and computes the discrepancy — the prediction error — between expected and observed. Large prediction errors signal novelty, danger, or opportunity and demand attention. Small prediction errors mean the environment matches expectations and no additional processing is needed.

**Synapse trait**: Part of `Scorer.score()` and Daimon cross-cut.
**Layer**: L2 Scaffold + Cognitive cross-cut.

### Step 4: GATE

The gating step implements Kahneman's (2011) dual-process theory via Friston's (2010) precision-weighted prediction error framework. The question: "Is this observation surprising enough to warrant expensive LLM deliberation, or can I handle it with cheap heuristics?"

```
error < 0.2  → T0 (suppress, no LLM)     ~80% of ticks
error < 0.6  → T1 (fast model, shallow)   ~15% of ticks
error ≥ 0.6  → T2 (full model, deep)      ~5% of ticks
```

The adaptive threshold considers:
- **Cognitive pressure**: Under budget pressure, the threshold lowers — the agent thinks harder about fewer things.
- **Arousal**: High arousal (surprise) lowers the threshold — the agent pays more attention.
- **Strategy confidence**: High confidence raises the threshold — the agent coasts on proven patterns.
- **Daimon behavioral state**: Struggling agents have a lower T2 trigger (escalate sooner). Coasting agents have a higher trigger (stay cheap longer).

**Synapse trait**: `Router.select()` — choose the cognitive tier.
**Layer**: L1 Framework.

### Step 5: SIMULATE (Domain-Specific)

Pre-flight verification before committing to action. This step exists because some actions are irreversible — you cannot undo a blockchain transaction or a deployed contract.

- **Chain agents**: Run proposed transactions in a local EVM fork via mirage-rs. Catches revert scenarios, unexpected gas costs, sandwich attack vulnerability.
- **Coding agents**: May run dry-run compilation or quick test suites before committing changes.
- **Research agents**: Typically skip this step (research actions are reversible).

This step does not exist in the universal loop — it is a domain-specific extension point between ATTEND and ACT. See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the full chain mapping.

### Step 6: VALIDATE

Check safety constraints. Every action passes through the safety layer before execution:

- Capability tokens (typed, unforgeable authorization — see topic [11-safety](../11-safety/INDEX.md))
- Position limits and approved asset lists (chain domain)
- File permission checks and scope limits (coding domain)
- Policy enforcement via `Policy.decide()` → permit/deny/modify/log

**Synapse trait**: `Gate.verify()` (safety pre-check).
**Layer**: L3 Harness.

### Step 7: EXECUTE

If validated, execute tool calls with capability authorization. The agent calls the LLM (for T1/T2 ticks), which produces tool invocations, which are executed through the tool dispatcher with full capability checking.

**Synapse trait**: `Agent.execute()` — call LLM backend, produce output.
**Layer**: L1 Framework.

### Step 8: VERIFY

Ground truth verification from external sources. This is NOT self-assessment — it is external truth:

- **Compiler**: Did it compile? Zero compiler errors?
- **Test suite**: Did tests pass? How many? Any regressions?
- **Blockchain**: Did the transaction succeed? What was the actual outcome vs. expected?
- **Linter**: Does the code meet quality standards?

This is where Roko differs fundamentally from frameworks that rely on self-evaluation. The Gate pipeline provides external ground truth that the agent cannot manipulate or self-rationalize.

**Synapse trait**: `Gate.verify()` — check against external ground truth.
**Layer**: L3 Harness.

### Step 9: REFLECT

Build the `DecisionCycleRecord` — a typed, self-contained record of everything that happened during this tick. Then fire the learning hooks:

- **Episode logging**: Record the full tick as an episode Engram in `.roko/episodes.jsonl`.
- **Daimon update**: Update the PAD affect vector based on outcome (success → pleasure increase; failure → pleasure decrease, arousal increase).
- **Neuro update**: Promote or demote knowledge tiers based on whether retrieved knowledge led to good or bad outcomes.
- **Router feedback**: Update bandit arms in the CascadeRouter based on which model tier succeeded or failed.
- **Prediction update**: Compare prediction (from Step 3) against actual outcome to calibrate the prediction engine.

**Synapse trait**: `Policy.decide()` — observe Engram streams, emit new Engrams. Plus `Substrate.put()` — persist output with lineage.
**Layer**: L3-L4 Harness/Orchestration + L0 Runtime (for persistence).

---

## The OODA Loop Mapping

Boyd's Observe-Orient-Decide-Act (OODA) loop maps directly onto the pipeline, with one critical addition: the REFLECT step closes the learning loop that OODA leaves open.

| OODA Phase | Pipeline Steps | What Happens |
|---|---|---|
| **Observe** | OBSERVE | Deterministic probes capture environment state |
| **Orient** | RETRIEVE + ANALYZE | Neuro retrieval + prediction error orient the agent |
| **Decide** | GATE + SIMULATE + VALIDATE | Adaptive threshold + pre-flight + safety → select action |
| **Act** | EXECUTE + VERIFY | Execute with capability tokens + ground truth verification |
| _(missing in OODA)_ | REFLECT | DecisionCycleRecord + learning hooks close the loop |

The REFLECT step is what makes Roko agents self-improving rather than merely reactive. Every tick that resolves — whether the agent acted or suppressed — produces data that improves future ticks. This is the cybernetic feedback loop that Boyd's OODA framework lacks.

---

## Why CoALA Over Alternatives

### ReAct (Yao et al. 2022)

ReAct interleaves reasoning and acting but has no explicit memory retrieval step, no gating mechanism, and no separation of cognitive tiers. It is a prompting strategy, not a cognitive architecture.

### Reflexion (Shinn et al. 2023)

Reflexion adds verbal self-reflection to trial-and-error. It provides the REFLECT step but lacks the structured OBSERVE-RETRIEVE-ANALYZE-GATE pipeline. It also uses self-assessment (LLM judges its own output) rather than external verification (compiler, tests, blockchain).

### AutoGPT / BabyAGI

Purely loop-based architectures without cognitive tiering, memory retrieval, or verification. Every iteration invokes the LLM, making them expensive and lacking the T0/T1/T2 cost optimization.

### Why CoALA Wins

CoALA provides the most complete mapping from cognitive science to agent implementation:

1. **Explicit decision cycle** (not conversation turns) — matches Roko's autonomous tick model.
2. **Memory retrieval as a first-class step** — matches Roko's Neuro integration.
3. **Learning from action outcomes** — matches Roko's Gate-based feedback loops.
4. **Grounding in established cognitive architectures** (Soar, ACT-R) — provides theoretical rigor rather than ad hoc design.
5. **Composable with active inference** — CoALA's prediction error mechanism maps directly to Friston's free energy minimization, enabling the T0/T1/T2 gating system that provides ~80% cost reduction.

---

## Cost Model

The 9-step pipeline with T0/T1/T2 gating produces a favorable cost distribution:

| Tier | Handler | Model | Cost/Call | Frequency | Trigger |
|------|---------|-------|-----------|-----------|---------|
| **T0** | Deterministic probes | None | $0.00 | ~80% of ticks | Prediction error < 0.2 |
| **T1** | Fast model | Haiku-class | $0.001–0.003 | ~15% of ticks | Prediction error in [0.2, 0.6) |
| **T2** | Full model | Sonnet/Opus-class | $0.01–0.25 | ~5% of ticks | Prediction error ≥ 0.6, or forced |

Without gating (every tick at T2): daily cost would be $100-500+ depending on tick frequency. With T0 suppression at ~80%: daily inference cost drops to ~$2-50. FrugalGPT (Chen et al. 2023, arXiv:2305.05176) demonstrated that cascade architectures can achieve up to 98% cost reduction while matching top-model quality. Roko's T0/T1/T2 system is a concrete realization of this principle.

---

## Academic Foundations

- **Sumers, Yao, Narasimhan & Griffiths 2023** — "Cognitive Architectures for Language Agents" (arXiv:2309.02427). The CoALA framework providing the organizing taxonomy.
- **Laird, Newell & Rosenbloom 1987** — "SOAR: An Architecture for General Intelligence" (Artificial Intelligence 33(1)). Production system with impasse-driven subgoaling.
- **Laird 2012** — "The Soar Cognitive Architecture" (MIT Press). Comprehensive Soar reference.
- **Anderson 1993, 2007** — "Rules of the Mind" (Erlbaum) and "How Can the Human Mind Occur in the Physical Universe?" (Oxford). ACT-R cognitive architecture.
- **Sun, Merrill & Peterson 2001** — "From Implicit Skills to Explicit Knowledge" (Cognitive Science 25(2)). CLARION dual-process architecture.
- **Sun 2005** — "The CLARION Cognitive Architecture" (The Cambridge Handbook of Computational Psychology). Comprehensive CLARION reference.
- **Newell 1990** — "Unified Theories of Cognition" (Harvard University Press). The argument for universal cognitive architectures.
- **Baars 1988** — "A Cognitive Theory of Consciousness" (Cambridge University Press). Global Workspace Theory.
- **Baddeley 2000** — "The Episodic Buffer: A New Component of Working Memory?" (Trends in Cognitive Sciences 4(11)). Working memory model.
- **Kahneman 2011** — "Thinking, Fast and Slow" (Farrar, Straus and Giroux). Dual-process theory (System 1 / System 2).
- **Friston 2010** — "The Free-Energy Principle: A Unified Brain Theory?" (Nature Reviews Neuroscience 11(2)). Precision-weighted prediction error.
- **Clark 2013** — "Whatever Next? Predictive Brains, Situated Agents, and the Future of Cognitive Science" (Behavioral and Brain Sciences 36(3)). Predictive processing framework.
- **Boyd 1986** — "Patterns of Conflict" (unpublished briefing). OODA Loop.
- **Chen et al. 2023** — "FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance" (arXiv:2305.05176). Cascade architecture cost reduction.
- **Cognitive Workspace 2025** — arXiv:2508.13171. Active memory management validation.
- **Bower 1981** — "Mood and Memory" (American Psychologist 36(2)). Mood-congruent memory retrieval.

---

## Current Status and Gaps

**What exists in the codebase today:**
- The orchestration loop in `roko-cli/src/orchestrate.rs` implements the basic Gamma-frequency tick cycle (Steps 1, 4-5, 7-9 in simplified form).
- `roko-learn/src/cascade_router.rs` implements the three-stage cascade routing (Static → Confidence → UCB1) that drives the T0/T1/T2 gating decision.
- `bardo-primitives/src/tier.rs` (to be renamed `roko-primitives`) defines the `InferenceTier` enum (T0/T1/T2) and the `TierRouter` for model selection.
- `roko-learn/src/episode_logger.rs` implements Step 9 (REFLECT) episode logging.
- `roko-gate` implements Step 8 (VERIFY) with 11 gate types.

**What is missing:**
- Formal OBSERVE step with the 16 T0 probes running at every tick (see implementation plan `12a-cognitive-layer.md` §I).
- Neuro-aware RETRIEVE step with multi-factor scoring (see `12a-cognitive-layer.md` §E).
- Daimon-modulated ANALYZE step with prediction error computation (see `12a-cognitive-layer.md` §F).
- Full `DecisionCycleRecord` struct capturing the complete tick state (currently, episodes capture partial information).

---

## Cross-References

- See [01-universal-loop-mapping.md](./01-universal-loop-mapping.md) for how CoALA maps to the Synapse universal loop
- See [02-chain-heartbeat-variant.md](./02-chain-heartbeat-variant.md) for the chain-specific SIMULATE/VALIDATE extension
- See [08-dual-process-t0-t1-t2.md](./08-dual-process-t0-t1-t2.md) for the full dual-process tier routing specification
- See [09-16-t0-probes.md](./09-16-t0-probes.md) for the 16 zero-cost deterministic probes
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture foundation
- See topic [02-agents](../02-agents/INDEX.md) for agent type compositions
- See topic [05-learning](../05-learning/INDEX.md) for the cybernetic feedback loops
