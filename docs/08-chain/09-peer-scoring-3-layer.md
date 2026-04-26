# Peer Scoring: 3-Layer Model

> Three layers of peer scoring protect the gossip network: protocol-level (GossipSub v1.1 mesh scoring), application-level (domain-specific behavior scoring), and economic-level (stake-weighted trust). Combined score determines mesh membership, message priority, and job eligibility.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [01-nunchi-chain-spec.md](./01-nunchi-chain-spec.md), [06-erc-8004-registries.md](./06-erc-8004-registries.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §B, `refactoring-prd/04-knowledge-and-mesh.md`

---

## Abstract

The Nunchi gossip network faces a fundamental challenge: agents are economically motivated participants that may behave adversarially. An agent might flood the network with messages to crowd out competitors, selectively withhold messages to disadvantage peers, or broadcast false anomaly alerts to trigger unnecessary responses. Without peer scoring, the gossip network is vulnerable to these attacks.

Nunchi uses a 3-layer peer scoring model that evaluates agent behavior at three levels: protocol, application, and economic. Each layer captures different aspects of trustworthiness, and the combined score determines an agent's standing in the gossip mesh.

---

## Layer 1: Protocol Scoring (GossipSub v1.1)

GossipSub v1.1 includes a built-in peer scoring function (Vyzovitis et al., 2020) that evaluates each peer based on their protocol-level behavior. This is the first line of defense against network-layer attacks.

### Scoring Parameters

```rust
pub struct ProtocolScoreParams {
    /// Weight for topic-specific scoring.
    /// Each topic has its own sub-scores.
    pub topic_score_cap: f64,      // default: 3600.0

    /// Weight for IP co-location penalty.
    /// Penalizes multiple peers from the same IP (sybil indicator).
    pub ip_colocation_factor_weight: f64,   // default: -35.11
    pub ip_colocation_factor_threshold: usize, // default: 10

    /// Decay interval for score components.
    pub decay_interval_secs: u64,  // default: slot_duration (50ms * 32 = 1.6s)

    /// Below this score, peer is graylisted (messages deprioritized).
    pub graylist_threshold: f64,   // default: -16000.0

    /// Below this score, peer is removed from mesh entirely.
    pub publish_threshold: f64,    // default: -8000.0

    /// Below this score, peer is disconnected.
    pub gossip_threshold: f64,     // default: -4000.0
}
```

### What Protocol Scoring Measures

| Behavior | Effect on Score | Rationale |
|---|---|---|
| **Consistent message delivery** | Positive | Reliable peers maintain mesh health |
| **Message validation success** | Positive | Valid messages indicate honest behavior |
| **First message delivery** | Bonus | Peers who deliver messages first are valuable relay nodes |
| **Invalid messages** | Negative | Invalid signatures, malformed payloads, or schema violations |
| **Message flooding** | Negative | Sending far more messages than expected rate per topic |
| **IP co-location** | Negative | Multiple peers from same IP suggest sybil operation |
| **Mesh participation** | Positive | Maintaining connections and relaying for others |

### Score Decay

Protocol scores decay toward zero over time. This serves two purposes:

1. **Forgiveness**: A peer that misbehaved briefly (e.g., during a network partition) recovers naturally as the negative score decays.
2. **Continuous evaluation**: A peer cannot rest on past good behavior. It must continue participating honestly to maintain a positive score.

Decay rate: configurable per topic, typically one decay interval per slot (12.8 seconds on Nunchi).

---

## Layer 2: Application Scoring

Application scoring evaluates domain-specific behavior that the protocol layer cannot assess. This layer knows about agent capabilities, domain performance, and marketplace behavior.

### Application Score Components

```rust
pub struct ApplicationScore {
    /// Knowledge contribution quality.
    /// Positive for entries that are confirmed by others.
    /// Negative for entries that are challenged or expire quickly.
    pub knowledge_quality: f64,

    /// Anomaly detection accuracy.
    /// Positive for anomalies that are independently confirmed.
    /// Negative for false alerts.
    pub anomaly_accuracy: f64,

    /// Job completion reliability.
    /// Positive for completing jobs on time and passing gates.
    /// Negative for abandoning jobs or failing gates.
    pub job_reliability: f64,

    /// Simulation sharing utility.
    /// Positive for sharing simulation results that others find useful.
    /// Measured by how many agents acted on the shared simulation.
    pub simulation_utility: f64,

    /// Governance participation quality.
    /// Positive for voting consistently and on substantive proposals.
    /// Negative for abstaining from critical governance votes (for Tier 0-1 agents).
    pub governance_participation: f64,
}
```

### Knowledge Quality Scoring

When an agent posts a knowledge entry on the `nunchi/knowledge/v1` topic:

```
Initial score contribution: 0 (neutral)

If the entry receives 5+ confirmations within 30 days:
  score += 0.1 per confirmation (capped at +1.0)

If the entry is challenged and removed:
  score -= 0.5

If the entry expires through demurrage (no confirmations):
  score -= 0.1 (mild penalty — the entry wasn't harmful, just not useful)
```

Over time, agents that consistently post useful knowledge entries accumulate positive application scores. Agents that post low-quality or harmful entries accumulate negative scores.

### Anomaly Detection Accuracy

When an agent broadcasts on the `nunchi/anomaly/v1` topic:

```
If the anomaly is confirmed by 3+ independent agents:
  score += 0.3 (correctly identified a real anomaly)

If the anomaly is not confirmed by anyone within 1 epoch:
  score -= 0.1 (possible false alert)

If the anomaly is explicitly debunked:
  score -= 0.5 (definitely false alert)
```

This creates an incentive to broadcast only high-confidence anomaly alerts. The -0.1 penalty for unconfirmed alerts is mild (avoiding over-penalizing agents who detect genuine but subtle anomalies that others miss), but the -0.5 for debunked alerts is significant.

### Job Reliability Scoring

Tracks marketplace behavior:

```
Job completed on time, all gates passed:
  score += 0.2

Job completed late but gates passed:
  score += 0.05

Job abandoned:
  score -= 1.0

Job completed but failed gates:
  score -= 0.3
```

---

## Layer 3: Economic Scoring

The economic layer weights peers by their on-chain stake. An agent with 25,000 NUNCHI staked (Sovereign tier) has more to lose from misbehavior than an agent with no stake (Edge tier). This asymmetry is reflected in peer scoring.

### Stake-Weighted Trust

```rust
pub fn economic_score(agent: &AgentIdentity) -> f64 {
    let total_stake: f64 = agent.domain_stakes.values()
        .map(|s| s.as_f64())
        .sum();

    let tier_multiplier = match agent.tier {
        AgentTier::Protocol  => 4.0,
        AgentTier::Sovereign => 3.0,
        AgentTier::Worker    => 2.0,
        AgentTier::Edge      => 1.0,
    };

    let stake_score = (total_stake / 10_000.0).min(5.0); // cap at 50K NUNCHI
    let slash_penalty = agent.slash_history.len() as f64 * -0.5;

    (stake_score * tier_multiplier + slash_penalty).max(-10.0)
}
```

**Key properties:**

- **Stake provides collateral**: High-stake agents have more to lose from slashing, so their messages are more trustworthy a priori.
- **Tier amplifies stake**: A Sovereign agent's stake counts 3× in the trust calculation. This reflects the additional verification requirements for higher tiers.
- **Slash history penalizes**: Each past slashing event permanently reduces economic score. An agent that has been slashed twice is less trusted than a clean-record agent, even with the same stake.
- **Capped at 5.0**: Stake beyond 50,000 NUNCHI provides no additional trust benefit, preventing plutocratic capture where wealthy agents dominate the mesh.

---

## Combined Score

The three layers combine into a single composite score:

```rust
pub fn combined_peer_score(
    protocol: f64,
    application: &ApplicationScore,
    economic: f64,
) -> f64 {
    let app_total = application.knowledge_quality * 0.3
        + application.anomaly_accuracy * 0.2
        + application.job_reliability * 0.3
        + application.simulation_utility * 0.1
        + application.governance_participation * 0.1;

    // Weights: protocol 40%, application 35%, economic 25%
    protocol * 0.40 + app_total * 0.35 + economic * 0.25
}
```

### Score Thresholds and Consequences

| Combined Score | Status | Consequences |
|---|---|---|
| > 0 | **Good standing** | Full mesh membership, normal message priority |
| -4,000 to 0 | **Degraded** | Messages deprioritized, not selected for relay |
| -8,000 to -4,000 | **Graylisted** | Excluded from publishing on high-value topics (job, reputation) |
| -16,000 to -8,000 | **Mesh excluded** | Removed from gossip mesh; can still read canonical (T3) state |
| < -16,000 | **Disconnected** | Peer connection severed; must re-register to rejoin |

### Recovery Path

An agent with a negative score can recover by:

1. **Score decay**: All negative scores decay toward zero over time (protocol layer)
2. **Positive behavior**: Posting confirmed knowledge entries, completing jobs reliably, accurate anomaly detection
3. **Staking**: Increasing on-chain stake improves economic score immediately
4. **Clean history**: Each day without a negative event improves the running average

A mildly degraded agent (-2,000) can recover within a few days of positive behavior. A severely penalized agent (-15,000) may take weeks to recover, during which time it is effectively excluded from network participation.

---

## Sybil Resistance

Peer scoring is a key sybil resistance mechanism. The three layers work together:

- **Protocol layer**: IP co-location penalty detects sybil clusters running on the same machine or subnet
- **Application layer**: Sybil agents that confirm each other's entries are detected by cross-referencing confirmation patterns (a confirmation ring where the same N agents always confirm each other's entries is flagged)
- **Economic layer**: Each sybil identity requires stake. Creating 100 sybil agents at Worker tier (5,000 NUNCHI each) costs 500,000 NUNCHI — a significant economic barrier

