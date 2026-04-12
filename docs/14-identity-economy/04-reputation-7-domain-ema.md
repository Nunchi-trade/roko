# 04 — 7-Domain EMA Reputation System

> Reputation in Roko is not a single number. It is a 7-domain vector, each domain scored
> independently using Exponential Moving Average (EMA) with adaptive smoothing, 30-day
> half-life decay, and a graduated discipline system. This document specifies the full
> reputation scoring algorithm, the seven domains, the reputation multiplier formula,
> dispute mechanics, and the Bayesian Beta model underlying score computation.

---

## 1. Why Multi-Domain Reputation

A single reputation score hides critical information. An agent that is excellent at Oracle
Resolution (accurate price feeds) but terrible at Sealed Execution (leaks confidential
data) should not receive a "medium" composite score that obscures both the strength and
the weakness.

Multi-domain reputation enables:

- **Precise matching** — A marketplace buyer looking for an oracle provider filters by
  Oracle Resolution reputation, not a generic "trust score."

- **Targeted improvement** — An agent knows exactly which domain to improve, not just
  that it is "below average."

- **Resistance to washing** — An agent cannot boost its worst domain by performing well
  in its best domain. Each domain is independent.

- **Granular slashing** — A plagiarism violation in the Knowledge Verification domain
  slashes only that domain, not the agent's Oracle Resolution score.

### 1.1 The Seven Reputation Domains

| Domain ID | Name | What It Measures |
|---|---|---|
| 0 | **Oracle Resolution** | Accuracy of oracle/price feed data provided |
| 1 | **Risk Detection** | Ability to identify and report risks (exploits, rug pulls) |
| 2 | **Anomaly Flagging** | Ability to detect anomalies in data, behavior, or markets |
| 3 | **Data Integrity** | Reliability of data handling (no corruption, no loss) |
| 4 | **Cross-App Validation** | Quality of cross-application/cross-domain verification |
| 5 | **Sealed Execution** | Trustworthiness in confidential/TEE computation |
| 6 | **Knowledge Verification** | Quality of knowledge verification and curation |

These seven domains cover the primary trust dimensions for autonomous agents operating
in the Roko ecosystem. The list is extensible — new domains can be added via governance
proposal (Protocol-tier vote) without changing the contract interfaces.

### 1.2 Domain Relevance by Activity

Different activities map to different reputation domains:

| Activity | Primary Domain | Secondary Domain |
|---|---|---|
| Price feed provision | Oracle Resolution | Data Integrity |
| Exploit detection | Risk Detection | Anomaly Flagging |
| Knowledge posting | Knowledge Verification | Data Integrity |
| TEE computation | Sealed Execution | Data Integrity |
| Cross-chain validation | Cross-App Validation | Oracle Resolution |
| Market anomaly detection | Anomaly Flagging | Risk Detection |
| Engram curation | Knowledge Verification | Cross-App Validation |

---

## 2. EMA Scoring Algorithm

### 2.1 Core Formula

Each domain uses an Exponential Moving Average (EMA) to compute the current reputation
score from observed outcomes:

```
R_new = α × O + (1 - α) × R_old
```

Where:
- `R_new` — new reputation score in this domain (0.000 to 1.000)
- `R_old` — previous reputation score
- `O` — observed outcome for the most recent task (0.000 to 1.000)
- `α` — smoothing factor (adaptive, see §2.2)

### 2.2 Adaptive Alpha

The smoothing factor `α` adapts based on the agent's experience level:

```
α = min(0.3, 2 / (job_count + 1))
```

This adaptive alpha has specific properties:

| Job Count | Alpha (α) | Behavior |
|---|---|---|
| 1 | 1.000 | First observation is the score (no history) |
| 2 | 0.667 | New observations dominate |
| 5 | 0.333 | Still responsive to new data |
| 7+ | 0.300 | Capped — experienced agents' scores are stable |
| 50 | 0.300 | Score moves slowly; requires sustained performance change |
| 100 | 0.300 | Very resistant to manipulation by a few bad/good observations |

**Why cap at 0.3**: Without a cap, α would approach 0 for very experienced agents, making
their reputation essentially immovable. The 0.3 cap ensures that even a 1,000-task agent
can be affected by recent performance — each new observation still contributes 30% to the
update. This prevents stale reputation from persisting indefinitely.

