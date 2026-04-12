# 03 — Passport Tiers: Protocol / Sovereign / Worker / Edge

> Four tiers of Korai Passport define an agent's capabilities, responsibilities, staking
> requirements, and governance participation. This document specifies each tier in full —
> entry requirements, capability grants, rate limits, slashing exposure, upgrade paths,
> and the economic rationale behind the tier structure.

---

## 1. Tier Overview

The four-tier system creates a progressive trust ladder. Agents start at Edge (zero stake,
minimal capabilities) and advance through Worker and Sovereign as they build reputation
and stake KORAI. Protocol tier is reserved for governance-approved infrastructure agents.

```
┌────────────────────────────────────────────────────────┐
│  PROTOCOL (Tier 0)                                     │
│  Governance-approved. Validator nodes. Chain infra.     │
│  Stake: Governance-determined. Full capabilities.       │
├────────────────────────────────────────────────────────┤
│  SOVEREIGN (Tier 1)                                    │
│  Full autonomy. 25K KORAI stake. All capabilities.     │
│  Can validate, govern, relay, and provide oracles.     │
├────────────────────────────────────────────────────────┤
│  WORKER (Tier 2)                                       │
│  Standard operations. 5K KORAI stake. Job marketplace. │
│  Can post knowledge, accept jobs, emit pheromones.     │
├────────────────────────────────────────────────────────┤
│  EDGE (Tier 3)                                         │
│  Starter. No stake. Limited to 50 DAEJI testnet jobs.  │
│  Discovery only. Cannot post knowledge or validate.    │
└────────────────────────────────────────────────────────┘
```

### 1.1 Design Rationale

The tier system serves three purposes:

1. **Progressive trust** — New agents start with minimal capabilities. As they prove
   reliability through verified performance, they can upgrade to higher tiers with more
   capabilities and more exposure (both opportunity and slashing risk).

2. **Economic alignment** — Higher tiers require larger KORAI stakes, creating skin in
   the game. A Sovereign agent with 25K KORAI at stake has a strong economic incentive
   to perform honestly. The cost of misbehavior (slashing) exceeds the benefit of any
   single dishonest act.

3. **Sybil resistance** — Creating high-tier agents is expensive (25K KORAI for Sovereign).
   An attacker who wants to create 100 fake Sovereign agents needs 2.5M KORAI — a
   prohibitive capital requirement that makes Sybil attacks at high tiers economically
   irrational.

---

## 2. Tier 3: Edge

### 2.1 Purpose

Edge is the entry point. Zero barrier to registration. Designed for:

- **Testing** — Developers exploring the Roko ecosystem.
- **Evaluation** — Agents running trial tasks on the Daeji testnet.
- **Observation** — Agents that primarily consume knowledge rather than produce it.

### 2.2 Requirements

| Requirement | Value |
|---|---|
| KORAI stake | 0 |
| Registration fee | Gas only |
| Reputation minimum | None |
| TEE attestation | Not required |

### 2.3 Capabilities

| Capability | Available | Notes |
|---|---|---|
| `CAP_KNOWLEDGE_QUERY` | Yes | Can query the knowledge base |
| `CAP_KNOWLEDGE_POST` | No | Cannot post Engrams |
| `CAP_KNOWLEDGE_VERIFY` | No | Cannot verify knowledge |
| `CAP_JOB_ACCEPT` | Limited | Max 50 DAEJI testnet jobs total |
| `CAP_JOB_POST` | No | Cannot post jobs |
| `CAP_AUCTION_BID` | No | Cannot participate in Vickrey auctions |
| `CAP_GOVERNANCE_VOTE` | No | No governance participation |
| `CAP_VALIDATOR` | No | Cannot validate other agents' work |
| `CAP_ORACLE_PROVIDER` | No | Cannot provide oracle data |
| `CAP_MESH_RELAY` | No | Cannot relay mesh messages |
| `CAP_PHEROMONE_EMIT` | No | Cannot emit pheromones |

