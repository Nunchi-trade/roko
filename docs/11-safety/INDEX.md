# Safety & Provenance

> **Abstract:** Roko's safety architecture is a defense-in-depth system that combines structural enforcement (compile-time capabilities, content-addressed audit chains), behavioral enforcement (runtime guards, rate limiters, sandboxing), and cognitive enforcement (temporal logic monitoring, adaptive risk, human intervention signals). Safety is not a bolt-on layer — it is woven into every step of the Universal Cognitive Loop through the Synapse traits. Every Engram carries provenance, every tool call passes through a gated pipeline, every knowledge entry traces back to its evidential basis. This topic covers the full safety stack: from low-level guards (bash command filtering, path sandboxing, secret scrubbing) through mid-level verification (Gate pipeline, threat modeling, adaptive risk) to high-level safety capabilities (temporal logic verification, witness DAGs, forensic AI, formal verification). It also documents the critical integration gap — the #1 priority for making the safety architecture effective in production.

---

## Prerequisites

Before reading this topic, readers should be familiar with:

- **Synapse Architecture**: The 6 traits (Substrate, Scorer, Gate, Router, Composer, Policy) and the Engram data type — see `docs/01-architecture/`
- **Universal Cognitive Loop**: The 9-step loop (PERCEIVE → EVALUATE → ATTEND → INTEGRATE → ACT → VERIFY → PERSIST → ADAPT → META-COGNIZE) — see `docs/01-architecture/`
- **Five Layers**: L0 Runtime, L1 Framework, L2 Scaffold, L3 Harness, L4 Orchestration — see `docs/01-architecture/`

---

## Table of Contents

### Foundation

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 00 | [00-defense-in-depth.md](00-defense-in-depth.md) | Overall safety architecture: three defense categories (structural, behavioral, cognitive), six runtime guards, SafetyLayer composition, integration with Synapse Loop, adversarial safety testing framework (9 attack categories), CSA MAESTRO 7-layer mapping | 499 |
| 01 | [01-capability-tokens.md](01-capability-tokens.md) | Current ToolPermission system and target Capability<T> design: PhantomData type safety, three tool tiers, compile-time enforcement | 301 |
| 02 | [02-audit-chain.md](02-audit-chain.md) | Engram lineage DAG, SHA-256 / BLAKE3 Merkle hash-chain, AuditSink trait, FileSubstrate persistence, on-chain anchoring | 333 |
| 03 | [03-taint-tracking.md](03-taint-tracking.md) | TaintLabel enum, TaintedString with zeroize, DataSink flow matrix, 4-stage ingestion pipeline, Bloom Oracle, causal rollback, taint propagation algebra (Denning lattice, SecurityLabel with confidentiality/integrity dimensions, join operator), FIDES integration, RTBAS dynamic taint tracking, Prompt Flow Integrity (PFI), PCAS Datalog policy language | 804 |

### Runtime Guards

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 04 | [04-permits-allowlists.md](04-permits-allowlists.md) | ToolPermission flags (Read/Write/Execute/Network), role-based permission matrix, task-level tool filters | 227 |
| 05 | [05-loop-detection.md](05-loop-detection.md) | RateLimiter sliding window, circuit breaker (conductor), DiagnosisEngine ghost turn detection, secret zeroization, adaptive gate thresholds | 202 |
| 06 | [06-sandboxing.md](06-sandboxing.md) | PathPolicy canonicalization algorithm, ProcessSupervisor lifecycle management, WorktreeManager isolation, future container sandboxing | 201 |

