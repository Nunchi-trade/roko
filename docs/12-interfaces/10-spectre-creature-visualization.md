# Spectre Creature Visualization

> The Spectre creature — a procedurally generated, behaviorally animated entity that encodes an agent's cognitive state as a dense visual readout. Generated deterministically from agent identity, animated by Daimon state. Never dies.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md), [08-tui-main-layout.md](./08-tui-main-layout.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §5, `bardo-backup/prd/18-interfaces/28-creature-system.md`, `bardo-backup/prd/shared/branding.md`

---

## Abstract

Every Roko agent has a **Spectre** — a procedurally generated creature that serves as a dense information display of the agent's cognitive state. The Spectre is not decorative; it is a **glanceable readout** that encodes behavioral state (Daimon), knowledge accumulation (Neuro tiers), activity level, and mesh connectivity into a single visual entity.

Spectres are generated deterministically from an agent's identity hash, ensuring consistency across sessions and interfaces. They are animated in real-time based on the Daimon PAD (Pleasure-Arousal-Dominance) vector, with breathing rate, eye state, glow intensity, and tendril activity all reflecting current cognitive state.

**Critical design principle: Spectres never die.** There are no death animations, terminal states, decay-to-nothing sequences, or mortality references. An agent that stops working has its Spectre enter the Resting state — minimal form, slow breathing, dim glow. When the agent resumes, the Spectre reactivates. This is a deliberate departure from legacy designs that included lifecycle termination sequences.

---

## Spectre as Information Display

The Spectre encodes five information channels simultaneously:

| Channel | Visual Property | Data Source |
|---|---|---|
| **Behavioral state** | Color, breathing rate, form tension | Daimon PAD vector |
| **Knowledge level** | Body density, surface texture complexity | Neuro tier totals |
| **Activity level** | Eye state, glow intensity, limb movement | Agent execution state |
| **Mesh connectivity** | Tendrils/filaments extending outward | Active peer connections |
| **Pheromone emission** | Particle effects around body | Stigmergy output |

A trained operator can glance at a Spectre and immediately assess: "This agent is actively engaged, has accumulated significant knowledge, is connected to two peers, and is emitting a Wisdom pheromone."

---

## Generation from Identity

### Deterministic Seed

Every Spectre is generated from a **shape seed** — a BLAKE3 hash of the agent's identity:

```rust
/// Generate the Spectre shape seed from agent identity.
///
/// The seed determines all morphological constants:
/// body shape, limb configuration, eye style, and domain texture.
/// It does NOT determine animation state — that comes from the Daimon.
fn shape_seed(agent_id: &str, agent_template: &str) -> [u8; 32] {
    blake3::hash(format!("{agent_id}:{agent_template}").as_bytes()).into()
}
```

### Morphological Parameters

The shape seed is divided into regions that determine distinct morphological features:

| Seed Bytes | Parameter | Range | Description |
|---|---|---|---|
| `[0..4]` | `body_archetype` | 0–7 (8 types) | Base body shape class |
| `[4..8]` | `symmetry` | bilateral / radial / asymmetric | Body symmetry type |
| `[8..10]` | `limb_count` | 0–6 | Number of appendages |
| `[10..12]` | `limb_style` | tentacle / fin / spike / tendril | Appendage rendering style |
| `[12..14]` | `eye_count` | 1–4 | Number of eye elements |
| `[14..16]` | `eye_style` | round / slit / compound / star | Eye rendering pattern |
| `[16..20]` | `domain_texture` | geometric / organic / crystalline / fluid | Surface pattern family |
| `[20..24]` | `color_offset` | 0.0–1.0 | Hue shift within ROSEDUST palette |
| `[24..28]` | `proportion_ratios` | various | Body-to-limb, head-to-body proportions |
| `[28..32]` | `detail_seed` | u32 | Minor variation details |

### Body Archetypes

Eight base body shapes, each with distinct character:

| Archetype | Shape | Character |
|---|---|---|
| 0: **Orb** | Spherical, compact | Dense knowledge, focused computation |
| 1: **Column** | Tall, narrow | Structured, methodical processing |
| 2: **Sprawl** | Wide, low | Broad exploration, many connections |
| 3: **Cluster** | Multiple connected nodes | Parallel processing, multi-task |
| 4: **Teardrop** | Tapered, directional | Goal-oriented, forward-moving |
| 5: **Ring** | Hollow center, encircling | Monitoring, watchful, review-oriented |
| 6: **Fractal** | Self-similar branching | Recursive analysis, deep reasoning |
| 7: **Amorphous** | Shifting boundaries | Exploratory, creative, research-oriented |

