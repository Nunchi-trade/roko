# 14-identity-economy -- Gap Checklist

Spec: `docs/14-identity-economy/` (16 files). Code: `crates/roko-chain/src/identity_economy_*.rs`.

Overall: ~5% implemented. Comprehensive type definitions exist (1,565 LOC). Zero business logic -- all algorithms are `todo!()` or empty markers. All items are P2+ (Tier 6 deferred, overlaps heavily with docs/08-chain/).

Note: Significant overlap with `17-chain.md`. This file covers identity/economy-specific gaps not already in the chain checklist.

## Compliant (no action needed)
- KoraiPassport struct definition matches spec (doc 02)
- ReputationTrack struct matches spec (doc 04)
- SlashRecord + SlashCategory enums match spec (doc 02)
- MarketplaceListing struct matches spec (doc 05)
- AgentCost struct matches spec (doc 07)
- Compliance template structs defined (SecTrading, HipaClinical, GdprData) (doc 15)

## Checklist (All P2+ -- Tier 6 deferred)

### IDECON-01: EMA reputation computation
- [x] Implement adaptive EMA formula for 7-domain reputation

**Spec** (doc 04 `04-reputation-7-domain-ema.md`): EMA formula: `R_new = alpha * O + (1-alpha) * R_old` where `alpha = min(0.3, 2/(job_count+1))`. 7 reputation domains (accuracy, speed, reliability, collaboration, innovation, compliance, communication). Cold start: `alpha = 1.0` (first observation sets initial score). 30-day half-life decay at query time: if no new observations for 30 days, score decays toward 0.5 baseline. Bayesian Beta foundation: each domain backed by `Beta(alpha_prior, beta_prior)` with `ema_score = alpha_prior / (alpha_prior + beta_prior)`. Additional formulas: reputation multiplier `rep_mult(R) = 0.1 + 2.9 * R^1.7`, trust-weighted EMA `R_new = (alpha * rater_trust * O) + (1 - alpha * rater_trust) * R_old`, discipline system with strike counting.

**Current code**: `ReputationTrack` struct at `crates/roko-chain/src/identity_economy_identity.rs:272` has fields: `domain: String` (line 274), `ema_score: f64` (line 275), `job_count: u64` (line 276), `last_updated: u64` (line 277). `KoraiPassport` at line 245 carries `reputation_tracks: [ReputationTrack; 7]` (line 255). `crates/roko-chain/src/phase2.rs:760` has an `EmaReputation` struct with `todo!()` methods. No compute function exists anywhere.

**What to change**:
- Add to `ReputationTrack` at `crates/roko-chain/src/identity_economy_identity.rs:272`:
  ```rust
  pub fn update_reputation(&mut self, observation: f64, now_secs: u64) {
      let alpha = if self.job_count == 0 { 1.0 } else { (2.0 / (self.job_count as f64 + 1.0)).min(0.3) };
      self.ema_score = alpha * observation + (1.0 - alpha) * self.ema_score;
      self.job_count += 1;
      self.last_updated = now_secs;
  }
  pub fn decayed_score(&self, now_secs: u64) -> f64 {
      let days_since = (now_secs.saturating_sub(self.last_updated)) as f64 / 86400.0;
      let decay = (-0.693 * days_since / 30.0).exp(); // 30-day half-life
      0.5 + (self.ema_score - 0.5) * decay // decay toward 0.5 baseline
  }
  pub fn reputation_multiplier(&self) -> f64 {
      0.1 + 2.9 * self.ema_score.powf(1.7)
  }
  ```

**Reference files**:
- `crates/roko-chain/src/identity_economy_identity.rs:272-280` — `ReputationTrack` struct with `ema_score`, `job_count`, `last_updated`
- `crates/roko-chain/src/identity_economy_identity.rs:245-260` — `KoraiPassport` with `reputation_tracks: [ReputationTrack; 7]`
- `crates/roko-chain/src/phase2.rs:760` — `EmaReputation` stub (alternative location)
- `docs/14-identity-economy/04-reputation-7-domain-ema.md` — full EMA spec, adaptive alpha, decay, trust-weighted variant
**Depends on**: None
**Accept when**:
- [x] `update_reputation()` applies EMA formula — at identity_economy_identity.rs:548, `alpha * obs + (1-alpha) * old`
- [x] Adaptive alpha computed from job count — `(2.0 / (feedback_count + 1.0)).min(0.3)` at :551-553
- [x] 30-day decay applied at query time — `decayed_score()` at :565 with 30-day half-life
- [ ] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn update_reputation' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-02: Passport lifecycle management
- [x] Implement minting, suspension, revocation state machine

