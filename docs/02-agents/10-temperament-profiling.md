# 10 — Temperament Profiling

> Sub-doc 10 of **02-agents** · Roko Documentation
>
> This document describes the temperament system — a single configuration
> dial that controls agent behavior across verbosity, tool selection, gate
> strictness, review depth, and model routing.


> **Implementation**: Shipping

---

## The Temperament Concept

Roko's temperament system provides a **single configuration dial** that adjusts
multiple agent behaviors simultaneously. Rather than tuning 15 individual
parameters (temperature, max_tokens, tool_selection_bias, gate_threshold,
review_passes, etc.), the operator selects one of four temperaments, and all
downstream behaviors adjust accordingly.

The temperaments are:

| Temperament | Use case | Key behaviors |
|---|---|---|
| **Conservative** | Production, safety-critical | Low temperature, strict gates, full review, minimal tool use |
| **Balanced** | Default development | Medium temperature, standard gates, standard review |
| **Aggressive** | Rapid prototyping | Higher temperature, relaxed gates, faster review, more tools |
| **Exploratory** | Research, experimentation | High temperature, permissive gates, broad tool access |

This design is documented in refactoring PRD §02-five-layers, which presents
the temperament table as part of the Layer 2 (Scaffold) specification.

---

## What Temperament Controls

### 1. Model Parameters

| Parameter | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| `temperature` | 0.1 | 0.3 | 0.7 | 1.0 |
| `top_p` | 0.9 | 0.95 | 0.98 | 1.0 |
| `max_tokens` | profile default | profile default | profile × 1.5 | profile × 2.0 |

Conservative temperament keeps the model focused on the most likely tokens,
reducing creativity but increasing reliability. Exploratory temperament
allows the full token distribution, encouraging novel approaches.

### 2. Tool Selection

| Behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Tool count | Minimal | Standard | Expanded | All available |
| Dangerous tools | Blocked | Blocked | Allowed with confirm | Allowed |
| Network access | Denied | Per-request | Allowed | Allowed |
| File writes | Confirmed | Allowed | Allowed | Allowed |

Conservative temperament restricts the agent to read-only tools by default,
requiring explicit approval for any write or exec operation. Exploratory
temperament gives the agent access to all registered tools including
network fetch and bash execution.

### 3. Gate Strictness

| Gate behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Compile gate | Required | Required | Required | Warning |
| Test gate | Required | Required | Warning | Skipped |
| Clippy gate | Required | Warning | Skipped | Skipped |
| Diff size gate | Strict (< 500 lines) | Standard (< 2000) | Relaxed (< 5000) | Disabled |
| Review gate | Required | Optional | Skipped | Skipped |

Conservative temperament requires all gates to pass before accepting agent
output. Aggressive temperament relaxes test and lint gates to speed iteration.
Exploratory temperament disables most gates entirely, useful for rapid
prototyping where correctness will be verified manually later.

### 4. Review Depth

| Review behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Review passes | 2 (double review) | 1 | 0 (self-review) | 0 |
| Review model | Premium tier | Standard tier | Same as implementer | None |
| Feedback loop | Required | Optional | Disabled | Disabled |

Conservative temperament runs two review passes using a Premium-tier model
to catch subtle issues. Aggressive temperament skips external review and
relies on the implementer's self-assessment.

### 5. Model Routing

| Routing behavior | Conservative | Balanced | Aggressive | Exploratory |
|---|---|---|---|---|
| Starting tier | Standard | Standard | Fast | Fast |
| Escalation threshold | High (0.9 confidence) | Medium (0.7) | Low (0.5) | Low (0.3) |
| Budget multiplier | 0.8× | 1.0× | 1.5× | 2.0× |
| Fallback on error | Always | Usually | Sometimes | Rarely |

Conservative temperament starts at the Standard tier and escalates to
Premium only when confidence is very high. Exploratory temperament starts
at Fast tier with low escalation thresholds, accepting more model variance
in exchange for lower cost and faster iteration.

---

## Configuration

Temperament is set in `roko.toml`:

```toml
[agent]
temperament = "balanced"  # conservative | balanced | aggressive | exploratory
```

Per-role overrides are supported:

```toml
[agent.roles.implementer]
temperament = "balanced"

[agent.roles.researcher]
temperament = "exploratory"

[agent.roles.auditor]
temperament = "conservative"
```

---

## Temperament in the CascadeRouter

The CascadeRouter (sub-doc 12) uses temperament to set its initial parameters:

- **Confidence threshold** — How confident the fast model must be before the
  task is accepted without escalation. Conservative: 0.9, Balanced: 0.7.
- **UCB exploration parameter** — Controls how much the LinUCB bandit
  explores vs. exploits. Exploratory: high exploration to try new models.
- **Cost weight in Pareto frontier** — How much cost factors into model
  selection. Aggressive: lower cost weight (willing to spend more for speed).

This means temperament affects not just the current task, but the learning
trajectory: an Exploratory temperament causes the router to try more model
combinations, building a richer reward signal for future decisions.

---

## Temperament and the Six Harness Principles

The temperament system implements Meta-Harness principle #5 (Graduate Autonomy
Based on Confidence) at the configuration level:

- **Conservative** = low autonomy, high validation
- **Exploratory** = high autonomy, low validation

The operator selects the trust level appropriate for the context — production
deployments use Conservative, development sprints use Balanced or Aggressive,
and research spikes use Exploratory.

This is a deliberate design choice: rather than having the system automatically
escalate autonomy (which could be unsafe), the operator explicitly sets the
autonomy level. Automatic escalation within a temperament level is handled by
the CascadeRouter's model tier selection, but the overall autonomy envelope
is set by the human.

---

## Temperament in Active Inference

The temperament system has a theoretical connection to the Free Energy
Principle (Friston, 2010). In active inference terms:

- **Conservative** = high precision on expected outcomes. The agent strongly
  expects correct code and requires strong evidence (gate passes) before
  accepting. This corresponds to a low free-energy tolerance.
- **Exploratory** = low precision on expected outcomes. The agent accepts
  more variance, allowing exploration of the state space. This corresponds
  to a high free-energy tolerance (more surprise is acceptable).

The precision parameter in active inference maps directly to the confidence
threshold in the CascadeRouter: higher precision means the agent demands
more confidence before committing to a model tier. This is not coincidental —
the temperament system was designed with this theoretical grounding in mind.

Reference: Friston, K. (2010). "The free-energy principle: a unified brain
theory?" Nature Reviews Neuroscience.

---

## Temperament Interaction with Budget

Temperament interacts with the per-role budget system (sub-doc 04):

| Temperament | Budget effect |
|---|---|
| Conservative | 0.8× multiplier (lower ceiling) |
| Balanced | 1.0× (no adjustment) |
| Aggressive | 1.5× (higher ceiling) |
| Exploratory | 2.0× (highest ceiling) |

Conservative temperament tightens the budget because it routes to Standard
tier and avoids Premium escalation. Exploratory temperament loosens the
budget because it may escalate frequently and try multiple models.

This creates a natural cost-safety tradeoff: Conservative is cheapest and
safest, Exploratory is most expensive but discovers optimal model routing
faster.

---

## Implementation Status

The temperament system is specified in the refactoring PRD but is not yet
fully wired into the runtime. Current status:

- **Specified** — Temperament table and per-behavior mapping defined.
- **Config schema** — The `temperament` field exists in `AgentConfig`.
- **Not wired** — The runtime does not yet read the temperament field and
  propagate it to gate thresholds, tool selection, model routing, and
  review depth. Each of these subsystems currently uses its own defaults.

The wiring is tracked as a Tier 2 (cognitive) implementation priority.

### Wiring plan

The implementation path for temperament propagation:

1. **Read temperament from config** — `AgentConfig::temperament` → parsed enum.
2. **Pass to CascadeRouter** — Set initial confidence threshold, exploration
   parameter, cost weight from the temperament table.
3. **Pass to gate pipeline** — Set `required` / `warning` / `skipped` per
   gate based on the temperament table.
4. **Pass to ToolDispatcher** — Adjust tool allowlists: Conservative restricts
   to read-only by default; Exploratory allows all tools.
5. **Pass to SystemPromptBuilder** — Include temperament-appropriate behavioral
   instructions in the role prompt layer.

Each step is independent and can be wired incrementally.

---

## Citations

1. Refactoring PRD §02-five-layers — Temperament Profiling table, Layer 2
   specification.
2. Lee, S. Y. et al. (2026). "Meta-Harness: Harness Engineering for LLM
   Agents." arXiv:2603.28052. — Principle #5: Graduate Autonomy.
3. Refactoring PRD §07-implementation-priorities — Tier 2: Temperament wiring.
4. `crates/roko-core/src/config/schema.rs` — AgentConfig temperament field.
5. `crates/roko-learn/` — CascadeRouter, adaptive gate thresholds.
