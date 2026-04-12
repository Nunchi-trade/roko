# Cognitive Signals

> Typed interrupts that carry semantic meaning. Not just "something
> happened" but "pause execution," "reprioritize this task," "inject
> this context," "escalate to a more capable model."

---

## Definition

Cognitive Signals are a proposed extension to Roko's signal system
that adds typed interrupt semantics. Where standard signals carry
data (token counts, gate verdicts, cost metrics), cognitive signals
carry intent — they tell the pipeline what to DO, not just what
IS.

```rust
pub enum CognitiveSignal {
    Pause,
    Resume,
    Reprioritize(TaskId),
    InjectContext(Engram),
    Escalate,
    Cooldown,
    Explore,
    Shutdown,
}
```

---

## Signal Semantics

### Pause

**Intent**: Temporarily halt execution of the current task or plan.

**When emitted**: The Conductor detects a condition that requires
external resolution before execution can continue productively.
Examples:
- Spec drift detected — need to verify the specification is still
  current before the agent makes more changes
- Cost approaching budget — need operator approval before spending
  more
- Infrastructure degraded — wait for recovery before dispatching
  more work

**Orchestrator response**: Suspend the affected agent process (or
stop sending it work). Preserve its state. Do not kill — the work
may be resumable.

**Difference from Restart**: Restart kills the agent and starts fresh.
Pause preserves the agent's state and conversation history. Pause is
appropriate when the agent's work is valid but the environment needs
to change before continuing.

### Resume

**Intent**: Continue execution after a Pause.

**When emitted**: The condition that caused the Pause has been
resolved. The spec has been verified, the budget has been approved,
the infrastructure has recovered.

**Orchestrator response**: Resume the suspended agent process or
restart dispatching work to it.

### Reprioritize(TaskId)

**Intent**: Change the priority of a specific task in the scheduling
queue.

**When emitted**: The Conductor or learning system determines that
a task's priority should change based on new information. Examples:
- A dependency of this task just completed — the task is now
  unblocked and should move up in priority
- The task's file set conflicts with a higher-priority in-flight
  task — deprioritize to avoid merge conflicts
- The task has been waiting too long — elevate priority to prevent
  starvation

**Orchestrator response**: Adjust the task's position in the
scheduling queue. This does not affect in-flight tasks — only queued
tasks waiting for dispatch.

### InjectContext(Engram)

**Intent**: Add specific context to the current agent's prompt.

**When emitted**: The Conductor has information that the agent needs
but does not have. Examples:
- The diagnosis engine classified an error and has a specific fix
  suggestion: "E0432 on line 42 — add `use crate::auth::AuthToken;`"
- A playbook rule was matched: "Past builds show auth types have
  lifetime parameters. Check actual signatures."
- Another agent's work produced relevant context: "Plan 3 just
  modified `mod.rs` — your imports may need updating."

**Engram**: In Roko's naming convention, an Engram is a unit of
persistent context. An `InjectContext` signal carries an Engram —
a typed piece of information that the orchestrator injects into the
agent's next prompt.

**Orchestrator response**: Append the Engram content to the agent's
context for its next turn. This may be injected via the system prompt
(`--append-system-prompt`), via `context/in/`, or via MCP tool
response.

### Escalate

**Intent**: Move the task to a more capable processing tier.

**When emitted**: The current model or agent configuration is
insufficient for the task. Examples:
- A Haiku-tier agent has failed twice on a complex task
- The diagnosis engine identified an error category (BorrowCheckerError,
  LifetimeError) that requires deeper reasoning
- The quality judge scored the output below threshold

**Orchestrator response**: Kill the current agent. Respawn with:
- A more capable model (Haiku → Sonnet → Opus)
- More context (add type signatures, dependency graph)
- Different tools (add `get_symbol_context`, `get_change_impact`)

**Connection to cascade router**: Escalation feeds a negative reward
to the cascade router for the current model-task combination. Over
time, the router learns to route complex tasks to capable models
directly, reducing the need for escalation.

### Cooldown

**Intent**: Reduce pressure on the current task or plan.

**When emitted**: The Conductor detects that the agent is under
too much pressure — approaching the Yerkes-Dodson collapse zone.
Indicators:
- Rapid context growth (agent is accumulating errors and retries)
- Decreasing output quality per turn
- Increasing token cost per turn with decreasing progress

**Orchestrator response**: Extend timeouts, reduce iteration
pressure, or add a deliberate pause before the next attempt. The
goal is to move the agent back toward the productive zone of the
Yerkes-Dodson curve.

**Yerkes-Dodson context**: Research on 770,000+ autonomous agents
shows cooperative behavior follows an inverted-U curve with
environmental pressure. Moderate pressure maximizes cooperation.
Extreme pressure collapses cooperative behavior within 5-12 turns.
The Cooldown signal is the mechanism for detecting and responding
to over-pressure.

Reference: Yerkes & Dodson (1908). "The relation of strength of
stimulus to rapidity of habit-formation."

### Explore

**Intent**: Grant the agent more freedom to explore alternative
approaches.