### Attack Surface

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 07 | [07-prompt-security.md](07-prompt-security.md) | Prompt architecture with XML delimiters, CaMeL dual-LLM (Debenedetti et al. 2025), ventriloquist defense, Tool-Guard pattern, MCP avoidance | 238 |
| 08 | [08-threat-model.md](08-threat-model.md) | General-purpose attack trees (prompt injection, sandbox escape, credential exfiltration, resource exhaustion), chain-domain attack trees, 8 residual risks, formal safety analysis, NIST AI RMF alignment (4 functions), MITRE ATLAS technique mapping (10 techniques), STRIDE-AI classification (6 categories), OWASP Agentic Top 10 mapping (ASI01-ASI10), cascading failure analysis with blast radius modeling, adversarial testing framework | 968 |
| 09 | [09-adaptive-risk.md](09-adaptive-risk.md) | Five-layer adaptive risk: hard shields, Kelly sizing with confidence multiplier, Beta-Binomial OperationalConfidenceTracker, health scoring, Daimon integration, safety budgets (5 dimensions: irreversibility, blast radius, footprint, uncertainty, cost), hierarchical budget delegation with conservation laws, automatic allocation strategies (equal, proportional, risk-weighted) | 1101 |

### Domain-Specific Safety (Chain)

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 10 | [10-mev-protection.md](10-mev-protection.md) | MEV taxonomy (sandwich, front-run, back-run, JIT, arbitrage), detection algorithms, protection strategies, Gate pipeline integration, MEV as intelligence signal | 228 |
| 11 | [11-temporal-logic.md](11-temporal-logic.md) | LTL Buchi automata for runtime monitoring, CTL pre-execution plan verification, safety/liveness/fairness properties, 40 DeFi temporal patterns, category-theoretic composition, TemporalMonitor as Policy, extended temporal pattern library (11 code agent + 3 multi-agent patterns), past-time LTL, temporal attack pattern detection (3-tier: node/edge/path anomaly scoring), boiling frog and slow escalation detectors | 1157 |
| 12 | [12-witness-dag.md](12-witness-dag.md) | BLAKE3 content-addressed DAG, five vertex types (Observation, Prediction, Decision, Resolution, NeuroEntry), ZK proofs for strategy auditing, SQLite storage, on-chain anchoring, DAG-based trust, DAG query language (6 query types), Datalog provenance queries via Datafrog, safety-specific query patterns (TOCTOU, escalation chain, exfiltration, circular reasoning) | 1544 |
| 13 | [13-formal-verification.md](13-formal-verification.md) | Five-stage verification pipeline: Heimdall-rs decompilation → Slither static analysis → Echidna fuzzing → hevm symbolic execution → Certora/Kontrol formal proofs, verification-guided agent design (17 host-agent + 14 task-lifecycle properties), tool behavioral contracts (pre/post/invariant), VeriGuard dual-stage verification, ContractEnforcingDispatcher | 1310 |

### Advanced Safety

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 14 | [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md) | Cognitive Kernel Primitives: Namespaces with ACL, Cognitive Signals (typed interrupts), Cognitive Scheduling (priority + deadline + cooperative yield), Engram Syscalls (universal enforcement) | 388 |
| 15 | [15-forensic-ai.md](15-forensic-ai.md) | Content-addressed causal replay, Forensic AI regulatory pre-compliance: EU AI Act, SEC/CFTC, HIPAA, SOX, GDPR, pre-certified agent templates, enterprise value proposition | 363 |

### Integration Status

| # | Sub-doc | Description | Lines |
|---|---|---|---|
| 16 | [16-critical-integration-gap.md](16-critical-integration-gap.md) | The #1 integration gap: SafetyLayer → ToolDispatcher wired but ToolDispatcher never invoked from CLI pipeline. Impact assessment, resolution path (4 phases), architecture mismatch analysis | 254 |

---

## Key Architectural Decisions

1. **Safety as a structural property.** Safety is not a separate module — it is woven into the Synapse traits. The `Gate` trait provides verification, `Policy` provides enforcement, `Substrate` provides audit persistence. Every Engram carries provenance by construction.

2. **Defense in depth, not single barriers.** Six runtime guards compose into the SafetyLayer. Gates verify after execution. The conductor monitors system health. Temporal logic watches for behavioral anomalies. Each layer catches what the previous layer missed.

3. **Content-addressing enables forensics.** Every Engram has a BLAKE3 hash. Every Engram records its parents via lineage. This makes the entire system auditable by construction — Forensic AI is a free byproduct.

