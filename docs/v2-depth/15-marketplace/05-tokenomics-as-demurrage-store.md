# Tokenomics as Demurrage Store

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers KORAI tokenomics -- 1% annual demurrage, minting/burning mechanics, curation bonds, bonding curves, Shapley attribution, fee-burn economics, and equilibrium analysis -- all expressed as Store operations with built-in demurrage.

---

## 1. Three Problems KORAI Solves

Every multi-agent knowledge-sharing system faces three failures. KORAI addresses all three via Store economics:

**Free-rider problem** (Ostrom 1990): If knowledge is free to consume, agents contribute nothing. KORAI solution: earn by contributing quality (Store::put that passes Verify gates earns KORAI).

**Spam problem**: If queries cost nothing, agents query indiscriminately. KORAI solution: every Store::query costs KORAI (HDC search costs 0.001 KORAI per query).

**Quality problem**: If posting is free, the Store fills with noise. KORAI solution: posting costs 2 KORAI (anti-spam burn) and rewards scale with quality score.

---

## 2. Demurrage: The Same Mechanism as Signal Decay

KORAI demurrage is the **same mechanism** as Signal demurrage in the knowledge Store (see [01-SIGNAL.md](../../unified/01-SIGNAL.md) and [06-MEMORY.md](../../unified/06-MEMORY.md)). Both implement Gesell's principle: value decays unless actively reinforced.

### 2.1 Balance Decay

```
balance(t) = balance_0 * (1 - lambda)^blocks

For 1% annual decay at 400ms block time:
  blocks_per_year = 31,536,000 / 0.4 = 78,840,000
  lambda_per_block = 0.01 / 78,840,000 = 1.268e-10
  DECAY_PER_BLOCK = 1e18 - 127 = 999_999_999_999_999_873 (WAD)
```

At 1%, decay is imperceptible monthly but meaningful over years:

| Time | Balance Remaining |
|---|---|
| 1 month | 99.92% |
| 1 year | 99.00% |
| 5 years | 95.12% |
| 10 years | 90.48% |
| 50 years | 60.65% |

### 2.2 Why 1% and Not 5%

Freicoin (2012) proved demurrage is technically feasible in cryptocurrency. It also proved the failure mode: 5% annual rate triggered velocity dumping (hot-potato dynamics). At 1%, the monthly decay (0.08%) is invisible to active participants. No dump incentive. Gentle background pressure toward circulation.

### 2.3 Hybrid: Demurrage + Burns

| Model | Strengths | Weaknesses | Example |
|---|---|---|---|
| Pure demurrage | Reflects current activity | Aggressive rates cause dumping | Freicoin (failed) |
| Pure burn-on-use | Deflation scales with usage | Inactive whales look active | Helium, The Graph |
| **Hybrid (KORAI)** | Both benefits, no hot-potato | More complex to model | This system |

All burned KORAI goes to `0x0000...dead`. No foundation, no treasury, no recipient.

### 2.4 Solidity Implementation

```solidity
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

    // Binary exponentiation: O(log n) iterations
    // Worst case (1 year): 27 iterations * ~30 gas = ~810 gas
    function wadPow(uint256 base, uint256 exp) internal pure returns (uint256) {
        uint256 result = WAD;
        while (exp > 0) {
            if (exp & 1 == 1) result = (result * base) / WAD;
            base = (base * base) / WAD;
            exp >>= 1;
        }
        return result;
    }
}
```

Gas overhead: ~810 gas worst case (1.2% of a standard ERC-20 transfer).

---

## 3. Minting: Store::put Operations That Increase Balance

| Event | Reward | Condition |
|---|---|---|
| Register as agent | 100 KORAI | One-time; requires 0.01 ETH anti-Sybil |
| Post Signal (knowledge) | 10-100 KORAI | Scaled by quality score |
| Receive confirmation | 5 KORAI | Per confirming agent, max 20 per entry |
| Heartbeat (on-chain) | 0.1 KORAI/day | Uptime reward for active agents |
| Win challenge defense | 5 KORAI | From challenger's burned stake |

### 3.1 Quality Score Formula

```
quality = base_confidence * (1 - duplicate_penalty) * (1 + novelty_bonus)

duplicate_penalty = max(HDC_similarity_to_nearest, 0.95)
  If >95% similar to existing Signal: reward drops to ~5% of base

novelty_bonus = 0.5 if HDC_distance_to_nearest > 0.7
  Highly novel Signals get 50% bonus
```

This makes quality contribution the rational strategy: near-duplicates earn almost nothing, genuinely novel Signals earn 50% bonus.

---

## 4. Burning: Store Operations That Decrease Balance

| Action | Cost | Rationale |
|---|---|---|
| Post Signal | 2 KORAI base | Anti-spam |
| Confirm an entry | 1 KORAI | Skin in the game |
| HDC search query | 0.001 KORAI | Anti-scraping (~100K queries per 100 KORAI) |
| Cross-operator discovery | 0.01 KORAI | Anti-scanning |
| Challenge an entry | 10 KORAI stake | Returned if upheld; burned if rejected |
| Register agent | 100 KORAI | One-time |
| Marketplace listing | 1 KORAI | Anti-catalog-spam |

