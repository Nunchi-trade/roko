# Cognitive Kernel Primitives: Safety Implications

> **Layer**: L0 Runtime (kernel primitives), L1 Framework (capability enforcement), L3 Harness (signal-based intervention)
>
> **Crate**: Cross-cutting: `roko-runtime` (scheduling, signals), `roko-core` (namespaces, syscalls), `roko-agent` (enforcement)
>
> **Synapse traits**: `Policy` (syscall enforcement), `Substrate` (namespace isolation), `Gate` (signal-driven verification)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [04-permits-allowlists.md](04-permits-allowlists.md), [06-sandboxing.md](06-sandboxing.md)


> **Implementation**: Specified

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

## CognitiveNamespace: full struct

```rust
use std::sync::Arc;
use std::collections::HashSet;

/// Unique identifier for a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NamespaceId(pub String);

/// Agent role for access control.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentRole(pub String);

/// Engram kind for channel filtering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Kind(pub String);

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
    /// Knowledge flows between namespaces only through these.
    pub channels: Vec<NamespaceChannel>,
    /// Maximum Engrams stored in this namespace.
    /// When exceeded, oldest Engrams are evicted per decay policy.
    pub capacity: usize,
    /// Namespace creation timestamp.
    pub created_at: std::time::Instant,
}

impl CognitiveNamespace {
    /// Check whether an agent with the given role can read from this namespace.
    pub fn can_read(&self, role: &AgentRole) -> bool {
        self.acl.allow_anonymous_read || self.acl.readers.contains(role)
    }

    /// Check whether an agent with the given role can write to this namespace.
    pub fn can_write(&self, role: &AgentRole) -> bool {
        self.acl.writers.contains(role)
    }

    /// Find a channel from this namespace to a target namespace.
    /// Returns None if no channel exists (transfer is blocked).
    pub fn channel_to(&self, target: &NamespaceId) -> Option<&NamespaceChannel> {
        self.channels.iter().find(|c| c.to == *target)
    }
}
```

### NamespaceChannel with ACL enforcement

```rust
/// A channel between two namespaces with explicit transfer rules.
pub struct NamespaceChannel {
    /// Source namespace.
    pub from: NamespaceId,
    /// Destination namespace.
    pub to: NamespaceId,
    /// Filter: which Engram kinds can flow through.
    pub allowed_kinds: HashSet<Kind>,
    /// Whether transfers are logged to the audit chain.
    pub audit_transfers: bool,
    /// Maximum transfer rate (Engrams per second).
    /// None = unlimited (not recommended for production).
    pub rate_limit: Option<f64>,
    /// Sliding window counter for rate limiting.
    rate_window: parking_lot::Mutex<RateWindow>,
}

struct RateWindow {
    count: u64,
    window_start: std::time::Instant,
    window_duration: std::time::Duration,
}

impl NamespaceChannel {
    /// Check whether a specific Engram kind can flow through this channel.
    pub fn permits_kind(&self, kind: &Kind) -> bool {
        self.allowed_kinds.is_empty() || self.allowed_kinds.contains(kind)
    }

    /// Check and consume one unit of rate limit.
    /// Returns false if rate limit exceeded.
    pub fn check_rate_limit(&self) -> bool {
        let Some(limit) = self.rate_limit else {
            return true;
        };
        let mut window = self.rate_window.lock();
        let now = std::time::Instant::now();
        if now.duration_since(window.window_start) > window.window_duration {
            // Reset window.
            window.count = 0;
            window.window_start = now;
        }
        let max_count = (limit * window.window_duration.as_secs_f64()) as u64;
        if window.count >= max_count {
            return false;
        }
        window.count += 1;
        true
    }

    /// Transfer an Engram through the channel.
    /// Checks kind filter, rate limit, and optionally logs to audit chain.
    pub fn transfer(
        &self,
        engram: &Signal,
        audit_chain: Option<&dyn Substrate>,
    ) -> Result<(), ChannelError> {
        let kind = Kind(engram.kind().to_string());
        if !self.permits_kind(&kind) {
            return Err(ChannelError::KindBlocked(kind));
        }
        if !self.check_rate_limit() {
            return Err(ChannelError::RateLimited);
        }
        if self.audit_transfers {
            if let Some(chain) = audit_chain {
                chain.write(&create_transfer_audit(
                    &self.from, &self.to, engram,
                ));
            }
        }
        Ok(())
    }
}

pub enum ChannelError {
    KindBlocked(Kind),
    RateLimited,
    TargetNamespaceNotFound,
}
```

---

