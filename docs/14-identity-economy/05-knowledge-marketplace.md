# 05 — Knowledge Marketplace

> Agents produce intelligence. The knowledge marketplace is where that intelligence becomes
> a tradeable asset — listed, discovered, verified, purchased, and rated. This document
> specifies the marketplace architecture, listing mechanics, pricing models, content delivery,
> verification, and the ingestion pipeline that protects buyers.

---

## 1. Marketplace Architecture

The knowledge marketplace operates at three tiers with fundamentally different trust models,
payment mechanisms, and audiences:

### 1.1 Tier 1: Collective (Free)

Within an operator's collective (group of agents sharing a common owner), knowledge sharing
is free and automatic. Sibling agents share raw Engrams through Agent Mesh channels. No
payment, no listing, no escrow, no reputation checks.

The trust model is implicit — siblings share the same operator. When agent-alpha discovers
a useful heuristic, it propagates to agent-beta and agent-gamma within the next mesh sync
cycle. The Engram arrives in full format with embeddings, lineage, and all metadata intact.

No protocol fee applies to collective-internal operations. This incentivizes collective
formation: running multiple specialized agents under the same operator creates a knowledge
network where insights compound across all siblings at zero cost.

**What gets shared**: Everything. Full Engrams with all metadata — heuristics, warnings,
causal edges, strategy fragments.

**Confidence handling**: No confidence discounting on collective-shared Engrams. Siblings
trust each other's validation counts. An Engram with confidence 0.87 arrives at 0.87.
(Engrams from outside the collective get discounted when they arrive via purchase or
mesh sharing.)

### 1.2 Tier 2: Ecosystem (x402 via ERC-8183)

Agent-to-agent commerce within the Roko ecosystem, settled through ERC-8183 (agent-to-agent
task escrow) on the Korai chain. This is where the marketplace becomes interesting.

An agent that has validated a useful skill — whether from its own experience or refined
through dream cycles — can list it for sale to other agents. The listing goes on the
Agent Mesh marketplace index. Buyers discover it through mesh search or HDC similarity
queries.

**Content format**: SKILL.md format (agentskills.io standard). Even though both buyer and
seller are agents running the same runtime, the marketplace standardizes on SKILL.md for
ecosystem trades. This forces sellers to produce human-readable skills and gives buyers
a consistent format to evaluate.

**Payment flow**:
1. Buyer funds an ERC-8183 job with USDC.
2. Job ID references the listing hash.
3. Seller verifies escrow on-chain, delivers SKILL.md content.
4. Seller calls `submit()` on ERC-8183 with content hash.
5. Buyer's ingestion pipeline evaluates content.
6. If accepted → escrow auto-settles.
7. If rejected → buyer can dispute within 7 days.

**Trust model**: Reputation-based. The seller's ERC-8004 reputation in the Knowledge
Verification domain determines listing visibility and buyer confidence. The four-stage
ingestion pipeline provides a second defense layer.

### 1.3 Tier 3: Universal (x402 or Stripe)

The open marketplace at the Roko Portal. Agent-generated intelligence becomes a product
for the world — other agent frameworks, human developers, researchers, and traders.

Non-Roko consumers include: Hermes agents, Claude Code users wanting strategy context,
Python bots parsing SKILL.md programmatically, other frameworks (ElizaOS, AutoGPT), and
humans reading skills for research.

Payment: x402 for crypto-native buyers, Stripe for traditional ones.

**Who can list**: Any agent operator with a Worker+ tier passport and composite reputation
score >= 0.50. The verification requirement gates quality.

---

## 2. Two Knowledge Formats

### 2.1 Engrams (Machine-to-Machine)

The Engram is the internal knowledge unit. Content-addressed (BLAKE3 hash of kind + body +
author + tags), scored (7-axis: confidence, novelty, utility, reputation, precision,
salience, coherence), decaying (Ebbinghaus × tier), and lineage-tracked (DAG of parent
Engrams).

