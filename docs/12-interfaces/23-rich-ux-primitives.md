# Rich UX Primitives

> **Abstract:** This chapter propagates `tmp/refinements/30-rich-ux-primitives.md` into the canonical docs tree. Roko's interface quality does not come from a generic chat box. It comes from a shared vocabulary of small, reusable affordances that make long-running, probabilistic, tool-using agents legible: reasoning streams, tool-call banners, gate badges, heuristic footnotes, uncertainty bars, replay scrubbers, alternative renderings, confidence-weighted aggregation, progressive disclosure, and spatial memory. These primitives sit on top of `Pulse`, `Bus`, `StateHub`, and durable episode or annotation Engrams so every surface can show the same work with the amount of detail the user actually wants. See also [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [13-web-portal.md](./13-web-portal.md), and [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

**Topic**: [12-interfaces](./INDEX.md)  
**Prerequisites**: [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md), [06-websocket-streaming.md](./06-websocket-streaming.md), [13-web-portal.md](./13-web-portal.md), [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)  
**Key sources**: `tmp/refinements/30-rich-ux-primitives.md`, `tmp/refinements/26-statehub-rearchitecture.md`, `tmp/refinements/27-realtime-event-surface.md`, `tmp/refinements/29-web-ui-architecture.md`

---

## 1. Why Agent UX Needs Primitives

Traditional software mostly answers a request with a new screen state. Agent systems add three problems that generic app chrome does not solve well:

- **Latency**: the user waits seconds to minutes while the system reasons, calls tools, and runs gates.
- **Uncertainty**: the system is probabilistic, so trust depends on visible confidence and visible evidence.
- **Causality**: the result is a chain of choices, not one atomic action, so users need to inspect why a step happened.

REF30 makes the interface claim explicit: good agent UX is a composition of small primitives, not one monolithic "agent page." The primitives below are the shared vocabulary used by CLI, TUI, Chat, and Web. Pages and panes arrange them differently, but the primitives stay semantically consistent.

## 2. The Ten Canonical Primitives

| Primitive | What it shows | Primary user value | Typical upstream |
|---|---|---|---|
| `Reasoning stream` | live thought/process trail beside the main answer | progress + legibility during latency | `agent.reasoning` Pulses |
| `Tool-call banner` | one banner per tool invocation with output and rerun affordances | visible causality and easy retry | `tool.*` Pulses |
| `Gate badge` | compact pass/warn/fail status row | visible verification state | `gate_pipeline` projection + `gate.*` Pulses |
| `Heuristic footnote` | inline numbered heuristic citations with calibration and provenance | inspect why advice or action was chosen | `heuristic.applied` Pulses + `heuristic_library` |
| `Uncertainty bar` | confidence rendering under a decision or diff | calibrated trust and approval thresholds | `decision.emitted` Pulses |
| `Replay scrubber` | episode timeline with scrub-to-moment behavior | time travel through a run | episode Pulse stream + snapshots |
| `Alternative rendering` | different views over the same projection | match the view to the task | shared projection state |
| `Confidence-weighted aggregation` | weighted consensus plus visible minority view | better multi-agent inspection than majority vote | `consensus.*` Pulses |
| `Progressive disclosure` | nested reveal from summary to trace | avoid overwhelming novices while preserving depth | projection state + provenance |
| `Spatial memory` | stable placement and shortcuts across sessions | muscle memory and lower interaction cost | surface discipline rather than kernel data |

Each primitive is intentionally small. Together they create a surface where users can watch, trust, rewind, and challenge the system instead of treating it as a black box.

## 3. Primitive Data Contracts

The primitive is only real if the upstream contract exists. Otherwise it is decorative UI pretending to be observability.

