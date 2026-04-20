# Hallucination Audit: Spec vs. Implementation Discrepancies

Master reference for every confirmed discrepancy between the specification documents
(`docs/`, `bardo-backup/tmp/agent-chain-new/`) and the implementation (`crates/`).

Each item is a checklist entry with:
- What the spec says (file path + line reference where possible)
- What the code does (file path)
- What needs to change
- Severity tag

---

## P0: Critical Hallucinations (wrong implementation)

These are cases where the code actively implements something different from what
the spec describes. The code is not just incomplete -- it is incorrect.

---

### P0-01: ISFR implements fact consensus instead of DeFi rate index

**Severity**: P0 -- completely wrong domain and algorithm

**Spec says** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 14-134):
- ISFR = "Intersubjective Fact Registry" but functions as a **collective price discovery**
  mechanism, analogous to SOFR/LIBOR.
- Agents submit `IsfrSubmission` with `market_id` (e.g., `knowledge/defi`, `compute/inference`),
  `rate: f64` (observed rate, bounded [-0.1, 0.1]), `components: Vec<f64>`, `confidence: f64`.
- Aggregation uses **weighted median** (not weighted mean): `isfr_rate = weighted_median(S', weights = s_i.confidence x rep_multiplier(R_i))`.
- Two-level aggregation: first compute initial median, then exclude 3-sigma outliers, then
  compute confidence-weighted median on the filtered set.
- Rates update every 8 hours (3 epochs/day).
- Output is `IsfrAggregate` with `median_rate`, `submission_count`, `std_deviation`,
  `excluded_count`, `tee_attestation`.

Also see (`docs/08-chain/21-isfr-clearing-settlement.md` lines 1-244):
- The older chain-docs version describes similar but slightly different semantics focused on
  "intersubjective facts" via reputation-weighted aggregation.

**Code does** (`crates/roko-chain/src/isfr.rs` lines 1-437):
- Implements a "Intersubjective Fact Registry" that resolves **fact claims** (FactClaim,
  FactValue::Numeric | Boolean | Score | Price) through reputation-weighted aggregation.
- Uses **weighted mean** (weighted average), not weighted median (line 206: `consensus = sum(w_i * v_i) / sum(w_i)`).
- No two-level aggregation (no initial median, no outlier exclusion, no 3-sigma filtering).
- No `market_id` field -- uses `FactTopic` enum (ServicePrice, QualityAssessment) instead.
- No `IsfrSubmission` or `IsfrAggregate` types matching the spec.
- No TEE attestation on output.

**What needs to change**:
- [x] Replace `FactClaim`/`FactValue` types with `IsfrSubmission` matching spec schema
- [x] Replace weighted-mean consensus (`solve_qp`) with weighted-median after 3-sigma outlier exclusion
- [x] Add two-level aggregation pipeline (initial median -> outlier filter -> weighted median)
- [x] Add `market_id` field and hierarchical market naming (`knowledge/defi`, `compute/inference`, etc.)
- [x] Add `IsfrAggregate` output type with `median_rate`, `std_deviation`, `excluded_count`
- [x] Add eligibility filter: submitter reputation >= 0.5, not in Quarantine/Revoked
- [x] Add rate bounds check: `|rate| <= 0.1`

---

### P0-02: ISFR missing data source adapters

**Severity**: P0 -- structural absence

**Spec says** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 118-134):
- Standard market IDs follow hierarchical naming: `knowledge/defi`, `knowledge/security`,
  `compute/inference`, `services/code-review`, etc.
- Custom market IDs can be registered by Sovereign-tier agents via governance.

**Code does** (`crates/roko-chain/src/isfr.rs`):
- No market ID system at all. Uses `FactTopic` enum with two variants (`ServicePrice`, `QualityAssessment`).
- No adapter pattern for different data sources.
- No market registration mechanism.

**What needs to change**:
- [x] Implement hierarchical market ID system with standard and custom IDs
- [x] Add data source adapter trait for different market types
- [x] Add Sovereign-tier market registration via governance

---

### P0-03: ISFR missing liveness state machine

**Severity**: P0 -- structural absence

**Spec says** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 112-116):
- Rates update every 8 hours (3 epochs per day).
- Clearing cycle is time-based with COMMIT/REVEAL/SOLVE/CERTIFICATE/VERIFY/SETTLE phases (lines 161-223).

**Code does** (`crates/roko-chain/src/isfr.rs`):
- `advance_epoch()` is a simple counter increment with no time-based scheduling.
- No phase state machine (no COMMIT/REVEAL/SOLVE/CERTIFICATE/VERIFY/SETTLE).
- No duration tracking, no configurable epoch timing beyond `epoch_duration_secs` (which is not used).

**What needs to change**:
- [x] Implement clearing phase state machine with COMMIT -> REVEAL -> SOLVE -> CERTIFICATE -> VERIFY -> SETTLE
- [x] Add time-based epoch advancement (8-hour default)
- [x] Add phase duration enforcement

---

### P0-04: ISFR missing oracle precompile interface

**Severity**: P0 -- structural absence

**Spec says** (`docs/08-chain/03-hdc-on-chain-precompile.md`, `docs/20-technical-analysis/02-chain-oracles.md` lines 486-512):
- ISFR predictions are published on-chain via `OnChainPredictionStore` at the ISFR contract address.
- Precompile at address `0xA01` handles HDC operations; ISFR is a smart contract.

**Code does** (`crates/roko-chain/src/isfr.rs`):
- No precompile interface.
- No on-chain prediction store integration.
- No contract address concept.

**What needs to change**:
- [ ] Add ISFR contract interface matching spec
- [ ] Wire ISFR to on-chain prediction infrastructure
- [ ] Add precompile interaction for HDC-based similarity checks

---

### P0-05: ISFR missing source health checks and CRPS scoring

**Severity**: P0 -- missing validation and scoring

**Spec says** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 86-106):
- Submitters must have domain reputation R >= 0.5.
- Submitters must not be in Quarantine or Revoked discipline state.
- Rate must be within bounds: `|rate| <= 0.1`.
- `components` must sum to rate within +/- 1 wei.

**Code does** (`crates/roko-chain/src/isfr.rs`):
- Reputation scores are used for weighting (line 174) but no minimum threshold is enforced.
- No discipline state check.
- No rate bounds check.
- No component sum validation.
- No CRPS (Continuous Ranked Probability Score) for proper scoring of distributions.

**What needs to change**:
- [x] Add minimum reputation threshold (R >= 0.5) for submission eligibility
- [x] Add discipline state check (reject Quarantine/Revoked)
- [x] Add rate bounds validation (|rate| <= 0.1)
- [x] Add component sum validation (components must sum to rate within tolerance)
- [ ] Implement CRPS proper scoring rule for evaluating distribution predictions

---

### P0-06: ISFR missing hybrid oracle+market formula

**Severity**: P0 -- missing economic mechanism

**Spec says** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 137-249):
- Cooperative clearing uses a QP solver minimizing total inventory cost.
- Soft-threshold bisection analytical solution with O(80n) convergence.
- ClearingCertificate with KKT optimality proof verified on-chain.
- 6-phase clearing protocol (COMMIT/REVEAL/SOLVE/CERTIFICATE/VERIFY/SETTLE).

**Code does** (`crates/roko-chain/src/isfr.rs` lines 199-218):
- `solve_qp()` computes a simple weighted average (line 206), not a proper QP solution.
- No inventory constraints, no position bounds, no risk aversion parameter.
- No soft-threshold bisection.
- No proper KKT verification (the "KKT residual" just checks stationarity of the trivial weighted mean).

**What needs to change**:
- [ ] Replace naive weighted-average solver with proper soft-threshold bisection QP solver
- [ ] Add agent inventory constraints (I_min, I_max)
- [ ] Add risk aversion parameter per agent
- [ ] Add proper KKT optimality verification (stationarity, primal/dual feasibility, complementary slackness)

