# Technical Analysis as Universal Oracle Primitives

> TA is NOT chain-only. It is a general-purpose prediction framework with domain-specific instances.

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture, [06-neuro](../06-neuro/INDEX.md) for HDC knowledge encoding
**Key sources**: `refactoring-prd/03-cognitive-subsystems.md` §4, `bardo-backup/prd/23-ta/00-witness-as-technical-analyst.md`, `refactoring-prd/09-innovations.md`

---

## Abstract

Technical analysis (TA) originated as a financial discipline — chart patterns, moving averages, momentum oscillators applied to price data. In Roko, TA is generalized far beyond its financial origins into a set of **universal oracle primitives**: prediction, evaluation, calibration, and feedback loops that operate identically across any domain where an agent interacts with a verifiable external system.

The core insight: code, markets, research, and operations all share the same structural properties that make TA useful. They are structured systems with measurable state variables, feedback loops, non-stationary dynamics, and adversarial participants. A build time trend is structurally analogous to a price trend. A test failure probability is structurally analogous to a risk assessment. A dependency vulnerability score is structurally analogous to portfolio risk. The mathematics is the same; the domain vocabulary changes.

This document establishes the vision: TA as a domain-agnostic cognitive capability that any Roko agent can use, regardless of whether it operates on blockchains, codebases, research corpora, or any other structured domain. We lead with the universal `Oracle` trait, then show how domain-specific instances (chain oracles, coding oracles, research oracles) implement the same interface.

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

## The universal loop with oracles

The 9-step universal cognitive loop (Synapse Loop) integrates oracles at every stage:

```
1. PERCEIVE      → Substrate.query()       Oracle reads current state
2. EVALUATE      → Scorer.score()          PredictiveScorer weights by prediction accuracy
3. ATTEND        → Router.select()         Oracle predictions bias attention to uncertain areas
4. INTEGRATE     → Composer.compose()      EFE-weighted context includes prediction context
5. ACT           → Agent.execute()         Agent makes prediction BEFORE acting
6. VERIFY        → Gate.verify()           Prediction resolved against external outcome
7. PERSIST       → Substrate.put()         Prediction + outcome stored as Engrams
8. ADAPT         → Policy.decide()         Residual correction updates calibration
9. META-COGNIZE  → Daimon.assess()         Prediction accuracy updates cognitive state
```

This maps to CoALA cognitive architecture (Sumers et al., 2023, arXiv:2309.02427) with the prediction loop inspired by active inference (Friston Free Energy Principle) and the Good Regulator Theorem (Conant & Ashby, 1970).

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

See `tmp/implementation-plans/modelrouting/12-advanced-patterns.md` for the Thompson Sampling and predictive foraging calibration implementation plan.

---

## Cross-references

- See [01-oracle-trait.md](./01-oracle-trait.md) for the full Rust trait signature and Prediction struct
- See [02-chain-oracles.md](./02-chain-oracles.md) for chain-specific TA primitives
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding equivalents of TA primitives
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the full prediction loop
- See topic [00-architecture](../00-architecture/INDEX.md) for the Synapse Architecture overview
- See topic [05-learning](../05-learning/INDEX.md) for the cybernetic feedback loops
- See topic [06-neuro](../06-neuro/INDEX.md) for HDC cross-domain transfer