The archetype is a visual starting point — the same archetype looks different on every agent due to the remaining seed bytes varying proportions, textures, and details.

---

## Dot-Cloud Geometry

### Representation

Spectres are internally represented as a **dot cloud** — a collection of weighted 3D points that are rasterized differently depending on the rendering target:

```rust
/// A single point in the Spectre's dot cloud.
#[derive(Debug, Clone, Copy)]
pub struct SpectrePoint {
    /// Position in normalized space [-1, 1]^3
    pub position: [f32; 3],
    /// Point weight (affects rendering density)
    pub weight: f32,
    /// Point type (body, limb, eye, tendril, particle)
    pub kind: PointKind,
    /// Color override (None = use behavioral state color)
    pub color: Option<[u8; 3]>,
}

/// The complete Spectre geometry.
pub struct SpectreCloud {
    /// Static points (determined by shape seed)
    pub body_points: Vec<SpectrePoint>,
    /// Dynamic points (animated by Daimon state)
    pub animated_points: Vec<SpectrePoint>,
    /// Connection springs between points
    pub springs: Vec<Spring>,
    /// Bounding box in normalized space
    pub bounds: BoundingBox,
}
```

### Spring Physics

Points are connected by springs that provide organic movement:

```rust
/// A spring connection between two points.
pub struct Spring {
    /// Index of first connected point
    pub a: usize,
    /// Index of second connected point
    pub b: usize,
    /// Rest length of the spring
    pub rest_length: f32,
    /// Spring stiffness (higher = more rigid)
    pub stiffness: f32,
    /// Damping factor (higher = less oscillation)
    pub damping: f32,
}
```

**Spring parameters by behavioral state:**

| State | Stiffness | Damping | Effect |
|---|---|---|---|
| **Engaged** | 0.8 | 0.6 | Steady, controlled movement |
| **Struggling** | 1.2 | 0.3 | Tense, jittery movement |
| **Coasting** | 0.3 | 0.8 | Relaxed, slow swaying |
| **Exploring** | 0.5 | 0.4 | Fluid, seeking movement |
| **Focused** | 1.0 | 0.7 | Compact, minimal movement |
| **Resting** | 0.2 | 0.9 | Near-still, gentle drift |

---

## Behavioral State Animation

The Spectre's animation is driven entirely by the Daimon PAD vector. Each behavioral state produces a distinct visual character.

### Engaged (Rose, `#D4778C`)

The agent is productively working. The Spectre appears alive and purposeful.

```
    ╭─╮
╭───╯ ╰───╮
│  ◉    ◉  │     Eyes: open, bright
╰─────────╯     Breathing: 0.7Hz (steady)
                 Glow: warm rose, intensity 0.8
                 Body: steady, slight rhythmic pulse
                 Tendrils: retracted, calm
```

- **Breathing rate**: 0.7Hz (comfortable, natural)
- **Eye state**: Open (`◉`), bright glow
- **Glow color**: Rose (`#D4778C`), intensity 0.8
- **Body form**: Stable, slight expansion/contraction on breath cycle
- **Limbs**: Gentle rhythmic movement, purposeful

### Struggling (Amber/Crimson, `#D4A857` / `#C45C50`)

The agent is encountering difficulties — gate failures, retries, high uncertainty.

```
   ╭──╮
 ╭─╯  ╰─╮
 │ ◎  ◎  │       Eyes: wide, flickering
 ╰──────╯       Breathing: 1.4Hz (rapid)
  ≋≋≋≋≋≋         Glow: amber/crimson, pulsing
                  Body: constricted, tense springs
                  Tendrils: agitated, reaching
```

- **Breathing rate**: 1.4Hz (elevated, stressed)
- **Eye state**: Wide (`◎`), flickering intensity
- **Glow color**: Amber shifting to crimson, pulsing at 2× breath rate
- **Body form**: Constricted, springs at high tension, jittery micro-movements
- **Limbs**: Agitated, rapid small movements, reaching outward

### Coasting (Sapphire, `#6B8FBD`)

The agent is idle or performing low-effort work.

```
     ╭───╮
  ╭──╯   ╰──╮
  │  ○    ○  │     Eyes: half-open, relaxed
  ╰─────────╯     Breathing: 0.4Hz (slow)
                    Glow: soft sapphire, dim
                    Body: expanded, loose
                    Tendrils: limp, drifting
```