---

### P0-07: Reputation domain names wrong (7 domains all different from spec)

**Severity**: P0 -- naming mismatch breaks cross-system references

**Spec says** (`docs/08-chain/14-reputation-system-7-domain.md` lines 24-33):
```
1. coding
2. security
3. research
4. chain
5. knowledge
6. operations
7. strategy
```

**Code does** (`crates/roko-chain/src/reputation_registry.rs` lines 35-43):
```
1. code_quality
2. reliability
3. speed
4. knowledge
5. collaboration
6. security
7. oracle
```

Only `knowledge` and `security` overlap (and `security` is in different positions).
Five of seven domains are completely different. `coding` vs `code_quality`, `research` vs
none, `chain` vs none, `operations` vs none, `strategy` vs none.

**What needs to change**:
- [x] Rename `REPUTATION_DOMAINS` array to match spec: `["coding", "security", "research", "chain", "knowledge", "operations", "strategy"]`
- [x] Update all references throughout `reputation_registry.rs`
- [x] Update all test fixtures and downstream consumers

---

### P0-08: Collusion penalty wrong (50% vs spec's feedback weight dilution)

**Severity**: P0 -- penalty mechanism differs from spec

**Spec says** (`docs/08-chain/14-reputation-system-7-domain.md` lines 300-302):
- "If collusion is detected, all members' feedback weight is reduced by 50% for 30 days."
- This is a **feedback weight dilution** (their future feedback as job posters carries reduced weight in EMA), not a direct reputation slash.

**Code does** (`crates/roko-chain/src/reputation_registry.rs` lines 71-79):
- `Collusion` violation has slash rate of `-0.50` applied directly to the reputation score.
- This is a **direct reputation score penalty**, not a feedback weight dilution.
- No 30-day duration tracking for the penalty.
- No distinction between "reputation slash" and "feedback weight reduction."

Additionally, the spec's `ViolationType` enum (`docs/08-chain/04-korai-passport-erc-721-soulbound.md` lines 85-93)
uses: `MissedDeadline`, `AbandonedJob`, `QualityRejection`, `RepeatedQualityFailure`, `Plagiarism`,
`ResultManipulation`, `TeeViolation`. The code uses: `IncompleteJob`, `QualityFailure`,
`Timeout`, `Collusion` -- a completely different set.

**What needs to change**:
- [x] Change collusion penalty from direct reputation slash to feedback weight dilution (-50% for 30 days)
- [x] Add duration tracking for penalty (30-day window)
- [x] Align `ReputationViolation` enum variants with spec's `ViolationType`
- [x] Add missing violation types: `MissedDeadline`, `AbandonedJob`, `RepeatedQualityFailure`, `Plagiarism`, `ResultManipulation`, `TeeViolation`

---

### P0-09: Passport tier names wrong

**Severity**: P0 -- naming mismatch

**Spec says** (`docs/08-chain/04-korai-passport-erc-721-soulbound.md` lines 100-105):
```
Tier 0: Protocol  (governance-approved)
Tier 1: Sovereign (25,000 KORAI stake)
Tier 2: Worker    (5,000 KORAI stake)
Tier 3: Edge      (no stake)
```
Ordered: Protocol (highest) > Sovereign > Worker > Edge (lowest).

**Code does** (`crates/roko-chain/src/phase2.rs` lines 332-343):
```rust
pub enum PassportTier {
    Protocol,   // strongest privileges
    Sovereign,  // high-trust operators
    Worker,     // normal marketplace (default)
    Edge,       // constrained
}
```
Names match, but the `#[default]` is `Worker` and `PartialOrd` ordering derives from enum declaration order,
making `Protocol < Sovereign < Worker < Edge` which is the **opposite** of the spec's hierarchy
(Protocol should be highest, Edge lowest).

**What needs to change**:
- [x] Reverse the `PartialOrd`/`Ord` derivation or implement manually so `Protocol > Sovereign > Worker > Edge`
- [x] Verify all tier comparisons throughout the codebase respect the correct ordering
- [x] Add stake thresholds as constants matching spec (25,000 / 5,000 / 0)

---

### P0-10: C-Factor/C-Score components diverge from spec

**Severity**: P0 -- measuring different things

**Spec says** (`docs/00-architecture/14-c-factor-collective-intelligence.md` lines 78-85):
Five process variables from Woolley et al.:
```
1. Turn-taking equality    (Pulse authorship entropy on Bus)
2. Social perceptiveness   (peer.prediction vs peer.outcome residuals)
3. Trust calibration       (citation reciprocity + gate survival in Substrate)
4. Channel openness        (Bus delivery confirmation + subscriber reach)
5. Cognitive diversity     (HDC distance across cohort Engrams)
```

With `CohortMetrics`:
```rust
pub struct CohortMetrics {
    pub turn_taking_entropy: f64,
    pub peer_prediction_accuracy: f64,
    pub citation_reciprocity: f64,
    pub delivery_rate: f64,
    pub hdc_diversity: f64,
}
```

**Code does** (`crates/roko-core/src/cfactor.rs` lines 7-29):
`CFactorSummary` has:
```rust
pub overall: f64,
pub trend: f64,
pub regression_drop: f64,
pub gate_pass_rate: f64,          // NOT in spec's 5 variables
pub turn_taking_equality: f64,    // matches
pub social_sensitivity: f64,      // renamed from "social_perceptiveness"
pub task_diversity_coverage: f64,  // renamed from "cognitive diversity"
pub episode_count: usize,
pub top_positive_contributors: Vec<String>,
pub top_negative_contributors: Vec<String>,
```

Missing spec variables:
- `citation_reciprocity` (trust calibration) -- entirely absent
- `delivery_rate` (channel openness) -- entirely absent

Extra code variables not in spec:
- `gate_pass_rate` -- not one of the 5 process variables
- `regression_drop` -- not in spec
- `trend` -- not in spec

The learned weight struct `CohortWeights` (spec lines 123-129) is entirely absent from code.

**What needs to change**:
- [x] Add `citation_reciprocity: f64` to `CFactorSummary`
- [x] Add `delivery_rate: f64` to `CFactorSummary`
- [x] Rename `social_sensitivity` to `social_perceptiveness` per spec
- [x] Rename `task_diversity_coverage` to `hdc_diversity` per spec
- [x] Implement `CohortMetrics` struct matching spec
- [x] Implement `CohortWeights` struct with per-variable weights + bias
- [x] Implement `CohortWeightsLearner` for online weight fitting

---

### P0-11: InsightStore taxonomy mismatch (pheromone kinds vs knowledge entry types)

**Severity**: P0 -- conflating two different type systems

**Spec says**: Two separate taxonomies exist:

1. **Knowledge entry types** (`bardo-backup/tmp/agent-chain-new/04-knowledge-layer.md` lines 9-48):
   - `Insight` (factual observation, 7-day half-life)
   - `Heuristic` (behavioral strategy, 15-day)
   - `Warning` (what NOT to do, 3-minute)
   - `CausalLink` (cause-and-effect, 15-day)
   - `StrategyFragment` (reusable partial plan, 15-day)
   - `AntiKnowledge` (explicitly wrong information, 15-day)

2. **Pheromone kinds** (`docs/13-coordination/04-pheromone-kinds.md`):
   - `Threat`, `Opportunity`, `Wisdom`, `Alpha`, `Pattern`, `Anomaly`, `Consensus`, `Custom(String)`

**Code does**:
- `roko-neuro` correctly has `KnowledgeKind` (`crates/roko-neuro/src/lib.rs` lines 76-93) with the 6 entry types.
- `roko-orchestrator` correctly has `PheromoneKind` (`crates/roko-orchestrator/src/coordination.rs` lines 190-207).
- However, the half-life values differ significantly from the on-chain spec:

| Kind | Spec (on-chain, blocks) | Code (roko-neuro, days) | Discrepancy |
|------|------------------------|------------------------|-------------|
| Insight | 1,512,000 blocks (~7 days) | 30 days | 4.3x longer |
| Heuristic | 3,240,000 blocks (~15 days) | 90 days | 6x longer |
| Warning | 450 blocks (~3 minutes) | 7 days | ~3360x longer |
| CausalLink | 3,240,000 blocks (~15 days) | 60 days | 4x longer |
| StrategyFragment | 3,240,000 blocks (~15 days) | 14 days | ~matches |
| AntiKnowledge | 3,240,000 blocks (~15 days) | 30 days | 2x longer |

The off-chain (roko-neuro) values are defensible -- local knowledge should persist longer than
on-chain knowledge subject to demurrage. But this should be explicitly documented as a
design decision, not an accidental divergence.

**What needs to change**:
- [x] Document the intentional difference between on-chain (spec) and off-chain (roko-neuro) half-lives
- [ ] Add on-chain half-life constants matching the spec for the chain domain
- [x] Fix Warning half-life: 7 days is orders of magnitude longer than the spec's 3 minutes -- this likely IS wrong even for off-chain

---

## P1: Missing Core Features

Features that the spec describes as core functionality that the code does not yet implement.

---

### P1-01: CRPS scoring (proper scoring rule for distributions)

- [x] **Spec** (`docs/20-technical-analysis/13-predictive-foraging-and-active-inference.md`): Predictions should be evaluated using CRPS.
- **FIXED**: Implemented `prediction::crps` module in `roko-core/src/prediction.rs` with:
  - `crps::gaussian(mean, std_dev, observation)` — closed-form for Gaussian forecasts
  - `crps::empirical(samples, observation)` — for sample-based forecasts
  - `crps::uniform(lower, upper, observation)` — closed-form for uniform interval forecasts
  - 8 unit tests covering perfect predictions, error scaling, spread penalty, degenerate cases

### P1-02: TraceRank (graph-based reputation from payment edges)

- [x] **Spec** (`bardo-backup/tmp/agent-chain-new/05-token-economics.md`): Graph-based reputation from payment flows.
- **FIXED**: Implemented `roko_chain::trace_rank` module:
  - `TraceRank` engine with configurable damping, convergence threshold, lookback window
  - `PaymentEdge` type: from/to/amount/quality/block with quality-weighted edges
  - Power iteration (PageRank algorithm) with teleportation for dangling nodes
  - `blend_reputation(ema, trace_rank)` for combining direct + graph signals
  - `normalized_rank()` for [0,1] scaling
  - Dust payment filtering, lookback window, convergence detection
  - 9 tests: empty graph, quality weighting, transitive propagation, cycle convergence, blending

### P1-03: Trust tiers (spec has different trust terminology)

- [x] **Spec** (`docs/08-chain/04-korai-passport-erc-721-soulbound.md` lines 107-118): Tier progression rules.
- **FIXED**: Implemented `TierProgressionRules` with full spec-aligned logic:
  - Edge->Worker: 5000 KORAI + 10 jobs + avg rep >= 0.5
  - Worker->Sovereign: 25000 KORAI + 100 jobs + avg rep >= 0.7
  - Sovereign->Protocol: requires governance_approved flag
  - Demotion on stake drop (immediate) or low reputation (30-day grace period)
  - `TierEvaluation` enum: Maintain/Promote/Demote/RequiresGovernance
  - 10 unit tests covering all paths

### P1-04: Token emission schedule (halving + terminal emission)

- [x] **Spec** (`bardo-backup/tmp/agent-chain-new/05-token-economics.md`): Token emission schedule.
- **FIXED**: Implemented `EmissionSchedule` in `roko-chain/src/korai_token.rs`:
  - Halving epochs: rate halves every `blocks_per_epoch` blocks
  - Terminal emission rate: floor that prevents minting from ever stopping completely
  - Max supply cap: minting stops when total supply reached
  - `rate_at_block(block)` — current emission rate
  - `emission_for_range(start, end)` — total emission across epoch boundaries
  - `default_korai()` — 100 KORAI/block, ~1 year halvings, 1 KORAI/block terminal, 1B max supply
  - 7 tests covering halving, terminal floor, supply cap, cross-epoch ranges

### P1-05: Service endpoints + runtime fingerprint on passports

- [ ] **Spec** (`docs/08-chain/04-korai-passport-erc-721-soulbound.md`): Passports carry service endpoints for agent discovery and runtime fingerprints for ventriloquist defense.
- **Code** (`crates/roko-chain/src/identity_economy_identity.rs` lines 57, 79, 629): `DidServiceEndpoint` and `ServiceEndpoint` types exist but are in the identity-economy module, not connected to the `AgentPassport` in `phase2.rs` (line 748). The `phase2::AgentPassport` has `service_endpoints: Vec<String>` as a simple string list, not the structured `DidServiceEndpoint` type.
- **Fix**: Unify passport service endpoint types between `phase2.rs` and `identity_economy_identity.rs`.

### P1-06: 7-day grace period for reputation decay

- [ ] **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` line 80): 30-day half-life decay on reads.
- **Code** (`crates/roko-chain/src/reputation_registry.rs` line 26): `HALF_LIFE_SECS = 30 * 24 * 3600`. No grace period before decay starts. Spec implies decay is continuous from last update, but many reputation systems include an initial grace period where the score is stable.
- **Fix**: Clarify whether spec intends an immediate-decay model (current code) or a grace-period model. If grace period is intended, add `grace_period_days: f64` config.

### P1-07: Knowledge entry lifecycle progression

- [x] **Spec** (`bardo-backup/tmp/agent-chain-new/04-knowledge-layer.md` lines 143-192): Knowledge lifecycle.
- **FIXED**: Implemented full lifecycle in knowledge_store.rs:
  - `DEATH_THRESHOLD = 0.01` (1% of initial weight → Death stage)
  - `is_dead(entry, now)` function checks recency factor vs threshold
  - `prune_dead()` method freezes entries below threshold (preserves for resurrection)
  - `resurrect(entry_id, confirming_episode)` method: unfreezes, resets confidence to 0.6, resets tier to Transient, increments confirmation_count
  - Confirmation-adjusted decay: `recency = base_decay * (1 + confirmations * 0.1)` — each confirmation extends effective lifetime by 10%

### P1-08: Shapley-value attribution

- [x] **Spec** (`bardo-backup/tmp/agent-chain-new/05-token-economics.md`): Fair credit attribution using Shapley values.
- **FIXED**: Implemented `roko_learn::shapley` module with:
  - `shapley_exact(n, v)` — exact O(2^n * n) computation for small groups
  - `shapley_monte_carlo(n, v, samples, seed)` — scalable approximation
  - `shapley_attribution(agent_ids, v, samples)` — high-level API with named results
  - `Coalition` bitmask type with set operations
  - 10 tests verifying game-theoretic axioms (efficiency, symmetry, null player, additivity)

### P1-09: ADAS meta-learning

- [ ] **Spec** (referenced in learning architecture): Adaptive Data-Augmented Strategy meta-learning for improving learning-to-learn capabilities.
- **Code** (`crates/roko-learn/src/adas.rs`): File exists but implementation status needs verification.
- **Fix**: Verify ADAS implementation matches spec, wire into learning pipeline if not already connected.

### P1-10: Dream phase naming alignment

- [x] **Spec** (`docs/10-dreams/01-three-phase-cycle.md` lines 20-23): Three phases.
- **VERIFIED**: `DreamPhase` enum in `phase2/cycle.rs:11` has `NremReplay`, `RemImagination`, `Integration` matching spec. `DreamPhaseKind` enum extends with `Hypnagogia` and `Evolution`. Budget tracking, model tier mapping, and phase transitions all wired in `cycle.rs`.

### P1-11: Adaptive alpha for reputation EMA

- [x] **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` lines 58-68): Alpha adapts based on job count.
- **FIXED**: Replaced volatility-based alpha with job-count-based tiers (0.30/0.15/0.08/0.04) in reputation_registry.rs.

