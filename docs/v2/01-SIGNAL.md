# 01 — Signal and Pulse

> Two mediums: Signal (durable) in Store, Pulse (ephemeral) on Bus. Graduation converts Pulse → Signal. Everything that flows through Roko is one or the other.

**Subsumes**: Engram, Pulse/Envelope, Artifact, Knowledge Entry, Pheromone, Evidence, Feed event, Finding.

---

## 1. Two Mediums

The system has two data shapes because reality has two timescales: things that persist and things that flow. The v1 spec claimed "one noun" (Signal) but the code already had two — `Engram` for durable data and `Envelope<E>` in `roko-runtime::event_bus` for ephemeral messages. This spec makes both first-class.

| Property | Signal (durable) | Pulse (ephemeral) |
|---|---|---|
| **Identity** | Content hash (SHA-256 of payload) | `(topic, seq)` tuple |
| **Durability** | Store (`.roko/signals.jsonl`, knowledge store) | Ring buffer on Bus (~4,096 entries default) |
| **Lineage** | Full `Vec<SignalRef>` provenance DAG | Optional `lineage_hint: Option<ContentHash>` |
| **Scoring** | 5-dimensional Score | None |
| **Retention** | Demurrage (Gesell 1916): balance decays unless actively used | Ring buffer eviction |
| **HDC fingerprint** | 10,240-bit binary vector (1,280 bytes, Kanerva 2009) | None (too transient) |
| **Taint** | Lattice-based IFC classification (see S10) | Inherited from source |
| **Typical rate** | 1 Hz – 1 kHz | 1 Hz – 1 MHz |
| **Typical lifetime** | Minutes to permanent | Milliseconds to seconds |

They are **siblings, not parent-child**. A Signal is not "a Pulse that grew up." The only bridges are explicit:

- **Graduation**: `Pulse::graduate(provenance, initial_balance, score, tags) → Signal` — the ONLY path from transport into the audit DAG.
- **Projection**: `Signal::to_pulse(topic, seq) → Pulse` — lossy broadcast of stored Signals.

---

## 2. Signal — The Durable Medium

```rust
pub struct Signal {
    // ── Identity ──────────────────────────────────────────────────
    pub id: SignalId,                    // ULID, globally unique
    pub content_hash: ContentHash,       // SHA-256 of canonical payload bytes
    pub kind: Kind,                      // discriminant (see §4)

    // ── Content ───────────────────────────────────────────────────
    pub payload: Value,                  // serde_json::Value, schema-validated
    pub schema: TypeSchema,              // structural type

    // ── Scoring ───────────────────────────────────────────────────
    pub score: Score,                    // 5-axis quality rating
    pub confidence: f64,                 // 0.0..=1.0

    // ── Demurrage ─────────────────────────────────────────────────
    pub balance: f64,                    // starts at 1.0, decays via demurrage
    pub demurrage_paid: f64,             // cumulative tax paid (monotonic)
    pub last_touched_at: DateTime<Utc>,  // last retrieval, citation, or gate-pass
    pub tier: Tier,                      // Transient | Working | Consolidated | Persistent
    pub created_at: DateTime<Utc>,

    // ── Lineage ───────────────────────────────────────────────────
    pub source: Vec<SignalRef>,          // upstream Signals (provenance DAG)
    pub provenance: Provenance,          // generation metadata, citations, taint, sources

    // ── Embedding ─────────────────────────────────────────────────
    pub hdc_fingerprint: HdcVector,      // 10,240-bit binary vector (1,280 bytes)

    // ── Authorship ────────────────────────────────────────────────
    pub author: Author,                  // agent ID, wallet address, or system
    pub tags: Vec<String>,               // topic tags for discovery
}
```

**Mapping to code**: `Signal` maps 1:1 to `roko-core::Engram`. The Rust struct remains `Engram`; new code bridges with `type Signal = Engram;`.

### 2.1 The Signal Struct as an Algebraic Object

The algebraic core lives in three fields:

- `content_hash` participates in the **lineage monoid** (append-only DAG).
- `hdc_fingerprint` participates in the **vector semiring** (bind + bundle).
- `kind` participates in the **kind lattice** (flat kinds join into Compound).

These three algebraic structures are independent but interact at composition boundaries. **Identity is algebraically exact (hash monoid); similarity is algebraically approximate (vector semiring).**

---

## 3. Pulse — The Ephemeral Medium

```rust
pub struct Pulse {
    pub seq: u64,                        // monotonic per Bus instance
    pub topic: Topic,                    // hierarchical string (OpenTelemetry-style)
    pub kind: Kind,                      // reused from Signal
    pub body: Value,                     // payload
    pub emitted_at_ms: i64,              // Unix ms, server clock
    pub source: PulseSource,             // who emitted
    pub lineage_hint: Option<ContentHash>, // back-reference to Signal context
    pub trace_id: Option<TraceId>,       // distributed tracing
}

pub enum PulseSource {
    Agent(AgentId),
    Cell(CellRef),
    Graph(GraphRef),
    System,
    External(String),
}
```

**Mapping to code**: Pulse replaces `Envelope<E>` in `roko-runtime::event_bus`.

### Topic taxonomy

```
orchestration.plan.started           Plan lifecycle
orchestration.task.ready             Task readiness
agent:{id}.heartbeat                 Agent heartbeat ticks
agent:{id}.output                    Streaming LLM output
agent:{id}.turn.completed            Turn completed
gate.verdict.emitted                 Gate results (graduates)
safety.approval.requested            Safety events (graduates)
conductor.circuit.tripped            Health events (graduates)
prediction.{operator}                Operator predictions (for calibration)
outcome.{operator}                   Operator outcomes (for calibration)
calibration.{operator}.updated       Error signals
pheromone.{location_hash}            Stigmergic coordination
cost.charged                         Budget tracking
ui.refresh.requested                 UI-only (does not graduate)
heartbeat.tick                       Clock infrastructure (does not graduate)
```

### Graduation policy

| Topic | Graduate? | Rationale |
|---|---|---|
| `gate.verdict.emitted` | Yes | Audit-critical |
| `agent.*.turn.completed` | Yes (batch) | Episodes feed learning |
| `safety.approval.requested` | Yes | Safety must be auditable |
| `conductor.circuit.tripped` | Yes | Health events are forensic |
| `cost.charged` | Yes | Accounting record |
| `agent.*.output` (chunks) | Batch on stream close | Individual chunks are noise; full response is artifact |
| `heartbeat.tick` | No | Latest is all that matters |
| `ui.refresh.requested` | No | UI-local |
| `pheromone.*` | No (on-chain only) | Ephemeral by design |

---

## 4. Kind System

Every Signal and Pulse has a `Kind` determining schema, demurrage behavior, and Cell interaction.

```rust
#[non_exhaustive]
pub enum Kind {
    // ── Core data ──────────────────────────────────────
    Text, Markdown, Json, Toml,
    Code { language: String },
    Diff, Binary { mime: String }, Image { format: String },

    // ── Artifacts ──────────────────────────────────────
    File { path: PathBuf },
    Artifact { kind: ArtifactKind },

    // ── Knowledge ──────────────────────────────────────
    Insight,                             // observed pattern + evidence
    Heuristic,                           // when/then + mandatory falsifier + calibration
    Warning,                             // transient danger flag
    CausalLink,                          // cause → effect
    StrategyFragment,                    // reusable strategy component
    AntiKnowledge,                       // known-bad (repels similar entries)

    // ── Coordination ───────────────────────────────────
    Pheromone { ptype: PheromoneType },  // stigmergic: location + intensity
    Heartbeat,
    Presence { event: PresenceEvent },

    // ── Execution ──────────────────────────────────────
    Evidence { kind: EvidenceKind },     // typed verification evidence (19 kinds)
    Finding { severity: Severity },      // verification finding
    Verdict,                             // pass/fail + reward + evidence
    Episode,                             // recorded agent turn
    CostReport,

    // ── Observation ────────────────────────────────────
    Observation, Alert { level: AlertLevel }, Trend, Anomaly,

    // ── Compound ───────────────────────────────────────
    Compound { kinds: Vec<Kind> },       // lattice join of multiple kinds (see §4.2)

    // ── User-defined ───────────────────────────────────
    Custom { name: String },
}
```

