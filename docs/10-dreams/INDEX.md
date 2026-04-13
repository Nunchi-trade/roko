# Dreams (Offline Learning and Consolidation)

Dreams are one of three cognitive cross-cuts in Roko — subsystems that span multiple architectural layers rather than living at a single layer. While Neuro provides persistent memory and Daimon provides emotional context, Dreams provide the offline consolidation mechanism that transforms raw episodic experience into durable semantic knowledge, creative hypotheses, and strategic insights. Dreams are the agent's sleep: a periodic offline process where the agent replays recent episodes, generates counterfactual scenarios, depotentiates emotional charge, discovers cross-episode patterns, and stages new knowledge for waking validation. Dreams are idle-triggered and scheduled — they fire when the agent has capacity, not in response to any mortality or termination signal.

## Prerequisites

Before reading this topic, familiarity with the following is helpful:

- **Synapse Architecture**: The 6-trait system (Substrate, Scorer, Gate, Router, Composer, Policy) through which all capabilities flow
- **5-Layer Taxonomy**: L0 Runtime, L1 Framework, L2 Scaffold, L3 Harness, L4 Orchestration
- **Engram**: The content-addressed, scored, decaying unit of cognition (currently named `Signal` in the codebase)
- **Neuro / NeuroStore**: The agent's persistent knowledge base (episodes, insights, heuristics, causal links)
- **Daimon**: The affect engine maintaining PAD (Pleasure-Arousal-Dominance) emotional state vectors

## Table of Contents

