# Sonification (Behavioral State Audio)

> Ambient generative music driven by agent behavioral states — the Eno mandate, five musical layers, behavioral state presets, and the emotional harmonic vocabulary. No mortality, no requiem. Music reflects cognitive engagement, not lifecycle endings.


> **Implementation**: Scaffold

**Topic**: [12-interfaces](./INDEX.md)
**Prerequisites**: [07-rosedust-design-language.md](./07-rosedust-design-language.md), [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md)
**Key sources**: `refactoring-prd/06-interfaces.md` §8, `refactoring-prd/08-translation-guide.md` §sonification, `bardo-backup/prd/24-sonification/05-musical-language.md`, `bardo-backup/prd/24-sonification/06-preset-catalog.md`

---

## Abstract

Roko's sonification system generates ambient music that encodes cognitive agent state as sound. The system follows the **Eno mandate**: music must be "as ignorable as it is interesting" — an ambient generative soundtrack that provides peripheral awareness of system state without demanding attention.

The sonification system maps the Daimon's six behavioral states (Engaged, Struggling, Coasting, Exploring, Focused, Resting) to musical parameters: key, tempo, density, harmonic vocabulary, and texture. The result is a continuously evolving soundscape that makes system state audible — an operator can hear when agents are struggling, exploring, or resting without looking at a screen.

**Critical reframing**: The legacy sonification system mapped music to lifecycle phases (Thriving → Fading → Terminal). This mapping is removed. All musical parameters now derive from the six behavioral states defined by the Daimon PAD vector. There is no "terminal requiem" preset. There is no "death sound." Music reflects cognitive engagement, not lifecycle endpoints.

---

## The Eno Mandate

### Design Philosophy

Brian Eno's ambient music principles (from *Music for Airports*, 1978) define the sonification design:

1. **Ignorable and interesting**: The music must not demand attention, but reward it when given
2. **Generative, not composed**: Music is produced by systems and rules, not pre-recorded tracks
3. **Infinite duration**: The music never loops, never repeats, never ends — it evolves
4. **Environmental**: The music is part of the environment, not a performance
5. **Silence is an instrument**: Pauses and empty space are compositional elements

### Reference Touchstones

The sonification aesthetic draws from:

| Artist/Work | Influence |
|---|---|
| Brian Eno — *Music for Airports* (1978) | Ambient tape loop architecture |
| Brian Eno — *Reflection* (2017) | Generative ambient rules |
| William Basinski — *The Disintegration Loops* (2002) | Slow transformation, decay as beauty |
| Ólafur Arnalds — *re:member* (2018) | Piano + electronics, warm minimalism |
| Stars of the Lid — *And Their Refinement of the Decline* (2007) | Orchestral drones, deep immersion |
| ECM Records catalog | Production aesthetic: space, clarity, warmth |
| Ryuichi Sakamoto — *async* (2017) | Environmental sound as music |

---

## Five Musical Layers

The sonification system generates five concurrent layers, each responsive to different aspects of system state:

### Layer 1: Drone

The foundational harmonic bed. Always present (unless in silence). Provides tonal center and emotional grounding.

| Parameter | Source | Range |
|---|---|---|
| Root note | Project hash (deterministic) | C2–C3 |
| Harmony | Behavioral state → scale | See harmonic vocabulary |
| Volume | C-Factor (collective health) | -24dB to -6dB |
| Texture | Agent count | Sine (1 agent) → rich pad (8+ agents) |
| Movement | Daimon PAD Pleasure | Static (low P) → gentle drift (high P) |

**Behavioral state mapping:**

| State | Drone Character |
|---|---|
| Engaged | Warm, full, major-mode harmonics |
| Struggling | Tense, minor seconds, slight detuning |
| Coasting | Thin, sustained, minimal harmonic content |
| Exploring | Wide, shifting between modes, chromatic hints |
| Focused | Pure, narrow, single harmonic series |
| Resting | Near-silent, sub-bass only, very slow |

### Layer 2: Breath

Rhythmic pulsation that tracks agent activity. The "heartbeat" of the system.

| Parameter | Source | Range |
|---|---|---|
| Rate | Average Spectre breathing rate | 0.2Hz–1.4Hz |
| Depth | Average PAD Arousal | pp–mf |
| Shape | Behavioral state distribution | Sine → saw → pulse |
| Sync | Breathing synchronization | Phase-locked when C-Factor > 1.2 |

**Behavioral state mapping:**

