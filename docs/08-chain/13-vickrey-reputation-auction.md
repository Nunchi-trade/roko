# Vickrey Reputation-Adjusted Auction

> In the Vickrey auction variant, bids are adjusted by agent reputation: `s_i = p_i × (1 + (1 - R_i))`. Higher-reputation agents can win with lower bids. Winner pays the second-highest adjusted score divided by their own adjustment factor. This makes truthful bidding incentive-compatible while rewarding reputation.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [10-spore-job-market.md](./10-spore-job-market.md), [12-three-hiring-models.md](./12-three-hiring-models.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §C, `refactoring-prd/04-knowledge-and-mesh.md`

---

## Abstract

The standard Vickrey (second-price sealed bid) auction assigns jobs to the highest bidder but charges the second-highest bid price. Truthful bidding is a dominant strategy: each agent bids their true valuation regardless of what others bid. However, a standard Vickrey auction treats all bidders equally — a brand-new agent with 0.5 reputation can outbid a proven agent with 0.95 reputation simply by offering more KORAI.

The Korai reputation-adjusted Vickrey auction modifies the scoring rule to incorporate agent quality. Bids are adjusted by a reputation factor that gives higher-reputation agents an advantage. This creates a two-sided incentive: agents are motivated to build reputation (because it makes them more competitive in auctions) and job posters receive better-quality agents (because reputation-adjusted scoring favors proven performers).

---

## Scoring Rule

### Adjusted Score

Each agent's bid is adjusted by their domain reputation:

```
s_i = p_i × (1 + (1 - R_i))
```

Where:
- `s_i` = adjusted score for agent i
- `p_i` = agent i's bid in KORAI
- `R_i` = agent i's reputation in the job's domain, range [0.0, 1.0]

The factor `(1 + (1 - R_i))` ranges from 1.0 (for perfect reputation R=1.0) to 2.0 (for zero reputation R=0.0). This means:

- **Agent with R=1.0**: adjustment factor = 1.0. Their adjusted score equals their raw bid.
- **Agent with R=0.5**: adjustment factor = 1.5. Their adjusted score is 1.5× their raw bid.
- **Agent with R=0.0**: adjustment factor = 2.0. Their adjusted score is 2.0× their raw bid.

**Interpretation**: A low-reputation agent's bid is inflated, making them appear more expensive than they actually are. To compete with a high-reputation agent, a low-reputation agent must bid lower (accept less money) to compensate for their unproven quality.

### Example

Job: "Implement ERC-4626 vault tests" with budget 1,000 KORAI.

| Agent | Reputation (R) | Bid (p) | Adjustment (1 + (1-R)) | Adjusted Score (s) |
|---|---|---|---|---|
| Agent A | 0.90 | 800 KORAI | 1.10 | 880 |
| Agent B | 0.70 | 750 KORAI | 1.30 | 975 |
| Agent C | 0.50 | 600 KORAI | 1.50 | 900 |

Ranking by adjusted score: B (975) > C (900) > A (880).

Agent B wins despite not having the highest raw bid. Agent A has the highest reputation but bid the most. Agent C bid the least but their low reputation inflates their adjusted score.

### Payment Rule

The winner pays the second-highest adjusted score divided by the winner's own adjustment factor:

```
payment_winner = s_second / (1 + (1 - R_winner))
```

In the example above:
- Winner: Agent B (s = 975)
- Second-highest adjusted score: Agent C (s = 900)
- Agent B's adjustment factor: 1.30
- Payment: 900 / 1.30 = **692.31 KORAI**

Agent B bid 750 KORAI but pays only 692.31 KORAI. This is the Vickrey property: the winner pays less than their bid, creating surplus. The amount of surplus depends on the gap between the winner's adjusted score and the second-highest score.

---

## Incentive Properties

### Truthful Bidding

The reputation-adjusted Vickrey auction preserves incentive compatibility. For each agent, bidding their true valuation (the maximum amount they would accept for the job) is a dominant strategy:

**If agent bids higher than true value**: They might win a job that pays less than their cost, resulting in a loss. The payment rule (second-highest adjusted score / winner's factor) means paying more than necessary.

**If agent bids lower than true value**: They might lose a job they could have profitably completed. The payment is not based on their bid but on the second-highest score, so bidding lower does not reduce payment — it only reduces the chance of winning.

**Bidding true value is optimal**: The agent wins exactly when the payment exceeds their cost, and never when it does not.

This analysis holds even with the reputation adjustment, because the adjustment is applied uniformly to scores and payments. The key insight: the adjustment changes the probability of winning but not the strategic incentive to bid truthfully.

### Reputation Incentive

The adjustment factor creates a direct economic incentive to build reputation:

```
Agent with R=0.5 bidding 600 KORAI: adjusted score = 900
Agent with R=0.9 bidding 600 KORAI: adjusted score = 660

The high-reputation agent's bid is more competitive at the same price.
```

Over time, this creates a virtuous cycle:

1. Agent completes jobs well → reputation increases
2. Higher reputation → lower adjustment factor → more competitive bids
3. More competitive bids → wins more jobs
4. More jobs → more opportunities to build reputation

The converse is also true: poor performance → lower reputation → less competitive → fewer jobs → less opportunity to recover. This is intentional — the marketplace rewards quality.

---

## Sealed Bid Commitment

Bids are submitted as cryptographic commitments to prevent front-running:

### Commit Phase

During the auction window, agents submit bid commitments:

```rust
pub struct BidCommitment {
    /// Hash of (bid_amount, salt, passport_id, job_id).
    pub commitment: [u8; 32],

    /// Passport ID of the bidding agent.
    pub bidder_passport_id: u256,

    /// Block number of commitment.
    pub committed_at_block: u64,
}

fn compute_commitment(bid: U256, salt: [u8; 32], passport_id: u256, job_id: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bid.to_be_bytes());
    hasher.update(salt);
    hasher.update(passport_id.to_be_bytes());
    hasher.update(job_id);
    hasher.finalize().into()
}
```

### Reveal Phase

After the auction window closes, a reveal window opens (typically 10 blocks). Agents submit their actual bid and salt:

```rust
pub struct BidReveal {
    pub bid_amount: U256,
    pub salt: [u8; 32],
    pub passport_id: u256,
    pub job_id: [u8; 32],
}
```

The contract verifies that `hash(bid_amount, salt, passport_id, job_id) == commitment`. If verification fails, the commitment is discarded.

### Non-Reveal Penalty

Agents who commit but do not reveal face a penalty:

- Forfeiture of a small deposit (1% of job budget, escrowed at commit time)
- -0.02 reputation penalty in the job's domain
- 3 consecutive non-reveals → 24-hour ban from auction participation

This prevents strategic non-revealing (committing to observe other bids, then deciding not to participate).

---

## Edge Cases

### Single Bidder

If only one agent bids, the auction reduces to a take-it-or-leave-it offer:

- The single bidder wins
- Payment: the minimum of their bid and the job's posted budget
- No surplus capture for the poster

### All Bids Exceed Budget

If all adjusted scores exceed the job's budget:

- Auction fails
- Budget returned to poster minus escrow fee
- Job can be re-posted with a higher budget or different hiring model

### Identical Adjusted Scores

Ties are broken by:
1. Lower raw bid (cheaper agent wins)
2. Higher reputation (proven agent wins)
3. Lower passport ID (deterministic tiebreaker)

---

## Relationship to Standard Auction Theory

The reputation-adjusted Vickrey auction is a special case of the **virtual valuation** framework from optimal auction design (Myerson, 1981). In Myerson's framework, the auctioneer transforms each bidder's value by a monotone function that accounts for asymmetric bidder types. The adjustment factor `(1 + (1 - R_i))` is a reputation-based virtual valuation transformation.

The Revenue Equivalence Theorem (Myerson, 1981) states that under certain regularity conditions, all standard auction formats yield the same expected revenue. The reputation adjustment breaks these conditions (bidders are asymmetric), so revenue equivalence does not hold. The Vickrey format is specifically chosen because it preserves truthful bidding under asymmetric conditions — a property that FPSB and Dutch auctions do not guarantee with heterogeneous bidders.

---

## Academic Foundations

- Vickrey, W. (1961). "Counterspeculation, Auctions, and Competitive Sealed Tenders." *Journal of Finance*. — The original second-price auction and proof of truthful bidding as dominant strategy.
- Myerson, R.B. (1981). "Optimal Auction Design." *Mathematics of Operations Research*. — Revenue-optimal mechanism design with asymmetric bidders; the virtual valuation framework that the reputation adjustment extends.
- Clarke, E.H. (1971). "Multipart Pricing of Public Goods." *Public Choice*. — The VCG mechanism (Vickrey-Clarke-Groves) that generalizes second-price auctions to multi-item settings.

---

## Current Status and Gaps

**Scaffold:**
- Scoring formula defined in implementation plan §C
- Commitment scheme uses standard SHA-256

**Not yet built (Tier 6):**
- Vickrey auction contract with reputation adjustment (§C14)
- Commit-reveal bid scheme (§C17)
- Non-reveal penalty enforcement (§C18)
- Integration with Reputation Registry for real-time R_i lookup (§C19)
- Multi-item Vickrey generalization for consortium jobs (§C20)

---

## Cross-references

- See [12-three-hiring-models.md](./12-three-hiring-models.md) for how the Vickrey auction relates to the other two hiring models
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for how domain reputation (R_i) is computed
- See [10-spore-job-market.md](./10-spore-job-market.md) for the marketplace framework containing the auction
- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for the Reputation Registry that provides R_i values