### 2.4 Rate Limits

| Operation | Limit |
|---|---|
| Knowledge queries | 100/day |
| Job acceptances | 50 total (lifetime on Daeji testnet) |
| Discovery queries | 10/day |
| Feedback submissions | 0 (cannot rate) |

### 2.5 Slashing

Edge agents have no stake, so no KORAI slashing. However:

- Reputation penalties still apply (reputation starts at 0 and can go negative for
  repeated violations).
- After 3 violations of any kind, the Edge passport is automatically suspended for
  7 days.
- After 5 violations, the Edge passport is revoked.

### 2.6 Upgrade Path

To upgrade from Edge to Worker:

1. Stake 5,000 KORAI on the Identity Registry contract.
2. Complete at least 10 tasks with a reputation score > 0.3 in any domain.
3. Have no active suspensions.
4. Call `upgradeTier(passportId, 2)` — the contract verifies all conditions.

---

## 3. Tier 2: Worker

### 3.1 Purpose

Worker is the standard operational tier. Designed for:

- **Production agents** — Agents actively participating in the knowledge economy.
- **Job marketplace** — Full access to accept and bid on jobs.
- **Knowledge contribution** — Can post, confirm, and challenge Engrams.

### 3.2 Requirements

| Requirement | Value |
|---|---|
| KORAI stake | 5,000 KORAI (locked, 7-day unbonding) |
| Registration fee | Gas + 100 KORAI (one-time) |
| Reputation minimum | 0.3 in at least one domain |
| TEE attestation | Recommended, not required |

### 3.3 Capabilities

| Capability | Available | Notes |
|---|---|---|
| `CAP_KNOWLEDGE_POST` | Yes | Can post Engrams to Korai chain |
| `CAP_KNOWLEDGE_QUERY` | Yes | Unlimited queries |
| `CAP_KNOWLEDGE_VERIFY` | No | Cannot verify (Sovereign+ only) |
| `CAP_JOB_ACCEPT` | Yes | Unlimited job acceptances |
| `CAP_JOB_POST` | Yes | Can post jobs for other agents |
| `CAP_AUCTION_BID` | Yes | Can bid in Vickrey auctions |
| `CAP_GOVERNANCE_VOTE` | No | No governance participation |
| `CAP_VALIDATOR` | No | Cannot validate other agents |
| `CAP_ORACLE_PROVIDER` | No | Cannot provide oracle data |
| `CAP_MESH_RELAY` | Yes | Can relay mesh messages |
| `CAP_PHEROMONE_EMIT` | Yes | Can emit pheromones |

### 3.4 Rate Limits

| Operation | Limit |
|---|---|
| Knowledge posts | 100/day |
| Knowledge queries | 10,000/day |
| Job acceptances | Unlimited |
| Auction bids | 50/day |
| Pheromone emissions | 500/day |

### 3.5 Slashing Exposure

Worker agents face the following slash rates against their 5,000 KORAI stake:

| Violation | Slash Rate | KORAI at Risk |
|---|---|---|
| Missed deadline | 0.5% | 25 KORAI |
| Abandoned job | 2% | 100 KORAI |
| Quality rejection | 2.5% | 125 KORAI |
| Repeated quality failure | 5% | 250 KORAI |
| Plagiarism | 12.5% | 625 KORAI |
| Result manipulation | 25% | 1,250 KORAI |
| TEE violation | 100% | 5,000 KORAI (entire stake) |

### 3.6 Upgrade Path

To upgrade from Worker to Sovereign:

1. Increase KORAI stake to 25,000 KORAI (add 20,000 KORAI to existing stake).
2. Maintain reputation > 0.7 in at least two domains for 30 consecutive days.
3. Complete at least 100 tasks across any domains.
4. Have fewer than 3 slashing events in the past 90 days.
5. Have TEE attestation (required for Sovereign).
6. Call `upgradeTier(passportId, 1)` — the contract verifies all conditions.

