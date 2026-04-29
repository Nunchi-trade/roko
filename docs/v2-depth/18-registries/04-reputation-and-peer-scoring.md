# Reputation and Peer Scoring

> Depth for [22-REGISTRIES.md](../../unified/22-REGISTRIES.md). How reputation emerges as a Score Cell with EMA internals, and peer scoring as a Pipeline of three Score Cells.

**Depends on**: [01-SIGNAL](../../unified/01-SIGNAL.md) (Signal, demurrage, content addressing), [02-CELL](../../unified/02-CELL.md) (Score protocol, Verify protocol, predict-publish-correct), [03-GRAPH](../../unified/03-GRAPH.md) (Pipeline pattern, Loop pattern), [07-LEARNING](../../unified/07-LEARNING.md) (L2 routing, adaptive thresholds), [22-REGISTRIES](../../unified/22-REGISTRIES.md) (ERC-8004 identity, reputation tiers, TraceRank)

**Source docs**: `docs/08-chain/09-peer-scoring-3-layer.md`, `docs/08-chain/14-reputation-system-7-domain.md`, `docs/08-chain/13-vickrey-reputation-auction.md`

---

## 1. The Insight: Reputation IS a Score Cell

The v1 registries spec (22-REGISTRIES SS3) defines reputation as an EMA over attestation deltas, computed inside a Solidity contract. The TraceRank model adds five dimensions (consistency, breadth, depth, recency, collaboration). Both are presented as standalone computations -- parallel to, but not unified with, the Score protocol that governs quality evaluation everywhere else in Roko.

The unified redesign: **reputation is a Score Cell**. The EMA update is the Cell's internal state. The seven reputation domains are seven Signal streams. The predict-publish-correct loop -- the same mechanism that calibrates Verify Cell thresholds and model routing -- calibrates reputation. There is no separate "reputation system." There is a Score Cell whose input is attestation Signals and whose output is reputation Signals.

Why this matters:

- **One learning mechanism.** Verify Cell thresholds learn via EMA on Verdicts. Model routing learns via bandit arms. Reputation learns via EMA on attestations. All three are Score Cells in Loop Graphs, updated by the same predict-publish-correct pattern. One mechanism to understand, debug, and tune.
- **Composability.** A reputation Score Cell composes with Route Cells (reputation-weighted hiring), Verify Cells (reputation as evidence), and Store Cells (reputation as Signal with demurrage). No special-case wiring.
- **Observability for free.** The same UsageLens and CFactorLens that observe Verify Pipelines observe reputation updates. No bespoke reputation dashboard.

---

## 2. The Reputation Score Cell

### 2.1 Seven Domains, One Cell

Each of the seven reputation domains (coding, security, research, chain, knowledge, operations, strategy) is a separate input stream to the same Score Cell type. The Cell is instantiated once per agent, processing attestation Signals across all domains.

```rust
/// Reputation Score Cell.
///
/// Maintains per-domain EMA scores for a single agent.
/// Implements the Score protocol: attestation Signal in, reputation Signal out.
/// Participates in predict-publish-correct: predicts next-period reputation,
/// publishes the prediction as a Pulse, corrects when the actual attestation arrives.
pub struct ReputationScoreCell {
    /// The agent whose reputation this Cell tracks.
    agent_identity: u128,
    /// Per-domain EMA state.
    domains: HashMap<ReputationDomain, DomainState>,
    /// Adaptive alpha configuration.
    alpha_config: AdaptiveAlphaConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ReputationDomain {
    Coding,
    Security,
    Research,
    Chain,
    Knowledge,
    Operations,
    Strategy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainState {
    /// Current EMA score, [0.0, 1.0].
    pub score: f64,
    /// Total attestation count (drives adaptive alpha).
    pub attestation_count: u64,
    /// Timestamp of last attestation (drives decay).
    pub last_attested: DateTime<Utc>,
    /// Running prediction for next attestation (predict-publish-correct).
    pub predicted_next: f64,
}

impl Cell for ReputationScoreCell {
    fn id(&self) -> CellId {
        CellId::compute(
            &format!("reputation-{}", self.agent_identity),
            &Version::new(1, 0, 0),
            &Author::System,
        )
    }
    fn name(&self) -> &str { "reputation-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let mut outputs = Vec::new();

        for signal in &input {
            match &signal.kind {
                Kind::Attestation => {
                    let att: Attestation = Signal::extract(signal)?;
                    let updated = self.process_attestation(&att, ctx).await?;
                    outputs.push(updated);
                }
                Kind::DecayTick => {
                    // Apply 30-day half-life decay to all domains
                    let decayed = self.apply_decay(ctx).await?;
                    outputs.extend(decayed);
                }
                _ => {} // Ignore irrelevant Signals
            }
        }

        Ok(outputs)
    }
}
```

### 2.2 The EMA Update: Predict-Publish-Correct

The EMA update is the Score Cell's internal calibration mechanism. It follows the predict-publish-correct pattern from [02-CELL.md](../../unified/02-CELL.md):

1. **Predict**: Before an attestation arrives, the Cell predicts what the next outcome will be (based on the current EMA).
2. **Publish**: The prediction is emitted as a Pulse on the Bus.
3. **Correct**: When the actual attestation arrives, the Cell computes the prediction error and updates the EMA.

