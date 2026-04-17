# Web Portal

> The Roko Portal — a React 19 + Next.js 15.5+ web dashboard for monitoring and controlling cognitive agents. ROSEDUST design language in CSS, glass morphism panels, WebGL Spectre rendering, and the shared realtime surface carried over WebSocket or SSE.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md), [05-http-api-roko-serve.md](./05-http-api-roko-serve.md), [06-websocket-streaming.md](./06-websocket-streaming.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §6, `bardo-backup/prd/18-interfaces/00-portal.md`

---

## Abstract

The Roko Portal is a web-based dashboard that provides the same monitoring and control capabilities as the TUI, with richer visualization affordances. Built on React 19 and Next.js 15.5+, it uses the ROSEDUST design language adapted for CSS/Tailwind, renders Spectre creatures in WebGL, and consumes shared `StateHub` projections through the realtime surface exposed by `roko-serve`.

The Portal is designed for three personas: **operators** monitoring active agent execution, **analysts** reviewing historical performance data, and **administrators** configuring the system. Under REF23, the initial first-party web scope should stay small and discoverable: a Web rendering of the same unified verb set and the same named sessions used by CLI, TUI, and Chat. REF27 adds the delivery rule: browser pages should speak the same `query`, `subscribe`, and optional `publish` vocabulary as every other consumer, rather than inventing a portal-only socket API. See [21-user-ux-running-agents.md](./21-user-ux-running-agents.md), [06-websocket-streaming.md](./06-websocket-streaming.md), and [tmp/refinements/27-realtime-event-surface.md](../../tmp/refinements/27-realtime-event-surface.md).

---

## REF23 Scope Reset

The first-party Web surface should start with a compact page set that covers the shared verb set cleanly:

| Page | Primary verbs |
|---|---|
| Home | `watch`, `inspect` |
| Ask | `ask`, `watch` |
| Plans | `plan`, `do`, `watch` |
| Episodes | `inspect`, `replay` |
| Heuristics | `learn`, `inspect` |
| Settings | `tune`, `connect` |

Richer pages remain valid later, but parity and continuity matter more than page count. The web UI should feel like the same product, not a second product.

---

## Technology Stack

| Layer | Technology | Version | Purpose |
|---|---|---|---|
| Framework | Next.js | 15.5+ | React server components, routing, API routes |
| UI library | React | 19 | Component model, concurrent features |
| Styling | Tailwind CSS | 4.x | Utility-first CSS with ROSEDUST config |
| 3D rendering | Three.js | latest | WebGL Spectre creatures |
| Charts | Recharts or Nivo | latest | Performance charts, C-Factor trends |
| Realtime client | native `WebSocket` plus browser `SSE` fallback | — | Shared `query + subscribe` surface from roko-serve |
| State | React Query + Zustand | latest | Server state caching + local UI state |
| Auth | Bearer token | — | `roko_sk_*` API key in session storage |

### ROSEDUST Tailwind Configuration

```javascript
// tailwind.config.js (ROSEDUST palette)
module.exports = {
  theme: {
    extend: {
      colors: {
        rosedust: {
          // Base
          'void':         '#1A1520',
          'twilight':     '#221D2A',
          'dusk':         '#3A3345',
          // Rose palette
          'rose':         '#D4778C',
          'rose-muted':   '#A05C6E',
          'rose-bright':  '#E8A0B4',
          // Accent colors
          'gold':         '#D4A857',
          'teal':         '#5DB8A3',
          'sapphire':     '#6B8FBD',
          'violet':       '#A08CC4',
          'coral':        '#C47A5C',
          // Semantic
          'success':      '#5DB8A3',
          'warning':      '#D4A857',
          'danger':       '#C45C50',
          'info':         '#6B8FBD',
          // Text
          'fg':           '#E8DFD5',
          'fg-muted':     '#8A7F8E',
        }
      },
      backdropBlur: {
        'glass': '16px',
      },
      borderColor: {
        'glass': 'rgba(212, 119, 140, 0.08)',
      },
      backgroundColor: {
        'glass': 'rgba(34, 29, 42, 0.72)',
      },
      boxShadow: {
        'glow': '0 0 20px rgba(212, 119, 140, 0.15)',
        'glow-strong': '0 0 40px rgba(212, 119, 140, 0.25)',
      },
      transitionTimingFunction: {
        'luxury': 'cubic-bezier(0.16, 1, 0.3, 1)',
      }
    }
  }
}
```

### Glass Morphism CSS

```css
.glass-panel {
  background: rgba(34, 29, 42, 0.72);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(212, 119, 140, 0.08);
  border-radius: 12px;
  box-shadow: 0 0 20px rgba(212, 119, 140, 0.15);
}

.glass-panel:hover {
  border-color: rgba(212, 119, 140, 0.15);
  box-shadow: 0 0 30px rgba(212, 119, 140, 0.2);
  transition: all 0.6s cubic-bezier(0.16, 1, 0.3, 1);
}
```

---

## Page Structure

### Page 1: Dashboard (Home)

The landing page — equivalent to the TUI main layout.

```
┌─────────────────────────────────────────────────────────┐
│  ROKO                                    C: 1.23  $12.34│
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌─ Agents ──────┐  ┌─ Active Plan ──────────────────┐  │
│  │ glass panel   │  │ glass panel                    │  │
│  │ agent cards   │  │ DAG visualization              │  │
│  │ with mini     │  │ task progress bars             │  │
│  │ spectres      │  │ gate status                    │  │
│  └───────────────┘  └────────────────────────────────┘  │
│                                                          │
│  ┌─ C-Factor ────┐  ┌─ Knowledge ───┐  ┌─ Health ──┐   │
│  │ gauge chart   │  │ tier bars     │  │ provider  │   │
│  │ trend sparkle │  │ recent items  │  │ status    │   │
│  └───────────────┘  └───────────────┘  └───────────┘   │
│                                                          │
│  ┌─ Spectre Viewport ────────────────────────────────┐  │
│  │ WebGL canvas — focused agent Spectre creature     │  │
│  │ (orbit, zoom, hover for data)                     │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

**Components:**
- Agent cards with mini Spectre (canvas thumbnails), status, behavioral state color
- Active plan DAG (interactive, click tasks for detail)
- C-Factor gauge with trend sparkline
- Knowledge tier summary bars
- Provider health indicators
- Full WebGL Spectre viewport for the focused agent

### Page 2: Agent Detail

Deep dive into a single agent — equivalent to TUI Region 2.

**Sections:**
- **Header**: Agent name, model, status, Spectre (large WebGL render)
- **Output Stream**: Live scrolling output with tool call highlighting
- **Gate Pipeline**: Visual rung diagram with pass/fail history
- **Daimon State**: PAD vector visualization (3D or radar chart), state timeline
- **Predictions**: Table of active predictions with calibration chart
- **Tool Trace**: Collapsible tool call log with timing
- **Cost**: Token usage chart, model breakdown pie chart, budget gauge

### Page 3: Plan Detail

Deep dive into a plan — equivalent to TUI Region 3.

**Sections:**
- **DAG**: Interactive plan graph (drag nodes, zoom, pan)
- **Task List**: Table of all tasks with status, agent, duration, cost
- **Merge Queue**: Branch list with gate status and merge readiness
- **Timeline**: Gantt chart of task execution
- **Worktrees**: Git worktree status cards

### Page 4: Collective Intelligence

Multi-agent dynamics — equivalent to TUI Region 5.

**Sections:**
- **C-Factor Dashboard**: Large gauge, trend chart (last 24h), component breakdown
- **Agent Comparison**: Multi-column comparison table with sortable metrics
- **Spectre Gallery**: WebGL collective view (all Spectres in shared 3D space with filaments)
- **Pheromone Landscape**: Heat map of pheromone activity over time
- **Stigmergy Events**: Timeline of indirect coordination events

### Page 5: Knowledge Explorer

Neuro knowledge store — equivalent to TUI Region 4.

**Sections:**
- **Search**: Full-text search with type and tier filters
- **Entry Detail**: Full entry view with score, lineage, decay countdown
- **Tier Progression**: Animated flow diagram (Sankey chart) of knowledge promotion/decay
- **Cross-Domain Map**: Force-directed graph of domain clusters with resonance edges
- **Knowledge Graph**: Interactive graph (click node → entry detail)

### Page 6: Plans

Plan list and management — equivalent to TUI Screen 1.2.

**Features:**
- Plan list with status filters
- Create new plan (form)
- Upload plan TOML
- Plan comparison (side-by-side)

### Page 7: System

Infrastructure monitoring — equivalent to TUI Region 6.

**Sections:**
- **Provider Health**: Per-provider cards with latency charts, rate limits, error rates
- **Resource Monitor**: CPU, memory, disk, network gauges
- **Activity Feed**: Searchable, filterable real-time activity stream
- **Circuit Breaker Status**: Per-provider circuit state (Closed/Open/HalfOpen)

### Page 8: Configuration

System configuration — equivalent to TUI Screen 1.6 plus editing capabilities.

**Features:**
- Visual roko.toml editor (form-based, validates on change)
- Model routing configuration (drag-and-drop tier assignment)
- Gate pipeline configuration (enable/disable, threshold sliders)
- MCP server management (add/remove/test)
- Budget configuration (daily/monthly limits, alerts)

### Page 9: Episodes & History

Historical analysis — equivalent to TUI Screen 1.5 with richer analysis.

**Features:**
- Episode timeline (filterable by agent, plan, outcome)
- Episode detail (turn-by-turn replay)
- Performance trends (charts over time)
- Cost analysis (breakdowns by period, agent, model, plan)
- Learning metrics (prediction accuracy, gate pass rate trends)

The six-page REF23 scope above is the first release bar. Additional pages and richer visualizations should be layered in only after the shared verb set, live progress, and session continuity are solid.

---

## Realtime Client Integration

The Portal should consume the same realtime protocol described in [06-websocket-streaming.md](./06-websocket-streaming.md). `WebSocket` is the default for interactive pages because the browser may need to `publish` user-originated Pulses. Passive views can fall back to `SSE` while keeping the same channel names, cursors, and reducers.

### Connection Management

```typescript
// Browser client over the shared realtime surface
class RokoRealtimeClient {
  private ws?: WebSocket;
  private reconnectAttempts = 0;
  private cursors = new Map<string, string>();

  connect(url: string, token: string) {
    this.ws = new WebSocket(url, []);
    this.ws.onopen = () => {
      this.ws.send(JSON.stringify({
        type: "subscribe",
        id: "cohort-health",
        payload: {
          channel: "projection:cohort_health",
          cursor: this.cursors.get("projection:cohort_health") ?? null,
          filter: { tenant: "acme" },
          mode: "Coalesce"
        }
      }));
    };
    this.ws.onmessage = ({ data }) => {
      const frame = JSON.parse(data);
      if (frame.cursor && frame.payload?.channel) {
        this.cursors.set(frame.payload.channel, frame.cursor);
      }
      this.dispatch(frame);
    };
    this.ws.onclose = () => this.reconnect();
  }

  reconnect() {
    const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);
    setTimeout(() => {
      this.reconnectAttempts++;
      this.connect(this.endpoint, this.token);
    }, delay);
  }
}
```

### Recommended Portal Subscriptions

| Channel | Page | Why it exists |
|---|---|---|
| `projection:active_tasks` | Home, Plans | task progress, ETA, and current assignee |
| `projection:agent_trails` | Ask, Agent Detail | token streaming, tool banners, and current action |
| `projection:gate_pipeline` | Plans, Episodes | rung status and verification deltas |
| `projection:cohort_health` | Home, Collective | c-factor, roster, and delivery metrics |
| `agent:agt_042` | Agent Detail | convenience alias for focused agent activity |
| `session:sess_xyz` | Ask, Episodes | cross-surface session continuity |

### React Query Integration

WebSocket data is fed into React Query's cache for consistent state management:

```typescript
// Realtime surface -> React Query bridge
function useRealtimeQuery<T>(channel: string, queryKey: string[]) {
  const queryClient = useQueryClient();

  useEffect(() => {
    const client = new RokoRealtimeClient();
    client.connect(getStreamUrl(), getToken());
    client.onMessage((frame: { type: string; payload: T }) => {
      if (frame.type === "state" || frame.type === "delta") {
        queryClient.setQueryData(queryKey, frame.payload);
      }
    });
    client.subscribe(channel);
    return () => client.disconnect();
  }, [channel]);

  return useQuery({ queryKey, queryFn: fetchInitial });
}
```

The important point is architectural, not library-specific: the Portal should hydrate from a normal HTTP `query`, then `subscribe` to the same named channel and fold deltas as they arrive. That keeps browser state aligned with TUI, CLI `watch`, and external dashboards.

---

## WebGL Spectre Integration

### Three.js Scene Setup

```typescript
// Spectre WebGL renderer
class SpectreRenderer {
  private scene: THREE.Scene;
  private camera: THREE.PerspectiveCamera;
  private renderer: THREE.WebGLRenderer;
  private composer: EffectComposer; // post-processing

