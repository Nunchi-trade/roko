# 13 — ISFR Clearing & Settlement

> The Intersubjective Fact Registry (ISFR) enables collective price discovery among
> agents. Cooperative clearing settles cross-agent obligations via a QP solver running
> inside a TEE. DVP (Delivery vs. Payment) ensures atomic settlement. This document
> specifies the ISFR protocol, the clearing engine, the ClearingCertificate with KKT
> optimality proof, the fallback ladder, escrow lifecycle, and settlement mechanics.

---

## 1. ISFR — Intersubjective Fact Registry

### 1.1 What ISFR Solves

In a multi-agent economy, agents need shared reference prices for knowledge, compute,
and services. Without a collective pricing signal, each agent prices independently,
leading to:

- **Price fragmentation** — identical knowledge priced differently by different agents
- **Arbitrage loops** — agents buy low from one source and sell high to another
  without adding value
- **Market instability** — no anchoring mechanism for fair value

ISFR is the agent economy's equivalent of SOFR/LIBOR — a collectively-discovered
reference rate that all agents can use for pricing decisions.

### 1.2 Protocol Overview

ISFR is the simplest collective intelligence signal on Korai (FABRIC V0). Agents
collectively discover fair rates by submitting independent rate estimates after each
clearing round:

```
1. Agent completes a clearing round (job completion, knowledge trade, etc.)

2. Agent publishes IsfrSubmission on korai/isfr gossip topic:
   {
     submitter: AgentId,
     market_id: String,      // which market/domain
     rate: f64,              // agent's observed rate
     confidence: f64,        // how confident (0.0-1.0)
     epoch_id: u64,          // which clearing epoch
     signature: Signature    // Ed25519 signed
   }

3. GossipMesh collects submissions per market

4. When min_submissions threshold met (default: 3):
   → compute median rate (outliers > 3σ excluded)
   → broadcast IsfrAggregate to all subscribers

5. All agents receive aggregate → update local pricing models
```

### 1.3 Data Schema

```rust
pub struct IsfrSubmission {
    pub submitter: u256,           // passport ID
    pub market_id: String,        // e.g., "knowledge/defi", "compute/inference"
    pub rate: f64,                 // observed rate, bounded [-0.1, 0.1] (10% bounds)
    pub components: Vec<f64>,      // must sum to rate within ±1 wei
    pub confidence: f64,           // 0.0-1.0
    pub epoch_id: u64,
    pub signature: Signature,
}

pub struct IsfrAggregate {
    pub market_id: String,
    pub epoch_id: u64,
    pub median_rate: f64,
    pub submission_count: u32,
    pub std_deviation: f64,
    pub excluded_count: u32,       // outliers excluded (> 3σ)
    pub timestamp: u64,
    pub tee_attestation: Hash,     // TEE computed the aggregate
}
```

### 1.4 Rate Computation

```
Given submissions S = {s_1, s_2, ..., s_n} for market M in epoch E:

1. Filter eligibility:
   - submitter must have domain reputation R ≥ 0.5
   - submitter must not be in Quarantine or Revoked discipline state
   - rate must be within bounds: |rate| ≤ 0.1

2. Compute initial median:
   median_0 = median(s_i.rate for i in 1..n)

3. Compute standard deviation:
   σ = sqrt(Σ(s_i.rate - median_0)² / n)

4. Exclude outliers:
   S' = {s_i ∈ S : |s_i.rate - median_0| ≤ 3σ}

5. Compute final rate:
   isfr_rate = weighted_median(S', weights = s_i.confidence × rep_multiplier(R_i))

6. Broadcast IsfrAggregate
```

The confidence-weighted median ensures that high-confidence submissions from
high-reputation agents have more influence, while still preventing any single agent
from dominating the rate.

### 1.5 Update Frequency

ISFR rates update every 8 hours (3 epochs per day). This matches the Korai chain's
clearing cycle and provides a stable reference that doesn't whipsaw on short-term
fluctuations.

### 1.6 Market IDs

Standard market IDs follow a hierarchical naming convention:

```
knowledge/defi          — DeFi knowledge pricing
knowledge/security      — security audit knowledge
knowledge/research      — general research
compute/inference       — LLM inference compute
compute/verification    — re-execution verification
services/code-review    — code review services
services/audit          — smart contract audit
services/analysis       — data analysis
```

Custom market IDs can be registered by Sovereign-tier agents via on-chain governance.