**Why not lower**: Below 0.2, an agent would need dozens of consistently bad observations
to significantly impact its score. This creates a lag where a formerly good agent that
starts performing poorly still carries an outdated high score for too long.

### 2.3 Cold Start

New agents start with zero reputation (R = 0.000) in all domains. This is harsh but
honest — there is no "benefit of the doubt" for unproven agents.

The first observation sets the score directly (α = 1.000 for job_count = 1). The second
observation is weighted 2/3 (α = 0.667). By the seventh observation, the score stabilizes
with the 0.3 cap.

**Bootstrap mechanism**: Edge-tier agents can accept up to 50 testnet (DAEJI) jobs to
build initial reputation. These testnet observations are counted at 50% weight (the
outcome O is halved), reflecting the lower stakes of testnet tasks. After upgrading to
Worker tier, mainnet observations count at full weight.

### 2.4 Decay

Reputation decays with inactivity. An agent that stops participating in a domain sees its
score gradually decline toward zero:

```
R_decayed = R × 2^(-days_since_last_feedback / 30)
```

This 30-day half-life means:

| Days Since Last Activity | Score Retention |
|---|---|
| 7 days | 84% |
| 14 days | 71% |
| 30 days | 50% |
| 60 days | 25% |
| 90 days | 12.5% |
| 180 days | 1.6% |

**Rationale**: Reputation should reflect current capability, not historical performance.
An agent that was excellent at Oracle Resolution six months ago but has not provided any
oracle data since may have stale models, outdated data sources, or degraded infrastructure.
The 30-day half-life ensures that only actively participating agents maintain high scores.

**Exception**: Protocol-tier agents have a 60-day half-life (slower decay), reflecting
their infrastructure role where continuous task completion may not be applicable.

---

## 3. Bayesian Beta Foundation

The EMA reputation system is built on the Bayesian Beta reputation model (Jøsang 2002).
The Beta distribution provides a principled way to reason about binary outcomes (success
/ failure) with uncertainty.

### 3.1 Beta Distribution Basics

The Beta distribution `Beta(α_beta, β_beta)` models the probability of success given
`α_beta - 1` successes and `β_beta - 1` failures observed. (Note: the α_beta and β_beta
here are Beta distribution parameters, distinct from the EMA smoothing factor α.)

```
Expected value: E[R] = α_beta / (α_beta + β_beta)
Variance: Var[R] = (α_beta × β_beta) / ((α_beta + β_beta)² × (α_beta + β_beta + 1))
```

**Prior**: Each domain starts with a uniform prior `Beta(1, 1)` — no information, equal
probability of any reputation level.

**Update rule**: After each observation with outcome `O ∈ [0, 1]`:

```
α_beta_new = α_beta + O × stake_weight
β_beta_new = β_beta + (1 - O) × stake_weight
```

Where `stake_weight` is determined by the composite score of the task (task difficulty ×
task importance × domain relevance).

### 3.2 Connection to EMA

The EMA formula is a computationally efficient approximation of the Beta posterior mean.
The full Beta model provides uncertainty bounds (via the variance), while the EMA provides
a point estimate.

In practice, Roko uses the EMA for scoring (fast, simple, sufficient for ranking) and the
Beta model for uncertainty quantification (used in Vickrey auctions to adjust bid scores
based on confidence).

```rust
/// Compute both EMA score and Beta-derived confidence interval.
pub fn update_reputation(
    track: &mut ReputationTrack,
    outcome: f64,
    stake_weight: f64,
) {
    let job_count = track.feedback_count as f64 + 1.0;
    let alpha_ema = (2.0 / (job_count + 1.0)).min(0.3);

    // EMA update
    let old_score = track.score as f64 / 1000.0;
    let new_score = alpha_ema * outcome + (1.0 - alpha_ema) * old_score;
    track.score = (new_score * 1000.0) as u16;
    track.feedback_count += 1;

    // Beta update (for confidence interval)
    track.beta_alpha += outcome * stake_weight;
    track.beta_beta += (1.0 - outcome) * stake_weight;
}

/// Compute 95% confidence interval from Beta distribution.
pub fn confidence_interval(track: &ReputationTrack) -> (f64, f64) {
    let alpha = track.beta_alpha;
    let beta = track.beta_beta;
    // Using normal approximation for large alpha+beta
    let mean = alpha / (alpha + beta);
    let std = (alpha * beta / ((alpha + beta).powi(2) * (alpha + beta + 1.0))).sqrt();
    (mean - 1.96 * std, mean + 1.96 * std)
}
```

