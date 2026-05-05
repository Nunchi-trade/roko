# 17-security — Depth Index

> Depth for [16-SECURITY.md](../../unified/16-SECURITY.md). Safety as compositions of Verify Cells in Pipeline Graphs, capability intersection, taint lattice IFC, and adaptive risk Loops.

---

## Depth docs (7)

| # | Filename | Covers |
|---|---|---|
| 01 | [immune-system-as-graph.md](immune-system-as-graph.md) | 5-layer immune pipeline as Graph (taint propagation → anomaly detection → quarantine → incident response → immune memory), HDC fingerprint matching, autoimmune protection |
| 02 | [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) | 7-layer defense as Pipeline Graph of Verify Cells, permits/allowlists, sandboxing, rate limiting, early-exit semantics, critical integration gap resolution |
| 03 | [03-capability-taint-and-ifc.md](03-capability-taint-and-ifc.md) | Capability&lt;T&gt; tokens as Cell-declared capabilities, taint taxonomy (5-fold lattice), three-layer capability intersection, CaMeL IFC, taint flow through Bus |
| 04 | [04-audit-witness-and-forensics.md](04-audit-witness-and-forensics.md) | Custody chain as Store lineage, witness DAG (5 vertex types), forensic replay as Lens, regulatory pre-compliance mapping |
| 05 | [05-adaptive-risk-as-loop.md](05-adaptive-risk-as-loop.md) | 5-layer runtime risk as Loop with predict-publish-correct, circuit breaker React Cell, LTL/CTL temporal logic monitoring, adaptive gate thresholds EMA |
| 06 | [06-prompt-security-and-camel.md](06-prompt-security-and-camel.md) | CaMeL dual-LLM as Extension with taint barrier, ventriloquist defense via Store+Verify, tool-guard Pipeline (schema→content→semantic) |
| 07 | [07-cognitive-kernel-and-formal-methods.md](07-cognitive-kernel-and-formal-methods.md) | Cognitive namespaces as Space, cognitive signals as Pulses, EDF scheduling, formal verification Pipeline (Heimdall→Slither→Echidna→hevm→Certora), MEV pre-flight Verify Cell |

---

## Source docs (17)

### Defense in depth and sandboxing

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/00-defense-in-depth.md` | **Absorbed** | [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) |
| `docs/11-safety/04-permits-allowlists.md` | **Absorbed** | [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) |
| `docs/11-safety/06-sandboxing.md` | **Absorbed** | [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) |
| `docs/11-safety/14-cognitive-kernel-safety.md` | **Absorbed** | [07-cognitive-kernel-and-formal-methods.md](07-cognitive-kernel-and-formal-methods.md) |

### Capability and access control

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/01-capability-tokens.md` | **Absorbed** | [03-capability-taint-and-ifc.md](03-capability-taint-and-ifc.md) |
| `docs/11-safety/03-taint-tracking.md` | **Absorbed** | [03-capability-taint-and-ifc.md](03-capability-taint-and-ifc.md) |

### Audit and provenance

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/02-audit-chain.md` | **Absorbed** | [04-audit-witness-and-forensics.md](04-audit-witness-and-forensics.md) |
| `docs/11-safety/12-witness-dag.md` | **Absorbed** | [04-audit-witness-and-forensics.md](04-audit-witness-and-forensics.md) |
| `docs/11-safety/15-forensic-ai.md` | **Absorbed** | [04-audit-witness-and-forensics.md](04-audit-witness-and-forensics.md) |

### Detection and risk

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/05-loop-detection.md` | **Absorbed** | [05-adaptive-risk-as-loop.md](05-adaptive-risk-as-loop.md) |
| `docs/11-safety/09-adaptive-risk.md` | **Absorbed** | [05-adaptive-risk-as-loop.md](05-adaptive-risk-as-loop.md) |
| `docs/11-safety/11-temporal-logic.md` | **Absorbed** | [05-adaptive-risk-as-loop.md](05-adaptive-risk-as-loop.md) |
| `docs/00-architecture/26-cognitive-immune-system.md` | **Absorbed** | [immune-system-as-graph.md](immune-system-as-graph.md) |

### Threat model and prompt security

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/07-prompt-security.md` | **Absorbed** | [06-prompt-security-and-camel.md](06-prompt-security-and-camel.md) |
| `docs/11-safety/08-threat-model.md` | **Absorbed** | [06-prompt-security-and-camel.md](06-prompt-security-and-camel.md) |
| `docs/11-safety/10-mev-protection.md` | **Absorbed** | [07-cognitive-kernel-and-formal-methods.md](07-cognitive-kernel-and-formal-methods.md) |

### Verification and forensics

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/13-formal-verification.md` | **Absorbed** | [07-cognitive-kernel-and-formal-methods.md](07-cognitive-kernel-and-formal-methods.md) |

### Integration gap

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/11-safety/16-critical-integration-gap.md` | **Absorbed** | [02-defense-in-depth-as-pipeline.md](02-defense-in-depth-as-pipeline.md) |

---

17 of 17 source docs absorbed across 7 depth docs.
