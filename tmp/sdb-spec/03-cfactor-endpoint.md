# Checklist: C-Factor, operating frequency, and cost tier endpoints

**Priority**: P1 — demo headline numbers
**Estimated LOC**: ~80 lines
**Source**: `workspace/sdb/unwired-api-functions-spec.md` §10, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

Dashboard has C-Factor card (10 sub-metrics), operating frequency indicator (4 modes), and cognitive cost tier display (T0/T1/T2) — all using hardcoded mock data. The C-Factor struct and computation already exist in `roko-learn`. Just needs HTTP endpoints.

## What already exists

### `crates/roko-learn/src/cfactor.rs`
- `CFactor` struct with `overall: f64`, `components: CFactorComponents`, `agent_contributions: Vec<AgentCFactorContribution>`, `computed_at`, `episode_count`
- `CFactorComponents` has all 10 sub-metrics: `gate_pass_rate`, `cost_efficiency`, `speed`, `information_flow_rate`, `first_try_rate`, `knowledge_growth`, `knowledge_integration_rate`, `task_diversity_coverage`, `convergence_velocity`, `turn_taking_equality`
- `AgentCFactorContribution` with leave-one-out `contribution_score`
- `CFactor::compute(&[Episode])` computes the composite score

### `crates/roko-learn/src/cascade_router.rs`
- Tracks model selection decisions per task (which model tier: T0 Rust/T1 Haiku/T2 Opus)
- `CascadeStage` enum with model tiers

## Files to modify

### 1. `crates/roko-serve/src/routes/learning.rs`

Current routes (line 22):
```rust
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/learning/efficiency", get(efficiency))
        ...
        .route("/learning/gate-thresholds", get(gate_thresholds))
}
```

- [ ] Add C-Factor route: `.route("/learning/cfactor", get(cfactor))`
- [ ] Add handler:
```rust
/// `GET /api/learning/cfactor` — composite C-Factor with all 10 sub-metrics.
async fn cfactor(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    let path = state.workdir.join(".roko/learn/episodes.jsonl");
    // Read recent episodes, compute CFactor
    let episodes = roko_learn::episode_logger::read_episodes(&path)
        .unwrap_or_default();
    let cf = roko_learn::cfactor::CFactor::compute(&episodes);
    Ok(Json(serde_json::to_value(&cf).map_err(|e| ApiError::internal(e.to_string()))?))
}
```

- [ ] Add cascade cost tier route: `.route("/learning/cost-tiers", get(cost_tiers))`
- [ ] Add handler that reads cascade router decisions and computes T0/T1/T2 percentages:
```rust
/// `GET /api/learning/cost-tiers` — T0/T1/T2 distribution and savings.
async fn cost_tiers(State(state): State<Arc<AppState>>) -> Result<Json<Value>, ApiError> {
    // Read routing decisions from .roko/learn/routing-decisions.jsonl
    // Count T0 (Rust FSM, cost=0), T1 (Haiku, cost~$0.45), T2 (Opus, cost~$2.10)
    // Return { t0_pct, t1_pct, t2_pct, t0_cost, t1_cost, t2_cost, total_savings_multiplier }
}
```

### 2. `apps/mirage-rs/src/http_api/agent.rs`

Current `get_agent_stats` (line 108) returns basic stats. Extend with operating frequency.

- [ ] Add operating frequency derivation to `get_agent_stats`:
```rust
// After existing stats JSON, add:
// Derive operating frequency from most recent traces
let frequency = chain.agent_registry.get_traces(&id, 10, 0)
    .map(|(traces, _)| derive_operating_frequency(traces))
    .unwrap_or("reactive".to_string());
// Include in response JSON:
"operating_frequency": frequency,
"frequency_distribution": { ... }
```

- [ ] Add helper function:
```rust
fn derive_operating_frequency(traces: &[AgentTrace]) -> String {
    // Count phase occurrences in recent traces
    // CognitivePhase::Retrieve → reactive
    // CognitivePhase::Reason → deliberative
    // CognitivePhase::Verify → meta_cognitive
    // No dream phase in current enum — map Act with certain patterns → creative
    // Return the dominant mode
}
```

## Response shapes

### `GET /api/learning/cfactor`
```json
{
  "overall": 0.184,
  "components": {
    "gate_pass_rate": 0.947,
    "cost_efficiency": 0.82,
    "speed": 0.75,
    "information_flow_rate": 0.68,
    "first_try_rate": 0.89,
    "knowledge_growth": 0.72,
    "knowledge_integration_rate": 0.65,
    "task_diversity_coverage": 0.71,
    "convergence_velocity": 0.58,
    "turn_taking_equality": 0.80,
    "social_sensitivity": 0.0
  },
  "agent_contributions": [
    { "agent_id": "golem-alpha-7f", "episode_count": 42, "without_agent_overall": 0.16, "contribution_score": 0.024 }
  ],
  "computed_at": "2026-04-14T...",
  "episode_count": 100
}
```

### `GET /api/learning/cost-tiers`
```json
{
  "t0_pct": 80.0,
  "t1_pct": 15.0,
  "t2_pct": 5.0,
  "t0_cost_usd": 0.0,
  "t1_cost_usd": 0.45,
  "t2_cost_usd": 2.10,
  "total_savings_multiplier": 18.4,
  "sample_count": 200
}
```

### `GET /api/agents/{id}/stats` (extended response)
```json
{
  "agent_id": "golem-alpha-7f",
  "...existing fields...",
  "operating_frequency": "deliberative",
  "frequency_distribution": { "reactive": 80, "deliberative": 12, "meta_cognitive": 5, "creative": 3 }
}
```

## Testing

- [ ] `GET /api/learning/cfactor` with no episodes → returns zeroed CFactor
- [ ] `GET /api/learning/cfactor` with episodes → returns computed scores
- [ ] `GET /api/learning/cost-tiers` → returns T0/T1/T2 percentages
- [ ] `GET /api/agents/{id}/stats` → includes `operating_frequency` field