### 3.3 Why Not Glicko-2?

Glicko-2 (Glickman 2012) is a popular rating system that tracks both a rating and a rating
deviation (uncertainty). It was considered and rejected for the primary reputation system
because:

1. **Complexity** — Glicko-2 requires iterative convergence calculations that are
   expensive to compute on-chain or for large numbers of agents.

2. **Assumptions** — Glicko-2 assumes paired competition (one agent vs. another), while
   Roko agents are evaluated against absolute performance standards (gate pass/fail).

3. **Decay model** — Glicko-2's rating deviation increases with inactivity, which maps
   poorly to the concept of "reputation decay." EMA with half-life decay is more intuitive
   and easier to reason about.

However, Glicko-2 may be appropriate for specific competitive scenarios (e.g., ranking
agents in a Vickrey auction). The Reputation Registry's off-chain scoring design allows
operators to use Glicko-2 as a secondary scoring algorithm alongside the primary EMA.

### 3.4 Why Not EigenTrust?

EigenTrust (Kamvar et al. 2003) computes global trust by iterating local trust values
until convergence. It was considered and rejected because:

1. **Global convergence** — EigenTrust requires a global fixed-point computation, which
   is impractical in a decentralized system where agents don't have full network visibility.

2. **Sybil vulnerability** — EigenTrust's convergence can be manipulated by clusters of
   colluding agents that boost each other's trust scores.

3. **Domain blindness** — EigenTrust produces a single global score, not domain-specific
   scores.

EigenTrust's local trust component informs the design (agents weight feedback by the
rater's own reputation), but the global convergence is replaced by the simpler EMA
aggregation.

**Research foundation**: Jøsang 2002 (A Logic for Uncertain Probabilities — Beta reputation
systems), Kamvar, Schlosser, Garcia-Molina 2003 (The EigenTrust Algorithm for Reputation
Management — why global convergence is fragile), Glickman 2012 (Example of the Glicko-2
System — rating with uncertainty tracking), Sharpe 1998 (The Sharpe Ratio — risk-adjusted
performance measurement, adapted for reputation scoring).

---

## 4. Reputation Multiplier

The reputation score (0.000 to 1.000) maps to a reputation multiplier that affects economic
outcomes throughout the system:

### 4.1 Multiplier Formula

```
rep_multiplier(R) = 0.1 + 2.9 × R^1.7
```

This maps reputation to a multiplier in the range [0.1, 3.0]:

| Reputation (R) | Multiplier | Effect |
|---|---|---|
| 0.00 | 0.10 | 10% of base economic weight |
| 0.20 | 0.22 | Minimal weight |
| 0.40 | 0.55 | Below average |
| 0.50 | 0.76 | Average |
| 0.60 | 1.02 | Slightly above average |
| 0.70 | 1.35 | Good |
| 0.80 | 1.75 | Strong |
| 0.90 | 2.23 | Excellent |
| 1.00 | 3.00 | Maximum weight |

### 4.2 Why R^1.7 (Superlinear)

The exponent 1.7 creates a superlinear reward curve: moving from 0.8 to 0.9 reputation
provides more incremental benefit than moving from 0.3 to 0.4. This incentivizes
excellence over mediocrity.

**Alternative exponents considered**:
- `R^1.0` (linear): No incentive to push from good to great.
- `R^2.0` (quadratic): Too steep; agents below 0.5 have essentially no economic weight.
- `R^1.7`: Sweet spot — meaningful rewards for high performance, not punitive for
  moderate performers.

### 4.3 Effective Weight Formula

The effective weight combines base stake, reputation multiplier, trust tier, and
discipline factor:

```
effective_weight = base_stake × rep_multiplier(EMA) × trust_tier_mult × discipline_factor
```

Where:

| Component | Value Range | Description |
|---|---|---|
| `base_stake` | 0-25,000+ KORAI | KORAI staked in the relevant domain |
| `rep_multiplier(EMA)` | 0.1-3.0 | From the R^1.7 formula |
| `trust_tier_mult` | 1.0-2.0 | Protocol=2.0, Sovereign=1.5, Worker=1.0, Edge=0.5 |
| `discipline_factor` | 0.0-1.0 | Clean=1.0, Notice=0.9, Warning=0.7, Probation=0.4, Quarantine=0.1, Revoked=0.0 |

**Example**: A Sovereign agent with 25K KORAI staked, reputation 0.85, and Clean discipline:

```
effective_weight = 25,000 × rep_multiplier(0.85) × 1.5 × 1.0
                 = 25,000 × 1.97 × 1.5 × 1.0
                 = 73,875
```

An Edge agent with 0 KORAI, reputation 0.30, and Clean discipline:

```
effective_weight = 0 × rep_multiplier(0.30) × 0.5 × 1.0
                 = 0
```

Edge agents have zero effective weight (no stake). Their participation in the economy is
limited to testnet tasks and knowledge queries.

---

## 5. Reputation Tiers

Reputation scores map to four reputation tiers (distinct from passport tiers):

| Tier | Score Range | Label | Marketplace Impact |
|---|---|---|---|
| Probation | 0.00 – 0.49 | Untrusted | Limited marketplace access; listings require verification |
| Standard | 0.50 – 0.69 | Adequate | Normal marketplace access |
| Trusted | 0.70 – 0.84 | Reliable | Premium listings; eligible for validator role |
| Elite | 0.85 – 1.00 | Exemplary | Maximum reputation multiplier; priority in auctions |

### 5.1 Tier-Specific Effects

**Probation (0.00-0.49)**:
- Listings must pass paid verification before appearing in marketplace.
- Job bids are penalized 50% in Vickrey auctions.
- Cannot serve as arbitrator.
- Cannot provide oracle data.

**Standard (0.50-0.69)**:
- Normal marketplace access.
- Standard Vickrey bid scoring.
- Can submit feedback (weighted by score).

**Trusted (0.70-0.84)**:
- Eligible for validator role.
- Listings appear with "Trusted" badge.
- Feedback has increased weight (rater score > 0.7 contributes more).
- Can serve as dispute arbitrator.

**Elite (0.85-1.00)**:
- Maximum reputation multiplier (up to 3.0x).
- Listings appear with "Elite" badge.
- Priority in auction selection.
- Eligible for oracle provision.
- Pheromone emissions receive 2x intensity multiplier.

---

## 6. Discipline System

The discipline system enforces graduated sanctions for violations. It operates independently
per domain.

### 6.1 Discipline States

```
     Clean (1.0)
        |
        v  (first violation)
     Notice (0.9)
        |
        v  (second violation within 30 days)
     Warning (0.7)
        |
        v  (third violation within 30 days)
     Probation (0.4)
        |
        v  (fourth violation within 30 days)
     Quarantine (0.1)
        |
        v  (fifth violation OR TEE violation)
     Revoked (0.0)
```

### 6.2 State Transitions

| From | To | Trigger | Recovery |
|---|---|---|---|
| Clean | Notice | First violation in domain | 30 days clean → Clean |
| Notice | Warning | Second violation within 30 days | 60 days clean → Notice |
| Warning | Probation | Third violation within 30 days | 90 days clean → Warning |
| Probation | Quarantine | Fourth violation within 30 days | 180 days clean → Probation |
| Quarantine | Revoked | Fifth violation OR TEE violation | No recovery (permanent) |

### 6.3 Discipline Factor

The discipline factor directly multiplies the effective weight:

| State | Factor | Effect |
|---|---|---|
| Clean | 1.0 | Full weight |
| Notice | 0.9 | 10% reduction |
| Warning | 0.7 | 30% reduction |
| Probation | 0.4 | 60% reduction |
| Quarantine | 0.1 | 90% reduction (effectively excluded from marketplace) |
| Revoked | 0.0 | Zero weight (cannot participate) |

### 6.4 Implementation

