# TUI Screen Architecture

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the TUI as a Graph of Lens Cells with keyboard-driven routing, the 29 screens across 6 regions, ratatui immediate-mode rendering as the Observe protocol, and the Elm Architecture as a state management pattern.

---

## 1. The TUI as a Graph of Lens Cells

The Roko TUI (`roko dashboard`) is a ratatui-based terminal application that renders 29 screens organized into 6 regions. In unified vocabulary, the TUI is a **Graph of Lens Cells** -- each screen is a Lens Cell that reads specific StateHub projections via the Observe protocol and renders them as terminal output. The Lens Cells never mutate state directly; they are read-only projections of kernel state.

Region switching (F1-F7) is handled by a **Route Cell** that responds to keyboard Pulses. Tab/Shift-Tab cycles focus within a region. This is the same Route protocol used everywhere in the system, applied to navigation.

See [02-CELL.md](../../unified/02-CELL.md) for the Observe protocol and [20-SURFACES.md](../../unified/20-SURFACES.md) SS4.2 for the TUI rendering target specification.

**Source**: `crates/roko-cli/src/tui/` (all TUI code).

---

## 2. Immediate-Mode Rendering as the Observe Protocol

ratatui uses an immediate-mode rendering pattern: every frame, the entire visible UI is redrawn from the current model state. This maps directly to the Observe protocol from [02-CELL.md](../../unified/02-CELL.md) -- read state, render, no mutation.

```rust
// The core loop: read state → render → no mutation
const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_micros(1_000_000 / TARGET_FPS);

loop {
    // 1. Poll input (non-blocking) — keyboard Pulses
    // 2. Drain event channel (non-blocking) — StateHub deltas
    // 3. Tick animations (dt-based)
    // 4. Render (immediate mode — terminal.draw())
    //    This is the Observe protocol: read model → produce output
    // 5. Check exit condition
}
```

The render loop runs on the main thread. Data ingestion (Bus Pulses, file changes) runs on Tokio async tasks. Events flow through `crossbeam::channel`. No async in the hot render path.

### Frame Budget

At 60fps, each frame has 16.6ms:

| Phase | Budget | Notes |
|---|---|---|
| Input polling | ~0.1ms | Keyboard events via crossterm |
| Event drain | ~0.5ms | StateHub deltas from async tasks |
| Animation tick | ~1.0ms | Breathing, sparklines, phase portraits |
| Layout computation | ~0.5ms | ratatui constraint solving |
| Widget rendering | ~2.0ms | All Lens Cell renders |
| Spectre rasterization | ~1.5ms | ASCII/braille creature |
| Terminal flush | ~2.0ms | Differential buffer write |
| **Total** | **~7.6ms** | 54% of budget = comfortable headroom |

### Double-Buffered Differential Flush

ratatui maintains two `Buffer` instances. After `terminal.draw()`, only changed cells are flushed. Static elements (borders, labels) are drawn once, then never re-transmitted. Typical flush for an idle dashboard: < 50 cells changed per frame (< 1KB).

---

## 3. The 29 Screens in 6 Regions

Screens are Lens Cells organized by domain. Region switching is a Route Cell responding to F-key Pulses:

### Region 1: Navigation (6 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 1.1 Agent List | `agent_vitality` | All agents with status indicators (`◉` active, `○` idle, `◌` resting) |
| 1.2 Agent Card | `agent_vitality`, `agent_trails` | Compact agent summary with sparkline |
| 1.3 Plan List | `plans_list` | Plans with running/done/failed counts |
| 1.4 Plan Summary | `plan_detail/<id>` | Selected plan DAG overview |
| 1.5 Mesh Status | `cohort_health` | Peer count, pheromone count, sync status |
| 1.6 Health Summary | `cost_meter`, `c_factor` | Cost today, c-factor trend, provider health |

### Region 2: Agent Detail (6 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 2.1 Agent Overview | `agent_vitality`, `agent_trails` | Name, model, task progress, c-factor contribution |
| 2.2 Daimon State | `agent_vitality` | PAD vector bars, behavioral state label, color-coded |
| 2.3 Gate Results | `gate_pipeline` | Pipeline status with pass/fail/pending indicators |
| 2.4 Predictions | `agent_trails` | Active predictions with actual vs. predicted values |
| 2.5 Episodes | `recent_episodes` | Recent agent episodes with outcomes |
| 2.6 Chat | `agent_trails` | Full-duplex chat with running agent |