```rust
impl ReputationScoreCell {
    /// Process a single attestation.
    ///
    /// The EMA update:
    ///   R_new = alpha * F + (1 - alpha) * R_old
    ///
    /// Where:
    ///   F = the attestation feedback (normalized to [0, 1])
    ///   R_old = current domain score
    ///   alpha = adaptive learning rate (see S3)
    async fn process_attestation(
        &mut self,
        att: &Attestation,
        ctx: &CellContext,
    ) -> Result<Signal, CellError> {
        let domain = att.domain;
        let state = self.domains.entry(domain).or_insert(DomainState {
            score: 0.5,  // neutral prior
            attestation_count: 0,
            last_attested: Utc::now(),
            predicted_next: 0.5,
        });

        // 1. Compute prediction error (for calibration tracking)
        let prediction_error = att.feedback - state.predicted_next;

        // 2. Get adaptive alpha
        let alpha = self.alpha_config.alpha_for(state.attestation_count);

        // 3. EMA update
        let old_score = state.score;
        state.score = alpha * att.feedback + (1.0 - alpha) * old_score;
        state.attestation_count += 1;
        state.last_attested = Utc::now();

        // 4. Update prediction for next attestation
        //    The prediction IS the current EMA -- the best estimator
        //    of the next observation given the current state.
        state.predicted_next = state.score;

        // 5. Publish correction Pulse (predict-publish-correct)
        ctx.bus.publish(Pulse::new(
            &format!("reputation.correction.{}.{:?}", self.agent_identity, domain),
            json!({
                "agent": self.agent_identity,
                "domain": domain,
                "old_score": old_score,
                "new_score": state.score,
                "feedback": att.feedback,
                "alpha": alpha,
                "prediction_error": prediction_error,
                "attestation_count": state.attestation_count,
            }),
        )).await?;

        // 6. Emit reputation Signal
        Ok(Signal::new(Kind::Reputation, ReputationUpdate {
            agent_identity: self.agent_identity,
            domain,
            score: state.score,
            attestation_count: state.attestation_count,
            alpha,
            prediction_error,
        }))
    }
}
```

The prediction error (`att.feedback - state.predicted_next`) is the key diagnostic. A well-calibrated reputation Cell has a prediction error centered at zero with low variance. Persistent positive errors mean the Cell is underestimating the agent. Persistent negative errors mean overestimating. The adaptive alpha (see next section) controls how quickly the Cell corrects.

---

## 3. Adaptive Alpha as Internal Calibration

The learning rate alpha is not fixed. It adapts based on the number of observations -- a Score Cell's internal calibration mechanism. The intuition: few observations warrant rapid adjustment (high alpha); many observations warrant stability (low alpha).

```rust
/// Adaptive alpha configuration.
///
/// The alpha schedule mirrors the adaptive Verify threshold system:
/// new cells (few observations) adjust rapidly; mature cells
/// (many observations) adjust slowly. The breakpoints and rates
/// are configurable but the defaults are calibrated from the
/// reputation source material.
pub struct AdaptiveAlphaConfig {
    /// Alpha breakpoints: (observation_count, alpha_value).
    /// Interpolated linearly between breakpoints.
    pub breakpoints: Vec<(u64, f64)>,
}

impl Default for AdaptiveAlphaConfig {
    fn default() -> Self {
        Self {
            breakpoints: vec![
                (0,   0.30),   // 0-10 jobs: high learning rate, rapid convergence
                (10,  0.30),
                (11,  0.15),   // 11-50 jobs: moderate, settling into estimate
                (50,  0.15),
                (51,  0.08),   // 51-200 jobs: low, estimate is reliable
                (200, 0.08),
                (201, 0.04),   // 200+ jobs: minimal, only significant events shift
            ],
        }
    }
}

impl AdaptiveAlphaConfig {
    /// Get the alpha for a given observation count.
    ///
    /// Piecewise constant with the breakpoints defined above.
    /// Within a band, alpha is constant. At boundaries, it
    /// steps to the next value.
    pub fn alpha_for(&self, count: u64) -> f64 {
        for window in self.breakpoints.windows(2) {
            let (c_lo, a_lo) = window[0];
            let (c_hi, _a_hi) = window[1];
            if count >= c_lo && count <= c_hi {
                return a_lo;
            }
        }
        // Beyond all breakpoints: use the last value
        self.breakpoints.last().map(|b| b.1).unwrap_or(0.04)
    }
}
```

This is the same design pattern as the cascade router's three-stage transition (static -> confidence -> UCB). The difference is scope: the cascade router's stages control model selection; the reputation Cell's alpha controls belief update speed. Same mechanism, different domain.

### 3.1 Convergence Properties

With adaptive alpha, the EMA converges at different rates depending on the observation count:

| Phase | Observations | Alpha | Half-convergence (steps) | Character |
|---|---|---|---|---|
| Bootstrap | 0-10 | 0.30 | ~2 | Rapid. Reputation swings widely. Intentional: early signal is strong. |
| Stabilizing | 11-50 | 0.15 | ~4 | Moderate. Large shifts require consistent evidence. |
| Mature | 51-200 | 0.08 | ~8 | Slow. Reputation is sticky. Single bad outcome barely moves it. |
| Established | 200+ | 0.04 | ~17 | Very slow. Only sustained behavioral change shifts the score. |

Half-convergence: the number of identical observations needed to move halfway from the current score to the observation value. Computed as `ceil(log(0.5) / log(1 - alpha))`.

---

## 4. 30-Day Decay as Signal Demurrage

The v1 spec defines reputation decay as a standalone formula: `decayed = 0.5 + (score - 0.5) * 0.5^(days/30)`. The unified redesign: **30-day decay IS demurrage applied to reputation Signals**.

Reputation Signals have a demurrage balance just like knowledge Signals (see [demurrage-economics.md](../01-signal/demurrage-economics.md)). The half-life is 30 days. The mechanism is identical:

1. Reputation Signals pay a holding cost proportional to their distance from the neutral point (0.5).
2. Active attestations reinforce the Signal, resetting its demurrage clock.
3. Absence of attestations lets the Signal decay toward 0.5 (neutral), not toward 0 (destruction).

```rust
/// Apply 30-day half-life decay to all domains.
///
/// decayed = 0.5 + (score - 0.5) * 0.5^(days / 30)
///
/// This IS demurrage. The reputation Signal's balance decays toward
/// the neutral point (0.5) unless refreshed by attestation.
/// The half-life of 30 days means an idle agent loses half
/// their deviation from neutral each month.
impl ReputationScoreCell {
    async fn apply_decay(
        &mut self,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let now = Utc::now();
        let mut outputs = Vec::new();

        for (domain, state) in &mut self.domains {
            let days_since = (now - state.last_attested)
                .num_seconds() as f64 / 86_400.0;

            if days_since < 1.0 {
                continue;  // No decay for recently attested domains
            }

            let old_score = state.score;
            let decay_factor = 0.5_f64.powf(days_since / 30.0);

            // Decay toward neutral (0.5), not toward zero
            state.score = 0.5 + (old_score - 0.5) * decay_factor;

            // Only emit if the change is material
            if (old_score - state.score).abs() > 0.001 {
                outputs.push(Signal::new(Kind::Reputation, ReputationDecay {
                    agent_identity: self.agent_identity,
                    domain: *domain,
                    old_score,
                    new_score: state.score,
                    days_since_attestation: days_since,
                    decay_factor,
                }));
            }
        }

        Ok(outputs)
    }
}
```

### 4.1 Why 0.5, Not 0

Decay toward 0.5 (neutral) rather than 0 (minimum) is deliberate. An agent who stops participating should not be treated as a bad actor -- they should be treated as unknown. 0.5 means "we have no current evidence." 0 means "we have strong evidence of incompetence." The two are categorically different, and conflating them would punish sabbaticals.

### 4.2 Demurrage Unification

The reputation decay formula and the knowledge demurrage formula (from [01-SIGNAL.md](../../unified/01-SIGNAL.md) SS6) are the same exponential decay with different parameters:

| Property | Knowledge Demurrage | Reputation Decay |
|---|---|---|
| Decay target | 0 (worthless) | 0.5 (neutral) |
| Half-life | Kind-dependent (3.5 to 693 days) | 30 days (fixed) |
| Reinforcement | Retrieved, Cited, Confirmed | Attested |
| Novelty weighting | Yes (anti-hoarding) | No (one agent, one score) |
| Tier progression | Transient -> Working -> Consolidated -> Persistent | Gray -> Copper -> Silver -> Gold -> Amber |

Both are instances of the same pattern: **a Score Cell whose balance decays unless reinforced by evidence of continued relevance**. Knowledge relevance is measured by retrieval. Reputation relevance is measured by attestation.

---

## 5. Discipline States as Tier Demotion

The v1 source material defines four discipline states: GoodStanding, Probation, Suspension, Banned. Recovery from Probation requires 10 jobs with average >= 0.6 and 30 clean days. Recovery from Suspension requires a 90-day wait, 2x stake, and verification.

The unified redesign: **discipline states ARE tier demotion**. There is no separate discipline system. Infractions demote the agent's reputation tier, and the existing tier system handles the consequences.

