# Reputation as Score Protocol

> Depth for [21-MARKETPLACE.md](../../unified/21-MARKETPLACE.md). Covers the 7-domain EMA reputation system, reputation multiplier, discipline state machine, EigenTrust hybrid, collusion detection, and Bayesian Truth Serum -- all expressed as compositions of Score, Verify, Route, and React Cells.

---

## 1. Reputation as Composition of Score Cells

Reputation in Roko is not a single number. It is a 7-domain vector where each domain is an independent Score Cell implementing the Score protocol (see [02-CELL.md](../../unified/02-CELL.md) for protocol definition). Each Score Cell maintains its own state, learns from outcomes via predict-publish-correct, and feeds into Route Cells that use reputation to weight candidate selection.

### 1.1 The Seven Reputation Domains

| Domain ID | Name | What It Measures |
|---|---|---|
| 0 | Oracle Resolution | Accuracy of oracle/price feed data |
| 1 | Risk Detection | Ability to identify risks (exploits, rug pulls) |
| 2 | Anomaly Flagging | Ability to detect anomalies in data/behavior/markets |
| 3 | Data Integrity | Reliability of data handling |
| 4 | Cross-App Validation | Quality of cross-application verification |
| 5 | Sealed Execution | Trustworthiness in confidential/TEE computation |
| 6 | Knowledge Verification | Quality of knowledge verification and curation |

These are extensible via governance proposal (Protocol-tier vote) without changing contract interfaces.

### 1.2 Why Multi-Domain

A single score hides critical information. An agent excellent at Oracle Resolution but terrible at Sealed Execution should not receive a "medium" composite that obscures both strength and weakness. Multi-domain enables:

- **Precise matching**: marketplace buyers filter by the specific domain that matters
- **Targeted improvement**: agents know exactly which domain to improve
- **Resistance to washing**: excellence in one domain cannot compensate for failure in another
- **Granular slashing**: violations affect only the relevant domain

---

## 2. EMA Score Cell Internals

### 2.1 Core Formula

Each domain Score Cell computes reputation via Exponential Moving Average:

```
R_new = alpha * O + (1 - alpha) * R_old
```

Where:
- `R_new` -- new score (0.000 to 1.000)
- `R_old` -- previous score
- `O` -- observed outcome for the most recent task (0.000 to 1.000)
- `alpha` -- adaptive smoothing factor

### 2.2 Adaptive Alpha

```
alpha = min(0.3, 2 / (job_count + 1))
```

| Job Count | Alpha | Behavior |
|---|---|---|
| 1 | 1.000 | First observation is the score |
| 2 | 0.667 | New observations dominate |
| 5 | 0.333 | Still responsive |
| 7+ | 0.300 | Capped -- experienced agents are stable |
| 100 | 0.300 | Resistant to manipulation by few bad observations |

**Why cap at 0.3**: Without a cap, alpha approaches 0 for experienced agents, making reputation immovable. The 0.3 cap ensures each new observation contributes 30% -- a formerly good agent that deteriorates will be detected within a handful of observations.

### 2.3 Cold Start

New agents start at R = 0.000 in all domains. No "benefit of the doubt." The first observation sets the score directly (alpha = 1.0). By the seventh observation, the score stabilizes at the 0.3 cap.

**Bootstrap**: Edge-tier agents accept up to 50 testnet (DAEJI) jobs with outcomes counted at 50% weight (O is halved). After upgrading to Worker, mainnet observations count at full weight.

### 2.4 Decay (Demurrage on Reputation)

Reputation decays with inactivity -- the same demurrage principle that applies to all Signals (see [01-SIGNAL.md](../../unified/01-SIGNAL.md)):

```
R_decayed = R * 2^(-days_since_last_feedback / 30)
```

30-day half-life means:

| Days Inactive | Score Retention |
|---|---|
| 7 days | 84% |
| 30 days | 50% |
| 60 days | 25% |
| 180 days | 1.6% |

Exception: Protocol-tier agents have 60-day half-life (slower decay for infrastructure roles).

---

## 3. Bayesian Beta Foundation

The EMA is a computationally efficient approximation of the Bayesian Beta posterior mean (Josang 2002). The full Beta model provides uncertainty bounds.

### 3.1 Beta Distribution

```
Prior: Beta(1, 1)  -- uniform, no information
Update: alpha_beta += O * stake_weight
        beta_beta += (1 - O) * stake_weight

Expected value: E[R] = alpha_beta / (alpha_beta + beta_beta)
95% CI: mean +/- 1.96 * sqrt(alpha_beta * beta_beta / ((a+b)^2 * (a+b+1)))
```

In practice: EMA for scoring (fast, sufficient for ranking), Beta model for uncertainty quantification (used in Vickrey auctions to adjust bid scores based on confidence).

```rust
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
```

---

## 4. Reputation Multiplier: Score-to-Route Bridge

The reputation score maps to an economic multiplier via:

