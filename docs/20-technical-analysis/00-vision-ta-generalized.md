# Technical Analysis as Universal Oracle Primitives

> TA is NOT chain-only. It is a general-purpose prediction framework with domain-specific instances.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture, [06-neuro](../06-neuro/INDEX.md) for HDC knowledge encoding
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4, legacy source `bardo-backup/prd/23-ta/00-witness-as-technical-analyst.md`, `refactoring-prd/09-innovations.md`

---

## Abstract

Technical analysis (TA) originated as a financial discipline — chart patterns, moving averages, momentum oscillators applied to price data. In Roko, TA is generalized far beyond its financial origins into a set of **universal oracle primitives**: prediction, evaluation, calibration, and feedback loops that operate identically across any domain where an agent interacts with a verifiable external system.

The core insight: code, markets, research, and operations all share the same structural properties that make TA useful. They are structured systems with measurable state variables, feedback loops, non-stationary dynamics, and adversarial participants. A build time trend is structurally analogous to a price trend. A test failure probability is structurally analogous to a risk assessment. A dependency vulnerability score is structurally analogous to portfolio risk. The mathematics is the same; the domain vocabulary changes.

This document establishes the vision: TA as a domain-agnostic cognitive capability that any Roko agent can use, regardless of whether it operates on blockchains, codebases, research corpora, or any other structured domain. Under the two-mediums/two-fabrics framing in the [glossary](../00-architecture/01-naming-and-glossary.md), TA is one of the places where Roko's compounding and superlinear product claim becomes measurable: each prediction, correction, and replay cycle should make the next cycle cheaper, faster, and better. See also [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md).

---

## Why generalize TA

### The structural analogy argument

Traditional TA works because financial markets have specific properties:

1. **Measurable state** — prices, volumes, open interest, volatility
2. **Time series dynamics** — trends, mean reversion, regime changes
3. **Feedback loops** — momentum traders amplify trends, mean-reversion traders dampen them
4. **Pattern recurrence** — similar market structures produce similar outcomes
5. **Adversarial dynamics** — participants adapt to each other's strategies
6. **External verification** — prices are observable, outcomes are deterministic

Codebases share every one of these properties:

1. **Measurable state** — compilation time, test pass rates, cyclomatic complexity, dependency counts
2. **Time series dynamics** — complexity trends, performance regression trajectories, test coverage drift
3. **Feedback loops** — tech debt accumulates → development slows → more shortcuts → more debt
4. **Pattern recurrence** — similar code structures produce similar bug patterns
5. **Adversarial dynamics** — security vulnerabilities, supply chain attacks
6. **External verification** — compilers, test suites, benchmarks produce objective outcomes

Research corpora share similar properties:

1. **Measurable state** — citation counts, publication velocity, contradiction density
2. **Time series dynamics** — field maturity, paradigm shifts, replication crises
3. **Feedback loops** — popular papers attract more citations → more visibility → more citations
4. **Pattern recurrence** — similar research methodologies produce similar reliability
5. **Adversarial dynamics** — p-hacking, selective reporting, predatory publishing
6. **External verification** — replication studies, meta-analyses, cross-validation

The structural analogy is not a metaphor — it is a mathematical fact. If we define TA as "systematic prediction from structured time series with feedback," then TA applies to any domain with those properties. Markets are one instance. Code is another. Research is a third. Roko treats all three identically at the trait level, specializing only in the domain-specific implementations.

### Cross-domain insight transfer

When a coding agent learns "high-churn modules need more review," it encodes this as an HDC vector:

```
BIND(high_complexity, more_review)
```

When a chain agent learns "high-volatility assets need more caution," it encodes:

```
BIND(high_volatility, more_caution)
```

Both encode the same abstract structure:

```
BIND(high_uncertainty, more_verification)
```

The Hamming similarity between these vectors is high because the HDC algebra preserves structural isomorphism. This means cross-domain insight transfer happens automatically through the Neuro (knowledge) system — a pattern learned in one domain can be detected as relevant in another.

This is the deepest justification for generalizing TA. If each domain used its own bespoke prediction framework, cross-domain transfer would require explicit translation. With universal oracle primitives, transfer happens through the HDC similarity space at nanosecond cost (Kleyko et al., 2022, ACM Computing Surveys).

