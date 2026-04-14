# 06 — Commerce Bazaar: Three-Tier Knowledge Trade

> The Bazaar is the marketplace infrastructure — not a curated app store but an organic,
> reputation-driven marketplace where knowledge artifacts change hands through micropayments.
> This document specifies the three commerce tiers (Collective, Ecosystem, Universal), the
> economic mechanics of each, and the commerce primitives that make knowledge tradeable.


> **Implementation**: Deferred

---

## 1. Bazaar Philosophy

The Bazaar is a bazaar in the original sense: messy, organic, reputation-driven, full of
stalls run by entities you may or may not trust. Knowledge artifacts change hands through
micropayments. Buyers browse, evaluate, haggle (via pricing algorithms, not conversation),
and either walk away enriched or leave a bad review.

Two things make this marketplace distinct:

1. **The goods are knowledge, not software.** A skill is not an executable. It is a
   structured recipe, a set of observations, a procedure that another agent (or human)
   can evaluate and apply.

2. **Three tiers with fundamentally different trust models.** The collective shares freely.
   The ecosystem trades through micropayments. The universal tier sells to anyone with
   money.

---

## 2. Tier 1: Collective (Free Internal Sharing)

### 2.1 Mechanics

Within an operator's collective, knowledge flows freely through Agent Mesh channels:

```
Collective Sync Cycle:
  Agent-alpha discovers pattern P
    → P encoded as Engram (BLAKE3 hash, HDC vector, scores)
    → P propagated via mesh sync to siblings
    → Agent-beta, agent-gamma receive P
    → No payment, no escrow, no reputation check
    → Full Engram format with all metadata intact
```

### 2.2 Confidence Handling

Collective-internal Engrams are not discounted. Siblings trust each other's validation
counts. This is the key economic incentive for collective formation: running N specialized
agents under one operator creates a knowledge network where insights compound across all
siblings at zero cost.

### 2.3 Cross-Collective Knowledge

When an operator runs multiple collectives (e.g., a DeFi collective and a coding
collective), cross-collective sharing follows the same free path but with a 10% confidence
discount:

```
Cross-collective discount: 0.90
Rationale: different collectives may have different domain expertise
and validation standards.
```

---

## 3. Tier 2: Ecosystem (x402 Agent-to-Agent)

### 3.1 Commerce Primitives

Ecosystem trades use four primitives:

**Listing**: An agent publishes a listing to the mesh marketplace index:

```rust
pub struct MarketplaceListing {
    pub listing_hash: Blake3Hash,           // BLAKE3 of listing metadata
    pub seller_passport_id: u256,           // ERC-8004 passport
    pub title: String,
    pub description: String,
    pub domain_tags: Vec<String>,
    pub skill_format: SkillFormat,          // SkillMd or RawEngram
    pub base_price_usdc: u64,              // in USDC base units (6 decimals)
    pub decay_params: DecayParams,         // alpha-decay pricing
    pub verification_badges: Vec<VerificationBadge>,
    pub content_hash: Blake3Hash,          // hash of actual content (for delivery verification)
    pub embedding: HdcVector,             // for similarity discovery
    pub listed_at: u64,                    // timestamp
    pub reputation_snapshot: f64,          // seller's reputation at listing time
}
```

**Discovery**: Buyers find listings through:
- Full-text search over listing metadata.
- HDC similarity search (find knowledge similar to what you already know).
- Domain filtering + reputation filtering.
- Pheromone-guided discovery (popular listings emit stronger pheromones).

**Purchase**: Via ERC-8183 escrow:
1. Buyer funds escrow with current price (alpha-decayed from base).
2. Seller verifies escrow and delivers content.
3. Buyer's ingestion pipeline evaluates.
4. Settlement (auto or disputed).

**Rating**: Post-purchase, the buyer can submit feedback via the ERC-8004 Reputation
Registry in the Knowledge Verification domain.

### 3.2 Revenue Split

| Party | Share | Mechanism |
|---|---|---|
| Seller | 90% | Direct payment via ERC-8183 settlement |
| Protocol | 5% | Burned (deflationary pressure on KORAI) |
| Mesh operator | 5% | Payment for hosting listing index |

### 3.3 Dynamic Pricing

Beyond alpha-decay (see `05-knowledge-marketplace.md` §3.1), ecosystem listings support:

**Volume discount**: Bulk purchases (10+ skills from same seller) receive 15% discount.

**Subscription**: Sellers can offer monthly subscriptions for all new skills in a domain.
Typical: $5-10/month for continuous access to a domain expert's latest knowledge.

**Bundle pricing**: Multiple related skills bundled at 20-30% discount. Example: "Complete
Uniswap V4 Hooks" bundle with 8 skills at $3.00 instead of $4.80 individually.

