# Sparrow: Power-of-Two-Choices Dispatch

> Sparrow is the fast-path dispatch protocol for urgent jobs. It uses the power-of-two-choices algorithm: probe 2 random eligible agents, assign the job to the one with lower load. Achieves O(log log N) maximum load with O(1) communication cost per assignment. Used for RandomVRF hiring model.


> **Implementation**: Built

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [10-spore-job-market.md](./10-spore-job-market.md)
**Key sources**: `roko/tmp/implementation-plans/12b-chain-layer.md` §C, `bardo-backup/tmp/agent-chain-new/12-agent-economy.md`

---

## Abstract

Sparrow is the dispatch protocol that handles fast-path job assignments when full auction-based hiring is unnecessary or too slow. It implements the "power of two choices" algorithm (Mitzenmacher, 2001; Ousterhout et al., 2013), which achieves near-optimal load balancing with minimal communication overhead.

The core insight: instead of probing all N eligible agents to find the least loaded (which costs O(N) communication), probe just 2 random agents and pick the less loaded one. This exponentially reduces maximum load from O(log N / log log N) to O(log log N). The improvement comes from breaking the symmetry that causes random assignment to cluster — even minimal information (comparing two instead of one) is enough to avoid the worst-case pileups.

Sparrow is used exclusively for the `RandomVRF` hiring model. Auction-based hiring uses the ERC-8183 job market directly. Direct hire bypasses both protocols.

---

## The Algorithm

### Why Not Pure Random?

If jobs are assigned to uniformly random eligible agents, the maximum load on any agent follows a Θ(log N / log log N) distribution. For a network of 1,000 agents processing 10,000 jobs, the most-loaded agent handles approximately 7× the average load. This is the "balls into bins" result from classical probability theory.

The power of two choices reduces this to O(log log N). For 1,000 agents, the most-loaded agent handles approximately 2× the average load — a dramatic improvement for a trivial change in protocol.

### Protocol

```
1. Job arrives for RandomVRF assignment

2. Sparrow selects 2 agents from the eligible pool:
   a. Filter by capability bitmask (same as ERC-8183 job market eligibility)
   b. Filter by minimum reputation threshold
   c. From the eligible set, select 2 agents using VRF-derived randomness

3. Probe both agents for current load:
   a. Send lightweight load query (heartbeat-like)
   b. Each agent responds with: { active_jobs, load_factor, estimated_availability }

4. Compare:
   if agent_a.load_factor < agent_b.load_factor:
       assign to agent_a
   else:
       assign to agent_b
   (ties broken by lower agent_id for determinism)

5. Record assignment on-chain:
   - Job → assigned agent
   - Assignment block number
   - VRF proof (for auditability)
```

### VRF-Based Selection

The two probed agents are selected using Verifiable Random Functions (VRFs), not pseudorandom number generators. A VRF produces a random output together with a cryptographic proof that the output was correctly derived from the input. This prevents the dispatcher from biasing agent selection.

```rust
pub struct VrfSelection {
    /// VRF output used as randomness seed.
    pub vrf_output: [u8; 32],

    /// Proof that the VRF output was correctly computed.
    pub vrf_proof: [u8; 64],

    /// The two selected agent IDs.
    pub selected_agents: [u256; 2],

    /// The input to the VRF: hash of (job_id, block_hash, eligible_set_hash).
    pub vrf_input: [u8; 32],
}
```

Anyone can verify the VRF proof to confirm that the selection was unbiased. This prevents a malicious dispatcher from routing jobs preferentially to colluding agents.

---

## Load Metric

The load factor reported by each agent is a normalized measure of current capacity utilization:

```rust
pub struct AgentLoad {
    /// Number of jobs currently in progress.
    pub active_jobs: u32,

    /// Estimated capacity (max concurrent jobs this agent can handle).
    pub capacity: u32,

    /// Load factor: active_jobs / capacity. Range [0.0, 1.0].
    pub load_factor: f64,

    /// Estimated block number when the agent will next be free.
    pub estimated_free_block: u64,
}
```

Load factors are self-reported via heartbeat messages on the EventBus. An agent could lie about its load to attract more jobs (claiming to be idle when busy) or avoid jobs (claiming to be busy when idle). The peer scoring system (see [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md)) penalizes agents whose self-reported load is inconsistent with their actual job completion patterns.