### 4.1 Kind::Heuristic — first-class learned rule

A Heuristic is a testable prediction with a mandatory falsifier and a live calibration track record grounded in episode outcomes (not LLM self-report). Heuristics are richer than playbooks (sequences of actions) and more formal than rules of thumb.

```rust
pub struct HeuristicPayload {
    pub when: Vec<Predicate>,            // preconditions (matchable)
    pub then: String,                    // action or prediction
    pub falsifier: String,               // "what would prove this wrong?"
    pub calibration: Calibration,        // live track record
    pub receipts: Vec<SignalRef>,        // episodes where tested
}

pub struct Calibration {
    pub trials: u32,
    pub confirmations: u32,
    pub violations: u32,
    pub brier_score: f64,                // calibration quality
    pub confidence_interval: (f64, f64), // Wilson score CI
}
```

Heuristics are live-calibrated from Bus events (gate verdicts, agent outcomes). Confidence CI decays via demurrage if unchallenged. **Worldviews** emerge as coherent clusters of co-citing heuristics with high calibration scores (see [Doc-06](06-MEMORY.md)). Multiple worldviews are maintained deliberately: main + challenger + niche specialists.

### 4.2 Compound Kinds as Join in a Lattice

The Kind system forms a **join-semilattice** where Compound is the join operation:

```
         Compound([A, B, C])
        /        |        \
  Compound([A,B]) Compound([A,C]) Compound([B,C])
      / \          / \          / \
     A   B        A   C        B   C
```

The lattice bottom is `Kind::Custom("empty_compound")` (the error state). There is no lattice top.

```rust
/// Kind lattice join: a \/ b = compound([a, b])
///
/// Properties:
///   a \/ a = a                    (idempotent)
///   a \/ b = b \/ a              (commutative)
///   (a \/ b) \/ c = a \/ (b \/ c)  (associative, via flatten)
///   a \/ bot = a                  (identity)
impl Kind {
    pub fn join(a: Kind, b: Kind) -> Kind {
        Kind::compound([a, b])
    }
}
```

Filter matching is a lattice-theoretic operation: `signal.kind.matches(filter) iff filter <= signal.kind`. A `Kind::Verdict` filter matches `Kind::Compound([Verdict, TestResult])` because `Verdict <= Compound([Verdict, TestResult])` in the lattice ordering.

**Scaling**: With K = 30 distinct kinds and a max compound size of 4, there are `C(K,2) + C(K,3) + C(K,4) = 31,900` possible compounds. Manageable. The cap at 4 prevents combinatorial explosion.

---

## 5. Scoring

Every Signal carries a 5-dimensional `Score`:

```rust
pub struct Score {
    pub relevance:  f64,     // 0.0..=1.0
    pub quality:    f64,     // 0.0..=1.0
    pub confidence: f64,     // 0.0..=1.0
    pub novelty:    f64,     // 0.0..=1.0  (attenuated: 1/(1+ln(freq)))
    pub utility:    f64,     // 0.0..=1.0
}
```

Score Cells produce these. Route Cells consume them. Compose uses them for budget-constrained assembly. **Novelty attenuation**: `novelty = 1/(1+ln(freq))` — habituation that never reaches zero, so even highly familiar Signals retain a nonzero novelty floor.

### 5.1 The Effective Score Formula

The 5 axes collapse into a single scalar when a total ordering is needed:

```rust
impl Score {
    /// Collapse 5 axes into a single scalar.
    ///
    /// Properties:
    ///   - confidence = 0 -> effective = 0 (bad data is worthless)
    ///   - novelty acts as a bonus multiplier via (1 + novelty)
    ///   - utility acts as a bonus multiplier via (1 + utility)
    ///   - relevance and quality enter multiplicatively
    ///   - Novelty uses attenuation: novelty_eff = 1/(1+ln(1+freq))
    pub fn effective(&self) -> f64 {
        self.confidence
            * self.relevance.max(0.1)     // floor prevents relevance from killing score
            * self.quality.max(0.1)        // floor prevents quality from killing score
            * (1.0 + self.novelty)         // novelty bonus
            * (1.0 + self.utility)         // utility bonus
    }
}
```

**Why multiplicative**: (1) Zero-confidence kills — data known to be wrong cannot be prioritized by gaming other axes. (2) Bonus stacking is superlinear — a Signal that is novel AND useful gets `(1 + novelty) * (1 + utility)`. The `max(0.1)` floor on relevance and quality prevents zeroing the effective score for Signals that may be relevant to a future context, while confidence 0 *should* kill the score.

### 5.2 The Score-Verify-Score Calibration Loop

The calibration loop is a **predict-observe-update** structure (canonical Bayesian inference):

```
Score Cells produce scores → Compose uses scores to build context →
Agent acts on composed context → Verify Cells produce binary verdicts →
Calibration Cells update Score Cell parameters → Score Cells produce BETTER scores
→ (repeat)
```

### 5.3 Temperature Scaling (Guo et al. 2017)

Temperature scaling normalizes scores across different Score Cells. A confidence of 0.8 from a compile Verify Cell (deterministic) is not the same as 0.8 from an LLM judge (probabilistic). Temperature scaling corrects this:

```rust
/// Temperature scaling for a single score axis.
///
///   calibrated = sigmoid(logit(s) / T)
///
/// T > 1: reduces confidence (overconfident scorer)
/// T < 1: increases confidence (underconfident scorer)
/// T = 1: no change (perfectly calibrated)
pub fn temperature_scale(raw: f64, temperature: f64) -> f64 {
    if raw <= 0.0 || raw >= 1.0 {
        return raw.clamp(0.0, 1.0);
    }
    let logit = (raw / (1.0 - raw)).ln();
    let scaled_logit = logit / temperature;
    1.0 / (1.0 + (-scaled_logit).exp())
}
```

Temperature T is learned by minimizing Expected Calibration Error (ECE):

```rust
/// ECE = sum_b (|B_b| / N) * |accuracy(B_b) - confidence(B_b)|
pub fn compute_ece(
    predictions: &[(f64, bool)],  // (score, verdict_passed)
    n_bins: usize,
) -> f64 {
    let n = predictions.len() as f64;
    let bin_width = 1.0 / n_bins as f64;
    let mut ece = 0.0;
    for bin in 0..n_bins {
        let lo = bin as f64 * bin_width;
        let hi = lo + bin_width;
        let in_bin: Vec<&(f64, bool)> = predictions.iter()
            .filter(|(score, _)| *score >= lo && *score < hi)
            .collect();
        if in_bin.is_empty() { continue; }
        let bin_size = in_bin.len() as f64;
        let avg_confidence: f64 = in_bin.iter().map(|(s, _)| s).sum::<f64>() / bin_size;
        let accuracy: f64 = in_bin.iter().filter(|(_, p)| *p).count() as f64 / bin_size;
        ece += (bin_size / n) * (accuracy - avg_confidence).abs();
    }
    ece
}

/// Find the temperature that minimizes ECE via golden section search.
pub fn optimize_temperature(predictions: &[(f64, bool)], n_bins: usize) -> f64 {
    let mut lo = 0.1_f64;
    let mut hi = 10.0_f64;
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
    for _ in 0..50 {
        let x1 = hi - golden * (hi - lo);
        let x2 = lo + golden * (hi - lo);
        let scaled_1: Vec<(f64, bool)> = predictions.iter()
            .map(|(s, p)| (temperature_scale(*s, x1), *p)).collect();
        let scaled_2: Vec<(f64, bool)> = predictions.iter()
            .map(|(s, p)| (temperature_scale(*s, x2), *p)).collect();
        if compute_ece(&scaled_1, n_bins) < compute_ece(&scaled_2, n_bins) {
            hi = x2;
        } else {
            lo = x1;
        }
    }
    (lo + hi) / 2.0
}
```

### 5.4 Beta-Binomial Conjugate Calibration

The confidence axis has the cleanest calibration path. The Beta distribution is the conjugate prior for the Binomial likelihood:

```rust
/// Per-axis calibrator using Beta-Binomial conjugate updates.
///
/// Posterior: Beta(alpha + passes, beta + fails)
/// Posterior mean: E[theta] = alpha / (alpha + beta)
/// Posterior variance: Var = (alpha * beta) / ((alpha + beta)^2 * (alpha + beta + 1))
pub struct AxisCalibrator {
    alpha: f64,                          // pseudo-count for positive evidence
    beta: f64,                           // pseudo-count for negative evidence
    window: usize,                       // window size for recent-only calibration
    recent: VecDeque<(f64, bool)>,       // rolling buffer of (score, verdict) pairs
    learned_temperature: f64,            // updated periodically from recent buffer
}

impl AxisCalibrator {
    /// Default: weakly informative prior Beta(2, 2).
    /// Centered at 0.5, equivalent to 4 pseudo-observations.
    pub fn new(window: usize) -> Self {
        Self { alpha: 2.0, beta: 2.0, window, recent: VecDeque::with_capacity(window),
               learned_temperature: 1.0 }
    }

    pub fn update(&mut self, raw_score: f64, passed: bool) {
        if passed { self.alpha += 1.0; } else { self.beta += 1.0; }
        self.recent.push_back((raw_score, passed));
        if self.recent.len() > self.window { self.recent.pop_front(); }
        if self.recent.len() >= 20 && self.recent.len() % 10 == 0 {
            let pairs: Vec<(f64, bool)> = self.recent.iter().cloned().collect();
            self.learned_temperature = optimize_temperature(&pairs, 10);
        }
    }

    pub fn posterior_mean(&self) -> f64 { self.alpha / (self.alpha + self.beta) }

    pub fn posterior_uncertainty(&self) -> f64 {
        let n = self.alpha + self.beta;
        ((self.alpha * self.beta) / (n * n * (n + 1.0))).sqrt()
    }

    pub fn credible_interval(&self, level: f64) -> (f64, f64) {
        let tail = (1.0 - level) / 2.0;
        (beta_quantile(self.alpha, self.beta, tail),
         beta_quantile(self.alpha, self.beta, 1.0 - tail))
    }
}
```

### 5.5 Precision-Weighted Score Aggregation

When multiple Score Cells produce scores for the same Signal, aggregate via precision-weighted averaging (low-uncertainty scorers dominate):

```rust
pub fn aggregate_scores(scores: &[(Score, &CalibratedScorer)]) -> Score {
    let mut axes = [0.0_f64; 5];
    let mut weights = [0.0_f64; 5];
    for (score, scorer) in scores {
        let values = [score.relevance, score.quality, score.confidence,
                      score.novelty, score.utility];
        for (i, &val) in values.iter().enumerate() {
            let precision = 1.0 / scorer.calibrators[i]
                .posterior_uncertainty().max(1e-6).powi(2);
            axes[i] += val * precision;
            weights[i] += precision;
        }
    }
    Score {
        relevance:  (axes[0] / weights[0].max(1e-6)).clamp(0.0, 1.0),
        quality:    (axes[1] / weights[1].max(1e-6)).clamp(0.0, 1.0),
        confidence: (axes[2] / weights[2].max(1e-6)).clamp(0.0, 1.0),
        novelty:    (axes[3] / weights[3].max(1e-6)).clamp(0.0, 1.0),
        utility:    (axes[4] / weights[4].max(1e-6)).clamp(0.0, 1.0),
    }
}
```

### 5.6 Pareto Front

When no single aggregation is appropriate, maintain the Pareto front — the set of non-dominated Signals:

```rust
pub fn pareto_front(signals: &[Signal]) -> Vec<&Signal> {
    signals.iter().filter(|candidate| {
        !signals.iter().any(|other|
            std::ptr::eq(*candidate, other) == false
            && dominates(&other.score, &candidate.score)
        )
    }).collect()
}

fn dominates(a: &Score, b: &Score) -> bool {
    let axes_a = [a.relevance, a.quality, a.confidence, a.novelty, a.utility];
    let axes_b = [b.relevance, b.quality, b.confidence, b.novelty, b.utility];
    axes_a.iter().zip(axes_b.iter()).all(|(a, b)| a >= b)
        && axes_a.iter().zip(axes_b.iter()).any(|(a, b)| a > b)
}
```

### 5.7 Meta-Calibration

Track ECE of *calibrated* scores. When calibrated ECE exceeds threshold, the calibration model itself is miscalibrated:

```rust
pub struct MetaCalibrator {
    calibrated_ece: f64,
    ece_threshold: f64,                  // default 0.05
    meta_window: VecDeque<(f64, bool)>,
    meta_window_size: usize,             // default 200
}

pub enum CalibrationRemediation {
    ResetPrior,          // severe: ECE > 3x threshold
    SwitchToIsotonic,    // moderate: ECE > 2x threshold
    IncreaseWindow,      // mild: ECE > 1x threshold
}
```

### 5.8 Isotonic Regression Fallback (PAVA)

When temperature scaling is insufficient, isotonic regression provides a non-parametric alternative via the pool-adjacent-violators algorithm:

```rust
pub struct IsotonicCalibrator {
    breakpoints: Vec<(f64, f64)>,  // (raw_score, calibrated_probability)
}

impl IsotonicCalibrator {
    /// Fit from (score, verdict) pairs using PAVA.
    pub fn fit(pairs: &mut [(f64, bool)]) -> Self {
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut blocks: Vec<(f64, f64, usize)> = pairs.iter()
            .map(|(s, p)| (*s, if *p { 1.0 } else { 0.0 }, 1)).collect();
        let mut i = 0;
        while i < blocks.len() - 1 {
            if blocks[i].1 > blocks[i + 1].1 {
                let (s1, v1, n1) = blocks[i];
                let (s2, v2, n2) = blocks[i + 1];
                let merged_val = (v1 * n1 as f64 + v2 * n2 as f64) / (n1 + n2) as f64;
                blocks[i] = ((s1 + s2) / 2.0, merged_val, n1 + n2);
                blocks.remove(i + 1);
                if i > 0 { i -= 1; }
            } else { i += 1; }
        }
        Self { breakpoints: blocks.into_iter().map(|(s, v, _)| (s, v)).collect() }
    }

    pub fn calibrate(&self, raw: f64) -> f64 {
        if self.breakpoints.is_empty() { return raw; }
        let idx = self.breakpoints.partition_point(|(s, _)| *s < raw);
        if idx == 0 { return self.breakpoints[0].1; }
        if idx >= self.breakpoints.len() { return self.breakpoints.last().unwrap().1; }
        let (s0, v0) = self.breakpoints[idx - 1];
        let (s1, v1) = self.breakpoints[idx];
        let t = (raw - s0) / (s1 - s0).max(1e-10);
        v0 + t * (v1 - v0)
    }
}
```

### 5.9 Multi-Axis Verdict Routing

Each Verify verdict routes to appropriate axis calibrators based on evidence kind:

```rust
pub fn route_verdict(
    calibrators: &mut [AxisCalibrator; 5],
    verdict: &Verdict,
    original_score: &Score,
) {
    match verdict.evidence_kind {
        EvidenceKind::Compile => {
            calibrators[CONFIDENCE].update(original_score.confidence, verdict.passed);
            calibrators[QUALITY].update(original_score.quality, verdict.passed);
        }
        EvidenceKind::Test => {
            calibrators[CONFIDENCE].update(original_score.confidence, verdict.passed);
        }
        EvidenceKind::Relevance => {
            calibrators[RELEVANCE].update(original_score.relevance, verdict.passed);
        }
        EvidenceKind::Novelty => {
            calibrators[NOVELTY].update(original_score.novelty, verdict.passed);
        }
        EvidenceKind::Utility => {
            calibrators[UTILITY].update(original_score.utility, verdict.passed);
        }
        _ => {
            calibrators[CONFIDENCE].update(original_score.confidence, verdict.passed);
        }
    }
}
```

---

## 6. Demurrage Model

Signals decay via **demurrage** (Gesell 1916) — an attention-weighted holding cost replacing pure time-based Ebbinghaus. Every Signal has a `balance` that starts at 1.0 and decreases unless actively reinforced.

### 6.1 The Gesell-Shannon Derivation

Two premises:

**P1 (Gesell)**: Idle assets should bear a holding cost proportional to their value and the duration of idleness.

**P2 (Shannon)**: The information content of a message is `H = -log2(p)`. Redundant messages (high p) carry less information. Novel messages (low p) carry more.