```rust
/// Discipline actions as tier demotion events.
///
/// Instead of a parallel GoodStanding/Probation/Suspension/Banned
/// state machine, infractions trigger tier demotion through the
/// same mechanism as natural reputation decay. The tier system
/// already controls access to job tiers, auction eligibility,
/// and meta-agent creation.
pub struct DisciplineCell {
    reputation_store: Arc<dyn Store>,
}

/// Infraction types and their effects.
///
/// Each infraction specifies:
/// - stake_slash_pct: fraction of staked balance to burn
/// - reputation_delta: per-domain reputation penalty
/// - domain_scope: which domains are affected
#[derive(Debug, Clone)]
pub struct Infraction {
    pub kind: InfractionKind,
    pub stake_slash_pct: f64,
    pub reputation_delta: f64,
    pub domain_scope: DomainScope,
}

#[derive(Debug, Clone)]
pub enum InfractionKind {
    MissedDeadline,    // 1% stake, -0.05 in job domain
    AbandonedJob,      // 3% stake, -0.10 in job domain
    Plagiarism,        // 10% stake, -0.30 in job domain
    TeeViolation,      // 10% total stake, -0.50 ALL domains
}

#[derive(Debug, Clone)]
pub enum DomainScope {
    /// Penalty applies only to the domain of the infraction.
    Single(ReputationDomain),
    /// Penalty applies to ALL domains (nuclear option).
    All,
}

impl Cell for DisciplineCell {
    fn id(&self) -> CellId { CellId::named("discipline") }
    fn name(&self) -> &str { "discipline" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score, ProtocolId::Store] }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let infraction: Infraction = Signal::extract(&input[0])?;
        let agent_id: u128 = ctx.get_param("agent_identity")?;

        // 1. Slash stake
        if infraction.stake_slash_pct > 0.0 {
            let current_stake = self.reputation_store
                .query_stake(agent_id).await?;
            let slash_amount = (current_stake as f64
                * infraction.stake_slash_pct) as u64;
            self.reputation_store
                .slash_stake(agent_id, slash_amount).await?;
        }

        // 2. Apply reputation penalty as negative attestation
        //    This flows through the ReputationScoreCell's EMA update,
        //    which naturally triggers tier demotion if the score
        //    drops below the tier threshold.
        let domains = match &infraction.domain_scope {
            DomainScope::Single(d) => vec![*d],
            DomainScope::All => ReputationDomain::all().to_vec(),
        };

        let mut outputs = Vec::new();
        for domain in &domains {
            let att = Attestation {
                agent_identity: agent_id,
                domain: *domain,
                feedback: infraction.reputation_delta,  // negative
                source: AttestationSource::Discipline(infraction.kind.clone()),
                evidence_hash: ctx.flow_id().as_bytes(),
            };
            outputs.push(Signal::new(Kind::Attestation, att));
        }

        Ok(outputs)
    }
}
```

### 5.1 Recovery as Natural Tier Re-Promotion

Recovery from disciplinary demotion uses the same mechanism as natural tier promotion. An agent demoted from Silver to Copper re-promotes to Silver the same way any Copper agent promotes: by accumulating positive attestations until their reputation score crosses the tier threshold.

The v1 Probation rules (10 jobs, avg >= 0.6, 30 clean days) become tier promotion criteria:

| v1 Concept | Unified Equivalent |
|---|---|
| GoodStanding | Agent's tier matches or exceeds their historical peak |
| Probation | Agent demoted one tier; regains it by crossing the promotion threshold |
| Suspension | Agent demoted two or more tiers; 90-day cooldown implemented as a temporary capability restriction on the ERC-8004 identity |
| Banned | Agent demoted to Gray with a permanent flag; identity can be transferred (sold) but starts fresh |

This eliminates a parallel state machine. Discipline and promotion are the same system, running in opposite directions.

---

## 6. Three-Layer Peer Scoring as a Pipeline of Score Cells

The v1 source material defines three layers of peer scoring: Protocol, Application, and Economic. Combined score: `protocol * 0.40 + application * 0.35 + economic * 0.25`. The unified redesign: **the three layers are three Score Cells in a Pipeline Graph.**

```toml
[graph]
name    = "peer-scoring-pipeline"
pattern = "pipeline"

[[nodes]]
id       = "protocol-score"
cell     = "roko:protocol-peer-score"
protocol = "Score"

[[nodes]]
id       = "application-score"
cell     = "roko:application-peer-score"
protocol = "Score"

[[nodes]]
id       = "economic-score"
cell     = "roko:economic-peer-score"
protocol = "Score"

[[nodes]]
id       = "combine"
cell     = "roko:weighted-combine-score"
protocol = "Score"
[nodes.params]
weights = [0.40, 0.35, 0.25]

[[edges]]
from = "protocol-score"
to   = "combine"

[[edges]]
from = "application-score"
to   = "combine"

[[edges]]
from = "economic-score"
to   = "combine"
```

Note: this is not a linear pipeline -- it is a fan-in. All three Score Cells execute in parallel (they have no data dependencies), and their outputs converge at the `combine` node. The Graph engine's parallel executor handles this automatically.

### 6.1 Protocol Score Cell (GossipSub v1.1)

Scores peer behavior at the network protocol level. Observes message delivery, connection stability, and network hygiene.

