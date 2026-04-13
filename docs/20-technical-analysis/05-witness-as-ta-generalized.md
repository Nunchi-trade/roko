# The Witness Pipeline — Generalized Data Ingestion for TA

> The witness is the perception layer of technical analysis. Originally designed for blockchain observation, it generalizes to any structured data stream. Every oracle needs a witness to feed it data.


> **Implementation**: Specified

**Topic**: [Technical Analysis](./INDEX.md)
**Prerequisites**: [01-oracle-trait](./01-oracle-trait.md) for Oracle trait, [00-architecture](../00-architecture/INDEX.md) for Synapse Architecture
**Key sources**: `bardo-backup/prd/23-ta/00-witness-as-technical-analyst.md`, `refactoring-prd/03-cognitive-subsystems.md`

---

## The witness concept

In the original chain-centric architecture, the "witness" was a module in `roko-chain` that observed blockchain state and translated it into signals for the TA subsystem. In the generalized Roko architecture, the witness is a **domain-agnostic data ingestion pipeline** that feeds structured observations to any Oracle implementation.

The witness maps to Step 1 (PERCEIVE) of the universal cognitive loop:

```
1. PERCEIVE → Substrate.query() → Witness reads current state
```

Every domain has its own witness:

| Domain | Witness source | Data type | Cadence |
|---|---|---|---|
| **Chain** | RPC nodes, indexers, mempools | Block, transaction, event data | Per-block (~12s on Ethereum) |
| **Coding** | File system, CI/CD, Git, test runners | Build results, test outcomes, code metrics | Per-commit or continuous |
| **Research** | APIs, databases, citation indices | Papers, citations, claims | On-demand or periodic |
| **Operations** | Metrics systems, log aggregators | Latency, error rates, throughput | Continuous (sub-second) |

---

## Generalized witness trait

```rust
/// Universal data ingestion interface.
///
/// A Witness observes a structured domain and produces Engrams
/// that feed into Oracles, Scorers, and the rest of the Synapse pipeline.
pub trait Witness: Send + Sync {
    /// Observe the current state of the domain.
    /// Returns a batch of Engrams representing new observations.
    async fn observe(&self, since: i64) -> Result<Vec<Engram>>;

    /// Subscribe to a real-time stream of observations.
    /// Returns a receiver that emits Engrams as events occur.
    async fn subscribe(&self) -> Result<mpsc::Receiver<Engram>>;

    /// Get the witness's current health status.
    fn health(&self) -> WitnessHealth;
}

pub struct WitnessHealth {
    /// Is the data source reachable?
    pub connected: bool,
    /// How far behind is the witness? (0 = real-time)
    pub lag_ms: i64,
    /// Number of observations since last health check.
    pub observations_since_last: u64,
    /// Error count since last health check.
    pub errors_since_last: u64,
}
```

### Chain witness implementation

```rust
/// Blockchain witness: observes chain state via RPC.
pub struct ChainWitness {
    client: Arc<dyn ChainClient>,
    filters: Vec<ChainFilter>,
    last_block: AtomicU64,
}

#[async_trait]
impl Witness for ChainWitness {
    async fn observe(&self, since: i64) -> Result<Vec<Engram>> {
        let current_block = self.client.block_number().await?;
        let last = self.last_block.load(Ordering::Relaxed);

        let mut engrams = Vec::new();
        for block_num in last..=current_block {
            let block = self.client.block(block_num).await?;
            for filter in &self.filters {
                let filtered = filter.apply(&block)?;
                engrams.extend(filtered.into_iter().map(|data| {
                    Engram::builder()
                        .kind(Kind::Observation)
                        .body(Body::Json(data))
                        .tag("domain", "chain")
                        .tag("block", block_num.to_string())
                        .build()
                }));
            }
        }

        self.last_block.store(current_block, Ordering::Relaxed);
        Ok(engrams)
    }

    async fn subscribe(&self) -> Result<mpsc::Receiver<Engram>> {
        let (tx, rx) = mpsc::channel(1024);
        let client = self.client.clone();
        let filters = self.filters.clone();

        tokio::spawn(async move {
            let mut stream = client.subscribe_blocks().await.unwrap();
            while let Some(block) = stream.next().await {
                for filter in &filters {
                    if let Ok(filtered) = filter.apply(&block) {
                        for data in filtered {
                            let engram = Engram::builder()
                                .kind(Kind::Observation)
                                .body(Body::Json(data))
                                .tag("domain", "chain")
                                .build();
                            let _ = tx.send(engram).await;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }

    fn health(&self) -> WitnessHealth {
        WitnessHealth {
            connected: self.client.is_connected(),
            lag_ms: self.compute_lag(),
            observations_since_last: self.observation_count.swap(0, Ordering::Relaxed),
            errors_since_last: self.error_count.swap(0, Ordering::Relaxed),
        }
    }
}
```