---

## 2. Cooperative Clearing

### 2.1 What Clearing Solves

After multiple agents trade knowledge, services, and compute with each other across
an epoch, a web of mutual obligations exists. Direct bilateral settlement would
require O(n²) transactions. Cooperative clearing **nets** these obligations,
minimizing the number of actual transfers:

```
Without clearing (bilateral):
  Agent A owes Agent B $10
  Agent B owes Agent C $8
  Agent C owes Agent A $5
  → 3 transfers, $23 total flow

With clearing (netted):
  Agent A nets: -$10 + $5 = -$5 (owes $5)
  Agent B nets: +$10 - $8 = +$2 (receives $2)
  Agent C nets: +$8 - $5 = +$3 (receives $3)
  → 3 transfers, $10 total flow (57% reduction)
  → Sum of all net transfers = 0 (zero-sum guarantee)
```

### 2.2 Clearing Protocol

The clearing protocol runs inside a TEE (AWS Nitro enclave) for privacy and
verifiability:

```
Phase 1: COMMIT
  Each agent submits a sealed commitment:
    commit = keccak256(γ, c, I_min, I_max, nonce)

  Where:
    γ     — risk aversion parameter (how much the agent dislikes inventory)
    c     — cost coefficients (per-asset holding costs)
    I_min — minimum inventory constraints (cannot go below)
    I_max — maximum inventory constraints (cannot go above)
    nonce — random value for commitment hiding

  Duration: configurable, default 24 hours for production, 5 minutes for testnet

Phase 2: REVEAL
  Agents reveal their parameters:
    reveal = (γ, c, I_min, I_max, nonce)
  Contract verifies: keccak256(reveal) == commit

  Early reveal penalty: if an agent reveals before the commit phase ends,
  their parameters are visible and exploitable. Penalty: 1% of stake.

  Duration: 12 hours production, 2 minutes testnet

Phase 3: SOLVE (off-chain, in TEE)
  QP solver minimizes total inventory cost:

    minimize Σ_i [γ_i × ||I_i + Δ_i||² + c_i · |Δ_i|]

    subject to:
      I_min_i ≤ I_i + Δ_i ≤ I_max_i    (position bounds)
      Σ_i Δ_i = 0                        (zero-sum: total inventory unchanged)

  Where Δ_i is the net transfer for agent i.

Phase 4: CERTIFICATE
  TEE produces a ClearingCertificate containing:
    - Net transfers Δ_i for each agent
    - KKT (Karush-Kuhn-Tucker) optimality conditions
    - TEE attestation proving computation integrity
    - PU18 precision (18-decimal fixed-point)

Phase 5: VERIFY (on-chain)
  Smart contract verifies KKT conditions in O(n):
    - Stationarity: ∇L = 0 at solution
    - Primal feasibility: I_min ≤ I + Δ ≤ I_max
    - Dual feasibility: λ ≥ 0
    - Complementary slackness: λ × g(x) = 0
  The contract does NOT re-solve the QP — it only checks that the
  certificate proves optimality.

Phase 6: SETTLE
  Net transfers executed atomically (all-or-nothing):
    For each agent i:
      if Δ_i > 0: credit Δ_i to agent i
      if Δ_i < 0: debit |Δ_i| from agent i
    Sum of all Δ_i = 0 (enforced by contract)
```

### 2.3 Soft-Threshold Analytical Solution

The QP solver uses a soft-threshold analytical approach with bisection for the dual
variable λ*, avoiding iterative QP solvers that are harder to verify:

```
Algorithm: Soft-Threshold Bisection

1. For a given λ (Lagrange multiplier for the zero-sum constraint):
   Each agent's optimal Δ_i has a closed-form solution:
     Δ_i(λ) = soft_threshold(-(I_i + λ/γ_i), c_i/γ_i)

   where soft_threshold(x, t) = sign(x) × max(|x| - t, 0)

2. Find λ* such that Σ_i Δ_i(λ*) = 0:
   Binary search (bisection) on λ
   Convergence: O(80) iterations (for PU18 precision)
   Total complexity: O(80 × n) = O(n) effectively

3. Verify:
   Result matches brute-force QP within PU18 precision (18-decimal)
```

This algorithm is deterministic, verifiable, and efficient. The 80-iteration bisection
converges to machine precision for the fixed-point arithmetic used on-chain.

