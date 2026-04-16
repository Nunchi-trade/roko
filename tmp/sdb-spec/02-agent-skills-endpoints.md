# Checklist: Per-agent skill configuration endpoints on mirage-rs

**Priority**: P0 — unblocks Strategy tab
**Estimated LOC**: ~150 lines
**Source**: `workspace/sdb/agent-skills-config-spec.md`, [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45)

## Problem

The dashboard Strategy tab has per-agent skill toggles (8 skills with config sliders) all in local React state. Needs backend persistence. Sam's spec recommended extending `PUT /api/config` on roko-serve (Option A), but that endpoint writes to a global `roko.toml` file — it's for server config, not per-agent state. Per-agent config must live on mirage-rs where agent state (`AgentEntry`) lives.

## Skill definitions

8 skills from `workspace/sdb/agent-skills-config-spec.md`:

| Skill | Domain | Key params |
|-------|--------|-----------|
| ISFR Observer | Oracle | `divergence_bps` (150), `confidence_threshold` (70), `check_interval_s` (60) |
| DeFi Router | Oracle | `min_improvement_pct` (0.5), `max_slippage_pct` (1.0), `max_routes` (5) |
| Risk Sentinel | Risk | `health_factor_threshold` (130), `confidence_threshold` (70), `check_interval_s` (30) |
| Knowledge Curator | Data | `similarity_threshold` (60), `min_reputation` (50), `max_challenges_per_hour` (10) |
| Prediction Agent | Predictions | `min_confidence` (60), `max_predictions_per_day` (20), `coverage_target` (85) |
| Market Maker | Trading | `max_inventory_eth` (10.0), `base_spread_bps` (5), `gamma` (10) |
| Hedge Agent | Trading | `inventory_threshold` (5.0), `reduce_target_pct` (50), `max_slippage_bps` (10) |
| Self-Tuner | Meta | `experiments_per_hour` (5), `min_improvement_pct` (1.0), `revert_threshold_pct` (-1.0) |

## Files to modify

### 1. `apps/mirage-rs/src/chain/agent.rs`

- [ ] Add `SkillConfig` struct:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    pub enabled: bool,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}
```

- [ ] Add `skills: HashMap<String, SkillConfig>` field to `AgentEntry` (with `#[serde(default)]`)
- [ ] Add `AgentRegistry::get_skills(&self, id: &str) -> Option<&HashMap<String, SkillConfig>>`
- [ ] Add `AgentRegistry::set_skills(&mut self, id: &str, skills: HashMap<String, SkillConfig>) -> bool`
- [ ] Add `AgentRegistry::set_skill(&mut self, id: &str, skill_name: &str, config: SkillConfig) -> bool`

### 2. New file: `apps/mirage-rs/src/http_api/skills.rs`

- [ ] Create this new file with 3 handlers:

```rust
/// GET /api/agents/{id}/skills — return current skill config for an agent
pub async fn get_agent_skills(...) -> Result<Json<Value>, ApiError>

/// PUT /api/agents/{id}/skills — replace all skills for an agent
pub async fn update_agent_skills(...) -> Result<Json<Value>, ApiError>

/// PUT /api/agents/{id}/skills/{skill} — update a single skill
pub async fn update_single_skill(...) -> Result<Json<Value>, ApiError>
```

**Validation rules** (return 422 on violation):
- `gamma_interval_s >= 1` (prevent starvation)
- `divergence_bps >= 0`
- `confidence_threshold` between 0 and 100
- `check_interval_s >= 1`
- No duplicate skill names
- Warn (not reject) on known-bad combos: two skills both managing inventory

**On successful update**: broadcast via `agent_bus` so WS subscribers see skill changes:
```rust
let _ = chain.agent_bus.send(AgentEvent::Stats {
    agent_id: id.clone(),
    delta: AgentStats::default(), // signal change without stat delta
});
```

### 3. `apps/mirage-rs/src/http_api/mod.rs`

- [ ] Add `mod skills;` to the module declarations (around line 10)
- [ ] Add routes after the existing agent routes (after line 251):
```rust
.route("/agents/{id}/skills", get(skills::get_agent_skills).put(skills::update_agent_skills))
.route("/agents/{id}/skills/{skill}", put(skills::update_single_skill))
```

## Request/Response shapes

### `GET /api/agents/{id}/skills`
```json
{
  "agent_id": "golem-alpha-7f",
  "skills": {
    "isfr-observer": { "enabled": true, "config": { "divergence_bps": 150, "confidence_threshold": 70, "check_interval_s": 60 } },
    "risk-sentinel": { "enabled": false, "config": {} }
  }
}
```

### `PUT /api/agents/{id}/skills`
Request body: same shape as `skills` object above.
Response: `{ "ok": true, "agent_id": "...", "skills_updated": 2 }`

### `PUT /api/agents/{id}/skills/{skill}`
Request body: `{ "enabled": true, "config": { "divergence_bps": 200 } }`
Response: `{ "ok": true, "agent_id": "...", "skill": "isfr-observer" }`

## Testing

- [ ] Register agent → GET skills → returns empty map
- [ ] PUT skills with valid config → GET returns saved config
- [ ] PUT single skill → only that skill changes, others untouched
- [ ] PUT with `gamma_interval_s: 0` → returns 422
- [ ] PUT to nonexistent agent → returns 404

## Dashboard impact

Sam needs a NEW `updateSkills()` function pointing at mirage-rs (not the existing `updateConfig()` which points at roko-serve):
```typescript
async function updateSkills(agentId: string, skills: Record<string, SkillConfig>) {
  return fetch(`${MIRAGE_URL}/api/agents/${agentId}/skills`, {
    method: 'PUT',
    body: JSON.stringify(skills),
  });
}
```
