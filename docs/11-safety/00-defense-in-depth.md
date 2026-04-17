# Defense-in-Depth: Architectural Safety for Autonomous Agents

> **Layer:** L0 Runtime, L1 Framework, L3 Harness, L4 Orchestration
>
> **Cross-cut:** Safety & Provenance
>
> **Alignment:** This doc is the chapter entrypoint for [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For naming, see [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Overview

Safety in Roko is a spine that runs across every layer and every speed of the seven-step loop. It is not just "the gate pipeline" and it is not just "sandboxing." The same vocabulary governs:

- Who is allowed to act.
- Where untrusted code is allowed to run.
- How dangerous inputs stay marked as tainted.
- Which actions require a human checkpoint.
- What durable evidence exists after the action completes.

The three concerns are distinct but intentionally stitched together:

| Concern | Question | Primary enforcement point |
|---|---|---|
| Authorization | May this principal perform this action on this target in this context? | Trait-level authz in the safety layer |
| Isolation | If code is untrusted or partially trusted, can it escape its declared envelope? | Worktree, process, container, and WASM boundaries |
| Provenance | Can an auditor reconstruct what happened and why? | Engram lineage, Custody records, taint metadata, and attestation |

This chapter uses "defense in depth" literally: no single guard is assumed sufficient. An action that matters should be subject to authorization, pre-call validation, post-call verification, taint-aware policy, and durable audit evidence.

---

## 1. Shared Permission Vocabulary

Every permission-gated action is evaluated against the same tuple:

- **Principal:** user id, agent id, or plugin id.
- **Action:** a controlled verb such as file read, file write, shell execution, dependency install, Bus publish, or network egress.
- **Target:** file path, tool id, topic, endpoint, Engram kind, or tenant namespace.
- **Context:** the `TypedContext` and domain profile active at decision time.
- **Authorization source:** role grant, session approval, one-shot approval, escalation, or plugin manifest declaration.

The core decision space is intentionally small:

```rust
pub enum AuthzDecision {
    Allow,
    AllowWithConfirm { prompt: String },
    AllowOnce,
    Deny { reason: String },
    Escalate { to: EscalationTarget },
}
```

The load-bearing property is that nothing "just happens." If the answer is conditional, the UI must surface that condition. If the answer is uncertain or high-risk, the action escalates instead of silently proceeding.

### Role authorization

The default policy is deny-by-default with profile-specific widening. Typical defaults:

- Workspace reads are broadly allowed.
- Workspace writes are limited to implementers and approved operators.
- Shell execution, dependency installation, and external API calls require confirm or escalate.
- High-risk actions such as destructive deletes, production writes, or chain signing always force review, even if the session already approved a narrower action.

Role authorization is therefore not a static ACL. It is a live decision that combines role, `TypedContext`, and the resource being touched.

---

## 2. Human-in-the-Loop Checkpoints

Human checkpoints are part of the permission model, not an exception to it.

### Permission checkpoint

Used when a decision is `AllowWithConfirm` or `AllowOnce`. The prompt must explain:

- Which principal is asking.
- Which action and target are in scope.
- Which heuristic, claim, or plan step led to the request.
- Whether approval applies once or only to the precise scope shown.

### Ambiguity checkpoint

Used when the model can proceed but the choice between options is under-specified or low-confidence. The checkpoint does two jobs:

- It prevents accidental commitment under uncertainty.
- It turns the user's choice into a future calibration signal.

### Review checkpoint

Used before destructive or externally visible actions such as deleting files, creating pull requests, publishing artifacts, sending outbound content, or signing chain actions. Prior permission does not waive review here. The user should be able to inspect the diff, parameters, and intended effect before final confirmation.

---

## 3. Pre-Call and Post-Call Enforcement

Every tool invocation should be wrapped by a paired safety envelope:

| Stage | Purpose | Examples |
|---|---|---|
| `safety.pre_call` | Stop disallowed actions before they execute | authorization, path checks, manifest permissions, egress allowlist, timeout budget |
| `safety.post_call` | Validate what actually happened before it becomes durable or actionable | secret scrubbing, taint assignment, output validation, result-size checks, custody emission |

This pair matters because pre-call checks reason about intent while post-call checks reason about consequence. Safe parameters can still yield dangerous output; dangerous intent should never run just because the eventual output might have been harmless.

Pre/post enforcement also provides the bridge between isolation and provenance:

- The pre-call gate records what was authorized.
- The post-call gate records what actually occurred.
- `02-audit-chain.md` stores the result as queryable Custody.

---

## 4. Safety Along the Seven-Step Loop

The safety spine is not a separate loop. It cuts through the existing seven steps:

| Loop step | Safety responsibilities |
|---|---|
| SENSE | Tag inbound data with taint, tenant namespace, and source metadata. |
| ASSESS | Apply role checks, conflict-of-interest rules, and risk-aware routing. |
| COMPOSE | Preserve taint in composed prompts and include only context allowed for the principal and tenant. |
| ACT | Enforce pre-call checks, sandbox limits, egress policy, and checkpoint requirements. |
| VERIFY | Run gate verdicts, attach review outcomes, and decide whether the result can persist or broadcast. |
| PERSIST / BROADCAST | Persist Engrams and Custody records in Substrate; publish Pulses only on allowed topics and namespaces. |
| REACT | Tighten permissions, disable plugins, or open incidents in response to verdicts, violations, or tainted outputs. |

Safety therefore lives at the point of action and in the after-action consequences, not only at the end of the turn.

---

## 5. Cross-Cutting Controls

### Network egress

All outbound network traffic should cross one egress shim. The shim evaluates:

- Whether the principal is allowed to perform network egress at all.
- Whether the destination host is on the profile or session allowlist.
- Whether the request leaves the current tenant or compliance boundary.
- Whether the action should produce a review checkpoint because it transmits user-controlled or tainted content.

Every outbound request should also emit a safety Pulse so dashboards and replay tools can answer who accessed which endpoint and when.

### Secrets

Secrets are not ordinary strings. The safety model assumes:

- Secret-typed values render as redacted in logs and UI surfaces.
- Substrate persistence and Bus publication scrub secret fields before emission.
- Plugins do not receive secrets unless their tier and manifest explicitly allow it and an operator approved that scope.
- Secret access itself is auditable.

### Multi-tenancy

Multi-tenant deployments cannot rely on the UI to separate data. Namespace separation belongs in Substrate keys, Bus topics, plugin scope, and authorization decisions. A tenant-scoped tool may be perfectly safe inside one namespace and a data leak in another.

---

## 6. Defense Layers

The concrete defensive stack, from lowest to highest:

1. **Path and workspace boundaries:** file access stays inside the authorized worktree.
2. **Process and runtime controls:** timeouts, resource limits, and subprocess supervision prevent runaway execution.
3. **Plugin sandbox tiers:** declarative tools, native extensions, and WASM extensions each get a different trust envelope.
4. **Policy and gate checks:** pre-call, post-call, taint-aware blocking, and review checkpoints.
5. **Custody and attestation:** durable evidence of authorization, heuristics, claims, verdicts, taint, and outcome.
6. **Threat monitoring:** residual-risk tracking, incident response, and replay.

If one layer fails, the next should either block the action or preserve enough evidence to contain and explain it.

---

## 7. What This Chapter Covers

The rest of the safety chapter decomposes the spine:

- [02-audit-chain.md](02-audit-chain.md) turns high-risk actions into durable Custody with optional attestation.
- [03-taint-tracking.md](03-taint-tracking.md) defines how untrusted inputs stay marked until reviewed.
- [06-sandboxing.md](06-sandboxing.md) defines plugin tiers, egress control, and runtime isolation.
- [08-threat-model.md](08-threat-model.md) states the trust assumptions, attack surfaces, and residual risks.

Together they support the operator-facing question that REF32 makes load-bearing: who did what, with what authorization, and with what consequence?