```rust
/// Protocol-layer peer scoring.
///
/// Based on GossipSub v1.1 peer scoring parameters.
/// Observes: message delivery ratio, duplicate rate, connection uptime,
/// IP colocation (Sybil indicator).
pub struct ProtocolPeerScoreCell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolScore {
    /// Fraction of expected messages actually delivered. [0, 1].
    pub delivery_ratio: f64,
    /// Fraction of messages that were duplicates. [0, 1]. Lower is better.
    pub duplicate_ratio: f64,
    /// Connection uptime as fraction of observation window. [0, 1].
    pub uptime: f64,
    /// Number of distinct IPs this peer shares an IP with.
    /// High colocation = potential Sybil. Penalized.
    pub ip_colocation_factor: f64,
    /// Combined protocol score. [0, 1].
    pub combined: f64,
}

impl Cell for ProtocolPeerScoreCell {
    fn id(&self) -> CellId { CellId::named("protocol-peer-score") }
    fn name(&self) -> &str { "protocol-peer-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let peer_stats: PeerNetworkStats = Signal::extract(&input[0])?;

        let delivery = peer_stats.messages_delivered as f64
            / peer_stats.messages_expected.max(1) as f64;
        let duplicate = peer_stats.duplicates as f64
            / peer_stats.messages_received.max(1) as f64;
        let uptime = peer_stats.connected_seconds as f64
            / peer_stats.observation_seconds.max(1) as f64;

        // IP colocation penalty: 0 peers sharing IP = 1.0,
        // 5+ peers sharing = 0.0 (likely Sybil)
        let colocation = 1.0 - (peer_stats.ip_colocation_count as f64 / 5.0)
            .clamp(0.0, 1.0);

        let combined = 0.35 * delivery
            + 0.20 * (1.0 - duplicate)
            + 0.25 * uptime
            + 0.20 * colocation;

        Ok(vec![Signal::new(Kind::PeerScore, ProtocolScore {
            delivery_ratio: delivery,
            duplicate_ratio: duplicate,
            uptime,
            ip_colocation_factor: colocation,
            combined,
        })])
    }
}
```

### 6.2 Application Score Cell

Scores peer behavior at the application layer. Five sub-dimensions covering knowledge quality, anomaly detection, job reliability, simulation participation, and governance.

```rust
/// Application-layer peer scoring.
///
/// Observes: knowledge contribution quality, behavioral anomalies,
/// job completion reliability, simulation participation, governance votes.
pub struct ApplicationPeerScoreCell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationScore {
    /// Quality of knowledge contributions (validated, challenged, retracted).
    pub knowledge: f64,     // weight: 0.30
    /// Anomaly score: low = normal behavior, high = suspicious. Inverted.
    pub anomaly: f64,       // weight: 0.20
    /// Job completion rate and quality.
    pub job_reliability: f64, // weight: 0.30
    /// Participation in simulation/arena runs.
    pub simulation: f64,    // weight: 0.10
    /// Governance participation rate and quality.
    pub governance: f64,    // weight: 0.10
    /// Combined application score. [0, 1].
    pub combined: f64,
}

impl Cell for ApplicationPeerScoreCell {
    fn id(&self) -> CellId { CellId::named("application-peer-score") }
    fn name(&self) -> &str { "application-peer-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let peer_app: PeerApplicationStats = Signal::extract(&input[0])?;

        let knowledge = compute_knowledge_score(&peer_app);
        let anomaly = 1.0 - compute_anomaly_score(&peer_app);  // inverted
        let job_reliability = compute_job_reliability(&peer_app);
        let simulation = compute_simulation_score(&peer_app);
        let governance = compute_governance_score(&peer_app);

        let combined = 0.30 * knowledge
            + 0.20 * anomaly
            + 0.30 * job_reliability
            + 0.10 * simulation
            + 0.10 * governance;

        Ok(vec![Signal::new(Kind::PeerScore, ApplicationScore {
            knowledge,
            anomaly,
            job_reliability,
            simulation,
            governance,
            combined,
        })])
    }
}

/// Knowledge score: positive validations vs challenges.
/// An agent who publishes knowledge that gets validated earns high score.
/// An agent whose knowledge gets challenged and retracted earns low score.
fn compute_knowledge_score(stats: &PeerApplicationStats) -> f64 {
    let total = stats.validations + stats.challenges;
    if total == 0 { return 0.5; }  // neutral: no data
    stats.validations as f64 / total as f64
}

/// Job reliability: completion rate weighted by job tier.
/// Higher-tier job completions count more than lower-tier.
fn compute_job_reliability(stats: &PeerApplicationStats) -> f64 {
    if stats.jobs_assigned == 0 { return 0.5; }
    let weighted_completions: f64 = stats.job_completions.iter()
        .map(|jc| jc.completion_rate * jc.tier_weight)
        .sum();
    let total_weight: f64 = stats.job_completions.iter()
        .map(|jc| jc.tier_weight)
        .sum();
    if total_weight == 0.0 { return 0.5; }
    (weighted_completions / total_weight).clamp(0.0, 1.0)
}
```

### 6.3 Economic Score Cell

Scores peer behavior at the economic layer. Stake-weighted with tier multiplier.