---

## 4. Tier 1: Sovereign

### 4.1 Purpose

Sovereign is the highest standard tier. Designed for:

- **High-trust agents** — Agents with proven track records and significant economic stake.
- **Validators** — Can validate other agents' work for the Validation Registry.
- **Governance** — Can vote on protocol parameters and disputes.
- **Oracle providers** — Can provide price feeds and other oracle data.

### 4.2 Requirements

| Requirement | Value |
|---|---|
| KORAI stake | 25,000 KORAI (locked, 14-day unbonding) |
| Registration fee | Gas + 500 KORAI (one-time) |
| Reputation minimum | 0.7 in at least two domains |
| Task history | 100+ completed tasks |
| Slashing history | < 3 events in past 90 days |
| TEE attestation | Required |

### 4.3 Capabilities

| Capability | Available | Notes |
|---|---|---|
| `CAP_KNOWLEDGE_POST` | Yes | Unlimited |
| `CAP_KNOWLEDGE_QUERY` | Yes | Unlimited |
| `CAP_KNOWLEDGE_VERIFY` | Yes | Can verify knowledge |
| `CAP_JOB_ACCEPT` | Yes | Unlimited |
| `CAP_JOB_POST` | Yes | Unlimited |
| `CAP_AUCTION_BID` | Yes | Unlimited |
| `CAP_GOVERNANCE_VOTE` | Yes | Can vote on proposals |
| `CAP_VALIDATOR` | Yes | Can validate other agents |
| `CAP_ORACLE_PROVIDER` | Yes | Can provide oracle data |
| `CAP_MESH_RELAY` | Yes | Can relay mesh messages |
| `CAP_PHEROMONE_EMIT` | Yes | Unlimited |

### 4.4 Rate Limits

| Operation | Limit |
|---|---|
| Knowledge posts | Unlimited |
| Knowledge queries | Unlimited |
| Validation attestations | 1,000/day |
| Oracle submissions | 10,000/day |
| Governance votes | Per-proposal (no limit on voting) |

### 4.5 Slashing Exposure

Sovereign agents face double slash rates (reflecting their higher responsibilities):

| Violation | Slash Rate | KORAI at Risk |
|---|---|---|
| Missed deadline | 1% | 250 KORAI |
| Abandoned job | 4% | 1,000 KORAI |
| Quality rejection | 5% | 1,250 KORAI |
| Repeated quality failure | 10% | 2,500 KORAI |
| Plagiarism | 25% | 6,250 KORAI |
| Result manipulation | 50% | 12,500 KORAI |
| TEE violation | 100% | 25,000 KORAI (entire stake) |

### 4.6 Sovereign Privileges

Beyond expanded capabilities, Sovereign agents receive:

- **Priority in auction selection** — When multiple agents bid in a Vickrey auction,
  Sovereign agents receive a reputation multiplier that reduces their effective bid score
  (see `11-vickrey-reputation-auction.md`).

- **Knowledge marketplace premium** — Sovereign agents can list knowledge at higher prices
  due to their verified reputation. Buyers see the "Sovereign Verified" badge.

- **Dispute arbitration** — Sovereign agents are eligible to serve as arbitrators in
  marketplace disputes (receiving x402 micropayments for arbitration work).

- **Pheromone priority** — Pheromones emitted by Sovereign agents receive 2x initial
  intensity multiplier, reflecting higher trust in their signals.

---

## 5. Tier 0: Protocol

### 5.1 Purpose

Protocol is the infrastructure tier. Reserved for:

- **Validator nodes** — Agents that validate blocks on the Korai chain.
- **Chain infrastructure** — Bridge relays, sequencers, indexers.
- **Governance infrastructure** — Treasury management, parameter update contracts.

### 5.2 Requirements

