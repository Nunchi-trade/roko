# Web Portal

> **Abstract:** This chapter propagates `tmp/refinements/29-web-ui-architecture.md` into the canonical docs tree and layers in `tmp/refinements/30-rich-ux-primitives.md`. The `Web Portal` is not a second server or a browser-only product fork. It is the first-party web UI: a deliberate small reference implementation that sits on top of `StateHub`, the shared realtime surface, and the existing HTTP control plane. The initial scope is five pages only: `Home`, `Chat`, `Plans`, `Beliefs`, and `Settings`. See also [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [06-websocket-streaming.md](./06-websocket-streaming.md), and [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md).

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [06-websocket-streaming.md](./06-websocket-streaming.md), [07-rosedust-design-language.md](./07-rosedust-design-language.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md)
**Key sources**: `tmp/refinements/29-web-ui-architecture.md`, `tmp/refinements/26-statehub-rearchitecture.md`, `tmp/refinements/27-realtime-event-surface.md`, `tmp/refinements/30-rich-ux-primitives.md`

---

## 1. What The Web Portal Is For

The browser surface exists because not every operator lives in a terminal and not every stakeholder should need a local Rust install. The first-party web UI serves five concrete use cases:

- First-time users who want to observe Roko before installing the CLI.
- Non-developer stakeholders who need visibility without direct operation.
- Shared team instances where multiple people need the same live view.
- Demos and presentations where a browser is the right medium.
- Mobile follow-up where a user wants to check whether work finished, failed, or needs approval.

The non-goals are equally important:

- It is not a full IDE.
- It does not replicate every TUI pane.
- It does not replace the CLI for power users.
- It does not own a separate backend state model.

The product move is deliberate small. Ship a clean reference implementation that other teams can fork, theme, or extend, not a kitchen-sink SaaS shell.

## 2. Architectural Boundary

The architectural rule is load-bearing: the Web Portal is a client of the kernel projection and transport layers, not a peer backend.

- `StateHub` owns shared application state through named projections.
- The realtime surface carries projection snapshots, deltas, cursors, and selected Topic streams.
- The HTTP control plane owns explicit mutations such as sending a prompt, pausing a plan, rotating a secret, or enabling a plugin.
- The browser owns only local view state such as scroll position, expanded rows, and input buffers.

That means the browser never reaches directly into `Substrate` or `Bus`. It issues explicit API calls, those calls emit Pulses, and the resulting projection deltas flow back through `StateHub`. The same state then appears in CLI, TUI, Chat, and Web with one source of truth.

## 3. Recommended Stack

The reference implementation should use a stack with a good long-term UX story while staying small enough to ship in the same release train as the backend.

| Layer | Recommendation | Notes |
|---|---|---|
| Framework | `SvelteKit` | Recommended reference stack: small bundles, direct reactivity, SSR by default, good fit for dashboard-heavy UI |
| Acceptable alternative | `React + Vite` | Valid if a larger team wants React ergonomics or ecosystem reach |
| Styling | `Tailwind CSS` | Shared design tokens, fast iteration, straightforward responsive behavior |
| UI primitives | `shadcn-svelte` or equivalent | Use commodity primitives instead of inventing custom widgets first |
| Charts | `visx`, `Observable Plot`, or `uPlot` | Good fit for c-factor, cost, gate-rate, and demurrage charts |
| Code editing | `CodeMirror 6` | Diff review, patch editing, inline code inspection |
| Rich text | `Tiptap` or `Lexical` | Heuristic notes, incident summaries, replication reports |
| Shared widgets | `@roko/ui` component library | Browser and third-party surfaces should share signature widgets and review primitives |

The stack decision should not leak into the transport contract. `SvelteKit` is the preferred reference implementation, but the architectural bet is `StateHub + realtime surface + control plane`, not a framework brand.

## 4. Page Model: Five Pages Only

The first-party web scope should ship exactly five core pages.

| Page | Purpose | Key projections | Typical mutations |
|---|---|---|---|
| `Home / Pulse` | Mobile-friendly "is everything OK?" glance surface | `cohort_health`, `active_tasks`, `recent_episodes`, `cost_meter`, `alerts` | acknowledge alert |
| `Chat` | Primary interactive surface with streaming and diff review | `agent_trails`, `recent_episodes`, `heuristic_library` | send prompt, apply diff, annotate |
| `Plans` | Plan list plus selected DAG and breakpoint control | `plans_list`, `plan_detail/<id>` | create, pause, resume, execute, reorder |
| `Beliefs` | Inspectable heuristic and worldview browser | `heuristic_library`, `worldview_clusters`, `replication_ledger` | challenge, retire, edit, import, export |
| `Settings` | Minimal control plane for config, plugins, and budgets | `config_current`, `plugins_list`, `secrets_status` | set config keys, rotate secrets, install or enable plugins |

Anything beyond those pages is either a later batch or a plugin-contributed extension.

### 4.1 Home / Pulse

`Home / Pulse` is the default landing page and the primary mobile view. It is a live tile layout with five elements:

- system pulse with current cohort `c-factor`
- active tasks with progress and current blockers
- recent episodes with one-line summaries
- cost meter for session or tenant spend
- alerts for gate failures, circuit breakers, or demurrage extremes

This page optimizes for glanceability, not deep operation.

The mobile view should stay read-biased and stable under interruption. High-signal tiles should surface summary states first, then expand into detail only when a user taps through. If a task is still running, the page should show progress and uncertainty plainly rather than hiding it behind a spinner.

The tile set should also surface browser-native rich UX primitives:

- reasoning stream summaries for the current active task
- compact gate badges for the latest pass, warn, and fail states
- uncertainty bars on decisions that may need approval or extra scrutiny
- replay entry points for recent episodes and notable state changes
- notification-worthy alerts only when the user is away, blocked, or explicitly waiting

### 4.2 Chat

`Chat` is the most-used page and the most important one to get right. It should provide:

- streaming token output
- a reasoning stream pane or inline ribbon that can be expanded from the transcript
- tool banners and approval checkpoints
- gate badges near the latest action so verification status is visible without leaving the conversation
- inline diff review with apply, reject, and edit affordances
- heuristic footnotes that explain why a response or tool choice was shaped a certain way
- uncertainty bars on proposed actions so confidence is visible before the user commits
- the same slash-command vocabulary as CLI where it maps cleanly
- file drop for additional context
- agent switching in the session header
- replay links for episode citations and episode-level rewind
- accessible voice input as a convenience, not a dependency
- progressive disclosure for all secondary detail: show the answer first, then expand the chain, trace, or footnotes on demand

This page is the browser continuation of the familiar workflow described in [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), not a separate chat product.

### 4.3 Plans

`Plans` is where Roko's orchestration differentiator becomes visible. The page should render:

- plan list on the left
- selected plan DAG on the right
- status-colored task nodes
- confidence-weighted aggregation when multiple agents or subsystems propose competing interpretations of the next step
- episode drill-down on node selection
- replay scrubber hooks for jumping to a previous execution point and inspecting the causal chain
- breakpoint controls for "pause here, require approval"
- drag-to-reorder for tasks that have not started yet on desktop
- progressive disclosure for plan metadata so the DAG stays readable at a glance
- uncertainty cues on branch points where the plan depends on incomplete evidence

On mobile, this same content degrades to a vertical task list instead of forcing a tiny DAG.

### 4.4 Beliefs

`Beliefs` is the inspectable worldview surface. It should include:

- heuristic table with calibration intervals and provenance
- heuristic footnotes that let users inspect the exact explanation, supporting evidence, and last calibration trail behind a belief
- worldview clusters showing dominant heuristics
- replication ledger showing source claims versus local results
- explicit challenge, retire, and edit actions
- confidence and calibration summaries so users can distinguish strong, weak, and stale heuristics quickly

This page exists because heuristics and falsifiers are first-class, inspectable system state. See [../05-learning/19-heuristics-worldviews-and-falsifiers.md](../05-learning/19-heuristics-worldviews-and-falsifiers.md), [tmp/refinements/14-worldview-validation.md](../../tmp/refinements/14-worldview-validation.md), and [tmp/refinements/16-research-to-runtime.md](../../tmp/refinements/16-research-to-runtime.md).

### 4.5 Settings

`Settings` should remain intentionally minimal:

- model and credential management through the control plane
- domain-profile selection and composition
- gate-threshold tuning with reset-to-adaptive affordance
- plugin install, enable, disable, and inspection
- budget limits and warning thresholds
- keyboard-first behavior for the whole page, including command palette access and reversible toggles
- clear confirmation for destructive or irreversible actions before any mutation is sent
- notification preferences that let the user suppress non-essential alerts and keep approval prompts visible

The browser should never expose raw provider keys to page scripts. Credential operations happen through authenticated control-plane endpoints and server-managed session state.

## 5. State Ownership And Projection Stores

State ownership rules prevent page races and keep cross-surface continuity intact.

- Page-local stores own transient view state only.
- `StateHub` projections own shared application state.
- Mutations go through explicit API calls.
- Projection deltas are authoritative when they arrive.
- Optimistic UI is allowed for high-latency actions, but speculative state must be visibly marked until the real delta lands.
- When projection data is incomplete or stale, the browser should degrade gracefully with the last known state, a clear stale marker, and an action to refresh or open replay.

In the reference `SvelteKit` client, the simplest pattern is one reactive store per projection:

```ts
import { writable } from "svelte/store";
import { roko } from "$lib/roko";

export const cohortHealth = writable<CohortHealthState | null>(null);

roko.subscribe("projection:cohort_health", {}, (frame) => {
  if (frame.type === "state") cohortHealth.set(frame.payload);
  if (frame.type === "delta") {
    cohortHealth.update((current) => applyDelta(current, frame.payload));
  }
});
```

The important rule is architectural rather than library-specific:

- query current projection state
- subscribe to the same projection
- fold deltas in one place
- render from the shared store

This is how Web stays aligned with TUI, CLI `watch`, and external dashboards.

## 6. Routing, Deep Links, And Replay

Every page must be deep-linkable because the browser is an observation and sharing surface as much as an operation surface.

| Route | Meaning |
|---|---|
| `/plans/<slug>` | plan view |
| `/plans/<slug>?task=<id>` | focused task inside a plan |
| `/chat/<session>` | named session view |
| `/chat/<session>?cursor=<c>` | jump to a specific point in the transcript |
| `/beliefs` | heuristic and worldview browser |
| `/beliefs/h/<id>?challenge=true` | open challenge flow for one heuristic |
| `/episodes/<hash>` | direct episode detail |
| `/replay/<episode>?t=0:45` | replay scrubber at a precise offset |

The routing model should preserve spatial memory. The same surfaces should stay in the same places across sessions, and the same deep link should reopen the same kind of detail view without forcing the user to re-learn the layout.

When a link is shared, the semantics should be explicit:

- public share links expire by default
- private links require normal auth
- replay links capture a snapshot cursor so later state changes do not silently rewrite the original context

## 7. Authentication And Security

The Web Portal should support three deployment modes:

| Mode | Use |
|---|---|
| `--allow-any` | local development only |
| Basic auth over HTTPS | small-team installs |
| OIDC | production deployments with Google Workspace, Microsoft, Okta, Authentik, or equivalent |

Security rules:

- session identity lives in an HTTP-only cookie
- all mutations carry CSRF protection
- the browser never stores raw provider API keys
- projection reads and control-plane writes respect the same tenant and role rules as the rest of the system
- plugin-contributed pages and tiles run in a constrained rendering boundary to prevent CSS or capability bleed

See [../11-safety/INDEX.md](../11-safety/INDEX.md), [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md), and [05-http-api-roko-serve.md](./05-http-api-roko-serve.md).

## 8. Theming, Accessibility, And Mobile

The browser surface must ship with:

- light and dark themes
- high-contrast mode
- reduced-motion support
- printable CSS for reports, PRDs, and incident summaries
- CSS-variable overrides rather than a proprietary theme layer

Accessibility rules are not optional:

- semantic HTML first and ARIA second
- full keyboard reachability for every control
- visible focus outlines
- screen-reader validation on core flows
- all user-facing strings stored in resource bundles for later translation
- the explainability panel should be available from any page and should summarize what the agent is doing, what heuristics are active, what gates are pending, and what budget remains
- keyboard-first navigation should work end to end, with shortcuts discoverable from a single help overlay
- notifications should remain conservative: alert for approval, failure, long-running completion, or user-blocking state, but not for every intermediate tool call

Mobile rules are page-specific:

- `Home / Pulse` stacks tiles vertically and acts as the default phone view
- `Chat` remains usable but read-biased, with collapsed tool banners by default
- `Plans` becomes a vertical task list on narrow screens
- `Beliefs` switches to stacked cards and horizontally scrolling worldview clusters
- `Settings` becomes selectively read-only for risky or irreversible operations
- reasoning streams, footnotes, and replay controls collapse into compact affordances first, then expand on demand
- uncertainty and gate status should remain legible at thumb distance, not buried behind hover-only interactions
- if a surface loses live data, show a coherent degraded view rather than empty chrome or a hard error state

Touch targets should stay at or above 44 pt and body text at or above 16 px to avoid mobile browser zoom traps.

## 9. Performance And Delivery Budget

The reference implementation should stay inside explicit performance budgets:

| Budget | Target |
|---|---|
| Time to interactive on `Home / Pulse` | under 2 seconds on a laptop |
| Shell bundle size | under 300 KB before page-level lazy loads |
| Browser memory after one hour of chat | under 150 MB |
| Main-thread blocking per update | under 16 ms |
| Lighthouse performance | `>= 90` on `Home`, `>= 85` on the other core pages |

Server-side rendering should be selective:

- `Home / Pulse` SSRs initial projection state, then hydrates into live subscription mode
- `Chat`, `Plans`, and `Beliefs` SSR the shell plus auth state, then load projections after hydration
- `Settings` SSRs auth only, then fetches mutable form state on the client

The shell should also behave like a PWA: static assets are cacheable, reconnect uses cursors, and short outages should not reset the user's view.

## 10. Component Library And Extension Points

The first-party surface should publish its reusable browser widgets through a shared component library such as `@roko/ui`.

Load-bearing primitives include:

- `<ReasoningStream>`
- `<ToolBanner>`
- `<GateBadge>`
- `<HeuristicFootnote>`
- `<UncertaintyBar>`
- `<ReplayScrubber>`
- `<ConsensusView>`
- `<ExplainabilityPanel>`
- `<CFactorGauge>`
- `<EpisodeCard>`
- `<TaskNode>`
- `<HeuristicRow>`
- `<CostMeter>`
- `<DiffView>`
- `<AgentAvatar>`

That component library is the contract third-party dashboards and plugin pages should build on.

The component set should encode progressive disclosure and spatial memory as defaults, not afterthoughts. Shared controls should open the same detail in the same place across pages whenever possible, and compact modes should preserve the same information hierarchy on smaller screens.

Plugins may extend the web surface in three ways:

- add custom `Home / Pulse` tiles backed by their own projections
- register opt-in top-level pages
- provide alternate visualizations for an existing projection

Lazy loading is required for plugin bundles, and third-party UI should be sandboxed so styling and permissions remain bounded.

## 11. Development And Shipping Sequence

A credible first-party browser release can ship incrementally:

1. `SvelteKit` shell, auth, and realtime connection.
2. `Home / Pulse` plus `Chat`.
3. `Plans`.
4. `Beliefs`.
5. `Settings`.
6. `@roko/ui` component library polish and docs.
7. Accessibility audit and performance hardening.

This order keeps the critical product promise intact early: even `Home / Pulse` plus `Chat` is already a meaningful release.

## 12. Why The Browser Surface Earns Its Keep

The CLI remains the strongest operating surface for developers. The Web Portal earns its place elsewhere:

- non-developers can observe without terminal fluency
- teams can share a common live view
- mobile users can check status quickly
- demos and reviews work better in a browser
- deep links and replay URLs make discussion easier

The browser surface therefore optimizes for observation-first continuity with selective operation, while the CLI and TUI stay the primary heavy-control surfaces.

## 13. Related Refinements

- [tmp/refinements/29-web-ui-architecture.md](../../tmp/refinements/29-web-ui-architecture.md) — canonical source for this chapter.
- [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md) — projection layer consumed by the Web Portal.
- [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md) — transport contract for projection snapshots, deltas, and cursor resume.
- [tmp/refinements/28-cli-parity-familiar-workflows.md](../../tmp/refinements/28-cli-parity-familiar-workflows.md) — browser affordances should preserve slash commands, diff-first review, and replay semantics where appropriate.
- [tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md) — plugin-contributed pages, tiles, and visualization hooks should reuse the same browser extension boundary.
- [tmp/refinements/30-rich-ux-primitives.md](../../tmp/refinements/30-rich-ux-primitives.md) — replay scrubbers, footnotes, banners, and diff review primitives layered on top of this architecture.
- [22-statehub-projection-layer.md](./22-statehub-projection-layer.md) — the projection store that feeds the browser primitives and replay views.
- [../00-architecture/13-cognitive-cross-cuts.md](../00-architecture/13-cognitive-cross-cuts.md) — cross-cut guidance for keeping browser affordances aligned with the broader loop and operator model.
- [tmp/refinements/32-safety-sandbox-provenance.md](../../tmp/refinements/32-safety-sandbox-provenance.md) — permission model for mutating actions and plugin surfaces.
- [tmp/refinements/33-observability-telemetry.md](../../tmp/refinements/33-observability-telemetry.md) — browser telemetry, error tracking, and client performance reporting.
