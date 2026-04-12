# Cognitive Kernel Primitives: Safety Implications

> **Layer**: L0 Runtime (kernel primitives), L1 Framework (capability enforcement), L3 Harness (signal-based intervention)
>
> **Crate**: Cross-cutting: `roko-runtime` (scheduling, signals), `roko-core` (namespaces, syscalls), `roko-agent` (enforcement)
>
> **Synapse traits**: `Policy` (syscall enforcement), `Substrate` (namespace isolation), `Gate` (signal-driven verification)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [04-permits-allowlists.md](04-permits-allowlists.md), [06-sandboxing.md](06-sandboxing.md)

---

## Overview

Roko implements OS-level primitives for agents that no other agent framework provides. These primitives — Cognitive Namespaces, Cognitive Signals, Cognitive Scheduling, and Engram Syscalls — are inspired by what Linux got right for process management and adapted for cognitive agent management.

This document covers the **safety implications** of each primitive: how namespaces prevent knowledge leakage, how signals enable safe intervention, how scheduling prevents resource starvation, and how syscalls provide a single enforcement point for all agent actions.

The key insight: just as a Unix kernel mediates all hardware access through system calls, Roko mediates all cognitive actions through Engram Syscalls. Every file write, API call, knowledge posting, and tool invocation passes through a controlled interface where security, auditing, rate limiting, and cost tracking are enforced uniformly.

---

## Cognitive Namespaces

### What They Are

