# Design Principles and Frontier Innovation Summary

> **Abstract:** Roko is guided by seven design principles (P1-P7) that constrain architectural
> decisions and prevent feature drift. Beyond these principles, fourteen frontier innovations
> — capabilities no competitor has — emerge naturally from the Synapse Architecture's
> composable trait system. This document enumerates the design principles, summarizes each
> frontier innovation, and shows how the innovations interconnect to form an autocatalytic
> improvement system.

**Topic**: [00-architecture](./INDEX.md)
**Prerequisites**: [00-vision-and-thesis](./00-vision-and-thesis.md), [06-synapse-traits](./06-synapse-traits.md), [12-five-layer-taxonomy](./12-five-layer-taxonomy.md)
**Key sources**:
- `/Users/will/dev/nunchi/roko/refactoring-prd/09-innovations.md` — All 14 frontier innovations
- `/Users/will/dev/nunchi/roko/refactoring-prd/00-overview.md` — Design principles, overview
- `/Users/will/dev/nunchi/roko/bardo-backup/prd/00-vision/05-manifesto.md` — Original manifesto (7 principles)

---

## Abstract

Design principles are the immune system of an architecture — they prevent bad decisions
before they are made. Roko's seven principles (P1 through P7) were extracted from the
patterns that made the original 108K-LOC Mori codebase succeed and the patterns that made
it fail. Each principle is a hard constraint: if a proposed change violates a principle, it
is rejected regardless of its other merits.

The fourteen frontier innovations are capabilities that no competing agent framework provides.
They are not aspirational features — each is a specific mechanism grounded in academic
research with a concrete implementation path through the existing Synapse Architecture. What
makes them "blue ocean" is that they emerge from architectural decisions that competitors
have not made: the trait-based composition system, the content-addressed Engram type, the
HDC encoding layer, the on-chain verification path, and the affect-driven cognitive
architecture.

This document serves as both a design guide (what principles govern decisions) and a
capability catalog (what Roko can do that nobody else can).

---

## 1. The Seven Design Principles

### P1: Composition Over Configuration

**Statement**: Every capability is a trait implementation. New behaviors emerge from
composing existing traits, not from configuration flags or feature gates.

**Rationale**: Configuration systems grow unboundedly — a dozen flags become a hundred,
interactions between flags become untestable, and users cannot reason about behavior.
Trait composition is bounded by the type system: if it compiles, the composition is valid.

**Application examples**:
- A new verification strategy is a new `Gate` implementation, not a `verification_strategy`
  config key.
- A new context assembly strategy is a new `Composer` implementation, not a
  `context_mode` flag.
- Domain-specific behavior (chain tools, coding tools) is a trait implementation plugged
  into the universal loop, not a `domain` configuration enum.

**Anti-pattern**: `if config.mode == "chain" { ... } else if config.mode == "coding" { ... }`.
This is a sign that domain behavior should be factored into a trait implementation.

### P2: Verify Everything

**Statement**: Every agent output passes through the Gate pipeline before being persisted
or acted upon. No unverified Engrams enter the audit DAG as trusted.

**Rationale**: LLMs hallucinate. Tools fail silently. External data sources lie. Without
systematic verification, errors compound through the lineage DAG. The Gate pipeline is the
firewall between "the agent produced something" and "the system trusts it."

**Application examples**:
- Code output passes through compile gate → test gate → clippy gate → diff gate before
  being accepted.
- Knowledge claims pass through confidence threshold → source verification → consistency
  check before being promoted from Transient to Working tier.
- Chain transactions pass through simulation gate (mirage-rs) → gas estimation → balance
  check before submission.

**Escape hatch**: Engrams can be marked with low confidence and stored as Transient tier
knowledge without full verification, allowing speculative hypotheses to exist in the system
while clearly flagged as unverified.

### P3: Budget-Aware by Default

**Statement**: Every operation has a budget (tokens, time, cost, signals). The system
degrades gracefully when budgets are exhausted, never crashes or produces unbounded output.

