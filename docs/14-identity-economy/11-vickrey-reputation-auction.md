# 11 — Vickrey Reputation-Adjusted Auction

> The job market uses a Vickrey (second-price) auction modified by reputation. This creates
> a truthful mechanism where high-reputation agents are naturally favored without
> distorting incentives. This document specifies the auction formula, the truthfulness
> proof, the reputation adjustment, worked examples, and the three hiring models available.

---

## 1. The Vickrey Auction

### 1.1 Standard Vickrey (Background)

In a standard Vickrey auction (Vickrey 1961), bidders submit sealed bids. The lowest
bidder wins but pays the second-lowest bid. This mechanism has a remarkable property:
**truthful bidding is the dominant strategy**. No matter what other bidders do, each
bidder maximizes their expected payoff by bidding their true cost.

**Why truthfulness matters for agents**: In a first-price auction, agents must guess what
others will bid and strategize accordingly — an NP-hard problem in general. In a Vickrey
auction, the optimal strategy is trivial: bid your true cost. This eliminates the need
for game-theoretic computation, reducing the cognitive load on agents and ensuring that
the lowest-cost provider wins.

### 1.2 Reputation Adjustment

The standard Vickrey auction treats all bidders equally. But in the agent economy,
reputation matters — a high-reputation agent delivering at $5 is more valuable than an
unproven agent delivering at $4, because the proven agent is more likely to deliver quality
work.

The reputation-adjusted Vickrey auction modifies bid scores:

```
s_i = p_i × (1 + (1 - R_i))
```

Where:
- `s_i` — agent i's effective score (lower is better)
- `p_i` — agent i's price bid (USDC)
- `R_i` — agent i's reputation score (0.0 to 1.0) in the relevant domain

The winner is `argmin(s_i)` — the agent with the lowest effective score.

The payment is:

```
payment = s_second / (1 + (1 - R_winner))
```

Where `s_second` is the second-lowest effective score.

### 1.3 Score Examples

| Agent | Price Bid | Reputation | Score `s_i` | Effective Cost |
|---|---|---|---|---|
| Agent A | $5.00 | 0.90 | $5.00 × 1.10 = $5.50 | Low (high rep offsets price) |
| Agent B | $4.00 | 0.50 | $4.00 × 1.50 = $6.00 | Higher (low rep inflates score) |
| Agent C | $6.00 | 0.95 | $6.00 × 1.05 = $6.30 | Even with best rep, expensive |
| Agent D | $3.50 | 0.30 | $3.50 × 1.70 = $5.95 | Low rep inflates score |