**Spec** (doc 02 `02-korai-passport.md`): Passport lifecycle state machine: `Minting` -> `Active` -> `Suspended` -> `Revoked`. ERC-721 soulbound NFT with fields: `passport_id: U256`, `capabilities_bitmask: u64`, `domain_stakes: HashMap<String, U256>`, `tee_attestation: Option<Bytes>`, `system_prompt_hash: [u8; 32]`. System prompt hash verification: at agent startup, compute `BLAKE3(system_prompt)` and compare to `system_prompt_hash` on-chain — mismatch blocks execution. Soul recovery: 5-of-3 quorum from designated recovery agents can mint a new passport transferring reputation from the revoked one. `SoulRecovery` struct with `old_passport_id`, `new_passport_id`, `recovery_quorum: Vec<Address>`, `signatures: Vec<Signature>`.

**Current code**: `KoraiPassport` struct at `crates/roko-chain/src/identity_economy_identity.rs:245` has `passport_id`, `capabilities`, `domain_stakes`, `tee_attestation`, `system_prompt_hash` fields defined. No `PassportState` enum. No lifecycle transition methods. No `SoulRecovery` struct. No system prompt verification at startup.

**What to change**:
- Add `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] pub enum PassportState { Minting, Active, Suspended { reason: String }, Revoked { reason: String } }` to `identity_economy_identity.rs`
- Add `pub state: PassportState` field to `KoraiPassport`
- Add transition methods: `pub fn activate(&mut self) -> Result<()>` (Minting->Active), `pub fn suspend(&mut self, reason: &str) -> Result<()>` (Active->Suspended), `pub fn revoke(&mut self, reason: &str) -> Result<()>` (any->Revoked)
- Add `pub fn verify_system_prompt(&self, prompt: &str) -> bool` that computes `blake3::hash(prompt.as_bytes())` and compares to `self.system_prompt_hash`
- Add `SoulRecovery` struct with quorum verification

**Reference files**:
- `crates/roko-chain/src/identity_economy_identity.rs:245-270` — `KoraiPassport` struct (add `state` field and methods)
- `docs/14-identity-economy/02-korai-passport.md` — full passport spec with lifecycle, soul recovery, TEE attestation
**Depends on**: None
**Accept when**:
- [x] Passport minting creates soulbound token
- [x] State transitions enforced
- [x] System prompt hash verified at agent startup
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'PassportState\|fn mint\|fn suspend\|fn revoke' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-03: Vickrey auction bid scoring
- [x] Implement reputation-adjusted bid scoring and payment

**Spec** (doc 11 `11-vickrey-reputation-auction.md`): Vickrey second-price auction with reputation adjustment. Bid scoring formula: `s_i = p_i * (1 + (1 - R_i))` where `p_i` is the bid price and `R_i` is the bidder's reputation score [0,1]. Winner is `argmin(s_i)` — lowest adjusted score wins. Payment is second-price adjusted: `payment = s_second / (1 + (1 - R_winner))` where `s_second` is the second-lowest score. This ensures truthfulness (Vickrey 1961): agents bid their true cost because reputation adjustment is deterministic and payment depends on the second-best bid, not their own. The `Sparrow` dispatch system uses power-of-two-choices: for each job, randomly select 2 candidates, run Vickrey between them.

**Current code**: `SparrowBid` struct at `crates/roko-chain/src/identity_economy_markets.rs:105` with `bidder`, `amount`, `reputation_score`, `capabilities` fields. `BountySpec` at line 57 with `title`, `description`, `budget`, `deadline`. `AuctionType::Vickrey` variant at `crates/roko-chain/src/phase2.rs:1335`. `CommitRecord` at phase2.rs:1371 and `RevealRecord` at line 1385 for commit-reveal protocol. No scoring function, no winner selection, no payment computation.