### The Roko thesis: "the scaffold IS the product"

Roko's architectural thesis is that agent performance varies dramatically based on the surrounding harness — context engineering, verification, learning loops, cognitive architecture. Meta-Harness (Lee et al., 2026, arXiv:2603.28052) demonstrated +7.7 points on text classification and +4.7 on IMO-level math from harness optimization alone, at 4× fewer tokens.

TA generalization is a direct expression of this thesis. By making prediction, calibration, and feedback universal primitives — not domain-specific add-ons — every agent benefits from the same self-improving prediction infrastructure. The harness (oracle + calibration tracker + residual corrector) works identically whether the agent is predicting build times, gas prices, or source reliability.

---

## Domain-agnostic prediction architecture

### Where oracles live in the Synapse Architecture

The Oracle subsystem operates across multiple layers of Roko's five-layer architecture:

| Layer | Oracle role | What it does |
|---|---|---|
| **L0 Runtime** | Prediction storage | `PredictionStore` persists predictions and outcomes |
| **L1 Framework** | Router integration | `Router.feedback()` uses prediction accuracy for bandit updates |
| **L2 Scaffold** | Context selection | EFE (Expected Free Energy) uses prediction confidence for context bidding |
| **L3 Harness** | Gate calibration | Prediction residuals calibrate adaptive gate thresholds |
| **L4 Orchestration** | Task prioritization | Prediction accuracy informs task scheduling priority |

The Oracle connects to all six Synapse traits:

- **Substrate** — `PredictionStore` persists predictions as Engrams via `Substrate.put()`
- **Scorer** — `PredictiveScorer` uses oracle accuracy to weight Engram relevance
- **Gate** — Prediction residuals feed adaptive gate thresholds (EMA per rung)
- **Router** — Accurate oracle predictions increase routing weight via `Router.feedback()`
- **Composer** — EFE-based context selection uses prediction confidence for VCG auction bids
- **Policy** — `PredictionPolicy` tracks accuracy, feeds back to all other traits

### The cognitive cross-cut dimension

Oracles are a cognitive cross-cut — they inject into multiple layers via trait objects, never hardcoded. The Daimon (motivation engine) modulates oracle behavior through PAD (Pleasure-Arousal-Dominance) state:

- **Low confidence (Dominance)** → Predictions are made conservatively, wider prediction intervals
- **High arousal** → More frequent prediction updates, faster residual correction
- **Low pleasure (after prediction failures)** → Automatic model recalibration

The Neuro (knowledge store) accumulates oracle patterns:

- Successful prediction strategies become `StrategyFragment` knowledge entries
- Systematic prediction biases become `Warning` entries
- Causal relationships discovered through prediction analysis become `CausalLink` entries

Dreams (offline learning) consolidate oracle performance:

- NREM replay evaluates which prediction patterns were reliable across episodes
- REM imagination generates counterfactual predictions to test oracle robustness
- Integration staging promotes validated prediction strategies to permanent knowledge

---

## The seven-step loop with oracles

The seven-step loop is where TA turns usage into compounding returns:

```
1. SENSE      → Substrate.query() + Bus.subscribe()
               Oracle reads state, prior Engrams, and live Pulses
2. ASSESS     → Scorer.score() + Router.select()
               PredictiveScorer weights uncertainty, drift, and likely payoff
3. COMPOSE    → Composer.compose()
               EFE-weighted context includes the smallest prediction-relevant slice
4. ACT        → Agent.execute()
               Agent emits prediction Pulses and final task output
5. VERIFY     → Gate.verify()
               Outcome closes the prediction and emits verdict Engrams/Pulses
6. PERSIST    → Substrate.put()
               Prediction, outcome, and calibration artifacts graduate to Engrams
   BROADCAST  → Bus.publish()
               Residuals, anomalies, and calibration Pulses feed other agents
7. REACT      → Policy.decide()
               Residual correction, heuristic updates, and routing changes fire
```

