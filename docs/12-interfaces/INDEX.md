# Topic 12: Interfaces

> CLI, HTTP API, TUI dashboard, Web Portal, Spectre creatures, sonification, StateHub projections, and generative UI — every way an operator interacts with Roko's cognitive agents.

---

## Overview

This topic covers all user-facing interfaces in Roko: the CLI binary (`roko`), the HTTP API server (`roko-serve`), the TUI terminal dashboard (ratatui-based), chat-oriented interaction surfaces, the Web Portal, the Spectre creature visualization system, ambient sonification, and the A2UI generative interface protocol. REF23 makes the chapter's primary UX claim explicit: Roko has four surfaces — CLI, TUI, Chat, and Web — exposing one unified verb set over the same Bus-backed progress stream and the same durable session/episode state. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) and [tmp/refinements/23-user-ux-running-agents.md](../../tmp/refinements/23-user-ux-running-agents.md). REF24 adds the deployment-facing complement: those same surfaces should configure and observe the same binary across five shapes — laptop, single-server, container, clustered, and edge — through profile-aware config and standard control-plane endpoints. See [../19-deployment/INDEX.md](../19-deployment/INDEX.md), [../19-deployment/10-secret-management.md](../19-deployment/10-secret-management.md), and [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md). REF25 adds the domain-specific-agent complement: onboarding and day-two control surfaces should let users install a domain profile, compose multiple profiles, inspect `TypedContext`, and review `Custody` for auditable actions. See [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md), [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [../02-agents/INDEX.md](../02-agents/INDEX.md), [../18-tools/INDEX.md](../18-tools/INDEX.md), and [tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md). REF26 adds StateHub as the kernel projection layer that lets TUI, Web, and external consumers share typed live views over Bus + Substrate. See [22-statehub-projection-layer.md](./22-statehub-projection-layer.md), [../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and [tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md). REF27 adds the shared realtime surface carried over WebSocket, SSE, and optional gRPC so those same projections reach browsers, bots, dashboards, and peer Roko instances through one cursor-aware protocol. See [06-websocket-streaming.md](./06-websocket-streaming.md) and [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md). All interfaces share the ROSEDUST design language and consume the same underlying data model (Engrams, Pulses, Topics, plugin capabilities, behavioral states, c-factor metrics, knowledge tiers, domain-profile metadata, and StateHub projections).

**Key design principles:**
- **Progressive disclosure**: Overview first, detail on demand
- **One verb set, four surfaces**: `ask`, `plan`, `do`, `watch`, `inspect`, `replay`, `learn`, `tune`, `connect`
- **ROSEDUST everywhere**: One design language across TUI, Web, and CLI
- **Realtime surface**: One protocol across WebSocket, SSE, and optional gRPC
- **Shared projections**: StateHub gives TUI, Web, and external consumers the same typed live views
- **Spectre as readout**: Every agent has a procedurally generated creature that encodes cognitive state
- **No ending framing**: Spectres stay continuous; music reflects engagement, not lifecycle

---

## Sub-Documents

