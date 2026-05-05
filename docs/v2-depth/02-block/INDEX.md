# 02-block — Depth Index

Depth for [02-CELL.md](../../unified/02-CELL.md)

---

## Source docs (32)

### Protocol traits

| Source doc | Status |
|---|---|
| `docs/00-architecture/06-synapse-traits.md` | Absorbed into `protocol-algebra.md`, `store-and-bus-duality.md` |
| `docs/00-architecture/07-substrate-trait.md` | Absorbed into `store-and-bus-duality.md` |
| `docs/00-architecture/07b-bus-transport-fabric.md` | Absorbed into `store-and-bus-duality.md` |
| `docs/00-architecture/08-scorer-gate-router-composer-policy.md` | Absorbed into `protocol-algebra.md`, `verify-as-universal-oracle.md` |

### Composition (Compose protocol)

| Source doc | Status |
|---|---|
| `docs/03-composition/00-composer-trait.md` | Absorbed into `compose-protocol-and-builder.md` |
| `docs/03-composition/01-prompt-composer.md` | Absorbed into `compose-protocol-and-builder.md` |
| `docs/03-composition/02-system-prompt-builder-7-layer.md` | Absorbed into `compose-protocol-and-builder.md` |
| `docs/03-composition/03-role-templates.md` | Absorbed into `compose-protocol-and-builder.md` |
| `docs/03-composition/04-enrichment-pipeline-13-step.md` | Absorbed into `enrichment-pipeline.md` |
| `docs/03-composition/05-token-budget-management.md` | Absorbed into `enrichment-pipeline.md` |
| `docs/03-composition/06-lost-in-the-middle-u-shape.md` | Absorbed into `positional-effects-and-retrieval.md` |
| `docs/03-composition/07-active-inference-context-selection.md` | Absorbed into `active-inference-context-selection.md` |
| `docs/03-composition/08-5-stage-assembly-pipeline.md` | Absorbed into `active-inference-context-selection.md` |
| `docs/03-composition/09-predictive-foraging-mvt.md` | Absorbed into `active-inference-context-selection.md` |
| `docs/03-composition/10-vcg-attention-auction.md` | Absorbed into `vcg-attention-auction.md` |
| `docs/03-composition/11-distributed-context-engineering.md` | Absorbed into `distributed-and-affect-composition.md` |
| `docs/03-composition/12-affect-modulated-retrieval.md` | Absorbed into `distributed-and-affect-composition.md` |
| `docs/03-composition/13-current-status-and-gaps.md` | Status absorbed across all three depth docs (mori-diffs reality sections) |

### Verification (Verify protocol)

| Source doc | Status |
|---|---|
| `docs/04-verification/00-gate-trait.md` | Absorbed into `verify-as-universal-oracle.md`, `verify-cells-and-pipeline.md` |
| `docs/04-verification/01-gate-implementations.md` | Absorbed into `verify-cells-and-pipeline.md` |
| `docs/04-verification/02-6-rung-selector.md` | Absorbed into `verify-cells-and-pipeline.md` |
| `docs/04-verification/03-gate-pipeline.md` | Absorbed into `verify-cells-and-pipeline.md` |
| `docs/04-verification/04-artifact-store.md` | Absorbed into `verify-cells-and-pipeline.md`, `process-reward-and-artifacts.md` |
| `docs/04-verification/05-ratcheting.md` | Absorbed into `ratcheting-and-adaptive-thresholds.md` |
| `docs/04-verification/06-adaptive-thresholds.md` | Absorbed into `ratcheting-and-adaptive-thresholds.md` |
| `docs/04-verification/07-process-reward-models.md` | Absorbed into `process-reward-and-artifacts.md` |
| `docs/04-verification/08-agent-feedback-from-gates.md` | Absorbed into `gate-feedback-and-retry.md` |
| `docs/04-verification/09-evaluation-lifecycle.md` | Absorbed into `eval-lifecycle-and-generation.md` |
| `docs/04-verification/10-autonomous-eval-generation.md` | Absorbed into `eval-lifecycle-and-generation.md` |
| `docs/04-verification/11-evoskills.md` | Absorbed into `eval-lifecycle-and-generation.md` |
| `docs/04-verification/12-forensic-ai-causal-replay.md` | Absorbed into `verdicts-as-signals.md` |
| `docs/04-verification/15-verdicts-as-signals.md` | Absorbed into `verdicts-as-signals.md` |