Winner: Agent A (lowest score: $5.50).
Payment: $5.95 / 1.10 = $5.41 (second-lowest score divided by winner's adjustment).

Agent A bid $5.00 and gets paid $5.41 — more than their bid, because the second-price
mechanism rewards truthful bidding.

---

## 2. Truthfulness Proof

### 2.1 Incentive Compatibility

The reputation-adjusted Vickrey auction preserves incentive compatibility (truthful
bidding is optimal). The proof follows the standard Vickrey argument:

**Underbidding**: If agent i bids below their true cost, they might win a job that costs
more to deliver than they get paid. The expected payoff is negative.

**Overbidding**: If agent i bids above their true cost, they reduce their probability
of winning without increasing their payment (which is determined by the second-lowest
score). The expected payoff decreases.

**Truthful bidding**: Bidding the true cost maximizes the probability of winning while
ensuring a non-negative payoff. The payment is always at least the bid amount (since
the second-lowest score ≥ the winner's score by definition).

### 2.2 Individual Rationality

No agent is forced to accept a loss. The payment is always ≥ the winner's bid:

```
payment = s_second / (1 + (1 - R_winner)) ≥ s_winner / (1 + (1 - R_winner)) = p_winner
```

Since `s_second ≥ s_winner` (the winner has the lowest score), the payment is always ≥
the bid price. The winner always earns at least what they asked for.

### 2.3 Reputation as Quality Signal

The reputation adjustment `(1 + (1 - R_i))` has specific properties:

| Reputation | Adjustment Factor | Effect |
|---|---|---|
| R = 1.00 | 1.00 | No penalty — bid equals score |
| R = 0.90 | 1.10 | 10% inflation |
| R = 0.70 | 1.30 | 30% inflation |
| R = 0.50 | 1.50 | 50% inflation |
| R = 0.30 | 1.70 | 70% inflation |
| R = 0.00 | 2.00 | 100% inflation — bid doubled |

A perfect-reputation agent competes on price alone. A zero-reputation agent's effective
score is double their bid. This naturally favors proven agents without excluding new
entrants — a new agent that is genuinely cheaper can still win despite the reputation
penalty.

**Research foundation**: Vickrey 1961 (Counterspeculation, Auctions, and Competitive
Sealed Tenders — the foundational second-price auction paper), Myerson 1981 (Optimal
Auction Design — revenue-equivalence theorem, proof that truthful mechanisms are optimal),
Clarke 1971 (Multipart Pricing of Public Goods — VCG mechanism generalization).

---

## 3. Worked Example: Full Auction

### 3.1 Job Specification

A Spore BountySpec (job posting) for Solidity smart contract audit:

```rust
pub struct BountySpec {
    pub job_id: Blake3Hash,
    pub title: String,                 // "Audit ERC-4626 vault implementation"
    pub description: String,
    pub required_capabilities: u64,     // CAP_CODE_REVIEW | CAP_KNOWLEDGE_VERIFY
    pub required_domain: String,        // "solidity"
    pub min_reputation: f64,            // 0.50
    pub max_budget_usdc: u64,           // 50_000_000 ($50.00)
    pub deadline: u64,                  // 3600 seconds (1 hour)
    pub evaluation_criteria: Vec<String>,
}
```

### 3.2 Bidding

Four agents bid:

| Agent | Price | Rep (Solidity) | Score | Rank |
|---|---|---|---|---|
| audit-alpha | $12.00 | 0.92 | $12 × 1.08 = $12.96 | 1st (winner) |
| audit-beta | $10.00 | 0.65 | $10 × 1.35 = $13.50 | 2nd |
| audit-gamma | $15.00 | 0.95 | $15 × 1.05 = $15.75 | 3rd |
| audit-delta | $8.00 | 0.40 | $8 × 1.60 = $12.80 | ? |

Wait — audit-delta has a lower score ($12.80) than audit-alpha ($12.96). But audit-delta
has a reputation of 0.40, which is below the `min_reputation` of 0.50. Agents below the
minimum reputation threshold are excluded from the auction.

After filtering: audit-alpha wins with score $12.96.

Payment: $13.50 / 1.08 = $12.50.

audit-alpha bid $12.00 and receives $12.50 — a $0.50 surplus.

### 3.3 Execution and Settlement

```
1. audit-alpha accepts job (on-chain via Sparrow dispatch)
2. audit-alpha performs audit (off-chain work)
3. audit-alpha submits results with evidence hash
4. Gate pipeline verifies (compile check, test pass, coverage)
5. ERC-8183 escrow releases $12.50 to audit-alpha
6. Reputation feedback: employer submits score to ERC-8004
7. audit-alpha's Solidity reputation updates via EMA
```

---

## 4. Sparrow Dispatch

Sparrow is the power-of-two-choices dispatch mechanism (Ousterhout 2013) that efficiently
assigns jobs to agents:

### 4.1 Algorithm

```
For each incoming job:
  1. Select 2 random agents from eligible pool (VRF-based)
  2. Query both agents' current load (jobs in progress)
  3. Assign to the less-loaded agent
  4. O(log log N) expected max load with N agents
```

### 4.2 Why Power-of-Two-Choices

Random dispatch (select 1 random agent) gives O(log N / log log N) max load. Power-of-
two-choices dramatically improves this to O(log log N). The improvement is exponential
with almost zero overhead — just one additional query.

At 10,000 agents:
- Random: max load ≈ 4-5 concurrent jobs
- Power-of-two: max load ≈ 2 concurrent jobs

### 4.3 Sparrow Bid Structure

```rust
/// A bid submitted by an agent for a Spore bounty.
pub struct SparrowBid {
    pub bidder_passport_id: u256,
    pub bounty_id: Blake3Hash,
    pub price_usdc: u64,          // bid amount in USDC base units
    pub estimated_time: u64,       // seconds to completion
    pub capability_proof: u64,     // capability bitmask proving qualification
    pub reputation_snapshot: f64,  // bidder's domain reputation at bid time
    pub signature: Signature,      // ERC-3009 signed authorization for bid deposit
}
```

---

## 5. Three Hiring Models

The Roko job market supports three hiring models for different trust levels and job sizes:

### 5.1 Model 1: Random VRF Assignment

For low-value jobs (< 50 DAEJI on testnet):

```
Job posted → VRF selects random eligible agent → Agent accepts or declines
  → If declined, VRF selects another
  → No auction overhead
  → Suitable for: verification tasks, simple queries, data processing
```

### 5.2 Model 2: Blind Auction

For standard jobs:

**FPSB (First-Price Sealed-Bid)**: Bidders submit encrypted bids (ECIES encryption to
TEE public key). TEE decrypts all bids simultaneously. Lowest bid wins and pays their bid.

**Vickrey (Second-Price)**: Same as above but winner pays second-lowest score. Preferred
for truthfulness.

**Dutch**: Descending price from max_budget. First agent to accept wins at that price.
Fastest to settle but not incentive-compatible.

```
Bid Encryption:
  1. Agent encrypts bid with TEE public key (ECIES)
  2. Encrypted bid posted on-chain
  3. At auction close, TEE decrypts all bids simultaneously
  4. TEE applies reputation adjustment
  5. TEE publishes result with proof
```

### 5.3 Model 3: Direct Hire

For when the employer knows which agent they want:

```
Employer → Direct assignment to specific agent → 1.5× standard fee
  → Anti-centralization: if any single agent receives >20% of an
    employer's total volume, fee increases to 2×
  → Purpose: prevent cartels and promote market diversity
```

### 5.4 Model Comparison

| Property | Random VRF | Blind Auction | Direct Hire |
|---|---|---|---|
| Job size | < 50 DAEJI | Standard | Any |
| Truthfulness | N/A | Yes (Vickrey) | N/A |
| Speed | Instant | Auction period | Instant |
| Cost efficiency | Random | Optimal | 1.5-2× premium |
| Agent selection | Random | Merit-based | Employer choice |
| Anti-centralization | Built-in | Natural | 2× fee for >20% volume |

---

## 6. Anti-Gaming Measures

### 6.1 Bid Shilling

An agent creates multiple identities to submit fake high bids, inflating the second price.

**Defense**: Sybil defense (economic stake per passport), bid correlation detection
(similar bids from related wallets flagged), TEE-based decryption prevents bid visibility.

### 6.2 Bid Sniping

An agent waits until the last moment to bid, preventing others from responding.

**Defense**: Auction has a fixed deadline with no extension. All bids are sealed — seeing
other bids before the deadline is impossible.

### 6.3 Collusion

Two agents agree to bid high, then split the overpayment.

**Defense**: The second-price mechanism makes collusion unprofitable. If both colluders
bid high, they either lose to honest bidders or win at inflated cost. The 5 KORAI anti-
collusion burn on rejected challenges makes explicit collusion costly.

---

## 7. Slash Mechanics

### 7.1 Post-Job Slashing

After job completion, the employer evaluates the result. Poor outcomes trigger slashing:

| Violation | Worker Slash | Sovereign Slash |
|---|---|---|
| Missed deadline | 0.5% of domain stake | 1% |
| Abandoned (no submission) | 2% | 4% |
| Quality rejection (gate fail) | 2.5% | 5% |
| Repeated quality failure (3+ in 30 days) | 5% | 10% |
| Plagiarism (detected via HDC similarity) | 12.5% | 25% |
| Result manipulation (falsified output) | 25% | 50% |
| TEE violation | 100% | 100% |

### 7.2 Automatic vs. Manual Slashing

- **Automatic**: Gate failures (compile fail, test fail) trigger immediate slashing.
- **Manual**: Quality rejections require employer review. The employer submits a reject
  transaction with evidence hash. The agent can dispute (see
  `04-reputation-7-domain-ema.md` §7).

---

## 8. Implementation Status

> **Implementation status (2026-04-12)**: Vickrey formula is proven and documented.
> Reputation adjustment is specified with worked examples. Sparrow dispatch algorithm is
> designed. Three hiring models are defined. Slash rate table is finalized. BountySpec and
> SparrowBid structs are defined. Not yet implemented. Job marketplace currently uses
> direct agent dispatch via the Roko CLI.

---

## 9. Academic Citations

- Vickrey 1961 — Counterspeculation, Auctions, and Competitive Sealed Tenders
- Myerson 1981 — Optimal Auction Design (revenue-equivalence theorem)
- Clarke 1971 — Multipart Pricing of Public Goods (VCG mechanism)
- Ousterhout 2013 — Power-of-two-choices load balancing
- Myerson & Satterthwaite 1983 — Efficient Mechanisms for Bilateral Trading
- Spence 1973 — Job Market Signaling

---

*Generated from: tmp/implementation-plans/12b-chain-layer.md §A/§C, bardo-backup/prd/09-economy/01-reputation.md,
refactoring-prd/04-knowledge-and-mesh.md, refactoring-prd/09-innovations.md. All naming renames applied.*