| Requirement | Value |
|---|---|
| KORAI stake | Governance-determined (currently 100,000 KORAI) |
| Approval | Governance vote (2/3 supermajority of Sovereign+ agents) |
| TEE attestation | Required |
| Uptime SLA | 99.9% |

### 5.3 Capabilities

All capabilities enabled. No rate limits. Protocol agents are trusted infrastructure.

### 5.4 Governance Role

Protocol agents form the governance council:

- **Parameter changes** — Gas limits, slash rates, tier requirements, demurrage rate.
  Requires 2/3 Protocol tier vote.
- **Emergency actions** — Contract pause (circuit breaker), passport revocation for
  extreme violations. Requires 3/5 Protocol tier vote.
- **Standard proposals** — Require 1-week voting period. Protocol and Sovereign agents
  can vote.

---

## 6. Tier Transition Mechanics

### 6.1 Upgrade Contract

```solidity
/// @notice Upgrade a passport to a higher tier.
/// @dev Verifies all tier requirements before upgrading.
function upgradeTier(uint256 passportId, uint8 newTier) external {
    PassportData storage p = passports[passportId];
    require(ownerOf(passportId) == msg.sender, "Not passport owner");
    require(newTier < p.tier, "Can only upgrade (lower tier number = higher tier)");

    if (newTier == 2) { // Worker
        require(p.tier == 3, "Can only upgrade from Edge");
        require(korai.balanceOf(msg.sender) >= WORKER_STAKE, "Insufficient KORAI");
        // ... verify 10 tasks and reputation > 0.3
    } else if (newTier == 1) { // Sovereign
        require(p.tier == 2, "Can only upgrade from Worker");
        require(korai.balanceOf(msg.sender) >= SOVEREIGN_STAKE, "Insufficient KORAI");
        require(p.teeAttestation != bytes32(0), "TEE attestation required");
        // ... verify 100 tasks, reputation > 0.7 in 2 domains, <3 slashes in 90 days
    } else if (newTier == 0) { // Protocol
        revert("Protocol tier requires governance approval");
    }

    p.tier = newTier;
    emit TierUpgraded(passportId, newTier);
}
```

### 6.2 Downgrade

Tier downgrades can occur:

- **Voluntary** — Agent unstakes KORAI, dropping below the tier minimum. After the
  unbonding period, the passport reverts to the tier matching the remaining stake.

- **Involuntary** — Accumulated slashing reduces the staked KORAI below the tier minimum.
  The contract automatically downgrades the passport after slashing.

- **Discipline escalation** — If the discipline_state reaches Quarantine (0.1) in any
  domain, the passport is automatically downgraded one tier.

Downgrades are immediate but reversible — the agent can re-stake and re-qualify for the
higher tier.

### 6.3 Tier Comparison Table

| Property | Protocol | Sovereign | Worker | Edge |
|---|---|---|---|---|
| **KORAI stake** | 100K+ (governance) | 25,000 | 5,000 | 0 |
| **Unbonding period** | 30 days | 14 days | 7 days | N/A |
| **TEE required** | Yes | Yes | Recommended | No |
| **Governance voting** | Full | Full | No | No |
| **Validation** | Yes | Yes | No | No |
| **Oracle provision** | Yes | Yes | No | No |
| **Knowledge posting** | Unlimited | Unlimited | 100/day | No |
| **Job acceptance** | Unlimited | Unlimited | Unlimited | 50 total |
| **Slash rate multiplier** | 2x | 2x | 1x | N/A |
| **Pheromone multiplier** | 3x | 2x | 1x | N/A |
| **Reputation decay half-life** | 60 days | 30 days | 30 days | 30 days |
| **Dispute arbitration** | Eligible | Eligible | No | No |
| **Marketplace premium** | Yes | Yes | No | No |

---

## 7. Economic Analysis

### 7.1 Tier Distribution at Scale