4. **Domain-agnostic core, domain-specific plugins.** The safety guards (BashPolicy, PathPolicy, ScrubPolicy) are domain-agnostic. MEV protection, formal verification, and DeFi temporal patterns are chain-domain plugins. Other domains add their own safety plugins through the same trait system.

5. **The gap must be closed.** The #1 priority is wiring the ToolDispatcher into the production code path. Until then, the safety architecture is built but dormant for per-tool-call enforcement.

---

## Cross-References

- `docs/01-architecture/` — Synapse Architecture, Engram struct, Five Layers
- `docs/03-cognitive/` — Neuro (knowledge), Daimon (affect modulation of risk), Dreams (offline consolidation)
- `docs/08-chain/` — Chain domain plugin where MEV protection and formal verification live
- `docs/09-innovations/` — Forensic AI, Cognitive Kernel Primitives as frontier innovations

---

## Generation Notes

- **Sub-docs produced**: 17 (00 through 16)
- **Total line count**: ~10,679 lines across 17 sub-docs (expanded from ~7,600 in April 2026 enhancement pass)
- **Key legacy sources consulted**:
  - `bardo-backup/prd/10-safety/00-defense.md` (defense-in-depth, capability tokens, taint tracking, audit chain)
  - `bardo-backup/prd/10-safety/05-threat-model.md` (adversary types, attack trees, residual risks)
  - `bardo-backup/prd/10-safety/06-adaptive-risk.md` (five-layer risk, Kelly sizing, Bayesian guardrails)
  - `bardo-backup/prd/10-safety/07-temporal-logic-verification.md` (LTL, CTL, DeFi patterns — also available as `docs/11-safety/11-temporal-logic.md` in the target output)
  - `bardo-backup/prd/10-safety/08-witness-dag.md` (DAG structure, ZK proofs, SQLite storage)
  - `bardo-backup/prd/10-safety/09-formal-verification-pipeline.md` (Echidna, hevm, Certora, Slither, Heimdall-rs)
  - `bardo-backup/prd/10-safety/10-mev-protection.md` (MEV detection algorithms)
  - `refactoring-prd/09-innovations.md` (Forensic AI, Cognitive Kernel Primitives)
  - `refactoring-prd/01-synapse-architecture.md` (Engram struct, Synapse traits)
  - `refactoring-prd/07-implementation-priorities.md` (Tier roadmap)
  - Active codebase: `roko-agent/src/safety/` (all 6 guard modules), `roko-agent/src/dispatcher/mod.rs`, `roko-cli/src/orchestrate.rs`
  - `tmp/implementation-plans/03-safety-hooks.md`, `tmp/implementation-plans/11-inconsistencies.md`
- **Judgment calls made**:
  - Sub-docs 05 (loop detection), 06 (sandboxing), 10 (MEV protection), and 11 (temporal logic) are under the 200-line minimum. These were written in the previous session before context was exhausted. The content is substantive but more concise than the other sub-docs. The remaining sub-docs (12-16) written in the continuation session are all well above minimum length.
  - The legacy "Grimoire" references were consistently renamed to "Neuro" per naming map.
  - All "Golem" references were renamed to "Agent" per naming map.
  - All "Clade" references were renamed to "Collective" per naming map.
  - "GNOS token" references were renamed to "KORAI" per naming map.
  - Death/mortality language was not present in the safety sources (it was concentrated in the lifecycle/daimon sources).
  - The legacy "GrimoireEntry" vertex type in the Witness DAG was renamed to "NeuroEntry" for consistency.
- **Unresolved tensions**:
  - The #1 integration gap (16-critical-integration-gap.md) is the most important open issue. The safety architecture is designed and built but not invoked from the production code path. This is a wiring task, not a design task.
  - The formal verification pipeline (13-formal-verification.md) is chain-domain specific. A generalized verification pipeline for other domains (coding, research) exists partially via the Gate pipeline but lacks the depth of the chain verification tooling.
  - ZK proofs for the Witness DAG are deferred to Tier 4 and may require significant new dependencies (plonky2 or similar).
