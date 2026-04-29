# Generative Interfaces and A2UI

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers A2UI as an Extension that emits Pulses in a UI-component vocabulary, the 12 component types as Pulse kinds, renderer Cells that translate to surface-specific output, sonification as a React Cell, and rich UX primitives as standardized Pulse-to-Lens compositions.

---

## 1. A2UI as an Extension Emitting Pulses

Generative Interfaces are a system where agents create their own UI components during execution. Instead of all UI being pre-designed, agents emit structured UI descriptions via the **A2UI (Agent-to-UI) protocol**. In unified vocabulary, A2UI is an **Extension** (see [12-EXTENSIONS.md](../../unified/12-EXTENSIONS.md)) that intercepts agent output and emits Pulses on a `ui.*` Bus topic.

The agent never generates raw HTML, CSS, or terminal escape codes. It describes *what* to show. Renderer Cells subscribe to the Bus and decide *how* to show it, using ROSEDUST design tokens for visual consistency.

```
Agent output stream
    |
    v
A2UI parser (Extension) -- extracts {"a2ui": ...} lines
    |
    v
Validate against component schema
    |
    v
Publish as Pulse on Bus topic "ui.component"
    |
    v
Renderer Cells subscribe:
    |-- TUI: ratatui widget conversion
    |-- Web: React/Svelte component instantiation
    |-- CLI: text-mode formatting
    |-- API: JSON passthrough
```

---

## 2. The 12 Component Types as Pulse Kinds

Each component type is a specific `Kind` on the `ui.*` Bus topic:

| Component | What It Renders | TUI | Web | CLI |
|---|---|---|---|---|
| `table` | Data table with headers and rows | Unicode table | HTML `<table>` with glass panel | Aligned columns |
| `progress` | Progress bar with label | Block characters + percentage | Animated bar with gradient | Text percentage |
| `chart` | Bar, line, pie, scatter | Braille/ASCII chart | Recharts/Nivo component | Sparkline text |
| `status` | Status indicator list | Checkmark/X/circle symbols | Colored badges | Text symbols |
| `code` | Syntax-highlighted code block | Colored text (truecolor) | Prism.js highlighting | Plain text |
| `callout` | Alert/notice box | Bordered text with icon | Glass panel with icon | Indented text |
| `tree` | Hierarchical data | ASCII tree lines | Collapsible tree | Indented list |
| `kv` | Key-value pairs | Aligned columns | Definition list | Colon-separated |
| `diagram` | Simple diagrams | ASCII box-and-arrow | SVG rendering | Text fallback |
| `form` | Input form (agent interaction) | Not supported | HTML form | Not supported |
| `markdown` | Rich text | Terminal rendering | HTML rendering | Plain text |
| `image` | Image reference | Not supported | `<img>` tag | URL text |

### JSONL Emission

Agents emit A2UI as JSONL within their output stream:

```jsonl
{"a2ui": "table", "title": "Dependencies", "columns": ["Name", "Version", "License"], "rows": [["tokio", "1.38", "MIT"], ["serde", "1.0", "MIT/Apache"]], "highlight_row": 0}
{"a2ui": "progress", "label": "Migration", "value": 0.67, "max": 1.0, "style": "success"}
{"a2ui": "callout", "level": "warning", "title": "Breaking Change", "content": "This changes the public API."}
```

### ROSEDUST Inheritance

All A2UI components automatically inherit ROSEDUST design tokens:

| Semantic Name | ROSEDUST Color | Usage |
|---|---|---|
| `primary` | rose | Default accent |
| `success` | jade | Pass states |
| `warning` | amber | Threshold alerts |
| `danger` | crimson | Failures |
| `info` | sapphire | Informational |
| `muted` | ghost | De-emphasized |
| `accent` | coral | Secondary accent |
| `highlight` | violet | Knowledge-related |

On the web, components are wrapped in glass morphism panels:
```css
.a2ui-component {
    background: rgba(34, 29, 42, 0.72);
    backdrop-filter: blur(16px);
    border: 1px solid rgba(212, 119, 140, 0.08);
    border-radius: 12px;
}
```

---

## 3. Incremental Updates via Bus

Following RFC 6902 JSON Patch semantics, A2UI supports streaming incremental updates:

```jsonl
{"a2ui": "progress", "id": "build-progress", "label": "Building", "value": 0.0}
{"a2ui_patch": "build-progress", "op": "replace", "path": "/value", "value": 0.50}
{"a2ui_patch": "build-progress", "op": "replace", "path": "/value", "value": 1.0}
{"a2ui_patch": "build-progress", "op": "replace", "path": "/label", "value": "Complete"}
```

This reduces bandwidth for frequently-updating components from O(component_size) to O(patch_size) per update. The `id` field enables patching; components without `id` are append-only.

---

## 4. Sandboxing

A2UI components are sandboxed:

| Constraint | What It Prevents |
|---|---|
| Viewport containment | Cannot modify TUI chrome, overlay other agents, trigger navigation |
| Schema validation | Unknown types render as raw JSON; invalid fields silently ignored |
| Resource limits | Max 10 components/turn, 50 table rows, 100 chart points, 200 code lines |
| Content sanitization | HTML-escaped on web (prevents XSS); no executable code |

This is the CaMeL IFC principle applied to UI generation: the agent can compose from a trusted catalog of component types, but cannot escape the catalog or inject arbitrary rendering.

---

## 5. Sonification as a React Cell

Sound effects map behavioral state Pulses to ambient audio parameters. This is a **React Cell** (see [02-CELL.md](../../unified/02-CELL.md)) that watches Pulses on the Bus and emits audio events:

| Trigger Pulse | Sound | Parameters |
|---|---|---|
| Cable plug-in (Canvas) | Perfect fifth | 60ms triangle wave |
| Cable mismatch | Minor second | 80ms |
| Verification pass | Short major chord arpeggio | 120ms |
| Verification fail | Low resonant tone | 200ms |
| Flow start | Octave | 120ms with reverb tail |
| Flow complete | High sine ping | 60ms |

The principle: **juice user actions and agent completions, never ambient state**. Sound is off by default and respects accessibility preferences.

For spatial computing (WebXR), each Spectre emits 3D-positioned audio via Web Audio HRTF panning, so agent events are spatialized to their location in the collective display.

---

## 6. Rich UX Primitives

Beyond the 12 base A2UI component types, the system defines standardized Pulse-to-Lens compositions for common UX patterns:

### Reasoning Stream

A live feed of the agent's reasoning process, rendered as a collapsible "thinking" section:

- **Pulse source**: `AgentThoughtChunk` from the inference pipeline
- **Lens composition**: `agent_trails` projection -> filter by reasoning markers -> render as expandable text
- **TUI**: Dimmed text above the agent's response, collapsible
- **Web**: Expandable drawer with syntax highlighting

### Tool Banners

Visual indicators when agents invoke tools:

- **Pulse source**: `ToolCallStart`, `ToolCallProgress`, `ToolCallResult`
- **Lens composition**: `agent_trails` projection -> filter by tool events -> render as banner
- **TUI**: Single-line banner with tool name, arguments, spinner
- **Web**: Glass panel with progress indicator and result preview

### Gate Badges

Inline verification status:

- **Pulse source**: Gate result Pulses from the Verify pipeline
- **Lens composition**: `gate_pipeline` projection -> render as badges
- **TUI**: `checkmark compile  checkmark test  circle review` inline
- **Web**: Colored badge row with expandable details

### Uncertainty Bars

Visual representation of agent confidence:

- **Pulse source**: Score protocol calibration data
- **Lens composition**: `active_tasks` projection -> confidence field -> render as bar
- **TUI**: Gradient bar (crimson -> amber -> jade)
- **Web**: Animated bar with tooltip showing calibration details

### Heuristic Footnotes

When the agent relies on a learned heuristic, a footnote links to the source:

- **Pulse source**: Heuristic match events from the Compose pipeline
- **Lens composition**: `heuristic_library` projection -> matched heuristic -> render as footnote
- **TUI**: Dim inline reference with `roko inspect` link
- **Web**: Hover tooltip with heuristic text, calibration score, and challenge history

---

## 7. A2UI Components as Signals

A2UI components emitted by agents are graduated from Pulse to Signal (stored as Signals with `kind: UiComponent`). This enables:

- **Replay**: Past agent output renders with A2UI components intact
- **Lineage**: Tracing which agent produced which UI element
- **Learning**: Analyzing which A2UI components correlate with task success (components that appear in successful episodes are reinforced)

---

## What This Enables

- **Agents that present, not just produce**: Research results appear as formatted tables, not flat text. Debugging output appears as hypothesis trees. Architecture proposals appear as trade-off matrices.
- **Multi-renderer consistency**: The same A2UI payload renders in TUI, web, CLI, and custom surfaces -- always with ROSEDUST styling.
- **Safe generative UI**: Agents compose from a trusted component catalog. No arbitrary HTML, no XSS, no viewport escape.
- **Progressive richness**: A surface that does not understand A2UI simply renders agent output as text. A2UI enhances output without replacing the core loop.

---

## Feedback Loops

- **Component usage -> learning**: Which A2UI components appear in successful episodes? The system can learn that `table` + `callout` correlates with higher task completion rates for research tasks, and encourage their use in future prompts.
- **Schema evolution -> catalog growth**: New component types can be added to the catalog without changing the protocol. Agents that propose unknown types produce `RawJson` fallback, which can be analyzed to identify useful new component patterns.
- **Accessibility -> automatic**: Every component type has built-in ARIA mapping. Tables get `role="grid"`, progress bars get `role="progressbar"` with `aria-valuenow`. Accessibility is structural, not opt-in.

---

## Open Questions

1. **Interactive A2UI**: The `form` component type allows agents to solicit structured input. How does this interact with the autonomy slider? Should form submissions be gated by the same approval flow as other agent actions?
2. **Component composition**: Can agents compose components (a table inside a callout inside a tree)? The current JSONL format is flat. Nested composition would require either recursive types or explicit ID references.
3. **Streaming rendering**: For long-running agent turns, should A2UI components render incrementally as they arrive, or batch until the turn completes?

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Define A2UI JSON schemas | `crates/roko-core/src/a2ui/` | New module with schema validation for all 12 component types |
| Build A2UI parser Extension | `crates/roko-agent/src/` | Extract `{"a2ui": ...}` lines from agent output, validate, publish as Pulses |
| Implement TUI A2UI renderer | `crates/roko-cli/src/tui/widgets/` | Convert A2UI Pulses to ratatui widgets with ROSEDUST styling |
| Implement CLI text-mode renderer | `crates/roko-cli/src/` | Convert A2UI Pulses to formatted text output |
| Add A2UI documentation to system prompt | `crates/roko-compose/src/templates/` | Include component type reference in agent system prompts |
| Graduate A2UI Pulses to Signals | `crates/roko-core/src/` | Store A2UI components as Signals with `kind: UiComponent` for replay |
| Implement incremental patch protocol | `crates/roko-core/src/a2ui/` | RFC 6902 JSON Patch for components with `id` field |