---

## Depth docs

| Doc | Covers | Source docs absorbed |
|---|---|---|
| [protocol-algebra.md](protocol-algebra.md) | Categorical structure of the 9 protocols: Cells as objects, typed Signal/Pulse flows as morphisms, composition rules, natural transformations (Score => Verify, Verify => React, Store <=> React), capability pullback, free monad over protocol vocabulary | `06-synapse-traits.md`, `08-scorer-gate-router-composer-policy.md` |
| [store-and-bus-duality.md](store-and-bus-duality.md) | Store/Bus duality (pull vs push), graduation (Pulse -> Signal) and projection (Signal -> Pulse) as adjoint functors, consistency under ring eviction, backpressure strategies, BroadcastBus implementation, distributed Bus design | `07-substrate-trait.md`, `07b-bus-transport-fabric.md` |
| [verify-as-universal-oracle.md](verify-as-universal-oracle.md) | Verify's four simultaneous roles (reward function, relabeling oracle, safety boundary, economic attestation), Goodhart-resistance proof sketch, Variance Inequality, Bradley-Terry aggregation for subjective criteria, meta-verification Loop, 4-role Verdict dispatch | `08-scorer-gate-router-composer-policy.md` (Verify aspects), `00-gate-trait.md` |
| [compose-protocol-and-builder.md](compose-protocol-and-builder.md) | Compose protocol: ComposeBid/ComposeResult types, PromptComposer greedy knapsack, VCG auction Cell (built but not runtime-called), 9-layer SystemPromptBuilder as Pipeline Graph, RoleSystemPromptSpec wrapper, 12 role templates with per-role budgets, complexity-adaptive budgets, cache alignment, --bare flag and per-section empirical analysis | `00-composer-trait.md`, `01-prompt-composer.md`, `02-system-prompt-builder-7-layer.md`, `03-role-templates.md` |
| [enrichment-pipeline.md](enrichment-pipeline.md) | 13-step enrichment pipeline as Pipeline Graph: PrdExtract through Scribe, LLM client abstraction, staleness checking, TOML repair, continue-on-failure, step selection (Self-RAG), three-tier budget architecture, differential budget principle, priority-ordered allocation, min-tokens guard, cost attribution, prefix stability for caching | `04-enrichment-pipeline-13-step.md`, `05-token-budget-management.md` |
| [positional-effects-and-retrieval.md](positional-effects-and-retrieval.md) | U-shape as algebraic property of causal decoders, Placement enum, PositionAttentionModel, section-to-placement mapping, cache alignment interaction, section effect tracking via BetaPosterior, leave-one-out influence, cost-aware effectiveness, information-theoretic density, dynamic LongLLMLingua-style reordering | `06-lost-in-the-middle-u-shape.md` |
| [active-inference-context-selection.md](active-inference-context-selection.md) | EFE scoring (pragmatic + epistemic - ambiguity) as Score Cell, MVT stopping rule as Route Cell, 5-stage assembly Pipeline Graph, multi-source foraging, social foraging via stigmergic Pulses, cold-to-warm transition, EFE/VCG convergence | `07-active-inference-context-selection.md`, `08-5-stage-assembly-pipeline.md`, `09-predictive-foraging-mvt.md` |
| [vcg-attention-auction.md](vcg-attention-auction.md) | VCG mechanism for budget-constrained context assembly, 8 bidder Score Cells, auctioneer Compose Cell, Thompson sampling LearningBidder with cost-awareness, externality payments as diagnostics, strategy auto-selection (WeightedSum/Vcg), cost attribution feedback loop, fairness alternatives (proportional, max-min, alpha-fairness) | `10-vcg-attention-auction.md` |
| [distributed-and-affect-composition.md](distributed-and-affect-composition.md) | Four context strategies (Write/Select/Compress/Isolate), three levels of context engineering, Daimon PAD affect as endofunctor F: Signal -> Signal, PAD octants and context bias, appraisal triggers, decay toward baseline, somatic marker analog, write-for-amnesia principle, cross-agent iteration memory | `11-distributed-context-engineering.md`, `12-affect-modulated-retrieval.md`, `13-current-status-and-gaps.md` |
| [gate-feedback-and-retry.md](gate-feedback-and-retry.md) | How Verify Cell Verdicts flow back to agents as structured GateFeedback Signals. Feedback classification Pipeline (noise filtering, severity ordering, 97.75% token reduction). Retry Loop pattern: gate fail -> classify -> enrich prompt -> re-dispatch. Section-effectiveness learning Loop. Model escalation on repeated failure. | `08-agent-feedback-from-gates.md` |
| [eval-lifecycle-and-generation.md](eval-lifecycle-and-generation.md) | 14 evaluation Loops across 5 speed tiers (machine -> cognitive -> consolidation -> retrospective -> meta). Autonomous eval generation: test/property/invariant generation by separate agent, immutable verification artifacts, cheap-model convergence Loop. EvoSkills: 3-tier skill hierarchy (episodes -> patterns -> playbook), MAP-Elites quality-diversity archive, adversarial surrogate verification, speciation, CMA-ES, AURORA learned descriptors. The meta-Loop where verification criteria evolve. | `09-evaluation-lifecycle.md`, `10-autonomous-eval-generation.md`, `11-evoskills.md` |
| [verdicts-as-signals.md](verdicts-as-signals.md) | Verdicts as first-class Signals with Kind, Score, demurrage (24h half-life), lineage, and content hash. Consumer specs: Score Cell appraisal, Route Cell escalation, Compose Cell injection, Dreams pattern extraction. Forensic AI causal replay: content-addressed chain, BLAKE3 integrity, regulatory compliance. Verdict aggregation: trend detection (CUSUM/BOCPD), co-failure patterns, signature clustering. Verdict-driven replanning and predictive gate selection. | `12-forensic-ai-causal-replay.md`, `15-verdicts-as-signals.md` |
| [verify-cells-and-pipeline.md](verify-cells-and-pipeline.md) | The 11 gate implementations as Verify Cells (ShellGate foundation, CompileGate/ClippyGate/TestGate/SymbolGate/DiffGate core, GeneratedTest/PropertyTest/Integration higher rungs, LlmJudge/VerifyChain auxiliary). 7-rung Pipeline Graph with short-circuit. Gate composition algebra (Sequential, Parallel, Fallback, Voting, Threshold). Verdict lattice. Probabilistic Verify Cells with Wilson interval. Progressive delivery pipeline. Rung selection as Route Cell. ArtifactStore for content-addressed evidence. Verification-first architecture (GVU). | `00-gate-trait.md`, `01-gate-implementations.md`, `02-6-rung-selector.md`, `03-gate-pipeline.md`, `04-artifact-store.md` |
| [ratcheting-and-adaptive-thresholds.md](ratcheting-and-adaptive-thresholds.md) | GateRatchet: monotonic constraint preventing verification regression (convergence thrashing). Adaptive thresholds as calibration Loop: EMA per rung (alpha=0.1), retry budget suggestion, skip advisory (20 consecutive passes). SPC extensions: CUSUM for sustained shifts, EWMA control chart with formal limits, BOCPD for regime change detection, PELT for offline analysis. Multi-gate coordination via Hotelling T-squared. Domain-specific threshold profiles. | `05-ratcheting.md`, `06-adaptive-thresholds.md` |
| [process-reward-and-artifacts.md](process-reward-and-artifacts.md) | Process Reward Models: Promise (within-attempt success likelihood) and Progress (cross-attempt advancement) as Score+Verify composition. Self-supervised PRM training from gate verdicts. Monte Carlo step-level Q-values. FoVer formally verified labels. Potential-based reward shaping (Ng et al. 1999). ThinkPRM generative verification. DPO/RLAIF/Constitutional AI alternatives. Dense reward schedule combining sparse gate verdicts with shaped potential rewards. Artifact store as evidence chain. | `04-artifact-store.md`, `07-process-reward-models.md` |