**When emitted**: The current approach has failed but the task
itself is believed to be solvable. The agent needs creative freedom
rather than tighter constraints. Examples:
- Two different approaches have both failed at the gate
- The diagnosis engine suggests the error requires an architectural
  change, not a local fix
- Historical episodes show this task type benefits from exploration

**Orchestrator response**: Expand the agent's tool access, increase
the iteration limit, or provide broader context. The agent gets
more rope — at the cost of more tokens and time.

**Tension with Cooldown**: Explore and Cooldown pull in different
directions. Explore grants more freedom (potentially more pressure).
Cooldown restricts freedom (less pressure). The Conductor must
choose between them based on the specific failure mode. Repeated
identical errors → Cooldown (more of the same approach will not
help). Diverse but unsuccessful attempts → Explore (the agent is
trying different things and needs room to find the right one).

### Shutdown

**Intent**: Gracefully terminate execution.

**When emitted**: The Conductor determines that the entire execution
should stop. Examples:
- Budget for the batch run is exhausted
- Critical infrastructure failure (all agents down)
- Operator-initiated shutdown (Ctrl+C)

**Orchestrator response**: Execute the graceful shutdown sequence:
1. Stop accepting new tasks
2. Drain in-flight tasks (30-second grace period)
3. Kill remaining agents if drain times out
4. Save checkpoint to `.roko/state/executor.json`
5. Flush logs
6. Exit

---

## Signal vs. Signal

Roko's core `Signal` type already carries typed data through the
pipeline. Cognitive Signals extend this with intent semantics:

| Aspect | Standard Signal | Cognitive Signal |
|--------|----------------|-----------------|
| **Purpose** | Data transport | Intent transport |
| **Content** | Measurement (tokens, cost, time) | Command (pause, escalate, inject) |
| **Producer** | Any component | Conductor, learning system |
| **Consumer** | Any component | Orchestrator |
| **Action** | Read and react | Execute the intent |

Cognitive Signals can be encoded as standard Signals using the
`Kind::Custom` variant:

```rust
// Encoding a cognitive signal as a standard signal
fn cognitive_to_signal(cs: &CognitiveSignal) -> Signal {
    match cs {
        CognitiveSignal::Pause => {
            Signal::builder(Kind::Custom("conductor.cognitive.pause".into()))
                .body(Body::text("pause execution"))
                .tag("cognitive_signal", "pause")
                .build()
        }
        CognitiveSignal::Escalate => {
            Signal::builder(Kind::Custom("conductor.cognitive.escalate".into()))
                .body(Body::text("escalate to higher tier"))
                .tag("cognitive_signal", "escalate")
                .build()
        }
        // ...
    }
}
```

This encoding preserves backward compatibility — the cognitive signal
is just a Signal with specific Kind and tags. Components that do not
understand cognitive signals can safely ignore them.

---

## Implementation Status

Cognitive Signals are defined in the refactoring PRD (§XII.2,
09-innovations.md) but not yet implemented as a formal type in the
codebase. The Conductor currently expresses its decisions through
`ConductorDecision` (Continue/Restart/Fail), which covers a subset
of cognitive signal semantics:

| ConductorDecision | Equivalent Cognitive Signal |
|-------------------|---------------------------|
| Continue | (no signal — healthy) |
| Restart | Escalate or InjectContext + Resume |
| Fail | Shutdown (for the specific plan) |

The missing cognitive signals (Pause, Resume, Reprioritize,
InjectContext, Cooldown, Explore) represent planned extensions that
would give the Conductor more nuanced control over execution.

**Path to implementation**:
1. Define `CognitiveSignal` enum in `roko-core`
2. Extend `ConductorDecision` to include cognitive signal variants
3. Teach the orchestrator to handle each signal type
4. Wire watchers to emit cognitive signals when appropriate
5. Add learning system integration (track which cognitive signals
   improve outcomes)

---

## Cognitive Signals in the Cybernetic Loop

Cognitive Signals enrich the Conductor's OODA loop:

**Without cognitive signals**: The Conductor can only Continue,
Restart, or Fail. Every anomaly gets one of three responses.

**With cognitive signals**: The Conductor can Pause (wait for
conditions to change), Cooldown (reduce pressure), Explore (grant
freedom), InjectContext (provide targeted help), Escalate (increase
capability), or Reprioritize (reorder the queue). The response
vocabulary grows from 3 to 8+, matching the variety of the
anomalies the Conductor can detect.

This directly addresses Ashby's Law: the regulator's variety (number
of distinct responses) must match the system's variety (number of
distinct failure modes). With only 3 responses, many distinct failure
modes receive the same generic treatment. With 8+ responses, each
failure mode can receive a tailored intervention.

---

## File Reference

| File | What |
|------|------|
| `refactoring-prd/09-innovations.md` §XII.2 | Cognitive Signal definition |
| `crates/roko-core/src/agent.rs` | ConductorDecision (current 3-state decision) |
| `crates/roko-conductor/src/conductor.rs` | evaluate() (where decisions are made) |
| `crates/roko-conductor/src/interventions.rs` | Intervention policy (decision resolution) |
