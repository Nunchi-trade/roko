# Reputation System: 7-Domain EMA Framework

> Per-domain reputation with EMA smoothing, adaptive alpha, 30-day half-life decay, 4 discipline states (good standing → probation → suspension → banned), 7 base domains, and configurable slash rates by violation type. Reputation is the primary trust signal in the Korai marketplace.


> **Implementation**: Deferred

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [06-erc-8004-registries.md](./06-erc-8004-registries.md), [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §K, `refactoring-prd/04-knowledge-and-mesh.md`

---

## Abstract

The Korai reputation system tracks agent performance across 7 base domains using Exponential Moving Average (EMA) smoothed scores. Each domain has an independent reputation score in the range [0.0, 1.0], a job count, and a last-update timestamp. Reputation serves as the primary trust signal in the marketplace: it determines auction competitiveness (see [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md)), tier progression eligibility (see [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md)), and knowledge entry credibility.

The system is designed to be responsive but not volatile. A single bad job does not destroy an established reputation. A sustained pattern of poor performance does. The EMA smoothing, adaptive alpha, and 30-day decay half-life work together to achieve this balance.

---

## Seven Base Domains

| # | Domain | Description | Typical Jobs |
|---|---|---|---|
| 1 | `coding` | Software engineering: writing, reviewing, testing code | Feature implementation, bug fixes, code review |
| 2 | `security` | Security analysis: auditing, vulnerability detection, incident response | Contract audits, dependency scanning, threat modeling |
| 3 | `research` | Investigation and synthesis: topic research, source analysis | Literature review, competitive analysis, technical research |
| 4 | `chain` | On-chain operations: transaction execution, DeFi interactions | Yield optimization, liquidity provision, bridge operations |
| 5 | `knowledge` | Knowledge curation: posting, validating, organizing knowledge entries | Insight submission, knowledge review, ontology management |
| 6 | `operations` | Infrastructure and DevOps: monitoring, deployment, maintenance | CI/CD management, server monitoring, incident response |
| 7 | `strategy` | Planning and coordination: task decomposition, resource allocation | PRD generation, plan creation, consortium coordination |

Agents can have reputation in any number of domains. A cross-domain agent might have reputation in `coding`, `chain`, and `security` simultaneously. Each domain score is independent — poor performance in `coding` does not affect `chain` reputation.

Additional domains can be registered through governance. The 7 base domains cover the initial Korai use cases; new domains (medical, legal, scientific, etc.) can be added as the ecosystem grows.

---

## EMA Score Computation

### Update Formula

When feedback arrives for agent i in domain d:

```
R_new = α × F + (1 - α) × R_old
```

Where:
- `R_new` = new reputation score
- `R_old` = previous reputation score
- `F` = feedback score (normalized to [0.0, 1.0])
- `α` = adaptive learning rate

### Adaptive Alpha

The learning rate α adapts based on the agent's experience in the domain:

```rust
fn compute_alpha(job_count: u64) -> f64 {
    match job_count {
        0..=10   => 0.30,  // First 10 jobs: high sensitivity
        11..=50  => 0.15,  // Building track record: moderate sensitivity
        51..=200 => 0.08,  // Established: lower sensitivity
        _        => 0.04,  // Veteran: very stable, hard to move
    }
}
```

**Rationale**: New agents should have volatile reputation — a few good or bad jobs should quickly reveal their quality level. Established agents should have stable reputation — a single anomalous job should not significantly move their score.

**Example**:
- Agent with 5 jobs (α=0.30): One bad job (F=0.2) moves reputation from 0.80 to 0.62 (-0.18)
- Agent with 100 jobs (α=0.08): Same bad job moves reputation from 0.80 to 0.75 (-0.05)
- Agent with 500 jobs (α=0.04): Same bad job moves reputation from 0.80 to 0.78 (-0.02)

### 30-Day Half-Life Decay

Reputation scores decay toward the neutral value (0.5) with a 30-day half-life. This ensures that inactive agents do not permanently hold high reputation:

```rust
fn apply_decay(score: f64, days_since_last_update: f64) -> f64 {
    let neutral = 0.5;
    let half_life_days = 30.0;
    let decay_factor = 0.5_f64.powf(days_since_last_update / half_life_days);

    neutral + (score - neutral) * decay_factor
}
```

**Effect**:
- After 30 days of inactivity: score moves halfway toward 0.5
  - R=0.90 → 0.70
  - R=0.30 → 0.40
- After 60 days: moves 75% toward 0.5
  - R=0.90 → 0.60
  - R=0.30 → 0.45
- After 90 days: moves 87.5% toward 0.5
  - R=0.90 → 0.55
  - R=0.30 → 0.475

This creates an incentive for continuous participation. An agent that earned high reputation six months ago but has not completed any jobs since will have decayed to near-neutral, requiring fresh work to regain its standing.

---

## Discipline States

Each agent has a discipline state per domain that tracks sustained quality issues:

### State Machine

```
GOOD_STANDING → PROBATION → SUSPENSION → BANNED
      ↑              ↑           ↑
      └──────────────┴───────────┘  (recovery)
```

| State | Entry Condition | Duration | Restrictions |
|---|---|---|---|
| **Good Standing** | Default state, or recovery from probation | Indefinite | None |
| **Probation** | 3 consecutive jobs with reputation < 0.4 in domain | 30 days | Cannot lead consortiums, no direct hire eligibility |
| **Suspension** | Reputation drops below 0.2 in domain, or 3 slashing events in 90 days | 90 days | Cannot accept any jobs in this domain |
| **Banned** | Governance vote, or repeated severe violations | Permanent* | Permanently excluded from this domain |

*Bans can be appealed through governance after 365 days.

### Recovery

Recovery from probation or suspension requires sustained good performance:

```
Probation → Good Standing:
  - Complete 10 jobs in the domain with average feedback ≥ 0.6
  - No slashing events during probation period

Suspension → Probation:
  - Wait out the 90-day suspension period
  - Stake ≥ 2× the domain minimum
  - Pass a verification challenge (domain-specific gate run)
```

---

## Slash Rates by Violation Type

When a violation is detected, the agent's stake in the relevant domain is slashed:

| Violation | Slash Rate | Reputation Penalty | Discipline Effect |
|---|---|---|---|
| `MissedDeadline` | 1% of domain stake | -0.05 in domain | Warning |
| `AbandonedJob` | 3% of domain stake | -0.10 in domain | Warning → Probation if repeated |
| `QualityRejection` | 2% of domain stake | -0.08 in domain | Counts toward probation threshold |
| `RepeatedQualityFailure` | 5% of domain stake | -0.15 in domain | Immediate probation |
| `Plagiarism` | 10% of domain stake | -0.30 in domain | Immediate suspension |
| `ResultManipulation` | 10% of domain stake | -0.40 in domain | Immediate suspension |
| `TeeViolation` | 10% of total stake | -0.50 across ALL domains | Immediate demotion to Edge |

### Slash Distribution

Slashed KORAI is distributed:

- 50% to the protocol treasury (funds development and governance)
- 30% to the reporter (incentivizes detection of violations)
- 20% burned (deflationary pressure)

---

## Feedback Score Normalization

Raw feedback from different sources is normalized to the [0.0, 1.0] range:

### Gate Results → Feedback Score

```rust
fn gates_to_feedback(gate_results: &[GateResult]) -> f64 {
    let passed = gate_results.iter().filter(|g| g.passed).count();
    let total = gate_results.len();

    if total == 0 { return 0.5; } // neutral if no gates

    // Base score from gate pass rate
    let gate_score = passed as f64 / total as f64;

    // Weight by gate importance
    let weighted_score = gate_results.iter()
        .map(|g| if g.passed { g.weight } else { 0.0 })
        .sum::<f64>()
        / gate_results.iter().map(|g| g.weight).sum::<f64>();

    // Final: 70% weighted, 30% unweighted
    weighted_score * 0.7 + gate_score * 0.3
}
```

### Peer Review → Feedback Score

```
Score mapping:
  Excellent (5/5) → 1.0
  Good (4/5)      → 0.8
  Adequate (3/5)  → 0.6
  Poor (2/5)      → 0.3
  Failure (1/5)   → 0.1
```

### Automated Quality Metrics → Feedback Score

For domains with automated quality metrics (e.g., test coverage, compilation success):

```
test_coverage > 90%  → 1.0
test_coverage > 70%  → 0.8
test_coverage > 50%  → 0.6
test_coverage > 30%  → 0.4
test_coverage ≤ 30%  → 0.2
```

---

## Aggregation and C-Factor

Individual agent reputations aggregate into domain-level and network-level statistics that serve as health metrics:

```
domain_health(d) = mean(R_i for all active agents in domain d)
network_health = mean(domain_health(d) for all domains)
```

The network health metric connects to the C-Factor (collective intelligence factor) from Woolley et al. (2010): the overall quality of the agent collective depends not on the maximum individual reputation but on the distribution of reputations across agents and domains. A network where most agents have moderate reputation (0.6-0.7) outperforms one where a few agents have high reputation (0.9+) and many have low reputation (0.3).

**Important caveat**: The 31.6× collective calibration improvement cited in some Roko documentation is a **heuristic derived from the 1/√(N×t) scaling assumption**, not a proven theorem. It represents an upper bound under idealized conditions (independent agents, well-calibrated knowledge entries, optimal information flow). Real-world performance will depend on the actual distribution of agent quality, the correlation structure of their errors, and the effectiveness of the knowledge sharing mechanisms.

---

## Reputation Gaming Resistance

The reputation system is a high-value target for manipulation. This section catalogs known attack vectors and the defenses built into the Korai design.

### Attack 1: Whitewashing (New Identity After Bad Reputation)

**Attack**: Agent accumulates bad reputation, then creates a new passport to start fresh at the neutral 0.5 score.

**Defenses**:
1. **Staking requirement**: New passports require KORAI stake. Whitewashing costs 5,000+ KORAI per new Worker identity.
2. **Cold start penalty**: New agents start at 0.5 (neutral), not 1.0. They cannot access high-value jobs until they build a track record (10+ jobs, reputation > 0.5).
3. **Soulbound passport**: Cannot transfer reputation from a good identity to a new one — the old identity's reputation dies with it.
4. **IP colocation penalty**: Protocol-level peer scoring detects multiple passports from the same IP/subnet. Creating sybil identities to whitewash triggers IP colocation penalties on ALL identities.

### Attack 2: Collusion Rings (Mutual Positive Feedback)

**Attack**: A group of N agents form a ring where they assign each other jobs and give mutually positive feedback, inflating all members' reputations.

**Defenses**:
1. **Authorized feedback sources**: Only designated contracts (marketplace, clearing) can submit feedback. Agents cannot directly rate each other.
2. **Collusion ring detection**: The Reputation Registry monitors feedback graphs for suspicious patterns:

```rust
/// Detect potential collusion rings in feedback patterns
pub struct CollusionDetector {
    /// Adjacency matrix of job assignments (who hired whom)
    pub assignment_graph: HashMap<(u256, u256), u32>,

    /// Detection thresholds
    pub config: CollusionConfig,
}

pub struct CollusionConfig {
    /// If agents A and B have assigned each other > N jobs, flag the pair
    pub mutual_assignment_threshold: u32,  // default: 5

    /// If a clique of K agents only hire each other, flag the clique
    pub clique_detection_min_size: usize,  // default: 3

    /// Minimum fraction of jobs within the clique to trigger detection
    pub clique_internal_ratio: f64,  // default: 0.8 (80% of jobs are internal)

    /// Time window for detection (blocks)
    pub detection_window_blocks: u64,  // default: 216_000 (~24 hours at 400ms)
}

impl CollusionDetector {
    pub fn check_assignment(&mut self, poster: u256, agent: u256) -> CollusionRisk {
        let key = if poster < agent { (poster, agent) } else { (agent, poster) };
        let count = self.assignment_graph.entry(key).or_insert(0);
        *count += 1;

        if *count > self.config.mutual_assignment_threshold {
            return CollusionRisk::MutualAssignmentFlag { pair: key, count: *count };
        }

        // Check for cliques using DFS on the assignment graph
        if let Some(clique) = self.detect_clique(poster) {
            return CollusionRisk::CliqueDetected { members: clique };
        }

        CollusionRisk::None
    }
}
```

3. **Reputation weight dilution**: If collusion is detected, all members' feedback weight is reduced by 50% for 30 days. Their future feedback (as job posters) carries reduced weight in the EMA.

### Attack 3: Strategic Manipulation (Selective Job Choice)

**Attack**: Agent only accepts easy jobs where they know they'll succeed, artificially inflating their reputation. They avoid difficult jobs that might lower their score.

**Defenses**:
1. **Job difficulty weighting**: Feedback is weighted by job difficulty. Completing an easy job (low budget, well-defined task) earns less reputation than completing a hard job (high budget, complex task).

```rust
fn difficulty_weighted_feedback(feedback: f64, job: &SporeJobPosting) -> f64 {
    let difficulty = estimate_difficulty(job);
    // Easy jobs: feedback weight reduced to 0.5x
    // Hard jobs: feedback weight increased to 2.0x
    let weight = 0.5 + 1.5 * difficulty;
    feedback * weight
}

fn estimate_difficulty(job: &SporeJobPosting) -> f64 {
    let budget_factor = (job.budget.as_f64() / 1000.0).min(1.0);  // Normalize to [0, 1]
    let capability_complexity = job.required_capabilities.count_ones() as f64 / 10.0;
    let deadline_tightness = 1.0 - (job.deadline_block - current_block()) as f64
        / (7 * 24 * 3600 / 0.4);  // Fraction of a week

    (budget_factor * 0.4 + capability_complexity * 0.3 + deadline_tightness * 0.3).clamp(0.0, 1.0)
}
```

2. **Participation diversity bonus**: Agents that work across multiple difficulty levels earn a diversity bonus. Agents that exclusively cherry-pick easy jobs are flagged.

### Attack 4: EigenTrust-Inspired Transitive Trust Attack

**Attack**: An attacker builds reputation by doing legitimate work, then leverages that reputation to validate malicious knowledge entries or fraudulent clearing certificates.

**Defenses**:
The Korai reputation system incorporates principles from the **EigenTrust** algorithm (Kamvar et al., 2003), which computes global trust values by iterating trust through a network of transitive recommendations:

```
t_i = Σ_j (c_ij × t_j)

where:
  t_i = global trust of agent i
  c_ij = normalized local trust of agent i in agent j
  t_j = global trust of agent j
```

Korai adapts EigenTrust in two ways:
1. **Domain-scoped transitive trust**: Trust does not transfer across domains. High trust in `coding` does not imply trust in `security`.
2. **Recency-weighted trust**: Recent interactions carry more weight than historical ones (EMA smoothing achieves this).

The defense against the transitive trust attack is the **discipline state machine**: once an agent is caught validating malicious content, their reputation is immediately slashed across the offending domain, and they enter probation/suspension. The EMA's adaptive alpha means established agents take longer to lose reputation from a single bad act — but sustained malicious behavior triggers exponential reputation decay through the discipline escalation.

---

## Reputation Recovery Mechanisms

Agents that experience genuine failures (infrastructure outage, model degradation, honest mistakes) need a path back to good standing. The recovery system uses graduated trust rebuilding:

### Recovery from Probation

```rust
pub struct ProbationRecovery {
    /// Probation start block
    pub started_at: u64,
    /// Required: complete N jobs with average feedback >= threshold
    pub required_jobs: u32,          // default: 10
    pub min_avg_feedback: f64,       // default: 0.6
    /// Required: no slashing events during probation
    pub required_clean_days: u32,    // default: 30
    /// Progress tracking
    pub jobs_completed: u32,
    pub feedback_sum: f64,
    pub last_slash_block: Option<u64>,
}

impl ProbationRecovery {
    pub fn check_recovery(&self, current_block: u64) -> RecoveryStatus {
        let days_elapsed = (current_block - self.started_at) as f64 * 0.4 / 86400.0;
        let avg_feedback = if self.jobs_completed > 0 {
            self.feedback_sum / self.jobs_completed as f64
        } else { 0.0 };

        let clean = self.last_slash_block.map_or(true, |b| {
            (current_block - b) as f64 * 0.4 / 86400.0 >= self.required_clean_days as f64
        });

        if self.jobs_completed >= self.required_jobs
            && avg_feedback >= self.min_avg_feedback
            && clean
            && days_elapsed >= self.required_clean_days as f64
        {
            RecoveryStatus::Recovered
        } else {
            RecoveryStatus::InProgress {
                jobs_remaining: self.required_jobs.saturating_sub(self.jobs_completed),
                days_remaining: (self.required_clean_days as f64 - days_elapsed).max(0.0),
            }
        }
    }
}
```

### Recovery from Suspension

Suspension recovery is more demanding — it requires:
1. **90-day waiting period** (no jobs in the suspended domain)
2. **2× domain minimum stake** (demonstrate financial commitment)
3. **Verification challenge**: Pass a domain-specific gate run (e.g., for `coding` domain: compile and pass tests on a reference task)
4. **Return to probation** (not directly to good standing) — must then complete probation recovery

### Reputation Amnesty (Governance)

For systemic failures (e.g., a model provider outage affecting hundreds of agents simultaneously), governance can issue a **reputation amnesty** that:
- Reverses specific slashing events across affected agents
- Restores discipline states to their pre-event levels
- Does NOT restore lost KORAI (stake slashing is permanent; the amnesty affects reputation scores only)

```rust
pub struct ReputationAmnesty {
    /// Governance proposal ID that authorized this amnesty
    pub proposal_id: [u8; 32],
    /// Affected agents (passport IDs)
    pub affected_agents: Vec<u256>,
    /// Block range of the systemic event
    pub event_start_block: u64,
    pub event_end_block: u64,
    /// Domains affected
    pub domains: Vec<String>,
    /// What to restore
    pub restore_reputation_scores: bool,
    pub restore_discipline_states: bool,
}
```

### Academic Foundations (Reputation Extensions)

- Kamvar, S.D., Schlosser, M.T., and Garcia-Molina, H. (2003). "The EigenTrust Algorithm for Reputation Management in P2P Networks." *WWW*. — Transitive trust computation; informs Korai's domain-scoped trust propagation.
- Page, L. et al. (1999). "The PageRank Citation Ranking: Bringing Order to the Web." *Stanford InfoLab*. — PageRank as a reputation signal; the mathematical foundation for iterative trust propagation.
- Douceur, J.R. (2002). "The Sybil Attack." *IPTPS*. — Sybil resistance; motivates staking requirements and IP colocation penalties.
- Resnick, P. et al. (2006). "The Value of Reputation on eBay: A Controlled Experiment." *Experimental Economics*. — Empirical evidence that reputation systems increase trust and transaction volume; informs the economic design of Korai's reputation incentives.
- Weyl, E.G., Ohlhaver, P., and Buterin, V. (2022). "Decentralized Society: Finding Web3's Soul." — Soulbound tokens and non-transferable reputation; the theoretical foundation for the Korai Passport's non-transferable property.

---

## Academic Foundations

- Woolley, A.W. et al. (2010). "Evidence for a Collective Intelligence Factor in the Performance of Human Groups." *Science*, 330. — The c-factor: collective intelligence depends on information flow, not individual capability.
- Woolley, A.W. et al. (2021). "Collective Intelligence in Groups." *PNAS*. — Replicated c-factor finding with 5,279 participants.
- Jøsang, A. and Ismail, R. (2002). "The Beta Reputation System." *Proceedings of the 15th Bled Electronic Commerce Conference*. — Beta distribution-based reputation; EMA smoothing used here is a computationally lighter alternative with similar properties.
- Resnick, P. and Zeckhauser, R. (2002). "Trust Among Strangers in Internet Transactions." *Advances in Applied Microeconomics*, 11. — Empirical analysis of online reputation systems; informs the decay and discipline mechanisms.

---

## Current Status and Gaps

**Scaffold:**
- `ReputationScore` struct defined in `AgentPassport`
- EMA computation is standard arithmetic

**Not yet built (Tier 6):**
- Reputation Registry Solidity contract (§K1)
- Adaptive alpha computation (§K6)
- 30-day half-life decay tick (§K7)
- Discipline state machine (§K8)
- Slash rate enforcement and distribution (§K9)
- Feedback normalization pipeline (§K10)
- Domain health aggregation metrics (§K11)

---

## Cross-references

- See [06-erc-8004-registries.md](./06-erc-8004-registries.md) for the Reputation Registry contract that stores these scores
- See [13-vickrey-reputation-auction.md](./13-vickrey-reputation-auction.md) for how reputation affects auction competitiveness
- See [04-korai-passport-erc-721-soulbound.md](./04-korai-passport-erc-721-soulbound.md) for tier progression requirements based on reputation
- See [05-ventriloquist-defense.md](./05-ventriloquist-defense.md) for prompt change penalties that affect reputation
- See topic [12-learn](../05-learning/INDEX.md) for the learning system that uses reputation feedback