**Rationale**: LLM inference is expensive. Context windows are finite. Execution time has
deadlines. A system that does not respect budgets either exceeds cost limits or truncates
output arbitrarily. Roko embeds budget awareness into the core types (Query has a Budget;
Composer takes a Budget parameter).

**Application examples**:
- `Composer.compose()` takes a `Budget { max_tokens, max_signals, max_bytes, max_wall_ms }`
  and assembles context within those constraints.
- `CascadeRouter` selects the cheapest model tier sufficient for the current prediction
  error, not the most capable.
- The orchestrator tracks total cost across a plan execution and can halt when budget is
  exhausted.

### P4: Content-Addressed Everything

**Statement**: Every Engram has a BLAKE3 content hash computed from its identity fields
(kind, body, author, tainted, lineage, tags). The hash is the Engram's identity. Two
Engrams with identical content have identical hashes.

**Rationale**: Content addressing provides three properties simultaneously:
1. **Deduplication**: Identical content is automatically deduplicated.
2. **Integrity verification**: Any modification to an Engram changes its hash, making
   tampering detectable.
3. **Lineage verification**: The audit DAG is a hash chain — replaying the lineage
   verifies that no intermediate Engram was modified.

This is the foundation of the Forensic AI capability — causal replay of agent decisions
with cryptographic verification.

### P5: Decay as Feature, Not Bug

**Statement**: Engrams decay by default. Information that is not reinforced fades. This is
a feature that prevents knowledge hoarding and ensures the system's working memory stays
relevant.

**Rationale**: Without decay, the Substrate grows unboundedly. Old, irrelevant Engrams
dilute search results. The system cannot distinguish between current knowledge and historical
artifacts. Biological memory systems (Ebbinghaus 1885) demonstrate that forgetting is
essential for efficient recall.

**Application examples**:
- Threat pheromones (HalfLife: 2 hours) fade quickly because threats are time-sensitive.
- Working knowledge (Ebbinghaus with strength 0.5) persists for hours to days, long enough
  to be useful within a session.
- Persistent knowledge (strength 5.0×) has effective half-lives of weeks to months, but
  still eventually decays if not reinforced through use.
- `Decay::None` exists for permanent records (e.g., on-chain attestations) but is the
  exception, not the default.

### P6: Self-Model Is Mandatory

**Statement**: Every agent maintains a self-model (the Daimon PAD vector) that tracks its
own cognitive state. This self-model modulates behavior — it is not cosmetic.

**Rationale**: The Good Regulator Theorem (Conant & Ashby 1970) proves that every good
regulator must contain a model of the system it regulates. An agent without self-awareness
cannot adaptively allocate cognitive resources, cannot detect when it is struggling, and
cannot decide when to escalate to a stronger model.

**Application examples**:
- An agent with declining Pleasure (task failures) and rising Arousal (increasing urgency)
  transitions from Engaged to Struggling state, shortening the Theta reflection cycle.
- An agent with stable Pleasure and low Arousal transitions to Coasting state, extending
  Gamma runs and preferring T0/T1 tiers.
- Somatic markers (positive/negative valence from past experiences) bias Router selection
  before analytical reasoning engages.

### P7: Observable by Default

**Statement**: Every Engram, every trait invocation, every gate verdict, every tier
selection is observable. The system is transparent to its operators at every level of
abstraction.

**Rationale**: Opaque agent systems are undeployable in regulated environments and
undebuggable in development. Roko's content-addressed lineage DAG, provenance tracking,
and taint propagation provide full observability without requiring additional instrumentation.

**Application examples**:
- `roko replay <hash>` walks the lineage DAG backward from any Engram to its root inputs.
- Gate verdicts include the gate name, score, reason, test counts, and error digest.
- CascadeRouter logs every tier selection decision with confidence and cost.
- Daimon PAD vector changes are recorded as Engrams, creating a visible emotional history.

---

## 2. The Fourteen Frontier Innovations

These are capabilities that no competing agent framework provides. Each emerges from the
Synapse Architecture's composable design — they are not bolted on but grow naturally from
the architectural choices.