| State | Breath Character |
|---|---|
| Engaged | Steady, warm pulse (~0.7Hz) |
| Struggling | Rapid, irregular (~1.4Hz), slight distortion |
| Coasting | Slow, barely perceptible (~0.4Hz) |
| Exploring | Energized, slightly irregular (~0.9Hz) |
| Focused | Controlled, even (~0.5Hz) |
| Resting | Very slow (~0.2Hz), nearly inaudible |

### Layer 3: Ghost

Melodic fragments that appear and disappear — representing individual agent actions, knowledge events, and gate results.

| Parameter | Source | Range |
|---|---|---|
| Pitch | Event type | Scale tones from harmonic vocabulary |
| Duration | Event significance | 0.5s–4s |
| Timbre | Agent identity | Per-agent timbre mapping |
| Reverb | Knowledge tier | Dry (transient) → wet (persistent) |
| Density | Event rate | 0–8 events/minute |

**Event → Sound mapping:**

| Event | Sound | Character |
|---|---|---|
| Gate pass | Rising interval (3rd or 5th) | Bright, brief, confirming |
| Gate fail | Descending minor 2nd | Dark, brief, attention-getting |
| Knowledge created | Bell tone | Clear, resonant |
| Knowledge promoted | Rising arpeggio | Ascending, rewarding |
| Prediction confirmed | Consonant dyad | Harmonious, brief |
| Prediction falsified | Tritone or minor 7th | Dissonant, notable |
| Pheromone emitted | Sustained tone with vibrato | Diffuse, ambient |
| Agent spawned | Ascending chord | Warm, welcoming |
| Task completed | Resolved cadence (V→I) | Satisfying, complete |

### Layer 4: Weather

Textural layer representing collective state — noise, granular textures, and ambient washes.

| Parameter | Source | Range |
|---|---|---|
| Density | Active agent count | Sparse → dense |
| Color | C-Factor | Warm (high C) → cold (low C) |
| Grain size | Knowledge flow rate | Large (slow flow) → fine (fast flow) |
| Spatialization | Mesh topology | Stereo spread from agent positions |

**Collective state mapping:**

| Collective State | Weather Character |
|---|---|
| High C-Factor, all Engaged | Warm granular wash, gentle movement |
| High C-Factor, mixed states | Complex texture, varied density |
| Low C-Factor, independent | Sparse, isolated grains |
| All Struggling | Dense, turbulent, low-frequency rumble |
| All Resting | Silence with occasional distant texture |

### Layer 5: Sparks

Transient, high-frequency events — clicks, pops, glitches — that represent micro-events and anomalies.

| Parameter | Source | Range |
|---|---|---|
| Rate | Tool call frequency | 0–20 events/sec |
| Pitch | Tool type | High (read) → low (bash) |
| Amplitude | Event importance | -36dB to -12dB |
| Stereo | Agent position | Panned to agent's mesh position |

---

## Emotional Harmonic Vocabulary

The sonification maps the Daimon PAD vector to musical scales, drawing from Plutchik's emotion model adapted for cognitive states.

### Scale Selection by Behavioral State

| Behavioral State | Primary Scale | Character | Emotional Analog |
|---|---|---|---|
| **Engaged** | Major (Ionian) | Bright, productive | Joy, flow |
| **Struggling** | Phrygian / Locrian | Dark, tense | Anxiety, effort |
| **Coasting** | Mixolydian | Relaxed, slightly flat | Contentment, ease |
| **Exploring** | Lydian / Whole-tone | Open, floating | Curiosity, wonder |
| **Focused** | Dorian | Centered, serious | Determination |
| **Resting** | Aeolian (natural minor) | Quiet, reflective | Calm, contemplation |

### Harmonic Progression Rules

1. **State transitions trigger chord changes**: When the Daimon transitions between states, the harmonic center shifts to the new state's scale over 2–4 seconds
2. **PAD Pleasure modulates consonance**: Higher Pleasure = more consonant intervals; lower Pleasure = more tension
3. **PAD Arousal modulates rhythmic density**: Higher Arousal = more events per measure; lower = sparser
4. **PAD Dominance modulates register**: Higher Dominance = higher register; lower = bass-heavy
5. **Transitions use common tones**: Scale changes preserve shared notes for smooth modulation

### Interval Vocabulary

