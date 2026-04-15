# Checklist: Add `owner` field to agent registration

**Priority**: P0 — unblocks 3 dashboard specs
**Estimated LOC**: ~40 lines
**Source**: [GitHub #45](https://github.com/Nunchi-trade/collaboration/issues/45), `workspace/sdb/mock-data-audit.md`, `workspace/sdb/agent-messaging-architecture.md`

## Problem

The dashboard Ask panel has a hardcoded `MY_AGENTS` array because there's no way to query "which agents belong to this user." Agent registration (`POST /api/agents`) has no `owner` field. The dashboard needs `GET /api/agents?owner={wallet}` to show "Your Agents" vs "Network" in the agent selector.

## Files to modify

### 1. `apps/mirage-rs/src/chain/agent.rs`

Current `AgentEntry` struct (line 69):
```rust
pub struct AgentEntry {
    pub id: String,
    pub address: Vec<u8>,
    pub role: String,
    pub registered_at: u64,
    pub last_heartbeat_block: u64,
    pub last_heartbeat_ts: u64,
    pub stats: AgentStats,
}
```

- [ ] Add `pub owner: String` field to `AgentEntry` (wallet address hex string, e.g. `"0x1234..."`)
- [ ] Update `AgentRegistry::register()` (line 136) to accept `owner: String` parameter and store it
- [ ] Add `pub fn list_agents_by_owner(&self, owner: &str) -> Vec<&AgentEntry>` method to `AgentRegistry`
- [ ] Update tests at bottom of file to include `owner` in register calls

### 2. `apps/mirage-rs/src/http_api/agent.rs`

Current `RegisterAgentRequest` (line 137):
```rust
pub struct RegisterAgentRequest {
    pub id: String,
    #[serde(default)]
    pub pubkey: String,
    pub role: String,
}
```

- [ ] Add `#[serde(default)] pub owner: String` to `RegisterAgentRequest`
- [ ] Pass `req.owner` to `chain.agent_registry.register()` in `register_agent()` handler (line 167)
- [ ] Include `"owner": req.owner` in the success JSON response

Current `list_agents()` (line 29):
```rust
pub async fn list_agents(State(state): State<ApiState>) -> impl IntoResponse {
    let chain = state.chain.read();
    let agents: Vec<_> = chain.agent_registry.list_agents()...
```

- [ ] Add `owner` query param: create `AgentListQuery` struct with `#[serde(default)] pub owner: Option<String>`
- [ ] If `owner` is set, filter using `list_agents_by_owner()` instead of `list_agents()`
- [ ] Include `"owner": a.owner` in the JSON output for each agent

### 3. `apps/mirage-rs/src/http_api/mod.rs`

Route wiring (line 240):
```rust
.route("/agents", get(agent::list_agents).post(agent::register_agent))
```

- [ ] No change needed — `list_agents` already handles GET and just needs the Query extractor added

## Testing

- [ ] `POST /api/agents` with `{"id": "test-1", "role": "researcher", "owner": "0xabc"}` → returns `owner` in response
- [ ] `GET /api/agents?owner=0xabc` → returns only agents owned by `0xabc`
- [ ] `GET /api/agents` (no owner filter) → returns all agents (backward compatible)
- [ ] Update existing tests in `agent.rs` to pass `owner` parameter

## Dashboard impact

Sam needs to change `MY_AGENTS` hardcoded array to:
```typescript
const { data: agents } = useMirage(() => fetchAgents({ owner: wallet.address }));
```