### 2.1 Summary Table

| # | Innovation | What It Does | Why Nobody Else Has It | Roko's Structural Advantage |
|---|---|---|---|---|
| 1 | **16 T0 Probes** | 80% of ticks cost zero LLM | Requires domain-agnostic probe framework | Extensible `Vec<Box<dyn Probe>>` |
| 2 | **VCG Attention Auction** | Optimal context allocation via truthful bidding | Requires multi-subsystem architecture | 6 traits = 6+ bidding subsystems |
| 3 | **Somatic Landscape** | Emotional fast-path decisions via k-d tree | Requires affect engine + spatial indexing | Daimon PAD + persistent somatic markers |
| 4 | **Hypnagogia Engine** | Solves Alpha Convergence Problem via creative divergence | Requires dream engine + episodic memory + HDC | Dreams + Neuro + HDC recombination |
| 5 | **31.6× Collective Calibration** | Network learning scales as sqrt(N) | Requires on-chain verified outcomes | Korai chain + Gate verdicts |
| 6 | **Predictive Foraging** | Self-correcting knowledge retrieval | Requires prediction tracking + external verification | Scorer + Gate feedback loop |
| 7 | **x402 Micropayments** | Self-funding agents | Requires chain + wallet + identity | Korai + ERC-8004 + roko-chain |
| 8 | **Forensic AI** | Cryptographic causal replay of agent decisions | Requires content-addressed lineage | Engram BLAKE3 + lineage DAG |
| 9 | **EvoSkills** | Self-evolving skill libraries via adversarial verification | Requires isolated verifier + cross-model transfer | Gate isolation + skill library |
| 10 | **ADAS** | Meta-agent architecture search — agents that design agents | Requires composable trait system | 6 traits as search dimensions |
| 11 | **Cognitive Kernel Primitives** | OS-level abstractions for agent cognition | Requires clean kernel abstraction | Synapse Architecture = cognitive kernel |
| 12 | **Cross-Domain Insight Resonance** | Structural analogy detection across domains in nanoseconds | Requires HDC + multi-domain operation | 10,240-bit HDC vectors + domain-agnostic core |
| 13 | **Generative Interfaces (A2UI)** | Agents create their own UI | Requires generative UI framework + design system | ROSEDUST design language + Spectre |
| 14 | **Knowledge Futures Market** | On-chain escrow for committed knowledge production | Requires chain + micropayments + verification | Korai + x402 + Gate pipeline |

### 2.2 Innovation Details

#### Innovation 1: 16 T0 Probes (Zero-Cost Cognitive Probes)

At every gamma tick (~5-15 seconds), 16 deterministic probes run with zero LLM cost. Each
probe is a pure function: `fn probe(state: &EngineState) -> f32`. Combined via weighted sum
into a prediction error scalar:

```
error < 0.2  → T0 (suppress, no LLM)     ~80% of ticks
error < 0.6  → T1 (fast model, shallow)   ~15% of ticks
error ≥ 0.6  → T2 (full model, deep)      ~5% of ticks
```

Probes are organized by domain:
- **Blockchain domain** (8 probes): Price delta, TVL delta, position health, gas spike,
  credit balance, RSI, MACD, circuit breaker.
- **Coding domain** (6 probes): Build health, test regression, complexity drift, dependency
  risk, coverage delta, error rate.
- **Universal** (2 probes): World model drift, causal consistency.

New domains add probes by implementing the probe function signature. The probe registry is
`Vec<Box<dyn Probe>>` — users compose whatever probe set matches their domain.

**Citation**: FrugalGPT (Chen et al. 2023, arXiv:2305.05176) demonstrated that cascade
architectures can achieve up to 98% cost reduction while matching top-model quality.

#### Innovation 2: VCG Attention Auction

When multiple subsystems compete for the limited context window, a Vickrey-Clarke-Groves
(VCG) auction allocates tokens optimally:

```
bid(section_i) = expected_value × urgency × affect_weight
```

Each winner pays the second-highest bid (VCG truthfulness guarantee). "Payment" is deducted
from the subsystem's attention budget for the next tick.

