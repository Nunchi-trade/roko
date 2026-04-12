# Spectre Rendering Per Interface

> How the Spectre creature is rendered across four interfaces: TUI ASCII art, Web Portal WebGL, CLI inline, and API JSON state. Same data model, four renderers.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md), [07-rosedust-design-language.md](./07-rosedust-design-language.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §5, `bardo-backup/prd/18-interfaces/28-creature-system.md`, `roko-cli/src/tui/widgets/braille.rs`

---

## Abstract

The Spectre creature is defined by a single data model (the `SpectreCloud` dot-cloud geometry plus animation parameters from the Daimon PAD vector). This data model is rendered differently depending on the interface:

1. **TUI ASCII** — Unicode character rasterization in the terminal, using ratatui
2. **Web Portal WebGL** — 3D rendering with shaders, bloom, and particle effects
3. **CLI Inline** — Minimal single-line or compact representation for text output
4. **API JSON** — Raw state data for custom renderers and external integrations

All four renderers consume the same Spectre state model (see [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md)), ensuring visual consistency across interfaces. The state is transmitted via the `/ws/spectre/:id` WebSocket endpoint.

---

## Renderer 1: TUI ASCII

The primary Spectre renderer for the terminal dashboard. Renders the dot-cloud geometry as Unicode characters within a ratatui `Widget`.

### Rasterization Pipeline

```
SpectreCloud (3D points)
    │
    ▼
Project to 2D (orthographic, front view)
    │
    ▼
Quantize to character grid (viewport width × height)
    │
    ▼
Assign characters based on point density and kind
    │
    ▼
Apply color from behavioral state + glow composite
    │
    ▼
Render as ratatui Cells
```

### Character Mapping

The rasterizer maps point density and kind to Unicode characters:

| Point Kind | Low Density | Medium Density | High Density |
|---|---|---|---|
| Body | `░` (light shade) | `▒` (medium shade) | `▓` (dark shade) / `█` (full block) |
| Limb | `─` / `│` (thin) | `━` / `┃` (thick) | `═` / `║` (double) |
| Eye | `○` (open dim) | `◉` (open bright) | `◎` (wide) |
| Tendril | `~` | `≈` | `≋` |
| Particle | `·` | `•` | `✦` |
| Outline | `╭` `╮` `╰` `╯` `│` `─` | — | — |

### Body Outline Rendering

The body is rendered as a Unicode box-drawing outline with interior fill:

```rust
/// Characters used for body outline.
const OUTLINE: OutlineChars = OutlineChars {
    top_left: '╭',
    top_right: '╮',
    bottom_left: '╰',
    bottom_right: '╯',
    horizontal: '─',
    vertical: '│',
    // Curved variants for organic shapes
    curve_left: '╭',
    curve_right: '╮',
};
```

### Viewport Sizing

The Spectre viewport adapts to available terminal space:

| Viewport Size | Detail Level | Features |
|---|---|---|
| < 20×8 | Minimal | Body outline + eyes only |
| 20×8 – 40×12 | Standard | Body + eyes + breathing + glow |
| 40×12 – 60×16 | Detailed | Full body + limbs + tendrils + particles |
| > 60×16 | Gallery | Multiple Spectres side by side |

### Color Application

Colors are applied per-cell using ratatui's `Style`:

```rust
fn spectre_cell(
    theme: &RosedustTheme,
    behavioral_state: &BehavioralState,
    point_kind: PointKind,
    density: f32,
    distance_from_center: f32,
) -> Style {
    let base_color = state_color(theme, behavioral_state);
    let glow_factor = glow_falloff(distance_from_center);

    match point_kind {
        PointKind::Body => {
            let fg = base_color;
            let bg = if glow_factor > 0.0 {
                gradient(theme.bg, base_color, glow_factor * 0.3)
            } else {
                theme.bg
            };
            Style::default().fg(fg).bg(bg)
        }
        PointKind::Eye => {
            let brightness = eye_brightness(behavioral_state);
            let fg = lighten(base_color, brightness);
            Style::default().fg(fg).add_modifier(Modifier::BOLD)
        }
        PointKind::Tendril => {
            let fg = gradient(theme.fg_muted, base_color, 0.5);
            Style::default().fg(fg)
        }
        PointKind::Particle => {
            let fg = lighten(base_color, 0.3);
            Style::default().fg(fg)
        }
        _ => Style::default().fg(base_color),
    }
}
```

### Breathing Animation (TUI)

Breathing is rendered by modulating the body outline scale on each frame:

```rust
fn breathing_scale(params: &BreathingParams, time: f64) -> f32 {
    let phase = time * params.rate * 2.0 * std::f64::consts::PI + params.phase as f64;
    // Asymmetric breathing: quick inhale, slow exhale
    let raw = if phase.sin() > 0.0 {
        // Inhale phase (shorter)
        (phase * (1.0 / params.asymmetry as f64)).sin()
    } else {
        // Exhale phase (longer)
        (phase * params.asymmetry as f64).sin()
    };
    1.0 + (raw as f32) * params.depth * 0.1 // ±10% scale at full depth
}
```

The scale factor is applied to the projection step, expanding/contracting the character grid positions. At 60fps, this produces a smooth, organic breathing effect.

### Frame Budget

Spectre rasterization targets ~1.5ms within the 16.6ms frame budget:

| Step | Budget |
|---|---|
| 3D→2D projection | ~0.2ms |
| Grid quantization | ~0.3ms |
| Character assignment | ~0.4ms |
| Color computation | ~0.3ms |
| Cell output | ~0.3ms |
| **Total** | **~1.5ms** |

### Braille Mode

For compact rendering (small viewports or the Spectre Gallery), the braille rendering widget (`roko-cli/src/tui/widgets/braille.rs`) can be used. Braille characters (`⠀`–`⣿`) encode 2×4 pixel grids per character cell, providing 8× the resolution of normal characters.

```
Normal character mode (20×10):     Braille mode (same space, 40×40 effective):

    ╭─╮                              ⠿⠿⠿⠿⠿
╭───╯ ╰───╮                       ⠿⠿⠿⠿⠿⠿⠿⠿⠿
│  ◉    ◉  │                      ⠿⠿⠿⠿⠿⠿⠿⠿⠿⠿
╰─────────╯                       ⠿⠿⠿⠿  ⠿⠿⠿⠿
                                    ⠿⠿⠿⠿⠿⠿⠿⠿
                                     ⠿⠿⠿⠿⠿⠿
```

---

## Renderer 2: Web Portal WebGL

The Web Portal (see [13-web-portal.md](./13-web-portal.md)) renders Spectres as 3D objects using WebGL 2.0 / WebGPU.

### Architecture

```
SpectreCloud (from WebSocket)
    │
    ▼
Three.js Scene Graph
    │
    ├── Point Cloud (instanced mesh)
    ├── Spring Lines (line geometry)
    ├── Eye Meshes (emissive spheres)
    ├── Glow Volume (additive blending)
    ├── Tendril Curves (tube geometry)
    └── Particle System (GPU particles)
    │
    ▼
Custom Shader Pipeline
    │
    ├── Body Shader (subsurface scattering approximation)
    ├── Glow Shader (bloom post-process)
    ├── Eye Shader (emissive + pupil animation)
    └── Particle Shader (point sprites)
    │
    ▼
Post-Processing
    │
    ├── Bloom (screen-space, Kawase blur)
    ├── Vignette (ROSEDUST dark edges)
    └── Color Grading (ROSEDUST palette mapping)
```

### Point Cloud Rendering

The dot-cloud is rendered as instanced spheres with radius proportional to point weight:

```glsl
// Vertex shader for instanced point cloud
uniform float u_breathing_scale;
uniform float u_time;

attribute vec3 a_position;   // from SpectrePoint.position
attribute float a_weight;    // from SpectrePoint.weight
attribute vec3 a_color;      // from SpectrePoint.color or state color

void main() {
    vec3 pos = a_position * u_breathing_scale;
    // Spring physics displacement applied on CPU, sent as updated positions
    float radius = a_weight * 0.02;
    // ... instanced sphere rendering
}
```

### Glow and Bloom

The ROSEDUST glow effect is achieved through a multi-pass bloom pipeline:

1. **Bright pass**: Extract pixels brighter than threshold (Spectre body, eyes, particles)
2. **Kawase blur**: 4-pass separable blur (matches ROSEDUST "luxury" feel)
3. **Composite**: Additive blend bloom back onto scene
4. **Color grade**: Apply ROSEDUST palette mapping (warm shift, rose tint in highlights)

### Breathing in WebGL

Breathing modulates the scale uniform and camera-space vertex positions:

```javascript
function updateBreathing(spectre, dt) {
    const phase = spectre.breathingPhase + dt * spectre.breathingRate * Math.PI * 2;
    spectre.breathingPhase = phase % (Math.PI * 2);

    const scale = 1.0 + Math.sin(phase) * spectre.breathingDepth * 0.15;
    spectre.mesh.scale.setScalar(scale);
}
```

### Interactive Features (Web Only)

The Web Portal adds interactive features not available in the TUI:

- **Orbit camera**: Click and drag to rotate the Spectre in 3D
- **Zoom**: Mouse wheel to zoom in/out
- **Hover tooltips**: Hover over body parts for data (e.g., hover over eye → "State: Engaged, PAD: [0.7, 0.5, 0.8]")
- **Click to focus**: Click a Spectre to navigate to the Agent Detail page
- **Touch support**: Pinch to zoom, two-finger rotate on mobile/tablet

### Performance Targets

| Metric | Target |
|---|---|
| Frame rate | 60fps (requestAnimationFrame) |
| Draw calls | < 10 per Spectre |
| Triangle count | < 50,000 per Spectre |
| GPU memory | < 20MB per Spectre |
| Fallback | Canvas 2D for devices without WebGL 2 |

---

## Renderer 3: CLI Inline

For text-mode output (`roko status`, `roko dashboard --text`), Spectres are rendered as compact inline representations.

### Single-Line Mode

Used in agent list output and status reports:

```
rust-impl-01: ◉ Engaged [▓▓▓▓▓░░] P:0.7 A:0.5 D:0.8  (sonnet-4.6, 3/7, $0.34)
```

Format: `{name}: {eye} {state} [{bar}] P:{p} A:{a} D:{d}  ({model}, {progress}, {cost})`

### Compact Mode (3 lines)

Used in `roko dashboard --text` when space allows:

```
◉ rust-impl-01  Engaged  sonnet-4.6
  [▓▓▓▓▓░░] P:0.7 A:0.5 D:0.8  C:+0.12
  Task 3/7 │ 12 turns │ $0.34 │ 2 peers
```

### Mini Spectre (5×3)

The smallest visual Spectre representation, used in compact dashboards:

```
 ╭╮
╭◉◉╮   ← body + eyes, colored by state
 ╰╯
```

State variations:
```
Engaged:    Struggling:  Coasting:   Exploring:  Focused:   Resting:
 ╭╮          ╭╮           ╭─╮         ╭╮          ╭╮         ╭╮
╭◉◉╮        ╭◎◎╮         ╭○ ○╮      ≋◉◉≋        ╭◉◉╮       ╭──╮
 ╰╯          ╰╯           ╰──╯        ╰╯          ╰╯         ╰╯
(rose)      (amber)      (sapphire)  (violet)    (jade)     (dim rose)
```

---

## Renderer 4: API JSON

The raw Spectre state is available as JSON via the `/ws/spectre/:id` WebSocket endpoint and the `/api/agents/:id/spectre` REST endpoint. This enables custom renderers and external integrations.

### Full State Response

See the complete JSON schema in [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) § "Spectre State Model".

### Minimal State Response

For bandwidth-constrained consumers, a minimal state is available via `?minimal=true`:

```json
{
  "agent_id": "rust-impl-01",
  "state": "Engaged",
  "pad": [0.7, 0.5, 0.8],
  "breathing_rate": 0.7,
  "eye_state": "open",
  "glow_color": "#D4778C",
  "knowledge_total": 142,
  "mesh_peers": 2
}
```

### Custom Renderer Integration

External renderers can consume the Spectre state and produce custom visualizations:

1. **Connect** to `/ws/spectre/:id` (or poll `/api/agents/:id/spectre`)
2. **Parse** the SpectreState JSON
3. **Generate geometry** from `body.shape_seed` using the morphological parameter tables
4. **Animate** using the `animation.*` fields
5. **Render** using any graphics framework (Unity, Godot, Processing, p5.js, etc.)

The state update rate is ~30Hz during active agent work, dropping to ~1Hz during Resting state.

---

## Renderer Comparison

| Feature | TUI ASCII | Web WebGL | CLI Inline | API JSON |
|---|---|---|---|---|
| **Fidelity** | Medium | High | Low | N/A (data) |
| **Body shape** | Unicode outlines | 3D mesh | Symbols only | Raw seed |
| **Eyes** | Unicode symbols | Emissive meshes | Single char | State string |
| **Breathing** | Scale modulation | Scale + vertex | None | Rate value |
| **Glow** | Color gradient | Bloom shader | Color code | RGB hex |
| **Tendrils** | Wave characters | Tube geometry | Count only | Positions |
| **Particles** | Sparse characters | GPU particles | None | Array |
| **Interaction** | Keyboard nav | Mouse/touch | None | Programmatic |
| **Frame rate** | 60fps | 60fps | Static | 30Hz updates |
| **Dependencies** | ratatui | Three.js/WebGL | None | HTTP/WS |

---

## Rendering Consistency

### Shared Color Mapping

All renderers use the same behavioral state → color mapping from the ROSEDUST palette:

```rust
pub fn state_color(theme: &RosedustTheme, state: &BehavioralState) -> Color {
    match state {
        BehavioralState::Engaged => theme.rose,           // #D4778C
        BehavioralState::Struggling => theme.warning,     // #D4A857 (primary)
        BehavioralState::Coasting => theme.blue,          // #6B8FBD
        BehavioralState::Exploring => theme.lavender,     // #A08CC4
        BehavioralState::Focused => theme.teal,           // #5DB8A3
        BehavioralState::Resting => theme.rose_muted,     // #A05C6E
    }
}
```

The Web Portal's ROSEDUST Tailwind config (see [13-web-portal.md](./13-web-portal.md)) defines the same hex values in CSS custom properties, ensuring TUI and Web colors match exactly.

### Shared Morphology

All renderers that generate geometry (TUI ASCII, Web WebGL, custom) use the same morphological parameter tables from [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md). A Spectre generated for `rust-impl-01` looks recognizably similar across all renderers, despite the different fidelity levels.

### Shared Animation Timing

Breathing rate, phase, and asymmetry values are shared via the Spectre state model. All renderers should produce visually consistent breathing rhythms, even if the rendering techniques differ.

---

## Current Status and Gaps

**Built:**
- Braille rendering widget (`roko-cli/src/tui/widgets/braille.rs`)
- ROSEDUST color functions (`roko-cli/src/tui/color.rs`: gradient, lighten, darken)
- WebSocket endpoint scaffold (`roko-serve/src/routes/ws.rs`)

**Not yet built:**
- TUI ASCII Spectre rasterizer
- WebGL Spectre renderer (Web Portal)
- CLI inline Spectre representation
- `/api/agents/:id/spectre` REST endpoint
- Shared morphological parameter generator
- Spring physics simulation (shared across renderers)
- Breathing animation system

---

## Cross-references

- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for the Spectre data model and behavioral state mapping
- See [12-spectre-as-collective-display.md](./12-spectre-as-collective-display.md) for multi-agent rendering
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color system and bloom effects
- See [13-web-portal.md](./13-web-portal.md) for the Web Portal WebGL integration
- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the TUI Spectre viewport placement