---

## P2: Missing Spec Features (not yet critical)

These are spec features that are not yet needed for the current milestone but will be
needed for the full vision.

---

### P2-01: Kauri BFT consensus

- [ ] **Spec** (`bardo-backup/tmp/agent-chain-new/03-chain-architecture.md` lines 122-196): Chain uses Simplex BFT (not Kauri). The spec explicitly chose Simplex for simplicity and single-slot finality. Kauri is mentioned nowhere -- this is a non-issue unless future specs reference it.
- **Code**: No consensus implementation at all (chain layer is type stubs).
- **Fix**: When implementing chain consensus, use Simplex BFT per spec, not Kauri.

### P2-02: EVM precompiles (0xA01+)

- [ ] **Spec** (`docs/08-chain/03-hdc-on-chain-precompile.md`, `docs/08-chain/INDEX.md` line 15): HDC precompile at `0xA01` with similarity, topk, bind, bundle operations. Also GolemRegistry and InsightLedger as native precompiles.
- **Code**: No EVM precompile implementations. `crates/roko-chain/` has Rust types only.
- **Fix**: Implement HDC precompile as revm handler at address `0xA01` when chain runtime is built.

### P2-03: InsightStore on-chain (InsightLedger smart contract)

- [ ] **Spec** (`bardo-backup/tmp/agent-chain-new/04-knowledge-layer.md` lines 107-141): InsightEntry struct with `contentHash`, `content`, `hypervector`, `entryType`, `postedBlock`, `halfLifeBlocks`, `poster`, `initialWeight`, `confirmations`, `cladeId`, `metadata`.
- **Code**: `roko-neuro` has off-chain `KnowledgeEntry` but no on-chain `InsightEntry` matching the Solidity struct spec.
- **Fix**: Implement `InsightLedger` contract types matching the on-chain spec.

### P2-04: Clearing engine (cooperative clearing with QP solver)

- [ ] **Spec** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 137-249): Full 6-phase cooperative clearing with QP solver in TEE, soft-threshold bisection, KKT verification, DVP settlement.
- **Code** (`crates/roko-chain/src/isfr.rs`): Simple weighted-average "solver" with no real optimization.
- **Fix**: Implement proper clearing engine with QP solver, TEE integration, and phased protocol.

### P2-05: Privacy / Gray Box layer

- [ ] **Spec** (referenced in chain architecture): ZK-proofs, sealed computation, privacy-preserving reputation.
- **Code**: No privacy layer implementation.
- **Fix**: Design and implement privacy primitives when chain runtime matures.

### P2-06: DID:Korai resolution

- [ ] **Spec** (`docs/08-chain/04-korai-passport-erc-721-soulbound.md`): DID method for Korai agent identities.
- **Code** (`crates/roko-chain/src/identity_economy_identity.rs`): Has `DidDocument` and `DidServiceEndpoint` types but no DID resolution protocol implementation.
- **Fix**: Implement `resolve(did:korai:...) -> DidDocument` resolver.

### P2-07: Sealed bidding (TEE)

- [ ] **Spec** (`docs/14-identity-economy/13-isfr-clearing-settlement.md` lines 162-188): Commit-reveal scheme with sealed commitments, TEE computation, early-reveal penalties.
- **Code**: No TEE integration, no commit-reveal protocol, no sealed bidding.
- **Fix**: Implement when TEE infrastructure is available.

### P2-08: Budget-feasible VCG approximation guarantee

- [ ] **Spec** (`docs/03-composition/` VCG auction references): VCG auction with budget-feasibility constraints for context allocation.
- **Code**: VCG types exist in `roko-compose` but no formal budget-feasibility guarantee proof.
- **Fix**: Add formal budget-feasibility analysis or approximation ratio documentation.

### P2-09: Nelson-Siegel yield curve

- [x] **Spec** (referenced in technical analysis for DeFi oracle yield curve modeling): Nelson-Siegel model.
- **FIXED**: Implemented `roko_chain::nelson_siegel::NelsonSiegel` with:
  - 4-parameter model (beta0, beta1, beta2, tau) for yield curve term structure
  - `rate(maturity)`, `forward_rate(maturity)`, `rate_curve(maturities)` computation
  - `short_rate()`, `long_rate()`, `term_spread()`, `hump_maturity()` analytics
  - `fit(observations)` — grid search + OLS least squares fitting
  - 10 tests covering flat curves, convergence, fitting, spread calculation

### P2-10: Kalman filter smoothing

- [x] **Spec** (referenced in technical analysis for signal processing): Kalman filter for signal smoothing.
- **FIXED**: Implemented `roko_learn::kalman::KalmanFilter` with:
  - Standard predict/update cycle for random-walk model
  - Factory methods: `for_oracle_smoothing()`, `for_tracking()`
  - Anomaly detection via normalized innovation squared
  - Dynamic noise adjustment, batch processing, reset
  - 9 tests: convergence, gain decrease, Q/R tradeoffs, anomaly detection

### P2-11: Collusion ring detection (EigenTrust)

- [x] **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` lines 250-302): Collusion ring detection.
- **FIXED**: Implemented `roko_chain::collusion::CollusionDetector` with:
  - Assignment graph from job marketplace edges
  - Mutual assignment ratio analysis (min/max count per pair)
  - Bron-Kerbosch algorithm with pivoting for maximal clique enumeration
  - Configurable thresholds: mutual_ratio, min_assignments, min_clique_size, lookback
  - CollusionReport with rings, suspicious pairs, agent/assignment counts
  - 6 tests: empty graph, 3-agent ring, low-volume filtering, directional filtering, clique size, lookback

### P2-12: Reputation recovery mechanisms

- [x] **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` lines 128-141): Recovery mechanisms.
- **FIXED**: Implemented `RecoveryTracker` with full spec-aligned logic:
  - Probation recovery: 10 jobs with avg feedback >= 0.6
  - Suspension recovery: 90-day wait + recovery stake + verification challenge
  - `RecoveryStatus` enum: Eligible/WaitingPeriod/NeedMoreJobs/FeedbackTooLow/NeedStake/NeedVerification
  - `attempt_recovery()` restores to GoodStanding when all conditions met
  - 4 tests covering probation recovery, low-feedback rejection, suspension recovery, and not-applicable

### P2-13: Governance amnesty for bans

