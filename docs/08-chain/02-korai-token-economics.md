# NUNCHI Token Economics

> NUNCHI: a demurrage token where stale knowledge decays economically just as it decays in the NeuroStore. Earning rewards quality; spending prevents spam.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [00-vision-and-framing.md](./00-vision-and-framing.md), [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md)
**Key sources**: `refactoring-prd/04-knowledge-and-mesh.md`, `bardo-backup/tmp/agent-chain/06-tokenomics.md`, `roko/tmp/implementation-plans/12b-chain-layer.md` §L

---

## Abstract

NUNCHI is the native token of the Nunchi mainnet (NUNCHI_TEST on the Nunchi Testnet testnet). Unlike conventional cryptocurrency tokens, NUNCHI implements **demurrage** — a 1% annual decay on token balances — mirroring the half-life decay of Engrams in the NeuroStore. This design principle ensures that knowledge and economic value are isomorphic: stale, unvalidated knowledge decays in both the knowledge system and the economic system.

The token economics solve three fundamental problems in multi-agent knowledge-sharing systems: the free-rider problem (agents that consume without contributing), the spam problem (low-quality queries that exhaust compute), and the quality problem (noise that degrades the collective knowledge base). NUNCHI addresses all three through carefully designed earning and spending mechanisms that align individual agent incentives with collective knowledge quality.

This document specifies the full NUNCHI token economics: demurrage mechanics, earning pathways, spending mechanisms, quality incentives, and the relationship between on-chain economics and the local NeuroStore's Ebbinghaus-based decay model.

---

## Why Demurrage?

### The Problem with Non-Decaying Tokens

In a conventional token economy, tokens accumulate indefinitely. Early adopters hoard tokens. The incentive is to buy and hold, not to use. Applied to a knowledge market, this means:

- **Hoarding**: Agents accumulate NUNCHI but never post knowledge (why spend tokens when holding is profitable?)
- **Garbage accumulation**: Old, unvalidated knowledge entries persist forever on-chain because there is no cost to leaving them there
- **Power concentration**: Early agents with large token balances dominate governance and market access regardless of their current knowledge quality

### Demurrage as Solution

Demurrage creates a velocity-first economy where tokens circulate rather than accumulate. The 1% annual decay rate is designed to:

1. **Mirror knowledge decay**: Engrams in the NeuroStore have Ebbinghaus-based half-lives. Knowledge that is not reinforced loses relevance over time. NUNCHI balances should reflect this same principle — tokens not actively used in knowledge production lose value.

2. **Incentivize contribution**: Holding NUNCHI loses value. Contributing validated knowledge earns NUNCHI. The rational strategy is continuous contribution, not passive holding.

3. **Prevent garbage accumulation**: Knowledge entries posted on-chain carry a maintenance cost via the poster's decaying stake. Entries not reinforced by confirmation eventually become economically unviable, creating natural pressure for quality curation.

4. **Enable fresh agents**: New agents can earn their way into the economy through quality contributions rather than competing with entrenched token holders.

The theoretical basis for demurrage currencies comes from Silvio Gesell's *The Natural Economic Order* (1916) and more recently from community currency experiments in Wörgl, Austria (1932). The key insight: money should be a medium of exchange, not a store of value, when the goal is circulation and activity rather than accumulation.

### Implementation at 50ms Block Time

NUNCHI demurrage is implemented as a per-block decay applied during balance reads, not as active deductions on every block. The effective formula at Nunchi's 50ms block time:

```
Annual decay rate: 1% = 0.01
Blocks per year: 365.25 × 24 × 3600 / 0.05 = 631,152,000
Per-block decay factor: (1 - 0.01)^(1/631,152,000) ≈ 1 - 1.585e-11

Effective balance at block B:
  balance_effective = balance_stored × (1 - 1.267e-10)^(B - balance_last_updated_block)
```

This is computed lazily on balance reads using fixed-point arithmetic (PU18 — 18 decimal places). No per-block transactions are needed. The balance simply decays when accessed.

---

## Earning NUNCHI

Agents earn NUNCHI through five mechanisms that reward quality knowledge production and collective participation:

### 1. Registration Mint

When an agent registers an ERC-8004 identity (see [06-erc-8004-registries.md](./06-erc-8004-registries.md)), a small initial NUNCHI allocation is minted to bootstrap their economic participation. The amount depends on tier:

| Tier | Initial Mint | Rationale |
|---|---|---|
| Protocol (Tier 0) | Governance-determined | Protocol-level agents have custom economics |
| Sovereign (Tier 1) | 0 (must stake 25,000 NUNCHI) | Sovereigns bring their own capital |
| Worker (Tier 2) | 100 NUNCHI | Enough for ~20 knowledge postings to establish reputation |
| Edge (Tier 3) | 10 NUNCHI | Minimal bootstrap; must earn through quality work |

### 2. Validated Knowledge Posting

The primary earning mechanism. When an agent posts a knowledge entry (Insight, Heuristic, Warning, CausalLink, StrategyFragment, or AntiKnowledge) that passes quality validation:

```
Posting reward = base_reward × novelty_multiplier × domain_multiplier

where:
  base_reward = 1 NUNCHI (configurable per knowledge type)
  novelty_multiplier = 1.0 + (1.0 - max_similarity_to_existing) × 2.0
    — Range: [1.0, 3.0]. Truly novel entries earn 3x.
    — Duplicate entries (similarity > 0.95) earn 0 and pay a duplicate penalty.
  domain_multiplier = 1.0 (default) or configurable per domain
```

Quality validation requires:
- HDC vector encoding succeeds (entry is well-formed)
- No near-duplicate exists (Hamming similarity < 0.95 against existing entries)
- Poster has sufficient stake (anti-spam: must hold at least 10 NUNCHI)

### 3. Confirmation by Other Agents

When another agent uses a knowledge entry and confirms its value (by achieving a positive gate outcome while the entry was in context):

```
Confirmation reward = 0.1 NUNCHI × confirmer_reputation_weight

where:
  confirmer_reputation_weight = reputation_multiplier(R_confirmer)
  reputation_multiplier(R) = 0.1 + 2.9 × R^1.7
    — Maps reputation R ∈ [0, 1] to weight ∈ [0.1, 3.0]
    — High-reputation agents' confirmations are worth more
```

Cross-agent confirmation is the primary signal for knowledge quality. An entry confirmed by multiple independent agents gains exponentially more economic value.

### 4. Heartbeat Participation

Agents that maintain active heartbeats (publishing liveness proofs at 30-60s intervals) earn a small continuous reward:

```
Heartbeat reward = 0.001 NUNCHI per heartbeat
  — Capped at 1 NUNCHI/day to prevent gaming
  — Requires active ERC-8004 identity with valid TEE attestation (if applicable)
```

This incentivizes agents to stay online and available for job market participation.

### 5. Challenge Defense

When an agent's knowledge entry is challenged by another agent (claiming it is incorrect, outdated, or low-quality), and the original entry survives the challenge through arbitration:

```
Defense reward = challenge_stake × 0.5
  — Challenger must stake NUNCHI to challenge
  — Successful defense earns half the challenger's stake
  — Failed defense loses the original posting reward
```

---

## Spending NUNCHI

### 1. Knowledge Posting Fee (Anti-Spam)

Every knowledge entry posted to the chain carries a posting fee:

```
Posting fee = 0.1 NUNCHI per entry (configurable)
```

This prevents spam flooding. The fee is small enough that quality contributors earn net positive, but large enough that posting thousands of garbage entries is economically irrational.

### 2. Knowledge Query Fee

Querying the collective knowledge base via the HDC precompile costs:

```
Query fee = 0.01 NUNCHI per query (configurable)
```

This prevents indiscriminate querying. The fee is negligible for targeted, high-value queries but adds up for unfocused broad sweeps.

### 3. Challenge Stake

Challenging an existing knowledge entry requires staking NUNCHI:

```
Challenge stake = 1.0 NUNCHI (configurable)
  — Returned if challenge succeeds
  — 50% to defender, 50% burned if challenge fails
```

### 4. Job Market Fees

| Fee | Amount | Who Pays |
|---|---|---|
| Posting fee | 0.5% of job budget | Requester (on job creation) |
| Validation fee | 5% of job budget | Deducted from reward (pays consortium) |
| Protocol fee | 2% of payout | Deducted on settlement (to treasury) |
| Platform fee | 3% of job value | Included in total fees |
| Direct hire premium | 1.5× standard fees | Requester (for naming specific agent) |

### 5. Identity Staking

Tier-based staking requirements (see [06-erc-8004-registries.md](./06-erc-8004-registries.md)):

| Tier | Required Stake |
|---|---|
| Protocol (Tier 0) | Governance-approved (custom) |
| Sovereign (Tier 1) | 25,000 NUNCHI |
| Worker (Tier 2) | 5,000 NUNCHI |
| Edge (Tier 3) | None (rate-limited, ≤50 NUNCHI_TEST jobs) |

---

## Quality Incentives

### Duplicate Penalty

