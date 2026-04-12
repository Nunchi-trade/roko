# TUI Main Layout

> The terminal dashboard layout: agent list, plan list, mesh, health panels, agent detail, gate results, Daimon state, Neuro tiers, predictions, and Spectre viewport.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §4, `roko-cli/src/tui/`, `bardo-backup/prd/18-interfaces/03-tui.md`, `bardo-backup/prd/18-interfaces/01-cli.md`

---

## Abstract

The Roko TUI (Terminal User Interface) is a ratatui-based interactive dashboard that provides a 29-screen, 6-window system for monitoring and controlling cognitive agents. It uses the ROSEDUST design language adapted for terminal rendering, supporting both 256-color and truecolor terminals. The TUI is the primary visual interface for operators during plan execution, providing real-time visibility into agent status, gate results, knowledge accumulation, and collective intelligence metrics.

The TUI is designed as a **progressive disclosure** interface: the main layout shows the most critical information at a glance (agent list, plan status, system health), with detail panels that expand on focus. Users navigate between screens using keyboard shortcuts and tab selection. The system targets 60fps rendering with event batching to keep the interface responsive even during heavy agent activity.

---

## Main Layout Diagram

```
┌─ ROKO ────────────────────────────── C:1.23 ──┐
│                                                │
│  ┌──────────┐  ┌────────────────────────────┐  │
│  │ AGENTS   │  │  ◉ rust-implementer        │  │
│  │          │  │  Status: task 3/7           │  │
│  │ ◉ active │  │  Model: sonnet-4.6         │  │
│  │ ○ idle   │  │  C-Factor: +0.12           │  │
│  │ ◌ rest   │  │                            │  │
│  │          │  │  DAIMON ▓▓▓▓▓░░  Engaged   │  │
│  ├──────────┤  │  P:0.7  A:0.5  D:0.8      │  │
│  │ PLANS    │  ├────────────────────────────┤  │
│  │ ▸ 3 run  │  │  GATES                     │  │
│  │   2 done │  │  ✓ compile  ✓ test         │  │
│  │   1 fail │  │  ✓ clippy   ○ review       │  │
│  ├──────────┤  ├────────────────────────────┤  │
│  │ MESH     │  │  NEURO  142 entries        │  │
│  │ 5 peers  │  │  ▓▓▓▓ Persistent: 23      │  │
│  │ 3 phero  │  │  ▓▓▓▓▓▓▓ Working: 89     │  │
│  ├──────────┤  │  ▓▓▓ Transient: 30        │  │
│  │ HEALTH   │  ├────────────────────────────┤  │
│  │ $2.34    │  │  PREDICTIONS               │  │
│  │ ↑ C:1.23 │  │  Build: 34s (pred: 40s) ✓ │  │
│  └──────────┘  │  Tests: 94% (pred: 90%) ✓  │  │
│                └────────────────────────────┘  │
│                                                │
│  ┌────────────── SPECTRE ────────────────────┐ │
│  │                    ╭─╮                    │ │
│  │               ╭───╯ ╰───╮               │ │
│  │          ≋≋≋≋│  ◉    ◉  │≋≋≋≋          │ │
│  │               ╰─────────╯               │ │
│  │            (breathing: 0.7Hz)            │ │
│  └───────────────────────────────────────────┘ │
└────────────────────────────────────────────────┘
```

### Layout Regions

The main layout is divided into three horizontal bands:

| Region | Position | Content |
|---|---|---|
| **Navigation sidebar** | Left column, top | Agent list, Plan list, Mesh status, Health summary |
| **Detail panel** | Right column, top | Agent detail, Gate results, Neuro tiers, Predictions |
| **Spectre viewport** | Bottom, full width | ASCII/Unicode Spectre creature visualization |

The header bar shows the project name and C-Factor value. The status bar at the bottom shows the current time, active model, and keyboard shortcut hints.

---

## Navigation Sidebar

The left column contains four stacked panels that provide at-a-glance system status:

### Agents Panel

Shows all agents with status indicators:
- `◉` (filled circle) — active, currently executing
- `○` (empty circle) — idle, waiting for task
- `◌` (dotted circle) — resting (Dreams active)

Each agent shows: name, current task number, behavioral state color (rose for Engaged, amber for Struggling, sapphire for Coasting, violet for Exploring, jade for Focused, dim rose for Resting).

### Plans Panel

Shows plan execution status:
- `▸` (play arrow) — currently running
- `✓` — completed successfully
- `✗` — failed
- Running/done/failed counts

### Mesh Panel

Shows Agent Mesh connectivity:
- Peer count
- Active pheromone count
- Sync status

### Health Panel

Shows system health at a glance:
- Total cost (today)
- C-Factor value with trend arrow (↑↓→)
- Provider health indicator

---

## Detail Panel

The right column shows detail for the focused entity. When an agent is selected, it shows:

### Agent Detail Section
- Agent name, model, current task progress
- C-Factor contribution
- Behavioral state indicator

