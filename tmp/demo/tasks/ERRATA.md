# Task Spec Errata — Critical Fixes

> Generated from a deep audit of all 17 task specs against the actual codebase.
> Every agent MUST read this file before starting any task.
> This file is injected as a preamble by the runner script.

---

## 1. Contract API Corrections

### BountyMarket.postJob — FOUR parameters, not two
```rust
// WRONG (shown in some specs):
market.postJob(spec_hash, bounty).send().await?;

// CORRECT (matches codebase):
market.postJob(spec_hash.into(), bounty_wei, deadline, min_tier).send().await?.watch().await?;
// spec_hash: FixedBytes<32> (from keccak256)
// bounty_wei: U256
// deadline: u64 (unix timestamp)
// min_tier: u8 (1 = Standard)
```

### BountyMarket.resolve — TWO parameters, not one
```rust
// WRONG:
market.resolve(job_id).send().await?;

// CORRECT:
market.resolve(job_id, true).send().await?.watch().await?;
// id: U256, accepted: bool
```

### InsightBoard.post — TWO parameters, not one
```rust
// WRONG:
board.post(content_hash).send().await?;

// CORRECT:
board.post(content_hash.into(), "demo://yield-routing".into()).send().await?.watch().await?;
// contentHash: FixedBytes<32>, uri: String
```

### WorkerRegistry.register — ONE parameter (msg.sender is the worker)
```rust
// WRONG:
registry.register(worker_address, stake).send().await?;

// CORRECT (called from worker's own wallet provider):
let worker_provider = ctx.wallet_provider("worker0")?;
let registry = WorkerRegistry::new(registry_addr, worker_provider);
registry.register(U256::from(STAKE)).send().await?.watch().await?;
// amount: U256 — the stake to bond
```

### AgentRegistry.register — (capabilities: String, passportHash: bytes32), NO constructor args
```rust
// WRONG:
agent_registry.register(worker_address, capabilities_string).send().await?;

// CORRECT:
let agent_reg = AgentRegistry::new(agent_registry_addr, worker_provider);
let passport = keccak256(format!("agent-{}", worker_name).as_bytes());
agent_reg.register("defi-routing,yield-optimization".into(), passport.into()).send().await?.watch().await?;
// capabilities: String, passportHash: FixedBytes<32>
```

### AgentRegistry — NO constructor arguments
The contract has no constructor. Deploy with empty args:
```toml
[[deploy.contracts]]
name = "AgentRegistry"
# No args — default constructor
```

### ConsortiumValidator constructor — (market, workerRegistry), NOT (bountymarket, workerregistry)
```toml
[[deploy.contracts]]
name = "ConsortiumValidator"
args = [
  { type = "address", value = "$contract(WorkerRegistry)" },
  { type = "address", value = "$contract(BountyMarket)" },
]
```
**Solidity constructor**: `constructor(address workerRegistry_, address market_)` — workerRegistry FIRST, then market.

### WorkerRegistry.bondOf — DOES NOT EXIST
There is no `bondOf` view function. Use `getWorker(address)` if available, or check by calling `reputationOf(address)`.

### BountyMarket.State enum values
```
None=0, Open=1, Funded=2, Assigned=3, Submitted=4, Terminal=5
```
NOT `Funded=0, Assigned=1, ...`

### All InsightBoard/BountyMarket IDs are U256, not u64
`nextInsightId()`, `nextJobId()`, `getInsight(id)`, job IDs — all `uint256` / `U256`.

---

## 2. Rust Syntax Corrections

### Token amounts — use `10u128.pow(18)`, NOT `1e18`
```rust
// WRONG (will not compile):
const POSTER_MINT: u128 = 1_000_000 * 1e18 as u128;

// CORRECT:
const POSTER_MINT: u128 = 1_000_000 * 10u128.pow(18);
```

### alloy contract call pattern
```rust
// CORRECT pattern (used everywhere in codebase):
let contract = ContractName::new(contract_addr, provider);
let pending = contract.method(arg1, arg2).send().await?;
pending.watch().await?;
// or for return values:
let result = contract.viewMethod(arg).call().await?;
```

---

## 3. Scenario TOML Fixture Schema

The CORRECT fixture format (matches `FixtureStep` struct with `#[serde(flatten)]`):
```toml
[[fixtures]]
name = "authorize-market"
kind = "contract-call"
contract = "WorkerRegistry"
method = "setAuthorized(address,bool)"
from = "deployer"
args = ["$contract(BountyMarket)", true]
```

NOT the nested `[fixtures]` / `[[fixtures.calls]]` format. Each fixture is a top-level `[[fixtures]]` array entry.

---

## 4. Workspace Dependencies

### Available as workspace deps:
- `tokio-tungstenite = { version = "0.26", features = ["rustls-tls-native-roots"] }` ✓
- `async-trait = "0.1"` ✓
- `reqwest = { version = "0.12", features = ["blocking", "json", "rustls-tls"] }` ✓

### NOT available as workspace deps (must add directly):
- `ratatui` — NOT in workspace. Add as `ratatui = "0.29"` directly to Cargo.toml
- `crossterm` — NOT in workspace. Add as `crossterm = "0.28"` directly to Cargo.toml

---

## 5. File Conflict Zones

These files are modified by multiple tasks. If running tasks in parallel within a batch, be aware:

| File | Modified by |
|------|------------|
| `main.rs` | T1.1, T1.4, T2.1, T2.4, T3.1, T3.6, T3.7 |
| `lib.rs` | T1.4, T2.4, T3.1, T3.7 |
| `bindings.rs` | T2.2, T2.3, T2.5 |
| `scenarios/mod.rs` | T1.2, T2.1 |

**Recommendation**: Run Batch 1 tasks that touch `main.rs` (T1.1, T1.4) sequentially, not in parallel.

---

## 6. getInsight already exists in Solidity

The `InsightBoard.sol` contract ALREADY has `getInsight(uint256 id) -> Insight memory`. It returns the full struct. T2.5 just needs to add the Rust binding — the Solidity is already done.