  constructor(canvas: HTMLCanvasElement) {
    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color('#1A1520'); // void-black

    this.camera = new THREE.PerspectiveCamera(45, aspect, 0.1, 100);
    this.camera.position.z = 5;

    this.renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
    });

    // ROSEDUST bloom post-processing
    this.composer = new EffectComposer(this.renderer);
    this.composer.addPass(new RenderPass(this.scene, this.camera));
    this.composer.addPass(new UnrealBloomPass(
      new THREE.Vector2(width, height),
      0.8,  // bloom strength
      0.4,  // bloom radius
      0.85  // bloom threshold
    ));
  }

  updateFromState(state: SpectreState) {
    // Update point cloud positions from spring physics
    // Update colors from behavioral state
    // Update eye state
    // Update breathing scale
    // Update glow intensity
    // Update tendrils from mesh connections
    // Update particles from pheromone emission
  }

  animate() {
    requestAnimationFrame(() => this.animate());
    // Tick spring physics
    // Tick breathing animation
    // Tick particle system
    this.composer.render();
  }
}
```

### React Component

```tsx
function SpectreViewport({ agentId }: { agentId: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<SpectreRenderer>();
  const spectreState = useWebSocketQuery<SpectreState>(
    `/ws/spectre/${agentId}`,
    ['spectre', agentId]
  );

  useEffect(() => {
    if (canvasRef.current && !rendererRef.current) {
      rendererRef.current = new SpectreRenderer(canvasRef.current);
      rendererRef.current.animate();
    }
  }, []);

  useEffect(() => {
    if (spectreState.data && rendererRef.current) {
      rendererRef.current.updateFromState(spectreState.data);
    }
  }, [spectreState.data]);

  return (
    <div className="glass-panel p-4">
      <canvas
        ref={canvasRef}
        className="w-full h-64 rounded-lg"
      />
      <div className="mt-2 flex justify-between text-rosedust-fg-muted text-sm">
        <span>{spectreState.data?.behavioral_state}</span>
        <span>{spectreState.data?.animation.breathing_rate}Hz</span>
      </div>
    </div>
  );
}
```

---

## Authentication

The Portal authenticates to roko-serve using the same API key system as the CLI:

1. **Login page**: User enters `roko_sk_*` API key
2. **Session storage**: Key stored in `sessionStorage` (not `localStorage` — cleared on tab close)
3. **Request auth**: Key sent as `Authorization: Bearer roko_sk_*` on all API/WS requests
4. **Token refresh**: Not needed — API keys don't expire (but can be revoked via config)

### No External Auth

The Portal does not integrate external OAuth providers. It is designed for local/team use where the API key is sufficient. The API key is generated by `roko config` and stored in `roko.toml`.

---

## Responsive Design

The Portal uses Tailwind responsive breakpoints:

| Breakpoint | Layout |
|---|---|
| < 640px (mobile) | Single column, stacked panels, mini Spectre |
| 640–1024px (tablet) | Two columns, condensed panels |
| 1024–1440px (desktop) | Full layout, Spectre viewport |
| > 1440px (wide) | Extra space for side-by-side comparisons |

### Mobile Considerations

- Touch gestures for WebGL Spectre (pinch zoom, two-finger rotate)
- Swipe between pages
- Bottom navigation bar
- Reduced animation (respect `prefers-reduced-motion`)

---

## Deployment

The Portal is served by roko-serve as static assets:

```
roko-serve
  ├── /api/*          → API routes (REST)
  ├── /ws/*           → WebSocket endpoints
  ├── /api/sse/*      → SSE endpoints
  └── /*              → Static Portal assets (Next.js export)
```

### Build and Serve

```bash
# Build Portal (static export)
cd portal/
npm run build  # Next.js static export → out/

# Serve via roko-serve
cargo run -p roko-cli -- serve
# Portal available at http://localhost:3000
# API available at http://localhost:3000/api/
```

### Port Allocation

| Port | Service |
|---|---|
| 3000 | Portal (static + API proxy) |
| 3001 | Portal dev server (Next.js dev) |
| 8080 | roko-serve API (direct) |

See [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md) for the full port allocation table.

---

## Current Status and Gaps

**Built:**
- roko-serve HTTP API with all route groups (`roko-serve/src/routes/`)
- WebSocket and SSE endpoints (scaffold)
- CORS configuration for Portal origin
- API authentication middleware

**Not yet built:**
- Portal application (React 19, Next.js 15.5+)
- ROSEDUST Tailwind configuration
- Glass morphism component library
- WebGL Spectre renderer (Three.js)
- WebSocket connection manager
- Dashboard page
- Agent Detail page
- Plan Detail page
- Collective Intelligence page
- Knowledge Explorer page
- Plans management page
- System monitoring page
- Configuration page
- Episodes & History page
- Responsive layout
- Mobile touch support

---

## Cross-References

- See [05-http-api-roko-serve.md](./05-http-api-roko-serve.md) for the REST API consumed by the Portal
- See [06-websocket-streaming.md](./06-websocket-streaming.md) for the WebSocket endpoints
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color system and design rules
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre state model
- See [11-spectre-rendering-per-interface.md](./11-spectre-rendering-per-interface.md) for WebGL renderer specification
- See [17-accessibility-and-current-status.md](./17-accessibility-and-current-status.md) for port allocation