Eight subsystems bid: Neuro (knowledge entries), Daimon (affect context), iteration memory
(past failures), code intelligence (symbols, types), playbook rules (heuristics), research
artifacts (analyses), task context (PRD sections), and oracle predictions (calibration).

**Citation**: VCG mechanism (Vickrey 1961, Clarke 1971, Groves 1973). Applied to attention
allocation following the attention economics framework in context engineering.

#### Innovation 3: Somatic Landscape

Damasio's (1994) somatic marker hypothesis implemented as a k-d tree over an 8-dimensional
strategy space:

```rust
pub struct SomaticLandscape {
    tree: KdTree<f64, SomaticMarker, 8>,
}

pub struct SomaticMarker {
    pub strategy_coords: [f64; 8],
    pub valence: f64,           // -1 to +1
    pub intensity: f64,         // 0 to 1
    pub episodes: Vec<ContentHash>,
}
```

Before acting, the agent queries nearest neighbors: positive valence → confidence (T1),
negative valence → caution (T2). Mandatory 15% contrarian retrieval (Bower 1981) prevents
emotional echo chambers.

The 8 dimensions are domain-configurable. Coding agents default to: complexity, risk,
novelty, confidence, time_pressure, scope, reversibility, dependency_depth. Chain agents
substitute: volatility, exposure, liquidity, correlation, leverage, time_horizon,
slippage_risk, counterparty_risk.

#### Innovation 4: Hypnagogia Engine

Solves the Alpha Convergence Problem: all agents using the same LLM produce identical
analyses, so alpha converges to zero. The hypnagogia engine forces divergence through unique
episodic recombination — each agent is "differently haunted" (Derrida 1993) by its own
experiential traces.

Four components:

| Component | Role | Implementation |
|---|---|---|
| **Thalamic Gate** | Redirects attention inward | HDC anti-correlated retrieval (lowest similarity to recent episodes) |
| **Executive Loosener** | Relaxes constraints for creative association | Temperature annealing: T=1.3-1.5 ideation, T=0.3-0.5 evaluation |
| **Dali Interrupt** | Captures partial insights before convergence | Stop completions at 50-100 tokens, collect 3-5 fragments per hypothesis |
| **Homuncular Observer** | Filters noise from genuine insight | Structured evaluation: novelty > 0.5, relevance > 0.3, coherence > 0.4 |

Cost: ~2 LLM calls + 15-25 partial completions at ~50 tokens each ≈ 2,000-4,000 tokens
≈ $0.01 per hypnagogia session.

**Citations**: Lacaux et al. 2021, Science Advances 7(50) (83% success on hidden rule
discovery during N1). MIT Dormio (Haar Horowitz 2020/2023, 43% creativity boost).
Derrida 1993. Boden 2004, "The Creative Mind".

#### Innovation 5: 31.6× Collective Calibration

> **Caveat**: Nunchi-derived heuristic, not a published theorem. Requires empirical validation.

Solo: `accuracy(t) = 1 - 1/sqrt(t)`. Collective (N agents): `accuracy(t) = 1 - 1/sqrt(N×t)`.
At N=1,000: sqrt(1000) ≈ 31.6× faster calibration (theoretical upper bound, independence
assumed).

Mechanism: Every falsifiable prediction is verified by an external oracle (compiler, test
suite, chain state). Residuals feed a CalibrationTracker that aggregates per (model,
task_category). On Korai, all agents read the collective's calibration; new agents inherit it.

See [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) for full details.

#### Innovation 6: Predictive Foraging

Every knowledge retrieval is a falsifiable prediction: the agent predicts the outcome before
executing, then compares against the actual result. Residuals feed an arithmetic corrector
(~50 nanoseconds per correction, no LLM). Combined with Charnov's Marginal Value Theorem
(1976) for an optimal stopping rule on context retrieval.

**Citations**: Charnov 1976 (Marginal Value Theorem). Pirolli & Card 1999 (Information
Foraging Theory applied to information retrieval).