### Coding witness implementation

```rust
/// Coding workspace witness: observes build results, test outcomes,
/// code metrics, and Git activity.
pub struct CodingWitness {
    /// File system watcher for code changes.
    fs_watcher: Arc<FsWatcher>,

    /// Git repository interface.
    git: Arc<GitRepository>,

    /// CI/CD pipeline interface.
    ci: Arc<dyn CiPipeline>,

    /// Code metrics calculator (via roko-index).
    metrics: Arc<CodeMetrics>,
}

#[async_trait]
impl Witness for CodingWitness {
    async fn observe(&self, since: i64) -> Result<Vec<Engram>> {
        let mut engrams = Vec::new();

        // Git changes since last observation
        let commits = self.git.commits_since(since).await?;
        for commit in commits {
            engrams.push(Engram::builder()
                .kind(Kind::Observation)
                .body(Body::Json(serde_json::to_value(&commit)?))
                .tag("domain", "coding")
                .tag("event", "commit")
                .tag("hash", &commit.hash)
                .build());
        }

        // Latest build result
        if let Some(build) = self.ci.latest_build().await? {
            engrams.push(Engram::builder()
                .kind(Kind::Observation)
                .body(Body::Json(serde_json::to_value(&build)?))
                .tag("domain", "coding")
                .tag("event", "build")
                .tag("status", if build.success { "pass" } else { "fail" })
                .build());
        }

        // Latest test results
        if let Some(tests) = self.ci.latest_test_results().await? {
            engrams.push(Engram::builder()
                .kind(Kind::Observation)
                .body(Body::Json(serde_json::to_value(&tests)?))
                .tag("domain", "coding")
                .tag("event", "tests")
                .tag("pass_rate", format!("{:.2}", tests.pass_rate))
                .build());
        }

        // Complexity metrics snapshot
        let complexity = self.metrics.workspace_complexity().await?;
        engrams.push(Engram::builder()
            .kind(Kind::Observation)
            .body(Body::Json(serde_json::to_value(&complexity)?))
            .tag("domain", "coding")
            .tag("event", "complexity")
            .build());

        Ok(engrams)
    }

    async fn subscribe(&self) -> Result<mpsc::Receiver<Engram>> {
        let (tx, rx) = mpsc::channel(1024);
        let fs_watcher = self.fs_watcher.clone();

        tokio::spawn(async move {
            let mut events = fs_watcher.watch().await.unwrap();
            while let Some(event) = events.next().await {
                let engram = Engram::builder()
                    .kind(Kind::Observation)
                    .body(Body::Json(serde_json::to_value(&event).unwrap()))
                    .tag("domain", "coding")
                    .tag("event", "fs_change")
                    .build();
                let _ = tx.send(engram).await;
            }
        });

        Ok(rx)
    }

    fn health(&self) -> WitnessHealth {
        WitnessHealth {
            connected: self.fs_watcher.is_watching() && self.ci.is_connected(),
            lag_ms: 0,  // file system events are real-time
            observations_since_last: self.observation_count.swap(0, Ordering::Relaxed),
            errors_since_last: self.error_count.swap(0, Ordering::Relaxed),
        }
    }
}
```

---

## Triage pipeline — Filtering and classification

Not every observation deserves attention. The triage pipeline filters and classifies incoming data before it reaches the oracle:

```rust
/// Triage pipeline: filter, classify, and prioritize observations.
///
/// Uses streaming anomaly detection (MIDAS-R) and percentile
/// estimation (DDSketch) to identify significant events without
/// storing the full data stream.
pub struct TriagePipeline {
    /// MIDAS-R anomaly detector: identifies sudden changes in
    /// streaming data using count-min sketch structures.
    /// O(1) memory, sub-microsecond per update.
    anomaly_detector: MidasR,

    /// DDSketch percentile estimator: tracks percentiles of
    /// streaming numeric data with relative-error guarantees.
    /// O(1) memory per sketch.
    percentile_tracker: DdSketch,

    /// Classification rules: map observations to categories.
    classifiers: Vec<Box<dyn ObservationClassifier>>,

    /// Priority scoring: determines which observations are
    /// worth routing to T1/T2 cognition.
    priority_scorer: PriorityScorer,
}

impl TriagePipeline {
    /// Process a batch of observations.
    /// Returns only the observations that warrant further analysis.
    pub fn triage(&mut self, observations: &[Engram]) -> Vec<TriagedObservation> {
        observations.iter().filter_map(|obs| {
            // Step 1: anomaly detection
            let anomaly_score = self.anomaly_detector.score(obs);

            // Step 2: percentile context
            let percentile = self.percentile_tracker.rank(obs.numeric_value()?);

            // Step 3: classification
            let category = self.classify(obs);

            // Step 4: priority scoring
            let priority = self.priority_scorer.score(anomaly_score, percentile, &category);

            if priority > 0.2 {  // threshold for attention
                Some(TriagedObservation {
                    observation: obs.clone(),
                    anomaly_score,
                    percentile,
                    category,
                    priority,
                })
            } else {
                None
            }
        }).collect()
    }
}
```

MIDAS-R (Bhatia et al., 2020, *AAAI*) provides streaming anomaly detection with O(1) memory — it identifies sudden changes in data streams without storing history. DDSketch (Masson et al., 2019, *PVLDB*) provides streaming percentile estimation with relative-error guarantees. Together, they allow the triage pipeline to process millions of observations per second while maintaining constant memory usage.

---

## CorticalState — The shared signal bus

The `CorticalState` is the working memory for the witness pipeline. All TA subsystems read from and write to this shared state:

```rust
/// Shared state for technical analysis across all domains.
///
/// The CorticalState is the "blackboard" that all TA components
/// read from and write to. It is domain-parameterized: chain agents
/// have chain-specific signals, coding agents have coding-specific
/// signals, but the structure is identical.
pub struct CorticalState<const N: usize> {
    /// N atomic signal values, updated by T0 probes.
    /// Chain: 8 signals (price, tvl, position, gas, credit, rsi, macd, circuit)
    /// Coding: 6 signals (build, test, complexity, deps, coverage, error_rate)
    pub signals: [AtomicF64; N],

    /// Current prediction error scalar (drives T0/T1/T2 routing).
    pub prediction_error: AtomicF64,

    /// Probe weights (used to combine signals into prediction error).
    pub weights: [AtomicF64; N],

    /// Current behavioral state from Daimon.
    pub behavioral_state: AtomicU8,

    /// Timestamp of last update.
    pub last_update_ms: AtomicI64,
}

/// Chain CorticalState with 8 signals.
pub type ChainCorticalState = CorticalState<8>;

/// Coding CorticalState with 6 signals.
pub type CodingCorticalState = CorticalState<6>;
```

The CorticalState is updated at Gamma frequency by the T0 probes. All operations are atomic — no locking, no allocation, sub-microsecond latency. This is what enables the "80% of ticks cost nothing" property: the probes update atomic values, combine them into a prediction error scalar, and the tier router reads the scalar to decide whether to invoke an LLM.

---

## Three cognitive speeds in the witness

The witness pipeline operates at all three cognitive speeds:

### Gamma (~5-15s) — Real-time observation

```
Witness.observe()
  → New observations
  → Triage pipeline (MIDAS-R + DDSketch)
  → CorticalState update (atomic signals)
  → T0 probes compute prediction error scalar
  → T0/T1/T2 routing decision
```

