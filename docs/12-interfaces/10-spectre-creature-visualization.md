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

## Procedural Generation Techniques

### L-System Body Construction

Spectre body topology can be generated using stochastic L-systems — parallel rewriting grammars that produce organic branching structures. The agent's shape seed selects production rules:

```rust
/// L-system for organic Spectre body topology generation.
pub struct SpectreGrammar {
    pub axiom: String,
    pub rules: Vec<ProductionRule>,
    pub iterations: u32,
}

pub struct ProductionRule {
    pub symbol: char,
    pub probability: f32,
    pub replacement: String,
}

/// Interpret L-system string as 3D turtle geometry.
/// F = forward, + = yaw left, - = yaw right,
/// ^ = pitch up, & = pitch down, [ = push, ] = pop
pub fn interpret_lsystem(program: &str, step: f32, angle: f32) -> Vec<SpectrePoint> {
    let mut points = Vec::new();
    let mut stack = Vec::new();
    let mut pos = [0.0f32; 3];
    let mut heading = [0.0, 1.0, 0.0]; // up
    for ch in program.chars() {
        match ch {
            'F' => {
                pos[0] += heading[0] * step;
                pos[1] += heading[1] * step;
                pos[2] += heading[2] * step;
                points.push(SpectrePoint {
                    position: pos,
                    weight: 1.0,
                    kind: PointKind::Body,
                    color: None,
                });
            }
            '[' => stack.push((pos, heading)),
            ']' => { if let Some((p, h)) = stack.pop() { pos = p; heading = h; } }
            '+' => rotate_yaw(&mut heading, angle),
            '-' => rotate_yaw(&mut heading, -angle),
            _ => {}
        }
    }
    points
}
```

### Reaction-Diffusion Surface Textures

The `domain_texture` parameter uses a Gray-Scott reaction-diffusion model to generate unique surface patterns. The model evolves two concentrations `A` and `B` on a 2D grid via coupled PDEs:

```
∂A/∂t = DA·∇²A − A·B² + f·(1 − A)
∂B/∂t = DB·∇²B + A·B² − (f + k)·B
```

**Pattern classes by (f, k) parameters (Pearson classification):**

| Class | f | k | Visual Pattern | Domain Texture |
|---|---|---|---|---|
| α | 0.010 | 0.047 | Scattered spots | `crystalline` |
| δ | 0.026 | 0.051 | Spots + stripes | `geometric` |
| η | 0.034 | 0.063 | Labyrinthine worms | `organic` |
| μ | 0.046 | 0.059 | Fingerprint waves | `fluid` |

The shape seed's `domain_texture` bytes select (f, k) parameters, producing deterministic patterns unique to each agent:

```rust
/// Gray-Scott reaction-diffusion step.
pub fn gray_scott_step(
    a: &mut [[f32; W]; H], b: &mut [[f32; W]; H],
    f: f32, k: f32, da: f32, db: f32,
) {
    let (a_prev, b_prev) = (a.clone(), b.clone());
    for y in 1..H-1 {
        for x in 1..W-1 {
            let lap_a = laplacian_9pt(&a_prev, x, y);
            let lap_b = laplacian_9pt(&b_prev, x, y);
            let ab2 = a_prev[y][x] * b_prev[y][x] * b_prev[y][x];
            a[y][x] = (a_prev[y][x] + da * lap_a - ab2 + f * (1.0 - a_prev[y][x]))
                      .clamp(0.0, 1.0);
            b[y][x] = (b_prev[y][x] + db * lap_b + ab2 - (f + k) * b_prev[y][x])
                      .clamp(0.0, 1.0);
        }
    }
}
```

### Fractional Brownian Motion for Organic Variation

Fine-grained organic detail uses fBm noise layered on body point positions:

```rust
/// Fractional Brownian Motion — multi-octave coherent noise.
pub fn fbm(x: f32, y: f32, seed: u32, octaves: u32) -> f32 {
    let mut value = 0.0f32;
    let mut amplitude = 0.5f32;
    let mut frequency = 1.0f32;
    for _ in 0..octaves {
        value += amplitude * simplex_noise(x * frequency, y * frequency, seed);
        amplitude *= 0.5;      // gain: each octave half amplitude
        frequency *= 2.0;      // lacunarity: each octave double frequency
    }
    value
}
```

### Verlet Integration for Spring Physics

Spring physics use Verlet integration — time-reversible, symplectic (energy-preserving), and O(h⁴) error without explicit velocity:

```rust
/// Verlet integration step for spring-mass system.
pub fn verlet_step(nodes: &mut [PhysicsNode], springs: &[Spring], dt: f32, gravity: [f32; 3]) {
    // Position update via Verlet
    for node in nodes.iter_mut().filter(|n| !n.pinned) {
        let vel = [
            node.pos[0] - node.prev[0],
            node.pos[1] - node.prev[1],
            node.pos[2] - node.prev[2],
        ];
        node.prev = node.pos;
        node.pos[0] += vel[0] + gravity[0] * dt * dt;
        node.pos[1] += vel[1] + gravity[1] * dt * dt;
        node.pos[2] += vel[2] + gravity[2] * dt * dt;
    }

    // Spring constraint satisfaction
    for s in springs {
        let delta = [
            nodes[s.b].pos[0] - nodes[s.a].pos[0],
            nodes[s.b].pos[1] - nodes[s.a].pos[1],
            nodes[s.b].pos[2] - nodes[s.a].pos[2],
        ];
        let dist = (delta[0]*delta[0] + delta[1]*delta[1] + delta[2]*delta[2]).sqrt().max(0.0001);
        let stretch = dist - s.rest_length;
        let correction = stretch * s.stiffness * 0.5;
        let dir = [delta[0]/dist, delta[1]/dist, delta[2]/dist];
        if !nodes[s.a].pinned {
            nodes[s.a].pos[0] += dir[0] * correction;
            nodes[s.a].pos[1] += dir[1] * correction;
            nodes[s.a].pos[2] += dir[2] * correction;
        }
        if !nodes[s.b].pinned {
            nodes[s.b].pos[0] -= dir[0] * correction;
            nodes[s.b].pos[1] -= dir[1] * correction;
            nodes[s.b].pos[2] -= dir[2] * correction;
        }
    }
}

pub struct PhysicsNode {
    pub pos: [f32; 3],
    pub prev: [f32; 3],
    pub mass: f32,
    pub pinned: bool,
}
```

### Phase-Space Portrait for Behavioral State Display

The Spectre's behavioral trajectory can be visualized as a phase-space portrait — plotting (Energy, Arousal) over a history window. This reveals attractor basins corresponding to behavioral modes:

- **Fixed points** (equilibria): circles where trajectory velocity → 0 (stable behavioral states)
- **Limit cycles** (periodic behavior): closed loops representing work-rest oscillations
- **Separatrices**: curves dividing different attractor basins

```rust
/// Phase portrait data for behavioral state visualization.
pub struct PhasePortrait {
    pub history: VecDeque<(f32, f32)>,  // (energy, arousal) samples
    pub max_history: usize,             // default: 200 samples
}

impl PhasePortrait {
    pub fn push(&mut self, energy: f32, arousal: f32) {
        self.history.push_back((energy, arousal));
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }
    }

    /// Render as braille canvas in a ratatui Rect.
    pub fn render(&self, area: Rect, buf: &mut Buffer, theme: &RosedustTheme) {
        // Map history to braille canvas coordinates
        // Color-code by recency (newer = brighter)
        // Draw trajectory as connected points
    }
}
```

### SDF Body Shape Composition

Body silhouettes are composed using Signed Distance Fields — combining primitives with smooth blending:

```rust
/// 2D SDF primitives for body shape composition.
pub fn sd_circle(p: [f32; 2], r: f32) -> f32 {
    (p[0]*p[0] + p[1]*p[1]).sqrt() - r
}

pub fn sd_ellipse(p: [f32; 2], ab: [f32; 2]) -> f32 {
    // Approximate: scale space then use circle
    let q = [p[0] / ab[0], p[1] / ab[1]];
    ((q[0]*q[0] + q[1]*q[1]).sqrt() - 1.0) * ab[0].min(ab[1])
}

/// Smooth union (Inigo Quilez) — organic blending of shapes.
pub fn op_smooth_union(d1: f32, d2: f32, k: f32) -> f32 {
    let h = (k - (d1 - d2).abs()).max(0.0) / k;
    d1.min(d2) - h * h * k * 0.25
}

/// Compose a Spectre body from archetype parameters.
pub fn compose_body(p: [f32; 2], archetype: BodyArchetype, seed: &ShapeSeed) -> f32 {
    match archetype {
        BodyArchetype::Orb => sd_circle(p, 0.4),
        BodyArchetype::Teardrop => {
            let head = sd_circle([p[0], p[1] - 0.2], 0.25);
            let body = sd_ellipse(p, [0.3, 0.5]);
            op_smooth_union(head, body, 0.15)
        }
        BodyArchetype::Ring => {
            let outer = sd_circle(p, 0.4);
            let inner = sd_circle(p, 0.2);
            outer.max(-inner)  // annular (hollow)
        }
        // ... other archetypes
        _ => sd_circle(p, 0.3),
    }
}
```