```rust
/// Economic-layer peer scoring.
///
/// Observes: staked amount, tier, slashing history, collateral ratio.
/// Stake-weighted: agents with more at risk are more trustworthy.
/// Tier multiplier: higher tiers get a bonus (they earned it).
pub struct EconomicPeerScoreCell;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicScore {
    /// Normalized stake: agent's stake / max observed stake. [0, 1].
    pub stake_normalized: f64,
    /// Tier multiplier: Gray=0.5, Copper=0.7, Silver=1.0, Gold=1.2, Amber=1.5.
    pub tier_multiplier: f64,
    /// Slash ratio: total slashed / total staked. Lower is better.
    pub slash_ratio: f64,
    /// Combined economic score. [0, 1].
    pub combined: f64,
}

impl Cell for EconomicPeerScoreCell {
    fn id(&self) -> CellId { CellId::named("economic-peer-score") }
    fn name(&self) -> &str { "economic-peer-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let peer_econ: PeerEconomicStats = Signal::extract(&input[0])?;
        let max_stake = ctx.get_param::<f64>("max_observed_stake")?;

        let stake_normalized = if max_stake > 0.0 {
            (peer_econ.staked_amount as f64 / max_stake).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let tier_mult = match peer_econ.tier {
            ReputationTier::Gray   => 0.5,
            ReputationTier::Copper => 0.7,
            ReputationTier::Silver => 1.0,
            ReputationTier::Gold   => 1.2,
            ReputationTier::Amber  => 1.5,
        };

        let slash_ratio = if peer_econ.total_staked > 0 {
            peer_econ.total_slashed as f64 / peer_econ.total_staked as f64
        } else {
            0.0
        };

        // Combined: stake * tier_mult * (1 - slash_ratio), normalized to [0, 1]
        let raw = stake_normalized * tier_mult * (1.0 - slash_ratio);
        let combined = (raw / 1.5).clamp(0.0, 1.0);  // 1.5 = max tier_mult

        Ok(vec![Signal::new(Kind::PeerScore, EconomicScore {
            stake_normalized,
            tier_multiplier: tier_mult,
            slash_ratio,
            combined,
        })])
    }
}
```

### 6.4 Weighted Combine Cell

The combine Cell is a generic Score Cell that takes N input Signals and produces a single weighted aggregate. This is reusable -- the same Cell works for any fan-in of Score Cells.

```rust
/// Weighted combine: N Score Signals -> one combined Score Signal.
///
/// Weights are configurable via Cell params.
/// For peer scoring: [0.40, 0.35, 0.25] (protocol, application, economic).
pub struct WeightedCombineScoreCell {
    weights: Vec<f64>,
}

impl Cell for WeightedCombineScoreCell {
    fn id(&self) -> CellId { CellId::named("weighted-combine-score") }
    fn name(&self) -> &str { "weighted-combine-score" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Score] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        if input.len() != self.weights.len() {
            return Err(CellError::precondition(&format!(
                "expected {} inputs, got {}", self.weights.len(), input.len(),
            )));
        }

        let mut combined = 0.0;
        let mut components = Vec::new();
        for (signal, weight) in input.iter().zip(&self.weights) {
            let score: f64 = signal.extract_field("combined")?;
            combined += score * weight;
            components.push(PeerScoreComponent {
                layer: signal.extract_field("layer")?,
                score,
                weight: *weight,
            });
        }

        Ok(vec![Signal::new(Kind::PeerScore, CombinedPeerScore {
            combined,
            components,
        })])
    }
}
```

---

## 7. Collusion Detection as an Observe Cell (Lens)

Collusion detection is an **Observe Cell** -- a Lens in the telemetry system -- that watches the assignment and attestation graphs for anomalous patterns. It does not actively intervene; it emits Signals that feed into the DisciplineCell and the Route Cells.

```rust
/// Collusion detection Lens.
///
/// Watches job assignment and attestation graphs for patterns
/// consistent with collusion: mutual assignment rings, cliques
/// with high internal transaction ratios, suspiciously correlated
/// bid patterns.
///
/// This is an Observe Cell -- it reads state and emits observations.
/// It does not modify reputation or block agents directly. Its
/// output Signals feed into the DisciplineCell (for penalties)
/// and Route Cells (for downweighting).
pub struct CollusionDetectionLens {
    /// Minimum mutual assignment count to flag a pair.
    mutual_threshold: u64,  // default: 5
    /// Minimum clique size to investigate.
    min_clique_size: usize,  // default: 3
    /// Internal transaction ratio above which a clique is suspicious.
    internal_ratio_threshold: f64,  // default: 0.8
    /// Period for feedback weight reduction on flagged agents.
    penalty_duration_days: u64,  // default: 30
    /// Feedback weight multiplier for flagged agents.
    feedback_weight_reduction: f64,  // default: 0.5
}

impl Cell for CollusionDetectionLens {
    fn id(&self) -> CellId { CellId::named("collusion-detection") }
    fn name(&self) -> &str { "collusion-detection" }
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Observe] }
    fn estimated_cost(&self) -> Option<Cost> { Some(Cost::zero()) }

    async fn execute(
        &self,
        input: Vec<Signal>,
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        let assignment_graph: AssignmentGraph = ctx.store
            .query_assignment_graph(Duration::days(90))
            .await?;

        let mut alerts = Vec::new();

        // 1. Mutual assignment detection
        //    If agent A and agent B assign each other jobs > threshold times,
        //    flag the pair.
        let mutual_pairs = assignment_graph.find_mutual_pairs(
            self.mutual_threshold,
        );
        for (a, b, count) in &mutual_pairs {
            alerts.push(CollusionAlert {
                kind: CollusionKind::MutualAssignment,
                agents: vec![*a, *b],
                evidence: json!({ "mutual_count": count }),
                severity: if *count > self.mutual_threshold * 2 {
                    Severity::High
                } else {
                    Severity::Medium
                },
            });
        }

        // 2. Clique detection
        //    Find groups of agents with internal transaction ratio > threshold.
        //    Internal ratio = transactions within group / total transactions by group.
        let cliques = assignment_graph.find_cliques(
            self.min_clique_size,
            self.internal_ratio_threshold,
        );
        for clique in &cliques {
            alerts.push(CollusionAlert {
                kind: CollusionKind::ClosedClique,
                agents: clique.members.clone(),
                evidence: json!({
                    "internal_ratio": clique.internal_ratio,
                    "size": clique.members.len(),
                }),
                severity: Severity::High,
            });
        }

        // 3. Emit Signals for each alert
        let outputs: Vec<Signal> = alerts.iter()
            .map(|alert| Signal::new(Kind::CollusionAlert, alert))
            .collect();

        Ok(outputs)
    }
}

/// Observe protocol: this Lens watches assignment graph changes.
impl Observe for CollusionDetectionLens {
    fn observes(&self) -> &[ObservableEventKind] {
        &[
            ObservableEventKind::JobAssigned,
            ObservableEventKind::ReputationAttested,
        ]
    }

    fn scope(&self) -> LensScope {
        LensScope::Global  // watches all agents, not scoped to one
    }
}
```

