# Affect Causal Discovery

> Treat the PAD (Pleasure-Arousal-Dominance) affect vector as a node in a Structural Causal Model so that do-calculus queries can identify when emotional state is *causing* task failures rather than merely correlating with them.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Causal inference, affective computing, cognitive science
**Affects subsystems**: Daimon, Learning (roko-learn), Dreams (REM counterfactuals)
**Last reviewed**: 2026-04-19

---

## The idea

Roko's Daimon tracks Pleasure-Arousal-Dominance (PAD) vectors that modulate behaviour: tier routing, exploration rate, context bidding, somatic marker lookup. But the relationship between affect and outcomes is treated as **correlational** — the OCC/Scherer appraisal pipeline maps events to PAD deltas via fixed coefficients (e.g., gate pass → P += 0.05, task failure → P −= 0.20).

Fixed coefficients cannot capture **causal structure**. When an agent is anxious (high arousal, low dominance) and a task fails, is the anxiety *causing* the failure (via conservative strategy selection), or is the failure *causing* the anxiety (via appraisal)? Without causal models the agent cannot distinguish these and cannot intervene effectively.

The solution: treat the PAD vector as a **node in a Structural Causal Model (SCM)** alongside task outcomes, strategy choices, context variables, and environmental state. Use **interventional queries** (do-calculus) to determine whether modifying affect would change outcomes, and use **counterfactual queries** to learn from hypothetical affect-strategy pairings.

```
Causal Graph:
  Environment → PAD → Strategy → Outcome
       ↓                            ↓
  Task_Difficulty ─────────→ Gate_Verdict
       ↑                            ↑
  Prior_Knowledge → Model_Choice → Quality

Interventions:
  do(PAD.arousal := 0.0)  → Would outcome change?
  do(Strategy := Exploratory) → Does affect still predict failure?
```

This enables **affect regulation as causal intervention**: the agent identifies when its emotional state is causally degrading performance and intervenes on the PAD vector directly (via contrarian retrieval, dream depotentiation, or forced behavioural state transition).

Structure learning uses the PC algorithm (Spirtes et al. 2000) on episode history (minimum 50 episodes, significance threshold p < 0.01). Counterfactuals use Halpern-Pearl abduction-action-prediction.

## Origin

- **Qian et al. (2025)** "Teleology-Driven Affective Computing: A Causal Framework for Sustained Well-Being," arXiv:2502.17172. Proposes causal modelling to infer agents' unique affective concerns.
- **Yang et al. (2024)** "Robust Emotion Recognition in Context Debiasing," CVPR 2024, arXiv:2403.05963. Generalised causal graph separating genuine emotional causes from confounding context.
- **SemEval-2024 Task 3** "Multimodal Emotion Cause Analysis in Conversations," arXiv:2405.13049. Emotion cause discovery as structured prediction over causal graphs.
- **Pearl (2009)** *Causality: Models, Reasoning, and Inference*. The three-level causal hierarchy: Association, Intervention, Counterfactual.

## Application to Roko

Seven integration steps are specified:

1. Add `AffectCausalModel` to `roko-daimon/src/causal.rs`.
2. Extract episode data with PAD snapshots — add PAD fields to `Episode` in `roko-learn`.
3. Run structure learning during Theta reflection (every 100 episodes).
4. Replace fixed appraisal deltas with learned causal coefficients in `roko-daimon/src/appraisal.rs`.
5. Run counterfactual analysis during REM (Phase 2) using Pearl SCM counterfactuals.
6. Inject optimal PAD pre-task via do-calculus grid search.
7. Persist causal graph to `.roko/learn/affect-causal.json`.

## Estimated impact

Source states: "Interventions reduce failure rate by ≥5% when activated" (test criterion). "Learned appraisal deltas converge within 200 episodes."

## Prerequisites

- Minimum 50 episodes per agent before structure learning can run.
- PAD snapshot added to every `Episode` record.
- PC algorithm implementation (or dependency on `rcd`/`pcalg` Rust crate).
- Counterfactual inference via Halpern-Pearl SCM.

## Status

Speculative — idea only; no formal evaluation. Ranked **P3** in the source implementation priority table (largest effort category).

## Risks and objections

- PC algorithm is sensitive to the faithfulness assumption; violation can produce incorrect causal graphs.
- With only 50 episodes minimum, causal graphs may be severely underpowered; false causal edges could harm behaviour.
- Do-calculus grid search over PAD space (3D × 21 values each = 9,261 evaluations) adds meaningful latency to task dispatch.
- Causal structure can shift as the agent learns new strategies — model may become stale.

## Related innovations

- [code-somatic-markers](./code-somatic-markers.md) — somatic markers provide fast-path heuristics; causal discovery provides the *why*
- [hdc-active-inference](./hdc-active-inference.md) — shared episode history feeds both HDC belief updates and causal discovery
- [witness-world-model](./witness-world-model.md) — Witness DAG is an alternative / complementary causal structure source
- [dream-verification](./dream-verification.md) — REM counterfactuals overlap with causal counterfactual analysis

## References

- Qian et al. (2025). Teleology-Driven Affective Computing: A Causal Framework for Sustained Well-Being. arXiv:2502.17172.
- Yang et al. (2024). Robust Emotion Recognition in Context Debiasing. CVPR 2024, arXiv:2403.05963.
- SemEval-2024 Task 3. Multimodal Emotion Cause Analysis in Conversations. arXiv:2405.13049.
- Pearl (2009). *Causality: Models, Reasoning, and Inference*. Cambridge University Press.
- Spirtes, Glymour & Scheines (2000). *Causation, Prediction, and Search*. MIT Press.
- Halpern & Pearl (2005). Causes and Explanations. *British Journal for the Philosophy of Science* 56(4).
