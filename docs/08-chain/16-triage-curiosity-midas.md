# Triage Pipeline: Curiosity Scoring and MIDAS-R Anomaly Detection

> The triage pipeline is a 4-stage classification system: rule-based filters → MIDAS-R streaming anomaly detection → contextual enrichment → HDC/Bayesian curiosity scoring. It assigns each transaction a curiosity score that determines whether it is noise (ignore), worth tracking (silent monitor), or worth escalating (LLM analysis). No LLM calls touch this path — speed comes from pure computation.

**Topic**: [08-chain](./INDEX.md)
**Prerequisites**: [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md), [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md)
**Key sources**: `bardo-backup/prd/14-chain/02-triage.md`, `roko/tmp/implementation-plans/12b-chain-layer.md` §H

---

## Abstract

The triage pipeline is the agent's judgment layer for on-chain activity. For every block that passes the Binary Fuse pre-screen (see [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md)), the pipeline classifies each transaction: what is it, how interesting is it, and what should happen next. No LLM calls touch this path — that is what makes it fast. LLM analysis happens asynchronously at Theta tick on high-scoring events only.

The pipeline synthesizes four research traditions — rule-based heuristics, streaming anomaly detection, hyperdimensional computing, and Bayesian surprise — into a unified 4-stage system that runs at block rate.

Each chain gets its own `TriageEngine` instance with its own statistical structures (MIDAS-R, DDSketch, Count-Min Sketch). Cross-chain signals flow through the event fabric, not shared mutable state. This eliminates Mutex contention across multiple chains.

---

## Pipeline Overview

```
Stage 1: CLASSIFY         Stage 2: ANOMALY          Stage 3: ENRICH          Stage 4: SCORE
─────────────────         ───────────────           ─────────────────        ──────────────
Rule-based                MIDAS-R streaming         Protocol state           HDC fingerprint
categorization            anomaly detection         enrichment               + Bayesian surprise
                                                                             → curiosity score

Input: NormalizedBlock    MIDAS-R hash table        ProtocolState lookup     HDC similarity
  └→ for each tx:        DDSketch quantiles         ABI decoding             Count-Min novelty
     decode logs          Count-Min Sketch           Cross-reference          4-factor composite
     match signatures     frequency tracking         known patterns
     categorize                                                              Output:
                                                                               curiosity_score
     Output:              Output:                   Output:                     ∈ [0.0, 1.0]
       category           anomaly_score             enriched_event
       confidence         statistical_context       protocol_context
```

---

## Stage 1: Rule-Based Classification

The first stage decodes each transaction and categorizes it:

### Categories

```rust
pub enum TxCategory {
    /// Known DeFi interaction: swap, mint, burn, deposit, withdraw, etc.
    DeFi { protocol_family: ProtocolFamily, action: DeFiAction },

    /// Token transfer (ERC-20, ERC-721, ERC-1155).
    Transfer { standard: TokenStandard, value: U256 },

    /// Contract deployment.
    Deployment { bytecode_hash: B256 },

    /// Governance action: proposal, vote, execution.
    Governance { protocol: String, action: GovernanceAction },

    /// Oracle update: price feed, data delivery.
    OracleUpdate { oracle: String, asset: String },

    /// Known attack pattern: flash loan, sandwich, reentrancy.
    SuspiciousPattern { pattern: AttackPattern, confidence: f64 },

    /// Unknown: transaction does not match any known signature.
    Unknown { selector: [u8; 4] },
}
```

Classification is done by matching transaction input selectors and log topics against a registry of known function signatures and event topics. The registry is populated from the protocol state engine's ABI resolution chain (see the protocol state system described in the legacy architecture).

### Performance

- Selector matching: O(1) via HashMap lookup
- Log topic matching: O(1) per topic via HashMap
- Classification of 200 transactions per block: ~100μs total
- No external calls, no LLM, pure in-memory computation

---

## Stage 2: MIDAS-R Anomaly Detection

### What is MIDAS-R?

MIDAS-R (Bhatia et al., 2020) is a streaming anomaly detection algorithm that maintains a compact hash table of edge counts in a temporal graph. When a new edge (transaction between two addresses) arrives, MIDAS-R computes a chi-squared statistic comparing the observed count to the expected count based on historical rates. Edges with counts significantly above the expected rate receive high anomaly scores.

MIDAS-R is designed for streaming data:
- **Constant memory**: O(w × d) where w = hash table width, d = hash table depth
- **Constant time per update**: O(d) per edge
- **Temporal decay**: Counts decay automatically, so the algorithm adapts to changing patterns

### Application to Chain Intelligence

Each transaction creates edges in a temporal graph:
- `from_address → to_address` (direct transfer edge)
- `from_address → contract_address` (contract interaction edge)
- `contract_address → event_topic` (event emission edge)

MIDAS-R tracks the frequency of these edges over time. A sudden burst of transactions between two addresses, or a sudden increase in a specific event type, produces a high anomaly score.

### Complementary Statistics

MIDAS-R is complemented by two additional structures:

**DDSketch** (Masson et al., 2019): Maintains quantile estimates for transaction values, gas prices, and other numerical fields. A transaction with gas price in the 99.9th percentile of recent history is flagged.

```rust
pub struct AnomalyState {
    /// MIDAS-R hash table for temporal edge anomaly detection.
    pub midas: MidasR,

    /// DDSketch for quantile estimates on numerical fields.
    pub sketches: HashMap<String, DDSketch>,

    /// Count-Min Sketch for frequency estimation.
    pub frequency: CountMinSketch,
}
```