### 7.1 Feedback Weight Reduction

When the CollusionDetectionLens flags agents, their attestations are downweighted for 30 days. This is implemented by the ReputationScoreCell checking for active collusion flags before applying the EMA update:

```rust
impl ReputationScoreCell {
    /// Check if an attestation source is flagged for collusion.
    /// If flagged, reduce the feedback weight by 50%.
    fn adjusted_feedback(
        &self,
        att: &Attestation,
        ctx: &CellContext,
    ) -> f64 {
        let flags = ctx.store.query_collusion_flags(att.source_identity);
        if flags.is_active() {
            att.feedback * flags.weight_reduction  // 0.5 = 50% weight
        } else {
            att.feedback
        }
    }
}
```

This is not a penalty on the flagged agent's own reputation. It is a reduction in the *influence* of their attestations on others. A colluding pair that mutually inflates each other's reputation sees their mutual attestations halved in effect.

---

## 8. The Reputation Loop Graph

All the pieces compose into a single Loop Graph that continuously processes attestations, applies decay, detects collusion, and emits updated reputation Signals.

```toml
[graph]
name = "reputation-loop"
loop = true
min_interval = "1m"

[[nodes]]
id       = "attestation-ingest"
cell     = "roko:attestation-ingest-cell"
protocol = "React"

[[nodes]]
id       = "collusion-check"
cell     = "roko:collusion-detection-lens"
protocol = "Observe"

[[nodes]]
id       = "reputation-update"
cell     = "roko:reputation-score-cell"
protocol = "Score"

[[nodes]]
id       = "decay-tick"
cell     = "roko:reputation-decay-cell"
protocol = "Score"

[[nodes]]
id       = "tier-evaluation"
cell     = "roko:tier-transition-cell"
protocol = "Score"

[[nodes]]
id       = "peer-scoring"
cell     = "roko:peer-scoring-pipeline"
protocol = "Score"

[[nodes]]
id       = "persist"
cell     = "roko:reputation-store-cell"
protocol = "Store"

[[edges]]
from = "attestation-ingest"
to   = "collusion-check"

[[edges]]
from = "collusion-check"
to   = "reputation-update"

[[edges]]
from = "reputation-update"
to   = "tier-evaluation"

[[edges]]
from = "tier-evaluation"
to   = "persist"

# Decay runs on a timer, parallel to attestation processing
[[edges]]
from = "decay-tick"
to   = "tier-evaluation"

# Peer scoring runs periodically, feeds into routing decisions
[[edges]]
from = "peer-scoring"
to   = "persist"

# Feedback: persisted reputation feeds back into routing
[[edges]]
from = "persist"
to   = "attestation-ingest"
condition = "always"
```

This Loop Graph runs continuously. Every attestation flows through collusion detection, EMA update, tier evaluation, and persistence. Decay ticks run on a timer (default: once per day). Peer scoring runs periodically (default: every 6 hours). All outputs are persisted as Signals with standard demurrage.

---

## 9. Reputation in the Cascade Router

Reputation feeds into model routing via the cascade router (see [bandit-routing-and-cascade.md](../10-learning-loops/bandit-routing-and-cascade.md)). The integration point: reputation Signals are one of the context features in the LinUCB bandit's 18-dimensional feature vector.

```rust
/// Reputation as routing context.
///
/// The cascade router's RoutingContext includes reputation features.
/// Higher-reputation agents are routed to more capable (and expensive)
/// models, because their tasks are more likely to be complex and
/// their outcomes more likely to produce useful learning signal.
pub fn reputation_to_routing_context(
    reputation: &CombinedPeerScore,
    domain_score: f64,
) -> RoutingFeatures {
    RoutingFeatures {
        // Aggregate reputation: combined peer score
        agent_reputation: reputation.combined,
        // Domain-specific: how good is this agent in the task's domain?
        domain_reputation: domain_score,
        // Tier as categorical feature (one-hot encoded in the bandit)
        tier: ReputationTier::from_score(reputation.combined),
    }
}
```

