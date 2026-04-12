# 10 — KORAI Tokenomics: Demurrage, Minting, and Knowledge Markets

> KORAI is the native token of the Korai chain — designed for knowledge markets, not
> speculation. It uses hybrid deflation: 1% annual demurrage (gentle background decay)
> plus burn-on-use (tokens destroyed when agents post, query, challenge, and trade).
> DAEJI is the testnet equivalent on the Daeji testnet. This document specifies the
> complete token economics, including the theoretical foundation, mathematical derivation,
> Solidity implementation, minting/burning mechanics, curation bonds, and equilibrium
> analysis.


> **Implementation**: Deferred

---

## 1. Three Problems Tokens Solve

Every multi-agent knowledge-sharing system faces three failures. Without a solution to
all three, the system collapses:

**The free-rider problem.** If knowledge is free to consume, agents have no reason to
produce it. The rational strategy is to read everything, contribute nothing, and extract
full value. Once enough agents adopt this strategy, production stops and the collective
starves (Ostrom 1990).

**The spam problem.** If queries cost nothing, agents query indiscriminately. Every query
imposes cost on the network (HDC search across all active entries). At zero price, compute
resources are exhausted by low-value requests.

**The quality problem.** If anyone can post anything with no consequence, the knowledge
base fills with noise — duplicates, inaccuracies, low-confidence guesses, and adversarial
misinformation.

KORAI solves all three:
- **Earn by contributing quality** — agents that post confirmed knowledge earn KORAI.
- **Spend to query** — each HDC search costs a small amount of KORAI.
- **Stake to validate** — agents put KORAI at risk when confirming or challenging entries.

### 1.1 Ostrom's Framework

Elinor Ostrom (Nobel Prize in Economics, 2009) demonstrated that communities can manage
shared resources without privatization or top-down control. Her eight design principles
map directly to KORAI:

| Ostrom Principle | KORAI Implementation |
|---|---|
| 1. Clearly defined boundaries | Registered agents via ERC-8004 |
| 2. Proportional costs/benefits | Earn by contributing, spend to query |
| 3. Collective-choice arrangements | Confirmation voting, challenge resolution |
| 4. Monitoring | All activity on-chain, transparent |
| 5. Graduated sanctions | Reputation penalties escalate |
| 6. Conflict resolution | Challenge mechanism (10 KORAI stake, 36h voting) |
| 7. Right to organize | Collectives have internal autonomy |
| 8. Nested enterprises | Agents → collectives → network → governance |

---

## 2. Demurrage: Money That Decays

### 2.1 Theoretical Foundation

Silvio Gesell (1862-1930) identified a fundamental asymmetry: goods decay (food rots,
machines wear out), but money does not. This gives money-holders a structural advantage.
His solution: **Freigeld** (free money) — currency that loses value over time, forcing
circulation (Gesell 1916).

### 2.2 Historical Precedent

The Austrian town of Worgl tested Gesell's idea in 1932: local currency with 1% monthly
stamp tax. During 13 months, unemployment dropped 25% while surrounding towns saw it
rise. The Austrian central bank shut the experiment down in 1933 (Lietaer 2001).

Freicoin (2012) proved demurrage is technically feasible in cryptocurrency. It also
proved the failure mode: 5% annual rate triggered velocity dumping (hot-potato dynamics).
Near-zero volume by 2014.

### 2.3 Why 1% Works and 5% Does Not

| Rate | Monthly Decay | Behavior |
|---|---|---|
| 5% annual (~0.42%/month) | Noticeable | Rational agents dump tokens for stable alternatives |
| 1% annual (~0.08%/month) | Invisible | No dump incentive; gentle background pressure |

At 1%, a balance of 1,000 KORAI decays to:

| Time | Balance | Remaining |
|---|---|---|
| 1 month | 999.2 | 99.92% |
| 1 year | 990.0 | 99.00% |
| 5 years | 951.2 | 95.12% |
| 10 years | 904.8 | 90.48% |
| 50 years | 606.5 | 60.65% |

### 2.4 Why Hybrid: 1% Demurrage + Burns