```
rep_multiplier(R) = 0.1 + 2.9 * R^1.7
```

This is the **bridge between Score and Route** -- Route Cells use the multiplier to weight candidates.

| Reputation (R) | Multiplier | Economic Effect |
|---|---|---|
| 0.00 | 0.10 | 10% of base weight |
| 0.40 | 0.55 | Below average |
| 0.60 | 1.02 | Slightly above average |
| 0.80 | 1.75 | Strong |
| 1.00 | 3.00 | Maximum weight |

### 4.1 Why R^1.7 (Superlinear)

The exponent 1.7 creates superlinear returns: moving from 0.8 to 0.9 provides more incremental benefit than 0.3 to 0.4. This incentivizes excellence over mediocrity. R^1.0 (linear) provides no incentive to push from good to great. R^2.0 (quadratic) is too steep, penalizing moderate performers excessively. R^1.7 is the sweet spot.

### 4.2 Effective Weight

The full effective weight combines multiple factors:

```
effective_weight = base_stake * rep_multiplier(EMA) * trust_tier_mult * discipline_factor

Where:
  base_stake:       0-25,000+ KORAI staked in domain
  rep_multiplier:   0.1-3.0 from R^1.7 formula
  trust_tier_mult:  Protocol=2.0, Sovereign=1.5, Worker=1.0, Edge=0.5
  discipline_factor: Clean=1.0, Notice=0.9, Warning=0.7, Probation=0.4,
                     Quarantine=0.1, Revoked=0.0
```

---

## 5. Discipline System as React Cell State Machine

The discipline system is a state machine React Cell that responds to Score thresholds. It operates independently per domain and implements the React protocol (see [02-CELL.md](../../unified/02-CELL.md)).

### 5.1 State Transitions

```
Clean (1.0) --[first violation]--> Notice (0.9)
  --[2nd within 30d]--> Warning (0.7)
  --[3rd within 30d]--> Probation (0.4)
  --[4th within 30d]--> Quarantine (0.1)
  --[5th or TEE violation]--> Revoked (0.0)
```

Recovery requires sustained clean operation:
- Notice to Clean: 30 days
- Warning to Notice: 60 days
- Probation to Warning: 90 days
- Quarantine to Probation: 180 days
- Revoked: permanent, no recovery

### 5.2 TEE Violations

TEE violations cause immediate Revoked state regardless of current discipline level. This reflects the severity of hardware trust breaches.

```rust
pub fn escalate_discipline(
    track: &mut ReputationTrack,
    violation: &ViolationType,
) {
    let next = match violation {
        ViolationType::TeeViolation => DisciplineState::Revoked,
        _ => match track.discipline_state {
            DisciplineState::Clean => DisciplineState::Notice,
            DisciplineState::Notice => DisciplineState::Warning,
            DisciplineState::Warning => DisciplineState::Probation,
            DisciplineState::Probation => DisciplineState::Quarantine,
            DisciplineState::Quarantine | DisciplineState::Revoked
                => DisciplineState::Revoked,
        },
    };
    track.discipline_state = next;
}
```

---

## 6. EigenTrust Hybrid: Recursive Score Composition

While pure EigenTrust was rejected for primary scoring (requires global convergence, vulnerable to Sybil clusters, domain-blind), a localized EigenTrust computation weights incoming feedback by rater trust. This is **recursive Score composition**: a Score Cell for rater trust feeds into a Score Cell for reputation.

### 6.1 Local EigenTrust

```rust
pub struct LocalEigenTrust {
    pub max_hops: u32,        // default 3, range [2, 5]
    pub damping: f64,         // default 0.5, range [0.3, 0.7]
    pub pre_trusted: Vec<u256>, // Protocol-tier agents
}
```

The rater trust score dampens feedback influence:

```
R_new = (alpha * rater_trust * O) + (1 - alpha * rater_trust) * R_old
```

When `rater_trust = 1.0`, this reduces to standard EMA. When `rater_trust = 0.3`, the observation's influence is dampened by 70%.

### 6.2 Configuration

| Parameter | Default | Range | Effect |
|---|---|---|---|
| max_hops | 3 | [2, 5] | Deeper = more global, slower |
| damping | 0.5 | [0.3, 0.7] | Higher = more network trust weight |
| pre_trusted count | 10-20 | [5, 50] | More seeds = more robust |
| refresh_interval | 1 hour | [10 min, 24h] | Recomputation frequency |

---

## 7. Collusion Detection as Lens Cell

Collusion detection is a Lens Cell (Cell + Observe protocol) that observes Score patterns without modifying them. See [15-TELEMETRY.md](../../unified/15-TELEMETRY.md) for the Lens pattern.

### 7.1 Detection Signals

| Signal | Threshold | Weight |
|---|---|---|
| Reciprocity | bidirectional edges / total edges > 0.6 | 0.30 |
| Temporal sync | Pearson correlation of timestamps > 0.8 | 0.25 |
| Dense subgraph | Internal density > 5x random expectation | 0.20 |
| Score inflation | Avg given > 1.5x network average | 0.15 |
| Isolation | External / internal edges < 0.2 | 0.10 |

