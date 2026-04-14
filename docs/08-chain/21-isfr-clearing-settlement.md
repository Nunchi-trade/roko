# ISFR: Clearing and Settlement

> ISFR (Intersubjective Fact Registry) provides collective fact validation and price discovery. The clearing mechanism uses a QP (Quadratic Programming) solver with bisection (O(80n)) to find market-clearing prices. Clearing certificates carry KKT optimality proofs verifiable on-chain.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [10-spore-job-market.md](./10-spore-job-market.md), [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §N (ISFR), §O (Clearing)

---

## Abstract

The ISFR (Intersubjective Fact Registry) is the mechanism by which the Korai agent collective validates facts and discovers prices through aggregated subjective assessments. Rather than relying on a single oracle or a simple average, the ISFR uses a structured process where agents submit weighted opinions, these opinions are aggregated using reputation-weighted scoring, and the result is validated through a clearing mechanism that produces mathematically verifiable optimality proofs.

The clearing mechanism is the settlement layer: it takes a set of orders (job bids, knowledge value claims, reputation adjustments) and finds the prices and allocations that satisfy all constraints while maximizing total welfare. The solver uses Quadratic Programming with bisection, running in O(80n) time, where n is the number of participants. Each clearing result comes with a KKT (Karush-Kuhn-Tucker) optimality certificate that can be verified on-chain in O(n) time.

---

## ISFR: Intersubjective Fact Registry

### What It Is

The ISFR is an on-chain registry where agents submit claims about factual matters and the collective determines truth through weighted aggregation:

- "The current fair price for a 1-hour security audit is 450 KORAI"
- "Agent #42's recent contribution to the coding domain was quality 0.85"
- "The HDC vector for concept X has similarity 0.73 with concept Y"

Individual agents submit their assessments. These are weighted by the agent's reputation in the relevant domain. The aggregate becomes the "intersubjective fact" — the collective's best estimate of truth.

### Why "Intersubjective"?

"Intersubjective" (from philosophy of mind) describes facts that exist through shared agreement rather than objective measurement. The price of a security audit is not an objective physical quantity — it is what the market agrees it should be. Agent reputation is not an objective measurement — it is the collective's assessment of quality. The ISFR makes these intersubjective facts explicit and verifiable.

### Fact Submission

```rust
pub struct FactClaim {
    /// Topic this claim addresses.
    pub topic: FactTopic,

    /// The claimed value (varies by topic type).
    pub value: FactValue,

    /// Confidence in the claim (0.0 to 1.0).
    pub confidence: f64,

    /// The submitting agent's passport ID.
    pub claimant_passport_id: u256,

    /// Domain for reputation weighting.
    pub domain: String,

    /// Block at which this claim was submitted.
    pub submitted_at_block: u64,
}

pub enum FactTopic {
    /// Price discovery for a service type.
    ServicePrice { service_type: String },

    /// Quality assessment of a specific work product.
    QualityAssessment { job_hash: [u8; 32] },

    /// Oracle resolution for a prediction.
    OracleResolution { prediction_id: [u8; 32] },

    /// Custom topic for governance or other uses.
    Custom(String),
}

pub enum FactValue {
    Numeric(f64),
    Boolean(bool),
    Score(f64),      // [0.0, 1.0]
    Price(U256),     // in KORAI wei
}
```

### Reputation-Weighted Aggregation

Individual claims are aggregated using reputation weights:

```
aggregate_value = Σ(w_i × v_i) / Σ(w_i)

where:
  w_i = R_i × c_i × stake_i^0.5
  R_i = agent i's reputation in the claim's domain
  c_i = agent i's confidence in their claim
  stake_i = agent i's domain stake (square root to prevent plutocracy)
  v_i = agent i's claimed value
```

The square root of stake prevents wealthy agents from dominating the aggregate while still giving staked agents more influence than unstaked ones.

---

## Clearing Mechanism

### Problem Statement

The clearing mechanism solves the allocation problem: given N agents with different bids, reputations, and constraints, find the prices and assignments that maximize total welfare subject to:

1. **Budget constraints**: Each agent's payment cannot exceed their budget
2. **Capacity constraints**: Each agent can handle at most K concurrent jobs
3. **Quality constraints**: Assignment quality (reputation × capability match) must exceed minimums
4. **Fairness constraints**: No agent receives disproportionately more or fewer jobs than their capacity allows

This is a constrained optimization problem that can be formulated as Quadratic Programming (QP).

### QP Formulation

```
Maximize:   Σ(q_ij × x_ij) - λ × Σ(x_ij × p_ij)²
Subject to:
  Σ(x_ij) ≤ 1        for each job j  (each job assigned to at most 1 agent)
  Σ(x_ij) ≤ K_i      for each agent i (capacity constraint)
  p_ij × x_ij ≤ B_j   for each job j  (budget constraint)
  x_ij ∈ {0, 1}       (binary assignment)

Where:
  x_ij = 1 if agent i is assigned to job j, 0 otherwise
  q_ij = quality score for agent i on job j (reputation × capability match)
  p_ij = price agent i charges for job j
  B_j  = budget for job j
  K_i  = capacity of agent i
  λ    = fairness parameter (penalizes concentration)
```

The quadratic term `λ × Σ(x_ij × p_ij)²` penalizes solutions where all high-value jobs go to a single agent, promoting distribution of work across the collective.

### Bisection Solver

The QP is solved using bisection on the dual variable (Lagrange multiplier for the budget constraint). The bisection approach:

1. Set lower bound L = 0, upper bound U = max_budget
2. At each iteration, set λ = (L + U) / 2
3. Solve the relaxed problem at this λ (linear after fixing λ)
4. Check if the budget constraint is satisfied
5. If violated: increase λ (tighten constraint), else decrease λ
6. Converge in O(log(1/ε)) iterations

Each relaxed subproblem is O(n) (linear assignment with fixed prices). With ε = 10⁻⁸ (typical numerical precision), this requires ~80 iterations. Total complexity: O(80n) where n = number of agents × number of jobs.

### KKT Optimality Certificate

Each clearing result produces a certificate proving the solution is optimal. The KKT (Karush-Kuhn-Tucker) conditions are:

```
1. Stationarity:    ∂L/∂x_ij = 0 for all active assignments
2. Primal feasibility: all constraints satisfied
3. Dual feasibility:   all Lagrange multipliers ≥ 0
4. Complementary slackness: μ_k × g_k(x) = 0 for all constraints
```

```rust
pub struct ClearingCertificate {
    /// The allocation result: which agent gets which job at what price.
    pub allocations: Vec<Allocation>,

    /// Lagrange multipliers for each constraint.
    pub dual_variables: Vec<f64>,

    /// KKT residual (should be < ε for a valid certificate).
    pub kkt_residual: f64,

    /// Total welfare achieved.
    pub total_welfare: f64,

    /// Block number at which the clearing was computed.
    pub clearing_block: u64,

    /// Merkle root of the full clearing data.
    pub merkle_root: [u8; 32],
}

pub struct Allocation {
    pub agent_passport_id: u256,
    pub job_id: [u8; 32],
    pub price: U256,
    pub quality_score: f64,
}
```

The KKT certificate can be verified on-chain in O(n) time: check that all constraints are satisfied and that the complementary slackness conditions hold. This is much cheaper than solving the QP on-chain (O(80n) with floating-point operations that are gas-prohibitive).

---

## Settlement Flow

```
1. Clearing phase: Off-chain QP solver produces allocations + KKT certificate

2. Submission: Clearing operator submits ClearingCertificate to on-chain contract

3. Verification: On-chain contract verifies KKT conditions (O(n))
   - If valid: proceed to settlement
   - If invalid: reject, slash the clearing operator

4. Settlement: For each allocation:
   - Transfer price from job escrow to agent
   - Deduct marketplace fee (3%)
   - Record work proof in Validation Registry

5. Finalization: Emit ClearingSettled event with Merkle root
```

The clearing operator (a Protocol-tier agent or validator) bears the computation cost but is compensated with a small clearing fee (0.5% of total cleared value).

---

## Academic Foundations

- Karush, W. (1939). "Minima of Functions of Several Variables with Inequalities as Side Conditions." *M.S. thesis, University of Chicago*. — KKT conditions for constrained optimization.
- Kuhn, H.W. and Tucker, A.W. (1951). "Nonlinear Programming." *Proceedings of the Second Berkeley Symposium*. — Formal statement of KKT optimality conditions.
- Boyd, S. and Vandenberghe, L. (2004). *Convex Optimization*. Cambridge University Press. — QP formulation and bisection methods for convex optimization.

---

## Current Status and Gaps

**Scaffold:**
- QP solver concepts well-understood (standard numerical libraries exist)
- KKT verification is straightforward linear algebra

**Not yet built (Tier 6):**
- ISFR fact submission contract (§N1)
- Reputation-weighted aggregation (§N2)
- QP clearing solver (§O1)
- Bisection algorithm implementation (§O2)
- KKT certificate generation (§O3)
- On-chain KKT verification contract (§O4)
- Settlement flow with escrow integration (§O5)

---

## Cross-References

- See [10-spore-job-market.md](./10-spore-job-market.md) for the marketplace that generates clearing inputs
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for the reputation weights used in aggregation
- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for the Validation Registry that stores clearing certificates
- See [02-korai-token-economics.md](./02-korai-token-economics.md) for the KORAI token flows in settlement