This mapping keeps TA aligned with the current architecture story in [00-architecture](../00-architecture/INDEX.md): Engrams carry durable prediction history in the Substrate, Pulses carry ephemeral prediction and outcome traffic on the Bus, and Policy closes the learning loop. See also [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the detailed prediction-resolution-calibration path.

---

## Three cognitive speeds for oracles

Oracles operate at all three of Roko's cognitive timescales:

| Speed | Period | Oracle activity |
|---|---|---|
| **Gamma** (~5-15s) | Real-time | T0 probes evaluate prediction error scalar. No LLM. Zero cost. |
| **Theta** (~75s) | Reflection | Pending predictions resolved. Residuals computed. CalibrationTracker updated. |
| **Delta** (hours) | Consolidation | Cross-domain prediction patterns consolidated. Routing tables updated. |

At Gamma frequency, the 16 T0 probes (FrugalGPT-inspired; Chen et al., 2023, arXiv:2305.05176) compute a prediction error scalar that drives T0/T1/T2 cognitive tier routing:

```
error < 0.2  → T0 (suppress, no LLM)     ~80% of ticks
error < 0.6  → T1 (fast model, shallow)   ~15% of ticks
error ≥ 0.6  → T2 (full model, deep)      ~5% of ticks
```

This means 80% of cognitive cycles cost nothing — the oracle decides no action is needed based on pure Rust probes running in microseconds.

---

## Why TA is a compounding system

REF15 makes an explicit product claim: Roko should improve superlinearly on real workloads because multiple feedback loops reinforce each other. Technical analysis is the instrumentation layer for that claim because each loop depends on prediction, verification, or calibration:

| REF15 loop | TA contribution | What should improve with use |
|---|---|---|
| Demurrage-weighted retrieval | Oracles measure which memories still predict useful outcomes | Fewer wasted tokens, better retrieval precision |
| Heuristic calibration | Prediction/outcome joins tighten confidence intervals | Better priors on similar tasks |
| HDC codebook cleanup | Oracle outcomes add cleaner labels to each HDC fingerprint | Faster cache hits and better analogical matches |
| c-factor feedback | Shared prediction accuracy reveals which cohorts learn well together | Better routing across agents and domains |
| Playbook distillation | Repeated predictions collapse into reusable strategy templates | Lower time-to-solution for recurring task classes |
| Cross-deployment heuristic commons | Portable calibration data bootstraps new deployments | Better first-week performance on fresh installs |
| Plugin ecosystem | Each plugin adds new measurable state and new verification surfaces | More domains that feed the same learning machinery |

The point is not just that these loops exist. The point is that they are observable. If TA cannot show improving slopes, then the broader compounding claim has not actually landed.

### Scaling dashboards

The technical-analysis chapter is the right home for the operator dashboards that REF15 calls out:

| Dashboard line | Interpretation | Failure signal |
|---|---|---|
| Retrieval quality vs. episode count | Demurrage and HDC are improving effective recall | Flat slope means retrieval is not learning |
| Mean calibration CI width per heuristic | Heuristic calibration is tightening with trials | Flat or rising width means the Calibrator lacks fresh trials |
| c-factor trend on sampled cohorts | Coordination is turning better predictions into better group output | Rising c with flat gate outcomes suggests reward hacking |
| HDC cleanup hit rate | The codebook is helping later retrieval and composition | Falling hit rate implies noisy fingerprints or poor cleanup |
| Mean time to first successful PR on a new codebase | Headline composite KPI across all loops | Flat curve means the product claim is not compounding |

### Anti-metrics

Superlinear capability gains are only credible if a few resource curves stay bounded:

| Anti-metric | Why it should stay flat or shrink |
|---|---|
| Warm-tier episode count | Demurrage should keep the working set bounded |
| Heuristic count with confirmations below 3 | Weak hypotheses should either be tested quickly or decay away |
| Mean lineage depth per response | Context depth should only grow when it buys quality |

If these anti-metrics blow out while the headline metrics improve, the system is probably cheating by hoarding context or retaining low-value memory.

### Evaluation guardrails

The compounding claim is only testable on real workloads with preserved state:

1. Evals must span sessions and days, not reset the Substrate between runs.
2. Each benchmark should be attempted multiple times so the slope of `time_to_solve` is measurable.
3. Commons-on versus commons-off runs should be compared explicitly to isolate the value of shared heuristics.
4. Operator dashboards should be read alongside task difficulty buckets so easier task selection cannot masquerade as progress.

Without those guardrails, TA can still report local prediction quality, but it cannot justify the stronger superlinear product claim.

---

## Why domain-specific instances matter

Universal primitives provide the architecture. Domain-specific instances provide the value. Each domain has unique:

- **State variables** to predict (prices vs. build times vs. citation counts)
- **Verification mechanisms** (blockchain finality vs. compiler output vs. replication studies)
- **Temporal dynamics** (block-time vs. CI pipeline cadence vs. publication cycles)
- **Adversarial threats** (MEV vs. supply chain attacks vs. p-hacking)
- **Feedback loop structure** (market impact vs. tech debt vs. citation cascades)

The Oracle trait abstracts over these differences. Domain-specific implementations handle the details. New domains are added by implementing the Oracle trait — not modifying the kernel.

---

## Academic foundations

- Friston, K. (2010). "The free-energy principle: a unified brain theory?" *Nature Reviews Neuroscience*, 11(2), 127-138. — Active inference framework underlying EFE-based context selection.
- Conant, R. C., & Ashby, W. R. (1970). "Every good regulator of a system must be a model of that system." *International Journal of Systems Science*, 1(2), 89-97. — Good Regulator Theorem motivating agent self-modeling via prediction.
- Sumers, T. R., et al. (2023). "Cognitive Architectures for Language Agents." arXiv:2309.02427. — CoALA framework that the Synapse Loop implements.
- Lee, S., et al. (2026). "Meta-Harness: Optimizing Harness, Not Model." arXiv:2603.28052. — +7.7 pts from scaffold optimization alone.
- Chen, L., et al. (2023). "FrugalGPT: How to Use Large Language Models While Reducing Cost and Improving Performance." arXiv:2305.05176. — Cascade architectures for cost reduction.
- Kleyko, D., et al. (2022). "A Survey on Hyperdimensional Computing." *ACM Computing Surveys*, 54(6), 1-51. — HDC foundations for cross-domain transfer.
- Kanerva, P. (2009). "Hyperdimensional Computing: An Introduction to Computing in Distributed Representation with High-Dimensional Random Vectors." *Cognitive Computation*, 1(2), 139-159. — Binary spatter codes for pattern encoding.
- Pirolli, P., & Card, S. (1999). "Information foraging." *Psychological Review*, 106(4), 643-675. — Optimal foraging theory applied to information retrieval, inspiring predictive foraging.
- Charnov, E. L. (1976). "Optimal foraging: the marginal value theorem." *Theoretical Population Biology*, 9, 129-136. — Stopping rule for context retrieval.

---

## Current status and gaps

**What exists today**:
- `CascadeRouter` in `roko-learn` implements tier-based model routing with UCB1 and LinUCB
- Adaptive gate thresholds with EMA per rung in `.roko/learn/gate-thresholds.json`
- Efficiency events logged per turn in `.roko/learn/efficiency.jsonl`
- Prompt experiments via `ExperimentStore` in `.roko/learn/experiments.json`

**What's scaffold/planned**:
- Full `Oracle` trait implementation (defined in `refactoring-prd/03-cognitive-subsystems.md`)
- `PredictionStore` with on-chain and off-chain variants
- `CalibrationTracker` per (model, task_category) pair
- Domain-specific oracle implementations (chain, coding, research)
- Active inference state space (factorized POMDP, 90 states)
- EFE-based context selection with pragmatic + epistemic - ambiguity decomposition
- Cross-session scaling dashboards for compounding, anti-metrics, and commons-on/off comparisons

See `tmp/implementation-plans/modelrouting/12-advanced-patterns.md` for the Thompson Sampling and predictive foraging calibration implementation plan. See also [tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md) for the canonical superlinear-scaling framing this chapter now measures.

---

## Cross-References

- See [01-oracle-trait.md](./01-oracle-trait.md) for the full Rust trait signature and Prediction struct
- See [02-chain-oracles.md](./02-chain-oracles.md) for chain-specific TA primitives
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding equivalents of TA primitives
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the full prediction loop
- See [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md) for the current two-mediums/two-fabrics vocabulary
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture overview
- See topic [05-learning](../05-learning/INDEX.md) for the cybernetic feedback loops
- See topic [06-neuro](../06-neuro/INDEX.md) for HDC cross-domain transfer
- See [../../tmp/refinements/15-exponential-scaling.md](../../tmp/refinements/15-exponential-scaling.md) for the canonical REF15 proposal
