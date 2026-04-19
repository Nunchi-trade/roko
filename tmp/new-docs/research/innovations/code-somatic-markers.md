# Code Somatic Markers

> Automatically generate somatic markers from code intelligence metrics (cyclomatic complexity, reverse dependencies, HDC novelty, test coverage) and store them in the Daimon's k-d tree, so the agent feels pre-analytically "uneasy" about high-risk code regions before it begins work.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Neuroscience (somatic marker hypothesis), software engineering, cognitive science
**Affects subsystems**: Daimon (somatic landscape), Code intelligence (roko-index), Heartbeat/gating, Dreams (Integration phase)
**Last reviewed**: 2026-04-19

---

## The idea

Roko's Daimon has somatic markers (Damasio 1994) — fast pre-analytical gut feelings stored in a k-d tree over the 8-dimensional strategy space. Separately, the code intelligence system (`roko-index`) computes structural metrics: PageRank importance, HDC fingerprints, dependency depth, cyclomatic complexity.

The gap: code metrics exist as cold numbers; somatic markers exist as emotional heuristics. They're not connected. When an agent encounters code with high cyclomatic complexity, deep dependency chains, and low test coverage, it *should* feel uneasy — a somatic marker should fire saying "this is dangerous territory, use Conservative strategy." But currently the agent treats all code regions equally until a gate fails.

The key neuroscience insight: **somatic markers are faster than analysis**. Damasio showed that patients with ventromedial prefrontal cortex damage can reason about risks analytically but cannot *feel* danger — and they make catastrophically bad decisions because analytical reasoning is too slow for real-time choice.

The mapping from code metrics to the 8-dimensional strategy space:

| Dimension | Metric |
|---|---|
| 0 (Complexity) | cyclomatic_complexity / max_complexity |
| 1 (Risk) | (1 − test_coverage) × reverse_dep_count / max_deps |
| 2 (Novelty) | 1 − max(hdc_similarity(region, known_patterns)) |
| 3 (Confidence) | agent's PAD.dominance mapped to [0,1] |
| 4 (Time Pressure) | deadline_proximity × blocker_count |
| 5 (Scope) | files_modified × lines_changed / max_scope |
| 6 (Reversibility) | is_additive ? 0.8 : 0.2 |
| 7 (Dep Depth) | transitive_dep_count / max_transitive |

After a gate verdict with |PAD_delta| > 0.15 (significant emotional event), a somatic marker is inserted into the landscape. Before starting work on a code region, the somatic landscape is queried with the region's metric profile. k=10 nearest neighbours within radius 0.5 are retrieved; intensity-weighted mean valence drives strategy/tier bias:
- avg_valence < −0.5 → Conservative + T2 (danger zone)
- avg_valence in [−0.5, −0.2] → Balanced + T1 (caution)
- avg_valence > +0.5 → Exploratory + T0 (safe territory)

## Origin

- **Fakhoury et al. (2024)** "EEG as a Potential Ground Truth for Cognitive State in Software Development Activities," *PLOS ONE*. Validates that developers' neural signals during code comprehension reliably predict cognitive difficulty.
- **Pargaonkar et al. (2024)** "Quality Evaluation of Modern Code Reviews Through Intelligent Biometric Program Comprehension." HRV and pupillary response predict review quality with 87.77% accuracy.
- **Kaur et al. (2025)** "Towards Decoding Developer Cognition in the Age of AI Assistants," arXiv:2501.02684. Operationalises the somatic marker hypothesis for programming.
- **Damasio (1994)** *Descartes' Error: Emotion, Reason, and the Human Brain*. Somatic markers bias decision-making toward positive outcomes before conscious reasoning begins.

## Application to Roko

Seven integration steps are specified:

1. Add `CodeSomaticEngine` to `roko-daimon/src/code_somatic.rs` (alongside existing somatic landscape).
2. Wire `roko-index` metrics (SymbolGraph, PageRank, HDC fingerprints) into the engine.
3. Query bias before task execution in `roko-cli/src/orchestrate.rs`.
4. Create markers after gate verdicts with significant PAD delta.
5. Dream Integration phase merges nearby markers.
6. TUI visualisation: per-file valence overlay in `roko-cli/src/tui/`.
7. Log per-crate somatic profiles to `.roko/learn/somatic-code.json`.

## Estimated impact

Source states: "Somatic query < 100μs for landscape with 10,000 markers." "Marker creation from gate failure produces negative valence marker." Ranked **P0** — highest priority alongside HDC active inference.

## Prerequisites

- `roko-index` symbol graph, PageRank scores, and HDC fingerprints already computed.
- Existing somatic landscape k-d tree in `roko-daimon`.
- PAD delta threshold configuration (default: |delta| > 0.15).

## Status

Speculative — idea only; no formal evaluation. Ranked **P0** in the source implementation priority table (highest priority).

## Risks and objections

- The 8D metric-to-strategy mapping is hand-engineered; incorrect normalisers (max_complexity, max_rev_deps) will distort the k-d tree queries.
- Markers persist and compound: if many failures occur in a large codebase, the agent may become overly conservative across the board.
- k-d tree radius query is sensitive to the radius parameter; too large and irrelevant markers pollute the valence estimate.
- Somatic markers created from rare gate failures (statistical noise) may cause premature pessimism.

## Related innovations

- [affect-causal-discovery](./affect-causal-discovery.md) — causal discovery provides *why* affect correlates with failures; somatic markers provide the *speed*
- [hdc-active-inference](./hdc-active-inference.md) — HDC fingerprints used in novelty dimension (dim 2)
- [knowledge-morphogenesis](./knowledge-morphogenesis.md) — both use knowledge about code regions to shape agent behaviour
- [dream-verification](./dream-verification.md) — Dream Integration phase merges somatic markers from verified and violated dreams

## References

- Fakhoury et al. (2024). EEG as a Potential Ground Truth for Cognitive State in Software Development Activities. *PLOS ONE*.
- Pargaonkar et al. (2024). Quality Evaluation of Modern Code Reviews Through Intelligent Biometric Program Comprehension.
- Kaur et al. (2025). Towards Decoding Developer Cognition in the Age of AI Assistants. arXiv:2501.02684.
- Damasio (1994). *Descartes' Error: Emotion, Reason, and the Human Brain*. Putnam.