### Region 3: Plan Detail (5 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 3.1 Task DAG | `plan_detail/<id>` | Live state-graph with node colors |
| 3.2 Task Detail | `plan_detail/<id>`, `agent_trails` | Selected task: agent, output, gates |
| 3.3 Artifacts | Store query | Produced Signals per task |
| 3.4 Timeline | `plan_detail/<id>` | Gantt waterfall of task execution |
| 3.5 Cost Breakdown | `cost_meter` | Per-task cost with model breakdown |

### Region 4: Knowledge (4 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 4.1 Knowledge Browser | `knowledge_health` | Entries by type, confidence, age, demurrage balance |
| 4.2 Resonance Graph | `knowledge_health` | HDC similarity relationships |
| 4.3 Lineage Walker | Store query | Signal lineage DAG |
| 4.4 Dream History | `knowledge_health` | Dream cycle reports and consolidation |

### Region 5: Collective (4 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 5.1 C-Factor Dashboard | `c_factor` | Composite score with component breakdown |
| 5.2 Coordination Map | `cohort_health` | Agent cluster topology |
| 5.3 Pheromone Landscape | Bus (pheromone Pulses) | Active pheromone fields with decay |
| 5.4 Spectre Gallery | `agent_vitality` (all agents) | Multiple Spectres with breathing sync |

### Region 6: System (4 screens)

| Screen | Projection Consumed | What It Shows |
|---|---|---|
| 6.1 Space Config | `config_current` | Resolved configuration |
| 6.2 Provider Health | `bus_stats` | Provider status, circuit breakers |
| 6.3 Cost Analysis | `cost_meter` | Spend trends, model distribution |
| 6.4 Autonomy Controls | `agent_vitality`, `c_factor` | Per-capability autonomy slider (levels 0-4) |

---

## 4. The Elm Architecture (TEA) Pattern

The TUI uses the Elm Architecture for clean separation of model, update, and view:

```rust
/// Central application model -- all TUI state lives here.
pub struct Model {
    pub screen: Screen,           // Current active screen
    pub mode: InputMode,          // Normal / Command / Filter
    pub agents: Vec<AgentState>,  // Agent data from projections
    pub plans: Vec<PlanState>,    // Plan data from projections
    pub focus: FocusRing,         // Tab/Shift-Tab focus cycling
    pub modal: Option<Modal>,     // Help, task detail, plan detail overlays
    pub history: Vec<Screen>,     // Back-navigation stack
    pub animations: AnimationState, // dt-based animation ticking
}

/// Messages that drive state transitions (Pulses in unified vocabulary).
pub enum Message {
    NavigateTo(Screen),    // F-key or region shortcut
    NavigateBack,          // Esc
    FocusNext,             // Tab
    FocusPrev,             // Shift-Tab
    AgentUpdate(AgentEvent),  // StateHub delta
    GateResult(GateEvent),    // StateHub delta
    CFactorUpdate(f64),       // StateHub delta
    OpenModal(Modal),
    CloseModal,
    Tick(Duration),        // Animation frame
    Quit,
}

/// Pure update function -- no side effects.
fn update(model: &mut Model, msg: Message) -> Option<Message> {
    match msg {
        Message::NavigateTo(screen) => {
            let prev = std::mem::replace(&mut model.screen, screen);
            model.history.push(prev);
            None
        }
        Message::Tick(dt) => {
            model.animations.tick(dt);
            None
        }
        // ... other handlers
        _ => None,
    }
}

/// Pure view -- read model, produce rendering, no mutation.
fn view(model: &Model, frame: &mut Frame) {
    let theme = active_theme();
    // Header bar: project name, c-factor
    // Body: active tab content (the Lens Cell for the current screen)
    // Transport strip: up to 3 active Flows with controls
    // Status bar: mode, key hints
}
```

This architecture keeps the TUI testable: the `update` function is pure (given a model and a message, produce the next model), and the `view` function is pure (given a model, produce rendering instructions). No side effects in either.

---

## 5. The Transport Strip

The bottom transport strip is always visible -- a persistent Workbench surface in compact form:

```
Transport:
* doc-ingest   53%  $1.84/$10  4m 12s  [pause] [cancel] [detach]
| deploy-rc   pending human input  [respond]
```

