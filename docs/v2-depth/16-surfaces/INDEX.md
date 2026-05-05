# 16-surfaces — Depth Index

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Every surface is a Lens Cell composition reading the same StateHub. CLI, TUI, HTTP, Web are projections — not separate systems.

---

## Depth docs (7)

| # | Filename | Covers |
|---|---|---|
| 01 | [01-surfaces-as-lens-composition.md](01-surfaces-as-lens-composition.md) | Core insight: surfaces as Lens compositions over StateHub, 9-verb model as Graph operations, projection catalog, query+subscribe protocol |
| 02 | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) | CLI subcommands as Graph triggers, layered config resolution, progressive help as React Cell, scaffolders as template Graphs |
| 03 | [03-tui-screen-architecture.md](03-tui-screen-architecture.md) | 29 TUI screens as Lens Cells, 6 regions with keyboard-driven Route Cell, ratatui immediate-mode as Observe protocol, event batching |
| 04 | [04-rosedust-and-spectre.md](04-rosedust-and-spectre.md) | ROSEDUST design tokens as config Signals, Spectre deterministic from HDC fingerprint, PAD-driven animation, 4 renderers as Cell specializations |
| 05 | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) | HTTP API as Connect Cell, ~85 routes, WebSocket/SSE as Bus subscriptions, frame vocabulary, cursor semantics, deployment shapes |
| 06 | [06-generative-interfaces-and-a2ui.md](06-generative-interfaces-and-a2ui.md) | A2UI as Extension emitting Pulses, 12 UI component kinds, sonification as React Cell, rich UX primitives as Pulse→Lens compositions |
| 07 | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) | SDK as Graph templates (Rack pattern), onboarding as Trigger Graph, ACP-first IDE strategy, domain profiles as parameterized Racks |

---

## Source docs (26)

### CLI

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/00-cli-overview.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |
| `docs/12-interfaces/01-cli-command-reference.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |
| `docs/12-interfaces/02-roko-new-scaffolders.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |
| `docs/12-interfaces/03-progressive-help-and-explain.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |
| `docs/12-interfaces/04-configuration-layered-resolution.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |
| `docs/CLI-REFERENCE.md` | **Absorbed** | [02-cli-and-command-graph.md](02-cli-and-command-graph.md) |

### HTTP API and WebSocket

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/05-http-api-roko-serve.md` | **Absorbed** | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) |
| `docs/12-interfaces/06-websocket-streaming.md` | **Absorbed** | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) |
| `docs/API-REFERENCE.md` | **Absorbed** | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) |

### TUI and design language

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/07-rosedust-design-language.md` | **Absorbed** | [04-rosedust-and-spectre.md](04-rosedust-and-spectre.md) |
| `docs/12-interfaces/08-tui-main-layout.md` | **Absorbed** | [03-tui-screen-architecture.md](03-tui-screen-architecture.md) |
| `docs/12-interfaces/09-tui-29-screens.md` | **Absorbed** | [03-tui-screen-architecture.md](03-tui-screen-architecture.md) |

### Spectre visualization

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/10-spectre-creature-visualization.md` | **Absorbed** | [04-rosedust-and-spectre.md](04-rosedust-and-spectre.md) |
| `docs/12-interfaces/11-spectre-rendering-per-interface.md` | **Absorbed** | [04-rosedust-and-spectre.md](04-rosedust-and-spectre.md) |
| `docs/12-interfaces/12-spectre-as-collective-display.md` | **Absorbed** | [04-rosedust-and-spectre.md](04-rosedust-and-spectre.md) |

### Web and portals

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/13-web-portal.md` | **Absorbed** | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) |
| `docs/12-interfaces/14-agent-onboarding-flow.md` | **Absorbed** | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) |

### Generative and advanced UX

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/15-generative-interfaces-a2ui.md` | **Absorbed** | [06-generative-interfaces-and-a2ui.md](06-generative-interfaces-and-a2ui.md) |
| `docs/12-interfaces/16-sonification-reframed.md` | **Absorbed** | [06-generative-interfaces-and-a2ui.md](06-generative-interfaces-and-a2ui.md) |
| `docs/12-interfaces/18-ux-innovation-proposals.md` | **Absorbed** | [06-generative-interfaces-and-a2ui.md](06-generative-interfaces-and-a2ui.md) |
| `docs/12-interfaces/23-rich-ux-primitives.md` | **Absorbed** | [06-generative-interfaces-and-a2ui.md](06-generative-interfaces-and-a2ui.md) |

### Developer experience

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/12-interfaces/19-rust-sdk-developer-ux.md` | **Absorbed** | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) |
| `docs/12-interfaces/20-ide-integration-strategy.md` | **Absorbed** | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) |
| `docs/12-interfaces/21-user-ux-running-agents.md` | **Absorbed** | [01-surfaces-as-lens-composition.md](01-surfaces-as-lens-composition.md) |
| `docs/12-interfaces/22-statehub-projection-layer.md` | **Absorbed** | [01-surfaces-as-lens-composition.md](01-surfaces-as-lens-composition.md) |

### Guides and benchmarks

| Source doc | Status | Absorbed by |
|---|---|---|
| `docs/QUICKSTART.md` | **Absorbed** | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) |
| `docs/INTEGRATION-GUIDE.md` | **Absorbed** | [07-developer-experience-and-onboarding.md](07-developer-experience-and-onboarding.md) |
| `docs/BENCHMARKS.md` | **Absorbed** | [05-http-api-and-realtime.md](05-http-api-and-realtime.md) |

---

26 of 26 source docs absorbed across 7 depth docs.

## Cross-references

- [09-telemetry/](../09-telemetry/) — StateHub origins, metric collection
- [07-agent-runtime/](../07-agent-runtime/) — Daimon PAD vectors (drive Spectre animation)
- [14-config/](../14-config/) — Configuration layering, domain profiles
- [22-code-intelligence/](../22-code-intelligence/) — MCP code server integration