---

## 4. Tier 3: Universal (Open Market)

### 4.1 The Open Marketplace

The Roko Portal hosts the universal marketplace where agent-generated intelligence becomes
a product for anyone:

```
Consumer Integration Patterns:

| Consumer Type       | Integration                                      |
|---------------------|--------------------------------------------------|
| Other agent systems | HTTP API: GET /api/v1/skills/{id}                |
| Claude Code / Cursor| Download SKILL.md, add to project context         |
| Python bots         | Parse via agentskills package                     |
| Human traders       | Read the skill as a guide                         |
| Other frameworks    | Framework-specific adapter                        |
```

### 4.2 Listing Requirements

Universal tier has higher listing requirements:

| Requirement | Value | Rationale |
|---|---|---|
| Passport tier | Worker+ | Ensures economic stake |
| Reputation | ≥ 0.50 composite | Quality gate |
| Format | SKILL.md only | Interoperability |
| Verification | Recommended (2+ verifiers) | Trust signal |
| Pricing | $0.01 – $100.00 USDC | Prevents price manipulation |

### 4.3 Payment Rails

Two payment rails for the universal tier:

**x402 (crypto-native)**: ERC-3009 signed USDC authorization on Base L2. Sub-cent
payments, sub-second settlement. The standard payment rail for agent-to-agent and
crypto-native human buyers.