- [x] **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` line 124): "Bans can be appealed through governance after 365 days."
- **FIXED**: Added `ban_agent(id, now)` with timestamp tracking, `amnesty_eligible(id, now)` returning days remaining, and `governance_amnesty(id, now)` to lift bans after 365-day wait. Test covers the full lifecycle.

---

## P3: Undocumented Enhancements (code has, spec doesn't)

These are features the code implements that go beyond what the spec describes. They may
be good ideas that need spec updates, or they may be accidental additions.

---

### P3-01: Alpha paradox (confirmation shortens half-life)

- [ ] **Status**: Good idea, needs spec update
- **Code** (`crates/roko-orchestrator/src/coordination.rs` lines 337-345, 384-411): For `PheromoneKind::Alpha` pheromones, confirmations **shorten** the effective half-life instead of extending it. This is called the "Alpha paradox" -- consensus makes alpha expire faster, preventing herding on ephemeral edges.
- **Spec** (`docs/13-coordination/04-pheromone-kinds.md`): The spec describes pheromone kinds and their behavior but does not explicitly document the Alpha paradox reversal.
- **Action**: Update `docs/13-coordination/04-pheromone-kinds.md` to document the Alpha paradox as a design decision.

### P3-02: 4-level scope hierarchy (Local -> Subnet -> Mesh -> Global)

- [ ] **Status**: Good idea, needs spec update
- **Code** (`crates/roko-orchestrator/src/coordination.rs` lines 832-833): Implements a 4-level scope promotion system: Local -> Subnet -> Mesh -> Global, with trust discounting per scope level.
- **Spec** (`docs/13-coordination/05-pheromone-scope.md`): Describes scopes but the implementation adds `SubnetId` with `collective_id` + `partition` which is more specific than the spec.
- **Action**: Update spec to document the `SubnetId` structure and trust discounting coefficients.

### P3-03: Trust discounting per scope

- [ ] **Status**: Good idea, needs spec update
- **Code** (`crates/roko-orchestrator/src/coordination.rs`): Implements `TrustDiscount` with configurable multipliers per scope level (e.g., Local=1.0, Subnet=0.8, Mesh=0.5, Global=0.3).
- **Spec**: Not explicitly documented with concrete values.
- **Action**: Add trust discount values to `docs/13-coordination/05-pheromone-scope.md`.

### P3-04: WisdomGate thresholds

- [ ] **Status**: Good idea, needs spec update
- **Code** (`crates/roko-orchestrator/src/coordination.rs`): Implements `WisdomGate` with `min_hdc_diversity`, `max_lineage_overlap`, `max_sender_share` thresholds.
- **Spec** (`docs/00-architecture/14-c-factor-collective-intelligence.md` lines 239-264): Describes `WisdomGate` in the spec, so this is partially aligned. But the code implementation may have additional fields or different thresholds.
- **Action**: Verify code's `WisdomGate` matches spec exactly. Update spec if code has useful additions.

### P3-05: Pattern detection (CEP-like)

- [ ] **Status**: Good but orthogonal to stigmergy spec
- **Code** (`crates/roko-conductor/src/pattern_detector.rs`): Implements complex event processing (CEP) style pattern detection for conductor signals.
- **Spec**: Not documented in the coordination or conductor specs.
- **Action**: Document pattern detection in `docs/07-conductor/` as a conductor capability.

### P3-06: Volatility-based EMA alpha

- [ ] **Status**: May be better than spec, needs evaluation
- **Code** (`crates/roko-chain/src/reputation_registry.rs` lines 9-11): `alpha = base_alpha * (1.0 + volatility)` where volatility is stddev of recent 10 observations.
- **Spec** (`docs/08-chain/14-reputation-system-7-domain.md` lines 58-68): Uses job-count-based alpha tiers (0.30/0.15/0.08/0.04).
- **Action**: Evaluate whether volatility-based alpha is strictly better. If so, propose spec amendment. Otherwise, align code to spec's job-count tiers.

### P3-07: Federation module in conductor

- [ ] **Status**: Undocumented enhancement
- **Code** (`crates/roko-conductor/src/federation.rs`): New file implementing federation capabilities.
- **Spec**: No federation concept in `docs/07-conductor/`.
- **Action**: Document federation in conductor spec or remove if not needed.

### P3-08: Calibration policy in learning

- [ ] **Status**: Undocumented enhancement
- **Code** (`crates/roko-learn/src/calibration_policy.rs`): New file implementing calibration policy.
- **Spec**: Not explicitly documented as a separate module.
- **Action**: Document in `docs/05-learning/` or fold into existing calibration tracker spec.

### P3-09: Research pipeline in learning

- [ ] **Status**: Undocumented enhancement
- **Code** (`crates/roko-learn/src/research_pipeline.rs`): New file implementing research pipeline.
- **Spec**: Not documented in `docs/05-learning/`.
- **Action**: Document in learning spec.

### P3-10: Hotelling gate

- [ ] **Status**: Undocumented enhancement
- **Code** (`crates/roko-gate/src/hotelling.rs`): Implements Hotelling T-squared test as a gate.
- **Spec**: Not documented in `docs/04-verification/`.
- **Action**: Document in verification spec as a statistical gate type.

---

## Batch 2-5 Additional Findings (Deep Audit)

### Reputation System (CRITICAL)

- [x] **P0-12: Reputation decay formula wrong -- no neutral-point convergence**
  - Spec (docs/08-chain/14-reputation-system-7-domain.md line 78-89): `neutral + (score - neutral) * decay_factor` where neutral=0.5
  - FIXED: Decay now converges toward neutral 0.5. Low scores recover UP, high scores decay DOWN.

- [x] **P0-13: Reputation alpha formula completely different from spec**
  - FIXED: Replaced volatility-based alpha with job-count-based tiers (0.30/0.15/0.08/0.04).

- [x] **P0-14: Slash violation types don't match spec (4 vs 7)**
  - FIXED: Now has all 7 spec types: MissedDeadline(1%), AbandonedJob(3%), QualityRejection(2%), RepeatedQualityFailure(5%), Plagiarism(10%), ResultManipulation(10%), TeeViolation(10%), plus Collusion (feedback weight dilution).

- [x] **P0-15: Discipline thresholds lower than spec**
  - FIXED: Probation < 0.4, Suspension < 0.2 (matching spec). Slash count uses 90-day rolling window.

### Neuro Knowledge System (CRITICAL)

- [x] **P0-16: Tier multiplier logic not implemented**
  - VERIFIED: `effective_half_life_days()` applies `base_half_life * tier.multiplier()` with (0.1x, 0.5x, 1.0x, 5.0x). Used in `recency_factor()` for decay.

- [x] **P0-17: Tier promotion/demotion counts wrong or missing**
  - VERIFIED: Transient->Working at 2 confirmations, Working->Consolidated at 3 distinct contexts. Promotion logic in knowledge_store.rs line 422-431.

- [x] **P0-18: CONFIRMATION_BOOST constant missing**
  - VERIFIED: `CONFIRMATION_BOOST = 1.5` exists at knowledge_store.rs line 30, applied in `confirmation_boost()` function.

### Gates (HIGH)

- [x] **P1-12: CUSUM sensitivity parameter mismatch**
  - FIXED: DEFAULT_CUSUM_SENSITIVITY changed from 0.05 to 0.25 per spec.

- [ ] **P1-13: PELT offline change point detection not implemented**
  - Spec describes PeltDetector with cost functions
  - Not found in code

- [ ] **P1-14: Domain-specific threshold profiles not implemented**
  - Spec describes ThresholdProfile with role-specific priors
  - Not found in code

### Heartbeat (HIGH)

- [ ] **P1-15: Theta reflective loop not wired to orchestration**
  - theta_consumer.rs exists as scaffold but five-phase reflection not invoked

- [ ] **P1-16: Delta consolidation loop not wired to dreams**
  - Dreams exist but not triggered by delta tick consumer

- [ ] **P1-17: Bus-backed topic subscriptions not wired**
  - Topic constants exist but Phase 2 dual-fabric consumer model not implemented

### Learning (MEDIUM)

- [x] **P1-18: Cost guardrails not enforced in CascadeRouter::select()**
  - VERIFIED: BudgetGuardrail wired at orchestrate.rs:12926. Checks task/session/plan limits, routes to cheaper model at 80%, blocks at 100%.

- [x] **P1-19: Provider health circuit breaker not filtering candidates**
  - VERIFIED: `healthy_model_slugs()` calls `provider_health.filter_arms_or_best()` at orchestrate.rs:12780-12784 before model selection.

- [ ] **P1-20: Pareto frontier refresh interval not enforced**
  - Spec says recompute every 50 observations, no interval constant found

### Safety Docs (HIGH - spec quality)

- [ ] **P1-21: Safety docs conflate target-state with actual implementation**
  - Capability<T> compile-time tokens described as existing but marked as "target state"
  - authorize() unified API described in present tense but noted as not implemented
  - Misleading for developers reading the docs

### Agents (MEDIUM - stale docs)

- [x] **P1-22: LlmBackend implementations described as "missing" but exist**
  - VERIFIED: OpenAI-compat, Cursor, Codex, Gemini, Ollama, Perplexity all implemented. Audit note is stale.

- [ ] **P1-23: ExecutorAction has 3 undocumented variants**
  - ApplyDagMutation, StartSpeculativeExecution, CancelSpeculativeExecution not in spec

### Dreams/Composition (MEDIUM)

- [ ] **P1-24: Dream depotentiation constants exist but wiring to Daimon unclear**
  - DEPOTENTIATION_DELTA_MIN/MAX defined but actual application path not visible

- [ ] **P1-25: Cache alignment markers (XML comments) may not be emitted in build()**
  - Spec requires `<!-- roko:layer:system -->` markers for KV-cache optimization

---

## Batch 6-9: Deep Audit of Original PRDs, Docs, and Papers

### Lost Core Ideas from Bardo PRDs (P0-level)

- [ ] **P0-19: OCC Appraisal Engine completely lost** — PRD 03-daimon specified full Ortony-Clore-Collins appraisal with trigger-grounded emotions, 7 appraisal dimensions. Code only has PAD octant classification.
  - Spec: bardo-backup/prd/03-daimon/01-appraisal.md
  - Code: crates/roko-daimon/src/lib.rs (only AffectOctant)

- [ ] **P0-20: Four-factor retrieval model collapsed to single score** — PRD specified recency(0.20) x importance(0.25) x relevance(0.35) x emotional_congruence(0.20) with weight learning. Code only has mood_congruent_score().
  - Spec: bardo-backup/prd/03-daimon/ (emotional memory)
  - Code: crates/roko-daimon/src/lib.rs

- [ ] **P0-21: Contrarian retrieval (anti-rumination) lost** — 15% mood-opposite injection over 200-tick window to prevent depressive loops. ContrarianTracker is a stub with no logic.
  - Spec: bardo-backup/prd/03-daimon/
  - Code: crates/roko-daimon/src/phase2_stubs.rs (empty struct)

- [x] **P0-22: Emotional consolidation bias lost** — FIXED: `emotional_consolidation_boost()` now implements McGaugh 2004 arousal-based priority: `boost *= 1.0 + arousal * 0.30` (up to 1.3x at max arousal). High-arousal episodes are consolidated with higher priority.

- [ ] **P0-23: Life review pipeline lost** — Butler 1963: retrieve top-20 emotional memories, detect turning points (PAD distance 0.5), classify narrative arc (Redemptive/Contaminating/Progressive/Tragic/Stable per McAdams). Completely absent.
  - Spec: bardo-backup/prd/03-daimon/ (mortality section)
  - Code: not found

- [ ] **P0-24: Behavioral phases (Camel/Lion/Child metamorphosis) lost** — Nietzsche three metamorphoses mapped to vitality. Replaced with generic BehavioralState enum.
  - Spec: bardo-backup/prd/03-daimon/
  - Code: crates/roko-core (only Thriving/Struggling/Coasting/Resting)

- [x] **P0-25: EmotionalTag exists but never used for retrieval** — FIXED: Two changes:
  1. Added `enrich_from_emotional_tag()` builder on `KnowledgeEntry` for callers to transfer episode tags
  2. Wired emotional tag transfer in dream cycle's `threat_warning_entries_with_floor()` — picks the most intense emotional tag from source episodes and sets it on the knowledge entry, enabling `emotional_retrieval_boost()` which was previously always 1.0

### Lost Core Ideas (P1-level)

- [ ] **P1-26: Mortality emotions lost** — Economic Anxiety (Jonas), Epistemic Vertigo (Dane), Stochastic Dread (Heidegger) — 3 mortality-specific PAD signatures completely absent.

- [ ] **P1-27: Sibling death contagion lost** — When a sibling dies, survivors should re-evaluate epistemic fitness, trigger dream cycle, reduce sharing threshold. Not implemented.

- [ ] **P1-28: Emotional death testament lost** — Death knowledge should carry emotional context, turning points, narrative arc. Current death protocol is basic.

- [x] **P1-29: Emotional diversity as quality signal lost** — FIXED: Added `EmotionalProvenance::compute_diversity(tags)` using normalized Shannon entropy of coarse emotion labels. Also added `from_tags(tags)` factory for building provenance from multiple episode tags. Diversity feeds into `emotional_consolidation_boost()` as a 15% weight.

- [x] **P1-30: Only 2 of 5 behavioral modulation channels wired** — FIXED: Added `risk_tolerance`, `probe_sensitivity`, `sharing_threshold` to `AffectBehaviorModulation`. All 5 channels now set per-octant: anxious hoards (sharing=0.75), confident shares freely (sharing=0.20), angry has high risk tolerance (0.60), etc.

- [ ] **P1-31: Only 2 of 4 dream replay modes functional** — Random and Consequence work. Causal (follow failure chains) and Hypothetical (counterfactuals) are stubs.

- [ ] **P1-32: Counterfactual imagination returns placeholders** — The imagination module exists but returns stub data, not actual alternative trajectories.

- [ ] **P1-33: EmotionalProvenance struct exists but is dead code** — Has average_pad, discovery_emotion, validation_arc, emotional_diversity fields but is never populated or read.

- [ ] **P1-34: ValidationArc enum exists but is dead code** — Redemptive/Contaminating/Stable/Progressive variants defined but never computed.

### Chain & Financial Layer (P0-level)

- [ ] **P0-26: Yield perpetuals not implemented** — Papers describe detailed mechanics (mark price, funding rate, 10x leverage, margin tracking). Only stub types exist.
  - Spec: papers/new/litepaper/08-yield-perpetuals.md
  - Code: crates/roko-chain/src/futures_market.rs (knowledge futures only, NOT yield perps)

- [ ] **P0-27: Cooperative clearing engine not implemented for financial settlement** — QP solver exists but only for ISFR fact resolution, NOT for yield perpetual clearing.
  - Spec: papers/new/blue-ocean/14-12-cooperative-clearing.md

- [ ] **P0-28: Product surfaces (AI Studio, Agent Studio, OpenClaw) not implemented** — Papers describe 3 customer-facing products as operational. None exist.
  - Spec: papers/new/litepaper/12-product-surfaces.md

- [ ] **P0-29: Kauri BFT consensus not in repo** — Papers claim operational with 1,389 tests. Not found.
  - Spec: papers/new/litepaper/10-chain-design.md

- [ ] **P0-30: SpecPool EVM not implemented** — Stub types only in phase2.rs.

### Chain Intelligence (P1-level)

- [ ] **P1-35: Entire chain-intelligence 5-crate pipeline deferred** — Bardo PRD specified: golem-witness (block subscription), golem-triage (MIDAS-R + HDC), golem-protocol-state (protocol cache), golem-chain-scope (dynamic attention), golem-stream-api (WebSocket streaming). Only type stubs exist in phase2.rs.
  - Spec: bardo-backup/prd/14-chain/00-architecture.md

- [ ] **P1-36: Cybernetic feedback loop lost** — Agent behavior -> observation interest -> Binary Fuse filter -> triage -> cognition -> behavior. Novel architecture not preserved.

- [ ] **P1-37: Binary Fuse filter is empty Vec** — Should be 8.7 bits/entry O(1) lookup. Currently `keys: Vec<u64>` placeholder.

- [ ] **P1-38: MIDAS-R streaming anomaly detection is stub** — Width/depth fields defined but no streaming algorithm.

### Safety Architecture (P1-level)

- [ ] **P1-39: PolicyCage on-chain enforcement completely absent** — Old PRD specified immutable smart contract enforcing spending caps, asset whitelists, drawdown limits. Current safety is behavioral (role/tool filtering), not cryptographic.
  - Spec: bardo-backup/prd/10-safety/02-policy.md

- [ ] **P1-40: 42+ agent archetypes collapsed to flat roles** — Old PRD specified 42+ behavioral presets across 14 categories with tool profiles, Hermes routing, delegation DAG, PLAYBOOK.md drift. Current code has simple role-based config.
  - Spec: bardo-backup/prd/19-agents-skills/00-agents-overview.md

### Documentation Misalignments

- [x] **P1-41: Score doc says extended axes "not yet implemented" but they ARE implemented** — FIXED: Updated doc to say "Implemented". Code has full 7-axis Score struct with soft-damping formula.

- [x] **P1-42: Taint enum variants don't match doc** — FIXED: Updated doc to show canonical Taint variants (Clean, LlmHallucination, ToolFailure, UserFlagged, StaleData, UnverifiedSource, Propagated, UserInput, Custom).

- [x] **P1-43: Kind::Compound helpers not implemented** — FIXED: Added full compositional API:
  - `Kind::compound(&[Kind])` — factory method
  - `matches(other)` — checks constituent containment (compound→simple, compound→compound subset)
  - `contains(part)` — direct containment check
  - `arity()`, `constituents()`, `is_compound()` helpers
  - 6 new tests for factory, containment, matching, subset, iteration

- [x] **P1-44: Demurrage config section entirely absent from code** — FIXED: Expanded DemurrageConfig from 2 to 9 parameters:
  rate_per_hour, min_balance, freeze_threshold, thaw_balance, max_balance,
  gc_interval_secs, kind_rate_multipliers (HashMap), freeze_before_delete, death_threshold.
  Updated example config writer. All 116 config tests pass.

- [ ] **P1-45: Oneirography is stubs only** — PRD described 6 art forms, PAD-reactive auctions, NFT minting, self-appraisal. Only image generation request types exist.
  - Spec: bardo-backup/prd/22-oneirography/00-overview.md
  - Code: crates/roko-dreams/src/phase2/oneirography.rs (request types only)

---

## Batch 10: Dead Wiring, Test Assertions, and Cross-Crate Mismatches

### Dead Wiring (Config sections defined but never read at runtime)

- [ ] **P1-46: AttentionConfig never read from config at runtime** — Section exists in schema.rs, loads from TOML, but `config.attention.*` fields are never accessed in orchestrate.rs or agent code. Config is dead weight.
  - Code: crates/roko-core/src/config/schema.rs (AttentionConfig)
  - Gap: No code reads config.attention.max_tokens_per_layer or any field

- [ ] **P1-47: ImmuneConfig never read at runtime** — Same as above. QuarantineVault exists separately but doesn't read from config.
  - Code: crates/roko-core/src/config/schema.rs (ImmuneConfig)

- [ ] **P1-48: TemporalConfig never read at runtime** — Allen interval relations exist but don't read config.temporal.*
  - Code: crates/roko-core/src/config/schema.rs (TemporalConfig)

- [ ] **P1-49: GoalsConfig never read at runtime** — GoalTree exists but doesn't read config.goals.*
  - Code: crates/roko-core/src/config/schema.rs (GoalsConfig)

- [ ] **P1-50: EnergyConfig never read at runtime** — EnergyPool exists but doesn't read config.energy.*
  - Code: crates/roko-core/src/config/schema.rs (EnergyConfig)

- [ ] **P1-51: OneirographyConfig defined in TWO places, neither used at runtime** — Duplicate definitions in schema.rs AND oneirography.rs, config never passed to DreamRunner.
  - Code: schema.rs line 1341 AND phase2/oneirography.rs line 335

- [ ] **P1-52: ToolsConfig defined but not used for tool filtering** — [tools.profile] section parsed but not wired into ToolDispatcher filtering.
  - Code: crates/roko-core/src/config/schema.rs (ToolsConfig)

### Dead Wiring (Features built but not dispatched from runtime)

- [ ] **P1-53: ContextBidder implementations (8 bidders) only called in tests** — NeuroBidder, DaimonBidder, etc. all exist with default_bidders() but never invoked from orchestrator heartbeat loop.
  - Code: crates/roko-runtime/src/heartbeat_attention.rs

- [ ] **P1-54: ColdSubstrate (ArchiveColdSubstrate) built but never instantiated** — Full implementation with tests, but no call site in orchestrate.rs.
  - Code: crates/roko-fs/src/cold_substrate.rs

- [ ] **P1-55: WitnessVerifier and CodingWitness built but not dispatched** — Witness trait and CodingWitness implemented with tests, but no code path invokes witness verification on oracle predictions.
  - Code: crates/roko-learn/src/oracles/witness.rs

- [ ] **P1-56: VCG auction (vcg_allocate) only called in tests, not in prompt composition** — Function exists and is unit-tested but PromptComposer doesn't call it in the live path.
  - Code: crates/roko-compose/src/auction.rs

### Cross-Crate Type Conflicts

- [x] **P0-31: TWO incompatible Taint enums in codebase** — FIXED: Renamed roko-agent's Taint to `CustodyTaint` (with `type Taint = CustodyTaint` alias for backward compat). Added `to_signal_taint()` bridge method. Documented architectural distinction (custody layer vs signal lineage layer).
  - roko-core: canonical signal-level Taint (9 variants)
  - roko-agent: CustodyTaint for action-centric safety (5 variants, bridges to core)

- [x] **P0-32: PassportTier derive(Ord) creates INVERTED privilege hierarchy** — FIXED: Manual Ord impl with privilege_level(). Protocol > Sovereign > Worker > Edge. Added `has_privilege()` helper and `min_stake()` constants.
  - Code: crates/roko-chain/src/phase2.rs

### Test Assertion Issues

- [ ] **P1-57: Knowledge store tier promotion test asserts wrong behavior** — Test `ingest_promotes_high_support_entries_to_longer_tiers` expects entry with 0 confirmations and 3 source_episodes to auto-promote to Consolidated, but code requires confirmation_count >= 2 for any promotion. Test may pass due to different code path but validates wrong scenario.
  - Code: crates/roko-neuro/src/knowledge_store.rs test at line 2982

### Tier System Fragmentation

- [ ] **P2-14: 10+ different tier enums across codebase with no compatibility matrix** — InferenceTier, ModelTier, CognitiveTier, KnowledgeTier, PassportTier, ContextTier, EpisodePriorityTier, TaskPhase, ThreatTier, GeminiContextTier all exist. No documentation maps which applies where.
  - Recommendation: Create tier-system compatibility matrix in docs

## Batch 11: Final Deep Pass (Agent-Chain-New, Shared PRDs, Dead Wiring Granularity)

### From agent-chain-new specs (newest, most authoritative chain specs)

- [x] **P0-33: Predictive Foraging residual corrector not implemented** — FIXED: Implemented `ResidualCorrector` with:
  - `ResidualBuffer`: O(1) circular buffer (capacity 200) with streaming mean/variance/coverage via running accumulators
  - Per-key state keyed by `(category, context, metric)` triples
  - Bias correction: `center -= mean_residual`
  - Interval width calibration: widens 5% when coverage < 80%, narrows when > 95%
  - Difficulty weighting: `category_variance × novelty × tightness`
  - 8 tests covering buffer wrapping, stats, bias removal, interval widening, difficulty scaling

- [x] **P0-34: Knowledge entry utility scoring from prediction accuracy missing** — FIXED: Added `score_prediction_utility(context_entry_ids, prediction_accurate, accuracy_score)` to `KnowledgeStore`. Accurate predictions bump `confidence_weight` by +0.05 × accuracy; inaccurate predictions decay by -0.03 × (1 - accuracy). Also adjusts demurrage balance. Shifts curation from popularity-based to effectiveness-based.

- [ ] **P1-58: No catalytic score tracking for autocatalytic knowledge networks** — Spec (agent-chain-new/13-exponential-growth.md) requires tracking how many new entries each entry enabled. When avg catalytic score > 1.5, network becomes self-sustaining. Not implemented.

- [ ] **P1-59: Context assembly missing four-factor composite weighting** — Spec (agent-chain-new/07-context-assembly.md) requires: HDC similarity (40%) + pheromone weight (30%) + PF utility (20%) + freshness (10%). Code uses simpler scoring without explicit weight factors.

- [ ] **P1-60: Context assembly missing 10-20% cross-domain diversity bonus** — Spec requires deliberately including cross-domain entries for serendipitous discoveries. Not implemented.

- [ ] **P1-61: Context assembly missing three-tier injection** — Spec requires: Tier 1 (Warning/Insight) compact inject, Tier 2 (Heuristic/StrategyFragment) relevant include, Tier 3 (CausalLink/AntiKnowledge) on-demand mid-task. Code doesn't differentiate tiers during assembly.

### From shared PRD specs

- [ ] **P1-62: TxHvEncoder (transaction HDC fingerprinting) not implemented** — Spec (bardo-backup/prd/shared/hdc-fingerprints.md) describes role-filler transaction encoding with thermometer encoding for gas tiers and value buckets. Generic Codebook exists but transaction-specific encoder absent.

- [ ] **P1-63: HDC applications layer (episode compression, legacy bundles, drift detection) not implemented** — Spec (bardo-backup/prd/shared/hdc-applications.md) describes bundling 500 episodes → 1 prototype vector. Foundation types exist but application layer absent.

- [ ] **P1-64: PhiEngine (full IIT computation) not implemented** — Spec (bardo-backup/prd/shared/integrated-information.md) describes Phi computation over 7 subsystems with 63 bipartition enumeration and Miller-Madow bias correction. Only SomaticOracleContext skeleton exists.

- [ ] **P1-65: Event catalog has 14 variants vs spec's 87** — Spec (bardo-backup/prd/shared/event-catalog.md) defines 87 GolemEvent variants. Code has DashboardEvent with 14 variants. Design divergence (intentional simplification), not a bug, but worth documenting.

### From roko/docs deep dive

- [x] **P1-66: Validation tiers doc claims "Built" but tier progression logic missing** — VERIFIED: Doc is accurate. KnowledgeTier enum, multiplier() method, promotion logic (2 confirmations → Working, 3 contexts → Consolidated), CONFIRMATION_BOOST = 1.5, and effective_half_life_days() are all implemented and tested.

- [x] **P1-67: Several docs correctly mark features as "Deferred"** — VERIFIED: All "Deferred" annotations are accurate. Chain, identity/economy, wallet, web portal, MEV protection, knowledge futures market correctly marked as Phase 2+.

---

## Summary Statistics

| Category | Count | Description |
|----------|-------|-------------|
| P0 | 34 | Code contradicts spec -- fix first |
| P1 | 67 | Spec expects features that don't exist yet |
| P2 | 14 | Advanced/roadmap features |
| P3 | 10 | Code has features spec doesn't document |
| **Total** | **125** | |

## Priority Order for Fixes

### Immediate (blocks correctness)
1. **P0-07**: Reputation domain names -- trivial rename, high cross-system impact
2. **P0-09**: Passport tier ordering -- subtle bug, comparisons will be wrong
3. **P0-08**: Collusion penalty mechanism -- applying wrong type of penalty
4. **P0-10**: C-Factor components -- missing 2 of 5 variables, extra non-spec variables
5. **P0-11**: Knowledge half-life for Warning (7 days vs 3 minutes)

### Next (blocks spec compliance for ISFR)
6. **P0-01**: ISFR aggregation method (weighted mean -> weighted median)
7. **P0-02**: ISFR data source adapters
8. **P0-03**: ISFR liveness state machine
9. **P0-05**: ISFR eligibility and bounds checks
10. **P0-06**: ISFR QP solver (replace naive weighted average)
11. **P0-04**: ISFR oracle precompile interface

### Lost PRD core ideas (Batch 6-9 critical additions)
12. **P0-19**: OCC Appraisal Engine -- full appraisal model collapsed to octant classification
13. **P0-20**: Four-factor retrieval -- 4 weighted factors collapsed to single score
14. **P0-22**: Emotional consolidation bias -- McGaugh 2004 arousal priority completely absent
15. **P0-23**: Life review pipeline -- Butler 1963 narrative arc classification absent
16. **P0-24**: Behavioral phases -- Nietzsche metamorphoses replaced with generic enum
17. **P0-25**: EmotionalTag dead data -- created but never read during retrieval

### Chain & financial layer (Batch 6-9 critical additions)
18. **P0-26**: Yield perpetuals -- detailed mechanics in papers, only stubs in code
19. **P0-27**: Cooperative clearing for financial settlement -- QP solver only wired to ISFR
20. **P0-28**: Product surfaces -- 3 products described as operational, none exist
21. **P0-29**: Kauri BFT consensus -- papers claim operational with 1,389 tests, not found
22. **P0-30**: SpecPool EVM -- stub types only

### Then (core features)
23. **P1-11**: Adaptive alpha (job-count-based tiers)
24. **P1-03**: Trust tier progression (add job count + reputation checks)
25. **P1-07**: Knowledge lifecycle (death threshold + resurrection)
26. **P1-10**: Dream phase naming alignment
27. **P1-01**: CRPS scoring
28. **P1-02**: TraceRank
29. **P1-08**: Shapley attribution

### Batch 6-9 P1 additions (behavioral, dreams, chain intelligence, safety, docs)
30. **P1-30**: Behavioral modulation -- only 2 of 5 channels wired
31. **P1-31**: Dream replay modes -- only 2 of 4 functional
32. **P1-35**: Chain-intelligence 5-crate pipeline -- entirely deferred to stubs
33. **P1-39**: PolicyCage on-chain enforcement -- completely absent
34. **P1-40**: 42+ agent archetypes -- collapsed to flat roles
35. **P1-41 through P1-45**: Documentation misalignments -- stale docs, wrong enum variants, missing config

### Spec updates needed for P3 items
36. All P3 items need spec documentation updates -- the code is likely correct, the spec just needs to catch up.

---

## Cross-References

- `tmp/docs-gaps/00-INDEX.md` -- master gap index (293 items)
- `tmp/docs-gaps/17-chain.md` -- chain-specific gaps
- `tmp/docs-gaps/23-identity-economy.md` -- identity/economy gaps
- `tmp/docs-gaps/22-coordination.md` -- coordination gaps
- `tmp/docs-gaps/19-dreams.md` -- dreams gaps
- `tmp/docs-gaps/29-technical-analysis.md` -- technical analysis gaps
- `docs/14-identity-economy/13-isfr-clearing-settlement.md` -- ISFR spec
- `docs/08-chain/14-reputation-system-7-domain.md` -- reputation spec
- `docs/08-chain/04-korai-passport-erc-721-soulbound.md` -- passport spec
- `docs/00-architecture/14-c-factor-collective-intelligence.md` -- C-Factor spec
- `docs/13-coordination/04-pheromone-kinds.md` -- pheromone kinds spec
- `docs/10-dreams/01-three-phase-cycle.md` -- dream phases spec
- `bardo-backup/tmp/agent-chain-new/04-knowledge-layer.md` -- knowledge layer spec
- `bardo-backup/tmp/agent-chain-new/05-token-economics.md` -- token economics spec