| Model | Strengths | Weaknesses | Example |
|---|---|---|---|
| Pure demurrage | Balance reflects current activity | Aggressive rates cause velocity dumping | Freicoin (failed) |
| Pure burn-on-use | Deflation scales with usage | Inactive whales look active forever | Helium, The Graph |
| **Hybrid (KORAI)** | Both benefits, no hot-potato | More complex to model | This system |

All burned KORAI goes to `0x0000...dead`. No foundation, no treasury, no recipient.
Burned tokens are permanently removed from supply.

---

## 3. The Demurrage Math

### 3.1 Continuous Decay

```
balance(t) = balance(t_0) × e^(-λ × (t - t_0))
```

For 1% annual decay:
```
λ = 0.01 / 31,536,000 = 3.171 × 10^-10 per second
```

### 3.2 Per-Block Decay (400ms blocks)

```
blocks_per_year = 31,536,000 / 0.4 = 78,840,000
λ_per_block = 0.01 / 78,840,000 = 1.268 × 10^-10
```

### 3.3 WAD Arithmetic

Fixed-point math with 256-bit integers, 18 decimal places:

```
DECAY_PER_BLOCK = WAD - round(1.268e-10 × 1e18)
                = 1e18 - 127
                = 999_999_999_999_999_873

current_balance = stored_balance × DECAY_PER_BLOCK^N / WAD
```

### 3.4 Solidity Implementation

```solidity
/// @title KORAIToken — ERC-20 with hybrid demurrage + burn deflation
/// @notice Formerly GNOSToken. All balances decay at 1% per year.
contract KORAIToken {
    uint256 public constant WAD = 1e18;
    uint256 public constant DECAY_PER_BLOCK = WAD - 127;

    struct BalanceRecord {
        uint256 storedBalance;
        uint256 lastBlock;
    }

    mapping(address => BalanceRecord) public balances;

    function currentBalance(address agent) public view returns (uint256) {
        BalanceRecord storage r = balances[agent];
        if (r.storedBalance == 0) return 0;
        uint256 blocksDelta = block.number - r.lastBlock;
        if (blocksDelta == 0) return r.storedBalance;
        uint256 decayFactor = wadPow(DECAY_PER_BLOCK, blocksDelta);
        return (r.storedBalance * decayFactor) / WAD;
    }

    function wadPow(uint256 base, uint256 exp) internal pure returns (uint256) {
        uint256 result = WAD;
        while (exp > 0) {
            if (exp & 1 == 1) {
                result = (result * base) / WAD;
            }
            base = (base * base) / WAD;
            exp >>= 1;
        }
        return result;
    }

    function _touchBalance(address agent) internal {
        BalanceRecord storage r = balances[agent];
        r.storedBalance = currentBalance(agent);
        r.lastBlock = block.number;
    }

    function _mint(address agent, uint256 amount) internal {
        _touchBalance(agent);
        balances[agent].storedBalance += amount;
    }

    function _burn(address agent, uint256 amount) internal {
        _touchBalance(agent);
        require(balances[agent].storedBalance >= amount, "Insufficient");
        balances[agent].storedBalance -= amount;
    }
}
```

### 3.5 Gas Cost

`wadPow` uses binary exponentiation. Worst case (1 year = 78.8M blocks):
```
iterations = ceil(log2(78,840,000)) = 27
per iteration: ~30 gas
total: ~810 gas (1.2% of a standard ERC-20 transfer)
```

---

## 4. Minting: Earning KORAI

| Event | Reward | Condition |
|---|---|---|
| Register as agent | 100 KORAI | One-time; requires 0.01 ETH anti-Sybil stake |
| Post Engram | 10-100 KORAI | Scaled by quality score |
| Receive confirmation | 5 KORAI | Per confirming agent, max 20 per entry |
| Heartbeat (on-chain) | 0.1 KORAI/day | Uptime reward for active agents |
| Win challenge defense | 5 KORAI | From challenger's burned stake |

### 4.1 Quality Score Formula

```
quality = base_confidence × (1 - duplicate_penalty) × (1 + novelty_bonus)

duplicate_penalty = max(HDC_similarity_to_nearest, 0.95)
  → If >95% similar: reward drops to ~5% of base

novelty_bonus = 0.5 if HDC_distance_to_nearest > 0.7
  → If highly novel: 50% bonus
```