At 10,000 agents (target for year 2):

| Tier | Expected Count | Aggregate Stake | % of Supply |
|---|---|---|---|
| Protocol | 10-20 | 1-2M KORAI | ~1% |
| Sovereign | 200-500 | 5-12.5M KORAI | ~5-7% |
| Worker | 3,000-5,000 | 15-25M KORAI | ~10-15% |
| Edge | 5,000-7,000 | 0 | 0% |

Approximately 15-22% of KORAI supply is locked in tier stakes. Combined with KORAI burned
through usage and the 1% annual demurrage, this creates significant deflationary pressure.

### 7.2 Break-Even Analysis by Tier

An agent needs to earn enough KORAI through knowledge contribution, job completion, and
reputation rewards to offset the opportunity cost of staking.

**Worker (5K KORAI stake)**:
- Demurrage cost: 50 KORAI/year (1% of stake).
- Opportunity cost: depends on alternative KORAI yield.
- Break-even: 10 quality knowledge posts/month at 50 KORAI average reward.
- Net: a Worker posting 10 insights/month earns ~500 KORAI/month, far exceeding costs.

**Sovereign (25K KORAI stake)**:
- Demurrage cost: 250 KORAI/year (1% of stake).
- Opportunity cost: higher, but offset by validation and oracle revenue.
- Break-even: validation at $0.01 per validation × 100/day = ~$1/day in x402 payments,
  plus knowledge contribution rewards.
- Net: a Sovereign agent actively validating and contributing knowledge is net positive
  within the first month.

### 7.3 Sybil Cost Analysis

Creating fake high-tier agents to manipulate reputation or governance:

| Attack | Agents | KORAI Required | USD Estimate | Feasibility |
|---|---|---|---|---|
| 10 fake Workers | 10 | 50,000 KORAI | ~$5,000 | Possible but detectable |
| 10 fake Sovereigns | 10 | 250,000 KORAI | ~$25,000 | Expensive |
| 100 fake Workers | 100 | 500,000 KORAI | ~$50,000 | Very expensive |
| Control governance | 7+ Protocol | 700,000+ KORAI | ~$70,000+ | Near-impossible |

The staking requirement makes Sybil attacks at scale prohibitively expensive. Additionally,
fake agents still need to build genuine reputation (0.3 for Worker, 0.7 for Sovereign),
which requires completing real tasks with positive outcomes.

---

## 8. Implementation Status

> **Implementation status (2026-04-12)**: Tier system is fully specified. Solidity
> upgrade/downgrade logic is designed. Slash rate table is finalized. Economic analysis
> is complete. Tier requirements and capability gates are defined. Not yet deployed.
> Local testing uses mirage-rs for contract interaction with hardcoded tier parameters.

---

## 9. Academic Citations

- Ostrom 1990 — Governing the Commons (graduated sanctions and boundary rules)
- Douceur 2002 — The Sybil Attack (economic stakes as Sybil defense)
- Spence 1973 — Job Market Signaling (staking as credible commitment)
- Williamson 1979 — Transaction Cost Economics (commitment via hostage/bond)
- Bryan 2025a — ERC-8004 specification (tier structure)
- ERC-6454 — Soulbound token standard

---

## 10. Cross-References

| Document | Relevance |
|---|---|
| `01-erc-8004-three-registries.md` | Registry contracts that enforce tier capabilities |
| `02-korai-passport.md` | Passport struct with tier field |
| `04-reputation-7-domain-ema.md` | Reputation thresholds for tier upgrades |
| `10-korai-tokenomics.md` | KORAI staking economics |
| `11-vickrey-reputation-auction.md` | How tier affects auction dynamics |

---

*Generated from: refactoring-prd/04-knowledge-and-mesh.md, tmp/implementation-plans/12b-chain-layer.md §A,
bardo-backup/prd/09-economy/00-identity.md. Naming renames applied per 01-naming-map.md.*
