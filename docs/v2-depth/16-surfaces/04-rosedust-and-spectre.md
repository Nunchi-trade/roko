# ROSEDUST and Spectre

> Depth for [20-SURFACES.md](../../unified/20-SURFACES.md). Covers the ROSEDUST visual language as a Signal-derived rendering system, the Spectre creature as a deterministic visualization from agent identity HDC fingerprint, PAD affect-driven animation, four renderers as Cell specializations, and color science foundations.

---

## 1. ROSEDUST as a Signal-Derived Rendering System

ROSEDUST is Roko's design language -- a comprehensive visual system that unifies the appearance of every surface. In unified vocabulary, ROSEDUST design tokens are **config Signals** stored in the `config_current` projection. Every rendering decision derives from these Signals: palette colors, typography rules, motion parameters, and glass morphism depths.

The name evokes the palette's character: rose light on a deliberate ground plane, as if viewing the system through a faintly glowing, dusty lens.

ROSEDUST is **dark-led** with light, dark, and high-contrast variants over the same semantic token system. The rose palette is the signature accent; semantic colors (jade, amber, crimson, violet, sapphire) provide differentiation without breaking the overall identity.

---

## 2. The Palette as Signal

### Background Hierarchy

Three layers of depth create visual hierarchy without explicit borders:

| Token | Hex | OKLCH | Usage |
|---|---|---|---|
| `void-black` | `#0a0a0f` | (0.09, 0.01, 280) | Deepest background |
| `twilight` | `#12101a` | (0.13, 0.02, 280) | Card and panel backgrounds |
| `dusk` | `#1a1726` | -- | Elevated surfaces, modals |

Never use pure `#000000`. The void-black has a violet undertone that gives it warmth.

### Rose Palette (Primary Accent)

| Token | Hex | OKLCH | Usage |
|---|---|---|---|
| `rose-dim` | `#8b5e6b` | (0.51, 0.06, 10) | Muted, inactive |
| `rose` | `#c77d8f` | (0.65, 0.10, 12) | Standard accent |
| `rose-bright` | `#e8a0b2` | (0.77, 0.09, 10) | Active, highlighted |
| `rose-glow` | `#ffc0d0` | (0.87, 0.07, 8) | Maximum emphasis |

### Semantic Colors

| Token | Hex | Meaning | System Mapping |
|---|---|---|---|
| `jade` | `#5eead4` | Success, passing gates | Verify::Pass |
| `amber` | `#fbbf24` | Warnings, thresholds | Verify::Warning |
| `crimson` | `#f87171` | Errors, failed gates | Verify::Fail |
| `violet` | `#a78bfa` | Knowledge, Memory entries | Signal(kind=Knowledge) |
| `sapphire` | `#60a5fa` | Agents, active processes | Agent activity |

### Color Harmony Construction

The palette uses **analogous harmony** centered on rose (H=12), with semantic colors at harmonic OKLCH intervals:

```
Rose family:   H = 8-15   (analogous cluster)
Amber:         H = 85     (warm complement quadrant)
Jade:          H = 170    (complementary)
Sapphire:      H = 250    (cool triadic)
Violet:        H = 290    (split-complementary)
```

All ROSEDUST palette steps maintain equal L (lightness) values within each tonal ramp.

### Color Science

All gradient interpolation uses OKLab (Ottosson 2020) for perceptual uniformity. sRGB interpolation produces muddy midpoints; OKLab guarantees equal Euclidean distances correspond to equal perceived color differences.

Contrast is verified against APCA (Advanced Perceptual Contrast Algorithm, WCAG 3.0 candidate). Body text on void-black achieves Lc +94.2, well above the 75 minimum. No information is conveyed by color alone -- all status indicators use symbols (`checkmark`/`x`/`circle`) plus color.

**Source**: `crates/roko-cli/src/tui/theme.rs` (RosedustTheme), `crates/roko-cli/src/tui/color.rs` (gradient, darken, lighten, HSV conversion).

---

## 3. The Spectre Creature

Every Roko agent has a **Spectre** -- a procedurally generated creature that serves as a dense information display of the agent's cognitive state. The Spectre is not decorative; it is a **glanceable readout** encoding behavioral state, knowledge accumulation, activity level, and connectivity into a single visual entity.

### Deterministic from Identity

The Spectre is generated deterministically from the agent's identity HDC fingerprint:

```rust
/// BLAKE3(agent_id + template_name) -> 32-byte shape seed
/// The seed determines ALL morphological constants.
/// Animation state comes from the Daimon, not the seed.
fn shape_seed(agent_id: &str, agent_template: &str) -> [u8; 32] {
    blake3::hash(format!("{agent_id}:{agent_template}").as_bytes()).into()
}
```

The same agent always produces the same Spectre body. Operators learn to recognize agents by silhouette, like recognizing a person by face.

### Morphological Parameters from Seed

The 32-byte seed is divided into regions:

