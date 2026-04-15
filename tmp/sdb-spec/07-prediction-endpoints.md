# Checklist: Mirofish prediction endpoints on mirage-rs

**Priority**: P1 — core product, lead demo feature
**Estimated LOC**: ~400 lines (new module)
**Source**: `workspace/sdb/mirofish-implementation-spec.md`, `workspace/sdb/prds/predictions-prd.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

Mirofish (prediction engine) has a full dashboard UI with simulated lifecycle but no backend. Users submit falsifiable prediction questions, agents register claims on-chain before the outcome, EVM grades them. This is the first deflationary mechanism in the points economy.

## What already exists that helps

- `crates/roko-learn/src/prediction.rs`: `PredictionRecord` + `CalibrationTracker` for routing calibration. `CalibrationTracker` implements `mean_bias()`, `coverage_rate()`, `recent_accuracy()`, `accuracy_trend()`, `adjust_prediction()` — architecturally identical to what Mirofish needs. Can factor out a shared `ResidualTracker<K>` generic, or just reuse the pattern.
- `crates/roko-neuro/src/knowledge_store.rs`: Knowledge entries with HDC search, temporal decay, confirmation boost — PredictionClaim is a knowledge entry with extra fields.
- `apps/mirage-rs/src/chain/task.rs`: `TaskStore` with `stake_wei` on tasks — points burn can piggyback.
- `apps/mirage-rs/src/http_api/mod.rs`: Existing router pattern for adding new route groups.

## Files to create/modify

### 1. New file: `apps/mirage-rs/src/chain/prediction.rs`

- [ ] Create structs:

```rust
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

pub type SessionId = String; // hex, 32 chars
pub type ClaimId = String;   // hex, 32 chars

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Dispatching, Collecting, Registered, Pending, Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimState {
    Registered, Pending, Resolved, Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionSession {
    pub id: SessionId,
    pub question: String,
    pub creator: String,
    pub staked_points: u64,
    pub target_block: u64,
    pub category: String,
    pub context: String,
    pub metric: String,
    pub state: SessionState,
    pub claims: Vec<ClaimId>,
    pub consensus_value: Option<f64>,
    pub consensus_confidence: Option<f64>,
    pub outcome: Option<f64>,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionClaim {
    pub id: ClaimId,
    pub agent_id: String,
    pub session_id: SessionId,
    pub predicted_value: f64,
    pub interval_width: f64,
    pub confidence: f64,
    pub entries_in_context: Vec<String>,
    pub registered_block: u64,
    pub state: ClaimState,
    pub actual_value: Option<f64>,
    pub residual: Option<f64>,
    pub covered: Option<bool>,
    pub difficulty_weight: f64,
    pub created_at: u64,
}

#[derive(Debug, Default)]
pub struct PredictionStore {
    sessions: HashMap<SessionId, PredictionSession>,
    claims: HashMap<ClaimId, PredictionClaim>,
    next_session_id: u64,
    next_claim_id: u64,
}
```

- [ ] Implement `PredictionStore` methods:
  - `create_session(question, creator, staked_points, target_block, category, context, metric, now) -> SessionId`
  - `submit_claim(session_id, agent_id, predicted_value, interval_width, confidence, entries_in_context, block, now) -> Result<ClaimId, PredictionError>` — compute `difficulty_weight` here
  - `resolve_session(session_id, actual_value, now) -> Result<ResolveResult, PredictionError>` — compute residuals, update claim states, compute consensus
  - `get_session(id) -> Option<&PredictionSession>`
  - `list_sessions(state_filter, creator_filter, limit, offset) -> (Vec<&PredictionSession>, usize)`
  - `get_claim(id) -> Option<&PredictionClaim>`
  - `list_claims(session_filter, agent_filter, limit, offset) -> (Vec<&PredictionClaim>, usize)`
  - `calibration_summary(agent_id) -> CalibrationSummary` — per-category mean_bias, coverage_rate, sample_count

- [ ] Implement difficulty weight formula:
```rust
fn compute_difficulty(interval_width: f64, sample_count: u64, domain_stddev: f64) -> f64 {
    let category_variance = (domain_stddev).clamp(0.05, 1.0);
    let novelty = (10.0 / (sample_count as f64)).max(0.1).min(1.0); // floor at 0.1
    let tightness = (1.0 - interval_width / (3.0 * domain_stddev)).clamp(0.0, 1.0);
    category_variance * novelty * tightness
}
```

### 2. `apps/mirage-rs/src/chain/mod.rs`

- [ ] Add `pub mod prediction;` and re-export types
- [ ] Add `prediction_store: PredictionStore` to `ChainContext`
- [ ] Add `prediction_bus: BroadcastBus<PredictionEvent>` to `ChainContext` (follow pattern of `task_bus`)

### 3. New file: `apps/mirage-rs/src/http_api/prediction.rs`

- [ ] Create 7 endpoint handlers:

| Handler | Method | Path | Description |
|---------|--------|------|-------------|
| `create_session` | POST | `/api/predictions/sessions` | Create prediction session |
| `list_sessions` | GET | `/api/predictions/sessions` | List sessions with filters |
| `get_session` | GET | `/api/predictions/sessions/{id}` | Get session with claims |
| `resolve_session` | POST | `/api/predictions/sessions/{id}/resolve` | Resolve with outcome |
| `submit_claim` | POST | `/api/predictions/claims` | Agent submits claim |
| `list_claims` | GET | `/api/predictions/claims` | List claims with filters |
| `get_calibration` | GET | `/api/predictions/calibration/{agent_id}` | Agent calibration profile |

### 4. `apps/mirage-rs/src/http_api/mod.rs`

- [ ] Add `mod prediction;`
- [ ] Add routes:
```rust
// Predictions
.route("/predictions/sessions", get(prediction::list_sessions).post(prediction::create_session))
.route("/predictions/sessions/{id}", get(prediction::get_session))
.route("/predictions/sessions/{id}/resolve", post(prediction::resolve_session))
.route("/predictions/claims", get(prediction::list_claims).post(prediction::submit_claim))
.route("/predictions/calibration/{agent_id}", get(prediction::get_calibration))
```

### 5. WebSocket events

- [ ] Add `PredictionEvent` enum to `apps/mirage-rs/src/chain/prediction.rs`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PredictionEvent {
    SessionCreated { session_id: String, question: String },
    ClaimSubmitted { session_id: String, agent_id: String, confidence: f64 },
    SessionRegistered { session_id: String, claim_count: usize },
    SessionResolved { session_id: String, consensus_residual: f64 },
}
```

- [ ] Wire `prediction_bus` into WS handler (follow pattern of `pheromone_bus` in `ws.rs`)

## Testing

- [ ] Create session → returns session_id, state = "dispatching"
- [ ] Submit claim for session → returns claim_id with difficulty_weight
- [ ] Submit claim for nonexistent session → returns 404
- [ ] Resolve session with outcome → computes residuals, updates claim states
- [ ] Resolve already-resolved session → returns 409
- [ ] Get calibration for agent with resolved claims → returns mean_bias, coverage_rate
- [ ] WS subscriber receives prediction events