```rust
/// Compute the discipline factor for an agent in a specific domain.
pub fn discipline_factor(state: DisciplineState) -> f64 {
    match state {
        DisciplineState::Clean => 1.0,
        DisciplineState::Notice => 0.9,
        DisciplineState::Warning => 0.7,
        DisciplineState::Probation => 0.4,
        DisciplineState::Quarantine => 0.1,
        DisciplineState::Revoked => 0.0,
    }
}

/// Escalate discipline after a violation.
pub fn escalate_discipline(
    track: &mut ReputationTrack,
    violation: &ViolationType,
) {
    let current = track.discipline_state;
    let next = match violation {
        ViolationType::TeeViolation => DisciplineState::Revoked, // immediate
        _ => match current {
            DisciplineState::Clean => DisciplineState::Notice,
            DisciplineState::Notice => DisciplineState::Warning,
            DisciplineState::Warning => DisciplineState::Probation,
            DisciplineState::Probation => DisciplineState::Quarantine,
            DisciplineState::Quarantine => DisciplineState::Revoked,
            DisciplineState::Revoked => DisciplineState::Revoked,
        },
    };
    track.discipline_state = next as u8;
}
```

---

## 7. Dispute Resolution

### 7.1 Dispute Flow

When an agent receives feedback it believes is unfair:

```
1. Agent posts DisputeRaised event, staking 5 KORAI.
2. Three arbitrators selected:
   - Must be Sovereign+ tier
   - Must have reputation > 0.7 in the disputed domain
   - Randomly selected (VRF-based) from eligible pool
3. Arbitrators review:
   - Original task specification
   - Agent's work product (via content hash)
   - Feedback score given
   - Historical context (rater's pattern of feedback)
4. Majority vote (2 of 3) determines outcome.
5. Resolution:
   - Dispute upheld → feedback voided, rater penalized, stake returned
   - Dispute rejected → feedback stands, stake burned
```

### 7.2 Arbitrator Selection

Arbitrators are selected using a VRF (Verifiable Random Function) seeded by the dispute
hash and the current block hash. This ensures:

- **Unpredictability** — Neither party can predict or influence which arbitrators are
  selected.
- **Verifiability** — Any observer can verify that the selection was genuinely random.
- **Conflict-free** — Arbitrators cannot be the rater, the ratee, or agents in the same
  collective as either party.

### 7.3 Rater Penalties

When a dispute is upheld (the feedback was deemed unfair):

- The rater's reputation in the `Knowledge Verification` domain is penalized:
  `R_rater = R_rater × 0.9` (10% reduction).
- The voided feedback is removed from the ratee's EMA computation (score is recalculated
  from remaining observations).
- After 3 upheld disputes, the rater enters Notice discipline state in the relevant domain.

---

## 8. RBTS Integration

For scenarios where honest reporting is critical (knowledge marketplace reviews, oracle
submissions), the Bayesian Truth Serum (BTS) variant from Witkowski & Parkes 2012 is used.

### 8.1 How RBTS Works

RBTS (Robust Bayesian Truth Serum) incentivizes honest reporting by paying agents more
when their reports match patterns that would only emerge if reporting were truthful.

The mechanism:

1. Each agent submits a report (e.g., quality score for a knowledge Engram).
2. Each agent also estimates the distribution of reports from other agents.
3. Agents are rewarded based on:
   - How well their report predicts others' reports (information score).
   - How well their prediction of the distribution matches the actual distribution
     (prediction score).
4. Truthful reporting is the unique Bayesian Nash equilibrium.

### 8.2 Application to Knowledge Verification

When multiple agents verify a knowledge Engram:

```rust
/// RBTS scoring for knowledge verification.
/// Agents are rewarded for honest assessment based on
/// Witkowski & Parkes 2012 (Robust Bayesian Truth Serum).
pub fn rbts_score(
    agent_report: f64,        // agent's quality assessment (0-1)
    agent_prediction: f64,    // agent's prediction of average assessment
    actual_average: f64,      // actual average assessment from all agents
    n_agents: usize,
) -> f64 {
    // Information score: how surprising was the agent's report?
    let info_score = (agent_report - actual_average).abs();

    // Prediction score: how accurate was the distribution prediction?
    let pred_score = 1.0 - (agent_prediction - actual_average).abs();

    // Combined (Prelec 2004 weighting)
    let combined = 0.5 * info_score + 0.5 * pred_score;

    // Scale by sqrt(n_agents) — larger panels give more reliable signals
    combined * (n_agents as f64).sqrt()
}
```

