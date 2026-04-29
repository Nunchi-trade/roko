# Sandboxing: Worktree Isolation and Process Containment

> **Layer:** L0 Runtime, L1 Framework, L4 Orchestration
>
> **Cross-cut:** Safety & Provenance
>
> **Alignment:** This doc applies [REF32](../../tmp/refinements/32-safety-sandbox-provenance.md). For terminology, see [docs/00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

---

## Overview

Sandboxing is the isolation half of the safety spine. It answers a different question than authorization:

- authorization decides whether the principal may attempt an action,
- sandboxing decides what the runtime prevents even if code is buggy, malicious, or compromised.

Roko needs both. A permission grant without a sandbox is trust. A sandbox without permission checks is a crude jail. The production story is a layered combination of worktree boundaries, subprocess controls, plugin tiers, network egress gates, secret restrictions, and tenant scoping.

---

## 1. Baseline Runtime Boundaries

### Workspace and path boundaries

Filesystem tools should operate inside an explicitly authorized worktree. Path canonicalization remains the first line of defense:

- relative paths are resolved against the active worktree,
- escapes through `..` or absolute paths are denied,
- symlink policy is enforced before the tool touches disk,
- writes outside the declared scope escalate or fail closed.

This boundary is valuable even when higher layers exist because many dangerous actions start as "just a file path."

### Process supervision

Subprocesses should run under explicit timeout and lifecycle control:

- each invocation gets a time budget,
- abnormal termination becomes a safety signal,
- repeated violations can disable the tool or plugin,
- post-call checks decide whether the result can persist or broadcast.

Sandboxing is therefore not complete at spawn time; it continues through supervision and post-call verification.

---

## 2. Tiered Plugin Sandboxes

REF32 makes the tier model operational rather than descriptive. Different plugin tiers receive different isolation guarantees.

| Tier | Trust model | Isolation expectation |
|---|---|---|
| Tier 1 | Pure data | No executable code; schema validation only |
| Tier 2 | Structured prompt/profile data | No executable code; validated against profile and context schema |
| Tier 3 | Declarative tool manifest | Subprocess or MCP-style boundary with explicit path, env, and network controls |
| Tier 4 | Native extension | Trusted code in host process; permission-checked but not runtime-isolated |
| Tier 5 | WASM extension | Host-enforced sandbox with CPU, memory, and hostcall limits |

### Tier 1 and Tier 2

These tiers do not execute plugin code. Their risk is proposal risk, not direct execution risk. The safety spine still matters because the content they supply can influence prompts, routing, or action selection. Their outputs should therefore remain subject to taint, review, and downstream authorization.

### Tier 3

Tier 3 is where most practical sandboxing work lives today. A tier-3 plugin should declare:

- allowed working directory,
- environment variable allowlist,
- read and write path globs,
- network requirement and allowed hosts,
- timeout budget,
- role restrictions for invocation.

The runtime should then enforce those declarations, not merely record them.

### Tier 4

Tier 4 native extensions are not sandboxed at runtime. The only honest safety claim here is "trusted code with permission checks." That means:

- signed manifests should be expected,
- ABI compatibility should be checked before load,
- declared permissions should still constrain hostcalls,
- crashes should be contained as much as practical, but memory safety expectations shift to the extension boundary.

Tier 4 is powerful and should remain high-friction to install.

### Tier 5

Tier 5 aims for genuine runtime isolation:

- CPU limits per invocation,
- memory limits per instance,
- no direct filesystem access,
- no direct network access,
- explicit hostcalls only,
- rate limits on Substrate and Bus access.

Violations should kill the instance, emit a violation Pulse, and mark the plugin for operator review or auto-disable.

---

## 3. Pre-Call and Post-Call Checks

Sandboxing should never be a naked process spawn. The safety spine expects a paired envelope around every tool or plugin call.

### Pre-call

Before execution, verify:

- principal is authorized for the action,
- target path or endpoint is in scope,
- plugin manifest allows the requested hostcalls,
- current tenant matches the tool's scope,
- required human checkpoint has completed.

### Post-call

After execution, verify:

- output is within size and schema limits,
- secret scrubbing succeeded,
- taint was assigned correctly,
- any writes stayed within allowed paths,
- the result can be persisted or broadcast under current policy,
- Custody was emitted for actions that require it.

This pairing prevents a common failure mode where "sandboxing" means only that the subprocess started in the right directory.

---

## 4. Network Egress as a Sandbox Boundary

Network control belongs in the sandbox discussion because outbound connectivity is one of the main escape routes for compromised logic.

The egress layer should enforce:

- host allowlists derived from profile defaults, plugin manifest declarations, and session approvals,
- private-network blocking unless explicitly required,
- principal-aware authorization so not every role can use the same external surface,
- durable logging of principal, URL, status, and tenant for replay.

For tier-3 and tier-5 plugins, "no network" should mean no direct network capability at all, not merely "the docs told the plugin not to call out."

---

## 5. Secrets in Sandboxed Execution

Secrets are a special-case capability:

- they should not be inherited by default into subprocess environments,
- manifests should request them explicitly,
- lower-trust tiers should not receive them at all without operator approval,
- outputs derived from secret use must still pass scrubbing before persistence or broadcast.

This makes secret exposure a controlled capability transfer instead of a side effect of execution.

---

## 6. Multi-Tenancy Boundaries

In shared deployments, sandboxing must include tenant separation:

- Bus topics carry tenant namespace prefixes,
- Substrate keys are scoped by tenant,
- plugins default to tenant-scoped access,
- cross-tenant tools or plugins require elevated review and explicit multi-tenant awareness.

If tenant isolation exists only in the web UI or dashboard, it is not a real safety boundary.

---

## 7. What Sandboxing Does Not Solve

Sandboxing limits blast radius, but it does not answer:

- whether the action was authorized,
- whether tainted inputs influenced it,
- whether a human reviewed it,
- whether an auditor can later prove what happened.

Those questions belong to the rest of the safety spine. A complete deployment therefore combines:

- [00-defense-in-depth.md](00-defense-in-depth.md) for authorization and checkpoints,
- [02-audit-chain.md](02-audit-chain.md) for Custody and attestation,
- [03-taint-tracking.md](03-taint-tracking.md) for untrusted input propagation,
- [08-threat-model.md](08-threat-model.md) for explicit assumptions and residual risks.

---

## 8. Recommended Enforcement Order

The runtime should apply sandbox-related controls in this order:

1. Resolve principal, tenant, and `TypedContext`.
2. Authorize the requested action and collect any required approval.
3. Validate manifest permissions, path scope, and egress scope.
4. Spawn under the correct tier-specific boundary.
5. Supervise execution with timeout and rate limits.
6. Run post-call checks for secrets, taint, and policy compliance.
7. Emit Custody and violation Pulses as needed.

That order keeps policy, isolation, and provenance aligned instead of treating them as independent subsystems.