The combination makes sybil attacks expensive (economic), detectable (application), and penalizable (protocol).

---

## Academic Foundations

- Vyzovitis, D. et al. (2020). "GossipSub: Attack-Resilient Message Propagation in the Filecoin and ETH2.0 Networks." — The GossipSub v1.1 peer scoring function that forms Layer 1.
- Douceur, J.R. (2002). "The Sybil Attack." *IPTPS*. — Original formalization of sybil attacks in peer-to-peer systems; motivates the economic layer of peer scoring.
- Woolley, A.W. et al. (2010). "Evidence for a Collective Intelligence Factor in the Performance of Human Groups." *Science*. — C-factor research: conversational turn-taking equality predicts collective intelligence. Peer scoring enforces analogous equality by penalizing agents that dominate message traffic.

---

## Current Status and Gaps

**Scaffold:**
- GossipSub v1.1 peer scoring available via `libp2p-gossipsub` crate
- On-chain stake data available from Identity Registry

**Not yet built (Tier 6):**
- Application scoring implementation (§B6)
- Economic scoring integration with on-chain ERC-8004 identity data (§B7)
- Combined score computation and threshold enforcement (§B8)
- Confirmation ring detection for sybil resistance (§B9)
- Score decay and recovery mechanics (§B10)

---

## Cross-References

- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for stake and tier data used in economic scoring
- See [14-reputation-system-7-domain.md](./14-reputation-system-7-domain.md) for the relationship between peer scoring and on-chain reputation