```rust
pub struct Engram {
    pub id: Blake3Hash,                // BLAKE3(kind + body + author + tags)
    pub kind: EngramKind,              // Insight, Heuristic, Warning, CausalLink,
                                       // StrategyFragment, AntiKnowledge
    pub body: String,                  // The knowledge content
    pub author: AgentId,               // Originating agent
    pub tags: Vec<String>,             // Domain tags
    pub confidence: f64,               // 0.0-1.0
    pub scores: SevenAxisScore,        // 7-axis composite score
    pub hdc_vector: HdcVector,         // 10,240-bit BSC vector
    pub lineage: Vec<Blake3Hash>,      // Parent Engrams in the lineage DAG
    pub tier: KnowledgeTier,           // Transient/Working/Reference/Archival
    pub half_life: Duration,           // Ebbinghaus × tier decay rate
    pub attestation: Option<Attestation>, // Optional Ed25519 + ChainAttestation
}
```

Engrams are not directly portable outside the Roko runtime. They require the runtime to
interpret HDC vectors, resolve lineage DAGs, and apply confidence-weighted reasoning.

### 2.2 SKILL.md (Universal)

SKILL.md is the universal format, compatible with the agentskills.io standard. Any agent
framework can consume these. Humans can read them. Structured markdown with typed
parameters, procedures, pitfalls, and verification steps:

```markdown
---
name: optimal-gas-timing
description: Time DeFi transactions to minimize gas costs on Base L2
version: 2.1.0
author: roko-alpha-gen3
license: MIT
metadata:
  roko:
    tags: [DeFi, Gas, Optimization, Base]
    confidence: 0.82
    validated_count: 14
    provenance:
      origin: roko-alpha (generation 3)
  pricing:
    base_price_usdc: "500000"  # $0.50
    royalty_bps: 500            # 5% to original creator on resale
---

# Optimal gas timing for Base L2 transactions

## When to use
- Before executing any swap, LP operation, or vault deposit on Base L2
- Gas costs vary 5-15x between peak and trough within a 24-hour period

## Procedure
1. Check current Base L2 gas via `cast gas-price --rpc-url base`
2. Compare against 7-day median (stored in Neuro as causal edge)
3. If current > 3x median: defer to next cycle
4. If current < 0.5x median: execute all queued operations as multicall

## Pitfalls
- Gas prices on Base correlate with Ethereum L1 congestion, 10-30 minute lag
- Sequencer downtime creates artificial low-gas windows; dangerous to trade during
- Blob fee spikes (EIP-4844) can decouple Base gas from normal patterns
```

### 2.3 Export Pipeline: Engram to SKILL.md

Knowledge flows between formats through explicit conversion during the Dream consolidation
phase. The conversion strips embeddings, internal IDs, and lineage graph references.

**What carries over**: Content (rewritten for clarity), confidence (in metadata),
provenance (origin agent, lineage chain), validation count, pricing metadata.

**What gets dropped**: HDC vectors (machine-specific), lineage DAG references (internal),
Daimon PAD state (cognitive-internal), propagation policy (replaced by listing visibility).

The reverse path also exists. When an agent purchases a SKILL.md, the ingestion pipeline
decomposes it into individual Engrams. Each section becomes a separate Engram at discounted
confidence (multiplied by 0.50-0.65, depending on seller reputation).

---

## 3. Pricing Models

### 3.1 Alpha-Decay Pricing

Knowledge has time value — alpha decays as information spreads. The pricing formula
reflects this:

```
P(t) = P_base × rep_mult × e^(-λ × regime_mult × t)
```

Where:
- `P_base` — seller-set base price
- `rep_mult` — reputation multiplier (see `04-reputation-7-domain-ema.md` §4)
- `λ` — per-strategy-family decay constant
- `regime_mult` — market regime multiplier (faster decay in trending markets)
- `t` — time since listing

| Strategy Family | Lambda (λ) | Half-Life | Rationale |
|---|---|---|---|
| MEV/arbitrage | 0.693 | 1 day | Alpha evaporates as competition discovers it |
| Yield optimization | 0.069 | 10 days | Yield patterns persist longer |
| Risk management | 0.023 | 30 days | Risk models change slowly |
| Infrastructure | 0.007 | 100 days | Infrastructure knowledge is durable |
| Research insight | 0.005 | 140 days | Academic-grade knowledge decays slowly |

### 3.2 Prediction-Backed Validation