| Seed Bytes | Parameter | Range | Determines |
|---|---|---|---|
| `[0..4]` | `body_archetype` | 0-7 (8 types) | Base body shape |
| `[4..8]` | `symmetry` | bilateral / radial / asymmetric | Body symmetry |
| `[8..10]` | `limb_count` | 0-6 | Appendage count |
| `[10..12]` | `limb_style` | tentacle / fin / spike / tendril | Appendage rendering |
| `[12..14]` | `eye_count` | 1-4 | Eye elements |
| `[14..16]` | `eye_style` | round / slit / compound / star | Eye pattern |
| `[16..20]` | `domain_texture` | geometric / organic / crystalline / fluid | Surface pattern |
| `[20..24]` | `color_offset` | 0.0-1.0 | Hue shift within ROSEDUST |
| `[24..28]` | `proportion_ratios` | various | Body-to-limb proportions |

### Eight Body Archetypes

| # | Archetype | Shape | Character |
|---|---|---|---|
| 0 | Orb | Spherical, compact | Dense knowledge, focused |
| 1 | Column | Tall, narrow | Structured, methodical |
| 2 | Sprawl | Wide, low | Broad exploration |
| 3 | Cluster | Multiple connected nodes | Parallel processing |
| 4 | Teardrop | Tapered, directional | Goal-oriented |
| 5 | Ring | Hollow center | Monitoring, watchful |
| 6 | Fractal | Self-similar branching | Recursive analysis |
| 7 | Amorphous | Shifting boundaries | Exploratory, creative |

### Dot-Cloud Geometry

Spectres are internally represented as a **dot cloud** -- weighted 3D points connected by springs:

```rust
pub struct SpectreCloud {
    pub body_points: Vec<SpectrePoint>,      // Static (from seed)
    pub animated_points: Vec<SpectrePoint>,  // Dynamic (from Daimon)
    pub springs: Vec<Spring>,                // Organic movement
    pub bounds: BoundingBox,
}
```

Spring physics use Verlet integration -- time-reversible, energy-preserving, O(h^4) error. Spring parameters vary by behavioral state: Engaged is steady (stiffness 0.8, damping 0.6), Struggling is tense (1.2, 0.3), Resting is near-still (0.2, 0.9).

---

## 4. PAD Affect Drives Animation

The Spectre's animation is driven entirely by the Daimon PAD (Pleasure-Arousal-Dominance) vector. Each behavioral state produces a distinct visual character:

| State | Color | Breathing Hz | Eye State | Body Form | Spring Tension |
|---|---|---|---|---|---|
| **Engaged** | Rose | 0.7 | Open, bright | Steady, slight pulse | Medium |
| **Struggling** | Amber/Crimson | 1.4 | Wide, flickering | Constricted, jittery | High |
| **Coasting** | Sapphire | 0.4 | Half-open | Expanded, loose | Low |
| **Exploring** | Violet | 0.9 | Scanning | Shifting edges | Variable |
| **Focused** | Jade | 0.5 | Narrowed, intense | Compact, sharp | High, minimal movement |
| **Resting** | Dim Rose | 0.2 | Closed | Minimal, slow drift | Near-zero |

**Breathing** is the primary ambient animation -- a continuous expansion/contraction of the entire dot cloud. Breathing rate maps to arousal; depth maps to the state's energy level. Asymmetric breathing (quick inhale, slow exhale) adds organic character.

**Eyes** track the PAD vector: Pleasure controls brightness, Arousal controls openness, Dominance controls focus direction. Blink rate is a function of arousal (low arousal: 4-6s between blinks; high arousal: 1-1.5s).

**Glow** color and intensity encode the behavioral state. On truecolor terminals, a bloom composite effect creates light bleeding from the body into surrounding cells (distance 1: 60% intensity, distance 2: 30%, distance 3+: none).

**Critical design principle: Spectres never die.** No death animations, terminal states, or decay-to-nothing. An agent that stops working enters Resting state -- minimal form, slow breathing, dim glow. When the agent resumes, the Spectre reactivates.

---

## 5. Four Renderers as Cell Specializations

The same Spectre state (dot cloud + animation parameters) is rendered by four different Cell specializations, each producing renderer-specific output from the same input Signal:

| Renderer | Medium | Fidelity | Technology |
|---|---|---|---|
| **TUI ASCII** | Terminal characters | Medium | ratatui, Unicode box-drawing + braille |
| **Web WebGL** | 3D rendered pixels | High | Three.js, custom shaders, Kawase bloom |
| **CLI Inline** | Single-line or 5x3 compact | Low | Plain text with Unicode symbols |
| **API JSON** | Structured data | N/A (data) | JSON over HTTP/WebSocket |

### TUI ASCII Rasterization

```
SpectreCloud (3D) → Project to 2D → Quantize to character grid
    → Assign characters by density and kind → Apply ROSEDUST colors
    → Render as ratatui Cells
```

Character mapping: body points use shade blocks (`light-shade`/`medium-shade`/`dark-shade`/`full-block`), eyes use circle variants, tendrils use wave characters, particles use dots and sparkles.

Viewport adapts to size: < 20x8 = minimal (outline + eyes only), 20x8 to 40x12 = standard (+ breathing + glow), > 40x12 = detailed (+ limbs + tendrils + particles).

