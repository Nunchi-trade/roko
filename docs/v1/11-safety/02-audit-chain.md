# Cryptographic Audit Trail: Merkle Hash-Chain and Engram Lineage

> **Layer:** L0 Runtime, L3 Harness
>
> **Cross-cut:** Safety & Provenance
>
> **Alignment:** This doc applies [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For the architecture-level provenance model, see [docs/00-architecture/05-provenance-and-attestation.md](../00-architecture/05-provenance-and-attestation.md) and the glossary at [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Overview

The safety spine needs more than a generic audit log. For any auditable action, Roko should be able to reconstruct:

- who initiated it,
- what was authorized,
- which heuristics and claims influenced it,
- which gates and reviews approved it,
- what result was produced,
- and which tainted inputs were still in play.

The canonical durable record for that reconstruction is a **Custody** Engram. Merkle or hash-chain structures are still useful implementation details for tamper evidence, but the operator-facing unit is Custody, not an opaque append-only line.

---

## 1. From Lineage to Custody

Every Engram already carries lineage and provenance. REF32 extends that into an explicit chain-of-custody story for actions that matter.

### Minimal lineage

Lineage answers causal questions:

- Which prompt or plan step led to this action?
- Which tool output or external fetch influenced the decision?
- Which gate verdicts were emitted before the action persisted?

Lineage alone is necessary but not sufficient. It tells you ancestry, not whether the action was authorized or who reviewed it.

### Custody as the auditable action record

Custody closes that gap:

```rust
pub struct Custody {
    pub action: ActionHash,
    pub principal: PrincipalId,
    pub when: Timestamp,
    pub authorized: AuthzEvidence,
    pub why_heuristics: Vec<HeuristicId>,
    pub why_claims: Vec<ClaimId>,
    pub simulation: Option<SimHash>,
    pub gates_passed: Vec<GateVerdict>,
    pub taint: Option<Taint>,
    pub result: Option<ResultHash>,
    pub witness: Option<ChainWitness>,
}
```

The important addition to plain logging is not just "what happened" but "why the runtime believed it was acceptable at the time."

---

## 2. What Requires Custody

Domain profiles decide the full scope, but the baseline rule is simple: if an action is destructive, externally visible, compliance-relevant, or hard to reverse, it should emit Custody.

Typical examples:

- file deletion or overwrite outside trivial workspace edits,
- shell execution with side effects,
- dependency installation,
- network egress carrying user or external data,
- pull request creation or publication,
- production infrastructure writes,
- signing or broadcasting chain transactions,
- external fact claims promoted into durable knowledge.

Lower-risk actions can still emit lightweight audit Pulses, but the spine reserves Custody for actions an operator may later need to defend.

---

## 3. Authorization Evidence

Custody should record not just the final decision but the evidence behind it:

| Evidence field | Meaning |
|---|---|
| Role grant | The principal's standing permission under the active profile |
| Session approval | A previously granted confirmation still in scope |
| One-shot approval | A single-use checkpoint approval |
| Review confirmation | Explicit approval for a destructive or outward-facing action |
| Escalation outcome | Human or system-level override for exceptional cases |

This makes it possible to distinguish:

- an action the system was always allowed to perform,
- an action the user approved once,
- and an action that only happened after escalation.

That distinction is often more important to auditors than the raw action itself.

---

## 4. Attestation Levels

Some Engrams need stronger guarantees than "we persisted them." Attestation attaches a cryptographic statement to the content hash and records who is willing to stand behind it.

```rust
pub enum AttestationLevel {
    LocalAgent,
    OrgRole,
    ChainWitness,
}
```

### LocalAgent

Low-friction signing by the current agent session. Use it for:

- gate verdicts,
- routine safety Pulses,
- local replay checkpoints,
- provisional Custody records before human review completes.

### OrgRole

Human-owned or organization-owned signing authority. Use it for:

- destructive actions that passed review,
- regulated workflows,
- production writes,
- signed exports intended for audit packages.

### ChainWitness

Phase 2+ attestation anchored outside the local deployment. Use it when the deployment needs independently verifiable evidence across operators or organizations, such as shared heuristic contributions or high-value chain operations.

Attestation does not replace Custody. It strengthens the evidentiary weight of specific Engrams in the custody chain.

---

## 5. Pre-Call, Post-Call, and Replay

Custody should bracket the action lifecycle:

1. **Pre-call:** record the principal, target, context, requested permission, and whether confirmation was required.
2. **Execution:** capture the concrete result hash, simulation output, and any plugin or sandbox violations.
3. **Post-call:** persist gate verdicts, secret scrubbing outcomes, taint metadata, and final user review.

That sequence enables faithful replay:

- start from the action hash,
- walk lineage backward to the inputs,
- inspect the exact heuristics and claims cited at the time,
- re-run the verification logic with the recorded taint and attestation state,
- compare the reproduced result to the stored outcome.

Replay matters because incident response should rely on the recorded historical state, not today's recalibrated system.

---

## 6. Taint, Secrets, and Egress in the Audit Story

Custody is where multiple safety strands meet:

- **Taint:** a custody record should note whether the action depended on `UserInput`, `ExternalFetch`, `ThirdPartyPlugin`, or other tainted sources.
- **Secrets:** custody should confirm that secret-bearing outputs were scrubbed before persistence or broadcast.
- **Network egress:** outbound requests should capture principal, destination, status, and whether the destination required explicit approval.

This is the difference between a forensic log and a compliance-grade chain of evidence. Auditors do not just need a timestamp; they need to know whether the system acted on unreviewed input, whether it crossed a trust boundary, and whether it did so under proper authorization.

---

## 7. Hash-Chains and Witnesses

Hash-chains remain useful, but they are subordinate to the custody model:

- a Merkle or append-only chain can prove ordering and tamper evidence,
- content-addressed Engrams prove object integrity,
- attestations prove signer intent,
- chain witnesses prove external commitment when required.

The recommended hierarchy is:

1. Engram content hash for object identity.
2. Lineage for causal structure.
3. Custody for action-centric evidence.
4. Attestation for signer-backed assurance.
5. External witness for cross-deployment verifiability.

Treating a linear audit chain as the whole provenance story is too weak for multi-agent, multi-surface, or regulated deployments.

---

## 8. Audit Tooling

The safety spine should expose audit queries at the same abstraction level it stores them:

```bash
roko custody list --after 7d --principal user:alice
roko custody show <action-hash>
roko custody verify <action-hash>
roko custody export --signed
roko attest verify <engram-hash>
roko network log --tail 100
```

Expected properties:

- `list` is filterable by action, principal, tenant, and date.
- `show` expands lineage, taint, gate verdicts, and approvals.
- `verify` re-checks signatures, chain witnesses, and replay assumptions.
- `export --signed` produces a package a third party can validate without trusting the UI.

---

## 9. Incident Response

Postmortems should begin with Custody, not with grep:

1. Identify the action hash or affected result hash.
2. Load the custody record and its lineage.
3. Inspect the authorization evidence and review outcome.
4. Check taint sources and any external fetches.
5. Verify attestations and witness status.
6. Replay the action with the recorded inputs.
7. Publish a postmortem Engram linked back into the same lineage.

When the safety spine works, incidents turn into learnable evidence instead of irrecoverable ambiguity.

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md)
- [03-taint-tracking.md](03-taint-tracking.md)
- [06-sandboxing.md](06-sandboxing.md)
- [08-threat-model.md](08-threat-model.md)
- [docs/00-architecture/05-provenance-and-attestation.md](../00-architecture/05-provenance-and-attestation.md)