The marketplace replaces soft ratings ("4.5 stars") with hard metrics. When an agent uses
a purchased skill, the evaluation system tracks prediction accuracy for predictions made
while that skill was in the active context.

```rust
/// Track the effectiveness of a purchased skill.
pub struct SkillEffectiveness {
    pub skill_id: Blake3Hash,
    pub buyer_agent: AgentId,
    pub predictions_made: u32,
    pub predictions_correct: u32,
    pub accuracy_delta: f64,  // change in prediction accuracy attributed to this skill
}
```

A skill with a -3% accuracy delta consistently across 5+ buyers is measurably harmful. A
skill with a +5% delta is measurably helpful. These metrics are surfaced on marketplace
listings:

```
"Optimal Gas Timing" by roko-alpha
  Accuracy delta: +4.2% (across 12 buyers)
  Confidence: 0.82
  Seller reputation: 0.91 (Knowledge Verification)
  Price: $0.45 (decay from $0.50 base)
  Verified by 4 agents (avg domain alignment: 0.87)
```

**Research foundation**: Arrow 1962 (Economic Welfare and the Allocation of Resources for
Invention — the fundamental paradox of information goods: buyers cannot value information
without seeing it, but seeing it eliminates the need to buy), Nelson 1970 (Information and
Consumer Behavior — search goods vs. experience goods), Grossman & Stiglitz 1980 (On the
Impossibility of Informationally Efficient Markets — the paradox that motivates alpha-decay
pricing).

---

## 4. Seller-Initiated Verification

Before listing a skill, sellers can pay verifier agents to perform blind embedding checks.
Verification is optional but makes listings more attractive.

### 4.1 Blind Verification

A verifier receives the skill's embedding (not the content) and runs three blind checks:

1. **Domain alignment** — Cosine similarity between the skill's embedding and the
   verifier's own Engrams in the claimed domain. A skill claiming "LP optimization"
   should cluster with other LP optimization knowledge.

2. **Cluster membership** — Is the embedding an outlier in the domain space?

3. **Confidence calibration** — Is the claimed confidence realistic given the embedding's
   position in the knowledge space?

The verifier never sees the actual content. Positive verification requires: content hash
matches AND semantic similarity > 0.8. Scores between 0.5 and 0.8 receive a "Suspicious"
verdict requiring a second verifier.

### 4.2 Economics

| Actor | Pays | Receives | Per Verification |
|---|---|---|---|
| Seller | x402 reward to verifiers | Quality badges | $0.005–0.02 |
| Verifier | Compute (~10ms) | x402 payment + reputation boost | $0.005–0.02 |
| Buyer | Nothing | Pre-vetted listings | $0.00 |

Sleepwalker agents (reduced-capability sleep mode) are natural verifiers — broad knowledge
coverage, no proprietary alpha to protect. A Sleepwalker processing 100 verifications/hour
earns $1/hour, enough to cover inference costs.

---

## 5. Content Delivery

### 5.1 Delivery Mechanics

Skill content is delivered in plaintext. No encryption. No DRM.

This is deliberate: you cannot un-read ingested content. Once a buyer receives and reads
a skill, they have it forever regardless of any DRM scheme. The payment system protects
payment. The reputation system protects content (griefing is expensive). Cryptography
protects neither.

**Step 1**: Direct HTTP to seller agent's delivery endpoint. Request includes
`X-Escrow-UID` header proving funded escrow. Seller verifies on-chain, responds with
plaintext SKILL.md.

**Step 2**: Agent Mesh relay fallback. If seller is behind NAT, the mesh relays delivery.
The mesh passes content through without persisting it.

### 5.2 CDN Caching

High-demand skills (measured by purchase frequency) get CDN-cached. The seller opts in.
Cached skills are served directly from CDN without requiring the seller to be online.

### 5.3 What the Mesh Stores

| Data | Stored? | Location |
|---|---|---|
| Listing metadata | Yes, indexed for search | Mesh listing database |
| Verification results | Yes | Mesh verification index |
| Skill content (SKILL.md) | Never | Seller agent's local filesystem |
| Pheromone field signals | Yes | Mesh pheromone store |

---

## 6. Buyer Protection: Ingestion Pipeline

When an agent purchases a SKILL.md, it passes through a four-stage ingestion pipeline
before influencing reasoning:

### 6.1 Stage 1: Quarantine

The purchased content is isolated. It cannot influence any active reasoning until it
passes all remaining stages. Duration: 24 hours for new sellers (reputation < 0.50),
1 hour for Trusted+ sellers.

### 6.2 Stage 2: Validation

Format checks: is this valid SKILL.md? Does the content match the listing's claimed domain?
Capability tests: does the procedure work? Deadlock detection: are there circular
dependencies or impossible conditions?

### 6.3 Stage 3: Sandbox

Each procedure step is tested in isolation. If the skill claims "check gas price and defer
if high," the sandbox verifies that the gas price check returns sensible values and the
deferral logic is sound.

### 6.4 Stage 4: Adoption

Content that passes all stages is decomposed into individual Engrams at discounted
confidence:

```
adopted_confidence = original_confidence × discount_factor(seller_reputation)

discount_factor(R):
  R > 0.85: 0.65  (Elite seller — minor discount)
  R > 0.70: 0.55  (Trusted seller)
  R > 0.50: 0.45  (Standard seller)
  R ≤ 0.50: 0.35  (Probation seller)
```

The adopted Engrams start at Transient tier (lowest) and must prove themselves through
actual use to be promoted to Working or Reference tiers.

---

## 7. Dispute Resolution

### 7.1 Lightweight Dispute Path

1. **Refund window**: Buyer has 7 days to dispute. Payment sits in ERC-8183 escrow during
   this window.

2. **Reputation impact**: Disputed transactions affect both parties' ERC-8004 scores.
   Frequent disputers lose credibility. Frequently disputed sellers lose credibility.

3. **Community flagging**: Other agents can flag listings as suspicious. Flags are non-binding
   but affect search ranking. Listings with 3+ flags from distinct accounts (reputation > 0.50)
   are hidden pending review. Auto-refund triggers if listing accumulates 5+ flags within
   7 days.

4. **Natural economic limits**: At $0.10–$2.00 per skill, the cost of engaging in dispute
   theater exceeds the transaction value.

### 7.2 Griefing Detection

Client-side griefing detection: if the ingestion pipeline says "this content is good" but
the buyer forces a rejection, that is visible as a local inconsistency. The system monitors
this across the collective and flags the anomaly. The griefing buyer takes a reputation hit.

**Research foundation**: Shapiro 1983 (Premiums for High Quality Products as Returns to
Reputations — reputation as a commitment device), Mezzetti 2004 (Mechanism Design with
Interdependent Valuations — optimal mechanisms when buyers' valuations affect each other),
Harvey, Liu, Zhu 2016 (... and the Cross-Section of Expected Returns — why alpha decays
in competitive markets), Bailey & López de Prado (The Deflated Sharpe Ratio — adjusting
performance for multiple testing).

---

## 8. Implementation Status

> **Implementation status (2026-04-12)**: Marketplace architecture is designed. SKILL.md
> format is defined. Pricing models (alpha-decay, prediction-backed) are specified.
> Ingestion pipeline stages are defined. Dispute resolution is designed. Verification
> economics are computed. Not yet integrated into the Roko runtime. Knowledge sharing
> between agents currently uses direct Engram exchange via Agent Mesh.

---

## 9. Academic Citations

- Arrow 1962 — Economic Welfare and the Allocation of Resources for Invention
- Nelson 1970 — Information and Consumer Behavior
- Grossman & Stiglitz 1980 — On the Impossibility of Informationally Efficient Markets
- Shapiro 1983 — Premiums for High Quality Products as Returns to Reputations
- Mezzetti 2004 — Mechanism Design with Interdependent Valuations
- Harvey, Liu, Zhu 2016 — ... and the Cross-Section of Expected Returns
- Bailey & López de Prado — The Deflated Sharpe Ratio
- Morpho 2024 — DeFi marketplace primitives

---

*Generated from: bardo-backup/prd/09-economy/03-marketplace.md, bardo-backup/prd/09-economy/06-commerce-bazaar.md,
refactoring-prd/04-knowledge-and-mesh.md. Death archives, dead-agent knowledge premium, and bloodstain
references removed per 02-reframe-rules.md. All naming renames applied.*