#### Innovation 7: x402 Micropayments

Integration of Coinbase's x402 protocol (now Linux Foundation, with AWS, Visa, Mastercard,
Stripe) for autonomous agent micropayments: per-API-call billing at <$0.001 per transaction,
sub-second USDC settlement on Base.

Economic cycle: Agent posts validated insight to Korai → earns KORAI → converts to USDC →
pays for inference → produces output → user pays agent → reinvests → cycle accelerates.

#### Innovation 8: Forensic AI (Causal Replay)

When an agent action causes harm, Roko can replay the exact decision context: which Engrams
were in the Substrate, which Scores were computed, which Router selected which candidate,
which Composer assembled context, which Gate verified output, which Policy fired. Every step
is content-addressed — replay is cryptographically verifiable.

Maps to regulatory requirements: EU AI Act (Article 14), SEC/CFTC trading reconstruction,
HIPAA audit trails, SOX financial controls.

#### Innovation 9: EvoSkills (Self-Evolving Skill Libraries)

Skills evolve through adversarial verification (EvoSkills, April 2026):

```
Round 1: Generate skill bundles from episodes
Round 2: Isolated Surrogate Verifier generates independent test assertions
Round 3: Skills that pass verification get promoted
Round 4: Failed skills get mutated and retried
Round 5: Cross-pollination between agents' skill libraries
Baseline: 32% → Round 5: 75% (surpasses human-curated by round 3)
```

Cross-model transfer: Skills evolved with one model transfer to six other models with
+35 to +44 percentage point gains.

#### Innovation 10: ADAS (Meta-Agent Architecture Search)

Hu et al. (ICLR 2025): A meta-agent iteratively programs new agent architectures in code.
Results: +14% on ARC Challenge, +13.6 F1 on reading comprehension, +14.4% on math versus
hand-designed agents. Discovered agents transfer across dissimilar domains.

Integration with Roko: The meta-agent explores the space of (Substrate, Scorer, Gate,
Router, Composer, Policy) implementations and discovers novel compositions that outperform
human-designed configurations.

#### Innovation 11: Cognitive Kernel Primitives

OS-level abstractions for agent cognition:

1. **Cognitive Namespaces**: Isolated knowledge spaces with explicit, auditable cross-namespace
   channels.
2. **Cognitive Signals**: Typed interrupts (Pause, Resume, Reprioritize, InjectContext,
   Escalate, Cooldown, Explore, Shutdown) that alter agent behavior without killing the
   process.
3. **Cognitive Scheduling**: Allocates reasoning resources by
   `priority = urgency × expected_value × (1/cognitive_cost)`.
4. **Engram Syscalls**: Every meaningful action passes through `Policy.decide()` for
   permit/deny/modify/log.

#### Innovation 12: Cross-Domain Insight Resonance

HDC-powered structural analogy detection across domains:

```
Coding:    BIND(high_complexity, more_review)
Chain:     BIND(high_volatility, more_caution)
Research:  BIND(contradictory_sources, more_verification)

All three share: BIND(high_uncertainty, more_verification)
Hamming similarity > threshold (0.526) → alert
```

False positive rate at threshold 0.526: < 1% against 100K vocabulary (Bonferroni-corrected).
Detection time: nanoseconds (pure Hamming distance computation on 10,240-bit vectors).

**Citations**: Kanerva 2009, Cognitive Computation 1(2). Plate 2003, Holographic Reduced
Representation. Kleyko et al. 2022, Artificial Intelligence Review.

#### Innovation 13: Generative Interfaces (A2UI)

Agents describe UI needs as structured JSONL → frameworks render automatically. Generated
interfaces inherit the ROSEDUST design system for visual consistency. Each agent's UI
includes its Spectre (procedurally generated creature) as a persistent visual anchor.

#### Innovation 14: Knowledge Futures Market

> **Implementation status**: P3 feature (Tier 6, deferred).