| # | Document | Summary |
|---|---|---|
| 00 | [00-cli-overview.md](./00-cli-overview.md) | CLI as primary interface — 5 operating modes, plugin workflow, command groups, global flags, event system, zero-to-agent quickstart |
| 01 | [01-cli-command-reference.md](./01-cli-command-reference.md) | Full command reference organized by group — Getting Started, Plugins, Scaffolding, Orchestration, PRD, Research, Knowledge, Infrastructure, Debugging, Deployment |
| 02 | [02-roko-new-scaffolders.md](./02-roko-new-scaffolders.md) | `roko new` scaffolder specifications for all 9 types (domain, gate, scorer, router, policy, substrate, probe, event-source, template) with generated code examples |
| 03 | [03-progressive-help-and-explain.md](./03-progressive-help-and-explain.md) | Progressive disclosure help system — `roko explain` 3-level output, error-as-teacher format, config wizard, TeachingError struct |
| 04 | [04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md) | Layered config resolution (CLI → env → TOML → defaults), profile-aware deployment overlays for laptop/single-server/container/clustered/edge, `ROKO_*` env vars, full `roko.toml` schema, auto-detection |
| 05 | [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) | roko-serve architecture — axum HTTP server, 12 route groups, REST endpoints, authentication, deployment probes, and profile-aware control-plane behavior |
| 06 | [06-websocket-streaming.md](./06-websocket-streaming.md) | Shared realtime surface — one protocol over WebSocket, SSE, and optional gRPC with `query`, `subscribe`, `publish`, channel taxonomy, cursors, auth, back-pressure, and client guidance |
| 07 | [07-rosedust-design-language.md](./07-rosedust-design-language.md) | ROSEDUST design system — void-black palette, rose accents, glass morphism (CSS + Rust), motion system, typography, RosedustTheme struct |
| 08 | [08-tui-main-layout.md](./08-tui-main-layout.md) | TUI main layout — 3 regions (sidebar, detail, Spectre viewport), rendering architecture, 60fps frame budget, bloom composite, responsive breakpoints |
| 09 | [09-tui-29-screens.md](./09-tui-29-screens.md) | Complete 29-screen inventory across 6 regions — Navigation (6), Agent Detail (6), Plan Detail (5), Knowledge (4), Collective (4), System (4) |
| 10 | [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) | Spectre creature system — deterministic generation from agent hash, dot-cloud geometry, spring physics, 6 behavioral state animations, eye/glow/tendril systems |
| 11 | [11-spectre-rendering-per-interface.md](./11-spectre-rendering-per-interface.md) | Per-renderer Spectre implementations — TUI ASCII rasterization, Web Portal WebGL, CLI inline, API JSON state |
| 12 | [12-spectre-as-collective-display.md](./12-spectre-as-collective-display.md) | Multi-agent Spectre visualization — filament connections, pheromone fields, breathing synchronization (Kuramoto coupling), C-Factor harmony encoding |
| 13 | [13-web-portal.md](./13-web-portal.md) | Web Portal — React 19 + Next.js 15.5+, ROSEDUST Tailwind config, glass morphism CSS, 9 pages, WebGL Spectre, WebSocket integration |
| 14 | [14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md) | Onboarding journey — domain selection, template instantiation, model routing, profile install/composition, knowledge bootstrapping, Spectre generation, first-task validation |
| 15 | [15-generative-interfaces-a2ui.md](./15-generative-interfaces-a2ui.md) | A2UI protocol — agents emit JSONL UI descriptions, 12 component types, ROSEDUST inheritance, sandboxed rendering across TUI/Web/CLI |
| 16 | [16-sonification-reframed.md](./16-sonification-reframed.md) | Ambient sonification — Eno mandate, 5 musical layers, 8 behavioral state presets, emotional harmonic vocabulary. No lifecycle audio. |
| 17 | [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md) | WCAG 2.1 AA targets, keyboard nav, screen reader support, reduced motion, port allocation, comprehensive implementation status |
| 18 | [18-ux-innovation-proposals.md](./18-ux-innovation-proposals.md) | 7 UX innovations — Conversational Development, Time-Travel Debugging, Dream Journal, Agent Garden, Pair Programming with Affect, Collaborative Planning, Knowledge Map |
| 19 | [19-rust-sdk-developer-ux.md](./19-rust-sdk-developer-ux.md) | Four-layer Rust SDK — one-liner, builder, trait impl, runtime impl, typed errors, docs/examples discipline, `cargo roko`, macros, testing ergonomics, tracing, release compatibility |
| 21 | [21-user-ux-running-agents.md](./21-user-ux-running-agents.md) | REF23 canonical user-UX chapter — four surfaces, unified verb set, interactive first-run, live progress, checkpoints, undo, named/shareable sessions, accessibility, and domain-profile install/composition flows from REF25 |
| 22 | [22-statehub-projection-layer.md](./22-statehub-projection-layer.md) | REF26 canonical StateHub chapter — projection trait, canonical projections, query+subscribe, filters, transport-agnostic delivery, replay/snapshot/testing, and shared TUI/Web/external consumers |

---

## Prerequisites

This topic builds on:

| Topic | What it provides |
|---|---|
| [01-core](../00-architecture/INDEX.md) | Engram data model, Synapse traits |
| [03-scaffold](../01-orchestration/INDEX.md) | Context engineering, prompt assembly |
| [04-harness](../04-verification/INDEX.md) | Gate pipeline (displayed in TUI) |
| [05-orchestration](../01-orchestration/INDEX.md) | Plan DAG (displayed in TUI, Portal) |
| [06-learning](../05-learning/INDEX.md) | CascadeRouter, episodes, predictions |
| [07-cfactor](../14-identity-economy/INDEX.md) | C-Factor metrics (displayed in all interfaces) |
| [08-mesh](../13-coordination/INDEX.md) | Agent Mesh, pheromones, stigmergy |
| [09-daimon](../09-daimon/INDEX.md) | PAD vector, behavioral states |
| [10-dreams](../10-dreams/INDEX.md) | Dreams consolidation (reflected in Spectre Resting state) |
| [11-neuro](../06-neuro/INDEX.md) | Knowledge store, tier progression |

---

## Key Architectural Decisions

1. **ROSEDUST is dark-only**: No light theme variant. The void-black background is integral to the design language, not a preference.

