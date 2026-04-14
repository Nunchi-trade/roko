# Inner Worlds and Dream Rendering

> **Layer**: L3 Harness (monitoring/visualization)
>
> **Synapse Traits**: N/A (visualization layer, does not implement Synapse traits)
>
> **Prerequisites**: [01-three-phase-cycle.md](01-three-phase-cycle.md), [07-hypnagogia-engine.md](07-hypnagogia-engine.md)


> **Implementation**: Scaffold

---

## What Dream Rendering Is

Dream rendering is the visual representation of each dream phase as it would appear in the Roko TUI (terminal dashboard). Where the other sub-docs describe what happens computationally during dreams, this document describes what it looks like. Each sleep stage has a distinct terminal rendering following a single principle: **the rendering should make the agent's cognitive state legible without reducing it to telemetry.**

The rendering specification connects to the Roko TUI scaffold (currently text-only — the interactive TUI is item 7 on the implementation roadmap in CLAUDE.md). When the TUI is fully wired, dream visualization will be accessible via a portal mode during agent sleep.

---

## NREM: The Replay Theater

During NREM, the agent replays recent episodes with minor mutations. The visual rendering makes the consolidation process visible:

### Episodes as Vignettes

Each episode renders as a self-contained panel (40–60 columns wide, 8–14 rows tall) containing key data filtered through the agent's emotional state at encoding time. Episodes encoded during high arousal render with jittery borders (alternating `│` and `┊`). Episodes encoded during calm render with stable double-line borders (`║`).

### Salience Ordering

Vignettes are arranged by emotional significance, not chronology. The most intense episode occupies the viewport center. Surrounding episodes radiate outward in rings of decreasing salience. The viewer sees what the agent considers most important by looking at what is closest to the center.

### Visible Mutations

During perturbed replay (see [02-nrem-replay.md](02-nrem-replay.md)), mutated values are marked: they render in desaturated blue-gray with a tilde prefix (`~value`). The viewer can tell which parts of a memory the agent trusts (rendered normally) and which it is questioning (tilde-prefixed, dimmed).

```
NREM REPLAY THEATER (salience-ordered, center = highest)
╔════════════════════════════════════════════════════╗
║        ┌──────────────────────────┐                ║
║        │ TASK 142  refactor auth  │                ║
║        │ EDIT 4 files @ ~3 tries  │  <-- mutated   ║
║        │ gate: compile ✓ test ✗   │                ║
║        │ ◈ somatic: anxiety 0.72  │                ║
║        └──────────────────────────┘                ║
║   ┊╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┊   ┌────────────────────┐║
║   ┊ TASK 89  add tests  ┊   │ TASK 105 fix lint  │║
║   ┊ EDIT @ ~2 files     ┊   │ EDIT 1 file        │║
║   ┊ gate: all ✓         ┊   │ ◈ somatic: calm    │║
║   ┊╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┊   └────────────────────┘║
╚════════════════════════════════════════════════════╝
```

---

## REM: The Counterfactual Garden

During REM, the rendering shows a decision tree rooted in a real episode:

- **Real path** (left, solid borders): What actually happened
- **Counterfactual paths** (right, dashed borders): What could have happened

The more semantically distant the combined elements (measured by HDC cosine distance), the more visually distorted the rendering:

| Distance Band | Visual Treatment |
|---------------|-----------------|
| Slightly improbable (0.3–0.5) | Faint dream tint, dashed borders |
| Moderately improbable (0.5–0.7) | Character substitution, overlapping text |
| Wildly improbable (0.7+) | Full psychedelic mode — saturated colors, drifting characters |

The degree of visual distortion is information: the viewer can tell how far from reality the agent has wandered by how strange the screen looks.

---

## Hypnagogia: The Liminal Threshold

The hypnagogia rendering reflects the in-between quality of the state:

### Kluver Form Constants

Heinrich Kluver (1926) catalogued four geometric hallucination patterns that reliably appear at the edge of sleep: tunnels/funnels, spirals, lattices/honeycombs, and cobwebs. These are not random — they arise from the visual cortex's self-organizing dynamics.

The terminal renders these form constants using braille characters (`U+2800`–`U+28FF`), providing 2×4 sub-character resolution per cell. The patterns oscillate between forms every 4–6 seconds, filling the background behind dream content.

### Fragment Surfacing

Over the phosphene background, text fragments from NeuroStore surface and dissolve. These are partial — a few words from an episode, the first clause of an insight, a heuristic's condition without its conclusion. Fragments drift, fade, and are replaced by new ones.