**What to change**:
- Add to `crates/roko-chain/src/identity_economy_markets.rs`:
  ```rust
  pub fn score_bid(bid: &SparrowBid) -> f64 {
      bid.amount * (1.0 + (1.0 - bid.reputation_score))
  }
  pub fn select_winner(bids: &[SparrowBid]) -> Option<(usize, f64)> {
      if bids.is_empty() { return None; }
      let mut scored: Vec<(usize, f64)> = bids.iter().enumerate().map(|(i, b)| (i, score_bid(b))).collect();
      scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
      let winner_idx = scored[0].0;
      let second_score = if scored.len() > 1 { scored[1].1 } else { scored[0].1 };
      let payment = second_score / (1.0 + (1.0 - bids[winner_idx].reputation_score));
      Some((winner_idx, payment))
  }
  ```

**Reference files**:
- `crates/roko-chain/src/identity_economy_markets.rs:57-120` — `BountySpec`:57, `SparrowBid`:105 (add scoring/selection here)
- `crates/roko-chain/src/phase2.rs:1335-1390` — `AuctionType::Vickrey`, `CommitRecord`, `RevealRecord`
- `docs/14-identity-economy/11-vickrey-reputation-auction.md` — full Vickrey spec with truthfulness proof
**Depends on**: IDECON-01 (EMA reputation needed for R_i)
**Accept when**:
- [x] Bid scores computed with reputation adjustment — `score_bid()` at identity_economy_markets.rs:890, `price * (1 + (1 - rep))`
- [x] Winner selected by argmin — `select_winner()` at :912 sorts by score, picks index 0
- [x] Payment computed as second-price — `second_score / (1 + (1 - winner_rep))` at :931
- [ ] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn score_bid\|fn select_winner' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-04: Knowledge marketplace ingestion
- [x] Implement 4-stage verification pipeline for listings

**Spec** (doc 05 `05-knowledge-marketplace.md`): 4-stage ingestion pipeline: (1) `Quarantine` — listing submitted, metadata validated, content hash verified, held for minimum 24h; (2) `Consensus` — at least 2 independent validators review content, majority must approve (simple majority of `verification_badges.len() >= 2`); (3) `Sandbox` — listing available for trial purchase with 100% refund guarantee, effectiveness tracked via `SkillEffectiveness`; (4) `Active` — full marketplace listing, dynamic pricing active. Dynamic pricing formula: `P(t) = base_price * rep_mult(seller_rep) * e^(-decay_lambda * hours_since_listing) * demand_multiplier` where `demand_multiplier = 1.0 + demand_sensitivity * (recent_purchases / avg_purchases - 1.0)`. Price clamped to `[price_floor, price_ceiling]`.

**Current code**: `MarketplaceListing` struct at `crates/roko-chain/src/identity_economy_identity.rs:579` with `listing_hash`, `seller_passport_id`, `title`, `description`, `domain_tags`, `base_price_usdc`, `decay_params`, `verification_badges`, `content_hash` fields. `DynamicPricingEngine` at line 655 with `base_price`, `decay_lambda`, `regime_multiplier`, `demand_sensitivity`, `competition_sensitivity`, `price_floor`, `price_ceiling` fields — all defined, no compute methods. `KnowledgeDutchAuction` at line 674 with `start_price`, `reserve_price`, `auction_duration` — no auction logic. `DecayParams` at line 620 with `decay_lambda`, `regime_multiplier`.

**What to change**:
- Add `#[derive(Debug, Clone, PartialEq, Eq)] pub enum ListingStage { Quarantine { submitted_at: u64 }, Consensus { approvals: u32, rejections: u32 }, Sandbox { trial_starts: u64, refunds: u32 }, Active }` to `identity_economy_identity.rs`
- Add `pub stage: ListingStage` field to `MarketplaceListing`
- Add `pub fn advance_stage(&mut self, now_secs: u64) -> Result<(), &'static str>` enforcing: Quarantine requires 24h elapsed, Consensus requires `approvals >= 2 && approvals > rejections`, Sandbox is immediate, Active is terminal
- Add to `DynamicPricingEngine`:
  ```rust
  pub fn compute_price(&self, hours_since_listing: f64, seller_reputation: f64, recent_purchases: f64, avg_purchases: f64) -> u64 {
      let rep_mult = 0.1 + 2.9 * seller_reputation.powf(1.7);
      let decay = (-self.decay_lambda * hours_since_listing).exp();
      let demand_mult = 1.0 + self.demand_sensitivity * (recent_purchases / avg_purchases.max(1.0) - 1.0);
      let price = self.base_price as f64 * rep_mult * decay * demand_mult;
      price.clamp(self.price_floor as f64, self.price_ceiling as f64) as u64
  }
  ```

