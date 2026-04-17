# References — Master Citation Index

> Complete academic bibliography for the Roko cognitive agent architecture. Every citation referenced in any Roko PRD document has an entry here, organized by domain with annotations linking each paper to the specific subsystem it grounds.

**Topic**: References
**Layer**: Cross-cutting
**Crates**: All

---

## Overview

This reference collection catalogs **260+ academic citations** across 26 research domains that ground Roko's cognitive architecture. Each citation appears in exactly one domain sub-doc, with cross-references to related domains. REF16 adds a dedicated research-to-runtime chapter so papers, claims, falsifiers, and the replication ledger stay visible as a live pipeline rather than disappearing into architecture folklore; see `../../tmp/refinements/16-research-to-runtime.md` and [25-research-to-runtime](./25-research-to-runtime.md). Every citation includes:

1. Standard academic format (authors, year, title, venue, arXiv ID when available)
2. A 1-2 sentence annotation explaining which Roko subsystem the paper grounds and how

### Methodology

Citations were collected from:
- **Primary sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` (130+ papers), `bardo-backup/prd/02-mortality/15-references.md` (162 citations), `bardo-backup/prd/shared/citations.md` (master index, 40 categories)
- **Implementation research**: `bardo-backup/tmp/agent-chain/08-references.md`, `bardo-backup/tmp/agent-chain/14-academic-foundations.md`, `bardo-backup/tmp/mori-agents/12-references.md`
- **Refactoring PRD**: All 12 refactoring-prd files scanned for inline citations
- **Implementation plans**: `tmp/implementation-plans/modelrouting/11-research-context.md`, `tmp/implementation-plans/12a-cognitive-layer.md`

All mortality-era citations are preserved with reframed annotations: "because the agent is dying" → "because the agent's budget/confidence/time is constrained."

---

## Sub-documents by Domain

| # | Domain | File | Key Citations | Count |
|---|---|---|---|---|
| 00 | [Lifecycle and Finite Agency](./00-lifecycle-and-finite-agency.md) | Resource-bounded cognition, plasticity loss, evolutionary computing | Ray 1991, Dohare 2024, Shuvaev 2024, Hinton 2022 | ~30 |
| 01 | [Memory Consolidation](./01-memory-consolidation.md) | CLS theory, forgetting, retrieval, replay, agent memory surveys | McClelland 1995, Ebbinghaus 1885, Park 2023, Mem0, Liu 2025 | ~29 |
| 02 | [Affective Computing](./02-affective-computing.md) | PAD model, somatic markers, mood-congruent retrieval, ALMA, emotion-in-loop | Mehrabian 1996, Damasio 1994, Zhang 2024, Yin 2025 | ~23 |
| 03 | [Dreams and Offline Learning](./03-dreams-and-offline-learning.md) | Hypnagogia, world models, sleep-time compute, NeuroDream, SleepGate | Lacaux 2021, DreamerV3, Lin 2025, NeuroDream 2025 | ~21 |
| 04 | [Coordination and Multi-Agent](./04-coordination-and-multi-agent.md) | Stigmergy, cooperation, emergent coordination, LLM collectives | Grassé 1959, Dorigo 1997, Emergence 2025, Stigmergy PDE 2024 | ~24 |
| 05 | [Biological Analogues](./05-biological-analogues.md) | Foraging, immune selection, niche construction, swarm robotics | Charnov 1976, Kauffman 1993, Turing 1952, Stigmergy 2024 | ~18 |
| 06 | [Self-Learning Systems](./06-self-learning-systems.md) | Reflexion, ExpeL, Voyager, SAMULE, multi-agent reflection | Lee 2026, Shinn 2023, Hu 2025, SAMULE 2025 | ~22 |
| 07 | [Context Engineering](./07-context-engineering.md) | ACE, CSO, ACON, RAG, context position, compression | ACE 2026, Liu 2024, Lewis 2020, Wei 2022 | ~16 |
| 08 | [Security and Provenance](./08-security-and-provenance.md) | CaMeL, OWASP, formal verification AI, safety guarantees | Debenedetti 2025, Cohen 1987, C2PA, Formal Methods 2025 | ~19 |
| 09 | [HDC and VSA](./09-hdc-vsa.md) | BSC, learned hashing, FLASH adaptive encoder, HDC++ systems | Kanerva 1988/2009, Kleyko 2022, FLASH 2024, HPVM-HDC 2024 | ~20 |
| 10 | [Market Microstructure](./10-market-microstructure.md) | AMM theory, vault mechanisms, risk/decision theory | Milionis 2023, Kelly 1956, Peters 2019, Taleb 2012 | ~12 |
| 11 | [Streaming Algorithms](./11-streaming-algorithms.md) | Adaptive windowing, calibration, distributional RL | Bifet 2007, Farquhar 2024, Dabney 2018/2020 | ~10 |
| 12 | [Signal Processing](./12-signal-processing.md) | TDA, persistent homology advances, information theory | Shannon 1948, Carlsson 2009, TDA Beyond PH 2025 | ~16 |
| 13 | [Philosophy](./13-philosophy.md) | Temporality, hauntology, embodiment, agency, narrative | Heidegger 1927, Derrida 1993, Camus 1942, Merleau-Ponty | ~18 |
| 14 | [Agent Harnesses and Tool Use](./14-agent-harnesses-and-tool-use.md) | Meta-Harness, SWE-agent, Aider, benchmarks | Lee 2026, Yang 2024, SWE-bench, RAGAS | ~14 |
| 15 | [Cybernetics and VSM](./15-cybernetics-and-vsm.md) | VSM, Good Regulator, autopoiesis, OODA, triple-loop | Beer 1972, Conant-Ashby 1970, Ashby 1956, Wiener 1948 | ~18 |
| 16 | [Active Inference](./16-active-inference.md) | FEP, DR-FREE robust inference, multi-LLM active inference | Friston 2006/2010/2015, Shafiei 2025, Koudahl 2024 | ~15 |
| 17 | [Process Reward Models](./17-process-reward-models.md) | Step-level verification, generation-verification gap | Lightman 2024, Song 2025, Huang 2024 | ~8 |
| 18 | [Collective Intelligence](./18-collective-intelligence.md) | C-Factor, dynamical emergence, LLM collectives, Thousand Brains | Woolley 2010, Emergence 2025, Hawkins 2017, Holland 1995 | ~14 |
| 19 | [Regulatory Compliance](./19-regulatory-compliance.md) | EU AI Act, SEC/CFTC, GDPR, SOX, C2PA | EU AI Act 2024, MiFID II, HIPAA, GDPR | ~8 |
| 20 | [Cognitive Architectures](./20-cognitive-architectures.md) | CoALA, dual-process, agentic AI surveys, cognitive LLMs | Sumers 2023, Kahneman 2011, Agentic AI Survey 2025 | ~15 |
| 21 | [Mechanism Design](./21-mechanism-design.md) | VCG auctions, agent exchange, LLM mechanism design | Vickrey 1961, AEX 2025, Duetting 2024, Ostrom 1990 | ~17 |
| 22 | [Protocol Standards](./22-protocol-standards.md) | ERC-8004, x402, ERC-4337, ERC-721, cryptographic | ERC-8004, x402, Merkle 1987, Goldwasser 1985 | ~16 |
| 23 | [Generational and Evolutionary](./23-generational-and-evolutionary.md) | Digital evolution, memetics, cultural transmission | Ray 1991, Dawkins 1976, Price 1970, Baldwin 1896 | ~18 |
| 24 | [2025 Additions](./24-additions-2025.md) | Cross-domain 2024-2025 papers, emerging research frontiers | DR-FREE 2025, NeuroDream 2025, SAMULE 2025, AEX 2025 | ~60 |
| 25 | [Research to Runtime](./25-research-to-runtime.md) | Paper -> Claim -> Heuristic -> Trial -> Calibration, replication ledger, ingestion lanes, replication contract | Kanerva 2009, Friston 2006, Woolley 2010, Auer 2002 | ~12 |

**Total**: ~260+ unique citations across 26 domains.

---

## Citation Format

Each citation uses standard academic format:

```
Author(s) (YYYY). Title. _Venue_, Volume(Issue), Pages. arXiv:ID.
  *Grounds: <subsystem> — <concept>.*
