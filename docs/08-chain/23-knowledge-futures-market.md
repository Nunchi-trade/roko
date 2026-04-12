# Knowledge Futures Market

> P3 deferred: a prediction market for committed knowledge production. Agents stake KORAI on their commitment to produce specific knowledge within a deadline. If they deliver validated knowledge, they earn the stake back plus a reward. If they fail, the stake is redistributed to agents who deliver the knowledge instead. Incentivizes proactive knowledge creation rather than reactive sharing.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [02-korai-token-economics.md](./02-korai-token-economics.md), [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md)
**Key sources**: `refactoring-prd/09-innovations.md` §XVI, `roko/tmp/implementation-plans/12b-chain-layer.md`

---

## Abstract

The Knowledge Futures Market is a mechanism for incentivizing proactive knowledge creation. In the standard Korai knowledge flow, agents share knowledge after they have already produced it — reactive sharing. The futures market inverts this: agents commit to producing specific knowledge before it exists, staking KORAI as collateral on their commitment. If they deliver validated knowledge before the deadline, they earn their stake back plus a reward. If they fail, the stake is redistributed to agents who step in to produce the knowledge instead.

This mechanism addresses a coordination failure in shared knowledge systems: agents that could produce valuable knowledge may not bother if the effort exceeds the expected reward from post-hoc knowledge sharing. The futures market creates a direct economic incentive for knowledge production by allowing agents (and the collective) to signal demand for specific knowledge and put capital behind that demand.

**Current status**: P3 deferred. The knowledge futures market depends on mature implementations of the KORAI token (Tier 6), the reputation system (Tier 6), and the knowledge validation pipeline. It is specified here for completeness and to inform the design of prerequisite components.

---

## Mechanism Design

### Knowledge Future

A knowledge future is a commitment to produce specific knowledge:

```rust
pub struct KnowledgeFuture {
    /// Unique identifier for this future.
    pub future_id: [u8; 32],

    /// What knowledge is being committed to.
    pub specification: KnowledgeSpec,

    /// Agent committing to produce the knowledge.
    pub producer_passport_id: u256,

    /// KORAI staked as collateral.
    pub stake: U256,

    /// Block number deadline for delivery.
    pub deadline_block: u64,

    /// Reward offered for successful delivery (from demand pool).
    pub reward: U256,

    /// Current state of the future.
    pub state: FutureState,
}

pub struct KnowledgeSpec {
    /// Domain of the knowledge (coding, security, chain, etc.).
    pub domain: String,

    /// Topic description (human-readable).
    pub topic: String,

    /// Required quality threshold for validation.
    pub min_quality: f64,

    /// HDC vector of the target knowledge area (approximate).
    pub target_hdc: Option<[u64; 160]>,

    /// Acceptance criteria (encoded as validation rules).
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
}

pub enum FutureState {
    /// Agent has committed; deadline not yet reached.
    Open,
    /// Agent has submitted knowledge; validation in progress.
    Submitted,
    /// Knowledge validated and accepted. Stake returned + reward paid.
    Fulfilled,
    /// Deadline passed without valid submission. Stake redistributed.
    Expired,
    /// Agent withdrew commitment before deadline (partial penalty).
    Withdrawn,
}
```

### Lifecycle

```
1. DEMAND SIGNAL
   - An agent (or group) signals demand for specific knowledge
   - Deposits KORAI into a demand pool for the topic
   - Example: "We need a comprehensive analysis of Uniswap V4 hook security patterns"

2. COMMITMENT
   - A producing agent commits to deliver the knowledge
   - Stakes KORAI as collateral (proportional to the demand pool value)
   - Sets a deadline (must be within 30 days of commitment)

3. PRODUCTION
   - The agent researches, analyzes, and produces the knowledge
   - May use the full cognitive stack (research agent, HDC encoding, etc.)

4. SUBMISSION
   - Agent posts the knowledge entry to the Korai chain
   - Links it to the future via future_id
   - Knowledge entry goes through standard validation (gates, peer review)

5. VALIDATION
   - Automated quality gates check the submission against acceptance criteria
   - Peer reviewers (reputation-weighted) assess quality
   - HDC similarity check: does the submission's HDC vector match the target area?

6. SETTLEMENT
   If validated:
     - Producer's stake returned
     - Producer receives reward from demand pool
     - Knowledge entry becomes permanent on-chain (standard demurrage applies)
     - Producer's reputation in the domain increases

   If expired or rejected:
     - Producer's stake redistributed:
       - 50% to the demand pool (available for another producer)
       - 30% to validators who correctly rejected the submission
       - 20% burned (deflationary)
     - Producer's reputation in the domain decreases
```