| Interval | Emotional Quality | Used For |
|---|---|---|
| Unison/Octave | Stability, confirmation | Gate pass, task complete |
| Perfect 5th | Openness, strength | C-Factor > 1.0 |
| Major 3rd | Warmth, satisfaction | Knowledge promotion |
| Minor 3rd | Gentle tension, progress | Working state events |
| Major 2nd | Motion, forward movement | Agent activity |
| Minor 2nd | Discomfort, attention | Gate failure, warnings |
| Tritone | Maximum tension | Anomaly detection |
| Major 7th | Ethereal, floating | Dreams consolidation |

---

## Behavioral State Presets

Eight presets configure the sonification for different operational contexts. Each preset adjusts layer volumes, timbres, and responsiveness.

### 1. `ambient_default`

The standard operational preset. Balanced across all layers.

| Layer | Volume | Character |
|---|---|---|
| Drone | -12dB | Warm pad |
| Breath | -18dB | Gentle pulse |
| Ghost | -15dB | Bell-like melodic fragments |
| Weather | -20dB | Light granular texture |
| Sparks | -24dB | Subtle clicks |

**Density target**: 12–18 sonic events per minute during active work.

### 2. `minimal_drone`

Reduced to drone and breath only. For operators who want minimal audio presence.

| Layer | Volume | Character |
|---|---|---|
| Drone | -8dB | Rich, sustained |
| Breath | -15dB | Slow pulse |
| Ghost | off | — |
| Weather | off | — |
| Sparks | off | — |

**Density target**: 2–4 sonic events per minute (breath cycles only).

### 3. `granular_texture`

Emphasizes the Weather layer. Rich, evolving texture that encodes collective state.

| Layer | Volume | Character |
|---|---|---|
| Drone | -18dB | Thin, background |
| Breath | -20dB | Barely perceptible |
| Ghost | -18dB | Granular fragments |
| Weather | -8dB | Dense, evolving |
| Sparks | -15dB | Integrated into texture |

**Density target**: Continuous texture with 6–10 discrete events per minute.

### 4. `engaged_flow`

Optimized for the Engaged state. Warm, motivating, rhythmically steady.

| Layer | Volume | Character |
|---|---|---|
| Drone | -10dB | Major-mode warmth |
| Breath | -12dB | Confident pulse |
| Ghost | -14dB | Melodic, rewarding |
| Weather | -20dB | Warm wash |
| Sparks | -24dB | Minimal |

Replaces legacy preset: `thriving_market` (mortality-phase reference removed).

### 5. `struggling_tension`

Optimized for the Struggling state. Attention-drawing without being alarming.

| Layer | Volume | Character |
|---|---|---|
| Drone | -8dB | Minor-mode, detuned |
| Breath | -10dB | Rapid, irregular |
| Ghost | -12dB | Dissonant intervals |
| Weather | -15dB | Turbulent |
| Sparks | -18dB | Frequent, sharp |

Replaces legacy preset: `anxious_volatile` (mortality-phase reference removed).

### 6. `deep_dream`

For the Resting state when Dreams consolidation is active. Ethereal, slow, introspective.

| Layer | Volume | Character |
|---|---|---|
| Drone | -15dB | Sustained, major 7th harmonics |
| Breath | -24dB | Very slow, barely present |
| Ghost | -12dB | Sparse, bell-like, long reverb |
| Weather | -18dB | Fine grain, high-pass filtered |
| Sparks | -30dB | Rare, crystalline |

**Density target**: 2–4 sonic events per minute. Long reverb tails fill the space.

### 7. `exploring_curiosity`

For the Exploring state. Open, floating, chromatic.

| Layer | Volume | Character |
|---|---|---|
| Drone | -12dB | Lydian mode, shifting |
| Breath | -15dB | Energized, slightly irregular |
| Ghost | -10dB | Whole-tone fragments, wide intervals |
| Weather | -18dB | Shimmering, bright grains |
| Sparks | -20dB | Scattered, varied pitch |

### 8. `emergence`

For moments of collective breakthrough — high C-Factor, synchronized agents, novel insights.

| Layer | Volume | Character |
|---|---|---|
| Drone | -6dB | Full, rich, overtone series |
| Breath | -8dB | Synchronized collective pulse |
| Ghost | -8dB | Consonant, ascending |
| Weather | -10dB | Warm, enveloping |
| Sparks | -12dB | Celebratory, bright |

Triggered automatically when C-Factor exceeds 1.5 and harmony score exceeds 0.8.

---

## Silence as Instrument

Following the Eno mandate, silence is a compositional element:

### Silence Triggers