### WebGL Rendering

The web portal renders Spectres as 3D objects with custom shaders:
- **Body**: Instanced spheres with subsurface scattering approximation
- **Glow**: Multi-pass Kawase bloom (bright pass -> blur -> composite)
- **Eyes**: Emissive meshes with procedural iris texture
- **Particles**: GPU-instanced point sprites

Interactive features (web only): orbit camera, zoom, hover tooltips, click-to-focus.

### Shared Consistency

All four renderers share:
- The same behavioral state -> color mapping from ROSEDUST
- The same morphological parameter tables
- The same breathing rate, phase, and asymmetry values

A Spectre generated for agent `rust-impl-01` looks recognizably similar across all renderers.

---

## 6. Collective Visualization

When multiple Spectres are displayed together (Spectre Gallery, Stigmergy Minimap), they form a **collective display** that encodes emergent properties:

- **Breathing synchronization**: Kuramoto-inspired phase coupling. Agents in the same behavioral state gradually synchronize breathing. Coupling strength is proportional to c-factor. Near-perfect sync = high collective intelligence.
- **Filament connections**: Mesh connections rendered as character sequences between Spectres. Thin for idle links, thick with flow particles for active data transfer, wavy for pheromone channels.
- **Force-directed layout**: Tightly connected agents cluster together. Isolated agents drift to periphery. The spatial arrangement reflects the mesh topology.
- **Pheromone fields**: Visible fields around emitting Spectres (gold sparkles for Wisdom, crimson flashes for Warning, violet diamonds for Discovery).

---

## 7. Epistemic Aesthetics

Visual properties encode epistemic state -- the system's confidence and knowledge quality:

| Visual Property | Data Source | Encoding |
|---|---|---|
| Glow intensity | Epistemic confidence (gate pass rate) | Brighter = higher confidence |
| Fade / decay | Knowledge staleness (demurrage balance) | Faded entries need re-validation |
| Turbulence | Contested knowledge entries | Shimmer/jitter = active dispute |
| Velocity streaks | Active agent output (tokens/sec) | Faster = higher throughput |
| Heartbeat pulse | Per-agent tick cadence (gamma/theta/delta) | Visible rhythm matches agent clock |
| Saturation | Validation strength (gate rung depth) | Deeper validation = richer color |

---

## What This Enables

- **Glanceable system assessment**: An operator can assess system health in 200ms by reading Spectre state, breathing synchronization, and color distribution.
- **Deterministic identity**: The same agent always has the same Spectre body. Operators develop recognition over time.
- **Multi-surface consistency**: ROSEDUST tokens and Spectre parameters are shared across CLI, TUI, web, and API. The visual identity is coherent everywhere.
- **Accessibility**: NO_COLOR support, symbol-plus-color status indicators, APCA contrast verification, `prefers-reduced-motion` respect.

---

## Feedback Loops

- **Behavioral state -> visual -> operator -> intervention**: The Spectre's visual state (e.g., all agents Struggling = rapid amber breathing) triggers operator attention, leading to intervention (adjust budget, change model, pause plan), which changes behavioral state, which changes the visual.
- **Collective sync -> c-factor -> sync strength**: Higher c-factor increases Kuramoto coupling, which increases breathing synchronization, which is visually obvious, which reinforces coordination behavior.
- **Knowledge accumulation -> body density**: As an agent accumulates knowledge, its Spectre body becomes visually denser and more textured, providing a long-term readout of expertise.

---

## Open Questions

1. **Spectre rendering cost at scale**: With 16+ agents, the TUI collective display approaches the 16.6ms frame budget. What is the right LOD reduction strategy for large collectives?
2. **Color variant system**: ROSEDUST is dark-led. The light and high-contrast variants share the same semantic tokens but need different background values. How much design work is needed for full variant coverage?
3. **Terminal capability detection**: How should ROSEDUST degrade for terminals that support only 16 colors? The 256-color OKLab mapping exists, but 16-color is more aggressive.

---

## Implementation Tasks

| Task | Where | What |
|---|---|---|
| Build Spectre dot-cloud geometry generator | `crates/roko-cli/src/tui/` or new `roko-spectre` crate | Generate SpectreCloud from shape seed bytes |
| Implement spring physics with Verlet integration | Same as above | Animate dot cloud with state-dependent spring parameters |
| Build TUI ASCII rasterizer | `crates/roko-cli/src/tui/widgets/` | Project 3D cloud to 2D, quantize to characters, apply ROSEDUST colors |
| Implement breathing animation system | Same as above | Asymmetric breathing with state-dependent rate, depth, and phase |
| Implement glow composite for truecolor terminals | `crates/roko-cli/src/tui/color.rs` | Multi-cell color falloff from body outward |
| Implement Kuramoto synchronization for collective | `crates/roko-cli/src/tui/` | Phase coupling between agents in same behavioral state |
| Add OKLab gradient interpolation | `crates/roko-cli/src/tui/color.rs` | Replace sRGB linear interpolation with perceptually uniform OKLab |
