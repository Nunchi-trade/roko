# Threat Model: 21 Failure Modes and Attack Trees

> **Layer:** Cross-cut
>
> **Cross-cut:** Safety & Provenance
>
> **Alignment:** This doc applies [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For shared terminology, see [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Overview

This threat model is scoped to the REF32 safety spine. Its purpose is not just to enumerate attacks, but to state which defenses are expected to stop them and which assumptions remain outside the model.

The chapter-level stance is:

- safety-critical actions must pass authorization,
- untrusted execution must stay inside declared sandboxes,
- untrusted data must remain tainted until reviewed,
- auditable actions must emit Custody with optional attestation,
- and cross-tenant or outbound effects must be visible after the fact.

If a deployment cannot satisfy those properties, it should not claim a production-grade safety posture.

---

## 1. Trust Assumptions

### Assumed trusted

- The host running the binary.
- Kernel-level default implementations for Substrate, Bus, and the safety layer.
- Operator-controlled role keys and secret store integrations.
- Signed, reviewed native extensions after installation.

### Assumed untrusted

- User prompts and pasted content.
- Remote model outputs.
- Third-party web content and API responses.
- Third-party plugins until their tier-specific controls say otherwise.
- Cross-tenant data in shared deployments.

### Outside the model

- Physical disk access by an attacker.
- Host root compromise.
- Upstream package ecosystem compromise.
- Side-channel resistance beyond what the host OS and platform already provide.

This boundary matters because threat models become misleading when they quietly rely on protections they do not actually implement.

---

## 2. Primary Adversaries

| Adversary | Goal | Typical path |
|---|---|---|
| Prompt injector | Redirect behavior through untrusted input | user content, fetched content, model output |
| Plugin attacker | Escape declared capability envelope | manifest abuse, native extension misuse, WASM hostcall abuse |
| Credential harvester | Exfiltrate secrets or sensitive outputs | tool output, logs, egress, plugin environment |
| Tenant breaker | Access another tenant's data or effects | namespace confusion, shared plugin state, weak authz |
| Review bypasser | Cause high-risk action without meaningful human approval | approval scope confusion, replayed consent, hidden side effects |
| Provenance attacker | Obscure who acted and why | missing Custody, weak attestation, poor replayability |

These adversaries overlap. A real incident often combines two or more of them.

---

## 3. Attack Surfaces and Expected Mitigations

### Prompt injection and tainted action

Attack path:

1. Untrusted content enters through user input, an external fetch, or plugin output.
2. The content influences composition or action selection.
3. The system attempts a high-risk action as if the input were trustworthy.

Expected mitigations:

- taint assignment at SENSE,
- taint propagation through COMPOSE and ACT,
- confirm, review, or escalate at action time,
- Custody recording the active taint state.

### Plugin sandbox escape

Attack path:

1. A tier-3, tier-4, or tier-5 plugin receives more capability than intended.
2. It reads unauthorized files, exceeds hostcall scope, or reaches the network.
3. The action leaves the declared envelope without prompt visibility.

Expected mitigations:

- manifest-scoped path and egress permissions,
- tier-specific isolation,
- violation Pulses and auto-disable behavior,
- tenant-aware authorization below the UI layer.

### Credential exfiltration

Attack path:

1. Secret-bearing material enters a tool result, plugin output, or prompt.
2. The system logs, persists, or transmits it.
3. The secret leaves the trusted boundary.

Expected mitigations:

- secret-typed wrappers and redaction,
- post-call scrubbing,
- outbound egress control,
- audit of secret access and transmission.

### Cross-tenant bleed

Attack path:

1. A principal or plugin acts with ambiguous tenant scope.
2. Substrate keys, Bus topics, or cached state cross namespaces.
3. Data or side effects leak between tenants.

Expected mitigations:

- tenant-prefixed topics and storage,
- tenant-scoped plugin defaults,
- authorization that includes tenant in the target,
- review for multi-tenant-aware plugins or actions.

### Provenance failure

Attack path:

1. High-risk action executes.
2. Authorization evidence or review scope is not durably recorded.
3. Replay cannot prove what happened or whether it was approved.

Expected mitigations:

- Custody for auditable actions,
- attestation for higher-assurance records,
- queryable lineage and replay tooling,
- signed export paths for third-party review.

---

## 4. Human Checkpoint Failure Modes

Human-in-the-loop controls are themselves attack surfaces if badly scoped.

Key failure modes:

- a one-shot approval silently becomes a session-wide approval,
- a permission prompt hides the true target or blast radius,
- a review prompt omits the visible side effects,
- approval is attached to the principal but not to the concrete action and target,
- the resulting approval is not stored in Custody.

The defense is not "ask the user more often." The defense is precise scope:

- show the principal,
- show the target,
- show whether the approval is once, session, or escalation,
- and store that exact scope durably.

---

## 5. Residual Risks

The safety spine improves the posture materially, but it does not remove all risk.

| Risk | Why it remains |
|---|---|
| Trusted native extension compromise | Tier 4 relies on installer trust more than runtime isolation |
| Host compromise | Sandboxes and authz inside the process cannot defend against root |
| Supply-chain compromise | Signed manifests help, but upstream dependency compromise remains external |
| Approval fatigue | Human checkpoints still degrade if the prompts are noisy or poorly scoped |
| Novel taint laundering patterns | Summaries and multi-hop transformations can hide risky provenance if propagation is incomplete |
| Misconfigured tenant boundaries | Shared deployments are brittle if namespace rules are inconsistently applied |

Residual-risk tracking should be explicit in deployment reviews, not buried in footnotes.

---

## 6. Incident Response Expectations

The threat model is only useful if it drives post-incident behavior.

Minimum response flow:

1. Identify the affected action or result hash.
2. Load the associated Custody record.
3. Confirm the authorization source and human checkpoint scope.
4. Inspect taint sources, plugin tier, egress logs, and tenant namespace.
5. Verify attestation and replayability.
6. Contain by tightening permissions, disabling the plugin, or reducing egress scope.
7. Publish a postmortem Engram linked into the same lineage.

That response flow turns the safety spine into an operational system rather than a design diagram.

---

## 7. Deployment Review Questions

Before calling a deployment production-ready, a reviewer should be able to answer "yes" to these:

- Are destructive and externally visible actions covered by Custody?
- Do role authorization decisions include principal, target, `TypedContext`, and tenant?
- Are tier-3, tier-4, and tier-5 plugins treated as distinct trust classes?
- Is outbound network egress centrally controlled and logged?
- Do secrets stay redacted in storage, logs, and Pulses?
- Can tainted inputs reach high-risk actions without review?
- Can cross-tenant access be denied at the Bus and Substrate layers?
- Can the operator replay a disputed action and verify its attestation state?

If any answer is no, the missing control should be treated as a named gap, not an implementation detail.

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md)
- [02-audit-chain.md](02-audit-chain.md)
- [03-taint-tracking.md](03-taint-tracking.md)
- [06-sandboxing.md](06-sandboxing.md)