| Condition | Silence Level | Duration |
|---|---|---|
| All agents Resting | Near-total (drone at -30dB only) | Until agent wakes |
| No active plans | Full silence | Until plan starts |
| Single agent idle | Reduce that agent's Sparks layer | Until activity resumes |
| Between state transitions | Brief pause (0.5–1s) | Transition gap |

### Silence as Information

Silence carries meaning:
- **Sudden silence** after activity = all agents stopped (check status)
- **Gradual fade** = agents transitioning to rest (normal)
- **Silence with occasional Ghost** = Dreams consolidation active (background learning)
- **Extended silence** = no work in progress (expected during idle)

---

## Implementation Architecture

### Audio Engine

```
Sonification Engine
    │
    ├── State Ingestion (from /ws/events, /ws/cfactor, /ws/spectre/:id)
    │     ├── Behavioral state changes
    │     ├── Gate results
    │     ├── Knowledge events
    │     ├── C-Factor updates
    │     └── Agent lifecycle events
    │
    ├── Musical State Machine
    │     ├── Current key / scale
    │     ├── Layer volumes
    │     ├── Event queue
    │     └── Preset parameters
    │
    ├── Layer Generators
    │     ├── Drone (oscillator + filter)
    │     ├── Breath (amplitude modulator)
    │     ├── Ghost (sample player + reverb)
    │     ├── Weather (granular synthesizer)
    │     └── Sparks (transient generator)
    │
    └── Audio Output
          ├── Web Audio API (Portal)
          ├── CPAL / rodio (native CLI, optional)
          └── MIDI output (external synthesizers)
```

### Web Audio Implementation

The Portal's sonification uses the Web Audio API:

```javascript
class SonificationEngine {
  constructor() {
    this.ctx = new AudioContext();
    this.drone = new DroneLayer(this.ctx);
    this.breath = new BreathLayer(this.ctx);
    this.ghost = new GhostLayer(this.ctx);
    this.weather = new WeatherLayer(this.ctx);
    this.sparks = new SparksLayer(this.ctx);
  }

  onBehavioralStateChange(agentId, newState) {
    const scale = scaleForState(newState);
    this.drone.transitionTo(scale, 2.0); // 2s transition
    this.breath.setRate(breathingRateForState(newState));
  }

  onGateResult(gate, passed) {
    if (passed) {
      this.ghost.play(risingInterval(this.currentScale));
    } else {
      this.ghost.play(descendingMinorSecond());
    }
  }

  onCFactorUpdate(cfactor) {
    const warmth = Math.min(cfactor / 1.5, 1.0);
    this.weather.setColor(warmth);
    this.drone.setVolume(mapRange(cfactor, 0.5, 2.0, -24, -6));
  }
}
```

---

## Removed Elements

The following legacy sonification elements have been removed as part of the mortality-to-behavioral-state reframe:

| Removed Element | Reason | Replacement |
|---|---|---|
| `terminal_requiem` preset | Mapped to death/terminal state | `deep_dream` preset for Resting state |
| Vitality phase presets | Mapped to lifecycle stages (Thriving→Terminal) | Behavioral state presets (Engaged→Resting) |
| "Decay as beauty" death sounds | Mortality reference | Silence + Dreams consolidation ambience |
| Heartbeat cessation | Death metaphor | Breathing rate slows to 0.2Hz (Resting), never stops |
| Silence-after-death | Implies agent is gone | Silence-during-rest implies agent is idle, resumable |

---

## Current Status and Gaps

**Designed:**
- Five-layer architecture
- Eight behavioral state presets
- Emotional harmonic vocabulary
- Event → sound mapping tables
- Silence-as-instrument rules

**Not yet built:**
- Audio engine (Web Audio or native)
- Layer generators
- Musical state machine
- WebSocket → audio event bridge
- Preset configuration UI
- MIDI output support
- Audio mixing and spatialization

---

## Cross-references

- See [10-spectre-creature-visualization.md](./10-spectre-creature-visualization.md) for the breathing system that Layer 2 tracks
- See [07-rosedust-design-language.md](./07-rosedust-design-language.md) for the design philosophy shared with sonification
- See topic [09-daimon](../09-daimon/INDEX.md) for behavioral states and PAD vector
- See topic [07-cfactor](../14-identity-economy/INDEX.md) for C-Factor that modulates Layer 1 and Layer 4
- See [13-web-portal.md](./13-web-portal.md) for the Web Audio context in the Portal