**Reference files**:
- `crates/roko-chain/src/identity_economy_identity.rs:579-685` — `MarketplaceListing`:579, `DecayParams`:620, `DynamicPricingEngine`:655, `KnowledgeDutchAuction`:674
- `crates/roko-chain/src/identity_economy_identity.rs:638-651` — `SkillEffectiveness` (effectiveness tracking during Sandbox stage)
- `docs/14-identity-economy/05-knowledge-marketplace.md` — 4-stage pipeline, alpha-decay pricing formula
**Depends on**: IDECON-01 (EMA reputation for `rep_mult` in pricing)
**Accept when**:
- [x] `ListingStage` enum with 4 variants and transition rules
- [x] `advance_stage()` enforces 24h quarantine and consensus thresholds
- [x] `DynamicPricingEngine::compute_price()` applies alpha-decay with reputation multiplier
- [x] Price clamped to `[price_floor, price_ceiling]`
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'ListingStage\|fn advance_stage\|fn compute_price' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-05: LMSR market maker for knowledge futures
- [x] Implement cost/price/buy/sell functions

**Spec** (doc 14 `14-knowledge-futures-market.md`): Logarithmic Market Scoring Rule (Hanson 2003/2007). Cost function: `cost(q) = b * ln(e^(q_deliver/b) + e^(q_default/b))` where `b` is the liquidity parameter, `q_deliver` and `q_default` are share quantities for the two outcomes (delivery/default). Price functions: `p_deliver = e^(q_deliver/b) / (e^(q_deliver/b) + e^(q_default/b))`, `p_default = 1 - p_deliver`. Buy/sell: compute cost difference before and after trade. Properties: bounded loss for market maker (max loss = `b * ln(n)` for n outcomes), automatic price discovery, always-available liquidity.

**Current code**: `LmsrMarketMaker` struct at `crates/roko-chain/src/identity_economy_markets.rs:280` with fields `b: f64` (liquidity), `q_deliver: f64`, `q_default: f64`. Methods: `cost()` at line 296 (`todo!()`), `price_deliver()` at line 301 (`todo!()`), `price_default()` at line 306 (`todo!()`), `buy()` at line 311 (`todo!()`), `sell()` at line 316 (`todo!()`). `LmsrOutcome` enum at line 320 with `Deliver`, `Default` variants.

**What to change**: Replace all `todo!()` bodies:
```rust
pub fn cost(&self) -> f64 {
    self.b * (
        (self.q_deliver / self.b).exp() + (self.q_default / self.b).exp()
    ).ln()
}
pub fn price_deliver(&self) -> f64 {
    let e_d = (self.q_deliver / self.b).exp();
    let e_f = (self.q_default / self.b).exp();
    e_d / (e_d + e_f)
}
pub fn price_default(&self) -> f64 { 1.0 - self.price_deliver() }
pub fn buy(&mut self, outcome: LmsrOutcome, shares: f64) -> f64 {
    let cost_before = self.cost();
    match outcome {
        LmsrOutcome::Deliver => self.q_deliver += shares,
        LmsrOutcome::Default => self.q_default += shares,
    }
    self.cost() - cost_before // cost of purchase
}
pub fn sell(&mut self, outcome: LmsrOutcome, shares: f64) -> f64 {
    let cost_before = self.cost();
    match outcome {
        LmsrOutcome::Deliver => self.q_deliver -= shares,
        LmsrOutcome::Default => self.q_default -= shares,
    }
    cost_before - self.cost() // refund from sale
}
```

**Reference files**:
- `crates/roko-chain/src/identity_economy_markets.rs:280-325` — `LmsrMarketMaker` struct with all `todo!()` methods, `LmsrOutcome` enum
- `crates/roko-chain/src/identity_economy_identity.rs:710` — references `LmsrMarketMaker`
- `docs/14-identity-economy/14-knowledge-futures-market.md` — LMSR spec with formulas and properties
**Depends on**: None
**Accept when**:
- [x] LMSR cost function computed correctly — `cost()` at identity_economy_markets.rs:520, `b * ln(e^(q_d/b) + e^(q_f/b))`
- [x] Buy/sell update share quantities — `buy()` at :546, `sell()` at :559 adjust shares and return cost delta
- [x] Prices adjust dynamically — `price_deliver()` at :528 and `price_default()` at :538
- [ ] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn cost\|fn buy\|fn sell' crates/roko-chain/src/identity_economy_markets.rs
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-06: Sybil resistance active
- [x] Implement PersonalizedPageRank and SybilRank detection