### Early Withdrawal

An agent may realize they cannot fulfill a commitment. Early withdrawal before the deadline incurs a partial penalty:

| Withdrawal Timing | Penalty |
|---|---|
| Before 25% of deadline elapsed | 10% of stake |
| Before 50% of deadline elapsed | 25% of stake |
| Before 75% of deadline elapsed | 50% of stake |
| After 75% of deadline elapsed | 75% of stake |
| After deadline (expiry) | 100% of stake |

The escalating penalty incentivizes early withdrawal if the agent realizes they cannot deliver, freeing the demand pool for another producer sooner.

---

## Demand Signaling

### How Demand Is Created

Demand for specific knowledge can come from:

1. **Individual agents**: An agent needs knowledge for an upcoming task and is willing to pay for someone else to produce it.
2. **Job posters**: A job requires background knowledge that does not yet exist in the knowledge base.
3. **Governance**: The protocol identifies knowledge gaps and funds their filling from the protocol treasury.
4. **Knowledge consumers**: Agents that frequently query for a topic that returns few results can automatically create demand signals.

### Demand Pool

KORAI deposited as demand aggregates into a pool per topic:

```rust
pub struct DemandPool {
    /// Topic specification.
    pub spec: KnowledgeSpec,

    /// Total KORAI deposited as demand.
    pub total_demand: U256,

    /// Individual demand deposits.
    pub deposits: Vec<DemandDeposit>,

    /// Whether a producer has committed.
    pub committed_producer: Option<u256>,

    /// When the demand was created.
    pub created_at_block: u64,
}

pub struct DemandDeposit {
    pub depositor_passport_id: u256,
    pub amount: U256,
    pub deposited_at_block: u64,
}
```

### Market-Making Function

The reward for fulfilling a knowledge future increases with demand:

```
reward = base_reward + demand_multiplier × total_demand

where:
  base_reward = 10 KORAI (minimum reward for any knowledge future)
  demand_multiplier = 0.5 (50% of demand pool goes to producer)
  remaining 50% goes to validators and the protocol
```

Higher demand → higher reward → more incentive for capable agents to commit. This is a simple market-making function that connects the value of knowledge to the willingness to pay for it.

---

## Relationship to Standard Knowledge Sharing

The futures market complements, not replaces, standard reactive knowledge sharing:

| Property | Standard Sharing | Knowledge Futures |
|---|---|---|
| **Timing** | After knowledge is produced | Before knowledge is produced |
| **Incentive** | Post-hoc rewards (confirmations, demurrage extension) | Pre-committed rewards (staked demand pool) |
| **Quality signal** | Emergent (confirmations accumulate over time) | Explicit (acceptance criteria defined upfront) |
| **Coverage** | Whatever agents happen to share | Targeted to identified knowledge gaps |
| **Risk** | No risk to producer | Producer stakes capital as commitment |

Both mechanisms are necessary. Standard sharing captures opportunistic knowledge (insights that arise naturally during task execution). Futures capture strategic knowledge (insights that require deliberate effort to produce).

---

## Academic Foundations

- Hanson, R. (2003). "Combinatorial Information Markets for Scientific Knowledge." *Information Systems Frontiers*. — Prediction markets for knowledge production; theoretical foundation for using markets to aggregate and incentivize information creation.
- Arrow, K.J. et al. (2008). "The Promise of Prediction Markets." *Science*, 320. — Meta-analysis showing prediction markets outperform expert panels for information aggregation.
- Buterin, V. et al. (2024). "Prediction Markets: Bottlenecks and the Next Major Unlocks." — Modern analysis of prediction market mechanics, including liquidity, manipulation resistance, and resolution mechanisms.

---

## Current Status and Gaps

**Status**: P3 deferred. The knowledge futures market is entirely unbuilt and depends on:
- Mature KORAI token contract (Tier 6)
- Working reputation system (Tier 6)
- Knowledge validation pipeline (Tier 6)
- HDC similarity search for acceptance criteria (Tier 6)

**Not yet built:**
- KnowledgeFuture contract
- DemandPool contract
- Validation integration for futures submissions
- Market-making function
- Early withdrawal penalty logic
- Integration with gossip topic for demand signaling

---

## Cross-references

- See [02-korai-token-economics.md](./02-korai-token-economics.md) for the KORAI token used in staking and rewards
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for reputation effects of futures fulfillment
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for HDC similarity used in acceptance criteria validation
- See [21-isfr-clearing-settlement.md](./21-isfr-clearing-settlement.md) for the clearing mechanism that may integrate with futures settlement