**Count-Min Sketch** (Cormode and Muthukrishnan, 2005): Estimates the frequency of any item in a data stream using sub-linear space. Used to track how often specific addresses, selectors, or event topics appear. Novel items (low frequency) score higher for curiosity.

---

## Stage 3: Contextual Enrichment

The enrichment stage adds protocol-specific context to each event:

1. **Protocol state lookup**: What is the current state of the contract being interacted with? (e.g., pool liquidity, vault share price, market utilization)
2. **ABI decoding**: Decode the transaction input and log data using the resolved ABI
3. **Cross-referencing**: Match the event against known patterns from the agent's knowledge base
4. **Historical context**: What has this address done in the past? (from the agent's local observation history)

The enriched event contains both the raw on-chain data and the interpretive context needed for scoring.

---

## Stage 4: Curiosity Scoring

The final stage combines all signals into a single curiosity score in [0.0, 1.0]:

### Four Factors

```rust
pub struct CuriosityScore {
    /// Rule-based relevance: does this match what the agent is watching?
    pub relevance: f64,

    /// MIDAS-R anomaly: is the pattern statistically unusual?
    pub anomaly: f64,

    /// HDC novelty: how different is this from the agent's existing knowledge?
    pub novelty: f64,

    /// Bayesian surprise: how much does this event change the agent's beliefs?
    pub surprise: f64,

    /// Weighted composite score.
    pub composite: f64,
}

fn compute_curiosity(
    relevance: f64,
    anomaly: f64,
    novelty: f64,
    surprise: f64,
) -> f64 {
    // Weights configurable per agent; defaults:
    let w_relevance = 0.30;
    let w_anomaly   = 0.25;
    let w_novelty   = 0.25;
    let w_surprise  = 0.20;

    w_relevance * relevance
        + w_anomaly * anomaly
        + w_novelty * novelty
        + w_surprise * surprise
}
```

### HDC Novelty

The transaction's event pattern is encoded as a 10,240-bit BSC vector (using the same HDC encoding as the knowledge system; see [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md)). This vector is compared against the agent's existing knowledge using Hamming similarity. Events that are highly dissimilar from existing knowledge (low similarity) receive high novelty scores — they represent something the agent has not seen before.

### Bayesian Surprise

Bayesian surprise measures how much an event changes the agent's probabilistic model. If the agent's model predicts that pool X receives ~10 swaps per hour, and pool X suddenly receives 200 swaps in an hour, the Bayesian surprise is high — the observation is strongly inconsistent with the agent's prior expectations.

```
surprise = KL_divergence(posterior, prior)
```

Where the prior is the agent's model before the observation and the posterior is the updated model after the observation. High KL divergence = the observation changed the model significantly = high surprise.

### Curiosity Thresholds

| Score Range | Action | Latency Budget |
|---|---|---|
| 0.0 – 0.2 | **Ignore**: Event is noise. No further processing. | 0 |
| 0.2 – 0.5 | **Silent monitor**: Log the event. Update statistical models. No LLM. | ~1ms |
| 0.5 – 0.8 | **Alert**: Emit event on `korai/anomaly/v1` gossip topic. Queue for Theta-tick LLM analysis. | ~10ms |
| 0.8 – 1.0 | **Escalate**: Immediate Theta-tick interrupt. LLM analysis with priority context. Possible action trigger. | ~100ms |

The thresholds are adaptive — they adjust based on the current background curiosity level. During a period of high market activity, thresholds increase to avoid alert fatigue. During quiet periods, thresholds decrease to maintain sensitivity.

---

## Academic Foundations

- Bhatia, S. et al. (2020). "MIDAS: Microcluster-Based Detector of Anomalies in Edge Streams." *AAAI*. — The streaming anomaly detection algorithm used in Stage 2.
- Masson, C. et al. (2019). "DDSketch: A Fast and Fully-Mergeable Quantile Sketch with Relative-Error Guarantees." *PVLDB*. — Quantile estimation for numerical anomaly detection.
- Cormode, G. and Muthukrishnan, S. (2005). "An Improved Data Stream Summary: The Count-Min Sketch and its Applications." *Journal of Algorithms*. — Frequency estimation for novelty tracking.
- Itti, L. and Baldi, P. (2009). "Bayesian Surprise Attracts Human Attention." *Vision Research*. — Bayesian surprise as an attention mechanism; the curiosity scoring is a computational analog.
- Kanerva, P. (2009). "Hyperdimensional Computing." *Cognitive Computation*, 1(2). — HDC encoding for novelty detection in Stage 4.

---

## Current Status and Gaps

**Scaffold:**
- `bardo-primitives/src/hdc.rs`: Local HDC operations for novelty computation
- Triage concept defined in legacy `bardo-triage` spec

**Not yet built (Tier 6):**
- `TriageEngine` with 4-stage pipeline (§H7)
- MIDAS-R integration for streaming anomaly detection (§H8)
- DDSketch and Count-Min Sketch for statistical tracking (§H9)
- Curiosity scoring with adaptive thresholds (§H10)
- Integration with ChainWitness as input (§H11)
- Integration with gossip anomaly topic for alert broadcast (§H12)

---

## Cross-references

- See [15-chainwitness-event-watching.md](./15-chainwitness-event-watching.md) for the upstream block ingestion that feeds this pipeline
- See [03-hdc-on-chain-precompile.md](./03-hdc-on-chain-precompile.md) for the HDC encoding used in novelty scoring
- See [19-chain-agent-heartbeat.md](./19-chain-agent-heartbeat.md) for how triage feeds into the ANALYZE step of the 9-step heartbeat
- See [08-eight-gossip-topics.md](./08-eight-gossip-topics.md) for the anomaly gossip topic that carries escalated events