It shows up to 3 active Flows with one-press controls. Urgency 2-3 Inbox items also appear here. The transport strip is the TUI's equivalent of a browser tab bar -- it provides awareness of background work regardless of which screen is focused.

---

## 6. State-Graph View

The plan detail screen (3.1) renders a live state-graph using ratatui-canvas with ROSEDUST node colors:

| Node State | Color | Visual |
|---|---|---|
| Completed | Jade (`#5DB8A3`) | Solid fill |
| In flight | Cyan, flashing | Pulsing outline |
| Queued | Amber (`#D4A857`) | Hollow outline |
| Failed | Crimson (`#C45C50`) | X mark |
| Awaiting human | Violet (`#A08CC4`) | Blinking indicator |
| Pruned | Dimmed | Ghost outline |

The graph uses braille markers for high-resolution rendering (2x4 sub-pixels per character cell, 8x effective resolution). Phase portraits for behavioral state use the same braille canvas.

---

## 7. File Watcher Integration

The TUI subscribes to filesystem changes via `notify::RecommendedWatcher`:

```rust
// Source: crates/roko-cli/src/tui/fs_watch.rs
// Watches: .roko/runs/, episodes.jsonl, artifacts/
// Triggers re-render on change -- reactive to daemon progress without polling
```

This means the TUI updates when agents write results, even if the agents are running in a separate daemon process. The file watcher is a Trigger Cell (see [13-TRIGGERS.md](../../unified/13-TRIGGERS.md)) that converts filesystem events into Bus Pulses.

---

## 8. Responsive Layout

The TUI adapts to terminal width:

| Terminal Width | Layout |
|---|---|
| < 80 columns | Single column -- navigation stacked above detail |
| 80-119 columns | Two columns -- navigation sidebar + detail panel |
| 120+ columns | Full layout with Spectre viewport |

---

## What This Enables

- **29 screens, one rendering loop**: All screens share the same 60fps immediate-mode loop, ROSEDUST theme, and keyboard navigation model.
- **Projection-driven updates**: Every screen reads from StateHub projections, so the TUI automatically reflects kernel state changes.
- **Keyboard-first**: Every mouse interaction has a keyboard equivalent. The TUI works over SSH, in tmux, without mouse support.
- **Offline mode**: When the daemon is unavailable, the TUI operates from cached `.roko/` files in read-only mode.

---

## Feedback Loops

- **Interactive verb actions**: The TUI renders the same 9-verb set as the CLI. `x` executes a plan, `p` pauses an agent, `a` approves an inbox item. Each action emits a Pulse through the Bus, which updates projections, which re-renders the TUI.
- **Animation as state encoding**: Breathing rate, sparkline shape, and phase portrait trajectory all encode agent state. An operator can glance at the TUI and assess system health in 200ms (the "monitoring zone" from Few's dashboard design).
- **Focus ring context**: The currently focused panel determines which keyboard shortcuts are active. The status bar always shows the available actions for the current focus.

---

## Open Questions

1. **Chat pane scope**: How deep should the TUI chat pane go? Full slash command support and diff review, or just text exchange with the focused agent?
2. **Screen count growth**: With 29 screens, discoverability is a concern. Should less-used screens be hidden behind a command palette rather than region shortcuts?
3. **Multi-space switching**: The TUI supports space switching (`w` key). How does this interact with the 29-screen model -- does each space have its own screen state, or is the screen shared?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Wire TUI tabs to StateHub projections | `crates/roko-cli/src/tui/` | Replace direct state reads with typed projection subscriptions |
| Implement 60fps render loop with animation | `crates/roko-cli/src/tui/app.rs` | Currently text-only; needs animation ticking and frame budget management |
| Build state-graph view with live colors | `crates/roko-cli/src/tui/` | ratatui-canvas DAG rendering with ROSEDUST node colors |
| Implement transport strip | `crates/roko-cli/src/tui/` | Persistent bottom bar showing active Flows and inbox items |
| Add responsive breakpoint handling | `crates/roko-cli/src/tui/` | Layout adaptation based on terminal width |
| Implement chat tab with slash commands | `crates/roko-cli/src/tui/` | Full-duplex agent chat within TUI |
| Implement command palette (`/` key) | `crates/roko-cli/src/tui/` | Fuzzy search across Graphs, Flows, artifacts, screens |