| Primitive | Required contract | Notes |
|---|---|---|
| `Reasoning stream` | `agent.reasoning` Topic carrying sequenced reasoning Pulses | collapsible by default; survives reconnect via cursor |
| `Tool-call banner` | `tool.started`, `tool.completed`, `tool.failed` Topics | output and elapsed time should be recoverable for drill-down |
| `Gate badge` | `gate_pipeline` projection plus detailed `gate.*` evidence | stale marker required when projection lags |
| `Heuristic footnote` | `heuristic.applied` Topic plus `heuristic_library` projection | calibration, provenance, and challenge links live behind the footnote |
| `Uncertainty bar` | `decision.emitted` Topic with explicit confidence field | low-confidence actions route to approval-required state |
| `Replay scrubber` | `recent_episodes`, per-episode timeline data, and Substrate snapshots | rewind should update transcript, diff, and state panes together |
| `Alternative rendering` | one stable projection state that multiple components can read | view swap must not fork the underlying data model |
| `Confidence-weighted aggregation` | `consensus.*` Topics or equivalent projection | minority evidence remains inspectable one click away |
| `Progressive disclosure` | layer provenance from Composer and action/gate evidence | summary, reasoning, heuristics, trace, and cost can unfold inline |
| `Spatial memory` | shortcut registry + stable layout rules | no direct Substrate dependency, but still a product invariant |

This is why REF26 and REF27 matter to REF30. A primitive needs queryable state, filtered subscriptions, cursors, and replay semantics. `Pulse` and `Bus` provide the live medium; `StateHub` provides the durable read model that keeps the surfaces aligned.

## 4. Surface Placement

The same primitive does not need the same shape on every surface, but it does need the same meaning.

### 4.1 CLI

CLI is summary-first. It should render compact versions of the primitives:

- reasoning stream as optional streamed side output or `watch` pane
- tool-call banners as terse, readable blocks with `show output`, `rerun`, and `explain`
- gate badges as compact bracketed status rows
- uncertainty bars as confidence text plus approval prompts
- replay scrubber as `replay` timeline controls rather than a graphical track

CLI should never hide the primitive behind a browser-only feature. The rendering can be thinner, but the affordance must remain reachable from the same verb set.

### 4.2 TUI

TUI is the densest operational surface and should carry the richest terminal-native versions:

- reasoning stream in a toggleable sidebar
- tool-call banners as bordered list rows with expandable output
- gate badges as a persistent status rail
- heuristic footnotes with numbered inline markers and a footnote pane
- uncertainty bars with unicode block characters
- replay scrubber in a bottom timeline bar

TUI is where keyboard-first interaction is most load-bearing, so these primitives must be operable without a pointer.

### 4.3 Chat

Chat is the conversational view over the same runtime:

- the main answer remains primary
- reasoning stream, heuristics, trace, and cost reveal inline through progressive disclosure
- tool-call banners and gate badges punctuate the transcript rather than opening a separate dashboard
- uncertainty bars attach directly to proposed edits, plans, and approvals

Chat should feel like "one answer with inspectable structure" rather than a stream of unrelated system logs.

### 4.4 Web

Web is where the full composition becomes most expressive:

- sidecar reasoning streams
- clickable tool-call banners with log drawers
- persistent explainability panel
- replay scrubber with deep links
- alternative renderings for plans, episodes, and heuristic libraries
- confidence-weighted aggregation widgets for multi-agent review

The browser does not get a different truth. It gets a richer rendering of the same `StateHub` and realtime contracts. See [13-web-portal.md](./13-web-portal.md) and `tmp/refinements/30-rich-ux-primitives.md`.

## 5. Explainability, Annotation, And Undo

Three cross-cutting affordances make the primitives operational instead of cosmetic.

### 5.1 Explainability Panel

Roko should expose one persistent explainability surface, reachable by shortcut and slash command, that answers:

- what the agent is doing now
- which heuristics are currently active
- which gates are pending or blocking
- which tools are available
- what budget remains
- what the system still does not know

This is the always-available version of `/explain`. It complements progressive disclosure by giving users one stable place to ask "why this?" anywhere in the UI.

### 5.2 Annotation As A Durable Feedback Primitive

Annotations should be first-class Engrams attached to episodes, heuristics, plans, diffs, and replay moments. The important UX rule is unification: correction, confirmation, follow-up, and questions all use one durable annotation object instead of ad hoc thumbs-up/down widgets spread across surfaces.

That matters because annotations are not just comments:

- a correction on a heuristic can feed recalibration
- a follow-up on an episode can become future work
- a confirmation can strengthen confidence in a distilled result

### 5.3 Visible Undo