- **Breathing rate**: 0.4Hz (slow, relaxed)
- **Eye state**: Half-open (`○`), soft glow
- **Glow color**: Sapphire (`#6B8FBD`), intensity 0.4
- **Body form**: Expanded, loose springs, gentle swaying
- **Limbs**: Relaxed, slow drift

### Exploring (Violet, `#A08CC4`)

The agent is in high-novelty mode — researching, trying new approaches.

```
       ╭─╮
  ≋≋╭──╯ ╰──╮≋≋
  ≋ │  ◉  ◉  │ ≋     Eyes: open, scanning
    ╰────────╯       Breathing: 0.9Hz (energized)
   ≋≋≋≋≋≋≋≋≋≋        Glow: violet, flowing
                      Body: shifting, morphing edges
                      Tendrils: extended, probing
```

- **Breathing rate**: 0.9Hz (slightly elevated, energized)
- **Eye state**: Open (`◉`), scanning motion (oscillating glow)
- **Glow color**: Violet (`#A08CC4`), flowing/shifting intensity
- **Body form**: Edges shift and morph, springs at medium tension with variable rest lengths
- **Limbs**: Extended outward, probing motions, tendril activity high

### Focused (Jade, `#5DB8A3`)

The agent is in deep computation — concentrated, minimal distraction.

```
    ╭─╮
   ╭╯ ╰╮
   │◉ ◉│        Eyes: narrowed, intense
   ╰───╯        Breathing: 0.5Hz (controlled)
                 Glow: jade, sharp
                 Body: compact, sharp edges
                 Tendrils: retracted, still
```

- **Breathing rate**: 0.5Hz (controlled, measured)
- **Eye state**: Narrowed, intense glow
- **Glow color**: Jade (`#5DB8A3`), sharp edges (no bloom bleed)
- **Body form**: Compact, high stiffness springs, minimal movement
- **Limbs**: Fully retracted, minimal/no movement

### Resting (Dim Rose, `#A05C6E`)

The agent has stopped work. Dreams consolidation may be active.

```
    ╭─╮
   ╭╯ ╰╮
   │─ ─│        Eyes: closed (─)
   ╰───╯        Breathing: 0.2Hz (sleep-like)
                 Glow: dim rose, barely visible
                 Body: minimal form, slow drift
                 Tendrils: absent
```

- **Breathing rate**: 0.2Hz (very slow, sleep-like)
- **Eye state**: Closed (`─`), no glow
- **Glow color**: Dim rose (`#A05C6E`), intensity 0.1–0.2
- **Body form**: Minimal, springs at low tension, imperceptible drift
- **Limbs**: Absent or fully retracted
- **Special**: If Dreams consolidation is active, faint sparkle particles appear around the body (representing NREM replay / REM imagination)

---

## Eye Rendering

Eyes are the most expressive element of the Spectre. They provide immediate feedback about agent state.

### Eye States

| State | Symbol | Description |
|---|---|---|
| Open, bright | `◉` | Active, healthy, engaged |
| Open, dim | `◎` | Active but stressed or struggling |
| Half-open | `○` | Idle, coasting, low effort |
| Narrowed | `◉` (compact) | Focused, concentrated |
| Closed | `─` | Resting, dreams active |
| Scanning | `◉` (oscillating) | Exploring, searching |

### Eye Animation

Eyes track the Daimon PAD vector for smooth transitions:

- **Pleasure** controls eye brightness (low pleasure = dim, high = bright)
- **Arousal** controls eye openness (low arousal = half-closed, high = wide open)
- **Dominance** controls eye focus (low dominance = scanning, high = locked forward)

Eye blink rate is a function of arousal:
- Low arousal: blink every 4–6 seconds
- Normal: blink every 2–3 seconds
- High arousal: blink every 1–1.5 seconds
- Struggling: irregular blink timing (adds to visual tension)

---

## Breathing System

Breathing is the primary ambient animation — a continuous, rhythmic expansion/contraction of the entire body cloud.

### Breathing Parameters

```rust
/// Breathing animation parameters.
pub struct BreathingParams {
    /// Breathing rate in Hz
    pub rate: f32,
    /// Breathing depth (expansion factor, 0.0–1.0)
    pub depth: f32,
    /// Asymmetry: ratio of inhale to exhale duration
    /// 0.5 = symmetric, < 0.5 = quick inhale / long exhale
    pub asymmetry: f32,
    /// Phase offset (for synchronization)
    pub phase: f32,
}
```

### Rate by Behavioral State