| # | Sub-Doc | Description |
|---|---------|-------------|
| 00 | [00-vision-and-dream-as-death-reframe.md](00-vision-and-dream-as-death-reframe.md) | Vision document: what dreams are, why they exist, and the critical reframe from death-triggered to idle-triggered consolidation |
| 01 | [01-three-phase-cycle.md](01-three-phase-cycle.md) | Three-phase dream cycle: NREM replay → REM imagination → Integration staging. Phase descriptions, state machine, resource allocation |
| 02 | [02-nrem-replay.md](02-nrem-replay.md) | NREM replay phase: Mattar-Daw utility formula, four replay modes, cross-episode pattern discovery, emotional modulation, replay fidelity spectrum, SM-2 scheduling, DRL experience replay connections |
| 03 | [03-rem-imagination.md](03-rem-imagination.md) | REM imagination phase: Pearl SCM counterfactuals, Boden's three creativity modes, emotional depotentiation, conceptual blending, imagination validation (GIRL trust-region), imagination budget, world models (DreamerV3/IRIS/Genie) |
| 04 | [04-consolidation-and-staging.md](04-consolidation-and-staging.md) | Integration/consolidation: SQLite staging buffer, confidence ladder, promotion mechanics, temporal decay, safety constraints |
| 05 | [05-dream-evolution.md](05-dream-evolution.md) | EVOLUTION fourth phase: memetic selection, strategy evolution, HDC knowledge recombination, dream-prediction feedback |
| 06 | [06-hdc-counterfactual-synthesis.md](06-hdc-counterfactual-synthesis.md) | HDC counterfactual synthesis: 10,240-bit BSC vectors, XOR binding, majority bundling, K-medoids clustering, counterfactual diversity (DiCE/DPP), plausibility scoring (FACE/LOF/causal consistency) |
| 07 | [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Hypnagogia engine: Thalamic Gate, Executive Loosener, Dali Interrupt, Homuncular Observer. Stochastic resonance, novelty filtering (Lehman-Stanley), hypnagogia-to-insight pipeline (Wallas/Collins-Loftus) |
| 08 | [08-divergence-and-alpha.md](08-divergence-and-alpha.md) | Alpha convergence problem, three levels of divergence, experiential wisdom thesis, alpha taxonomy |
| 09 | [09-threat-simulation.md](09-threat-simulation.md) | Revonsuo's Threat Simulation Theory, three-tier threat taxonomy, threat rehearsal, gap analysis, systematic threat enumeration (FMEA/FTA/ATLAS), severity assessment (CVSS/DREAD/Bayesian) |
| 10 | [10-hauntology-in-dreams.md](10-hauntology-in-dreams.md) | Derrida hauntology, spectral traces, compound escape from monoculture, knowledge transfer as backup/restore |
| 11 | [11-inner-worlds-and-rendering.md](11-inner-worlds-and-rendering.md) | Visual rendering for each dream phase: NREM theater, REM garden, hypnagogia phosphenes, integration crystallization |
| 12 | [12-sleep-time-compute.md](12-sleep-time-compute.md) | Lin et al. 2025 sleep-time compute, budget allocation, CascadeRouter model selection, Sleepwalker mode |
| 13 | [13-scheduling-and-triggers.md](13-scheduling-and-triggers.md) | Three trigger types (idle, scheduled, manual), frequency adaptation, intensive mode, orchestrator coordination |
| 14 | [14-oneirography.md](14-oneirography.md) | Dream art and creative expression: image generation pipeline, self-appraisal, affect-reactive auctions, extended art forms |
| 15 | [15-cross-system-integration.md](15-cross-system-integration.md) | Integration with Neuro, Daimon, Learn, Compose, Gate, Mesh, Orchestrator, Hypnagogia, Supervisor |
| 16 | [16-implementation-status.md](16-implementation-status.md) | Current code status, implementation plan items, roko-golem dissolution plan, roadmap, open questions |
| 17 | [17-advanced-dream-concepts.md](17-advanced-dream-concepts.md) | Dream sharing across mesh, nightmare detection and containment, persistent dream journals, lucid dream monitoring |

## Related Topics

| Topic | Relationship |
|-------|-------------|
| `04-neuro/` | Knowledge store that dreams read from and write to |
| `05-daimon/` | Affect engine providing emotional context and receiving depotentiation updates |
| `08-agent-mesh/` | Mesh coordination for sharing dream insights across agents |
| `02-architecture/` | 5-layer taxonomy and Synapse traits that dreams implement |
| `06-context/` | Context engineering that injects dream insights into waking prompts |
| `07-gates/` | Gate pipeline whose results feed dream replay and whose thresholds dreams can update |
| `09-orchestrator/` | Plan execution coordinating with dream scheduling |

## Key Academic Citations

The dream subsystem draws on extensive academic research:

| Citation | Topic |
|----------|-------|
| Mattar & Daw, *Nature Neuroscience*, 2018 | Prioritized memory replay utility formula |
| McClelland et al., *Psychological Review*, 1995 | Complementary Learning Systems (CLS) theory |
| Walker & van der Helm, *Psychological Bulletin*, 2009 | REM emotional depotentiation ("overnight therapy") |
| Pearl, *Causality*, Cambridge University Press, 2009 | Structural Causal Models for counterfactual generation |
| Boden, *The Creative Mind*, Routledge, 2004 | Three creativity modes (combinational, exploratory, transformational) |
| Revonsuo, *Behavioral and Brain Sciences*, 2000 | Threat Simulation Theory |
| Gammaitoni et al., *Reviews of Modern Physics*, 1998 | Stochastic resonance for noise-enhanced signal detection |
| Lin et al., 2025 | Sleep-time compute: 5× reduction in test-time compute |
| Kanerva, *Cognitive Computation*, 2009 | Hyperdimensional computing for knowledge representation |
| Epstude & Roese, *PSPR*, 2008 | Functional theory of counterfactual thinking |
| Byrne, *The Rational Imagination*, MIT Press, 2005 | Fault lines in mental models |
| Fauconnier & Turner, *The Way We Think*, Basic Books, 2002 | Conceptual blending theory |
| Derrida, *Specters of Marx*, 1993 | Hauntology — spectral traces of past agents |
| Grassé, 1959 | Stigmergy — indirect coordination through environmental modification |
| Grossman & Stiglitz, *AER*, 1980 | Information paradox and knowledge marketplace value |
| Zahavi, *J. Theoretical Biology*, 1975 | Handicap principle — costly signals as honest indicators |
| Thaler, *Marketing Science*, 1985; *J. Behavioral Decision Making*, 1999 | Mental accounting theory |
| Hafner et al., *Nature*, 2025 (DreamerV3) | World model learning and dream-based planning |
| Tancik et al., CVPR 2020 (StegaStamp) | Steganographic encoding in images |
| WSCL 2024 | 38% reduction in catastrophic forgetting |
| Schaul et al., *ICLR*, 2016 (PER) | Prioritized experience replay with TD-error priority and IS correction |
| Andrychowicz et al., *NeurIPS*, 2017 (HER) | Hindsight experience replay — relabeling failed episodes with achieved goals |
| Shin et al., *NeurIPS*, 2017 | Continual Learning with Deep Generative Replay — Scholar architecture |
| Mnih et al., *Nature*, 2015 (DQN) | Experience replay for breaking temporal correlations |
| Wang & Ross, 2019 (ERE) | Emphasizing Recent Experience — recency-biased replay |
| Helfrich et al., *Nature Neuroscience*, 2023 | SO-spindle-ripple triple coupling for memory consolidation |
| Mothilal et al., *FAT\**, 2020 (DiCE) | Diverse Counterfactual Explanations via DPP |
| Wachter et al., *Harvard JOLT*, 2018 | Original counterfactual explanation formulation |
| Poyiadzi et al., *AIES*, 2020 (FACE) | Feasible Actionable Counterfactual Explanations via density paths |
| Karimi et al., *FAccT*, 2021 | Algorithmic recourse — from counterfactual explanations to interventions |
| Lehman & Stanley, *Evolutionary Computation*, 2011 | Novelty search — evolution through search for novelty alone |
| Collins & Loftus, *Psychological Review*, 1975 | Spreading activation theory of semantic memory |
| Perez et al., *EMNLP*, 2022 | Red teaming language models with language models |
| Ganguli et al., Anthropic, 2022 | Red teaming language models to reduce harms — 38,961 attacks |
| Mazeika et al., 2024 (HarmBench) | Standardized evaluation framework for automated red teaming |
| MITRE ATLAS v5.1.0, 2025 | Adversarial Threat Landscape for AI Systems |
| Micheli et al., *ICLR*, 2023 (IRIS) | Discrete world model with transformers |
| Bruce et al., *ICML*, 2024 (Genie) | Generative interactive environments from unlabeled video |
| Filevich et al., *Journal of Neuroscience*, 2015 | Metacognitive mechanisms underlying lucid dreaming |

---

## Generation Notes

- **Sub-docs produced**: 18 (00 through 17) plus INDEX.md
- **Total line count**: ~8,100+ lines across all sub-docs
- **Key legacy sources consulted**:
  - `bardo-backup/prd/05-dreams/` — dream cycle architecture, consolidation, scheduling
  - `bardo-backup/prd/06-hypnagogia/` — hypnagogia engine, inner worlds, divergence
  - `bardo-backup/prd/22-oneirography/` — creative expression (dream journals, self-appraisal, auctions, extended forms; `02-death-masks.md` skipped per instructions)
  - `refactoring-prd/03-cognitive-layer.md` — three cognitive speeds, dream phases, CLS theory
  - `refactoring-prd/04-systems-and-synapse-traits.md` — Synapse trait mapping for dreams
  - `tmp/implementation-plans/12a-cognitive-layer.md` §G — implementation plan items G1-G8
  - Active code: `roko-dreams/src/runner.rs`, `roko-dreams/src/cycle.rs`, `roko-dreams/src/lib.rs`, `roko-golem/src/dreams.rs`, `roko-golem/src/hypnagogia.rs`, `roko-learn/src/pattern_discovery.rs`, `roko-learn/src/hdc_clustering.rs`
- **Decisions requiring judgment calls**:
  - **Death mask exclusion**: `22-oneirography/02-death-masks.md` was skipped entirely per prompt instructions. All references to death masks in other oneirography docs were reframed or removed.
  - **EVOLUTION phase**: Presented as a fourth dream phase in `05-dream-evolution.md` based on source material, even though the primary cycle is three-phase (NREM/REM/Integration). The EVOLUTION phase is an extension, not a replacement.
  - **Oneirography domain-agnostic reframe**: The legacy oneirography spec was heavily blockchain/NFT-specific. The Roko version presents the core pipeline (dream→image→score) as domain-agnostic, with NFT minting as a blockchain domain extension.
  - **Naming consistency**: All instances of "Golem" → "Agent", "Grimoire" → "Neuro/NeuroStore", "Signal" → "Engram" (with code-name notes where relevant), "Styx" → "Agent Mesh/Mesh", "GNOS" → "KORAI/DAEJI", "Clade" → "Collective/Mesh", "golem.toml" → "roko.toml".
  - **Vitality phases removed**: All references to Thriving/Stable/Conservation/Declining/Terminal behavioral phases have been reframed as continuous budget and knowledge metrics rather than discrete mortality phases.
- **Unresolved tensions**:
  - The `roko-golem` crate still exports `ScaffoldEngine` which `roko-dreams/src/lib.rs` re-exports. This dependency should be removed when `roko-golem` is dissolved.
  - The Daimon (affect engine) is referenced extensively in dream design but is not yet implemented in the codebase.
  - HDC vectors (`bardo-primitives`) are built but not called from the dream subsystem.