**Spec** (doc 01 `01-erc-8004-three-registries.md`): PersonalizedPageRank (PPR) for trust propagation. Formula: `t_i = alpha * seed_i + (1-alpha) * sum(w_ij * t_j)` where `alpha` is teleport probability (default 0.15), `seed_i` is 1.0 for trusted seed nodes and 0.0 otherwise, `w_ij` are normalized edge weights. Iterative computation converges when max delta < epsilon (default 1e-6). SybilRank (Cao et al. 2012): flow-based detection — start with uniform trust budget on seed nodes, propagate through graph for `O(log n)` steps, agents with low final trust scores are flagged as potential Sybils. The detection threshold is derived from honest-region density. Collusion ring detection: identify dense subgraphs with few external connections — `internal_edge_density > 0.5 && external_edge_count / members.len() < 2` suggests a collusion cluster.

**Current code**: `PersonalizedPageRank` at `crates/roko-chain/src/identity_economy_identity.rs:148` with `alpha: f64`:150, `seed_set: Vec<u256>`:152, `max_iterations: u32`:154, `epsilon: f64`:156. `compute()` at line 161 returns `todo!()`. `InteractionGraph` at line 167 with `nodes: Vec<u256>`:170, `edges: Vec<(u256, u256, f64)>`:172. `SybilRankDetector` at line 177 with `walk_length: u32`, `trust_seed: Vec<u256>`, `threshold: f64`. `SybilScanResult` at line 188 with `flagged_agents`, `clusters`, `honest_region_size`, `scan_timestamp`. `SybilCluster` at line 201 with `members`, `internal_edge_density`, `external_edge_count`, `estimated_sybil_probability`. Reference implementation: `crates/roko-index/src/graph.rs` has a working `pagerank()` function with damping factor 0.85 — same algorithm, different data types.

**What to change**:
- Replace `todo!()` in `PersonalizedPageRank::compute()` at `crates/roko-chain/src/identity_economy_identity.rs:161`:
  ```rust
  pub fn compute(&self, graph: &InteractionGraph) -> HashMap<u256, f64> {
      let n = graph.nodes.len();
      let mut scores: HashMap<u256, f64> = graph.nodes.iter().map(|id| (*id, 1.0 / n as f64)).collect();
      let seed_val = 1.0 / self.seed_set.len().max(1) as f64;
      for _ in 0..self.max_iterations {
          let mut new_scores = HashMap::new();
          for &node in &graph.nodes {
              let seed_component = if self.seed_set.contains(&node) { self.alpha * seed_val } else { 0.0 };
              let neighbor_sum: f64 = graph.edges.iter()
                  .filter(|(_, to, _)| *to == node)
                  .map(|(from, _, w)| scores.get(from).unwrap_or(&0.0) * w)
                  .sum();
              new_scores.insert(node, seed_component + (1.0 - self.alpha) * neighbor_sum);
          }
          let max_delta = graph.nodes.iter()
              .map(|n| (new_scores[n] - scores[n]).abs())
              .fold(0.0f64, f64::max);
          scores = new_scores;
          if max_delta < self.epsilon { break; }
      }
      scores
  }
  ```
- Add `pub fn detect(&self, graph: &InteractionGraph) -> SybilScanResult` to `SybilRankDetector` — propagate trust from seed for `walk_length` steps, flag nodes with score below `threshold`
- Add `pub fn detect_collusion_rings(clusters: &[SybilCluster]) -> Vec<&SybilCluster>` filtering clusters with `internal_edge_density > 0.5 && external_edge_count < members.len() * 2`

**Reference files**:
- `crates/roko-chain/src/identity_economy_identity.rs:148-225` — `PersonalizedPageRank`:148, `InteractionGraph`:167, `SybilRankDetector`:177, `SybilScanResult`:188, `SybilCluster`:201
- `crates/roko-index/src/graph.rs` — working `pagerank()` function with damping (reference implementation for algorithm)
- `docs/14-identity-economy/01-erc-8004-three-registries.md` — PPR formula, SybilRank, collusion detection
**Depends on**: None
**Accept when**:
- [x] `PersonalizedPageRank::compute()` returns trust scores with convergence check — at identity_economy_identity.rs:165 with epsilon convergence
- [x] `SybilRankDetector::detect()` propagates trust and flags low-score agents — at :237 with walk_length propagation
- [x] Collusion rings identified by dense subgraph with few external connections — `detect_collusion_rings()` at :304 with density>0.5 && external<members*2
- [ ] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn compute\|fn detect\|collusion' crates/roko-chain/src/identity_economy_identity.rs
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-07: x402 HTTP payment protocol
- [x] Implement HTTP 402 challenge-response with ERC-3009