Combining: the holding cost of a Signal is inversely related to its informational contribution:

```
effective_rate(signal) = base_rate / (1 + novelty(signal))
```

Where `novelty(signal) = 1 - max_similarity(signal, top_K_neighbors)` using HDC Hamming distance. A perfectly novel Signal pays `base_rate / 2`. A perfectly redundant Signal pays the full `base_rate`.

### 6.2 Rate Law

The balance evolves according to a differential equation with three terms:

```
d(balance)/dt = -r - beta * balance + reinforcement(t)
```

Discretized per tick:

```rust
/// Demurrage tick: charge holding cost and apply reinforcement.
///
/// Three terms:
///   1. Flat tax:     -r * dt           (constant drain, floor-aware)
///   2. Proportional: -beta * b * dt    (wealthy Signals pay more)
///   3. Reinforcement: +bonus * novelty (earned by use)
pub fn demurrage_tick(
    signal: &mut Signal,
    dt_days: f64,
    config: &DemurrageConfig,
    novelty: f64,
    reinforcement: Option<ReinforceKind>,
) {
    let tier = signal.tier;
    let charge_mult = tier.charge_multiplier() as f64;
    let flat_charge = config.flat_tax_per_day * dt_days * charge_mult;
    let prop_charge = config.exp_decay_per_day * signal.balance * dt_days * charge_mult;
    let total_charge = flat_charge + prop_charge;
    signal.balance -= total_charge;
    signal.demurrage_paid += total_charge;

    if let Some(kind) = reinforcement {
        let base_bonus = config.bonus_for(kind);
        let novelty_weight = novelty;
        let reinforce_mult = tier.reinforcement_multiplier() as f64;
        signal.balance += base_bonus * novelty_weight * reinforce_mult;
    }

    let floor = tier.cold_floor() as f64;
    if signal.balance < floor { signal.balance = floor; }
    signal.last_touched_at = Utc::now();
}
```

### 6.3 Reinforcement

Active usage restores balance, weighted by novelty (anti-hoarding mechanism):

```rust
pub enum ReinforceKind {
    Retrieved,      // returned in a query
    Cited,          // in another Signal's source[] lineage
    GatePassed,     // in context pack when gate passed
    Surprised,      // high prediction error (Shannon surprise as economic bonus)
    AgentQuoted,    // agent referenced in output
}
```

`balance += bonus(kind) * novelty(signal)` where `novelty = 1 - max_similarity` against top-K HDC neighbors. Citing a common Signal → small bump. Citing a rare Signal → large bump.

### 6.4 Steady-State Analysis

At steady state, `d(balance)/dt = 0`:

```
b_ss = (reinforcement_rate - r) / beta
```

Critical reinforcement rate: `reinforcement_critical = r`. Any Signal whose reinforcement rate exceeds the flat tax survives indefinitely. The exponential term `beta * b` prevents balance from growing without bound.

### 6.5 The Half-Life Correspondence

For a Signal receiving no reinforcement:

```
b(t) = (b_0 + r/beta) * exp(-beta * t) - r/beta
t_half = ln(2) / beta
```

For default `beta = 0.02/day`, half-life is `ln(2) / 0.02 = 34.7 days`. This is the connection to legacy `Decay::HalfLife`: demurrage with zero flat tax and zero reinforcement reduces to exponential decay. Ebbinghaus decay is the special case where `beta` varies with retrieval count (spaced repetition). **Demurrage subsumes both legacy models.**

### 6.6 Per-Kind Default Rates

| Kind | Flat tax (r) | Exp decay (beta) | Half-life (days) | Rationale |
|---|---|---|---|---|
| Core data (Text, Code) | 0.001 | 0.001 | 693 | Data artifacts are inherently stable |
| Verdict | 0.002 | 0.003 | 231 | Audit evidence, long-lived |
| Heuristic | 0.005 | 0.010 | 69 | Behavioral rules are durable once proven |
| Episode | 0.005 | 0.010 | 69 | Episodes feed learning loops |
| CausalLink | 0.005 | 0.008 | 87 | Cause-effect survives longer than episode |
| Insight | 0.01 | 0.02 | 35 | Observations need ongoing confirmation |
| AntiKnowledge | 0.01 | 0.02 | 35 | What-not-to-do stays relevant |
| StrategyFragment | 0.02 | 0.03 | 23 | Strategies go stale in evolving codebases |
| Warning | 0.10 | 0.20 | 3.5 | Danger signals are deliberately short-lived |

These are *unreinforced* half-lives. A heavily-cited heuristic can persist indefinitely despite having a 69-day base half-life.

### 6.7 Tiers

```rust
pub enum Tier {
    Transient,     // 0.1x multiplier — decays 10x faster
    Working,       // 0.5x — decays 2x faster
    Consolidated,  // 1.0x — base rate
    Persistent,    // 5.0x — decays 5x slower
}
```

Progression: Transient → Working (3+ gate passes) → Consolidated (5+ across distinct contexts) → Persistent (consortium approval or freeze).

### 6.8 The Phase Space

State of a Signal: `(balance, tier, novelty)`. Phase space with fixed points:

| Tier | Balance band | Charge mult | Reinforcement mult | Character |
|---|---|---|---|---|
| Transient | < 0.35 | 2.0x | 1.5x | Unstable: high charge pushes toward cold or Working |
| Working | 0.35 - 0.80 | 1.0x | 1.0x | Metastable: survives with moderate reinforcement |
| Consolidated | 0.80 - 1.20 | 0.5x | 0.75x | Stable: low charge, broad reinforcement keeps it here |
| Persistent | > 1.20 | 0.1x | 0.5x | Deeply stable: requires sustained contradiction to dislodge |

**Transient is an unstable equilibrium** — new knowledge either proves itself quickly or gets out of the way.

Tier transitions are phase transitions that change charge and reinforcement multipliers discontinuously:

```rust
pub fn check_tier_transition(
    signal: &mut Signal,
    stats: &UsageStats,
) -> Option<TierTransition> {
    if let Some(target) = check_promotion(signal, stats) {
        let from = signal.tier;
        signal.tier = target;
        return Some(TierTransition { from, to: target, direction: Direction::Promotion,
            trigger: stats.last_event.clone() });
    }
    if let Some(target) = check_demotion(signal, stats) {
        let from = signal.tier;
        signal.tier = target;
        return Some(TierTransition { from, to: target, direction: Direction::Demotion,
            trigger: stats.last_event.clone() });
    }
    None
}
```

### 6.9 Tier Progression as Markov Chain

```
                p_tw                p_wc                p_cp
  Transient ────────► Working ────────► Consolidated ────────► Persistent
      ^                  ^                    ^                    |
      |   p_wt           |   p_cw             |    p_pc            |
      ◄──────────────────◄────────────────────◄────────────────────
      |
      |  p_t_cold
      v
  Cold Storage  ──(thaw)──► Transient (restart)
```

Default transition probabilities per day:

| Transition | Rate | Notes |
|---|---|---|
| Transient → Working | 0.15 | 3+ gate passes |
| Transient → Cold | 0.10 | No reinforcement |
| Working → Transient | 0.05 | Balance drops |
| Working → Consolidated | 0.08 | 5+ distinct contexts |
| Consolidated → Working | 0.02 | Contradiction |
| Consolidated → Persistent | 0.03 | 10+ uses, no contradictions |
| Persistent → Consolidated | 0.005 | Sustained contradiction |
| Cold → Transient (thaw) | 0.01 | Query hit or explicit |

**Stationary distribution** (long-run equilibrium): ~15% Transient, ~35% Working, ~30% Consolidated, ~10% Persistent, ~10% Cold. The chain is **ergodic** (every tier reachable from every other via Cold/Thaw), guaranteeing a unique stationary distribution and convergence regardless of initial conditions.

### 6.10 The VCG Attention Auction as Live-Economy Dual

Demurrage governs *who stays in memory*; the VCG auction governs *who enters the context window*:

| Property | Demurrage (memory ledger) | VCG Auction (loop ledger) |
|---|---|---|
| What is spent | Balance (holding cost) | Attention tokens (compute cost) |
| When charged | Between loops (idle tax) | During loops (active spend) |
| Who pays | The Signal (for existing) | The loop (for using) |
| Incentive | Signals must earn reinf. to survive | Signals must bid high to get selected |

