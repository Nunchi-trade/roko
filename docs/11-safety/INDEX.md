# Safety & Provenance

> **Abstract:** Safety in Roko is a single spine, not a grab-bag of guards. The safety chapter ties together trait-level authorization, human-in-the-loop checkpoints, per-tier plugin sandboxes, taint propagation, attestation, chain-of-custody, network egress control, secret handling, and multi-tenant isolation so an operator can answer: who did what, with what authorization, on what inputs, and with what consequence?
>
> **Alignment:** This chapter reflects [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For shared vocabulary, see [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Prerequisites

Before reading this topic, readers should be familiar with:

- The two-medium / two-fabric framing: Engrams persist in Substrate, Pulses move on the Bus.
- The seven-step loop: SENSE, ASSESS, COMPOSE, ACT, VERIFY, PERSIST and BROADCAST, REACT.
- `TypedContext`, domain profiles, and role separation in the architecture chapter.

Recommended companion docs:

- `docs/00-architecture/05-provenance-and-attestation.md`
- `docs/00-architecture/09-universal-cognitive-loop.md`
- `docs/00-architecture/25-attention-as-currency.md`
- `docs/00-architecture/26-cognitive-immune-system.md`

---

## Table of Contents

### Safety Spine

| # | Sub-doc | Description |
|---|---|---|
| 00 | [00-defense-in-depth.md](00-defense-in-depth.md) | Canonical safety spine: authorization, isolation, provenance, pre/post checks, checkpoints, egress, secrets, and tenant boundaries |
| 01 | [01-capability-tokens.md](01-capability-tokens.md) | Type-level capability design and permission tokens |
| 02 | [02-audit-chain.md](02-audit-chain.md) | Custody records, attestation levels, replay, and exportable audit evidence |
| 03 | [03-taint-tracking.md](03-taint-tracking.md) | Taint propagation from untrusted inputs through composition, action, persistence, and review |

### Runtime Controls

| # | Sub-doc | Description |
|---|---|---|
| 04 | [04-permits-allowlists.md](04-permits-allowlists.md) | Permit matrices and allowlist mechanics for tools and resources |
| 05 | [05-loop-detection.md](05-loop-detection.md) | Rate limits, breakers, and detection of runaway behavior |
| 06 | [06-sandboxing.md](06-sandboxing.md) | Tiered plugin sandboxes, worktree and path isolation, egress gates, and tenant-scoped runtime boundaries |

### Attack Surface

| # | Sub-doc | Description |
|---|---|---|
| 07 | [07-prompt-security.md](07-prompt-security.md) | Prompt-injection defenses and safe prompt composition |
| 08 | [08-threat-model.md](08-threat-model.md) | Threat assumptions, adversary paths, residual risks, and incident response for the safety spine |
| 09 | [09-adaptive-risk.md](09-adaptive-risk.md) | Dynamic risk budgets and escalation based on uncertainty and blast radius |

### Domain-Specific Safety

| # | Sub-doc | Description |
|---|---|---|
| 10 | [10-mev-protection.md](10-mev-protection.md) | Chain-domain transaction safety and MEV defenses |
| 11 | [11-temporal-logic.md](11-temporal-logic.md) | Runtime monitors and temporal properties for agent behavior |
| 12 | [12-witness-dag.md](12-witness-dag.md) | Rich causal replay structures and witness queries |
| 13 | [13-formal-verification.md](13-formal-verification.md) | Formal methods and contract-oriented verification for high-risk domains |

### Advanced Safety

| # | Sub-doc | Description |
|---|---|---|
| 14 | [14-cognitive-kernel-safety.md](14-cognitive-kernel-safety.md) | Cognitive namespaces, scheduling, and kernel-level enforcement |
| 15 | [15-forensic-ai.md](15-forensic-ai.md) | Replay, postmortems, and regulator-facing forensic workflows |
| 16 | [16-critical-integration-gap.md](16-critical-integration-gap.md) | The remaining wiring needed for end-to-end safety enforcement |

---

## Chapter Through-Line

This chapter should be read as one defensive story:

1. `00-defense-in-depth.md` defines the spine and the shared vocabulary.
2. `02-audit-chain.md` explains how high-risk actions become durable Custody Engrams with attestation.
3. `03-taint-tracking.md` explains how untrusted inputs stay marked until explicitly reviewed.
4. `06-sandboxing.md` explains how plugin tiers, subprocesses, files, network, and tenants are isolated.
5. `08-threat-model.md` states what Roko trusts, what it treats as hostile, and what remains outside the model.

The rest of the safety chapter deepens those controls for specific attack classes or domains rather than replacing them.

---

## Key Decisions

1. Safety is enforced where the agent cannot wish it away: authorization checks, sandboxes, gates, and Substrate persistence.
2. High-risk actions must be explainable after the fact through Custody, lineage, taint, and attestation.
3. Plugin safety is tiered. Trusted native code, declarative tools, and WASM extensions do not share the same trust model.
4. Human approval is part of the permission system, not an afterthought. Confirm, allow-once, and escalate are first-class outcomes.
5. Cross-tenant isolation and secret scrubbing are enforced below the UI so the same guarantees hold in CLI, TUI, web, and automation.

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md)
- [02-audit-chain.md](02-audit-chain.md)
- [03-taint-tracking.md](03-taint-tracking.md)
- [06-sandboxing.md](06-sandboxing.md)
- [08-threat-model.md](08-threat-model.md)
- [docs/00-architecture/05-provenance-and-attestation.md](../00-architecture/05-provenance-and-attestation.md)
- [docs/00-architecture/26-cognitive-immune-system.md](../00-architecture/26-cognitive-immune-system.md)