## CognitiveSignal: full enum and delivery

```rust
/// Typed interrupts for cognitive agents.
/// All variants are non-destructive: no signal causes
/// abrupt termination with state loss.
#[derive(Debug, Clone)]
pub enum CognitiveSignal {
    /// Suspend reasoning, serialize state to disk.
    Pause,
    /// Resume from serialized state.
    Resume,
    /// Change current task priority.
    Reprioritize(TaskId),
    /// Add context mid-reasoning without interrupting.
    InjectContext(Box<Signal>), // Engram injected into active context.
    /// Switch to stronger model immediately.
    Escalate,
    /// Reduce arousal, slow down.
    Cooldown,
    /// Switch to exploratory mode.
    Explore,
    /// Graceful termination.
    Shutdown,
}

/// Priority ordering for signal preemption.
/// Lower number = higher priority.
impl CognitiveSignal {
    pub fn priority(&self) -> u8 {
        match self {
            CognitiveSignal::Shutdown => 1,
            CognitiveSignal::Pause => 2,
            CognitiveSignal::Escalate => 3,
            CognitiveSignal::Cooldown => 4,
            CognitiveSignal::Reprioritize(_) => 5,
            CognitiveSignal::InjectContext(_) => 6,
            CognitiveSignal::Explore => 7,
            CognitiveSignal::Resume => 8,
        }
    }
}
```

### Signal delivery queue and processing order

```rust
use std::collections::BinaryHeap;
use std::cmp::Ordering;

/// A queued signal with priority ordering.
struct QueuedSignal {
    signal: CognitiveSignal,
    queued_at: std::time::Instant,
    /// Timeout: if not acknowledged within this duration,
    /// escalate (e.g., Pause -> Shutdown).
    timeout: std::time::Duration,
}

impl Ord for QueuedSignal {
    fn cmp(&self, other: &Self) -> Ordering {
        // Lower priority number = higher priority in the heap.
        other
            .signal
            .priority()
            .cmp(&self.signal.priority())
            .then_with(|| self.queued_at.cmp(&other.queued_at))
    }
}

/// Signal delivery queue. Signals are processed in priority order.
/// Higher-priority signals preempt lower-priority ones.
pub struct SignalQueue {
    queue: parking_lot::Mutex<BinaryHeap<QueuedSignal>>,
    /// Default timeout per signal type.
    default_timeouts: HashMap<u8, Duration>,
}

impl SignalQueue {
    /// Enqueue a signal for delivery.
    pub fn send(&self, signal: CognitiveSignal) {
        let priority = signal.priority();
        let timeout = self
            .default_timeouts
            .get(&priority)
            .copied()
            .unwrap_or(Duration::from_secs(30));

        self.queue.lock().push(QueuedSignal {
            signal,
            queued_at: std::time::Instant::now(),
            timeout,
        });
    }

    /// Dequeue the highest-priority signal.
    /// Returns None if the queue is empty.
    pub fn recv(&self) -> Option<CognitiveSignal> {
        self.queue.lock().pop().map(|qs| qs.signal)
    }

    /// Check for timed-out signals and escalate them.
    /// Pause -> Shutdown, Cooldown -> Pause, etc.
    pub fn check_timeouts(&self) -> Vec<CognitiveSignal> {
        let mut escalations = Vec::new();
        let mut queue = self.queue.lock();
        let now = std::time::Instant::now();

        // Collect timed-out signals.
        let mut remaining = BinaryHeap::new();
        while let Some(qs) = queue.pop() {
            if now.duration_since(qs.queued_at) > qs.timeout {
                // Escalate.
                let escalated = match qs.signal {
                    CognitiveSignal::Cooldown => CognitiveSignal::Pause,
                    CognitiveSignal::Pause => CognitiveSignal::Shutdown,
                    CognitiveSignal::Reprioritize(_) => CognitiveSignal::Pause,
                    other => other, // Shutdown cannot escalate further.
                };
                escalations.push(escalated);
            } else {
                remaining.push(qs);
            }
        }
        *queue = remaining;
        escalations
    }
}
```

**Configuration:**

```toml
[runtime.signals]
pause_timeout_secs = 30      # Time before Pause escalates to Shutdown. Range: 5..300.
cooldown_timeout_secs = 60   # Time before Cooldown escalates to Pause. Range: 10..600.
reprioritize_timeout_secs = 15 # Time before Reprioritize escalates. Range: 5..120.
default_timeout_secs = 30    # Default for unspecified signal types. Range: 5..300.
```

### Priority inversion prevention (Sha et al. 1990)