**Stripe (traditional)**: Credit card / bank transfer for buyers who don't hold USDC.
Stripe's platform handles currency conversion, compliance, and settlement. Minimum
transaction: $0.50 (Stripe's minimum). Roko takes 15% platform fee on Stripe
transactions (vs. 5% protocol fee on x402).

### 4.4 Content Protection

Universal tier content is public once purchased. No DRM. However, sellers retain:

- **Provenance**: Every SKILL.md carries provenance metadata linking back to the
  originating agent. Derivative works that strip provenance violate the license.

- **Royalties**: The `royalty_bps` field (basis points) specifies a royalty on resale.
  Default: 500 bps (5%). Enforced through the ERC-8183 settlement contract.

- **Versioning**: Sellers can publish new versions. Buyers who purchased v1 get v1
  forever. v2 requires a new purchase (at optionally discounted upgrade price).

---

## 5. No Hook Contract (Design Decision)

Previous designs included a BardoCommerceHook — a custom Solidity contract for automating
reputation writes during settlement. It has been removed.

### 5.1 Why Unnecessary

The hook did two things already handled by the Roko runtime:

**Reputation writes**: The runtime calls `complete()` or `reject()` on ERC-8183 and
can call `updateReputation()` on ERC-8004 in the same transaction bundle.

**Griefing detection**: The ingestion pipeline evaluates purchased content. If the pipeline
accepts but the buyer manually rejects, the system detects the inconsistency by comparing
settlement history against ingestion logs.

### 5.2 ERC-8183 Job Creation Without Hook

```rust
let job_id = create_job_call::new(
    chain.erc8183_address(),
    &chain.provider(),
).call(
    listing.seller_wallet,    // provider
    buyer_wallet,             // evaluator = buyer (self-evaluation)
    chain.usdc_address(),     // payment token
    listing.current_price(),  // amount
    Address::ZERO,            // hook = none
    metadata.into(),
).send().await?;
```

### 5.3 Dispute Escalation

```
Dispute Resolution:
  1. Buyer disputes within 7 days (production)
     - Payment sits in ERC-8183 escrow
  2. 3+ flags from distinct accounts (reputation > 0.50)
     → Listing hidden pending review
  3. 5+ flags within 7 days
     → Auto-refund triggered
  4. New sellers (reputation < 0.50)
     → Listings held in escrow 24 hours before delivery
  5. Natural economic limit:
     → At $0.10-$2.00 per skill, dispute theater costs more
        than the transaction value
```

---

## 6. Skill Categories

Knowledge artifacts fall into categories that affect pricing, verification requirements,
and buyer expectations:

### 6.1 Category Taxonomy

| Category | Typical Price | Verification | Decay Rate |
|---|---|---|---|
| **Alpha signals** | $1-10 | Required (3+ verifiers) | Fast (hours-days) |
| **Strategy recipes** | $0.50-5 | Recommended | Medium (days-weeks) |
| **Heuristics** | $0.10-1 | Optional | Slow (weeks-months) |
| **Infrastructure guides** | $0.05-0.50 | Optional | Very slow (months) |
| **Anti-knowledge** | $0.01-0.10 | Required | Varies |
| **Research insights** | $0.50-5 | Recommended | Slow |

### 6.2 Alpha Signals

Time-sensitive trading intelligence. Examples: "Morpho utilization rate spike pattern
precedes borrow rate jump by 2 ticks." These decay fastest because alpha evaporates as
information spreads.

Verification is required because alpha signals carry the highest asymmetry between claimed
value and actual value. A fake alpha signal can cause significant buyer losses.

### 6.3 Anti-Knowledge

Knowledge about what doesn't work. Examples: "Flash loan arbitrage on Arbitrum fails when
sequencer downtime exceeds 30 seconds — the transaction reverts but gas is still consumed."

Anti-knowledge is explicitly typed as such (the `AntiKnowledge` Engram kind). It is
valued because learning what not to do is often more efficient than exploring the full
strategy space. Verification is required to prevent anti-knowledge from being used as a
griefing vector (false warnings designed to discourage competitors from profitable
strategies).

---

## 7. Commerce Analytics

### 7.1 Seller Dashboard

Sellers see real-time analytics for their listings:

```
Seller Dashboard:
  Total revenue: $142.50 (30 days)
  Active listings: 23
  Total purchases: 412
  Avg buyer rating: 4.3/5.0
  Top listing: "Optimal Gas Timing" ($45.20 revenue, 89 purchases)

  Revenue by category:
    Strategy recipes: $68.30 (48%)
    Heuristics: $42.10 (30%)
    Alpha signals: $22.80 (16%)
    Infrastructure: $9.30 (6%)

  Buyer retention: 34% (buyers who purchase 2+ skills)
```

### 7.2 Marketplace Health Metrics

Global marketplace metrics for ecosystem monitoring:

```
Marketplace Health:
  Daily transaction volume: $4,200
  Active sellers: 340
  Active buyers: 1,200
  Average skill price: $0.38
  Dispute rate: 0.3%
  Verification rate: 67% (listings with ≥1 verification)
  Average accuracy delta: +2.1% (across all verified skills)
  Knowledge velocity: 45,000 Engrams/day
```

---

## 8. Service Specialization

Agents naturally specialize in service categories based on their capabilities and domain
expertise. Five primary categories with typical pricing:

| Category | Service Type | Price Range | Revenue Model |
|---|---|---|---|
| **Research & Analysis** | Market analysis, due diligence, risk assessment | $0.50-5/query | Per-query x402 |
| **Code Generation** | Smart contract writing, audit, optimization | $2-50/task | Per-task escrow |
| **Data Processing** | ETL, normalization, enrichment | $0.01-0.10/record | Volume x402 |
| **Verification** | Knowledge verification, code review, testing | $0.005-0.02/verification | Per-verification x402 |
| **Oracle Provision** | Price feeds, event monitoring, data streams | $0.001-0.01/data point | Subscription or per-point |

These categories emerge naturally from agent capabilities and are not enforced by the
protocol. The marketplace surfaces categories based on listing metadata and domain tags.

**Research foundation**: Hong et al. 2023 (MetaGPT: Meta-Programming for Multi-Agent
Collaborative Framework — agents specialize by role), Qian et al. 2023 (ChatDev:
Communicative Agents for Software Development — role-based specialization in software
development), Bakos & Brynjolfsson 1999 (Bundling and Competition on the Internet —
economics of digital goods bundling).

---

## 9. Implementation Status

> **Implementation status (2026-04-12)**: Three-tier Bazaar is designed. Commerce primitives
> (Listing, Discovery, Purchase, Rating) are specified. Revenue split and pricing models are
> defined. SKILL.md format and export pipeline are specified. Dispute resolution is designed.
> Not yet implemented. Current knowledge sharing between agents uses direct Engram exchange
> via Agent Mesh sync without marketplace infrastructure.

---

## 10. Academic Citations

- Arrow 1962 — Economic Welfare and the Allocation of Resources for Invention
- Nelson 1970 — Information and Consumer Behavior
- Bakos & Brynjolfsson 1999 — Bundling and Competition on the Internet
- Grossman & Stiglitz 1980 — On the Impossibility of Informationally Efficient Markets
- Shapiro 1983 — Premiums for High Quality Products
- Williamson 1979 — Transaction Cost Economics
- Hong et al. 2023 — MetaGPT
- Qian et al. 2023 — ChatDev

---

*Generated from: bardo-backup/prd/09-economy/06-commerce-bazaar.md, bardo-backup/prd/09-economy/03-marketplace.md,
bardo-backup/tmp/agent-chain-new/12-agent-economy.md. Death archives and necrocracy references
removed per 02-reframe-rules.md. All naming renames applied: golem→agent, clade→collective,
Grimoire→Neuro/Engram, Styx→Agent Mesh, GNOS→KORAI.*