**Spec** (doc 08 `08-x402-micropayments.md`): Coinbase x402 protocol for machine-to-machine micropayments. Flow: (1) Agent sends HTTP request to a paid resource, (2) Server returns `HTTP 402 Payment Required` with `X-Payment-Challenge` header containing `{ amount: u64, recipient: Address, token: Address, network: String, nonce: u64, expires_at: u64 }`, (3) Agent signs an ERC-3009 `transferWithAuthorization` payload (gasless USDC transfer: `from`, `to`, `value`, `validAfter`, `validBefore`, `nonce`, `v`, `r`, `s`), (4) Agent retries the request with `X-Payment-Authorization` header containing the signed authorization, (5) Server verifies the authorization on-chain and grants access, returning `X-Payment-Receipt` with `{ tx_hash, amount, receipt_id }`. Self-funding loop: agent earns USDC from bounties (IDECON-03) and spends it on resources via x402.

**Current code**: `X402Client` at `crates/roko-chain/src/identity_economy_identity.rs:749` with `signer: String`, `balance: u64`, `http: String` fields — no methods. `X402Receipt` at line 922 with receipt fields. `x402_receipt` field referenced in `crates/roko-chain/src/identity_economy_markets.rs:220`. Phase2 attestation at `crates/roko-chain/src/phase2.rs:1771`. No HTTP challenge/response logic, no ERC-3009 signing.

**What to change**:
- Add methods to `X402Client` at `crates/roko-chain/src/identity_economy_identity.rs:749`:
  ```rust
  impl X402Client {
      pub fn parse_challenge(header: &str) -> Result<PaymentChallenge, Error> { /* deserialize JSON */ }
      pub fn sign_authorization(&self, challenge: &PaymentChallenge) -> Result<Erc3009Auth, Error> {
          // Construct ERC-3009 transferWithAuthorization payload
          // Sign with self.signer key
          // Return { from, to, value, valid_after, valid_before, nonce, v, r, s }
      }
      pub fn make_payment_header(&self, auth: &Erc3009Auth) -> String { /* serialize to header */ }
      pub async fn pay_and_retry(&self, url: &str, challenge: &PaymentChallenge) -> Result<Response, Error> { /* ... */ }
  }
  ```
- Add `PaymentChallenge` struct with `amount`, `recipient`, `token`, `network`, `nonce`, `expires_at` fields
- Add `Erc3009Auth` struct with ERC-3009 `transferWithAuthorization` fields
- Wire into agent resource acquisition: when a tool call returns HTTP 402, automatically attempt x402 payment

**Reference files**:
- `crates/roko-chain/src/identity_economy_identity.rs:749-756` — `X402Client` struct (add methods here)
- `crates/roko-chain/src/identity_economy_identity.rs:922` — `X402Receipt` struct
- `crates/roko-chain/src/identity_economy_markets.rs:220` — `x402_receipt` field reference
- `crates/roko-chain/src/phase2.rs:1771` — Phase2 attestation for x402
- `docs/14-identity-economy/08-x402-micropayments.md` — full x402 protocol, ERC-3009, self-funding loop
**Depends on**: None
**Accept when**:
- [x] `X402Client::parse_challenge()` deserializes `X-Payment-Challenge` header
- [x] `X402Client::sign_authorization()` produces ERC-3009 `transferWithAuthorization` signature
- [x] `X402Client::pay()` executes payment flow with balance deduction and receipt
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn parse_challenge\|fn sign_authorization\|fn pay_and_retry\|X402Client' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-08: Regulatory compliance enforcement
- [x] Wire compliance template policies into gate evaluation