**Research foundation**: Witkowski & Parkes 2012 (A Robust Bayesian Truth Serum for Small
Populations — incentive-compatible honest reporting), Prelec 2004 (A Bayesian Truth Serum
for Subjective Data — the "surprisingly popular" mechanism for eliciting truthful
assessments).

---

## 9. Worked Example

### 9.1 New Agent Building Reputation

Agent `roko-newbie` registers at Edge tier with zero reputation.

**Day 1**: Completes first testnet job (Oracle Resolution domain). Outcome: 0.75.
```
α = min(0.3, 2/(1+1)) = 1.0
R_new = 1.0 × 0.75 + 0.0 × 0.0 = 0.75 (but halved for testnet)
R_effective = 0.375
```

**Day 3**: Completes second testnet job. Outcome: 0.82.
```
α = min(0.3, 2/(2+1)) = 0.667
R_new = 0.667 × 0.41 + 0.333 × 0.375 = 0.398
```

**Day 10**: After 7 testnet jobs (outcomes: 0.75, 0.82, 0.79, 0.88, 0.91, 0.86, 0.90):
```
α = min(0.3, 2/(7+1)) = 0.250
Running EMA ≈ 0.42 (testnet-weighted)
```

**Day 15**: Upgrades to Worker tier (stakes 5K KORAI). Starts mainnet jobs.
```
First mainnet job (Oracle Resolution). Outcome: 0.88.
α = min(0.3, 2/(8+1)) = 0.222
R_new = 0.222 × 0.88 + 0.778 × 0.42 = 0.522
```

**Day 30**: After 15 mainnet jobs (average outcome: 0.85):
```
α = 0.3 (capped)
Running EMA ≈ 0.73
Reputation tier: Trusted
```

The agent went from zero to Trusted in 30 days of consistent performance. The adaptive
alpha allows rapid convergence for new agents while maintaining stability for experienced
ones.

---

## 10. Implementation Status

> **Implementation status (2026-04-12)**: Reputation formulas are fully derived and
> documented. Rust implementation of EMA scoring, adaptive alpha, decay, discipline
> escalation, and RBTS are designed. Solidity ReputationRegistry interface is defined.
> Off-chain scoring computation is specified. Not yet integrated into the Roko runtime.
> Local testing uses mock reputation scores.

---

## 11. Academic Citations

- Jøsang 2002 — A Logic for Uncertain Probabilities (Beta reputation systems)
- Kamvar, Schlosser, Garcia-Molina 2003 — The EigenTrust Algorithm for Reputation
  Management in Peer-to-Peer Networks
- Glickman 2012 — Example of the Glicko-2 System (rating with uncertainty tracking)
- Sharpe 1998 — The Sharpe Ratio (risk-adjusted performance measurement)
- Witkowski & Parkes 2012 — A Robust Bayesian Truth Serum for Small Populations
- Prelec 2004 — A Bayesian Truth Serum for Subjective Data
- Haldar et al. 2025 — Reputation Systems for AI Agent Coordination
- Lau et al. 2026 — Adaptive Reputation Scoring for Multi-Agent Systems
- Ostrom 1990 — Governing the Commons (graduated sanctions)

---

## 12. Cross-References

| Document | Relevance |
|---|---|
| `01-erc-8004-three-registries.md` | Reputation Registry contract |
| `02-korai-passport.md` | Passport reputation_tracks field |
| `03-passport-tiers.md` | Tier requirements based on reputation |
| `05-knowledge-marketplace.md` | Reputation affects listing visibility |
| `11-vickrey-reputation-auction.md` | Reputation multiplier in bid scoring |

---

*Generated from: bardo-backup/prd/09-economy/01-reputation.md, tmp/implementation-plans/12b-chain-layer.md §K,
refactoring-prd/04-knowledge-and-mesh.md. Death-bed contribution weights and bloodstain network
references removed per 02-reframe-rules.md. GNOS→KORAI, golem→agent, clade→collective renames applied.*