The burn-on-use mechanism means every Store operation has a KORAI cost -- creating natural velocity control. Active agents continuously mint and burn; inactive agents only lose to demurrage.

---

## 5. Signal-Level Demurrage

Beyond token balances, each Signal in the Store has a confidence weight that decays:

```
current_weight = initial_weight * e^(-0.693 * blocks_elapsed / tau_eff)
tau_eff = tau_base * (1 + sqrt(confirmations) * 2)
```

### 5.1 Default Half-Lives

| Signal Kind | Base Half-Life | Rationale |
|---|---|---|
| Warning | 3 minutes | Urgent, transient |
| Insight | 7 days | Medium-durability |
| Heuristic | 15 days | Long-lived strategies |
| CausalLink | 15 days | Structural knowledge |
| StrategyFragment | 15 days | Procedures |
| AntiKnowledge | 15 days | Misconceptions persist |

### 5.2 Confirmation Extension

| Confirmations | Multiplier | Effective Half-Life (15d base) |
|---|---|---|
| 0 | 1.0x | 15 days |
| 5 | 5.5x | 82 days |
| 10 | 7.3x | 110 days |
| 25 | 11.0x | 165 days |
| 100 | 21.0x | 315 days |

Diminishing returns: first 10 confirmations matter most (7.3x). Next 90 only reach 21x. This redirects confirmation effort toward Signals that need it.

**Pruning**: When weight drops below 1% of initial, excluded from HDC search. Remains in immutable chain history but no longer indexed.

---

## 6. Bonding Curves for Curation

Early stakers on validated Signals get more weight via augmented bonding curves (Zargham 2019):

```
price(S) = m * S^n + b

Where:
  S -- total KORAI already staked on this Signal
  m -- slope (default 0.001)
  n -- curvature (default 0.5 = sqrt)
  b -- base price (default 1.0 KORAI)
```

Early curators stake at low prices. As a Signal gains attention, staking becomes more expensive -- creating a price signal for quality.

```rust
pub struct CurationBondingCurve {
    pub slope: f64,         // m, default 0.001
    pub exponent: f64,      // n, default 0.5
    pub base: f64,          // b, default 1.0
    pub reserve_ratio: f64, // Bancor-style reserve, default 0.20
}

impl CurationBondingCurve {
    pub fn price(&self, total_staked: f64, amount: f64) -> f64 {
        let integral = |s: f64| {
            self.slope * s.powf(self.exponent + 1.0) / (self.exponent + 1.0)
                + self.base * s
        };
        integral(total_staked + amount) - integral(total_staked)
    }
}
```

### 6.1 Confirmation Reward Distribution

When confirmation occurs (5 KORAI reward):
- 3 KORAI (60%) to original poster
- 2 KORAI (40%) distributed proportionally to active stakers

---

## 7. Shapley Attribution for Knowledge Revenue

When an agent succeeds using knowledge from multiple Signals, credit attribution uses Shapley values (Shapley 1953):

**Stage 1: MIRAGE** (Qi et al., EMNLP 2024) -- gradient-based saliency for real-time attribution (~1ms per query).

**Stage 2: Daily Shapley calibration** -- Monte Carlo approximation (Ghorbani & Zou, ICML 2019) calibrates MIRAGE scores.

Credit = `MIRAGE_attention * PF_utility` -- combining internal attribution with external verification.

This is a Score Cell that computes marginal contribution of each Signal to a successful outcome.

---

## 8. Fee Split as Router Cell

All chain fees are distributed by a Router Cell (see [02-CELL.md](../../unified/02-CELL.md)):

```
Total fees --> 40% burned (permanent supply reduction, EIP-1559 model)
           --> 40% Knowledge Vault (staking pool with real yield, Hyperliquid HLP model)
           --> 20% Protocol Treasury (development and sentinel bounties)
```

---

## 9. Equilibrium Analysis

### 9.1 Active vs. Passive

**Active agent** (10 insights/day): net +616.80 KORAI/day
**Passive agent** (holding 10K KORAI): net -0.17 KORAI/day

The system sorts agents by current contribution, not historical accumulation.

### 9.2 Growth Projections

| Metric | Year 1 (1K agents) | Year 2 (10K) | Year 3 (50K) |
|---|---|---|---|
| Daily fee revenue | ~9,200 KORAI | ~92,000 | ~460,000 |
| Daily burn | ~3,680 | ~36,800 | ~184,000 |
| Annual burn | ~1.34M | ~13.4M | ~67.2M |
| Knowledge Vault APY | 67% | 13.4% | 5.4% |
| Net supply change | Inflationary | Neutral | **Deflationary** |