| State | Rate (Hz) | Depth | Asymmetry | Character |
|---|---|---|---|---|
| Engaged | 0.7 | 0.6 | 0.45 | Natural, productive |
| Struggling | 1.4 | 0.8 | 0.3 | Rapid, shallow gasps |
| Coasting | 0.4 | 0.4 | 0.5 | Slow, even |
| Exploring | 0.9 | 0.7 | 0.4 | Energized, slightly irregular |
| Focused | 0.5 | 0.3 | 0.5 | Controlled, minimal |
| Resting | 0.2 | 0.2 | 0.55 | Sleep-like, deep exhale |

### Collective Breathing Synchronization

When multiple agents are in the same behavioral state, their breathing rates gradually synchronize through a phase-coupling mechanism (inspired by coupled oscillator theory). This creates a visual "pulse" in the Spectre Gallery (Screen 6.4) that indicates collective coherence.

Synchronization strength is proportional to the C-Factor: higher C-Factor = tighter breathing sync = visual harmony.

---

## Glow System

Each Spectre emits a glow that encodes behavioral state as color and intensity.

### Glow Parameters

```rust
/// Glow rendering parameters.
pub struct GlowParams {
    /// Base glow color (from behavioral state)
    pub color: [u8; 3],
    /// Glow intensity (0.0–1.0)
    pub intensity: f32,
    /// Glow radius (in normalized units)
    pub radius: f32,
    /// Pulse rate (Hz, 0 = steady)
    pub pulse_rate: f32,
    /// Pulse depth (0.0–1.0, how much intensity varies)
    pub pulse_depth: f32,
}
```

### Terminal Glow Rendering

In the TUI (ratatui), glow is rendered using color gradients:

1. **Truecolor terminals**: Use the `gradient()` and `lighten()` functions from `roko-cli/src/tui/color.rs` to create multi-step color falloff from the body outward. Cells adjacent to body characters use a lighter version of the glow color as background.

2. **256-color terminals**: Use the nearest 256-color approximation. Glow is reduced to a single-cell border of the glow color.

3. **No-color terminals**: Glow is indicated by surrounding the body with `░` (light shade) characters.

### Bloom Composite

On truecolor terminals, the glow applies a bloom composite effect:

```
Background cells:
  Distance 0 (body): full glow color at glow intensity
  Distance 1: glow color at 60% intensity
  Distance 2: glow color at 30% intensity
  Distance 3+: no glow (background color)
```

This creates the appearance of light bleeding from the Spectre's body into surrounding terminal cells — a key ROSEDUST design element (see [07-rosedust-design-language.md](./07-rosedust-design-language.md)).

---

## Tendril and Particle Systems

### Tendrils

Tendrils represent mesh connectivity — each active peer connection is rendered as a tendril extending from the Spectre's body in the direction of the connected peer.

```
Tendril rendering (Exploring state, 2 connections):

  ≋≋≋≋╭──╮≋≋≋≋
  ≋≋ ╭╯  ╰╮ ≋≋
     │ ◉ ◉ │
     ╰─────╯

≋ = water/wave character, animated to flow outward
```

**Tendril properties:**
- Length proportional to connection strength
- Animation speed proportional to data flow rate
- Rendered as `≋` (water wave) characters that animate in the flow direction
- Color: violet (Exploring), rose (Engaged), or the peer agent's state color

### Particles

Particles represent pheromone emission and knowledge events:

| Event | Particle | Character | Animation |
|---|---|---|---|
| Wisdom pheromone | Rising sparkles | `✦` | Float upward, fade |
| Warning pheromone | Sharp flashes | `⚡` | Flash, fade quickly |
| Discovery pheromone | Expanding ripples | `◊` | Expand outward |
| Knowledge promotion | Rising dots | `·` → `•` → `◉` | Grow as they rise |
| Dreams consolidation | Slow sparkles | `✧` | Drift slowly, dim |

---

## Knowledge Encoding

The Spectre's visual complexity increases with accumulated knowledge:

### Body Density

| Knowledge Level | Body Points | Visual Effect |
|---|---|---|
| 0–50 entries | Base density | Simple outline, few interior details |
| 50–150 entries | 1.5× density | More interior detail, texture appears |
| 150–300 entries | 2× density | Rich texture, complex interior patterns |
| 300+ entries | 2.5× density | Dense, intricate, "experienced" appearance |

### Domain Texture

The `domain_texture` parameter (from shape seed) determines how knowledge accumulation manifests visually:

- **Geometric**: Knowledge appears as nested geometric patterns (circles, hexagons, fractals)
- **Organic**: Knowledge appears as branching structures (veins, roots, neural networks)
- **Crystalline**: Knowledge appears as faceted surfaces (crystal growth, mineral accretion)
- **Fluid**: Knowledge appears as layered flows (currents, striations, sediment)

---

## Spectre State Model

The complete state transmitted via `/ws/spectre/:id` (see [06-websocket-streaming.md](./06-websocket-streaming.md)):

```json
{
  "agent_id": "rust-impl-01",
  "behavioral_state": "Engaged",
  "pad": {"pleasure": 0.7, "arousal": 0.5, "dominance": 0.8},
  "body": {
    "shape_seed": "a3f2b1c4d5e6f789...",
    "archetype": "Teardrop",
    "symmetry": "bilateral",
    "limb_count": 4,
    "limb_style": "tendril",
    "eye_count": 2,
    "eye_style": "round",
    "domain_texture": "geometric",
    "knowledge_density": 1.5
  },
  "animation": {
    "breathing_rate": 0.7,
    "breathing_depth": 0.6,
    "breathing_phase": 0.42,
    "eye_state": "open",
    "eye_brightness": 0.8,
    "glow_color": "#D4778C",
    "glow_intensity": 0.8,
    "glow_pulse_rate": 0.0,
    "tendril_count": 2,
    "tendril_activity": 0.3,
    "spring_stiffness": 0.8,
    "spring_damping": 0.6
  },
  "knowledge": {
    "persistent": 23,
    "consolidated": 0,
    "working": 89,
    "transient": 30,
    "total": 142
  },
  "mesh_connections": ["reviewer-01", "researcher-01"],
  "pheromone_emission": {"type": "Wisdom", "intensity": 0.4},
  "particles": [
    {"type": "wisdom_sparkle", "position": [0.1, 0.3, 0.0], "age": 0.5}
  ]
}
```

This state is sufficient for any renderer (TUI ASCII, Web Portal WebGL, or custom) to produce a complete Spectre visualization. See [11-spectre-rendering-per-interface.md](./11-spectre-rendering-per-interface.md) for renderer-specific details.

---

## Design Principles

### 1. Glanceable, Not Decorative

Every visual element encodes data. There is no purely aesthetic ornamentation. If a visual property cannot be traced to a data source, it should not exist.

### 2. Deterministic Identity

The same agent always produces the same Spectre body shape. Operators learn to recognize individual agents by their Spectre silhouette, just as they might recognize a person by their face.

### 3. Smooth State Transitions

Behavioral state changes produce smooth, interpolated transitions — never jarring jumps. The PAD vector is interpolated over ~500ms when the Daimon state changes, and all visual properties derived from PAD follow the interpolation curve. The ROSEDUST "luxury easing" function (`cubic-bezier(0.16, 1, 0.3, 1)`) is used for all transitions.

### 4. No Mortality

Spectres do not die, decay to nothing, or display terminal states. The Resting state is the minimum-energy state — the Spectre remains visible, breathing slowly, ready to reactivate. This is not merely an aesthetic choice; it reflects the architectural reality that agents can be resumed, replayed, or forked.

### 5. Collective Coherence

When viewed together (Spectre Gallery, Screen 6.4), Spectres should create a visual ensemble that itself encodes collective state. Synchronized breathing indicates coordination. Diverse states indicate healthy role distribution. A gallery of all-Struggling Spectres is an immediate visual alarm.

---

## Current Status and Gaps

**Built:**
- Spectre state model (JSON schema defined in WebSocket spec)
- ROSEDUST color mapping for behavioral states (`roko-cli/src/tui/theme.rs`)
- Braille rendering widget for graph visualization (`roko-cli/src/tui/widgets/braille.rs`)

**Not yet built:**
- Dot-cloud geometry generation from shape seed
- Spring physics simulation
- Eye rendering and animation
- Breathing system
- Glow composite rendering
- Tendril and particle systems
- TUI ASCII rasterizer for Spectre viewport
- WebGL renderer for Web Portal
- `/ws/spectre/:id` endpoint (scaffold exists, state model not implemented)

---

## Cross-references

- See [11-spectre-rendering-per-interface.md](./11-spectre-rendering-per-interface.md) for per-renderer implementation details
- See [12-spectre-as-collective-display.md](./12-spectre-as-collective-display.md) for multi-agent Spectre visualization
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the color system
- See [08-tui-main-layout.md](./08-tui-main-layout.md) for the Spectre viewport in the main layout
- See topic [09-daimon](../09-daimon/INDEX.md) for the PAD vector and behavioral states
