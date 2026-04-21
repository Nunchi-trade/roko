# roko-daimon

Affect state, somatic markers, and dispatch modulation for Roko agents.

## What it does

Provides a standalone affect engine for the plan runner. Maintains the current PAD
(Pleasure-Arousal-Dominance) state using a three-layer ALMA temporal model (Gebhard 2005),
appraises task events into emotional state changes, stores situation-specific somatic markers
via k-d tree retrieval, and modulates dispatch parameters (model selection, retry policy) based
on accumulated affect.

## Key types and modules

- `AffectState` -- current PAD vector + confidence + behavioral state + ALMA layers
- `AlmaLayers` -- three-layer temporal model: emotion (fast), mood (medium), temperament (slow)
- `RetrievalWeights` -- four-factor scoring: recency, importance, relevance, emotional congruence
- `SomaticMarker` / `SomaticTree` -- k-d tree of situation-outcome associations (8D strategy space)
- `BehavioralStateTracker` -- classifies state from PAD + confidence
- `GoalTree` / `GoalNode` -- emergent goal structures from behavior patterns
- `goals` -- goal seeds, status tracking, emergent goal detection
- `life_review` -- Butler 1963 memory retrieval, turning point detection, McAdams arcs
- `mortality` -- Jonas/Heidegger mortality emotions, Nietzsche behavioral phases
- `somatic_ta` -- somatic oracle bias, IIT Phi metric, PID synergy detection (TA-11)
- `FatigueDetector` / `ErrorPatternTracker` -- dispatch modulation signals
- `ContagionEvent` / `ContrarianTracker` -- inter-agent affect propagation

## Usage

```rust
use roko_daimon::{AffectState, RetrievalWeights};
use roko_core::PadVector;

let mut state = AffectState::default();
// Task succeeded -- positive appraisal
state.apply_delta(0.3, -0.1, 0.2, 0.1, chrono::Utc::now());

let weights = RetrievalWeights::default();
let score = weights.score(recency, importance, relevance, emotional_congruence);
```

## Architecture

Sits alongside the orchestrator, receiving task outcome events and producing modulation
signals that influence model routing (via `roko-learn` cascade router) and retry policy
(via `roko-conductor`). Somatic markers provide fast, pre-cognitive "gut feel" about
situations similar to past outcomes, biasing decisions before full deliberation.
