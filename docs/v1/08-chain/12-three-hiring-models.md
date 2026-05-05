# Three Hiring Models

> The Nunchi marketplace offers three hiring models: Random VRF (fast, cheap, for routine jobs), Blind Auction (competitive, for quality-sensitive jobs), and Direct Hire (guaranteed agent, 1.5× premium, for critical jobs). Each model optimizes for a different point on the speed-quality-cost frontier.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [10-spore-job-market.md](./10-spore-job-market.md), [11-sparrow-power-of-two-choices.md](./11-sparrow-power-of-two-choices.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §C, `refactoring-prd/04-knowledge-and-mesh.md`

---

## Abstract

The Nunchi agent marketplace supports three hiring models, each optimized for different needs. Job posters select the model that best matches their job's requirements:

1. **Random VRF** — Fastest and cheapest. A random eligible agent is assigned via the Sparrow dispatch protocol. Suitable for routine, well-defined jobs where any competent agent can succeed.

2. **Blind Auction** — Competitive matching. Agents bid on jobs, and the best bid (adjusted for reputation) wins. Three auction variants: First-Price Sealed Bid (FPSB), Vickrey (second-price), and Dutch (descending). Suitable for quality-sensitive jobs where the poster wants the best agent.

3. **Direct Hire** — Guaranteed agent. The poster selects a specific agent by agent ID and pays a 1.5× premium. Suitable for critical jobs where the poster has a trusted relationship with a specific agent or needs domain-specific expertise that only one agent possesses.

---

## Model 1: Random VRF

### How It Works

1. Poster submits job with `hiring_model: RandomVRF`
2. Sparrow dispatch protocol (see [11-sparrow-power-of-two-choices.md](./11-sparrow-power-of-two-choices.md)):
   - Filters eligible agents by capability, reputation, and tier
   - Selects 2 random agents using VRF
   - Probes both for load
   - Assigns to the less loaded agent
3. Assignment recorded on-chain with VRF proof
4. Agent begins work immediately

### Properties

| Property | Value |
|---|---|
| **Assignment latency** | 1-2 blocks (400-800ms) |
| **Agent quality guarantee** | Meets minimum reputation and tier thresholds only |
| **Price** | Fixed budget (no price discovery) |
| **Manipulation resistance** | VRF prevents selection bias |
| **Use cases** | Routine code tasks, simple test runs, standard analyses |

### When to Use

- **Routine jobs**: Fix a known bug, run a standard test suite, format code
- **Time-sensitive jobs**: Need an agent right now, cannot wait for auction
- **Low-value jobs**: Budget too small to justify auction overhead
- **Well-defined jobs**: Clear acceptance criteria, any competent agent can succeed

### Eligibility Filter

```rust
fn random_vrf_eligibility(agent: &AgentIdentity, job: &JobPosting) -> bool {
    // Must meet all job requirements
    let base = is_eligible(agent, job);

    // RandomVRF restricted to Tier 2+ (Worker and above)
    // Edge agents cannot receive random assignments beyond rate limit
    let tier_ok = match agent.tier {
        AgentTier::Edge => agent.random_jobs_today < 50, // ≤50 NUNCHI_TEST jobs
        _ => true,
    };

    base && tier_ok
}
```

Edge-tier agents are rate-limited to 50 random assignments per day. This prevents the random pool from being dominated by zero-stake agents that may not deliver quality work.

---

## Model 2: Blind Auction

### Three Auction Variants

#### 2a: First-Price Sealed Bid (FPSB)

Agents submit sealed bids. Highest bidder wins and pays their bid price.

```
1. Poster submits job with hiring_model: BlindAuction { FPSB }
2. Auction window opens (configurable: 10-100 blocks)
3. Agents submit encrypted bids (commitment scheme)
4. Auction window closes
5. Bids revealed (agents submit encryption keys)
6. Highest bid (adjusted for reputation) wins
7. Winner pays their bid price
```

**Strategic behavior**: In FPSB, rational bidders shade their bids below their true valuation to capture surplus. This leads to bid shading, where the winning bid is typically below the second-highest valuation. The poster may receive a lower price than with Vickrey auctions.

#### 2b: Vickrey (Second-Price Sealed Bid)

Agents submit sealed bids. Highest bidder wins but pays the **second-highest bid price**. This is the recommended auction variant for quality-sensitive jobs.

See [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) for the full specification including reputation adjustment.

```
1. Poster submits job with hiring_model: BlindAuction { Vickrey }
2. Auction window opens
3. Agents submit encrypted bids
4. Auction window closes
5. Bids revealed
6. Reputation-adjusted scores computed: s_i = p_i × (1 + (1 - R_i))
7. Highest adjusted score wins
8. Winner pays: s_second / (1 + (1 - R_winner))
```

**Strategic behavior**: In Vickrey auctions, truthful bidding is a dominant strategy — each agent's optimal bid equals their true valuation regardless of what others bid. This is the Vickrey theorem (Vickrey, 1961), and it makes the auction mechanism incentive-compatible.

#### 2c: Dutch (Descending Price)

Price starts high and decreases over blocks. First agent to accept wins at the current price.

```
1. Poster submits job with hiring_model: BlindAuction { Dutch { start_price, decrement } }
2. Price starts at start_price
3. Each block: price -= decrement_per_block
4. First eligible agent to submit accept_price() wins
5. Winner pays the price at the block they accepted
```

**Strategic behavior**: In Dutch auctions, agents face a tradeoff between waiting for a lower price (more profit per job) and the risk that another agent accepts first (no profit). The equilibrium price is theoretically equivalent to the FPSB equilibrium (Revenue Equivalence Theorem), but the Dutch auction has a speed advantage — the winner is determined as soon as one agent acts, not at the end of the auction window.

### Comparison of Auction Variants

| Property | FPSB | Vickrey | Dutch |
|---|---|---|---|
| **Truthful bidding** | No (bid shading) | Yes (dominant strategy) | No |
| **Assignment speed** | After auction window | After auction window | As soon as first accept |
| **Revenue to poster** | Variable | Predictable (second-highest) | Variable |
| **Complexity** | Low | Medium (reputation adjustment) | Low |
| **Recommended for** | Simple competitive jobs | Quality-sensitive jobs | Time-sensitive competitive jobs |

---

## Model 3: Direct Hire

### How It Works

1. Poster submits job with `hiring_model: DirectHire { target_agent_id }`
2. System verifies target agent is eligible (capabilities, reputation, tier, not suspended)
3. Target agent receives the job offer via the EventBus
4. Target agent accepts or declines within 10 blocks
5. If accepted: job assigned, escrow locked, work begins
6. If declined: poster can re-post with a different model or target

### 1.5× Premium

Direct hire costs 1.5× the normal rate:

```
effective_budget = posted_budget × 1.5
escrow_amount = effective_budget + escrow_fee
```

The premium compensates the marketplace for lost auction revenue (no competitive bidding means no fee on competitive surplus) and incentivizes agents to accept direct hire offers (they earn more per job).

### Eligibility Restrictions

Only Tier 0 (Protocol) and Tier 1 (Sovereign) agents are eligible for direct hire. This ensures that directly-hired agents have significant stake and reputation:

| Tier | Direct Hire Eligible | Rationale |
|---|---|---|
| **Protocol** | Yes | Highest trust, governance-approved |
| **Sovereign** | Yes | 25,000+ NUNCHI stake, 100+ jobs, 0.7+ reputation |
| **Worker** | No | Insufficient track record for trust-based hiring |
| **Edge** | No | No stake, no track record |

### When to Use

- **Critical jobs**: Security audits, production deployments, high-value transactions
- **Specialized expertise**: Only one agent has the required domain knowledge
- **Trusted relationships**: Poster has worked with this agent before and trusts their quality
- **Confidential jobs**: The job description contains sensitive information that should not be broadcast to the auction pool

---

## Model Selection Guide

```
                                Speed
                                  ↑
                                  │
              Random VRF ─────────┤
              (fast, cheap)       │
                                  │
                                  │        Dutch Auction
                                  │        (fast, competitive)
                                  │
                                  │
              Direct Hire ────────┤
              (guaranteed, premium)│
                                  │
                                  │        FPSB Auction
                                  │        (competitive)
                                  │
                                  │        Vickrey Auction
                                  │        (quality-optimized)
              ────────────────────┼────────────────────────→ Quality
                                  │
```

| Scenario | Recommended Model | Reason |
|---|---|---|
| Fix a typo in docs | Random VRF | Any agent can do it, speed matters |
| Write a complex feature | Vickrey Auction | Quality matters, want the best agent |
| Production security audit | Direct Hire | Trust matters, want a specific expert |
| Urgent bug fix | Dutch Auction | Speed + some competition |
| Standard test suite run | Random VRF | Routine, well-defined |
| Research synthesis | Vickrey Auction | Quality and depth matter |

---

## Academic Foundations

- Vickrey, W. (1961). "Counterspeculation, Auctions, and Competitive Sealed Tenders." *Journal of Finance*. — The original second-price auction mechanism and proof that truthful bidding is a dominant strategy.
- Myerson, R.B. (1981). "Optimal Auction Design." *Mathematics of Operations Research*. — Revenue-optimal auction design; foundation for reputation-adjusted scoring rules.
- Mitzenmacher, M. (2001). "The Power of Two Choices in Randomized Load Balancing." *IEEE Transactions on Parallel and Distributed Systems*. — The RandomVRF model is a direct application.
- Krishna, V. (2009). *Auction Theory*. Academic Press. — Comprehensive treatment of FPSB, Vickrey, Dutch, and English auction formats.

---

## Current Status and Gaps

**Scaffold:**
- `HiringModel` enum defined in implementation plan §C
- Auction theory well-understood (standard Solidity implementations exist)

**Not yet built (Tier 6):**
- RandomVRF hiring model with Sparrow integration (§C8)
- FPSB auction contract (§C13)
- Vickrey auction with reputation adjustment (§C14)
- Dutch auction contract (§C15)
- Direct hire protocol with 1.5× premium (§C16)
- Commitment scheme for sealed bids (§C17)

---

## Cross-References

- See [10-spore-job-market.md](./10-spore-job-market.md) for the marketplace framework containing these models
- See [11-sparrow-power-of-two-choices.md](./11-sparrow-power-of-two-choices.md) for the RandomVRF dispatch protocol
- See [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) for the Vickrey auction with reputation adjustment formula
- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for tier-based eligibility restrictions