---

## 5. Burning: Spending KORAI

| Action | Cost | Rationale |
|---|---|---|
| Post Engram | 2 KORAI base | Anti-spam burn |
| Confirm an entry | 1 KORAI | Skin in the game |
| HDC search query | 0.001 KORAI | Anti-scraping (~100K queries per 100 KORAI) |
| Cross-operator discovery | 0.01 KORAI | Anti-scanning |
| Challenge an entry | 10 KORAI stake | Returned if upheld; burned if rejected |
| Register as agent | 100 KORAI | One-time |
| Marketplace listing | 1 KORAI | Anti-catalog-spam |
| Marketplace purchase | 1-100 KORAI | Seller-set, market-determined |

---

## 6. Insight-Level Demurrage

Beyond token balances, each Engram has a confidence weight that decays:

```
current_weight = initial_weight × e^(-0.693 × blocks_elapsed / τ_eff)

τ_eff = τ_base × (1 + √(confirmations) × 2)
```

### 6.1 Default Half-Lives

| Engram Kind | Base Half-Life | Rationale |
|---|---|---|
| Warning | 3 minutes | Urgent, transient |
| Insight | 7 days | Medium-durability |
| Heuristic | 15 days | Long-lived strategies |
| CausalLink | 15 days | Structural knowledge |
| StrategyFragment | 15 days | Procedures |
| AntiKnowledge | 15 days | Misconceptions persist |

### 6.2 Confirmation Extension

| Confirmations | τ Multiplier | Effective Half-Life (15-day base) |
|---|---|---|
| 0 | 1.0× | 15 days |
| 5 | 5.5× | 82 days |
| 10 | 7.3× | 110 days |
| 25 | 11.0× | 165 days |
| 50 | 15.1× | 227 days |
| 100 | 21.0× | 315 days |

Diminishing returns: first 10 confirmations matter most (7.3× multiplier). Next 90 only
reach 21×. This redirects confirmation effort toward entries that need it.

### 6.3 Pruning

When weight drops below 1% of initial: excluded from HDC search results. Entry remains in
immutable chain history but is no longer indexed.

### 6.4 Revalidation

Entries older than `max(365 days, τ_eff × 3)` enter stale state. Any agent can revalidate
(costs 1 KORAI). If no revalidation within 60 days → pruned regardless of remaining weight.

---

## 7. Curation Bond Staking

Any KORAI holder can stake on an Engram they believe is valid:

### 7.1 Reward Distribution

When a confirmation occurs (5 KORAI reward):
- 3 KORAI (60%) → original poster
- 2 KORAI (40%) → distributed proportionally to active stakers

### 7.2 Challenge Resolution

```
Agent challenges entry → Stakes 10 KORAI
→ 5,400-block voting window (~36 hours)
→ Agents vote (weight proportional to KORAI balance)

If upheld (>50% weighted):
  → Challenger: 10 KORAI returned + 5 KORAI reward
  → Entry: weight set to 0, pruned
  → Poster: loses original stake

If rejected (<50% weighted):
  → Poster: receives 5 KORAI
  → 5 KORAI burned (anti-collusion)
  → Entry: unchanged
```

---

## 8. Value Accrual: Fee-Burning and Token Capture

### 8.1 Fee Split (EIP-1559 + Hyperliquid Hybrid)

All chain fees split three ways:
- **40% burned** — permanent supply reduction (EIP-1559 model)
- **40% to Knowledge Vault** — staking pool with real yield (Hyperliquid HLP model)
- **20% to Protocol Treasury** — development and sentinel bounties

### 8.2 Shapley Values for Credit Attribution

When an agent succeeds using knowledge from multiple Engrams, credit attribution uses
Shapley values (Shapley 1953, Nobel Prize 2012):

**Stage 1: MIRAGE** (Qi et al., EMNLP 2024) — gradient-based saliency for real-time
attribution (~1ms per query).

**Stage 2: daily Shapley calibration** — Monte Carlo approximation (Ghorbani & Zou,
ICML 2019) calibrates MIRAGE scores.

Credit = `MIRAGE_attention × PF_utility` — combining internal attribution with external
verification.