### Connection Flashes

When two drifting fragments pass near each other and the agent detects a novel association (via the Homuncular Observer), a brief bright line draws between them (warm off-white, 200ms duration). This is the creative spark — the moment that hypnagogia is designed to produce. The flashes are rare, and their variable-ratio reinforcement pattern makes watching the hypnagogic display inherently engaging.

---

## Integration: Knowledge Crystallization

During integration, new insights materialize as text that types itself character by character:

- **High confidence**: Fast typing (15–20 chars/sec), steady rhythm
- **Low confidence**: Stuttering, backspacing, characters replaced before settling
- **Below threshold**: Characters drift apart, dissolve. The insight was not strong enough to survive

Fragments that make it to permanent NeuroStore render in warm off-white as they finalize — the same color used for connection flashes during hypnagogia. The color marks crystallization: this knowledge went from half-formed fragment through dream development through evaluation and into permanent doctrine.

---

## Portal Mode During Sleep

The TUI (when fully implemented) will provide a portal mode during agent sleep:

| Aspect | Waking Portal | Dream Portal |
|--------|---------------|--------------|
| Role | Active participant | Passive observer |
| Visual treatment | Sharp borders, full opacity | Blurred borders, 85% opacity |
| Content reliability | Knowledge entries are factual | Dream content is speculative |
| Navigation | Full | Phase-dependent or disabled |

During hypnagogia, navigation is disabled entirely (dream atonia). During NREM, arrow keys skip between vignettes. During REM, arrow keys traverse the counterfactual tree (but "back" is approximate — the dream space recombines between traversals). During Integration, observation only.

---

## Implementation Status

Dream rendering is **not yet implemented**. The TUI is currently text-only (item 7 on the implementation roadmap). The rendering specifications here describe the target visualization for when `ratatui` or a similar terminal UI library is wired.

---

## Dream Rendering Pipeline: Technical Specification

Formalize the rendering pipeline for when the TUI is fully wired with ratatui:

```rust
/// Dream rendering configuration for the TUI portal.
pub struct DreamRenderConfig {
    /// Maximum frames per second for dream animations.
    pub target_fps: u16,                  // default: 10, range: 4-30
    /// Phosphene pattern rotation speed (degrees per second).
    pub phosphene_rotation_speed: f64,    // default: 15.0, range: 5.0-45.0
    /// Fragment surface duration (milliseconds).
    pub fragment_surface_ms: u64,         // default: 3000, range: 1000-8000
    /// Connection flash duration (milliseconds).
    pub connection_flash_ms: u64,         // default: 200, range: 100-500
    /// Whether to render braille phosphenes in hypnagogia.
    pub braille_phosphenes: bool,         // default: true
    /// Dream phase transition animation duration (milliseconds).
    pub phase_transition_ms: u64,         // default: 1500, range: 500-3000
    /// Opacity for dream portal mode (0.0-1.0).
    pub dream_opacity: f64,              // default: 0.85, range: 0.60-1.0
}

/// Dream phase visual treatment specification.
pub struct PhaseVisualSpec {
    pub phase: DreamPhase,
    pub border_style: BorderStyle,
    pub content_opacity: f64,
    pub animation_type: AnimationType,
    pub color_palette: ColorPalette,
}

pub enum BorderStyle {
    /// Stable double-line borders for NREM (calm, structured).
    StableDouble,
    /// Dashed borders for REM counterfactuals.
    Dashed,
    /// Blurred/oscillating borders for hypnagogia.
    Oscillating { frequency_hz: f64 },
    /// No borders for integration (knowledge crystallization).
    None,
}

pub enum AnimationType {
    /// Static with periodic updates (NREM vignettes).
    StaticPeriodic { update_interval_ms: u64 },
    /// Continuously drifting (hypnagogia fragments).
    Drift { speed_chars_per_sec: f64 },
    /// Typing animation (integration crystallization).
    TypeWriter { chars_per_sec: f64 },
    /// Decision tree expansion (REM counterfactuals).
    TreeGrowth { branch_delay_ms: u64 },
}
```

---

## Cross-References

| Document | Relevance |
|----------|-----------|
| [01-three-phase-cycle.md](01-three-phase-cycle.md) | Dream phases that each have distinct visual treatments |
| [07-hypnagogia-engine.md](07-hypnagogia-engine.md) | Computational engine that produces the rendered content |
| [02-nrem-replay.md](02-nrem-replay.md) | Replay mechanics behind the vignette display |