The reinforcement loop closes: Signals that win auctions get reinforced → keeps balance high → keeps them available for future auctions. Novelty weighting prevents monopolization.

```rust
pub fn auction_and_reinforce(
    candidates: &mut [Signal],
    auction: &AttentionAuction,
    budget: &mut AttentionToken,
) -> AuctionOutcome {
    let mut bids: Vec<AttentionBid> = candidates.iter()
        .map(|s| AttentionBid {
            signal_ref: s.ref_(),
            bid_value: s.score.effective(),
            estimated_cost: estimate_context_cost(s),
            priority: classify_priority(s),
        }).collect();
    let outcome = auction.run(&mut bids, budget);
    for winner in &outcome.winners {
        if let Some(signal) = candidates.iter_mut()
            .find(|s| s.ref_() == winner.signal_ref) {
            let novelty = compute_novelty(signal);
            demurrage_reinforce(signal, ReinforceKind::Retrieved, novelty);
        }
    }
    outcome
}
```

### 6.11 Cold Storage and Thaw

When balance drops below `COLD_THRESHOLD` (default 0.01), the Signal enters cold storage. Body moves to slower storage; content hash stays valid; HDC fingerprint stays in warm index for thaw discovery; lineage preserved. **Thaw** restores balance to a starter value (default 0.3) at Tier::Transient and publishes `knowledge.thawed` on Bus.

Thaw triggers: (1) query similarity hit, (2) explicit request by hash, (3) lineage cross-reference from new Signal, (4) consolidation gap discovery during Dreams.

Frozen Signals skip demurrage entirely — they are bedrock knowledge.

### 6.12 Why Demurrage Instead of Ebbinghaus

Ebbinghaus is the special case where no interactions occur. Demurrage is strictly more expressive:
- **Self-trimming**: duplicates get fewer citations → faster decay. Unique insights get cited → stay warm.
- **Usage-based**: a Signal retrieved daily stays fresh; one never accessed fades.
- **Compounding**: the retrieval → gate-pass → reinforcement loop is superlinear.
- **Observable**: balance is a first-class field — visible in TUI, queryable via API.
- **Economically grounded**: Gesell's insight is that idle value is a social cost; same applies to idle knowledge.

### 6.13 Demurrage Telemetry

```rust
pub struct DemurrageTelemetry {
    pub timestamp: DateTime<Utc>,
    pub balance_histogram: Vec<(f64, usize)>,
    pub tier_counts: BTreeMap<Tier, usize>,
    pub total_charged: f64,
    pub total_reinforced: f64,
    pub net_flow: f64,                   // reinforced - charged (positive = healthy)
    pub promotions: Vec<TierTransition>,
    pub demotions: Vec<TierTransition>,
    pub freezes: usize,
    pub thaws: usize,
    pub hoarding_index: f64,             // fraction with balance > 2.0
    pub starvation_index: f64,           // fraction with balance < 0.1
    pub reinforcement_by_kind: BTreeMap<String, f64>,
    pub attention_leaderboard: Vec<(SignalRef, f64)>,
}
```

---

## 7. Signal Algebra

Signal and Pulse form a **semiring** under two operations on HDC vectors:

### 7.1 Bind — The Multiplicative Operation (XOR)

```rust
/// Bind: XOR in HDC space.
/// Properties:
///   a * a = 0          (self-inverse)
///   a * b = b * a      (commutative)
///   (a * b) * c = a * (b * c)  (associative)
///   a * 0 = a          (identity)
/// Abelian group under XOR.
pub fn bind(a: &HdcVector, b: &HdcVector) -> HdcVector {
    HdcVector::xor(a, b)
}

/// Unbind: recover b from (a * b) given a.
/// Because XOR is self-inverse: a * (a * b) = b.
pub fn unbind(key: &HdcVector, bound: &HdcVector) -> HdcVector {
    HdcVector::xor(key, bound)
}
```

**Bind as role-filler encoding**: Encode structured records as single vectors. Each axis is bound to a role vector, then all role-filler pairs are bundled into a single record vector.

**Bind for causal association**: `create_causal_link(cause, effect)` stores `bind(cause.hdc, effect.hdc)` as a `Kind::CausalLink` Signal. Later, given a new error similar to `cause`, unbind to recover effect-like vectors pointing toward potential fixes.

### 7.2 Bundle — The Additive Operation (Majority Vote)

```rust
/// Bundle: majority vote across bit positions.
/// Properties:
///   a + b = b + a                 (commutative)
///   (a + b) + c ~ a + (b + c)    (approximately associative)
///   a + a = a                     (idempotent for odd counts)
/// Commutative semigroup (no inverse, no true identity).
pub fn bundle(vectors: &[HdcVector]) -> HdcVector {
    if vectors.is_empty() { return HdcVector::zero(); }
    let n = vectors.len();
    let threshold = n / 2;
    let mut result = HdcVector::zero();
    for bit_pos in 0..HDC_DIMENSION {
        let ones: usize = vectors.iter().filter(|v| v.get_bit(bit_pos)).count();
        if ones > threshold {
            result.set_bit(bit_pos, true);
        } else if ones == threshold && n % 2 == 0 {
            result.set_bit(bit_pos, vectors[0].get_bit(bit_pos));
        }
    }
    result
}
```

**Bundle as Compound Kind**: When constructing `Kind::compound([GateVerdict, TestResult])`, the HDC fingerprint is the bundle of kind-specific role vectors. The compound Signal appears in similarity searches for *either* constituent kind.

**Bundle as cluster centroid**: During consolidation, clusters of related Signals are bundled into centroids — not a single best Signal, but the consensus vector of all contributors.

**Bundle noise at scale**: SNR degrades as `sqrt(N)`. For N=100 at D=10,240: ~0.5% error rate. For N=1000: ~5%. Practical limit: bundle no more than ~200 vectors before re-encoding via hierarchical bundle tree:

```rust
const CHUNK_SIZE: usize = 64;
pub fn hierarchical_bundle(vectors: &[HdcVector]) -> HdcVector {
    if vectors.len() <= CHUNK_SIZE { return bundle(vectors); }
    let chunks: Vec<HdcVector> = vectors.chunks(CHUNK_SIZE)
        .map(|chunk| bundle(chunk)).collect();
    hierarchical_bundle(&chunks)
}
```

### 7.3 Permute — Temporal Ordering (Cyclic Rotation)

```rust
/// Permute: cyclic bit rotation for positional encoding.
/// Properties:
///   permute(a, 0) = a
///   permute(permute(a, i), j) = permute(a, i+j)  (group under addition mod D)
///   similarity(a, permute(a, k)) ~ 0.5 for k > 0  (near-orthogonal)
pub fn permute(v: &HdcVector, positions: usize) -> HdcVector {
    v.rotate_left(positions % HDC_DIMENSION)
}

/// Encode an ordered sequence of Signals (preserves positional information).
pub fn encode_sequence(signals: &[Signal]) -> HdcVector {
    let positioned: Vec<HdcVector> = signals.iter().enumerate()
        .map(|(i, s)| permute(&s.hdc_fingerprint, i)).collect();
    bundle(&positioned)
}
```

### 7.4 Semiring Laws Summary

| Law | Bind (*) | Bundle (+) |
|---|---|---|
| Closure | HdcVector → HdcVector | HdcVector → HdcVector |
| Associative | Exact | Approximate (noise accumulates) |
| Commutative | Yes | Yes |
| Identity | Zero vector (all 0s) | None (semigroup) |
| Inverse | Self-inverse: a * a = 0 | None (lossy) |
| Distributive | a * (b + c) ~ (a * b) + (a * c) | Approximate |

At 10,240 bits, approximation error is ~1% per operation (`1/sqrt(D)`). The semiring laws hold *in expectation*, not bit-for-bit.

---

## 8. Content Addressing

Signals are content-addressed via SHA-256:

```rust
impl Signal {
    pub fn compute_hash(payload: &Value) -> ContentHash {
        let canonical = serde_json::to_vec(payload).expect("serializable");
        ContentHash(sha2::Sha256::digest(&canonical).into())
    }
}
```