### Procedural Iris Generation

Eyes use radially symmetric procedural textures with fiber patterns:

```rust
/// Generate eye iris texture for a Spectre.
pub struct IrisParams {
    pub pupil_dilation: f32,  // 0.15 (constricted) to 0.65 (dilated)
    pub base_color: [u8; 3],  // from behavioral state
    pub fiber_count: u32,     // 20–60, from shape seed
    pub ring_frequency: f32,  // 40–80, from shape seed
}

impl IrisParams {
    /// Sample iris color at point p relative to eye center.
    pub fn sample(&self, p: [f32; 2]) -> Option<[u8; 3]> {
        let r = (p[0]*p[0] + p[1]*p[1]).sqrt();
        let theta = p[1].atan2(p[0]);

        // Pupil
        if r < self.pupil_dilation { return Some([5, 5, 8]); }

        // Outside iris
        if r > 1.0 { return None; }

        // Radial fibers + concentric rings
        let fiber = (theta * self.fiber_count as f32).sin();
        let ring = (r * self.ring_frequency).sin() * 0.5 + 0.5;
        let intensity = 0.6 + 0.4 * fiber * ring;

        Some([
            (self.base_color[0] as f32 * intensity) as u8,
            (self.base_color[1] as f32 * intensity) as u8,
            (self.base_color[2] as f32 * intensity) as u8,
        ])
    }
}
```

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

## Test Criteria

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_seed_is_deterministic() {
        let seed1 = shape_seed("agent-01", "code-implementer");
        let seed2 = shape_seed("agent-01", "code-implementer");
        assert_eq!(seed1, seed2, "Same identity must produce same seed");
    }

    #[test]
    fn different_agents_produce_different_seeds() {
        let seed1 = shape_seed("agent-01", "code-implementer");
        let seed2 = shape_seed("agent-02", "code-implementer");
        assert_ne!(seed1, seed2);
    }

    #[test]
    fn body_archetype_from_seed_in_range() {
        for i in 0..256u8 {
            let seed = [i; 32];
            let archetype = body_archetype_from_seed(&seed);
            assert!((archetype as u8) < 8, "Archetype must be 0–7");
        }
    }

    #[test]
    fn breathing_scale_is_bounded() {
        let params = BreathingParams { rate: 1.4, depth: 1.0, asymmetry: 0.3, phase: 0.0 };
        for t in (0..1000).map(|i| i as f64 * 0.01) {
            let scale = breathing_scale(&params, t);
            assert!(scale > 0.85 && scale < 1.15, "Breathing scale must stay within ±15%");
        }
    }

    #[test]
    fn spring_physics_converges() {
        let mut nodes = vec![
            PhysicsNode { pos: [0.0, 0.0, 0.0], prev: [0.0, 0.0, 0.0], mass: 1.0, pinned: true },
            PhysicsNode { pos: [2.0, 0.0, 0.0], prev: [2.0, 0.0, 0.0], mass: 1.0, pinned: false },
        ];
        let springs = vec![Spring { a: 0, b: 1, rest_length: 1.0, stiffness: 0.5, damping: 0.6 }];
        for _ in 0..1000 {
            verlet_step(&mut nodes, &springs, 0.016, [0.0, 0.0, 0.0]);
        }
        let dist = ((nodes[1].pos[0] - nodes[0].pos[0]).powi(2)).sqrt();
        assert!((dist - 1.0).abs() < 0.1, "Spring should converge toward rest length");
    }

    #[test]
    fn sdf_smooth_union_is_symmetric() {
        let d1 = 0.5;
        let d2 = 0.3;
        let k = 0.1;
        let r1 = op_smooth_union(d1, d2, k);
        let r2 = op_smooth_union(d2, d1, k);
        assert!((r1 - r2).abs() < 1e-6, "Smooth union must be commutative");
    }

    #[test]
    fn gray_scott_concentration_stays_bounded() {
        let mut a = [[1.0f32; 64]; 64];
        let mut b = [[0.0f32; 64]; 64];
        // Seed a small perturbation
        for y in 28..36 { for x in 28..36 { b[y][x] = 0.25; a[y][x] = 0.75; } }
        for _ in 0..100 {
            gray_scott_step(&mut a, &mut b, 0.034, 0.063, 1.0, 0.5);
        }
        for row in &a { for &v in row { assert!(v >= 0.0 && v <= 1.0); } }
        for row in &b { for &v in row { assert!(v >= 0.0 && v <= 1.0); } }
    }
}
```

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