---

## What This Enables

1. **Reputation as a composable primitive.** Reputation Signals compose with any Cell that accepts Score output. A Route Cell can use reputation for hiring. A Verify Cell can use reputation as evidence. A Compose Cell can use reputation for context prioritization. No special-case integration.

2. **Unified learning mechanism.** Verify thresholds, model routing, and reputation all learn through the same predict-publish-correct loop. Debugging one teaches you all three. Tuning the EMA alpha is the same operation whether applied to Verify pass rates or reputation scores.

3. **Graceful cold start.** Adaptive alpha handles the cold-start problem. New agents have alpha = 0.30 (rapid convergence to a useful estimate from few observations). Established agents have alpha = 0.04 (stability against noise). No special-case "new agent" logic.

4. **Collusion resistance without centralization.** The CollusionDetectionLens is an Observe Cell -- it watches the same Signals that every other Cell watches. It does not require a privileged position or admin access. Any node running the Lens can detect collusion patterns. The penalty (feedback weight reduction) is proportional and time-limited, not a binary ban.

5. **Peer scoring for free.** The three-layer peer scoring Pipeline produces a combined score that is immediately usable by any downstream Cell. The gossip system uses it for peer selection. The job market uses it for bid weighting. The relay network uses it for routing priority. One Pipeline, many consumers.

---

## Feedback Loops

1. **Attest -> EMA -> Tier -> Access -> Work -> Attest.** Positive attestations raise the EMA. Higher EMA promotes the tier. Higher tier unlocks higher-value jobs. Higher-value jobs produce more attestations. Virtuous cycle, bounded by the adaptive alpha ceiling (established agents change slowly even with many positive attestations).

2. **Decay -> Lower Score -> Lower Tier -> Fewer Jobs -> Less Attestation -> More Decay.** An idle agent decays toward neutral. Lower score means lower tier. Lower tier means fewer eligible jobs. Fewer jobs mean fewer attestations to counter the decay. This vicious cycle is intentional: it prevents stale high-reputation agents from squatting on scarce job slots. The escape is straightforward: do work.

3. **Collusion -> Flag -> Weight Reduction -> Reduced Influence -> Collusion Less Profitable.** Detected collusion reduces the colluding agents' attestation influence by 50% for 30 days. Their mutual reputation inflation becomes half as effective. If collusion is the only way they maintain their tier, they will decay. If they are also doing legitimate work, the legitimate attestations (from non-colluding sources) maintain their score while the colluding channel is dampened.

4. **Peer Score -> Routing Priority -> Better Connections -> Better Peer Score.** Agents with high peer scores get preferential treatment in gossip routing and relay selection. Better connectivity means faster message delivery, which improves the Protocol Score layer, which raises the overall peer score. Checked by the IP colocation penalty: connecting from the same infrastructure as many other agents penalizes the Protocol Score regardless of delivery quality.

5. **Reputation -> Model Routing -> Outcome Quality -> Reputation.** Higher-reputation agents get routed to more capable models via the cascade router. More capable models produce better outcomes. Better outcomes generate positive attestations. Positive attestations raise reputation. This is the intended Matthew effect: agents that earn trust get better tools and produce better work. Checked by demurrage: the advantage decays without continued performance.

---

## Open Questions

1. **Cross-domain reputation transfer.** Should a high reputation in "coding" provide any boost in "security"? The v1 spec treats domains as independent. But in practice, a strong coder is more likely to be a competent security reviewer than a random agent. A cross-domain transfer function (e.g., 20% of coding reputation counts toward security) would model this, but it also creates attack vectors: an agent could inflate an easy domain to gain a foothold in a hard one.

2. **Reputation portability.** If an agent migrates from one Roko instance to another, should their reputation transfer? The ERC-8004 identity is on-chain and portable. But reputation is instance-specific (different jobs, different peers). A "reputation passport" using ZK-HDC proofs (22-REGISTRIES SS4.6) could prove reputation without revealing the attestation history, but the receiving instance has no reason to trust the proving instance's standards.

3. **Alpha floor.** The current schedule bottoms out at alpha = 0.04. Should there be a mechanism to temporarily raise alpha for established agents facing a regime change (e.g., a model upgrade that changes what "good performance" looks like)? A "recalibration event" that resets the attestation count for a domain would do this, but it requires governance to trigger.

4. **Decay toward domain mean vs. neutral.** The current decay target is 0.5 (neutral). An alternative: decay toward the domain mean. This would make decay less punishing in domains where the average agent scores 0.7 (decaying to 0.7 instead of 0.5). But it also makes decay weaker for agents below the mean (they would decay toward the mean instead of toward neutral, effectively getting a free reputation boost from inactivity).

5. **Collusion detection sensitivity.** The mutual assignment threshold (5) and internal ratio threshold (0.8) are static. Should these be adaptive, based on the overall network structure? In a small network with 20 agents, any pair will interact frequently. In a large network with 10,000 agents, mutual assignment of 5 is highly suspicious. The thresholds should probably scale with network size, but the scaling law is not obvious.