When a high-priority signal depends on the completion of a low-priority task, priority inheritance prevents deadlock:

```
priority_inheritance(high_signal, blocking_task):
    # Step 1: Detect the dependency.
    # High-priority signal (e.g., Shutdown) cannot proceed
    # because the agent is executing a low-priority task
    # that holds a resource (e.g., a file lock).

    # Step 2: Temporarily elevate the blocking task's priority.
    original_priority = blocking_task.priority
    blocking_task.priority = high_signal.priority()

    # Step 3: Execute the blocking task at elevated priority.
    # The scheduler now treats it as high-priority, preventing
    # other medium-priority tasks from preempting it.
    execute(blocking_task)

    # Step 4: Restore original priority after completion.
    blocking_task.priority = original_priority

    # Step 5: Process the high-priority signal.
    process(high_signal)
```

This prevents the classic priority inversion scenario where a Shutdown signal is blocked by a routine task that is itself preempted by medium-priority work.

---

## Policy trait: universal enforcement path

The `Policy` trait is the single enforcement point for all agent actions. Every action passes through `decide()` before execution.

```rust
/// The four decision modes for the Policy enforcement point.
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    /// Allow the action to proceed. Log the approval.
    Permit,
    /// Block the action. Return an error Engram to the agent.
    Deny { reason: String },
    /// Allow but alter the action (e.g., reduce scope, scrub secrets).
    Modify { modified_engram: Signal },
    /// Allow but create a detailed audit record.
    Log { detail_level: AuditDetailLevel },
}

#[derive(Debug, Clone)]
pub enum AuditDetailLevel {
    /// Log action type and result only.
    Summary,
    /// Log full action parameters and result.
    Detailed,
    /// Log everything including context window contents.
    Forensic,
}

/// Composite Policy: chains multiple policies.
/// All policies must agree to Permit; any Deny blocks.
pub struct CompositPolicy {
    policies: Vec<Box<dyn Policy>>,
}

impl Policy for CompositPolicy {
    fn decide(&self, engrams: &[Signal]) -> Vec<Signal> {
        let mut all_decisions = Vec::new();
        for policy in &self.policies {
            let decisions = policy.decide(engrams);
            // If any policy emits a Deny Engram, the action is blocked.
            if decisions.iter().any(|d| is_deny(d)) {
                return decisions; // Short-circuit on first denial.
            }
            all_decisions.extend(decisions);
        }
        all_decisions
    }
}
```

**Current composite policy chain** (wired in `roko-agent/src/safety/mod.rs`):

```
SafetyLayer::check_pre_execution()
  |
  +--> BashPolicy::decide()      -- deny dangerous shell commands
  +--> GitPolicy::decide()        -- deny force-push, protected branches
  +--> NetworkPolicy::decide()    -- deny private networks, enforce HTTPS
  +--> PathPolicy::decide()       -- deny paths outside worktree
  +--> RateLimiter::check()       -- deny if rate limit exceeded
  |
  [post-execution]
  +--> ScrubPolicy::scrub_output() -- modify: redact secrets in output
```

The universal enforcement path extends this to cover all action types:

```
[Current: ToolDispatcher only]
  Tool call -> SafetyLayer -> execute -> ScrubPolicy

[Target: Universal Engram Syscall]
  Any action -> CompositPolicy::decide() -> execute -> post-check
    |
    +--> Tool call: SafetyLayer chain
    +--> Knowledge post: NamespaceChannel.transfer()
    +--> Mesh relay: TaintedString.can_flow_to(MeshRelay)
    +--> File write: PathPolicy + TaintedString check
    +--> API call: NetworkPolicy + RateLimiter
```

### Test criteria

- `CognitiveNamespace::can_read()` returns false for unlisted roles when `allow_anonymous_read` is false
- `CognitiveNamespace::can_write()` returns false for reader-only roles
- `NamespaceChannel::permits_kind()` blocks non-whitelisted Engram kinds
- `NamespaceChannel::check_rate_limit()` returns false after exceeding configured rate
- `NamespaceChannel::transfer()` logs to audit chain when `audit_transfers` is true
- `CognitiveSignal::priority()` returns 1 for Shutdown (highest) and 8 for Resume (lowest)
- `SignalQueue` delivers Shutdown before Cooldown regardless of enqueue order
- `SignalQueue::check_timeouts()` escalates a timed-out Pause to Shutdown
- `CompositPolicy` short-circuits on the first Deny from any sub-policy
- Priority inheritance completes: a high-priority signal blocked by a low-priority task eventually proceeds after task elevation

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