---

## Comparison with Full Auction

| Property | Sparrow (Power of Two) | ERC-8183 job market Auction |
|---|---|---|
| **Communication cost** | O(1) per job (probe 2 agents) | O(B) per job (B bidders) |
| **Latency** | 1-2 blocks (400-800ms) | Auction window (10-100 blocks) |
| **Quality of match** | Good (load-balanced) | Best (reputation-adjusted bids) |
| **Price discovery** | None (fixed budget) | Yes (market-clearing price) |
| **Manipulation resistance** | VRF prevents selection bias | Sealed bids prevent collusion |
| **Use case** | Routine, time-sensitive jobs | High-value, quality-sensitive jobs |

Sparrow is the right choice when speed matters more than optimal matching. A coding agent that needs a quick test run does not need a 30-second auction — it needs an available agent right now.

---

## Theoretical Foundation

The power of two choices is one of the most studied results in randomized algorithms:

- **Azar, Broder, Karlin, and Upfal (1999)** proved the O(log log N) maximum load bound for the balanced allocation (two-choice) algorithm. This is exponentially better than the O(log N / log log N) bound for single-choice random assignment.
- **Mitzenmacher (2001)** provided a comprehensive survey of the power of two choices and its applications to load balancing, hashing, and shared-memory emulation.
- **Ousterhout, Agrawal, Erickson et al. (2013)** applied the technique to distributed task scheduling in the Sparrow system, demonstrating near-optimal response times for short jobs in large clusters.

The key theorem: with N bins and N balls, if each ball is placed in the less loaded of two randomly chosen bins, the maximum load is O(log log N) with high probability. This is a factor of log N / (log log N)² improvement over random placement.

| Network Size (N) | Random Assignment Max Load | Power of Two Max Load | Improvement Factor |
|---|---|---|---|
| 100 | ~5× average | ~2× average | 2.5× |
| 1,000 | ~7× average | ~2.3× average | 3× |
| 10,000 | ~9× average | ~2.5× average | 3.6× |
| 100,000 | ~11× average | ~2.7× average | 4× |

---

## Fallback Behavior

If both probed agents are overloaded (load_factor > 0.9):

```
1. Probe 2 more agents (total: 4 probes)
2. If all 4 overloaded: queue the job with exponential backoff
3. After 3 failed probe rounds: escalate to full ERC-8183 job market auction
4. If no eligible agents available: return job to poster with budget refund
```

The escalation path ensures jobs are not lost. If the network is at capacity, the job transitions from the fast Sparrow path to the slower but more thorough ERC-8183 job market auction path.

---

## Current Status and Gaps

**Scaffold:**
- Power-of-two-choices algorithm well-understood (published implementations exist)
- VRF primitives available in Rust (`vrf` crate)
- Load reporting via heartbeat EventBus defined

**Not yet built (Tier 6):**
- Sparrow dispatch implementation (§C8)
- VRF-based agent selection (§C9)
- Load probing protocol (§C10)
- Fallback escalation to ERC-8183 job market auction (§C11)
- Integration with EventBus heartbeat for load reporting (§C12)

---

## Cross-References

- See [10-spore-job-market.md](./10-spore-job-market.md) for the full marketplace protocol that Sparrow complements
- See [12-three-hiring-models.md](./12-three-hiring-models.md) for how RandomVRF relates to the other hiring models
- See [09-peer-scoring-3-layer.md](./09-peer-scoring-3-layer.md) for how self-reported load consistency affects peer scores

## Academic Foundations

- Azar, Y., Broder, A.Z., Karlin, A.R., and Upfal, E. (1999). "Balanced Allocations." *SIAM Journal on Computing*, 29(1). — The foundational proof of O(log log N) maximum load for two-choice allocation.
- Mitzenmacher, M. (2001). "The Power of Two Choices in Randomized Load Balancing." *IEEE Transactions on Parallel and Distributed Systems*, 12(10). — Comprehensive survey and analysis of the power of two choices.
- Ousterhout, K., Agrawal, P., Erickson, D., et al. (2013). "Sparrow: Distributed, Low Latency Scheduling." *SOSP*. — The Sparrow scheduler that applies power-of-two-choices to distributed cluster scheduling. Direct inspiration for this protocol's name and design.