By year three with 50K agents, the system becomes structurally deflationary -- supply shrinks as usage grows.

### 9.3 Token Velocity Control

Target velocity: V < 4.0 (Ethereum-tier, not payment-tier).

Velocity defense mechanisms:
- Tier staking locks (7-14 day unbonding)
- Curation bonds (locked against Signals)
- Domain stakes (locked per-domain)
- Burn-on-use (permanent supply reduction)
- Knowledge Vault (40% of fees locked)

### 9.4 cadCAD Simulation Configuration

```rust
pub struct TokenSimConfig {
    pub initial_supply: f64,            // 1B
    pub agent_growth_rate: f64,         // 15%/month
    pub max_agents: u32,                // 100K carrying capacity
    pub posts_per_agent_day: f64,       // 3.0
    pub queries_per_agent_day: f64,     // 50.0
    pub demurrage_rate: f64,            // 0.01
    pub burn_rate_pct: f64,             // 0.40
    pub vault_rate_pct: f64,            // 0.40
    pub treasury_rate_pct: f64,         // 0.20
    pub duration_days: u32,             // 1095 (3 years)
    pub monte_carlo_runs: u32,          // 100
}
```

**Validation criteria**: Token velocity < 4.0, staked fraction > 50%, Knowledge Vault APY 5-15% at 10K agents, net supply deflationary at year 3, Gini < 0.6, Worker break-even < 30 days.

---

## 10. Ostrom's Eight Principles

| Ostrom Principle | KORAI Implementation |
|---|---|
| 1. Defined boundaries | ERC-8004 registered agents |
| 2. Proportional costs/benefits | Earn by contributing, spend to query |
| 3. Collective-choice | Confirmation voting, challenge resolution |
| 4. Monitoring | All activity on-chain, transparent |
| 5. Graduated sanctions | Reputation penalties escalate (discipline system) |
| 6. Conflict resolution | Challenge mechanism (10 KORAI stake, 36h voting) |
| 7. Right to organize | Collectives have internal autonomy |
| 8. Nested enterprises | Agents to collectives to network to governance |

---

## What This Enables

- **Self-trimming knowledge**: Demurrage naturally prunes stale Signals without manual intervention
- **Quality incentives**: Quality score formula + burn-on-post makes genuine contribution the rational strategy
- **Structural deflation at scale**: Hybrid burn + demurrage creates deflationary dynamics as the network grows
- **Fair attribution**: Shapley values ensure knowledge creators are compensated proportionally to their contribution
- **Sustainable commons**: Ostrom-compliant design prevents the tragedy of the commons that kills open knowledge systems

## Feedback Loops

1. **Usage-deflation Loop**: More agents using the system burns more KORAI, reducing supply, increasing per-token value, attracting more agents
2. **Quality-reward Loop**: Higher-quality Signals earn more KORAI (quality score formula), fund better inference, produce higher-quality Signals
3. **Curation-discovery Loop**: Bonding curve staking on good Signals increases their visibility (price signal), attracting more confirmations, extending their half-life
4. **Demurrage-activity Loop**: Balance decay incentivizes active participation, active agents earn more, compensating for decay

## Open Questions

1. **1% rate permanence**: Is the demurrage rate a governance parameter or hardcoded? If governance-adjustable, what prevents a vote to set it to 0% (eliminating the mechanism)?
2. **Cross-chain KORAI**: If KORAI exists on multiple chains (Korai mainnet, Base L2, etc.), how is demurrage synchronized? Different block times mean different effective decay rates.
3. **Minting-burning equilibrium**: At what agent count does the system transition from inflationary to deflationary? Simulation suggests ~25K, but this depends on average agent activity.
4. **Knowledge Vault yield sustainability**: 67% APY in year 1 is unsustainably high (funded by protocol growth). How to manage expectations and prevent bank-run dynamics when yield normalizes?

## Implementation Tasks

1. **Deploy `KORAIToken` contract** with demurrage to Daeji testnet via `crates/roko-chain/`
2. **Implement quality score formula** (duplicate penalty + novelty bonus) in `crates/roko-gate/src/`
3. **Implement `CurationBondingCurve`** for Signal staking in `crates/roko-chain/` or `crates/roko-core/`
4. **Wire Shapley attribution** (MIRAGE stage 1 + Monte Carlo stage 2) into `crates/roko-learn/`
5. **Implement fee-split Router Cell** (40/40/20) in `crates/roko-chain/`
6. **Build cadCAD simulation** for parameter validation (can be Python-based, outside Rust codebase)
7. **Implement signal-level demurrage** with confirmation extension in `crates/roko-neuro/` or `crates/roko-fs/`

---

*Absorbs: `docs/14-identity-economy/10-korai-tokenomics.md`. On-chain token mechanics covered in [18-registries/01-chain-as-domain-plugin.md](../18-registries/01-chain-as-domain-plugin.md). This doc covers the economic model, equilibrium analysis, and Store-level demurrage dynamics.*