### 2.4 ClearingCertificate

```rust
pub struct ClearingCertificate {
    pub epoch_id: u64,
    pub participants: Vec<u256>,           // passport IDs
    pub net_transfers: Vec<i128>,          // Δ_i in base units (sum = 0)
    pub kkt_stationarity: Vec<f64>,        // ∇L values (should be ~0)
    pub kkt_primal_feasibility: bool,      // all constraints satisfied
    pub kkt_dual_feasibility: Vec<f64>,    // λ values (all ≥ 0)
    pub kkt_complementary_slackness: f64,  // λ·g(x) product (should be ~0)
    pub tee_attestation: TeeAttestation,   // AWS Nitro attestation
    pub precision: u8,                     // 18 (PU18)
    pub solve_time_ms: u64,               // solver duration
}

pub struct TeeAttestation {
    pub enclave_id: [u8; 32],
    pub pcr0: [u8; 48],                   // platform configuration register
    pub timestamp: u64,
    pub signature: Signature,              // attestation signature
}
```

### 2.5 On-Chain Verification

The on-chain verification contract checks KKT conditions without re-solving:

```solidity
function verifyClearingCertificate(
    ClearingCertificate calldata cert
) external view returns (bool) {
    // 1. Zero-sum check
    int256 sum = 0;
    for (uint i = 0; i < cert.netTransfers.length; i++) {
        sum += cert.netTransfers[i];
    }
    require(sum == 0, "NOT_ZERO_SUM");

    // 2. Primal feasibility
    require(cert.kktPrimalFeasibility, "PRIMAL_INFEASIBLE");

    // 3. Dual feasibility (all multipliers non-negative)
    for (uint i = 0; i < cert.kktDualFeasibility.length; i++) {
        require(cert.kktDualFeasibility[i] >= 0, "DUAL_INFEASIBLE");
    }

    // 4. Complementary slackness (product near zero)
    require(
        cert.kktComplementarySlackness < 1e-12,
        "SLACKNESS_VIOLATED"
    );

    // 5. Stationarity (gradient near zero)
    for (uint i = 0; i < cert.kktStationarity.length; i++) {
        require(
            abs(cert.kktStationarity[i]) < 1e-12,
            "STATIONARITY_VIOLATED"
        );
    }

    // 6. TEE attestation
    require(
        verifyTeeAttestation(cert.teeAttestation),
        "INVALID_ATTESTATION"
    );

    return true;
}
```

Gas cost: O(n) where n is the number of participants. For 100 agents, approximately
50,000 gas.

---

## 3. Fallback Ladder

### 3.1 Deterministic Fallback Chain

If the QP solve fails, a deterministic fallback ladder activates. Each step is tried
in order; the first success terminates the ladder:

```
Step 1: Full QP Solve
  → Standard solve with all participants
  → Success? → Produce certificate → Done

Step 2: Pruned Solve
  → Remove smallest 10% of positions (by absolute value)
  → Retry QP with reduced set
  → Pruned agents keep their current positions (no change)
  → Success? → Produce certificate (marks pruned agents) → Done

Step 3: External Hedge
  → Route excess inventory to external venue (DEX, OTC)
  → Adjust positions to make QP feasible
  → Retry QP with adjusted constraints
  → Success? → Produce certificate + hedge receipt → Done

Step 4: Safe Mode
  → Freeze all positions (no transfers this epoch)
  → Notify governance
  → All agents retain current positions
  → Flag epoch as "failed clearing" in on-chain record
  → Governance must approve resolution before next clearing
```

### 3.2 Fallback Statistics

In simulation (from mechanism design analysis):

```
Step 1 (Full solve):     ~95% of epochs succeed
Step 2 (Pruned solve):   ~4% of epochs (handles corner cases)
Step 3 (External hedge): ~0.9% of epochs (rare market stress)
Step 4 (Safe mode):      ~0.1% of epochs (extreme conditions only)
```

---

## 4. Escrow Lifecycle

### 4.1 ERC-8183 Escrow

All job payments flow through ERC-8183 escrow. The lifecycle:

```
1. POST JOB
   Requester calls postJob(bountySpec)
   → Contract transfers max_budget_usdc from requester to escrow
   → Job state = Open
   → Event: JobPosted(job_id, requester, budget)

2. CLAIM JOB
   Winning agent calls claimJob(job_id)
   → Agent's domain stake is locked (anti-abandonment)
   → Job state = Claimed
   → Event: JobClaimed(job_id, agent_id)

3. SUBMIT RESULT
   Agent calls submitResult(job_id, delivery_hash, evidence)
   → Job state = Running → PendingVerification
   → Event: ResultSubmitted(job_id, delivery_hash)

4a. APPROVE (automatic or consortium)
   Gate pipeline passes → resolveJob(job_id, approved, quality_score)
   → Payment released to agent (minus fees)
   → Agent stake unlocked
   → Reputation updated
   → Job state = Completed
   → Event: JobCompleted(job_id, payment, quality_score)

4b. REJECT
   Gate pipeline fails → resolveJob(job_id, rejected, quality_score)
   → Slash applied to agent stake (per slash schedule)
   → Budget refunded to requester (minus posting fee)
   → Reputation penalty applied
   → Job state = Failed
   → Event: JobFailed(job_id, slash_amount, reason)

4c. TIMEOUT
   Deadline passes with no submission
   → Slash applied (2% for abandoned)
   → Budget refunded to requester (minus posting fee)
   → Job state = Failed
   → Event: JobTimeout(job_id)

5. DISPUTE (optional)
   Agent disputes rejection within 24 hours:
   → 3-agent panel selected (R ≥ 0.6, independent)
   → Commit-reveal vote (24h commit, 12h reveal)
   → Majority decides
   → Loser pays dispute fee (5 KORAI anti-spam)
   → Event: DisputeResolved(job_id, verdict)
```

### 4.2 Slash Distribution

When an agent is slashed, the slashed funds are distributed:

```
Slash amount → 50% to requester (compensation)
             → 30% burned (deflationary)
             → 20% to consortium (if applicable, else burned)
```

### 4.3 Staking Requirements

```
Minimum agent stake: 1,000 DAEJI (testnet) / 1,000 KORAI (mainnet)

Effective stake scales with job value:
  effective_stake = max(base_stake, min(job_value × 0.10, 100_000))

Lockup period: 7 days after deregistration
  → prevents stake-and-run attacks
  → agent cannot withdraw stake while any jobs are in progress
```

---

## 5. DVP — Delivery vs. Payment

### 5.1 Atomic Settlement

DVP ensures that delivery and payment happen atomically — neither party can cheat:

```
DVP Protocol:
  1. Agent submits delivery_hash (BLAKE3 of output)
  2. Contract verifies delivery_hash is non-zero
  3. Gate pipeline verifies quality
  4. If quality ≥ threshold:
     → atomically: (a) mark delivery as accepted AND (b) transfer payment
  5. If quality < threshold:
     → atomically: (a) mark delivery as rejected AND (b) apply slash
  6. No state where delivery is accepted but payment is pending
     No state where payment is sent but delivery is not verified
```

### 5.2 Multi-Party DVP

For consortium-validated jobs, DVP extends to include validator payments:

```
On successful completion:
  reward = job_budget - posting_fee
  agent_payment = reward × 0.93           (after 5% validation + 2% protocol)
  validator_pool = reward × 0.05          (split among consortium)
  protocol_fee = reward × 0.02            (to treasury)

  All three distributions happen in a single atomic transaction.
```

---

## 6. Settlement Batching

### 6.1 Epoch-Based Settlement

Rather than settling each job individually, Roko batches settlements by epoch
(every 8 hours, aligned with ISFR updates):

```
Epoch settlement:
  1. Collect all completed/failed jobs in epoch
  2. Compute net transfers per agent
  3. Apply cooperative clearing (§2)
  4. Execute netted transfers atomically
  5. Publish settlement report on-chain

Benefits:
  - Gas efficiency: 1 batch tx vs. N individual txs
  - Netting: reduces total transfer volume by 40-60%
  - Atomicity: all-or-nothing per epoch
```

### 6.2 Urgent Settlement

Jobs marked as `urgent: true` bypass epoch batching and settle immediately after
gate verification. This costs more gas but provides instant payment to the agent.

Urgent settlement fee: 0.1% of payment (on top of standard fees).

---

## 7. Fee Economics

### 7.1 Fee Flow Diagram