At Gamma, only the triage pipeline and T0 probes run. No LLM. Cost: microseconds.

### Theta (~75s) — Reflective analysis

```
Witness.observe() accumulates since last Theta tick
  → Pending predictions resolved against observations
  → Residuals computed, CalibrationTracker updated
  → Oracle re-predicts for next horizon
  → Significant observations stored to Neuro as knowledge
```

At Theta, the oracle makes explicit predictions and resolves pending ones. The LLM may be involved (T1 or T2) if the prediction error scalar warrants it.

### Delta (hours) — Consolidation

```
Dreams process accumulated observations
  → NREM replay of significant observation episodes
  → REM counterfactual: "what if this observation pattern recurred?"
  → Cross-domain pattern consolidation
  → Routing table updates based on observation patterns
```

At Delta, the Dreams subsystem consolidates observation patterns into permanent knowledge. This is where the witness's observations become long-term learning.

---

## Witness pipeline integration with VCG auction

Witness observations compete for context window space through the VCG attention auction:

```rust
/// Witness observations bid for context inclusion.
///
/// High-anomaly observations bid aggressively (high surprise value).
/// Routine observations bid modestly (low information content).
/// The VCG mechanism ensures truthful bidding.
pub fn observation_bid(obs: &TriagedObservation, ctx: &AuctionContext) -> f64 {
    let surprise_value = obs.anomaly_score;
    let relevance = ctx.task_relevance(&obs.category);
    let urgency = ctx.daimon_arousal;

    surprise_value * relevance * urgency
}
```

---

## Memory architecture — Three timescales

The witness integrates with Roko's three-timescale memory architecture, mirroring the Complementary Learning Systems (CLS) theory (McClelland, 1995):

| Timescale | Memory type | What it stores | Decay |
|---|---|---|---|
| **Gamma** (seconds) | CorticalState | Current signal values, prediction error | Overwritten each tick |
| **Theta** (minutes) | Working Engrams | Recent observations, pending predictions | Hours (Ebbinghaus) |
| **Delta** (hours) | Neuro knowledge | Validated patterns, calibration data | Days to months (tier-dependent) |

Fast episodic memory (Gamma/Theta) captures details. Slow semantic memory (Delta/Neuro) captures patterns. Dreams consolidate fast to slow, replicating the hippocampal-cortical memory consolidation that CLS theory describes.

---

## Academic foundations

- Bhatia, S., Hooi, B., Yoon, M., Shin, K., & Faloutsos, C. (2020). "MIDAS: Microcluster-Based Detector of Anomalies in Edge Streams." *AAAI 2020*. — Streaming anomaly detection.
- Masson, C., et al. (2019). "DDSketch: A Fast and Fully-Mergeable Quantile Sketch." *PVLDB*, 12(12), 2195-2205. — Streaming percentile estimation.
- McClelland, J. L., McNaughton, B. L., & O'Reilly, R. C. (1995). "Why there are complementary learning systems in the hippocampus and neocortex." *Psychological Review*, 102(3), 419-457. — CLS theory.
- Charnov, E. L. (1976). "Optimal foraging: the marginal value theorem." *Theoretical Population Biology*, 9, 129-136. — Stopping rule for observation foraging.
- Friston, K. (2010). "The free-energy principle." *Nature Reviews Neuroscience*, 11(2), 127-138. — Active inference driving observation priority.
- Chen, L., et al. (2023). "FrugalGPT." arXiv:2305.05176. — T0 probe architecture.

---

## Cross-References

- See [01-oracle-trait.md](./01-oracle-trait.md) for the Oracle trait that witnesses feed
- See [02-chain-oracles.md](./02-chain-oracles.md) for chain witness integration
- See [03-coding-oracles.md](./03-coding-oracles.md) for coding witness integration
- See [08-adaptive-signal-metabolism.md](./08-adaptive-signal-metabolism.md) for how signals evolve in the witness pipeline
- See [13-predictive-foraging-and-active-inference.md](./13-predictive-foraging-and-active-inference.md) for the full prediction loop