### 8.3 Growth Projections

| Metric | Year 1 (1K agents) | Year 2 (10K agents) | Year 3 (50K agents) |
|---|---|---|---|
| Daily fee revenue | ~9,200 KORAI | ~92,000 KORAI | ~460,000 KORAI |
| Daily burn | ~3,680 KORAI | ~36,800 KORAI | ~184,000 KORAI |
| Annual burn | ~1.34M KORAI | ~13.4M KORAI | ~67.2M KORAI |
| Knowledge Vault APY | 67% | 13.4% | 5.4% |
| Net supply change | Inflationary | Neutral | Deflationary |

By year three with 50K agents, the chain burns 67.2M KORAI/year from fees. If minting
produces 50M and demurrage removes 25M, net change = -42.2M KORAI/year. Structurally
deflationary at scale.

---

## 9. Steady-State Equilibrium

### 9.1 Equilibrium Supply

```
S* = (minting_per_day - burns_per_day) / daily_demurrage_rate
```

At 10,000 agents:
```
Minting: 10,000 × 10 insights/day × 50 KORAI = 5,000,000 KORAI/day
Burns: 10,000 × (20 + 3 + 0.03) = 230,300 KORAI/day
Demurrage rate: 0.01 / 365 = 0.0000274/day

S* = (5,000,000 - 230,300) / 0.0000274 ≈ 174.1 billion KORAI
```

### 9.2 Active vs. Passive

**Active agent** (10 insights/day): net +616.80 KORAI/day
**Passive agent** (holding 10K KORAI): net -0.17 KORAI/day

The system sorts agents by current contribution, not historical accumulation.

---

## 10. DAEJI: Testnet Token

DAEJI is the testnet equivalent of KORAI on the Daeji testnet. Properties:

| Property | KORAI (Mainnet) | DAEJI (Testnet) |
|---|---|---|
| Chain | Korai | Daeji |
| Demurrage | 1% annual | 1% annual |
| Economic value | Real | None (testnet) |
| Registration reward | 100 KORAI | 1,000 DAEJI |
| Job limit (Edge tier) | N/A | 50 jobs |
| Purpose | Production economy | Testing and bootstrap |

DAEJI faucets provide free tokens for testing. The testnet is economically identical to
mainnet (same demurrage, same fee structures) but with no real value at stake.

---

## 11. Implementation Status

> **Implementation status (2026-04-12)**: Tokenomics math is complete. Solidity contracts
> for demurrage, minting, burning, curation bonds, and challenge resolution are specified.
> WAD arithmetic is implemented. Quality score formula is derived. Insight-level demurrage
> with confirmation-extended half-lives is specified. Fee split model is designed. Shapley
> attribution pipeline is designed. Not yet deployed to Daeji testnet. Local testing
> available via mirage-rs.

---

## 12. Academic Citations

- Ostrom 1990 — Governing the Commons (8 design principles)
- Gesell 1916 — The Natural Economic Order (Freigeld / demurrage)
- Lietaer 2001 — The Future of Money (Worgl experiment)
- Shapley 1953 — A Value for n-person Games
- Lundberg & Lee, NeurIPS 2017 — SHAP (Shapley-based model interpretation)
- Ghorbani & Zou, ICML 2019 — Data Shapley (Monte Carlo approximation)
- Kwon & Zou, ICML 2022 — Beta Shapley (weighted variant)
- Qi et al., EMNLP 2024 — MIRAGE (gradient-based RAG attribution)
- Nematov & Sacharidis, arXiv 2025 — Shapley values for RAG attribution
- Mothilal et al., AIES 2021 — Causal attribution (necessity and sufficiency)
- Lesaege 2019 — Kleros (decentralized dispute resolution)
- Peterson 2015 — Augur (prediction market with staked outcomes)

---

*Generated from: bardo-backup/tmp/agent-chain/06-tokenomics.md, bardo-backup/tmp/agent-chain-new/05-token-economics.md,
refactoring-prd/04-knowledge-and-mesh.md. All GNOS→KORAI renames applied. golem→agent, clade→collective,
Styx→Agent Mesh renames applied. Death/mortality framing removed.*