A novel financial primitive on Korai: research agents publish Knowledge Futures ("I will
produce analysis X within 24 hours"), operations agents purchase futures (funding the
research agent's inference costs), and escrow releases upon verified delivery. Creates a
predictive market for knowledge production.

---

## 3. Innovation Interconnection Map

The innovations form an autocatalytic network — each feeds into others:

| Innovation | Feeds Into | Via |
|---|---|---|
| T0 Probes (1) | Tier Router → VCG Auction (2) | Prediction error drives tier selection, tier drives context budget |
| VCG Auction (2) | Context Assembly → LLM Execution | Optimal context → better LLM output |
| Somatic Landscape (3) | Tier Router (1) + Skill Selection (9) | Emotional fast-path biases model and skill choice |
| Hypnagogia (4) | Neuro Store → Cross-Domain Resonance (12) | Novel hypotheses expand knowledge for cross-domain detection |
| Collective Calibration (5) | Predictive Foraging (6) → T0 Probes (1) | Better calibration → more accurate probes → higher T0 suppression |
| Predictive Foraging (6) | Calibration (5) + Context Selection (2) | Better predictions → better context choices |
| x402 Micropayments (7) | Knowledge Market (14) + Self-funding | Economic substrate enables agent-as-business |
| Forensic AI (8) | Compliance → Enterprise adoption | Regulatory pre-compliance enables regulated industries |
| EvoSkills (9) | Skill Library → ADAS (10) | Evolved skills become search targets for meta-architecture |
| ADAS (10) | New trait compositions → All innovations | Meta-search discovers novel compositions of all capabilities |
| Cognitive Kernel (11) | All layers — foundational infrastructure | Namespaces, signals, scheduling underpin all other innovations |
| Cross-Domain Resonance (12) | Knowledge → Hypnagogia (4) → Novel hypotheses | Cross-domain insights seed creative divergence |
| Generative Interfaces (13) | User adoption → More agents → Calibration (5) | Accessible interfaces attract users, growing the network |
| Knowledge Futures (14) | Directed research → Knowledge → All innovations | Market directs compute toward highest-value knowledge production |

---

## 4. Empirical Validation Status

Each innovation has different validation requirements:

| Innovation | Validation Status | What Would Validate It |
|---|---|---|
| 16 T0 Probes | Theoretical | Measure T0 suppression rate on real workloads (target: >70%) |
| VCG Attention Auction | Algorithm specified | A/B test: VCG vs. fixed-priority context selection |
| Somatic Landscape | Cited foundations | A/B test: agents with vs. without somatic markers |
| Hypnagogia | Cited foundations | Generate hypotheses, have domain experts evaluate genuine vs. spurious |
| 31.6× Calibration | Heuristic model | Run 100+ agents on Daeji testnet, measure actual speedup vs. solo |
| Predictive Foraging | Specified | Measure prediction accuracy improvement over time |
| x402 Micropayments | Protocol exists | Integration test on Base testnet |
| Forensic AI | Architecture supports it | Construct causal replay for a real incident, verify completeness |
| EvoSkills | Published results | Replicate EvoSkills within Roko's Gate pipeline |
| ADAS | Published results (ICLR 2025) | Run ADAS meta-search over Synapse trait compositions |
| Cognitive Kernel | Specified | Stress-test namespace isolation, signal handling |
| Cross-Domain Resonance | Theoretical | Generate cross-domain insights, measure false positive rate |
| Generative Interfaces | Specified | Build prototype A2UI renderer with ROSEDUST |
| Knowledge Futures | Deferred (P3) | Depends on Korai chain deployment |

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| Chen et al. 2023, arXiv:2305.05176 | FrugalGPT: cascade cost optimization, up to 98% reduction |
| Vickrey 1961; Clarke 1971; Groves 1973 | VCG mechanism: truthful bidding for resource allocation |
| Damasio 1994, "Descartes' Error" | Somatic marker hypothesis: emotion biases decisions |
| Bower 1981 | Mood-congruent memory: emotional state biases recall |
| Lacaux et al. 2021, Science Advances 7(50) | Hypnagogia: 83% hidden rule discovery in N1 |
| Haar Horowitz 2020/2023 (MIT Dormio) | Targeted dream incubation: 43% creativity boost |
| Derrida 1993 | Hauntology: "differently haunted" experiential traces |
| Boden 2004, "The Creative Mind" | Three creativity modes: combinational, exploratory, transformational |
| Hu et al. 2025, ICLR | ADAS: meta-agent architecture search, +14% ARC Challenge |
| Kanerva 2009, Cognitive Computation 1(2) | HDC: hyperdimensional computing for similarity search |
| Plate 2003 | Holographic Reduced Representations: HDC for knowledge |
| Kleyko et al. 2022, Artificial Intelligence Review | Survey of HDC applications |
| Charnov 1976 | Marginal Value Theorem: optimal foraging stopping rule |
| Pirolli & Card 1999 | Information Foraging Theory |
| Conant & Ashby 1970, IJSS 1(2) | Good Regulator Theorem |
| Lee et al. 2026, arXiv:2603.28052 | Meta-Harness: +7.7 pts text classification, +4.7 pts IMO math from harness optimization |
| Karpathy 2025 | Context engineering: "the delicate art of filling the context window" |

---

## Current Status and Gaps

- **Design principles (P1-P7)**: Documented and enforced in architectural review. Not
  formally encoded as automated checks (a linter for principle violations would be useful).
- **T0 Probes**: Specified but not implemented. The probe registry (`Vec<Box<dyn Probe>>`)
  is straightforward; individual probes need domain-specific implementations.
- **VCG Auction**: Specified with bid computation formulas. Not yet wired into the shipping
  Composer. Current context selection uses score-based ranking.
- **Somatic Landscape**: `roko-daimon` has PAD vector and behavioral states. The k-d tree
  somatic landscape is specified but not built.
- **Hypnagogia**: Specified in `roko-dreams` as scaffold. Not shipping.
- **31.6× Calibration**: Heuristic model only. Requires Korai chain for validation.
- **Predictive Foraging**: CalibrationTracker specified in `roko-learn`. Prediction
  registration wired; foraging stopping rule not yet implemented.
- **x402**: Protocol integration path identified. Not yet implemented.
- **Forensic AI**: Content-addressed lineage DAG exists. Causal replay tooling (`roko replay`)
  exists for simple cases. Full forensic reconstruction not yet built.
- **EvoSkills**: Skill library exists in `roko-learn`. Adversarial verification loop not
  yet implemented.
- **ADAS**: Not started. Depends on stable trait compositions.
- **Cognitive Kernel**: Cognitive signals partially implemented (SIGPAUSE in conductor).
  Namespaces and scheduling not yet built.
- **Cross-Domain Resonance**: HDC vectors built in `roko-primitives`. Cross-domain detection
  not yet wired.
- **Generative Interfaces**: Specified. Depends on ROSEDUST design system (not yet built).
- **Knowledge Futures**: P3 feature. Deferred.

---

## Cross-References

- See [00-vision-and-thesis](./00-vision-and-thesis.md) for the core thesis underlying these principles
- See [04-decay-variants](./04-decay-variants.md) for P5 (Decay as Feature) details
- See [05-provenance-and-attestation](./05-provenance-and-attestation.md) for P4 (Content-Addressed) and P7 (Observable) details
- See [11-dual-process-and-active-inference](./11-dual-process-and-active-inference.md) for T0/T1/T2 tier routing (Innovation 1)
- See [13-cognitive-cross-cuts](./13-cognitive-cross-cuts.md) for Daimon (P6), Dreams (Innovation 4), and Neuro
- See [14-c-factor-collective-intelligence](./14-c-factor-collective-intelligence.md) for Innovation 5 (31.6× Calibration) details
- See [16-autocatalytic-and-cybernetics](./16-autocatalytic-and-cybernetics.md) for the autocatalytic improvement theory
- See topic [08-chain](../08-chain/INDEX.md) for Korai chain and on-chain innovations (7, 8, 14)
- See topic [10-dreams](../10-dreams/INDEX.md) for Hypnagogia and Dreams engine details