**Spec** (doc 15 `15-regulatory-moat-and-current-status.md`): Three pre-certified compliance templates: (1) `SecTradingTemplate` — SEC trading compliance with `BestExecutionPolicy` (agent must demonstrate best-price selection across venues), `PositionLimitPolicy` (max position size per asset), `WashTradeSafeguard` (detect self-dealing patterns), `InsiderTradingPolicy` (information barriers between agents); (2) `HipaaClinicalTemplate` — HIPAA clinical data compliance with `MinimumNecessaryPolicy` (access only required PHI fields), `AuditTrailPolicy` (all PHI access logged with purpose), `DeidentificationPolicy` (strip identifiers before storage); (3) `GdprDataTemplate` — GDPR data protection with `ConsentTrackingPolicy` (track per-field consent), `RightToErasurePolicy` (delete data on request), `DataMinimizationPolicy` (collect only necessary data). Each template consists of multiple policy structs, each with a `fn check(&self, action: &Action) -> ComplianceResult` method. `ComplianceGate` integrates into the gate pipeline as a domain-specific gate.

**Current code**: `SecTradingTemplate` at `crates/roko-chain/src/identity_economy_markets.rs:513` with `best_execution`, `position_limits`, `wash_trade`, `insider_trading` fields — all empty marker types with no check methods. `HipaaClinicalTemplate` at line 540 with `minimum_necessary`, `audit_trail`, `deidentification` fields — markers only. `GdprDataTemplate` at line 563 with `consent_tracking`, `right_to_erasure`, `data_minimization` — markers only. `ComplianceGate` placeholder at line 35 — macro-generated marker type with no gate logic.

**What to change**:
- Replace marker types with policy structs that have `check()` methods. Start with `SecTradingTemplate`:
  ```rust
  pub struct BestExecutionPolicy { pub max_slippage_bps: u32, pub min_venues_checked: u32 }
  impl BestExecutionPolicy {
      pub fn check(&self, selected_price: f64, best_available: f64, venues_checked: u32) -> ComplianceResult {
          let slippage_bps = ((selected_price - best_available) / best_available * 10_000.0).abs() as u32;
          if venues_checked < self.min_venues_checked { return ComplianceResult::Violation("Insufficient venues checked".into()); }
          if slippage_bps > self.max_slippage_bps { return ComplianceResult::Violation(format!("Slippage {}bps exceeds limit {}bps", slippage_bps, self.max_slippage_bps)); }
          ComplianceResult::Pass
      }
  }
  pub enum ComplianceResult { Pass, Violation(String), Warning(String) }
  ```
- Implement `ComplianceGate` as a `Gate` trait impl in `crates/roko-gate/src/` or inline in `identity_economy_markets.rs` — wraps template checks, returns gate verdict
- Wire `ComplianceGate` into the gate pipeline: register as an optional gate for agents with `domain = "trading"` or `domain = "clinical"` in their config

**Reference files**:
- `crates/roko-chain/src/identity_economy_markets.rs:35` — `ComplianceGate` marker (replace with real gate)
- `crates/roko-chain/src/identity_economy_markets.rs:513-585` — `SecTradingTemplate`:513, `HipaaClinicalTemplate`:540, `GdprDataTemplate`:563 (replace marker fields with policy structs)
- `crates/roko-gate/src/` — gate pipeline (register `ComplianceGate` here)
- `crates/roko-agent/src/safety/provenance.rs:50` — `Custody` struct (record compliance violations)
- `docs/14-identity-economy/15-regulatory-moat-and-current-status.md` — three templates, policy specs
**Depends on**: SAFE-02 (custody persistence for recording violations)
**Accept when**:
- [x] `BestExecutionPolicy::check()` validates slippage and venue count
- [x] At least one template (`SecTradingTemplate`) has all policy `check()` methods implemented
- [x] `ComplianceGate` aggregates all policy checks and returns first violation
- [x] `PositionLimitPolicy::check()` validates position size
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'fn check\|ComplianceResult\|ComplianceGate' crates/roko-chain/src/ --include='*.rs'
grep -rn 'ComplianceGate' crates/roko-gate/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

### IDECON-09: Three hiring models (VRF, auction, direct) not implemented

- [x] Implement random VRF assignment, blind auction, and direct hire dispatch

