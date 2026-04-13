# TUI Main Layout

> The terminal dashboard layout: agent list, plan list, mesh, health panels, agent detail, gate results, Daimon state, Neuro tiers, predictions, and Spectre viewport.


> **Implementation**: Scaffold

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

## Dashboard Design Principles (Tufte + Few)

The TUI dashboard follows the quantitative visualization principles of Edward Tufte (*The Visual Display of Quantitative Information*, 1983) and Stephen Few (*Information Dashboard Design*, 2006).

### Tufte's Data-Ink Ratio

**Principle:** `data-ink ratio = data-ink / total ink used`. Maximize this ratio — every terminal cell should convey data or essential structure, never decoration.

Applied to the TUI:
- **No chartjunk:** No decorative borders around individual values, no redundant axis labels, no shadows
- **Sparklines beside every metric:** Tufte's word-sized graphics — a number is meaningless without its history
- **Direct annotation:** Label data points inline rather than using a separate legend that requires eye-bouncing

```
Cost: $2.34 ▁▂▃▅▆▇█▆▅ ↑23%     ← sparkline + trend inline
C-Factor: 1.23 ▃▃▄▅▅▆▆▇ ↑0.04  ← history visible at a glance
Gate: 94% ████████▓░ 47/50      ← ratio bar with fraction
```

### Few's Three Dashboard Zones

The layout maps to Few's functional zones:

| Zone | Position | Content | Read Time |
|---|---|---|---|
| **Monitoring** | Header bar + Health panel | C-Factor, cost, alerts | 200ms glance |
| **Analysis** | Sidebar panels + Detail panel | Agent list, plan status, gate results | 30s investigation |
| **Detail** | Expanded screens (Enter to access) | Full agent output, episode history | Minutes of study |

**Critical rule:** Monitoring information never scrolls. The header bar and health summary are always visible regardless of terminal height.

### Bullet Graphs (Few's Gauge Replacement)

Agent progress and system metrics use Few's bullet graph rather than pie charts or circular gauges:

```
CPU  [██████████░░░░░░░░░] 48%  ┃75%     ← bar=actual, ┃=target
MEM  [████████████████░░░] 82%  ┃90%
COST [████░░░░░░░░░░░░░░░] $2.34/$50     ← budget fraction
```

**Components:** thick bar (featured measure), thin `┃` line (comparative target), background shading (qualitative ranges: ░ good, ▒ satisfactory, ▓ poor).

### Small Multiples for Agent Comparison

When multiple agents are active, the sidebar uses Tufte's small multiples — identical mini-displays repeated for each agent:

```
◉ rust-impl    ▃▅▇▆▅ Engaged  3/7  $0.34
◉ reviewer     ▅▇███ Focused  1/3  $0.12
○ researcher   ▁▁▂▁▁ Resting  0/0  $0.00
```

Same scale, same encoding, same visual weight — enables instant cross-agent comparison.

### Micro/Macro Readings

Every display element supports two reading levels:
- **Macro (200ms):** The overall shape/color/trend. A red sparkline means trouble; an ascending green one means progress.
- **Micro (deliberate):** The exact number, the specific gate that failed, the precise cost.

Both are always visible simultaneously at different visual weights (macro = color/shape, micro = text values).

---

## Ratatui Advanced Architecture

### The Elm Architecture (TEA) Pattern

The TUI uses the Elm Architecture for clean separation of model, update, and view:

```rust
/// Central application model — all TUI state lives here.
pub struct Model {
    pub screen: Screen,
    pub mode: InputMode,
    pub agents: Vec<AgentState>,
    pub plans: Vec<PlanState>,
    pub focus: FocusRing,
    pub modal: Option<Modal>,
    pub history: Vec<Screen>,       // for back-navigation
    pub animations: AnimationState, // dt-based ticking
}

/// Messages that drive state transitions.
#[derive(Debug, Clone)]
pub enum Message {
    // Navigation
    NavigateTo(Screen),
    NavigateBack,
    FocusNext,
    FocusPrev,
    // Data
    AgentUpdate(AgentEvent),
    GateResult(GateEvent),
    CFactorUpdate(f64),
    // Modal
    OpenModal(Modal),
    CloseModal,
    // System
    Tick(Duration),
    Quit,
}

/// Pure update function — returns optional chained message.
fn update(model: &mut Model, msg: Message) -> Option<Message> {
    match msg {
        Message::NavigateTo(screen) => {
            let prev = std::mem::replace(&mut model.screen, screen);
            model.history.push(prev);
            None
        }
        Message::NavigateBack => {
            if let Some(prev) = model.history.pop() {
                model.screen = prev;
            }
            None
        }
        Message::Tick(dt) => {
            model.animations.tick(dt);
            None
        }
        // ... other message handlers
        _ => None,
    }
}

/// Pure view — no side effects, just rendering.
fn view(model: &Model, frame: &mut Frame) {
    let theme = active_theme();
    let [header, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ]).areas(frame.area());

    render_header(model, frame, header, &theme);
    render_body(model, frame, body, &theme);
    render_status_bar(model, frame, status, &theme);

    // Modal overlay (renders on top, captures input)
    if let Some(modal) = &model.modal {
        render_modal(modal, frame, &theme);
    }
}
```