```

The `*Grounds:*` annotation links the paper to the specific Roko subsystem and concept it validates. This creates a traceable chain from academic research to architectural decision.

---

## Cross-Domain Papers

Some papers are foundational to multiple domains. These appear in their primary domain with cross-references:

- **Damasio (1994)** — Primary: [02-affective-computing](./02-affective-computing.md), Cross-ref: [13-philosophy](./13-philosophy.md)
- **Derrida (1993)** — Primary: [13-philosophy](./13-philosophy.md), Cross-ref: [03-dreams](./03-dreams-and-offline-learning.md)
- **Park (2023) Generative Agents** — Primary: [01-memory](./01-memory-consolidation.md), Cross-ref: [03-dreams](./03-dreams-and-offline-learning.md), [14-harnesses](./14-agent-harnesses-and-tool-use.md)
- **Lee et al. (2026) Meta-Harness** — Primary: [14-harnesses](./14-agent-harnesses-and-tool-use.md), Cross-ref: [06-self-learning](./06-self-learning-systems.md)
- **Friston (2010) FEP** — Primary: [16-active-inference](./16-active-inference.md), Cross-ref: [15-cybernetics](./15-cybernetics-and-vsm.md)
- **Shafiei et al. (2025) DR-FREE** — Primary: [16-active-inference](./16-active-inference.md), Cross-ref: [24-additions-2025](./24-additions-2025.md)
- **Emergent Coordination (2025)** — Primary: [04-coordination](./04-coordination-and-multi-agent.md), Cross-ref: [18-collective-intelligence](./18-collective-intelligence.md)
- **NeuroDream (2025)** — Primary: [03-dreams](./03-dreams-and-offline-learning.md), Cross-ref: [24-additions-2025](./24-additions-2025.md)
- **SAMULE (2025)** — Primary: [06-self-learning](./06-self-learning-systems.md), Cross-ref: [24-additions-2025](./24-additions-2025.md)
- **Stigmergy PDE (2024)** — Primary: [04-coordination](./04-coordination-and-multi-agent.md), Cross-ref: [05-biological](./05-biological-analogues.md)

---

## Naming Conventions

All citations use the updated Roko terminology:
- **Golem** (retired) → **Agent**
- **Grimoire** (retired) → **Neuro**
- **Styx** (retired) → **Mesh**
- **Clade** (retired) → **Fleet**
- **GNOS** → **KORAI** (mainnet) / **DAEJI** (testnet)
- **Bardo** (retired) → **Roko**

Legacy citations that referenced mortality mechanisms are preserved with reframed annotations per `refactoring-prd/08-translation-guide.md`.