2. **TUI is the primary development interface**: The CLI + TUI combination is the most capable interface. The Portal is a monitoring/analysis complement, not a replacement.

3. **Spectre creatures are information displays, not decoration**: Every visual property traces to a data source. If it can't be grounded in data, it doesn't exist.

4. **One realtime protocol, multiple transports**: WebSocket, SSE, and optional gRPC carry the same subscription vocabulary, cursors, and auth model.

5. **A2UI is agent-authored, not user-authored**: Agents create UI components; users consume them. The rendering is always in ROSEDUST.

6. **Sonification is optional and ambient**: Audio enhances but never replaces visual interfaces. It can be fully disabled.

---

## Crates and Source Locations

| Component | Crate | Key Files |
|---|---|---|
| CLI binary | `roko-cli` | `crates/roko-cli/src/main.rs`, `src/lib.rs` |
| TUI framework | `roko-cli` | `crates/roko-cli/src/tui/` (app, dashboard, theme, color, views/, widgets/, modals/) |
| HTTP server | `roko-serve` | `crates/roko-serve/src/` (lib.rs, routes/, state.rs, event_bus.rs) |
| System prompt builder | `roko-compose` | `crates/roko-compose/src/system_prompt_builder.rs` |
| ROSEDUST theme | `roko-cli` | `crates/roko-cli/src/tui/theme.rs`, `crates/roko-cli/src/tui/color.rs` |

---

## Generation Notes

This topic was generated from:
- `refactoring-prd/06-interfaces.md` (primary architectural source)
- `refactoring-prd/08-translation-guide.md` (naming and conceptual reframes)
- `refactoring-prd/09-innovations.md` (A2UI, Spectre, sonification innovations)
- `refactoring-prd/10-developer-guide.md` (DX principles, onboarding)
- `bardo-backup/prd/18-interfaces/` (legacy interface specs: CLI, Portal, TUI, creature system)
- `bardo-backup/prd/24-sonification/` (legacy music specs: musical language, preset catalog)
- `bardo-backup/prd/shared/` (branding, port allocation)
- Active code: `roko-cli/src/tui/`, `roko-serve/src/routes/`, `roko-cli/src/main.rs`

All naming follows the authoritative naming map. Legacy/renamed terms include bardo→Roko, golem→agent, grimoire→Neuro, Signal→Engram, clade→collective/mesh, and mori→Roko Orchestrator. Legacy lifecycle and death framing has been removed per the reframe rules. Sonification presets have been remapped from lifecycle phases to behavioral states.
REF17 adds the interface-side plugin surface; start with [00-cli-overview.md](./00-cli-overview.md),
[01-cli-command-reference.md](./01-cli-command-reference.md), and
[tmp/refinements/17-plugin-extension-architecture.md](../../tmp/refinements/17-plugin-extension-architecture.md).
REF22 adds the Rust SDK developer-UX chapter; start with [19-rust-sdk-developer-ux.md](./19-rust-sdk-developer-ux.md) and [tmp/refinements/22-developer-ux-rust.md](../../tmp/refinements/22-developer-ux-rust.md).
REF24 adds the profile-aware deployment and control-plane framing; start with
[04-configuration-layered-resolution.md](./04-configuration-layered-resolution.md),
[05-http-api-roko-serve.md](./05-http-api-roko-serve.md),
[../19-deployment/INDEX.md](../19-deployment/INDEX.md),
[../19-deployment/10-secret-management.md](../19-deployment/10-secret-management.md),
and [tmp/refinements/24-deployment-ux.md](../../tmp/refinements/24-deployment-ux.md).
REF25 adds the domain-profile install/composition framing; start with
[00-cli-overview.md](./00-cli-overview.md), [01-cli-command-reference.md](./01-cli-command-reference.md),
[14-agent-onboarding-flow.md](./14-agent-onboarding-flow.md),
[21-user-ux-running-agents.md](./21-user-ux-running-agents.md), and
[tmp/refinements/25-domain-specific-agents.md](../../tmp/refinements/25-domain-specific-agents.md).
REF26 adds the StateHub projection layer; start with
[22-statehub-projection-layer.md](./22-statehub-projection-layer.md),
[../00-architecture/01-naming-and-glossary.md](../00-architecture/01-naming-and-glossary.md), and
[tmp/refinements/26-statehub-rearchitecture.md](../../tmp/refinements/26-statehub-rearchitecture.md).
REF27 adds the shared realtime surface; start with
[06-websocket-streaming.md](./06-websocket-streaming.md),
[05-http-api-roko-serve.md](./05-http-api-roko-serve.md),
[13-web-portal.md](./13-web-portal.md),
[../19-deployment/11-remote-orchestrator.md](../19-deployment/11-remote-orchestrator.md), and
[tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md).
