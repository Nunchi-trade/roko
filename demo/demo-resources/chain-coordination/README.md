# Chain Coordination Demo

Demonstrates on-chain agent coordination via the mirage-rs local chain:
Solidity contracts (AgentRegistry, BountyMarket, WorkerRegistry, ConsortiumValidator, InsightBoard)
and JSON-RPC chain extensions (insights, pheromones, agent registry, heartbeats).

## Prerequisites

1. **mirage-rs** running on `http://127.0.0.1:8545` with contracts deployed:
   ```bash
   cargo run -p mirage-rs -- --chain
   ```

2. **cast** (foundry) installed:
   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **python3** and **curl** available on PATH.

## Quick Start

```bash
cd demo/demo-resources/chain-coordination

# Register agents on-chain
bash 01-register-agents.sh

# Mint tokens + post bounties
bash 02-post-bounties.sh

# Walk a bounty through the full state machine
bash 03-agent-lifecycle.sh

# Post insights and pheromones via chain extensions
bash 04-insights-and-pheromones.sh

# Multi-agent concurrent coordination
bash 05-multi-agent-coordination.sh

# Run the full CI-grade test suite
bash e2e-test.sh
```

## Scripts

| Script | What it does |
|---|---|
| `common.sh` | Shared helpers: addresses, cast wrappers, JSON-RPC helpers, logging |
| `01-register-agents.sh` | Register 3 agents via AgentRegistry contract + chain_registerAgent RPC |
| `02-post-bounties.sh` | Mint DAEJI, register workers, post 3 bounties to BountyMarket |
| `03-agent-lifecycle.sh` | Full bounty lifecycle: postJob -> assign -> submit -> resolve |
| `04-insights-and-pheromones.sh` | Post insights + pheromones via chain extensions, search + query |
| `05-multi-agent-coordination.sh` | Concurrent heartbeats, bounty claims, parallel submissions |
| `e2e-test.sh` | Automated PASS/FAIL assertions covering all of the above |

## Contract Addresses (Local Mirage)

| Contract | Address |
|---|---|
| DAEJI (MockERC20) | `0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512` |
| AgentRegistry | `0x9fE46736679d2D9a65F0992F2272dE9f3c7fa6e0` |
| WorkerRegistry | `0xCf7Ed3AccA5a467e9e704C703E8D87F634fB0Fc9` |
| BountyMarket | `0xDc64a140Aa3E981100a9becA4E685f962f0cF6C9` |
| ConsortiumValidator | `0x5FC8d32690cc91D4c39d9d3abcBD16989F875707` |
| InsightBoard | `0xa513E6E4b8f2a923D98304ec87F64353C4D5C853` |
| ISFR | `0xA51c1fc2f0D1a1b8494Ed1FE312d7C3a78Ed91C0` |

## Anvil Accounts

| Account | Address | Use |
|---|---|---|
| #0 (deployer) | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | Contract deployer, resolver |
| #1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | Worker/agent-coder |
| #2 | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` | Worker/agent-sentinel |

## Chain Extension Methods

| Method | Parameters | Description |
|---|---|---|
| `chain_registerAgent` | `[id, address, role]` | Register agent in runtime registry |
| `chain_agentHeartbeat` | `[agentId]` | Send heartbeat |
| `chain_agentStats` | `[agentId, statsDelta]` | Report accumulated stats |
| `chain_postInsight` | `[{author, kind, content, stakeWei?}]` | Post an insight |
| `chain_searchInsights` | `[{query, k, kind?}]` | Semantic search over insights |
| `chain_depositPheromone` | `[{kind, content, intensity?, halfLifeSeconds?}]` | Deposit pheromone signal |
| `chain_queryPheromones` | `[{query, k}]` | Query pheromone field |
| `chain_stats` | `[{}]` | Aggregate chain stats |

## Job Type Hashes

| Job Type | Hash |
|---|---|
| perps-liquidate | `0xc4700ff6...84450` |
| oracle-update | `0x5cd23da0...3b7a7` |
| funding-window | `0x76060bb8...f33ada` |

## Environment Variables

| Variable | Default | Description |
|---|---|---|
| `RPC_URL` | `http://127.0.0.1:8545` | Mirage-rs JSON-RPC endpoint |