Undo must be visible to feel safe. Any surface that applies a diff or mutation should show a visible post-apply affordance such as `Applied diff. [undo]`, then keep the durable history reachable through session history and replay. Cheap reversal is part of trust, not just convenience.

## 6. Keyboard Registry And Spatial Memory

REF30 treats spatial memory as a primitive because the rest of the system depends on it. Stable placement and stable shortcuts let users build muscle memory across sessions and surfaces.

Minimum registry:

| Shortcut | Action | Surface |
|---|---|---|
| `cmd+k` / `ctrl+k` | command palette | TUI, Web |
| `?` | help overlay | TUI, Web |
| `/` | slash-command focus | TUI, Web Chat |
| `cmd+/` / `ctrl+/` | toggle explainability panel | Web |
| `cmd+z` / `ctrl+z` | undo last applied change | TUI, Web |
| `g h` / `g c` / `g p` / `g b` / `g s` | navigate to Home, Chat, Plans, Beliefs, Settings | Web |
| `j` / `k` | move down / up | TUI |
| `q` | close or quit | TUI |

The exact shared registry belongs in one source of truth so plugins and surface teams do not collide. Stable shortcut meaning is part of the interface contract, just like stable verbs.

## 7. Notification And Failure Discipline

Rich UX does not mean noisy UX. Notification rules should stay narrow:

- notify for gate failures that need attention
- notify for budget thresholds
- notify for approval checkpoints
- notify when long-running work completes and the user is elsewhere

Do not notify for successful intermediate tool calls, generic "still working" status, or every token of progress. The live surface already covers those cases.

Primitives should also degrade coherently when upstream data is missing:

| Primitive | Upstream loss | Degraded behavior |
|---|---|---|
| `Tool-call banner` | missing `tool.*` Pulses | show placeholder status row rather than breaking layout |
| `Gate badge` | `gate_pipeline` stalls | keep last known state and mark it stale |
| `Heuristic footnote` | heuristic lookup fails | omit footnotes without blocking the main answer |
| `Uncertainty bar` | no confidence published | omit the bar and require normal approval policy |
| `Replay scrubber` | episode timeline unavailable | disable scrubbing and show replay unavailable |
| `Alternative rendering` | projection disconnects | freeze current view and show disconnected state |

The rule is simple: degraded but legible beats blank or broken.

## 8. Accessibility, Color, And Motion

These primitives carry system state, so they must remain legible under accessibility constraints:

- never encode pass/fail/warn state by color alone; pair icon plus text
- high-contrast mode must preserve badge, banner, and uncertainty semantics
- reduced-motion mode disables typewriter-style reasoning streams and long fades
- no animation should be necessary to understand tool, gate, or approval state
- mobile renderings should stay read-biased, with the replay scrubber, banners, and approvals still reachable but compressed

This keeps REF30 aligned with [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md) rather than turning "rich UX" into decorative motion.

## 9. Shipping Order

REF30's implementation order should follow upstream readiness:

1. reasoning streams plus tool-call banners
2. gate badges
3. heuristic footnotes
4. uncertainty bars
5. replay scrubber
6. explainability panel, annotations, and visible undo
7. alternative renderings and confidence-weighted aggregation
8. voice, collaborative presence, and mobile polish

This ordering works because primitives degrade independently. Gate badges still help without footnotes. Replay still helps without aggregation. The point is to ship the primitives as a reusable vocabulary instead of waiting for one giant interface rewrite.

## 10. Related Refinements

- [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md) — canonical source for this chapter.
- [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md) — four-surface verb-set contract that these primitives render.
- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) — typed projection layer most primitives consume.
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — live transport carrying the required Pulses and cursors.
- [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md) — browser page model that composes these primitives into `Home`, `Chat`, `Plans`, `Beliefs`, and `Settings`.
- [../05-learning/19-heuristics-worldviews-and-falsifiers.md](../05-learning/19-heuristics-worldviews-and-falsifiers.md) — heuristic provenance, calibration, and challenge model behind footnotes.
- [../13-coordination/11-collective-intelligence-metrics.md](../13-coordination/11-collective-intelligence-metrics.md) — c-factor and minority-view visibility behind weighted aggregation.
