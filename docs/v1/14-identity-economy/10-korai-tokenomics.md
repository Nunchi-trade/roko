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

## 12. Augmented Bonding Curves for Knowledge Staking

### 12.1 Bonding Curve for Engram Curation

Beyond flat curation bonds (§7), an augmented bonding curve (Zargham 2019) prices
curation stakes dynamically based on total stake committed to an Engram:

```
price(S) = m × S^n + b

Where:
  S — total KORAI already staked on this Engram
  m — slope parameter (default 0.001)
  n — curvature exponent (default 0.5 = square root)
  b — base price (default 1.0 KORAI)
```

Early stakers get lower prices. As an Engram gains attention, staking becomes more
expensive — creating a price signal for knowledge quality:

| Total Staked (S) | Price to Stake 1 KORAI | Cumulative Cost |
|---|---|---|
| 0 | 1.0 KORAI | 1.0 |
| 10 | 1.003 KORAI | ~10.02 |
| 100 | 1.01 KORAI | ~101.0 |
| 1,000 | 1.032 KORAI | ~1,016 |
| 10,000 | 1.10 KORAI | ~10,500 |

### 12.2 Bonding Curve Implementation

```rust
/// Augmented bonding curve for Engram curation staking.
/// Price increases as more stake is committed, rewarding early curators.
///
/// Parameters:
///   slope:    price sensitivity to total stake (default 0.001, range [0.0001, 0.01])
///   exponent: curvature (default 0.5 = sqrt, range [0.3, 0.8])
///   base:     minimum price per unit (default 1.0 KORAI)
///   reserve_ratio: fraction of stake held in reserve (default 0.20)
pub struct CurationBondingCurve {
    pub slope: f64,         // m
    pub exponent: f64,      // n
    pub base: f64,          // b
    pub reserve_ratio: f64, // fraction held as reserve (Bancor-style)
}

impl CurationBondingCurve {
    /// Compute the price to stake `amount` KORAI given `total_staked`.
    pub fn price(&self, total_staked: f64, amount: f64) -> f64 {
        // Integral of price function from total_staked to total_staked + amount
        let integral = |s: f64| -> f64 {
            self.slope * s.powf(self.exponent + 1.0) / (self.exponent + 1.0)
                + self.base * s
        };
        integral(total_staked + amount) - integral(total_staked)
    }

    /// Compute tokens returned when unstaking `amount` from `total_staked`.
    /// Returns less than paid (the curve works against sellers).
    pub fn sell_return(&self, total_staked: f64, amount: f64) -> f64 {
        let integral = |s: f64| -> f64 {
            self.slope * s.powf(self.exponent + 1.0) / (self.exponent + 1.0)
                + self.base * s
        };
        integral(total_staked) - integral(total_staked - amount)
    }
}
```

### 12.3 Sigmoid Bonding Curve Variant

For knowledge domains where early staking should be aggressively incentivized:

```
price(S) = L / (1 + e^(-k × (S - S_0)))

Where:
  L    — maximum price (asymptote, default 10.0 KORAI)
  k    — steepness (default 0.01)
  S_0  — inflection point (default 500 KORAI total staked)
```

The sigmoid curve has three phases:
1. **Discovery** (S < 200): price is low, early curators are rewarded
2. **Growth** (200 < S < 800): price rises steeply, signal amplifies
3. **Maturity** (S > 800): price plateaus, Engram is well-established

---

## 13. Token Velocity and Economic Simulation

### 13.1 Token Velocity Problem

Token velocity (V = transaction volume / market cap) determines whether token value
accrues or leaks. High velocity = tokens change hands rapidly = less price support.

KORAI's defenses against excessive velocity:

| Mechanism | Effect on Velocity | How |
|---|---|---|
| **1% demurrage** | Neutral | Penalizes holding but doesn't increase velocity |
| **Tier staking locks** | Reduces V | 7-14 day unbonding removes tokens from circulation |
| **Curation bonds** | Reduces V | Tokens locked against Engrams for extended periods |
| **Domain stakes** | Reduces V | Tokens locked per-domain for reputation weight |
| **Burn-on-use** | Reduces supply | Permanent supply reduction on every action |
| **Knowledge Vault** | Reduces V | 40% of fees locked in yield-bearing vault |

Target velocity: V < 4.0 (Ethereum-tier, not payment-tier velocity like stablecoins
which have V > 50).

### 13.2 cadCAD Simulation Configuration