Enables: deduplication, integrity verification, lineage chain validation, semantic caching (5x cost reduction via content-addressed reuse across Flows), and on-chain commitments (hash on-chain, content off-chain).

---

## 9. HDC Fingerprint

Every Signal carries a 10,240-bit binary HDC vector (Kanerva 2009) for similarity search and cross-domain pattern discovery.

### Encoding

Structured information enters a single vector through role-filler binding:

```rust
pub fn encode_signal(signal: &Signal) -> HdcVector {
    let pairs = vec![
        ("kind", signal.kind.to_string()),
        ("tags", signal.tags.join(",")),
        ("author", signal.author.to_string()),
        // ... kind-specific fields
    ];
    HdcVector::encode_structured(&pairs)
}
```

Deterministic across deployments via BLAKE3-seeded `WordMemory`. Encoder version tracked to prevent drift.

### Operations

| Operation | What | Cost | Reference |
|---|---|---|---|
| **Bind** (XOR) | Role-filler binding | O(n) | Rachkovskij 2001 |
| **Bundle** (majority) | Consensus: similar to all inputs | O(n*k) | Kanerva 2009 |
| **Permute** (rotation) | Positional encoding | O(n) | Plate 2003 |
| **Similarity** (Hamming) | Overlap via POPCNT | <1 us | Hardware |
| **Resonate** | Factorize: recover constituents | O(n*k*iter) | Frady et al. 2020 |

### Cross-domain resonance

When Signals from different domains have similar HDC fingerprints, they share structural properties despite surface differences. Retrieval gives cross-domain matches a **15% bonus** (additive when domains differ).

### Why HDC instead of float embeddings

| Property | HDC (10,240-bit binary) | Float (1536-d float32) |
|---|---|---|
| Size per vector | 1,280 bytes | 6,144 bytes |
| Similarity cost | XOR + POPCNT (~1 ns) | Dot product (hundreds FLOPs) |
| Compositionality | Native (bind/bundle/permute/resonate) | Requires learned operations |
| Privacy | Non-invertible after PP-HDC | Invertible via decoder |
| Determinism | Identical seeds → identical vectors | Depends on model version |

At 10,240 bits, **800K fingerprints fit in 1 GB RAM**; brute-force SIMD comparison is **<1 ms** for the full set. No external vector store needed.

---

## 10. Provenance and Taint

### 10.1 The Taint Lattice

Every Signal carries a taint classification forming a security lattice for information flow control (IFC):

```
                    Propagated
                   /          \
        LlmGenerated      ExternalFetch
                   \          /
                  UserInput
                      |
                    Clean
```

Taint propagation is join (least upper bound) in this lattice:

```rust
#[non_exhaustive]
pub enum Taint {
    Clean,
    UserInput { detail: String },
    ExternalFetch { url: Option<String>, detail: String },
    LlmGenerated { model: String, detail: String },
    Propagated { max_upstream: Box<Taint>, inherited_from: Vec<ContentHash> },
    StaleData { threshold_ms: i64 },
    UserFlagged { reason: String },
    ToolFailure { tool: String, detail: String },
    Custom(String),
}

impl Taint {
    /// Lattice join: least upper bound of two taint levels.
    pub fn join(a: &Taint, b: &Taint) -> Taint {
        if a.flows_to(b) { b.clone() }
        else if b.flows_to(a) { a.clone() }
        else {
            Taint::Propagated {
                max_upstream: Box::new(if a.severity() >= b.severity() {
                    a.clone() } else { b.clone() }),
                inherited_from: Vec::new(),
            }
        }
    }

    pub fn join_all(taints: &[Taint]) -> Taint {
        taints.iter().fold(Taint::Clean, |acc, t| Taint::join(&acc, t))
    }
}
```

### 10.2 Propagation Rules

1. **Compose preserves taint**: output taint = `Taint::join_all(inputs.map(|s| s.provenance.taint))`.
2. **Derivation preserves taint**: derived Signals inherit parent taint unless declassified.
3. **Taint only increases**: information flows upward in the lattice.
4. **Verify does not clear taint**: validation != provenance. Taint is a historical fact.

### 10.3 Declassification

Taint can only decrease through explicit human-initiated declassification recorded in the audit trail via a Custody record.

### 10.4 Action-Time Taint Gate

Tainted data is allowed into the system (blocking at intake would be censorship). Taint gates *actions*: the riskier the action and the more tainted the context, the stronger the authorization required:

```rust
pub fn taint_gate(action_risk: ActionRisk, context_taint: &Taint) -> AuthorizationRequirement {
    match (action_risk, context_taint.severity()) {
        (ActionRisk::Low, _) => AuthorizationRequirement::None,
        (ActionRisk::Medium, 0) => AuthorizationRequirement::None,
        (ActionRisk::Medium, 1..=2) => AuthorizationRequirement::SessionApproval,
        (ActionRisk::Medium, 3..) => AuthorizationRequirement::HumanConfirmation,
        (ActionRisk::High, 0) => AuthorizationRequirement::SessionApproval,
        (ActionRisk::High, 1..) => AuthorizationRequirement::HumanConfirmation,
        (ActionRisk::Critical, _) => AuthorizationRequirement::HumanConfirmationWithAttestation,
    }
}
```

### 10.5 Custody Records as Dependent Types

Custody is a witness that a specific action was authorized — a dependent type `Custody<A>` depending on action `A`:

```rust
pub struct Custody {
    pub action: ActionId,
    pub principal: Principal,
    pub when: DateTime<Utc>,
    pub authorized: AuthzEvidence,
    pub why_heuristics: Vec<SignalRef>,
    pub why_claims: Vec<SignalRef>,
    pub simulation: Option<SignalRef>,
    pub gates_passed: Vec<SignalRef>,
    pub result: Option<SignalRef>,
    pub witness: Option<ExternalWitness>,
}

pub enum AuthzEvidence {
    RoleGrant { role: String, scope: String },
    HumanConfirmation { confirmer: String, channel: String },
    Escalation { original_denial: SignalRef, override_by: String },
    SessionApproval { session_id: String },
    Automatic { policy: String },
}
```

**CustodyGatedStore**: Certain Store operations are gated on custody. Privileged kinds (Declassification, Deployment, ExternalWrite, NetworkEgress, FileDelete) require a valid Custody witness for `Store.put()`.

### 10.6 Attestation Levels

Attestation is orthogonal to taint. Taint tracks **trust** (where data came from); attestation tracks **integrity** (who signed):

```rust
pub enum AttestationLevel {
    LocalAgent,     // ephemeral session key — low friction, low assurance
    OrgRole,        // human-held org key — medium friction, medium assurance
    ChainWitness,   // on-chain independent verifier — high friction, high assurance
}
```

### 10.7 Cross-Space Taint

When Signals move between Spaces, taint is re-evaluated at the boundary: `import_taint = join(original_taint, space_trust_level)`. Trust domains (clusters of mutually-trusted Spaces) allow Signals to flow freely within but re-taint at boundaries.

### 10.8 The Provenance Struct

```rust
pub struct Provenance {
    pub author: Author,
    pub trust: f64,                              // snapshot at emission time
    pub taint: Taint,
    pub session: Option<String>,
    pub source_files: Vec<SourceFileRange>,
    pub generation: Option<GenerationProvenance>,
    pub web_fetch: Option<WebFetchProvenance>,
    pub citations: Vec<Citation>,
}

pub enum Author {
    User(String), Agent(AgentId), Gate(String),
    System, External(String), Wallet(Address),
}

pub struct GenerationProvenance {
    pub model: String,
    pub prompt_hash: ContentHash,
    pub temperature: f64,
    pub seed: Option<u64>,
    pub tokens_used: usize,
}
```

---

## 11. Lineage

### 11.1 The Lineage DAG as a Free Category

The `source: Vec<SignalRef>` field defines edges in a DAG forming a **free category**:

- **Objects**: Signals (identified by ContentHash)
- **Morphisms**: lineage edges (A in B's `source` means "A contributed to B")
- **Composition**: transitive closure
- **Identity**: each Signal contributed to itself

```rust
pub async fn ancestry(store: &dyn Store, target: &Signal) -> Vec<Signal> {
    let mut visited = HashSet::new();
    let mut queue: VecDeque<SignalRef> = target.source.iter().cloned().collect();
    let mut ancestors = Vec::new();
    while let Some(ref_) = queue.pop_front() {
        if !visited.insert(ref_.content_hash) { continue; }
        if let Some(ancestor) = store.get(&ref_.id).await? {
            queue.extend(ancestor.source.iter().cloned());
            ancestors.push(ancestor);
        }
    }
    ancestors
}

/// Out-degree in the reversed DAG. High = this Signal was generative.
pub async fn autocatalytic_score(store: &dyn Store, signal: &Signal) -> usize {
    store.query(StoreQuery { lineage_contains: Some(signal.content_hash),
        ..Default::default() }).await?.len()
}
```

### 11.2 Lineage Laws

1. **Acyclicity**: No Signal in its own transitive ancestry. Enforced structurally — ContentHash computed before storage.
2. **Hash stability**: References must be resolvable. Dangling references (pruned parent) emit warnings.
3. **Monotonic growth**: Signals are append-only; edges never removed. Cold storage preserves hashes.
4. **Bounded fan-in**: Max 32 parents per Signal. Deeper derivations use intermediate consolidation.

### 11.3 Scaling

At 1M Signals with average fan-in of 3: 3M edges. Practical mitigation: depth-limited traversal (default 100 ancestors, 1000 max):

```rust
pub struct LineageQuery {
    pub target: SignalRef,
    pub max_depth: usize,       // default 100
    pub max_ancestors: usize,   // default 1000
    pub kind_filter: Option<Kind>,
}
```

---

## 12. Graduation and Projection as Functors

Graduation and projection are the only bridges between Pulse and Signal. Algebraically, they are functors between categories:

### 12.1 Graduation — Enrichment Functor (F: Pul → Sig)

```rust
/// Graduation preserves { kind, body, emitted_at_ms → created_at }.
/// Adds: content_hash, hdc_fingerprint, score, balance, lineage, provenance, tier.
impl Pulse {
    pub fn graduate(
        &self, provenance: Provenance, initial_balance: f64,
        score: Score, tags: Vec<String>,
    ) -> Signal {
        Signal {
            id: SignalId::new(),
            content_hash: Signal::compute_hash(&self.body),
            kind: self.kind.clone(),
            payload: self.body.clone(),
            score, confidence: score.confidence,
            balance: initial_balance, demurrage_paid: 0.0,
            last_touched_at: Utc::now(), tier: Tier::Transient,
            created_at: DateTime::from_timestamp_millis(self.emitted_at_ms),
            source: self.lineage_hint.iter().map(|h| SignalRef::from_hash(*h)).collect(),
            provenance,
            hdc_fingerprint: encode_signal_from_parts(&self.kind, &self.body),
            author: Author::from_pulse_source(&self.source),
            tags,
            schema: TypeSchema::infer(&self.body),
        }
    }
}
```

### 12.2 Projection — Forgetful Functor (G: Sig → Pul)

```rust
/// Projection preserves { kind, body, created_at → emitted_at_ms }.
/// Forgets: content_hash, hdc_fingerprint, score, balance, tier, full lineage, provenance.
impl Signal {
    pub fn to_pulse(&self, topic: Topic, seq: u64) -> Pulse {
        Pulse {
            seq, topic,
            kind: self.kind.clone(),
            body: self.payload.clone(),
            emitted_at_ms: self.created_at.timestamp_millis(),
            source: PulseSource::from_author(&self.author),
            lineage_hint: Some(self.content_hash),
            trace_id: None,
        }
    }
}
```

### 12.3 The Round-Trip Property

`project(graduate(pulse))` preserves `{ kind, body, emitted_at_ms }`.
`graduate(project(signal))` produces a *different* Signal (new content_hash, new id). The projection functor is lossy; the graduation functor adds information not recoverable from the Pulse alone. This asymmetry is intentional.

### 12.4 The Store-Bus Adjunction

Graduation and Projection form an **adjunction** `F -| G`:

```
Hom_Bus(Pulses, G(Signals))  ~=  Hom_Store(F(Pulses), Signals)
```

Subscribing to store-write notifications on Bus is equivalent to querying Store for recently graduated Signals. The **unit** `eta: P → G(F(P))` produces a "cleaned" Pulse with SignalRef and content hash. The **counit** `eps: F(G(S)) → S` is idempotent on content-addressed Signals.

---

## 13. Bus — Ephemeral Transport

The **Bus** is the ephemeral transport fabric — a kernel-level pub/sub system alongside Store.

```rust
#[async_trait]
pub trait Bus: Send + Sync {
    async fn publish(&self, pulse: Pulse) -> Result<u64>;
    fn subscribe(&self, filter: TopicFilter) -> PulseStream;
    async fn replay_since(&self, since: u64, filter: &TopicFilter) -> Result<Vec<Pulse>>;
    async fn current_seq(&self) -> Result<u64>;
    fn ring_capacity(&self) -> usize;
}

pub enum TopicFilter {
    Exact(Topic),
    Glob(String),           // e.g., "agent:*:heartbeat"
    AnyOf(Vec<Topic>),
    All,
    And(Box<TopicFilter>, Box<TopicFilter>),
    Or(Box<TopicFilter>, Box<TopicFilter>),
    Not(Box<TopicFilter>),
}
```

Bus is **broadcast**: every subscriber sees every matching Pulse. No queuing or redelivery. For critical data, graduate to Signal.

### Backpressure

| Strategy | Used for | Behavior |
|---|---|---|
| Coalesce | Heartbeats | Buffer, send latest per interval |
| Drop-oldest | Streaming output | Ring buffer, slow consumers miss old |
| Lossless | Gate results | Queue with flow control |
| Sample | Feed data | Every Nth update |

### Backends

| Backend | Scope | Status |
|---|---|---|
| `BroadcastBus` (`tokio::sync::broadcast`) | In-process | Ships immediately |
| `MemoryBus` | Testing | Ships immediately |
| `NatsBus` / `KafkaBus` | Multi-process | Phase 2 |
| `ChainBus` | On-chain events | Phase 2+ |

### Why Bus is kernel-level

The event bus already existed but was architecturally invisible — no trait, no doc chapter. This caused the `roko-conductor → roko-learn` layer violation. With Bus as L0, both subsystems subscribe to `gate.verdict.emitted` independently — no compile-time coupling.

---

## 14. Store — Persisted Storage

```rust
pub trait Store: Cell {
    async fn put(&self, signal: Signal) -> Result<SignalRef>;
    async fn get(&self, id: &SignalId) -> Result<Option<Signal>>;
    async fn query(&self, query: StoreQuery) -> Result<Vec<Signal>>;
    async fn query_similar(
        &self, fp: &HdcVector, radius: f32, limit: usize,
    ) -> Result<Vec<(SignalRef, f32)>>;
    async fn prune(&self, threshold: f64) -> Result<PruneReport>;
}
```

`query_similar` is native HDC similarity over stored Signals. No external vector store. At 10,240 bits and 800K entries, brute-force SIMD is <1 ms. `query_similar` respects demurrage — Signals below prune threshold are not returned.

### Storage layout

```
.roko/
├── signals.jsonl          # primary Signal log (append-only)
├── neuro/
│   └── knowledge.jsonl    # knowledge Signals (demurrage, tiers)
├── episodes.jsonl         # episode Signals
├── runs/<run-id>/
│   ├── artifacts/
│   └── events.jsonl       # graduated Pulse snapshots
└── learn/
    ├── reflexes.jsonl     # promoted T0 reflex Signals
    └── efficiency.jsonl
```

---

## 15. Store-Bus Duality

Store and Bus are dual — two views of the same information flow:

| Store (pull) | Bus (push) |
|---|---|
| Consumer initiates (`query`) | Producer initiates (`publish`) |
| Durable (survives restart) | Ephemeral (bounded ring) |
| Identity is content hash | Identity is sequence number |
| Supports similarity (`query_similar`) | Supports topic routing (`TopicFilter`) |
| Retention is decay-based (demurrage) | Retention is capacity-based (ring eviction) |
| Medium: Signal | Medium: Pulse |

### Consistency Guarantees

1. **Store-first**: graduation writes to Store before publishing downstream Pulses. If Store write fails, no Pulse emitted.
2. **Projection-best-effort**: the projection Pulse after a Store write is best-effort. Signal is safe in Store regardless.
3. **Idempotent graduation**: graduating the same Pulse twice produces the same SignalRef.
4. **Ring eviction is not data loss**: graduated content is in Store. Un-graduated content was deemed ephemeral.

### Catch-Up Strategy

```rust
async fn catch_up(checkpoint_seq: u64, checkpoint_time: i64,
    bus: &dyn Bus, store: &dyn Store, filter: &TopicFilter) -> Result<CatchUpResult> {
    let bus_pulses = bus.replay_since(checkpoint_seq, filter).await?;
    let store_signals = store.query(StoreQuery {
        since_ms: Some(checkpoint_time), ..Default::default() }).await?;
    Ok(merge_bus_and_store(bus_pulses, store_signals))
}
```

---

## 16. AntiKnowledge

When a previously trusted Signal is proven wrong, an **AntiKnowledge** Signal actively repels future Signals in the same HDC region. Popper's falsificationism applied to learned rules.

```rust
const ANTI_KNOWLEDGE_WARN_THRESHOLD: f64 = 0.5;      // log warning
const ANTI_KNOWLEDGE_DISCOUNT_THRESHOLD: f64 = 0.7;  // halve initial balance
const ANTI_KNOWLEDGE_REJECT_THRESHOLD: f64 = 0.9;    // reject outright
```

AntiKnowledge itself decays via demurrage (30-day effective rate). Old mistakes eventually stop blocking new discoveries.

---

## 17. Signal Lifecycle

```
Created (by Cell or external source)
    |
    +-- Pulse path --> Bus topic --> consumed by subscribers
    |                                    |
    |                       graduate() if graduation policy says yes
    |                                    v
    +-- Signal path --> Store.put() --> scored --> routed --> composed
                             |
                             +-- retrieved --> balance up (reinforcement)
                             +-- gate passed --> balance up, tier up
                             +-- challenged --> balance down, tier down
                             +-- demurrage --> balance down over time
                             +-- cold --> balance < 0.01 --> archive
                             +-- frozen --> permanent (skip demurrage)
```

---

## 18. Feedback Loops

1. **Score -> Bind -> Store -> Score**: Causal link Signals created via bind start with utility 0. As verified, utility increases. High-utility links survive demurrage; low-utility ones decay.

2. **Bundle -> Consolidation -> Bundle**: During delta-speed consolidation, clusters are bundled into centroids that participate in future bundles. Hierarchy deepens over time, compressing knowledge.

3. **Compound Kind -> Filter -> Compound Kind**: As consumers learn to filter by Compound kinds, frequent compounds become first-class patterns.

4. **Use -> Reinforce -> Survive -> Use**: Useful Signals get reinforced → stay warm → available for future use. Checked by novelty weighting.

5. **Novelty -> Bonus -> Survive -> Reduce Novelty**: Novel Signals get larger bonuses, but as similar Signals accumulate, novelty decreases, bonuses shrink. Only the most distinctive Signal in a cluster survives.

6. **Contradiction -> Demote -> Low Balance -> Freeze**: The knowledge immune system — bad knowledge is actively expelled.

7. **Thaw -> Transient -> Prove -> Promote**: Frozen knowledge restarts at Transient, must re-earn its place. Prevents zombie knowledge.

8. **Score -> Compose -> Act -> Verify -> Calibrate -> Score**: Core calibration loop. Convergence depends on Verify informativeness.

9. **Temperature -> ECE -> Temperature**: Fixed-point iteration converging when temperature stabilizes.

---

## 19. Citations

| Concept | Citation |
|---|---|
| Demurrage (carrying charge) | Gesell, S. (1916). *The Natural Economic Order*. |
| Hyperdimensional computing | Kanerva, P. (2009). Hyperdimensional computing: An introduction. *Cognitive Computation*, 1(2), 139-159. |
| VSA binding | Rachkovskij, D. A. (2001). Representation and processing of structures with binary sparse distributed codes. *Knowledge-Based Systems*. |
| VSA survey | Levy, S. D., & Gayler, R. W. (2008). Vector symbolic architectures. *Artificial Intelligence*. |
| Permute encoding | Plate, T. A. (2003). *Holographic Reduced Representation*. CSLI. |
| Resonator networks | Frady, E. P., et al. (2020). Resonator networks. *Neural Computation*, 32. |
| Biological precedent | Olshausen, B. A., & Field, D. J. (1996). Sparse coding. *Nature*, 381. |
| Temperature scaling | Guo, C., et al. (2017). On calibration of modern neural networks. *ICML*. |
| Expected Calibration Error | Naeini, M. P., et al. (2015). Obtaining well calibrated probabilities. *AAAI*. |
| Isotonic regression (PAVA) | de Leeuw, J., Hornik, K., & Mair, P. (2009). Isotone optimization in R. *J. Stat. Software*. |
| Information flow control | Denning, D. E. (1976). A lattice model of secure information flow. *CACM*, 19(5). |
| Active inference / predict-publish-correct | Friston, K. (2006). A free energy principle for the brain. *J. Physiology-Paris*, 100(1-3). |

---

## 20. Acceptance Criteria

| Criterion | Verification |
|---|---|
| Signal struct compiles with `balance`, `demurrage_paid`, `last_touched_at`, `tier` | Compile check |
| Pulse struct compiles with `seq`, `topic`, `kind`, `body`, `source`, `lineage_hint` | Compile check |
| Content hash deterministic: same payload → same hash | Unit test |
| Demurrage: balance decreases over time, increases on reinforcement | Unit test with mock clock |
| Novelty-weighted reinforcement: rare Signals get larger bonus | Unit test (two Signals at different HDC distances) |
| Tier progression: 3 gate-passes promote Transient → Working | Integration test |
| Steady-state balance: `b_ss = (reinf_rate - r) / beta` | Unit test |
| Half-life correspondence: `t_half = ln(2) / beta` for zero-reinforcement | Unit test |
| Markov chain ergodicity: every tier reachable from every other | Property test |
| AntiKnowledge: >0.9 HDC similarity rejected | Unit test |
| Bus: Pulse published to topic received by subscriber | Integration test |
| Bus replay: reconnecting subscriber receives missed Pulses within ring capacity | Integration test |
| Graduation: `Pulse.graduate()` produces valid Signal with provenance | Unit test |
| Projection: `Signal.to_pulse()` produces valid Pulse | Unit test |
| Round-trip: `project(graduate(pulse))` preserves kind, body, timestamp | Unit test |
| Adjunction: graduation + projection notification equivalent to Store query | Integration test |
| Store round-trip: put + get returns identical Signal | Integration test |
| `Store.query_similar`: returns Signals ranked by HDC similarity | Integration test |
| Cold threshold: balance < 0.01 triggers archive | Unit test |
| Thaw: frozen Signal restored to Transient with starter balance | Integration test |
| Heuristic kind: `when` + `then` + `falsifier` + `calibration` fields present | Compile check |
| Compound kind: lattice join is idempotent, commutative, associative | Unit test |
| Lineage walk: `source[]` recursion produces correct DAG | Integration test |
| HDC fingerprint determinism: same inputs → same fingerprint | Unit test |
| Bind self-inverse: `bind(a, bind(a, b)) = b` | Unit test |
| Bundle idempotent: `bundle([a, a, a]) ~ a` | Property test (within noise) |
| Hierarchical bundle: noise stays below 1% for 1000 vectors | Property test |
| Bus `TopicFilter::Glob` matches expected topics | Unit test |
| Graduation policy: `gate.verdict.emitted` graduates, `heartbeat.tick` does not | Integration test |
| Taint lattice join: `join(Clean, LlmGenerated) = LlmGenerated` | Unit test |
| Taint propagation: Compose output taint = join of all input taints | Unit test |
| Action-time taint gate: high-risk + tainted requires human confirmation | Unit test |
| Temperature scaling: ECE decreases after calibration | Integration test |
| Beta-Binomial: posterior converges with increasing observations | Unit test |
| Precision-weighted aggregation: low-uncertainty scorer dominates | Unit test |
| Score effective formula: confidence=0 → effective=0 | Unit test |
| Store-Bus consistency: Store-first guarantee holds under Bus failure | Integration test |