### Async Event Loop with Tokio + Crossterm

```rust
use crossterm::event::EventStream;
use futures::StreamExt;

pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut tick = tokio::time::interval(tick_rate);
            loop {
                tokio::select! {
                    maybe_event = reader.next() => {
                        if let Some(Ok(event)) = maybe_event {
                            let _ = tx.send(AppEvent::Terminal(event));
                        }
                    }
                    _ = tick.tick() => {
                        let _ = tx.send(AppEvent::Tick);
                    }
                }
            }
        });
        Self { rx }
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}
```

### Constraint-Based Layout (ratatui 0.30+)

The layout uses ratatui's `Flex` modes for precise spatial control:

```rust
// Main body: sidebar + detail with responsive breakpoints
let body_layout = if area.width >= 120 {
    // Full layout: sidebar + detail + spectre
    Layout::vertical([
        Constraint::Fill(1),      // upper panels
        Constraint::Length(12),   // spectre viewport
    ]).areas(area)
} else if area.width >= 80 {
    // Two-column: sidebar + detail only
    Layout::horizontal([
        Constraint::Min(20),
        Constraint::Fill(1),
    ]).areas(area)
} else {
    // Single column: stacked
    (area, Rect::ZERO)
};

// Sidebar panels with Flex::SpaceBetween for even distribution
let sidebar_panels = Layout::vertical([
    Constraint::Ratio(1, 4),  // Agents
    Constraint::Ratio(1, 4),  // Plans
    Constraint::Ratio(1, 4),  // Mesh
    Constraint::Ratio(1, 4),  // Health
])
.flex(Flex::SpaceBetween)
.areas(sidebar_area);
```

### Double-Buffered Differential Flush

ratatui maintains two `Buffer` instances internally. After `terminal.draw()`, only changed cells are flushed to the terminal backend. This means:

1. Static elements (borders, labels) are drawn once, then never re-transmitted
2. Only animated elements (sparklines, breathing, glow) cause terminal writes
3. Typical flush for idle dashboard: < 50 cells changed per frame (< 1KB)

### Focus Ring Management

```rust
/// Manages Tab/Shift+Tab focus cycling across panels.
pub struct FocusRing {
    panels: Vec<PanelId>,
    current: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PanelId {
    Agents,
    Plans,
    Mesh,
    Health,
    Detail,
    Spectre,
}

impl FocusRing {
    pub fn next(&mut self) { self.current = (self.current + 1) % self.panels.len(); }
    pub fn prev(&mut self) {
        self.current = if self.current == 0 { self.panels.len() - 1 } else { self.current - 1 };
    }
    pub fn focused(&self) -> PanelId { self.panels[self.current] }
    pub fn is_focused(&self, panel: PanelId) -> bool { self.focused() == panel }
}
```

### Data Visualization Markers

Terminal data density using ratatui marker types:

| Marker | Resolution/Cell | Use Case |
|---|---|---|
| `Braille` | 2×4 (8 sub-pixels) | Sparklines, Spectre viewport, phase portraits |
| `HalfBlock` | 1×2 (fg + bg color) | Heat maps, gradient bars (2-color per cell) |
| `Quadrant` | 2×2 (4 sub-pixels) | Dense charts, good font support |
| `Block` | 1×1 | Bar charts, simple fills |

**Sparkline with Unicode blocks** (8 height levels):

```rust
const SPARK_CHARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn render_sparkline(data: &[f64], area: Rect, buf: &mut Buffer, color: Color) {
    let (min, max) = data.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = (max - min).max(1e-10);
    for (i, &val) in data.iter().rev().take(area.width as usize).enumerate() {
        let idx = (((val - min) / range) * 8.0).round() as usize;
        let ch = SPARK_CHARS[idx.min(8)];
        buf.get_mut(area.right() - 1 - i as u16, area.y)
            .set_char(ch)
            .set_fg(color);
    }
}
```

---

## Keyboard Navigation

### Primary Bindings

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Cycle focus between panels |
| `↑`/`↓` or `j`/`k` | Navigate within focused list |
| `Enter` | Expand detail / enter screen |
| `Esc` | Back to parent screen |
| `q` | Quit |
| `?` | Help overlay |
| `1`-`6` | Jump to window region |
| `/` | Open command palette |
| `Ctrl+P` | Fuzzy finder for agents/plans |

### Vim-Style Modal Input

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputMode {
    #[default]
    Normal,     // Navigation — j/k/Enter/Esc
    Command,    // : prefix — type command, Enter to execute
    Filter,     // / prefix — fuzzy filter active list
}
```

The current mode is always shown in the status bar (left-aligned). Context-sensitive key hints display in the status bar right side:

```
NORMAL  [j/k] navigate  [Enter] select  [?] help  [/] filter  [q] quit
FILTER  [↑/↓] results   [Enter] select  [Esc] cancel          filter: _
```

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
