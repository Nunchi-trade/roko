# Generative Interfaces (A2UI)

> Agents create their own UI components — the A2UI (Agent-to-UI) protocol enables cognitive agents to emit structured UI descriptions that render in ROSEDUST across any interface.

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md), [05-http-api-roko-serve.md](./05-http-api-roko-serve.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §7, `refactoring-prd/09-innovations.md` §13

---

## Abstract

Generative Interfaces are a Roko innovation (Innovation #13) where cognitive agents create their own UI components during execution. Instead of all UI being pre-designed, agents can emit structured UI descriptions via the A2UI (Agent-to-UI) protocol. These descriptions are rendered by the TUI, Web Portal, or CLI using the ROSEDUST design language, ensuring visual consistency regardless of what the agent generates.

The A2UI protocol is inspired by Google's Agent-to-UI research, adapted for Roko's architecture. Agents emit JSONL (JSON Lines) descriptions of UI elements — tables, charts, status indicators, progress bars, forms — that are rendered by the host interface. The agent never generates raw HTML, CSS, or terminal escape codes; it describes *what* to show, and the renderer decides *how* to show it.

This is a **Priority 3 (P3)** feature — designed but not yet implemented.

---

## Motivation

### Why Agents Need to Create UI

Pre-designed dashboards work for known data shapes — agent status, gate results, C-Factor metrics. But cognitive agents encounter novel situations that require novel presentation:

1. **Research results**: An agent researching a topic produces structured findings that benefit from custom tables, comparison matrices, and annotated code blocks
2. **Architectural proposals**: An agent designing a system produces diagrams, dependency graphs, and trade-off matrices
3. **Debugging workflows**: An agent debugging an issue produces hypothesis trees, evidence tables, and reproduction steps
4. **Domain-specific views**: A financial agent produces charts, a security agent produces vulnerability matrices, a data agent produces schema diagrams

Without A2UI, this information is rendered as flat text output. With A2UI, agents can create structured, interactive UI components that make their output dramatically more useful.

### Design Constraints

1. **Agents never emit raw rendering code**: No HTML, CSS, ANSI codes, or ratatui widgets. Only semantic descriptions.
2. **ROSEDUST consistency**: All generated UI inherits the ROSEDUST palette and design rules automatically.
3. **Multi-renderer**: The same A2UI description renders in TUI (Unicode), Web (HTML/CSS), and CLI (text).
4. **Sandboxed**: A2UI output cannot escape its viewport, access other agents' data, or modify system UI.
5. **Optional**: Agents work fine without A2UI. It enhances output, not replaces the core loop.

---

## A2UI Protocol

### JSONL Emission

Agents emit A2UI descriptions as JSONL within their output stream. Each line is a self-contained UI component:

```jsonl
{"a2ui": "table", "title": "Dependency Comparison", "columns": ["Name", "Version", "License", "Size"], "rows": [["tokio", "1.38", "MIT", "2.3MB"], ["async-std", "1.12", "MIT/Apache", "1.8MB"]], "highlight_row": 0}
{"a2ui": "progress", "label": "Migration progress", "value": 0.67, "max": 1.0, "style": "success"}
{"a2ui": "chart", "type": "bar", "title": "Test Coverage by Module", "data": [{"label": "auth", "value": 94}, {"label": "api", "value": 87}, {"label": "db", "value": 72}]}
{"a2ui": "status", "items": [{"label": "Compile", "state": "pass"}, {"label": "Test", "state": "pass"}, {"label": "Lint", "state": "fail", "detail": "3 warnings"}]}
{"a2ui": "code", "language": "rust", "title": "Proposed Implementation", "content": "pub fn validate_token(token: &str) -> Result<Claims> {\n    // ...\n}"}
{"a2ui": "callout", "level": "warning", "title": "Breaking Change", "content": "This migration changes the public API surface. All downstream consumers need updating."}
```

### Component Types

| Component | Description | TUI Rendering | Web Rendering |
|---|---|---|---|
| `table` | Data table with headers and rows | Unicode table (`─│┌┐└┘`) | HTML `<table>` with glass panel |
| `progress` | Progress bar with label | `████░░░░` with percentage | Animated bar with gradient |
| `chart` | Data visualization (bar, line, pie) | Braille/ASCII chart | Recharts/Nivo component |
| `status` | Status indicator list | `✓`/`✗`/`○` symbols | Colored badges |
| `code` | Syntax-highlighted code block | Colored text (if truecolor) | Prism.js highlighting |
| `callout` | Alert/notice box | Bordered text with icon | Glass panel with icon |
| `tree` | Hierarchical data | ASCII tree (`├──`, `└──`) | Collapsible tree component |
| `kv` | Key-value pairs | Aligned columns | Definition list |
| `diagram` | Simple diagrams | ASCII box-and-arrow | SVG rendering |
| `form` | Input form (for agent interaction) | Not supported (TUI) | HTML form |
| `markdown` | Rich text | Rendered in terminal | Rendered HTML |
| `image` | Image reference (URL or base64) | Not supported (TUI) | `<img>` tag |

### Component Schema

Each component type has a defined JSON schema. Example for `table`:

```json
{
  "a2ui": "table",
  "title": "string (optional)",
  "columns": ["string"],
  "rows": [["string"]],
  "highlight_row": "number (optional)",
  "highlight_col": "number (optional)",
  "sortable": "boolean (optional, default false)",
  "max_rows": "number (optional, default 50)"
}
```

Example for `chart`:

```json
{
  "a2ui": "chart",
  "type": "bar | line | pie | scatter",
  "title": "string (optional)",
  "data": [{"label": "string", "value": "number"}],
  "x_label": "string (optional)",
  "y_label": "string (optional)",
  "color": "string (optional, ROSEDUST color name)"
}
```

---

## ROSEDUST Inheritance

All A2UI components automatically inherit the ROSEDUST design language:

### Color Mapping

A2UI components use semantic color names that resolve to ROSEDUST values:

| Semantic Name | ROSEDUST Color | Hex |
|---|---|---|
| `primary` | rose | `#D4778C` |
| `success` | teal | `#5DB8A3` |
| `warning` | gold | `#D4A857` |
| `danger` | danger red | `#C45C50` |
| `info` | sapphire | `#6B8FBD` |
| `muted` | fg-muted | `#8A7F8E` |
| `accent` | coral | `#C47A5C` |
| `highlight` | lavender | `#A08CC4` |

Agents can specify colors by semantic name:
```json
{"a2ui": "progress", "label": "Build", "value": 0.8, "style": "success"}
```

Or by state-based coloring:
```json
{"a2ui": "status", "items": [{"label": "Gate", "state": "pass"}]}
// "pass" → success/teal, "fail" → danger, "pending" → muted
```

### Glass Morphism

In the Web Portal, A2UI components are automatically wrapped in glass morphism panels:

```css
.a2ui-component {
  background: rgba(34, 29, 42, 0.72);
  backdrop-filter: blur(16px);
  border: 1px solid rgba(212, 119, 140, 0.08);
  border-radius: 12px;
  padding: 16px;
  box-shadow: 0 0 20px rgba(212, 119, 140, 0.15);
}
```

### Typography

A2UI text inherits ROSEDUST typography:
- **Titles**: Bold, fg color (`#E8DFD5`)
- **Body**: Regular, fg color
- **Labels**: Muted (`#8A7F8E`)
- **Values**: Bold, semantic color
- **Code**: Monospace (JetBrains Mono / system mono)

---

## Rendering Pipeline

### Agent → A2UI → Renderer

```
Agent output stream
    │
    ▼
A2UI parser (extracts {"a2ui": ...} lines)
    │
    ▼
Validate against component schema
    │
    ▼
Route to active renderer:
    ├── TUI: ratatui Widget conversion
    ├── Web: React component instantiation
    └── CLI: text-mode formatting
    │
    ▼
Render in agent output viewport
    (sandboxed to agent's output area)
```

### TUI Rendering

A2UI components are converted to ratatui widgets:

```rust
fn render_a2ui(component: &A2uiComponent, area: Rect, buf: &mut Buffer, theme: &RosedustTheme) {
    match component {
        A2uiComponent::Table { title, columns, rows, .. } => {
            // Render as ratatui Table widget with ROSEDUST styling
            let header = Row::new(columns.iter().map(|c| Cell::from(c.as_str())))
                .style(theme.accent_bold());
            let table = Table::new(
                rows.iter().map(|r| Row::new(r.iter().map(|c| Cell::from(c.as_str())))),
                columns.iter().map(|_| Constraint::Percentage(100 / columns.len() as u16)),
            )
            .header(header)
            .block(Block::default().title(title.as_deref().unwrap_or("")).borders(Borders::ALL)
                .border_style(theme.border_style()));
            table.render(area, buf);
        }
        A2uiComponent::Progress { label, value, style, .. } => {
            let color = semantic_color(theme, style);
            let gauge = Gauge::default()
                .label(label.as_str())
                .ratio(*value as f64)
                .gauge_style(Style::default().fg(color));
            gauge.render(area, buf);
        }
        A2uiComponent::Chart { chart_type, title, data, .. } => {
            // Render as braille sparkline or bar chart
            render_chart(chart_type, data, area, buf, theme);
        }
        // ... other components
    }
}
```

### Web Rendering

A2UI components are instantiated as React components:

```tsx
function A2UIRenderer({ component }: { component: A2UIComponent }) {
  switch (component.a2ui) {
    case 'table':
      return (
        <div className="glass-panel">
          {component.title && <h3 className="text-rosedust-fg font-bold">{component.title}</h3>}
          <table className="w-full">
            <thead>
              <tr>{component.columns.map(col => <th key={col}>{col}</th>)}</tr>
            </thead>
            <tbody>
              {component.rows.map((row, i) => (
                <tr key={i} className={i === component.highlight_row ? 'bg-rosedust-twilight' : ''}>
                  {row.map((cell, j) => <td key={j}>{cell}</td>)}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case 'chart':
      return <A2UIChart data={component.data} type={component.type} />;
    case 'progress':
      return <A2UIProgress {...component} />;
    // ... other components
  }
}
```

### CLI Rendering

A2UI components are rendered as formatted text:

```
┌─ Dependency Comparison ─────────────────────┐
│ Name       │ Version │ License     │ Size   │
│────────────┼─────────┼─────────────┼────────│
│ tokio      │ 1.38    │ MIT         │ 2.3MB  │  ← highlighted
│ async-std  │ 1.12    │ MIT/Apache  │ 1.8MB  │
└─────────────────────────────────────────────┘
```

---

## Sandboxing

### Viewport Containment

A2UI components render only within the agent's output viewport. They cannot:
- Modify the TUI chrome (header, sidebar, status bar)
- Overlay other agents' output
- Trigger navigation or state changes
- Access DOM elements outside their container (Web)
- Execute arbitrary code

### Schema Validation

All A2UI input is validated against the component schema before rendering:
- Unknown component types are rendered as raw JSON (code block)
- Invalid fields are silently ignored
- Excessive data is truncated (e.g., `max_rows: 50` for tables)
- String content is HTML-escaped (Web) to prevent XSS

### Resource Limits

| Limit | Value | Reason |
|---|---|---|
| Max components per turn | 10 | Prevent output flooding |
| Max table rows | 50 | Memory and rendering budget |
| Max chart data points | 100 | Rendering performance |
| Max code block lines | 200 | Viewport space |
| Max callout content | 1000 chars | Viewport space |

---

## Agent Authoring

### How Agents Emit A2UI

Agents emit A2UI by including JSON objects with the `"a2ui"` key in their output stream. The system prompt includes A2UI documentation:

```
When you want to present structured data, emit A2UI components as JSON on a single line:

Table: {"a2ui": "table", "title": "...", "columns": [...], "rows": [[...]]}
Chart: {"a2ui": "chart", "type": "bar", "data": [{"label": "...", "value": N}]}
Progress: {"a2ui": "progress", "label": "...", "value": 0.0-1.0}
Status: {"a2ui": "status", "items": [{"label": "...", "state": "pass|fail|pending"}]}
Code: {"a2ui": "code", "language": "...", "content": "..."}
Callout: {"a2ui": "callout", "level": "info|warning|danger", "title": "...", "content": "..."}

The UI renderer will display these as formatted components.
Available colors: primary, success, warning, danger, info, muted, accent, highlight.
```

### A2UI as Engram

A2UI components emitted by agents are stored as Engrams with `kind: Kind::UiComponent`. This allows:
- **Replay**: Reviewing past agent output with A2UI components intact
- **Lineage**: Tracing which agent produced which UI element
- **Learning**: Analyzing which A2UI components correlate with task success

---

## Use Cases

### Research Output

A researcher agent investigating dependencies:

```jsonl
{"a2ui": "callout", "level": "info", "title": "Research: Auth Libraries", "content": "Compared 4 Rust auth libraries for JWT validation."}
{"a2ui": "table", "title": "Library Comparison", "columns": ["Library", "Stars", "Last Update", "Deps", "Recommendation"], "rows": [["jsonwebtoken", "1.2K", "2024-12", "3", "✓ Recommended"], ["jwt-simple", "340", "2024-11", "2", "Alternative"], ["frank_jwt", "120", "2023-06", "5", "✗ Stale"], ["alcoholic_jwt", "80", "2022-03", "4", "✗ Abandoned"]], "highlight_row": 0}
{"a2ui": "chart", "type": "bar", "title": "Dependency Count", "data": [{"label": "jsonwebtoken", "value": 3}, {"label": "jwt-simple", "value": 2}, {"label": "frank_jwt", "value": 5}, {"label": "alcoholic_jwt", "value": 4}]}
```

### Debugging Output

An implementer agent debugging a test failure:

```jsonl
{"a2ui": "callout", "level": "danger", "title": "Test Failure Analysis", "content": "3 tests failing in auth module after middleware change."}
{"a2ui": "status", "items": [{"label": "test_valid_token", "state": "pass"}, {"label": "test_expired_token", "state": "fail", "detail": "Expected Err(Expired), got Err(Invalid)"}, {"label": "test_missing_header", "state": "fail", "detail": "Panic at unwrap() line 45"}, {"label": "test_refresh_flow", "state": "fail", "detail": "Timeout after 5s"}]}
{"a2ui": "tree", "title": "Root Cause Analysis", "root": {"label": "Middleware ordering changed", "children": [{"label": "Auth check now runs before token parsing", "children": [{"label": "Expired tokens hit Invalid error path"}, {"label": "Missing headers cause unwrap panic"}]}, {"label": "Refresh endpoint unreachable after reorder", "children": [{"label": "Timeout waiting for token refresh"}]}]}}
```

### Architecture Proposal

An architect agent proposing a design:

```jsonl
{"a2ui": "callout", "level": "info", "title": "Architecture Proposal: Event Bus Refactor", "content": "Proposing migration from crossbeam channels to tokio broadcast for event distribution."}
{"a2ui": "table", "title": "Trade-off Matrix", "columns": ["Criterion", "crossbeam", "tokio broadcast", "Winner"], "rows": [["Backpressure", "Manual", "Automatic", "tokio"], ["Multi-consumer", "Clone per consumer", "Built-in", "tokio"], ["Async support", "Requires wrapper", "Native", "tokio"], ["Performance (1M msg/s)", "2.1μs/msg", "3.4μs/msg", "crossbeam"], ["Memory", "Fixed buffer", "Per-consumer lag", "crossbeam"]]}
{"a2ui": "kv", "title": "Recommendation", "items": [{"key": "Approach", "value": "tokio broadcast"}, {"key": "Reason", "value": "Async-native, simpler multi-consumer"}, {"key": "Risk", "value": "~60% throughput reduction (acceptable for our 10K msg/s load)"}, {"key": "Migration effort", "value": "~2 hours, 4 files"}]}
```

---

## Current Status and Gaps

**Built:**
- Agent output streaming (text mode)
- Engram storage for agent output
- ROSEDUST theme system (TUI and planned CSS)
- Braille chart rendering widget

**Not yet built:**
- A2UI protocol specification (JSON schema)
- A2UI parser (extract components from agent output)
- A2UI → ratatui renderer
- A2UI → React renderer
- A2UI → CLI text renderer
- Agent system prompt A2UI documentation
- Sandboxing and validation
- A2UI as Engram kind
- Component library (table, chart, progress, status, code, callout, tree, kv, diagram, form, markdown)

---

## Cross-references

- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color system and glass morphism
- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the agent output viewport
- See [13-web-portal.md](./13-web-portal.md) for the Web Portal rendering context
- See `refactoring-prd/09-innovations.md` §13 for the innovation context
