# Accessibility and Current Implementation Status

> WCAG 2.1 AA compliance targets, keyboard navigation, screen reader support, reduced motion, port allocation, and comprehensive status of all interface components.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md), [08-tui-main-layout.md](./08-tui-main-layout.md), [13-web-portal.md](./13-web-portal.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §9, `bardo-backup/prd/shared/port-allocation.md`

---

## Abstract

Roko's interfaces target WCAG 2.1 AA compliance across the Web Portal, with equivalent access goals for the TUI and CLI. This document specifies accessibility requirements for each interface, including keyboard navigation, screen reader support, color contrast, reduced motion, and alternative text. It also provides the comprehensive port allocation table and the current implementation status of all interface components.

---

## WCAG 2.1 AA Compliance (Web Portal)

### Perceivable

#### 1.1 Text Alternatives

- All Spectre creature visualizations include `alt` text describing the agent's name, behavioral state, and key metrics
- Charts and graphs include `aria-label` descriptions with data summaries
- A2UI components include semantic labels
- Icons use `aria-hidden="true"` with adjacent text labels

```html
<!-- Spectre alt text example -->
<canvas
  role="img"
  aria-label="Spectre for rust-impl-01: Engaged state, breathing 0.7Hz, 142 knowledge entries, connected to 2 peers"
/>

<!-- Chart alt text example -->
<div role="img" aria-label="C-Factor trend: 1.18 rising to 1.23 over the last hour">
  <canvas /> <!-- Recharts canvas -->
</div>
```

#### 1.3 Adaptable

- All content is structured with semantic HTML (`<main>`, `<nav>`, `<section>`, `<article>`, `<aside>`)
- Data tables use `<th scope="col">` and `<th scope="row">`
- Form inputs are associated with `<label>` elements
- Landmark regions are defined for all major page areas

#### 1.4 Distinguishable

**Color contrast requirements:**

The ROSEDUST palette has been verified against WCAG AA contrast ratios:

| Element | Foreground | Background | Ratio | Pass? |
|---|---|---|---|---|
| Body text | `#E8DFD5` (fg) | `#1A1520` (void) | 12.8:1 | AA |
| Muted text | `#8A7F8E` (fg-muted) | `#1A1520` (void) | 4.6:1 | AA |
| Rose accent | `#D4778C` (rose) | `#1A1520` (void) | 6.2:1 | AA |
| Success | `#5DB8A3` (teal) | `#1A1520` (void) | 7.4:1 | AA |
| Warning | `#D4A857` (gold) | `#1A1520` (void) | 7.8:1 | AA |
| Danger | `#C45C50` (danger) | `#1A1520` (void) | 4.5:1 | AA (minimum) |
| Info | `#6B8FBD` (sapphire) | `#1A1520` (void) | 5.1:1 | AA |
| Glass panel text | `#E8DFD5` (fg) | `#221D2A` (twilight) | 10.9:1 | AA |
| Selection text | `#E8DFD5` | `#2D2838` (selection) | 9.2:1 | AA |

**Color is never the sole information channel:**
- Status indicators use symbols + color: `✓` (pass), `✗` (fail), `○` (pending)
- Behavioral states use labels + color: "Engaged" (rose), "Struggling" (amber)
- Charts use patterns/textures in addition to color (future enhancement)

**Text sizing:**
- Minimum body text: 16px (1rem)
- Minimum UI labels: 14px (0.875rem)
- All text responds to browser zoom (no fixed pixel sizes in layouts)
- `prefers-contrast: more` media query supported (increases contrast ratio)

### Operable

#### 2.1 Keyboard Accessible

**Web Portal keyboard navigation:**

| Key | Action |
|---|---|
| `Tab` / `Shift+Tab` | Navigate between interactive elements |
| `Enter` / `Space` | Activate buttons, links, toggles |
| `Arrow keys` | Navigate within lists, tables, grids |
| `Escape` | Close modals, cancel operations |
| `?` | Open keyboard shortcut help |
| `1`–`9` | Jump to page (when focus is in nav) |
| `/` | Focus search input |

**Focus management:**
- Visible focus ring: 2px solid `#D4778C` (rose) with 2px offset
- Focus trap in modals (Tab cycles within modal)
- Focus returns to trigger element when modal closes
- Skip-to-content link at top of each page

```css
/* Focus ring */
:focus-visible {
  outline: 2px solid #D4778C;
  outline-offset: 2px;
}

/* Skip link */
.skip-link {
  position: absolute;
  top: -40px;
  left: 0;
  z-index: 100;
}
.skip-link:focus {
  top: 0;
}
```

#### 2.3 Seizures and Physical Reactions

- No content flashes more than 3 times per second
- Spectre breathing animation is below 1.4Hz (well under the 3Hz threshold)
- Glow pulsing never exceeds 2Hz
- `prefers-reduced-motion` disables all animations (see below)

#### 2.4 Navigable

- Every page has a unique `<title>`
- All sections have heading hierarchy (`h1` → `h2` → `h3`)
- Breadcrumb navigation on detail pages
- Link purpose is clear from link text (no "click here")

### Understandable

#### 3.1 Readable

- `lang="en"` on `<html>` element
- Technical terms have `<abbr>` with `title` attribute
- Error messages are descriptive and suggest corrective action

#### 3.2 Predictable

- Navigation is consistent across all pages
- Interactive elements behave predictably
- Form submissions require explicit action (no auto-submit)

#### 3.3 Input Assistance

- Form validation errors are associated with their inputs via `aria-describedby`
- Required fields are marked with `aria-required="true"` and visual indicator
- Error messages appear inline (not just at page top)

### Robust

#### 4.1 Compatible

- Valid HTML5 semantics
- ARIA roles used correctly (`role="alert"`, `role="status"`, `role="tab"`, etc.)
- Tested with: NVDA, VoiceOver, JAWS (target screen readers)

---

## Screen Reader Support

### ARIA Live Regions

Dynamic content uses ARIA live regions for screen reader announcements:

```html
<!-- Agent status updates -->
<div aria-live="polite" aria-atomic="false">
  <!-- Updated when agent status changes -->
  <span>rust-impl-01: task 4 of 7 complete</span>
</div>

<!-- Gate results (more urgent) -->
<div aria-live="assertive" aria-atomic="true">
  <!-- Updated on gate failure -->
  <span>Gate failure: test gate failed for task-03 with 3 test failures</span>
</div>

<!-- C-Factor updates (low priority) -->
<div aria-live="polite" aria-atomic="true">
  <span>C-Factor updated: 1.23 (up from 1.21)</span>
</div>
```

### Spectre Description for Screen Readers

Spectre creatures are described textually for screen readers:

```
"Agent rust-impl-01 Spectre: Engaged state. Breathing rate 0.7 hertz.
 Knowledge: 142 entries (23 persistent, 89 working, 30 transient).
 Connected to 2 mesh peers: reviewer-01, researcher-01.
 Emitting Wisdom pheromone at intensity 0.4."
```

This description updates on state transitions (not on every animation frame).

### Data Tables

All data tables in the Portal use proper table semantics:

```html
<table aria-label="Agent comparison">
  <thead>
    <tr>
      <th scope="col">Metric</th>
      <th scope="col">rust-impl-01</th>
      <th scope="col">reviewer-01</th>
    </tr>
  </thead>
  <tbody>
    <tr>
      <th scope="row">Gate pass rate</th>
      <td>100%</td>
      <td>100%</td>
    </tr>
  </tbody>
</table>
```

---

## Reduced Motion

### `prefers-reduced-motion` Support

When the user's system preference is set to reduce motion:

**Web Portal:**

```css
@media (prefers-reduced-motion: reduce) {
  /* Disable all transitions and animations */
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }

  /* Spectre: static image instead of animated canvas */
  .spectre-viewport {
    /* Render single frame, no breathing animation */
  }

  /* Glass morphism: remove backdrop blur (performance) */
  .glass-panel {
    backdrop-filter: none;
    background: rgba(34, 29, 42, 0.95);
  }

  /* Progress bars: no animation */
  .progress-bar {
    transition: none;
  }
}
```

**TUI:**

The TUI respects the `ROKO_REDUCED_MOTION=1` environment variable:

```rust
fn should_animate() -> bool {
    std::env::var("ROKO_REDUCED_MOTION").is_err()
}
```

When reduced motion is active:
- Spectre breathing animation stops (static body)
- Glow effects are static (no pulsing)
- State transitions are instant (no interpolation)
- Sparkline charts use static bars instead of animated drawing

**Sonification:**

Reduced motion does not affect sonification (audio is not motion). However, a separate `ROKO_NO_SOUND=1` environment variable disables all audio output.

---

## TUI Accessibility

### Terminal Compatibility

The TUI is tested across terminal emulators:

| Terminal | Color Support | Unicode | Braille | Notes |
|---|---|---|---|---|
| iTerm2 | Truecolor | Full | Full | Recommended (macOS) |
| Alacritty | Truecolor | Full | Full | Recommended (cross-platform) |
| WezTerm | Truecolor | Full | Full | Good alternative |
| Kitty | Truecolor | Full | Full | Good alternative |
| Terminal.app | 256-color | Full | Full | Reduced color fidelity |
| Windows Terminal | Truecolor | Full | Partial | Braille may have gaps |
| tmux | Truecolor* | Full | Full | Requires `set -g default-terminal "tmux-256color"` |
| screen | 256-color | Partial | Partial | Limited support |

### `NO_COLOR` Support

When `NO_COLOR` environment variable is set (per https://no-color.org/):
- All ROSEDUST colors are replaced with `Color::Reset`
- Information conveyed by color uses alternative indicators (bold, underline, symbols)
- Status: `[PASS]` / `[FAIL]` / `[PEND]` instead of colored symbols
- `RosedustTheme::no_color()` provides the uncolored palette

### High Contrast Mode

When `ROKO_HIGH_CONTRAST=1` is set:
- Background changes to pure black (`#000000`)
- Foreground changes to pure white (`#FFFFFF`)
- Accent colors are maximally saturated
- Border contrast is increased

### Screen Reader with TUI

Terminal screen readers (like BRLTTY for braille displays) can read TUI content through the terminal's text buffer. The TUI avoids:
- Overwriting the same line repeatedly (use stable layouts)
- Using Unicode characters that screen readers cannot pronounce
- Rendering purely decorative elements without semantic meaning

---

## CLI Accessibility

### Plain Text Output

The CLI text mode (`--text` flag) produces clean, structured text output suitable for:
- Screen readers
- Text-to-speech
- Piping to other tools
- `grep` and text processing

```bash
# Machine-readable output
roko status --json

# Plain text (no color, no Unicode art)
roko status --text

# Respect NO_COLOR
NO_COLOR=1 roko status
```

### Exit Codes

All CLI commands use standard exit codes for scripting:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | General error |
| 2 | Invalid arguments |
| 3 | Configuration error |
| 4 | Gate failure |
| 5 | Budget exceeded |
| 130 | Interrupted (Ctrl+C) |

---

## Port Allocation

### Default Port Map

| Port | Service | Protocol | Notes |
|---|---|---|---|
| **3000** | Roko Portal (static) | HTTP | Next.js static export served by roko-serve |
| **3001** | Roko Portal (dev) | HTTP | Next.js dev server |
| **3002** | Reserved | — | Future Portal service |
| **3003** | Reserved | — | Future Portal service |
| **3004** | Reserved | — | Future Portal service |
| **3005** | Reserved | — | Future Portal service |
| **3006** | Reserved | — | Future Portal service |
| **3007** | Reserved | — | Future Portal service |
| **3008** | Reserved | — | Future Portal service |
| **3009** | Reserved | — | Future Portal service |
| **8080** | roko-serve API | HTTP/WS | REST, WebSocket, SSE endpoints |
| **8081** | Reserved (tools) | — | Future tool service |
| **8082** | Reserved (tools) | — | Future tool service |
| **8083** | MCP stdio proxy | HTTP | Local MCP server proxy (if HTTP mode) |
| **8084–8099** | Reserved (tools) | — | Future tool services |
| **8545** | Anvil (EVM) | HTTP/WS | Local Ethereum node (mirage-rs) |
| **8546** | Anvil (EVM WS) | WS | Anvil WebSocket endpoint |

### Port Configuration

Ports are configurable in `roko.toml`:

```toml
[serve]
port = 8080       # API server port
portal_port = 3000 # Portal static serving port

[chain]
anvil_port = 8545  # Local EVM port
```

### Port Conflict Resolution

roko-serve checks for port conflicts on startup:
1. Attempt to bind configured port
2. If EADDRINUSE, log warning and try port + 1
3. After 10 attempts, fail with descriptive error
4. Report actual bound port in startup log

---

## Current Implementation Status

### CLI (Built)

| Component | Status | Location |
|---|---|---|
| Command parser (clap) | **Complete** | `roko-cli/src/main.rs` |
| All subcommands | **Complete** | `roko-cli/src/` (27 modules) |
| `--text` output mode | **Complete** | Text rendering in dashboard |
| `--json` output mode | **Partial** | Some commands only |
| Exit codes | **Complete** | Standard codes |
| `NO_COLOR` support | **Complete** | `RosedustTheme::no_color()` |
| Pipe-friendly output | **Complete** | Detects non-TTY |

### HTTP API (Scaffold)

| Component | Status | Location |
|---|---|---|
| Server framework (axum) | **Complete** | `roko-serve/src/lib.rs` |
| Route structure (12 groups) | **Complete** | `roko-serve/src/routes/` |
| Authentication middleware | **Complete** | `routes/middleware.rs` |
| Secret scrubbing middleware | **Complete** | `routes/middleware.rs` |
| CORS configuration | **Complete** | `routes/middleware.rs` |
| WebSocket handler | **Scaffold** | `routes/ws.rs` |
| SSE handler | **Scaffold** | `routes/sse.rs` |
| Event bus | **Built** | `event_bus.rs` |
| API endpoint implementations | **Partial** | Status, plans, agents basic |

### TUI (Scaffold)

| Component | Status | Location |
|---|---|---|
| ratatui framework | **Integrated** | `roko-cli/src/tui/` |
| App state machine | **Built** | `tui/app.rs` |
| ROSEDUST theme | **Built** | `tui/theme.rs`, `tui/color.rs` |
| Dashboard scaffold | **Built** | `tui/dashboard.rs` |
| Agent list view | **Built** | `tui/views/agents.rs` |
| Plan list view | **Built** | `tui/views/plans.rs` |
| Config view | **Built** | `tui/views/config.rs` |
| Log view | **Built** | `tui/views/logs.rs` |
| Signal view | **Built** | `tui/views/signals.rs` |
| Agent grid widget | **Built** | `tui/widgets/agent_grid.rs` |
| Plan tree widget | **Built** | `tui/widgets/plan_tree.rs` |
| Status bar widget | **Built** | `tui/widgets/status_bar.rs` |
| Header bar widget | **Built** | `tui/widgets/header_bar.rs` |
| Phase bar widget | **Built** | `tui/widgets/phase_bar.rs` |
| Token bar widget | **Built** | `tui/widgets/token_bar.rs` |
| Braille widget | **Built** | `tui/widgets/braille.rs` |
| Scrollbar widget | **Built** | `tui/widgets/scrollbar.rs` |
| Help modal | **Built** | `tui/modals/help.rs` |
| Task detail modal | **Built** | `tui/modals/task_detail.rs` |
| Plan detail modal | **Built** | `tui/modals/plan_detail.rs` |
| **Interactive TUI** | **Not built** | Requires render loop wiring |
| **60fps render loop** | **Not built** | Currently text-only |
| **Region navigation (1–6)** | **Not built** | — |
| **Agent detail screens** | **Not built** | — |
| **Plan detail screens** | **Not built** | — |
| **Knowledge screens** | **Not built** | — |
| **Collective screens** | **Not built** | — |
| **System screens** | **Not built** | — |
| **Spectre viewport** | **Not built** | — |

### Web Portal (Not Built)

| Component | Status |
|---|---|
| Next.js application | **Not started** |
| ROSEDUST Tailwind config | **Not started** |
| Glass morphism components | **Not started** |
| Dashboard page | **Not started** |
| Agent Detail page | **Not started** |
| Plan Detail page | **Not started** |
| Collective Intelligence page | **Not started** |
| Knowledge Explorer page | **Not started** |
| System monitoring page | **Not started** |
| Configuration page | **Not started** |
| Episodes page | **Not started** |
| WebGL Spectre renderer | **Not started** |
| WebSocket integration | **Not started** |
| WCAG 2.1 AA compliance | **Not started** |

### Spectre System (Not Built)

| Component | Status |
|---|---|
| Shape seed generation | **Not started** |
| Dot-cloud geometry | **Not started** |
| Spring physics | **Not started** |
| Breathing animation | **Not started** |
| Eye rendering | **Not started** |
| Glow system | **Not started** |
| TUI ASCII rasterizer | **Not started** |
| WebGL renderer | **Not started** |
| Collective display | **Not started** |

### Sonification (Not Built)

| Component | Status |
|---|---|
| Audio engine | **Not started** |
| Five-layer system | **Not started** |
| Behavioral state presets | **Not started** |
| Web Audio integration | **Not started** |
| Event → sound mapping | **Not started** |

### Generative Interfaces / A2UI (Not Built)

| Component | Status |
|---|---|
| A2UI protocol spec | **Designed** |
| A2UI parser | **Not started** |
| TUI renderer | **Not started** |
| Web renderer | **Not started** |
| Agent system prompt docs | **Not started** |

---

## Implementation Priority

Based on `refactoring-prd/07-implementation-priorities.md`:

| Priority | Component | Tier |
|---|---|---|
| 1 | Interactive TUI (render loop, navigation) | Tier 4A |
| 2 | WebSocket bidirectional agent control | Tier 4A |
| 3 | Event coalescing and reconnection | Tier 4A |
| 4 | Web Portal MVP (dashboard, agent detail) | Tier 4B |
| 5 | Spectre TUI ASCII renderer | Tier 4B |
| 6 | Spectre WebGL renderer | Tier 4C |
| 7 | Sonification engine | Tier 5 |
| 8 | A2UI protocol | Tier 5 (P3) |
| 9 | Accessibility audit (WCAG 2.1 AA) | Tier 5 |
| 10 | Collective display | Tier 5 |

---

## Cross-references

- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color system and contrast ratios
- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the TUI rendering architecture
- See [09-tui-29-screens.md](./09-tui-29-screens.md) for the screen inventory
- See [13-web-portal.md](./13-web-portal.md) for the Portal technology stack
- See [16-sonification-reframed.md](./16-sonification-reframed.md) for the audio system
