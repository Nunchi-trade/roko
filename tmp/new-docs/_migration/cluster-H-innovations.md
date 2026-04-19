# Cluster H Migration Log — Innovations Split

**Date**: 2026-04-19  
**Source**: `docs/00-architecture/30-cross-pollination-innovations.md` (2,848 lines, 117 KB)  
**Target**: `research/innovations/` (one file per distinct innovation)  
**Operator**: Subagent — Cluster H  

---

## Summary

| Metric | Value |
|---|---|
| Source file lines | 2,848 |
| Source file size | 117 KB (116,149 bytes) |
| Innovations extracted | **8** |
| Supporting/index files created | 12 (README, cross-interactions, 7 domain files, 9 subsystem files) |
| Status assigned | All 8: Speculative |
| Implementation priority range | P0 (highest) to P3 |

---

## Source H2/H3 heading → new slug mapping

| Source heading | New slug | Notes |
|---|---|---|
| `## 1. HDC + Active Inference` | `hdc-active-inference.md` | Full section including algorithm, Rust sketch, integration plan, test criteria |
| `### Motivation` (§1) | absorbed into `hdc-active-inference.md` — The idea | subsection |
| `### Research Basis` (§1) | absorbed into `hdc-active-inference.md` — Origin | subsection |
| `### Core Idea` (§1) | absorbed into `hdc-active-inference.md` — The idea | subsection |
| `### Algorithm: HDC Free Energy Minimization` | absorbed into `hdc-active-inference.md` — The idea | algorithm prose summarised; full pseudocode preserved in source |
| `### Rust Sketch` (§1) | absorbed into `hdc-active-inference.md` — Application to Roko | Rust sketches referenced but not reproduced verbatim (structural composition note) |
| `### Integration Plan` (§1) | absorbed into `hdc-active-inference.md` — Application to Roko | 7-step table included |
| `### Test Criteria` (§1) | absorbed into `hdc-active-inference.md` — Estimated impact + Application | checkboxes converted to prose |
| `## 2. Affect + Causal Discovery` | `affect-causal-discovery.md` | Full section |
| `### Motivation` (§2) | absorbed | |
| `### Research Basis` (§2) | absorbed → Origin | |
| `### Core Idea` (§2) | absorbed → The idea | |
| `### Algorithm: Affective Causal Discovery` | absorbed → The idea | |
| `### Rust Sketch` (§2) | absorbed → Application to Roko | |
| `### Integration Plan` (§2) | absorbed → Application to Roko | |
| `### Test Criteria` (§2) | absorbed → Estimated impact | |
| `## 3. Dreams + Formal Verification` | `dream-verification.md` | Full section |
| `### Motivation` (§3) | absorbed | |
| `### Research Basis` (§3) | absorbed → Origin | |
| `### Core Idea` (§3) | absorbed → The idea | |
| `### Algorithm: Dream Verification Pipeline` | absorbed → The idea | |
| `### Rust Sketch` (§3) | absorbed → Application to Roko | |
| `### Integration Plan` (§3) | absorbed → Application to Roko | |
| `### Test Criteria` (§3) | absorbed → Estimated impact | |
| `## 4. Morphogenesis + Knowledge` | `knowledge-morphogenesis.md` | Full section |
| (all subsections §4) | absorbed | same pattern as above |
| `## 5. Bandits + Pheromones` | `stigmergic-bandits.md` | Full section |
| (all subsections §5) | absorbed | |
| `## 6. Witness DAG + Active Inference` | `witness-world-model.md` | Full section |
| (all subsections §6) | absorbed | |
| `## 7. Somatic Markers + Code Intelligence` | `code-somatic-markers.md` | Full section |
| (all subsections §7) | absorbed | |
| `## 8. Token Economy + Dream Quality` | `dream-token-economy.md` | Full section |
| (all subsections §8) | absorbed | |
| `## Cross-Innovation Interactions` | `_cross-innovation-interactions.md` | Separate supporting file; also summarised in each innovation's Related innovations section |
| `## Implementation Priority` | Absorbed into README.md master table + each innovation's Status section | Priority table (P0–P3) preserved verbatim |

---

## Files created

### Individual innovation files (8)

```
research/innovations/hdc-active-inference.md
research/innovations/affect-causal-discovery.md
research/innovations/dream-verification.md
research/innovations/knowledge-morphogenesis.md
research/innovations/stigmergic-bandits.md
research/innovations/witness-world-model.md
research/innovations/code-somatic-markers.md
research/innovations/dream-token-economy.md
```

### Index and supporting files (12)