**Spec** (doc 12 `12-three-hiring-models.md`): Three job assignment models: (1) Random VRF — verifiable random function selects agent from eligible pool using power-of-two-choices (Ousterhout 2013: randomly pick 2 candidates, compare scores `reputation * capability_match`, assign best — this is O(1) per dispatch and achieves near-optimal load balancing), (2) Blind auction — sealed-bid variants (FPSB first-price, Vickrey second-price, Dutch descending-price) with commit-reveal protocol (`CommitRecord { commit_hash, agent_id, timestamp }`, `RevealRecord { bid_amount, nonce, agent_id }`), (3) Direct hire — operator specifies agent directly with anti-centralization fee computed as `base_fee * (1 + ln(1 + repeat_count))` where `repeat_count` is the number of times this hirer has hired this specific agent in the last 30 days. All models produce `DispatchDecision { winner: AgentId, payment: f64, model: HiringModel, audit_trail: Vec<String> }`.

**Current code**: `BountySpec` at `crates/roko-chain/src/identity_economy_markets.rs:57` with `title`, `description`, `budget`, `deadline`, `required_capabilities` fields. `SparrowBid` at line 105 with `bidder`, `amount`, `reputation_score`, `capabilities` fields. `AuctionType` enum at `crates/roko-chain/src/phase2.rs:1335` with `Vickrey`, `FirstPrice`, `Dutch` variants. `CommitRecord` at phase2.rs:1371 and `RevealRecord` at line 1385. No `HiringModel` enum in `identity_economy_markets.rs`. No dispatch functions. No anti-centralization fee logic.

**What to change**:
- Add to `crates/roko-chain/src/identity_economy_markets.rs`:
  ```rust
  pub enum HiringModel {
      RandomVrf,
      BlindAuction { auction_type: AuctionType },
      DirectHire { agent_id: String },
  }
  pub struct DispatchDecision {
      pub winner: String,      // AgentId
      pub payment: f64,
      pub model: HiringModel,
      pub audit_trail: Vec<String>,
  }
  pub fn dispatch_random_vrf(pool: &[SparrowBid], bounty: &BountySpec) -> Option<DispatchDecision> {
      if pool.len() < 2 { return pool.first().map(|b| DispatchDecision { winner: b.bidder.clone(), payment: bounty.budget as f64, model: HiringModel::RandomVrf, audit_trail: vec![] }); }
      // Power-of-two-choices: pick 2 random candidates
      let mut rng = rand::rng();
      let i = rng.random_range(0..pool.len());
      let j = loop { let k = rng.random_range(0..pool.len()); if k != i { break k; } };
      let score_i = pool[i].reputation_score; // * capability_match
      let score_j = pool[j].reputation_score;
      let winner = if score_i >= score_j { &pool[i] } else { &pool[j] };
      Some(DispatchDecision { winner: winner.bidder.clone(), payment: bounty.budget as f64, model: HiringModel::RandomVrf, audit_trail: vec![format!("P2C: {} vs {}", i, j)] })
  }
  pub fn anti_centralization_fee(base_fee: f64, repeat_count: u32) -> f64 {
      base_fee * (1.0 + (1.0 + repeat_count as f64).ln())
  }
  ```
- `dispatch_blind_auction()` delegates to `select_winner()` from IDECON-03 for Vickrey, implements first-price and Dutch variants separately

**Reference files**:
- `crates/roko-chain/src/identity_economy_markets.rs:57-120` — `BountySpec`:57, `SparrowBid`:105 (existing structs)
- `crates/roko-chain/src/phase2.rs:1335-1390` — `AuctionType` enum:1335, `CommitRecord`:1371, `RevealRecord`:1385
- `crates/roko-chain/src/identity_economy_markets.rs:105` — `score_bid()` and `select_winner()` from IDECON-03 (Vickrey scoring)
- `docs/14-identity-economy/12-three-hiring-models.md` — power-of-two-choices, anti-centralization fee formula
**Depends on**: IDECON-03 (Vickrey scoring for blind auction model)
**Accept when**:
- [x] `dispatch_random_vrf()` selects from pool using power-of-two-choices (pick 2, compare, assign best)
- [x] `dispatch_blind_auction()` delegates to `select_winner()` for Vickrey scoring
- [x] `anti_centralization_fee()` returns `base_fee * (1 + ln(1 + repeat_count))`
- [x] `DispatchDecision` struct captures winner, payment, model, and audit trail
- [x] `cargo test -p roko-chain`
**Verify**:
```bash
grep -rn 'HiringModel\|DispatchDecision\|dispatch_random_vrf\|anti_centralization_fee' crates/roko-chain/src/ --include='*.rs'
cargo test -p roko-chain
```

**Priority**: P2

---

## Verify
```bash
cargo test -p roko-chain
```