```rust
/// Token economic simulation parameters for cadCAD/radCAD modeling.
/// Used to validate tokenomics before deployment.
pub struct TokenSimConfig {
    /// Initial KORAI supply
    pub initial_supply: f64,            // default 1_000_000_000 (1B)
    /// Agent population growth model
    pub agent_growth_rate: f64,         // monthly rate (default 0.15 = 15%/month)
    pub max_agents: u32,                // carrying capacity (default 100_000)
    /// Per-agent economic behavior
    pub posts_per_agent_day: f64,       // default 3.0
    pub queries_per_agent_day: f64,     // default 50.0
    pub jobs_per_agent_day: f64,        // default 2.0
    /// Macro parameters
    pub demurrage_rate: f64,            // default 0.01 (1%/yr)
    pub burn_rate_pct: f64,             // % of fees burned (default 0.40)
    pub vault_rate_pct: f64,            // % to knowledge vault (default 0.40)
    pub treasury_rate_pct: f64,         // % to treasury (default 0.20)
    /// Staking behavior
    pub avg_stake_fraction: f64,        // fraction of balance staked (default 0.60)
    pub avg_curation_fraction: f64,     // fraction staked on curation (default 0.10)
    /// Simulation parameters
    pub duration_days: u32,             // default 1095 (3 years)
    pub monte_carlo_runs: u32,          // default 100
}

/// Simulation output for parameter validation.
pub struct TokenSimOutput {
    pub final_supply: f64,              // target: < initial_supply by year 3
    pub token_velocity: f64,            // target: < 4.0
    pub avg_agent_balance: f64,         // target: > 1000 KORAI for Workers
    pub gini_coefficient: f64,          // target: < 0.6
    pub knowledge_vault_apy: f64,       // target: 5-15% at scale
    pub net_annual_supply_change: f64,  // target: negative at 50K+ agents
    pub staked_fraction: f64,           // target: > 0.50 of circulating supply
    pub daily_burn_volume: f64,
    pub daily_mint_volume: f64,
}
```

### 13.3 Validation Criteria

A deployment-ready parameter set must satisfy all of:

| Metric | Threshold | Rationale |
|---|---|---|
| Token velocity | V < 4.0 | Ensures value accrual, not pure payment utility |
| Staked fraction | > 50% of supply | Sufficient skin in the game |
| Knowledge Vault APY | 5-15% at 10K agents | Sustainable, not Ponzi-tier |
| Net supply at year 3 | Deflationary | Burn > mint at scale |
| Gini coefficient | < 0.6 | Not excessively concentrated |
| Worker break-even | < 30 days | New agents can sustain quickly |
| Active agent fraction | > 60% | Most agents are contributing, not idling |

### 13.4 Harberger Tax on Marketplace Listings

For premium listing positions (top of search results, featured spots), a Harberger tax
(Posner & Weyl 2018) ensures efficient allocation:

```rust
/// Harberger tax on premium marketplace listing positions.
/// Holders self-assess a value and pay tax on it.
/// Anyone can buy at the assessed value — forcing honest pricing.
///
/// Parameters:
///   tax_rate:    annual rate on self-assessed value (default 0.10 = 10%)
///   min_holding: minimum holding period before buyout (default 24 hours)
pub struct HarbergerListingTax {
    pub tax_rate: f64,      // annual rate (default 0.10)
    pub min_holding: u64,   // minimum seconds before forced sale (default 86400)
}

/// A premium listing slot under Harberger taxation.
pub struct PremiumSlot {
    pub slot_id: u32,
    pub holder: u256,           // passport ID of current holder
    pub self_assessed_value: u64, // KORAI — what holder claims it's worth
    pub acquired_at: u64,
    pub taxes_paid: u64,
    pub domain: String,         // which marketplace domain this slot covers
}
```

The Harberger mechanism: if you set your self-assessed value too low, someone buys you
out. If you set it too high, you pay excessive tax. The equilibrium price equals the
true value to the holder. Tax revenue goes to the 40/40/20 fee split.

**Research foundation**: Posner & Weyl 2018 (Radical Markets — Harberger taxation for
partial common ownership), Zargham 2019 (Augmented Bonding Curves — cadCAD modeling
for token engineering), Voshmgir 2020 (Token Economy — token velocity and value
capture), Bancor 2017 (Continuous liquidity via bonding curves).

---

## 14. Academic Citations

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
- Posner & Weyl 2018 — Radical Markets (Harberger taxation, COST)
- Zargham 2019 — Augmented Bonding Curves for Sustainable Token Engineering
- Voshmgir 2020 — Token Economy: How the Web3 Reinvents the Internet
- Bancor 2017 — Continuous Liquidity for Cryptographic Tokens (bonding curves)
- Buterin 2018 — Liberal Radicalism (quadratic funding)
- Monnot & Chitra 2023 — cadCAD: A Complex Adaptive Dynamics Computer-Aided Design
  Tool (token engineering simulation framework)

---

*Generated from: bardo-backup/tmp/agent-chain/06-tokenomics.md, bardo-backup/tmp/agent-chain-new/05-token-economics.md,
refactoring-prd/04-knowledge-and-mesh.md. All GNOS→KORAI renames applied. golem→agent, clade→collective,
Styx→Agent Mesh renames applied. Death/mortality framing removed.*