```
research/innovations/README.md                                    (master table)
research/innovations/_cross-innovation-interactions.md            (interaction map)
research/innovations/_by-domain/neuroscience.md
research/innovations/_by-domain/causal-inference.md
research/innovations/_by-domain/biology.md
research/innovations/_by-domain/information-theory.md
research/innovations/_by-domain/formal-methods.md
research/innovations/_by-domain/economics.md
research/innovations/_by-domain/reinforcement-learning.md
research/innovations/_by-subsystem/neuro.md
research/innovations/_by-subsystem/daimon.md
research/innovations/_by-subsystem/dreams.md
research/innovations/_by-subsystem/heartbeat-gating.md
research/innovations/_by-subsystem/coordination-pheromones.md
research/innovations/_by-subsystem/learning.md
research/innovations/_by-subsystem/code-intelligence.md
research/innovations/_by-subsystem/gate-safety.md
research/innovations/_by-subsystem/witness-dag.md
```

Total: **8 innovation files + 18 index/support files = 26 files**

---

## Domain breakdown

| Domain | Count | Slugs |
|---|---|---|
| Neuroscience | 2 | hdc-active-inference, code-somatic-markers |
| Causal inference | 2 | affect-causal-discovery, witness-world-model |
| Information theory / HDC | 1 | hdc-active-inference |
| Biology / morphogenesis | 2 | knowledge-morphogenesis, stigmergic-bandits |
| Formal methods | 1 | dream-verification |
| Economics / mechanism design | 1 | dream-token-economy |
| Reinforcement learning | 2 | stigmergic-bandits, dream-token-economy |
| Affective computing | 2 | affect-causal-discovery, code-somatic-markers |
| Software engineering | 1 | code-somatic-markers |

(Innovations have multiple source domains; sum > 8.)

---

## Status-tier breakdown

| Status | Count |
|---|---|
| Speculative | 8 |
| Evaluated | 0 |
| Queued | 0 |
| Absorbed | 0 |
| Rejected | 0 |

**Rationale for all-Speculative**: The source document is titled "Cross-Pollination Innovations" and all eight innovations are explicitly framed as speculative compositions with no indication any have been implemented or merged. The Implementation Priority table (P0–P3) guides future development order but does not indicate any are "already shipping."

---

## Ambiguous / cross-cutting passages and resolution

### 1. Rust sketches — verbatim vs. prose

The source file contains 8 × ~100-line Rust sketches (total ~800 lines of code) for each innovation. Resolution: the innovation files **describe** the Rust sketch in application terms (struct names, crate locations, key methods) rather than reproducing verbatim code. The full code lives in the source file and will be copied to the target crate files during implementation. This keeps innovation files readable and focused on the *idea*.

### 2. Algorithm pseudocode blocks

Each innovation has a ~50-line algorithm pseudocode block. Resolution: the key data structures and high-level steps are described in the "The idea" section; specific parameters are quoted. Full pseudocode is preserved in the source; it is not reproduced verbatim in innovation files to avoid duplication.

### 3. Test criteria checkboxes

Each innovation has 6–8 test criteria checkboxes. Resolution: quantitative test criteria (with numbers) are quoted verbatim in "Estimated impact." Qualitative criteria are absorbed into "Prerequisites" or "Risks and objections."

### 4. Cross-Innovation Interactions section

This section describes the four feedback loops connecting all 8 innovations. Resolution: extracted to its own file `_cross-innovation-interactions.md` and also summarised in the "Related innovations" section of each individual file.

### 5. Implementation Priority table

The P0–P3 priority table at the end of the source covers all 8 innovations. Resolution: reproduced in the README master table and in each innovation's "Status" section.

### 6. Overlap between HDC Beliefs [1] and Witness World Model [6]

The source explicitly states these two complement each other: "Use HDC for fast (~8μs) per-tick inference, Witness model for deeper (~50ms) reflection." Resolution: both kept as separate innovation files with cross-references. The complementarity is described in the Related innovations sections of both files.

### 7. Overlap between Affect Causal Discovery [2] and Code Somatic Markers [7]

The source states: "Causal model provides the *why*; somatic markers provide the *speed*." Resolution: kept separate; cross-referenced.

---

## Content accounting

All content from the source file has a home:

| Source content type | Destination |
|---|---|
| Title + abstract (lines 1–11) | README.md preamble |
| Table of Contents (lines 14–24) | README master table (reformatted) |
| 8 innovation H2 sections (lines 27–2781) | 8 individual .md files |
| Cross-Innovation Interactions (lines 2784–2833) | _cross-innovation-interactions.md |
| Implementation Priority table (lines 2836–2848) | README.md + each innovation's Status section |

**No content was discarded.**