```
Requester posts job with budget B
  │
  ├─ Posting fee: B × 0.5% → Protocol Treasury
  │
  └─ Escrowed: B × 99.5%
       │
       ├─ On completion:
       │    ├─ Validation fee: B × 5% → Consortium validators
       │    ├─ Protocol fee: B × 2% → Protocol Treasury
       │    ├─ Platform fee: B × 3% → Platform operator
       │    └─ Agent payment: B × 89.5% → Winning agent
       │
       └─ On failure:
            ├─ Slash from agent stake → 50% requester / 30% burn / 20% consortium
            └─ Remaining escrow → Refunded to requester
```

### 7.2 Fee Distribution per Epoch

At the protocol level, accumulated fees are distributed per epoch:

```
Total epoch fees → 40% burned (deflationary pressure on KORAI)
                → 40% Knowledge Vault (funds knowledge mining rewards)
                → 20% Protocol Treasury (funds development, governance)
```

This is the same 40/40/20 split described in `10-korai-tokenomics.md`.

---

## 8. Arbitration Panel

### 8.1 Panel Selection

When a dispute is filed, a 3-agent arbitration panel is selected:

```
Eligibility:
  - Domain reputation R ≥ 0.6
  - Not the executor (agent being disputed)
  - Not the requester (party filing dispute)
  - Not operated by the same entity as either party
  - Not involved in the same job
  - Has completed ≥ 10 jobs in the relevant domain

Selection:
  VRF selects 3 eligible agents
  If fewer than 3 eligible → expand to adjacent domains
  If still fewer than 3 → escalate to governance

Compensation:
  Each panelist receives: dispute_fee / 3 (from loser)
  Plus: reputation bonus (+0.01) for participation
```

### 8.2 Commit-Reveal Voting

```
Phase 1: COMMIT (24 hours)
  Each panelist submits:
    commit = keccak256(vote, justification_hash, nonce)

  vote ∈ {approve_agent, approve_requester, split}

Phase 2: REVEAL (12 hours)
  Each panelist reveals:
    (vote, justification_hash, nonce)
  Contract verifies: keccak256(reveal) == commit

Phase 3: RESOLUTION
  Majority vote wins (2 of 3)
  If split → proportional resolution based on quality score

  Losing party pays dispute fee: 5 KORAI
  If requester loses 3+ disputes in 30 days → reputation penalty (-0.05)
  If agent loses 3+ disputes in 30 days → discipline escalation
```

---

## 9. Implementation Status

> **Implementation status (2026-04-12)**: ISFR protocol is designed with data schemas,
> rate computation algorithm, and gossip topic structure. Cooperative clearing protocol
> is specified with commit-reveal phases, QP solver algorithm (soft-threshold bisection),
> ClearingCertificate structure with KKT conditions, and on-chain verification contract.
> Fallback ladder is defined (4 steps). Escrow lifecycle, DVP, and fee economics are
> documented. Not yet implemented. Chain infrastructure (Korai/Daeji, mirage-rs mock)
> is a prerequisite. ISFR is tracked as Tier 6 (P3) in the implementation plan.

---

## 10. Academic Citations

- Boyd & Vandenberghe 2004 — Convex Optimization (QP formulation, KKT conditions)
- Karush 1939, Kuhn & Tucker 1951 — KKT optimality conditions
- Vickrey 1961 — Counterspeculation, Auctions, and Competitive Sealed Tenders
- Myerson 1981 — Optimal Auction Design
- Arrow & Debreu 1954 — Existence of equilibrium for a competitive economy
- Duffie & Zhu 2011 — Does a central clearing counterparty reduce risk?
- Ousterhout 2013 — Power-of-two-choices load balancing

---

## 11. Cross-References

| Topic | Document |
|---|---|
| KORAI tokenomics and fee split | `10-korai-tokenomics.md` |
| Vickrey auction and scoring | `11-vickrey-reputation-auction.md` |
| Three hiring models | `12-three-hiring-models.md` |
| Reputation system and slashing | `04-reputation-7-domain-ema.md` |
| Passport tiers and staking | `03-passport-tiers.md` |
| x402 micropayments | `08-x402-micropayments.md` |
| Knowledge futures market | `14-knowledge-futures-market.md` |

---

*Generated from: tmp/implementation-plans/12b-chain-layer.md §N/§O, collaboration/docs/marketplace/specs/mechanism-design.md,
collaboration/docs/marketplace/specs/onchain-offchain-protocol.md, collaboration/docs/marketplace/specs/output-materialization.md,
bardo-backup/prd/09-economy/04-coordination.md. All naming renames applied.*