Cognitive Namespaces are isolated knowledge spaces with explicit, auditable cross-namespace channels. They are the knowledge-domain equivalent of Linux namespaces (which isolate processes' view of the system into separate PID, network, mount, and user namespaces).

```rust
/// A cognitive namespace: an isolated knowledge domain.
pub struct CognitiveNamespace {
    /// Unique identifier for this namespace.
    pub id: NamespaceId,
    /// The Substrate (storage) backing this namespace.
    /// Each namespace has its own Substrate instance.
    pub substrate: Arc<dyn Substrate>,
    /// Access control list: who can read/write.
    pub acl: AccessControlList,
    /// Explicit cross-namespace channels.
    /// Knowledge only flows between namespaces through these.
    pub channels: Vec<NamespaceChannel>,
}

/// A channel between two namespaces with explicit transfer rules.
pub struct NamespaceChannel {
    /// Source namespace.
    pub from: NamespaceId,
    /// Destination namespace.
    pub to: NamespaceId,
    /// Filter: which Engram kinds can flow through.
    pub allowed_kinds: Vec<Kind>,
    /// Whether transfers are logged to the audit chain.
    pub audit_transfers: bool,
    /// Maximum transfer rate (Engrams per second).
    pub rate_limit: Option<f64>,
}

/// Access control for a namespace.
pub struct AccessControlList {
    /// Agent roles that can read from this namespace.
    pub readers: Vec<AgentRole>,
    /// Agent roles that can write to this namespace.
    pub writers: Vec<AgentRole>,
    /// Whether anonymous (unauthenticated) reads are allowed.
    pub allow_anonymous_read: bool,
}
```

### Safety Properties

**Isolation guarantee.** An agent's private knowledge is isolated within its namespace. No other agent can access it without an explicit channel. This prevents:
- Knowledge poisoning: a compromised agent cannot directly corrupt another agent's Neuro store
- Information leakage: proprietary strategies stay within their namespace
- Cross-contamination: experimental knowledge (Dreams output, hypothesis fragments) cannot accidentally pollute production knowledge

**Explicit channels.** Knowledge sharing happens only through declared channels that log every transfer. This provides:
- Audit trail: every cross-namespace knowledge transfer is recorded with timestamp, source, destination, and content hash
- Rate limiting: channels can limit transfer rate to prevent flooding
- Kind filtering: only specific Engram kinds (e.g., Insight but not StrategyFragment) can flow through a channel
- Directionality: channels are one-way, preventing unintended bidirectional leakage

**Namespace hierarchy for Collectives.** In a Collective (a group of cooperating agents on the Korai network):
- Each agent has a private namespace
- The Collective has a shared namespace
- The public Korai network is a global namespace
- Knowledge flows: private → shared (controlled by channel policy) → public (controlled by posting policy)

### Relation to Existing Safety Guards

The existing PathPolicy (see [06-sandboxing.md](06-sandboxing.md)) provides filesystem-level isolation. Cognitive Namespaces extend this to knowledge-level isolation. The two compose:
- PathPolicy prevents an agent from reading files outside its worktree
- CognitiveNamespace prevents an agent from reading Engrams outside its namespace
- Together, they enforce both physical and logical isolation

---

## Cognitive Signals (Typed Interrupts)

### What They Are

Cognitive Signals are typed interrupts that alter agent behavior without killing the process. They are the cognitive equivalent of Unix signals (SIGTERM, SIGKILL, SIGSTOP, SIGCONT), but instead of process lifecycle signals, they are cognitive lifecycle signals.

```rust
/// Typed interrupts for cognitive agents.
/// Unlike process signals, these alter behavior rather than
/// process lifecycle.
pub enum CognitiveSignal {
    /// Suspend reasoning, serialize state to disk.
    /// Agent can be resumed later from the serialized state.
    /// Analogous to SIGSTOP but with state preservation.
    Pause,

    /// Resume from serialized state.
    /// Loads state from the snapshot and continues execution.
    /// Analogous to SIGCONT.
    Resume,

    /// Change current task priority.
    /// A critical production incident preempts routine work.
    Reprioritize(TaskId),

    /// Add context mid-reasoning.
    /// Inject an Engram into the agent's active context
    /// without interrupting the current reasoning chain.
    InjectContext(Engram),

    /// Switch to stronger model immediately.
    /// Forces T2 routing regardless of current uncertainty level.
    /// Used when an operator detects a situation requiring
    /// deeper reasoning.
    Escalate,

    /// Reduce arousal, slow down.
    /// Modulates the Daimon PAD vector to decrease arousal,
    /// causing the agent to favor safer, more conservative actions.
    Cooldown,

    /// Switch to exploratory mode.
    /// Increases the agent's exploration budget, causing it to
    /// try novel approaches rather than exploiting known strategies.
    Explore,

    /// Graceful termination.
    /// Agent completes current work unit, persists state,
    /// and shuts down cleanly.
    Shutdown,
}
```

### Safety Properties

**Human oversight (EU AI Act Article 14).** Cognitive Signals provide the mechanism for human oversight of autonomous agents. An operator can:
- **Pause** an agent mid-execution if behavior seems anomalous
- **InjectContext** to provide new safety constraints without restarting
- **Escalate** to force deeper reasoning when the agent is cutting corners
- **Cooldown** to reduce risk-taking when market conditions are volatile
- **Shutdown** for graceful termination with full state preservation

This directly satisfies the EU AI Act Article 14 requirement for human oversight mechanisms, as detailed in the Forensic AI regulatory mapping (see [15-forensic-ai.md](15-forensic-ai.md)).

**Non-destructive intervention.** Unlike Unix SIGKILL, no Cognitive Signal causes abrupt termination with state loss. Even Shutdown allows the agent to finish its current work unit. This prevents the safety risk of killing an agent mid-transaction (which could leave positions unwound, files half-written, or states inconsistent).

**Signal delivery guarantees.** Signals are delivered through the event bus in `roko-runtime`, which provides:
- Ordered delivery: signals arrive in the order they were sent
- At-least-once delivery: signals are persisted before acknowledgment
- Timeout detection: if a signal is not acknowledged within a configurable timeout, the runtime escalates (e.g., Pause that is not acknowledged becomes Shutdown)

**Signal priority.** Signals have an implicit priority ordering:

| Signal | Priority | Effect on Current Work |
|---|---|---|
| Shutdown | 1 (highest) | Complete current unit, then exit |
| Pause | 2 | Serialize state, suspend immediately |
| Escalate | 3 | Switch model tier, continue work |
| Cooldown | 4 | Modulate affect, continue work |
| Reprioritize | 5 | Reorder task queue, continue work |
| InjectContext | 6 | Add to context, continue work |
| Explore | 7 | Change exploration mode, continue work |
| Resume | 8 (lowest) | Resume from suspended state |

Higher-priority signals preempt lower-priority ones. If a Shutdown arrives while a Cooldown is being processed, the Shutdown takes precedence.

### Integration with Existing Safety Architecture

Cognitive Signals integrate with the existing Gate and conductor systems:

- The **conductor** (circuit breaker in `roko-conductor`) can emit Cooldown signals when health metrics deteriorate
- The **Gate pipeline** can emit Escalate signals when verification confidence is low
- The **DiagnosisEngine** (in `roko-conductor`) can emit Pause signals when it detects anomalous behavior patterns
- The **adaptive risk system** (see [09-adaptive-risk.md](09-adaptive-risk.md)) can emit Shutdown signals when risk thresholds are breached

---

## Cognitive Scheduling

### What It Is

Cognitive Scheduling allocates reasoning resources based on priority, deadline, and expected value. It is the cognitive equivalent of a process scheduler (CFS in Linux), but instead of CPU time, it allocates LLM inference budget and context window space.

```
cognitive_priority = task_urgency × expected_value × (1 / cognitive_cost)
```

A critical production incident preempts routine report generation. The scheduler reasons about cognitive cost — deep reasoning chains get more budget than routine lookups.

### Safety Properties

**Starvation prevention.** The scheduler implements fairness properties analogous to CFS (Completely Fair Scheduler):
- No task can monopolize reasoning resources indefinitely
- Tasks that have been waiting longest get priority boosts
- Minimum time slices ensure every queued task makes progress

These properties correspond to the fairness temporal logic formulas defined in [11-temporal-logic.md](11-temporal-logic.md):
```
GF(queued_task → dispatched)
    "A queued task is infinitely often considered for dispatch"
```

**Deadline enforcement.** Tasks with deadlines (e.g., "respond to this API call within 5 seconds") are scheduled using Earliest Deadline First (EDF). Safety-critical tasks (e.g., unwinding a position before liquidation) always receive deadline priority.

**Cost accounting.** Every reasoning step has a cost (LLM tokens consumed, wall-clock time). The scheduler tracks cumulative cost per task, per agent, and per Collective. When budget limits are reached:
- Budget soft limit: emit a Warning Engram, continue with reduced tier (T2 → T1 → T0)
- Budget hard limit: emit a Cooldown signal, pause non-critical work
- Budget exhaustion: emit a Shutdown signal for non-essential agents

This prevents the runaway cost scenario where an agent spirals into increasingly expensive reasoning without producing results.

**Priority inversion prevention.** When a high-priority task depends on a result from a low-priority task, the scheduler temporarily elevates the low-priority task's priority (priority inheritance protocol). This prevents deadlocks where safety-critical work is blocked by routine processing.

---

## Engram Syscalls

### What They Are

Engram Syscalls are the controlled interface through which every meaningful agent action passes. Just as a Linux process cannot directly access hardware without going through a system call, a Roko agent cannot perform external actions without going through an Engram Syscall.

```
Agent wants to write a file  → Policy.decide() → permit / deny / modify / log
Agent wants to call an API   → Policy.decide() → permit / deny / modify / log
Agent wants to post to Korai → Policy.decide() → permit / deny / modify / log
Agent wants to execute a tool → Policy.decide() → permit / deny / modify / log
```

The `Policy` Synapse trait is the enforcement mechanism:

```rust
/// The Policy trait: observes Engram streams and emits decisions.
/// This is the "system call handler" of the cognitive kernel.
pub trait Policy: Send + Sync {
    /// Decide whether to permit, deny, modify, or log an action.
    /// Receives a batch of Engrams (the action request + context)
    /// and emits new Engrams (the decision, audit records, etc.)
    fn decide(&self, engrams: &[Signal]) -> Vec<Signal>;
    // Note: Signal will be renamed to Engram in Tier 0D
}
```

### Safety Properties

**Single enforcement point.** All agent actions pass through Policy.decide(). This means:
- Security rules are enforced in exactly one place
- There is no way to bypass safety checks by using an alternative code path
- Audit logging captures every action attempt, whether permitted or denied
- Rate limiting applies uniformly across all action types

This is the cognitive kernel equivalent of Linux's system call table — a single choke point where all security policies are enforced.

**Four decision modes.** For each action request, the Policy can:

1. **Permit**: Allow the action to proceed. Log the approval.
2. **Deny**: Block the action. Log the denial with reason. Return an error Engram to the agent.
3. **Modify**: Allow the action but alter it. For example, reduce the size of a trade, add safety headers to an API call, or scrub secrets from output.
4. **Log**: Allow the action but create a detailed audit record. Used for monitoring actions that are permitted but sensitive.

**Composable policies.** Multiple Policy implementations compose:
- The `SafetyLayer` (see [00-defense-in-depth.md](00-defense-in-depth.md)) is a composite Policy that chains BashPolicy, GitPolicy, NetworkPolicy, PathPolicy, ScrubPolicy, and RateLimiter
- Additional policies can be added for specific domains (DeFi position limits, API rate limits, cost caps)
- Policy composition follows the product rule: all policies must agree to permit; any single denial blocks the action

### Relation to Existing ToolDispatcher

The `ToolDispatcher` in `roko-agent/src/dispatcher/mod.rs` already implements the Engram Syscall pattern for tool invocations:

```
dispatch() pipeline:
  1. validate (is this a valid tool call?)
  2. tool_filter (is this tool allowed for this role?)
  3. permission (does the agent have permission?)
  4. safety.check_pre_execution (SafetyLayer pre-checks)
  5. handler (execute the tool)
  6. truncate (enforce output size limits)
  7. safety.scrub_output (ScrubPolicy post-processing)
```

Each step emits an audit Engram via `emit_audit()`. This is the existing Engram Syscall implementation for tool dispatch. The Cognitive Kernel Primitives vision extends this pattern to **all** agent actions, not just tool invocations.

**Current gap.** The ToolDispatcher implements syscall-style enforcement, but it is only invoked when tools are dispatched through it. The #1 integration gap (see [16-critical-integration-gap.md](16-critical-integration-gap.md)) means that `orchestrate.rs` calls `ExecAgent::run()` directly, bypassing the ToolDispatcher and its syscall enforcement. Closing this gap is the highest priority for making the Engram Syscall pattern effective.

---

## Security Model: Defense in Depth via Kernel Primitives

The four Cognitive Kernel Primitives compose into a defense-in-depth model:

```
┌─────────────────────────────────────────────┐
│         Engram Syscalls (outermost)          │
│  Every action passes through Policy.decide() │
│                                              │
│  ┌─────────────────────────────────────────┐ │
│  │     Cognitive Namespaces                │ │
│  │  Knowledge isolation + channels         │ │
│  │                                         │ │
│  │  ┌───────────────────────────────────┐  │ │
│  │  │    Cognitive Scheduling           │  │ │
│  │  │  Fair resource allocation         │  │ │
│  │  │                                   │  │ │
│  │  │  ┌─────────────────────────────┐  │  │ │
│  │  │  │   Cognitive Signals         │  │  │ │
│  │  │  │  Human intervention point   │  │  │ │
│  │  │  └─────────────────────────────┘  │  │ │
│  │  └───────────────────────────────────┘  │ │
│  └─────────────────────────────────────────┘ │
└─────────────────────────────────────────────┘
```

- **Engram Syscalls** prevent unauthorized actions (outermost layer)
- **Cognitive Namespaces** prevent unauthorized knowledge access
- **Cognitive Scheduling** prevents resource starvation and cost runaway
- **Cognitive Signals** provide human override capabilities (innermost safety net)

This maps to the defense-in-depth architecture described in [00-defense-in-depth.md](00-defense-in-depth.md), adding kernel-level primitives to the existing behavioral and structural safety guards.

---

## Comparison with Linux Kernel Primitives

| Linux Primitive | Purpose | Roko Equivalent | Purpose |
|---|---|---|---|
| Namespaces (PID, net, mount) | Process isolation | Cognitive Namespaces | Knowledge isolation |
| Signals (SIGTERM, SIGKILL) | Process control | Cognitive Signals | Agent behavioral control |
| Scheduler (CFS) | CPU time allocation | Cognitive Scheduling | Reasoning resource allocation |
| System calls (syscall table) | Hardware access control | Engram Syscalls | Action access control |
| Capabilities (CAP_NET_RAW) | Fine-grained permissions | Capability<T> tokens | Fine-grained tool permissions |
| cgroups (resource limits) | Resource containment | Budget limits + rate limiters | Cost containment |
| seccomp (syscall filtering) | Syscall allowlist | ToolPermission | Tool allowlist |
| SELinux/AppArmor (MAC) | Mandatory access control | SafetyLayer (composite Policy) | Mandatory safety checks |

The analogy is structural, not superficial. Roko's Cognitive Kernel Primitives solve the same problems for agents that Linux kernel primitives solve for processes: isolation, control, fair scheduling, and mediated access.

---

## Implementation Status

| Component | Status | Location |
|---|---|---|
| SafetyLayer (composite Policy) | Built | `roko-agent/src/safety/mod.rs` |
| ToolDispatcher (syscall-style dispatch) | Built | `roko-agent/src/dispatcher/mod.rs` |
| ProcessSupervisor (process-level signals) | Built | `bardo-runtime/src/process.rs` |
| Event bus (signal delivery) | Built | `bardo-runtime/` |
| RateLimiter (resource limits) | Built | `roko-agent/src/safety/rate_limit.rs` |
| Cognitive Namespaces | Design only | Target: Tier 3 |
| Cognitive Signals (full enum) | Design only | Target: Tier 2 |
| Cognitive Scheduling | Design only | Target: Tier 3 |
| Engram Syscalls (universal enforcement) | Partial (ToolDispatcher) | See [16-critical-integration-gap.md](16-critical-integration-gap.md) |

---

## Academic References

| Paper | Contribution |
|---|---|
| Saltzer & Schroeder (1975), "The Protection of Information in Computer Systems" | Principle of complete mediation — every access must be checked (Engram Syscalls) |
| Dennis & Van Horn (1966), "Programming Semantics for Multiprogrammed Computations" | Capability-based access control (Cognitive Namespaces ACL) |
| Sha, Rajkumar, Lehoczky (1990), "Priority Inheritance Protocols" | Priority inversion prevention (Cognitive Scheduling) |
| Arpaci-Dusseau & Arpaci-Dusseau (2018), "Operating Systems: Three Easy Pieces" | Modern OS primitives that inspire the cognitive kernel design |
| Sumers et al. (2023, arXiv:2309.02427), "Cognitive Architectures for Language Agents" | CoALA — the cognitive loop that generates syscall requests |

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Kernel primitives as another defense layer
- [01-capability-tokens.md](01-capability-tokens.md) — Capability<T> tokens are the fine-grained permission system within Engram Syscalls
- [04-permits-allowlists.md](04-permits-allowlists.md) — ToolPermission is the current syscall filter implementation
- [06-sandboxing.md](06-sandboxing.md) — PathPolicy provides filesystem-level isolation complementing Cognitive Namespaces
- [16-critical-integration-gap.md](16-critical-integration-gap.md) — The gap between ToolDispatcher (partial syscall) and universal enforcement