Combined confidence thresholded at 0.7 for action.

### 7.2 Actions

- **Reduce collective weight**: sqrt(count) instead of linear
- **Reputation penalty**: -0.05 per detection
- **Discipline escalation**: after 3 detections, escalate to Warning
- **Void feedback**: specific feedback events from detected ring removed from EMA

---

## 8. Calibration Loop: Predict-Publish-Correct

Every Score Cell participates in predict-publish-correct (see [07-LEARNING.md](../../unified/07-LEARNING.md)):

```
1. PREDICT: Score Cell predicts next-job performance based on current EMA
2. PUBLISH: Prediction published as Pulse on Bus topic "reputation/predictions/<domain>"
3. OBSERVE: Actual outcome arrives via FeedbackSubmitted event
4. CORRECT: Beta posterior update corrects the Score Cell's internal calibration

Calibration error = |prediction - outcome|
Running calibration tracked per Score Cell
Well-calibrated agents have error < 0.1 after 50+ observations
```

This calibration loop means reputation is not just a score -- it is a prediction about future performance, continuously corrected against reality.

---

## 9. RBTS for Honest Reporting

For scenarios requiring honest reporting (knowledge verification, oracle submissions), Robust Bayesian Truth Serum (Witkowski & Parkes 2012) incentivizes truthfulness:

```rust
pub fn rbts_score(
    agent_report: f64,        // agent's quality assessment
    agent_prediction: f64,    // agent's prediction of average
    actual_average: f64,      // actual average from all agents
    n_agents: usize,
) -> f64 {
    let info_score = (agent_report - actual_average).abs();
    let pred_score = 1.0 - (agent_prediction - actual_average).abs();
    let combined = 0.5 * info_score + 0.5 * pred_score;
    combined * (n_agents as f64).sqrt()
}
```

Truthful reporting is the unique Bayesian Nash equilibrium -- agents maximize payoff by reporting honestly.

---

## What This Enables

- **Domain-specific trust**: buyers filter by the exact capability that matters
- **Superlinear returns to quality**: R^1.7 multiplier rewards excellence
- **Resistance to manipulation**: adaptive alpha, EigenTrust weighting, collusion detection, and RBTS create multiple barriers
- **Self-correcting scores**: predict-publish-correct via Bus means reputation converges to true capability
- **Graduated enforcement**: discipline state machine provides proportional sanctions

## Feedback Loops

1. **Score-Route Loop**: Higher reputation (Score) wins more jobs (Route), providing more opportunities to build reputation
2. **EigenTrust-feedback Loop**: Rater trust scores (Score Cell 1) weight feedback into reputation (Score Cell 2), which determines future rater trust
3. **Calibration Loop**: Predictions about performance are corrected against outcomes, improving the predictive accuracy of the Score Cell itself
4. **Discipline-capability Loop**: Discipline state affects effective weight, which affects job access, which affects violation rate

## Open Questions

1. **Alpha cap sensitivity**: Is 0.3 the right cap? Simulation suggests it works, but real-world adversarial conditions may require adjustment. Should the cap be per-domain?
2. **Cross-domain reputation transfer**: When an agent that is excellent at Oracle Resolution tries Knowledge Verification, should any reputation transfer? Currently domains are fully independent.
3. **Decay rate heterogeneity**: Should different domains have different half-lives? Oracle accuracy may degrade faster than infrastructure knowledge.
4. **Collusion detection false positives**: Domain specialists naturally interact more. How to distinguish organic clustering from collusion?

## Implementation Tasks

1. **Define `ReputationScoreCell`** implementing Score protocol in `crates/roko-gate/src/` or a new `crates/roko-reputation/`
2. **Implement `DisciplineReactCell`** as a React Cell responding to Score thresholds in the same crate
3. **Wire `LocalEigenTrust`** into feedback weighting path, with configurable parameters in `roko.toml`
4. **Implement `CollusionDetectorLens`** as an Observe Cell in `crates/roko-learn/` or `crates/roko-gate/`
5. **Add RBTS scoring** for knowledge verification feedback in `crates/roko-gate/src/`
6. **Integrate predict-publish-correct** for reputation predictions into Bus topics in `crates/roko-runtime/`
7. **Add cadCAD simulation harness** for parameter validation (target: honest avg > 0.7, collusion detection > 0.85, false positive < 0.05)

---

*Absorbs: `docs/14-identity-economy/04-reputation-7-domain-ema.md`, `docs/14-identity-economy/11-vickrey-reputation-auction.md` (reputation aspects). On-chain reputation registry mechanics covered in [18-registries/04-reputation-and-peer-scoring.md](../18-registries/04-reputation-and-peer-scoring.md). This doc covers the off-chain Score Cell dynamics, calibration, and economic effects.*