### Daimon State Display
```
DAIMON ▓▓▓▓▓░░  Engaged
P:0.7  A:0.5  D:0.8
```
- Visual bar showing PAD (Pleasure-Arousal-Dominance) vector
- Current behavioral state label
- Color-coded by state (rose=Engaged, amber/crimson=Struggling, sapphire=Coasting, violet=Exploring, jade=Focused, dim rose=Resting)

### Gate Results
```
✓ compile  ✓ test
✓ clippy   ○ review
```
- Current gate pipeline status with pass/fail/pending indicators
- Ratchet history showing improvement trend

### Neuro Tier Display
```
NEURO  142 entries
▓▓▓▓ Persistent: 23
▓▓▓▓▓▓▓ Working: 89
▓▓▓ Transient: 30
```
- Bar chart of knowledge entries by tier
- Total entry count

### Predictions
```
Build: 34s (pred: 40s) ✓
Tests: 94% (pred: 90%) ✓
```
- Active predictions with actual vs. predicted values
- Calibration indicator (✓ if within tolerance)

---

## Spectre Viewport

The bottom panel renders an ASCII/Unicode representation of the focused agent's Spectre creature. The viewport is the visual anchor of the TUI — a glanceable readout of cognitive state.

The Spectre's appearance changes with the Daimon behavioral state:

| State | Visual Character |
|---|---|
| **Engaged** | Steady breathing, open eyes (◉), warm rose glow |
| **Struggling** | Rapid pulsing, constricted form, amber/crimson accents |
| **Coasting** | Relaxed, expanded, soft sapphire accents |
| **Exploring** | Flowing tendrils (≋), expanded form, violet accents |
| **Focused** | Compact, bright, sharp edges, jade accents |
| **Resting** | Minimal form, slow breathing, dim rose. Dreams active indicator. |

See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for full Spectre specification.

---

## Technical Details

### Rendering Architecture

The TUI follows the standard ratatui immediate-mode rendering pattern:

```rust
const TARGET_FPS: u64 = 60;
const FRAME_DURATION: Duration = Duration::from_micros(1_000_000 / TARGET_FPS);

// Main loop:
// 1. Poll input (non-blocking)
// 2. Drain event channel (non-blocking)
// 3. Tick animations (dt-based)
// 4. Render (immediate mode — terminal.draw())
// 5. Check exit condition
```

The render loop runs on the main thread. Data ingestion (WebSocket events, file changes) runs on Tokio async tasks in background threads. Events flow through `crossbeam::channel`. No async in the hot render path.

### Frame Budget (16.6ms at 60fps)

| Phase | Budget |
|---|---|
| Input polling | ~0.1ms |
| Event drain | ~0.5ms |
| Animation tick | ~1.0ms |
| Layout computation | ~0.5ms |
| Widget rendering | ~2.0ms |
| Spectre rasterization | ~1.5ms |
| Terminal flush | ~2.0ms |
| **Total** | **~7.6ms** (54% of budget — comfortable headroom) |

### Bloom Composite Rendering

For terminals that support truecolor (24-bit), the TUI applies a bloom composite effect to ROSEDUST glow elements. This is achieved by rendering glow elements at higher brightness and using the `gradient()` and `lighten()` functions from `roko-cli/src/tui/color.rs` to create the appearance of light bleeding from bright elements.

### Responsive Breakpoints

| Terminal Width | Layout |
|---|---|
| < 80 columns | Single column — navigation stacked above detail |
| 80-119 columns | Two columns — navigation sidebar + detail panel |
| 120+ columns | Full layout with Spectre viewport |

### Offline Mode

The TUI can operate with cached state when the roko-serve backend is unavailable. Cached state is loaded from `.roko/` files, providing a read-only view of the last known system state.

---

## Keyboard Navigation

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle focus between panels |
| `↑`/`↓` | Navigate within focused list |
| `Enter` | Expand detail / enter screen |
| `Esc` | Back to parent screen |
| `q` | Quit |
| `?` | Help overlay |
| `1`-`6` | Jump to window region |

---

## Current Status and Gaps

**Built:**
- TUI framework with ratatui (`roko-cli/src/tui/`)
- App state machine (`app.rs`)
- ROSEDUST theme (`theme.rs`, `color.rs`)
- Dashboard scaffold with text rendering (`dashboard.rs`)
- Multiple view modules (agents, plans, config, logs, signals, dashboard)
- Multiple widget modules (agent_grid, plan_tree, status_bar, header_bar, phase_bar, token_bar, braille, scrollbar, etc.)
- Modal system (help, task_detail, plan_detail)

**Not yet built:**
- Interactive TUI (currently text-only mode — `--text` flag required)
- 60fps render loop with animation ticking
- Spectre ASCII rendering
- Bloom composite effects
- Responsive breakpoint handling

---

## Cross-references

- See [09-tui-29-screens.md](./09-tui-29-screens.md) for the full 29-screen inventory
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color palette
- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for Spectre rendering
- See topic [09-daimon](../09-daimon/INDEX.md) for behavioral states and PAD vector