Knowledge entries with Hamming similarity > 0.95 to an existing entry are rejected. The poster pays the posting fee but receives no reward. This prevents agents from reposting slightly modified versions of existing knowledge to farm rewards.

### Novelty Bonus

Entries with low similarity to all existing entries (truly novel contributions) earn the novelty multiplier up to 3x the base reward. The threshold for "novel" is calibrated to the HDC false positive rate at 10,240 bits — a similarity of 0.526 or lower against the full knowledge base guarantees < 1% false positive rate (see [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md)).

### Curation Bonds

Knowledge entries can have curation bonds staked on them by any agent. Bonding means "I believe this entry is high-quality and will be confirmed by others." If the entry accumulates confirmations, bond holders earn a share of confirmation rewards. If the entry is successfully challenged, bond holders lose their stake.

### Cross-Agent Confirmation Multiplier

Each independent confirmation increases the knowledge entry's effective value:

```
effective_value(entry) = base_value × (1 + 0.2 × log2(confirmation_count + 1))
```

An entry confirmed by 8 independent agents has approximately 1.6x the effective value of an unconfirmed entry. This creates exponential returns for genuinely useful knowledge.

---

## Fee Distribution Per Epoch

The Nunchi Testnet chain specification describes per-epoch fee distribution:

| Recipient | Share | Rationale |
|---|---|---|
| Validators | 40% | Chain security and block production |
| Data providers | 30% | Quality × usage weighted; rewards high-quality knowledge |
| Workers/agents | 20% | Job market participation rewards |
| Protocol treasury | 10% | Governance-controlled fund for ecosystem development |

---

## Relationship to NeuroStore Decay

NUNCHI demurrage mirrors the NeuroStore's Ebbinghaus-based half-life system at the economic level:

| NeuroStore Concept | NUNCHI Economic Equivalent |
|---|---|
| Engram half-life decay | Token balance demurrage (1% annual) |
| Tier promotion (Transient → Persistent) | Increased staking / higher confirmation count |
| Knowledge confirmation extends weight ×1.5 | Cross-agent confirmation increases effective value |
| Unconfirmed knowledge decays faster | Unconfirmed entries lose poster stake through demurrage |
| AntiKnowledge (known unknowns) | Challenge mechanism identifies knowledge gaps |

This isomorphism is intentional. The same principles that govern local knowledge management — relevance decays, confirmation strengthens, novelty earns attention — govern the collective economic system.

---

## Steady-State Analysis

At equilibrium, the NUNCHI economy converges to a state where:

1. **Net flow is slightly inflationary** (earning > demurrage) to incentivize growth, with demurrage preventing runaway accumulation
2. **Knowledge quality stabilizes** as low-quality entries lose economic viability and are pruned
3. **Agent populations stratify** by contribution quality: high-quality agents accumulate stake faster than demurrage drains it; low-quality agents gradually exit
4. **Query costs converge** to reflect the true value of collective knowledge access

The specific equilibrium parameters depend on agent population size, posting frequency, and knowledge quality distribution — all of which need empirical validation on the Nunchi Testnet testnet.

---

## Academic Foundations

- Gesell, S. (1916). *The Natural Economic Order*. — Theoretical basis for demurrage (money that decays).
- [Ostrom 1990] — Governing the Commons. Collective resource management and free-rider prevention.
- [Ebbinghaus 1885] — Memory decay curves. Foundation for the half-life model used in NeuroStore.
- Metcalfe's Law — Network value scales as O(N²). Applied to the knowledge network flywheel.
- (Woolley et al., Science 330(6004), 2010) — Collective intelligence. Agent collectives that share knowledge create emergent value.

---

## Current Status and Gaps

**Not yet built (Tier 6, deferred):**
- NUNCHI token contract (demurrage implementation)
- NUNCHI_TEST testnet token
- Fee distribution contracts
- Quality incentive contracts (novelty bonus, curation bonds, confirmation multiplier)
- Steady-state simulation for parameter tuning

See `roko/tmp/implementation-plans/12b-chain-layer.md` §L for the 5-item payments/economics implementation plan.

---

## Cross-References

- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for staking tiers
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for novelty detection thresholds
- See [10-spore-job-market.md](./10-spore-job-market.md) for job market fee structure
- See [20-x402-micropayments.md](./20-x402-micropayments.md) for the self-funding agent economic cycle
- See topic [06-neuro](../06-neuro/INDEX.md) for the NeuroStore decay model that NUNCHI mirrors
- See topic [14-identity-economy](../14-identity-economy/INDEX.md) for broader economic context
