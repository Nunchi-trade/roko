# Taint-Aware Ingestion and Data Flow Control

> **Layer**: L2 Scaffold (context engineering), L3 Harness (ingestion gates), Cross-cut (Neuro)
>
> **Crate**: `roko-agent` (ScrubPolicy), target: `roko-neuro` (taint labels), `roko-compose` (context assembly)
>
> **Synapse traits**: `Gate` (verify taint compliance before data flows to sinks), `Scorer` (rate trust level of ingested data)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), Neuro knowledge store documentation


> **Implementation**: Specified

---

## The Problem: Untrusted Data in Agent Context

An autonomous agent ingests data from many sources: tool results, LLM outputs, knowledge store entries, operator inputs, external APIs, and peer agent messages. Not all of these sources are equally trustworthy. A prompt injection attack can cause the LLM to include sensitive data in its output. If that output is logged, synced to the Agent Mesh (formerly "Styx"), or broadcast, the sensitive data is exposed.

The fundamental challenge is distinguishing data from instructions. CaMeL (Debenedetti et al., 2025) addresses this by separating control flow from data flow. Taint tracking generalizes this principle: every piece of data carries labels describing its sensitivity and trust level, and these labels are checked before data crosses any trust boundary.

---

## Taint Labels

Every piece of sensitive data carries taint labels that propagate through the system. Before data enters a sink (LLM context, audit log, mesh relay, event fabric), the taint checker verifies that no forbidden label reaches that sink.

### Label Taxonomy

```rust
/// Taint labels: what kind of sensitive data is this?
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TaintLabel {
    /// Wallet private key material.
    /// NEVER leaves the local process. Not even to the LLM context.
    WalletSecret,

    /// Owner API keys, service credentials.
    /// Never enters LLM context or mesh relay.
    OwnerSecret,

    /// Proprietary strategy parameters (alpha).
    /// Never enters the mesh relay. May enter collective (same owner's group).
    StrategyConfidential,

    /// Owner personal data (email, wallet addresses).
    /// Never enters mesh relay without anonymization.
    UserPII,

    /// Data from untrusted external sources (mesh entries, marketplace).
    /// Must be validated before use in configuration or tool parameters.
    UntrustedExternal,
}
```

### TaintedString Type

```rust
/// A taint-tracked string. The sensitive content is wrapped in
/// Zeroizing<String> (from the zeroize crate) which automatically
/// overwrites the memory on drop — preventing key recovery from
/// memory dumps.
pub struct TaintedString {
    value: zeroize::Zeroizing<String>,
    labels: std::collections::HashSet<TaintLabel>,
}
```

The `Zeroizing<String>` wrapper is critical for key material. When a `TaintedString` containing a private key goes out of scope, the memory is overwritten with zeros before being freed. This prevents key recovery from memory dumps, core files, or swap space — a real threat when agents run in shared cloud environments.

### Data Sinks

```rust
/// Data sinks: where can data flow?
#[derive(Clone, Copy, Debug)]
pub enum DataSink {
    /// The LLM's input (system prompt + messages).
    LlmContext,
    /// Content-addressed audit trail.
    AuditLog,
    /// Agent Mesh shared knowledge relay.
    MeshRelay,
    /// Broadcast to surfaces (TUI, web, notifications).
    EventFabric,
    /// Peer-to-peer collective sync.
    CollectivePeer,
    /// Local Neuro store (everything allowed).
    LocalNeuro,
}
```

### Flow Rules Matrix

| Label | LlmContext | AuditLog | MeshRelay | EventFabric | CollectivePeer | LocalNeuro |
|-------|-----------|----------|-----------|-------------|----------------|------------|
| WalletSecret | BLOCKED | BLOCKED | BLOCKED | BLOCKED | BLOCKED | Allowed |
| OwnerSecret | BLOCKED | BLOCKED | BLOCKED | BLOCKED | Allowed | Allowed |
| StrategyConfidential | Allowed | Allowed | BLOCKED | Allowed | Allowed | Allowed |
| UserPII | Allowed | Allowed | BLOCKED | BLOCKED | Allowed | Allowed |
| UntrustedExternal | Allowed | Allowed | Allowed | Allowed | Allowed | Allowed |

```rust
impl TaintedString {
    /// Can this data flow to the specified sink?
    /// Returns false if any taint label is forbidden for that sink.
    pub fn can_flow_to(&self, sink: DataSink) -> bool {
        match sink {
            DataSink::LlmContext => {
                !self.labels.contains(&TaintLabel::WalletSecret)
                    && !self.labels.contains(&TaintLabel::OwnerSecret)
            }
            DataSink::AuditLog => {
                !self.labels.contains(&TaintLabel::WalletSecret)
            }
            DataSink::MeshRelay => {
                !self.labels.contains(&TaintLabel::StrategyConfidential)
                    && !self.labels.contains(&TaintLabel::UserPII)
                    && !self.labels.contains(&TaintLabel::WalletSecret)
            }
            DataSink::EventFabric => {
                !self.labels.contains(&TaintLabel::WalletSecret)
                    && !self.labels.contains(&TaintLabel::OwnerSecret)
            }
            DataSink::CollectivePeer => {
                !self.labels.contains(&TaintLabel::WalletSecret)
            }
            DataSink::LocalNeuro => true,
        }
    }
}
```

---

## Four-Stage Ingestion Pipeline

All data from external sources passes through a four-stage ingestion pipeline before entering the Neuro knowledge store. This pipeline is the primary defense against knowledge poisoning attacks.

### Trust Levels

```rust
/// Trust level assigned to ingested data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustLevel {
    /// Untrusted: raw external data, not yet validated.
    Untrusted,
    /// Quarantined: passed initial checks, awaiting consensus.
    Quarantined,
    /// Verified: passed consensus validation, safe to use.
    Verified,
    /// Trusted: from a known, high-reputation source.
    Trusted,
}
```

### Stage 1: Quarantine

Every piece of external data enters at `TrustLevel::Untrusted` and is immediately quarantined. During quarantine:

- The data is stored in a separate quarantine partition, isolated from the main Neuro store
- It is not available for retrieval during agent reasoning
- Basic structural validation runs: JSON schema checks, numeric bounds, format verification
- A content hash is computed for deduplication

**Research basis**: The quarantine pattern is inspired by the immune system's innate immunity — pattern-matching defense that activates immediately without prior exposure. Known-bad patterns (SQL injection strings, shell metacharacters, known malicious payloads) are rejected outright.

### Stage 2: Consensus Validation

Quarantined data that passes structural checks enters consensus validation. This is a multi-layer process:

**Layer 1: TrustRAG anomaly detection.** Uses the HDC (Hyperdimensional Computing) similarity engine in `roko-primitives` to compare the incoming entry against existing knowledge. Entries with low similarity to anything in the store (novel claims) are flagged for additional scrutiny. Entries with high similarity to known anti-knowledge (previously falsified claims) are rejected.

Reference: TrustRAG builds on anomaly detection principles from HDC nearest-neighbor classification (Kanerva, 2009, Cognitive Computation 1(2)).

**Layer 2: A-MemGuard behavioral analysis.** Checks whether the incoming entry is consistent with the agent's existing causal model. An entry claiming "ETH always goes up" contradicts the agent's volatility models and is flagged. Consistency is measured using the coherence axis of the 7-axis Engram Score.

**Layer 3: Multi-validator consensus.** For high-stakes entries (those that would change decision thresholds or strategy parameters), multiple independent validators must agree. The threshold is 2-of-3 for normal entries and 3-of-5 for entries that modify safety parameters.

### Stage 3: Sandbox Testing

Entries that pass consensus enter sandbox testing. The agent runs a counterfactual simulation: "If this entry had been in my knowledge store for the last N decisions, would those decisions have been better or worse?"

This is implemented as a lightweight replay of recent episodes (see `roko-learn/episode_logger.rs`) with the candidate entry injected into the Neuro store. If the counterfactual outcomes are worse (lower gate pass rates, higher cost, more failures), the entry is rejected.

### Stage 4: Adoption with Causal Rollback

Entries that survive sandbox testing are adopted into the Neuro store at `TrustLevel::Verified`. They enter at the Transient tier (0.1x base half-life) and must earn promotion to Working (0.5x), Consolidated (1.0x), and Persistent (5.0x) tiers through demonstrated utility.

If an adopted entry later proves harmful (detected via declining gate pass rates correlated with the entry's use), the system performs causal rollback:

1. Identify all decisions that used the harmful entry as context
2. Mark the entry as anti-knowledge (a falsified claim that inoculates against similar future entries)
3. Store "dual memory" lessons: what went wrong and what the correct approach was
4. Optionally replay the affected decisions without the harmful entry

---

## External Data Taint Sources

All data that crosses a trust boundary is tainted:

| Source | Taint Type | Validation Gates |
|--------|-----------|-----------------|
| Tool result (bash, read_file) | `UntrustedExternal` | JSON schema, output truncation, secret scrubbing |
| LLM inference output | `UntrustedExternal` | JSON schema (action grammar), regex (address format) |
| Mesh relay entries | `UntrustedExternal` | 4-stage ingestion pipeline |
| Operator TUI input | `UntrustedExternal` (auto-promoted) | Regex, numeric bounds |
| Peer agent messages | `UntrustedExternal` | Reputation-weighted trust, consensus validation |
| API response bodies | `UntrustedExternal` | JSON schema, numeric bounds |

The LLM's own output is tainted — this is counterintuitive but correct. The LLM produces text that gets parsed into proposed actions. Those proposed actions must pass through validation (JSON schema matching the action grammar) before they can become executable. Pan et al. (ACL 2024) documented how compressed or injected context can redirect LLM behavior; taint tracking prevents injected content from reaching execution paths regardless of whether the LLM "believes" the injection.

---

## Current Implementation: ScrubPolicy

The current Roko codebase implements a post-hoc approximation of taint tracking through the `ScrubPolicy` in `roko-agent/src/safety/scrub.rs`. This is not compile-time taint tracking — it is regex-based pattern matching that catches secrets after they appear in tool output.

The scrubber applies 9 default regex patterns covering:
- API keys (Anthropic, OpenAI, AWS, GitHub, GitLab, Slack)
- JWTs (three base64url segments starting with `eyJ`)
- Private key blocks (`-----BEGIN * PRIVATE KEY-----`)
- Env-file assignments (`PASSWORD=`, `SECRET=`, `TOKEN=`, etc.)

Additional user-defined patterns can be added via `ScrubPolicy::extra_patterns`.

**Limitation**: Regex-based scrubbing is a heuristic. It catches common patterns but cannot prevent all data exfiltration. A determined attacker could encode secrets in base64, split them across multiple tool calls, or use steganographic techniques. The full `TaintedString` compile-time tracking (Tier 2) addresses these limitations by preventing tainted data from reaching sinks at the type level.

---

## TaintedString: full implementation

The `TaintedString` type provides label manipulation, propagation through string operations, and controlled access to the inner value.

```rust
impl TaintedString {
    /// Create a new tainted string with a single label.
    pub fn new(value: String, label: TaintLabel) -> Self {
        let mut labels = HashSet::new();
        labels.insert(label);
        Self {
            value: zeroize::Zeroizing::new(value),
            labels,
        }
    }

    /// Add a taint label. Labels accumulate -- once tainted, always tainted.
    pub fn with_label(mut self, label: TaintLabel) -> Self {
        self.labels.insert(label);
        self
    }

    /// Merge labels from another TaintedString.
    /// Used when concatenating or interpolating tainted values.
    pub fn merge_labels(&mut self, other: &TaintedString) {
        self.labels.extend(&other.labels);
    }

    /// Access the inner value. Caller must have verified
    /// can_flow_to() for the target sink.
    ///
    /// # Panics
    /// Debug builds panic if called without a prior can_flow_to() check.
    /// Release builds return the value (enforcement is at the sink boundary).
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Check all labels at once.
    pub fn labels(&self) -> &HashSet<TaintLabel> {
        &self.labels
    }

    /// True if this string carries no taint labels.
    pub fn is_clean(&self) -> bool {
        self.labels.is_empty()
    }

    /// Strip a label after sanitization.
    /// Only ScrubPolicy and the ingestion pipeline should call this.
    pub fn declassify(&mut self, label: TaintLabel) {
        self.labels.remove(&label);
    }
}
```

Label propagation rule: when two `TaintedString` values combine (concatenation, interpolation, format strings), the result carries the union of both label sets. This prevents label stripping through string manipulation.

---

## Bloom Oracle (Design Target)

The Bloom Oracle is a probabilistic filter for rapid initial screening of ingested data. Using a Bloom filter trained on known-bad patterns (malicious payloads, known attack signatures, previously rejected entries), it provides O(1) rejection of data that has a high probability of being harmful.

False positive rate: configurable, default 0.1% (one legitimate entry in a thousand incorrectly flagged -- flagged entries go to Stage 2 consensus, not rejected outright). False negative rate: zero by construction (Bloom filters never produce false negatives).

### BloomOracle struct and configuration

```rust
use bitvec::prelude::*;

/// Probabilistic filter for rapid screening of ingested data.
/// Uses k independent hash functions over a bit array of size m.
pub struct BloomOracle {
    /// Bit array backing the filter.
    bits: BitVec,
    /// Number of hash functions (k).
    num_hashes: u32,
    /// Number of bits (m). Determines false positive rate.
    num_bits: usize,
    /// Number of items inserted so far (n).
    count: usize,
}
```

**Parameter selection.** Given a target false-positive rate `p` and expected item count `n`:

| Parameter | Formula | Default |
|-----------|---------|---------|
| Bits (m) | `m = -(n * ln(p)) / (ln(2)^2)` | 9,586 bits for n=1000, p=0.001 |
| Hash functions (k) | `k = (m / n) * ln(2)` | 10 for default parameters |
| False positive rate (p) | `p = (1 - e^(-kn/m))^k` | 0.001 (0.1%) |
| False negative rate | 0 by construction | 0 |

```rust
impl BloomOracle {
    /// Create a new BloomOracle with target false-positive rate.
    ///
    /// # Parameters
    /// - `expected_items`: estimated number of entries (n). Range: 100..10_000_000.
    /// - `false_positive_rate`: target FP rate (p). Range: 0.0001..0.1.
    pub fn new(expected_items: usize, false_positive_rate: f64) -> Self {
        assert!(
            (0.0001..0.1).contains(&false_positive_rate),
            "FP rate must be in [0.0001, 0.1]"
        );
        let ln2 = std::f64::consts::LN_2;
        let num_bits =
            (-(expected_items as f64) * false_positive_rate.ln() / (ln2 * ln2)).ceil() as usize;
        let num_hashes = ((num_bits as f64 / expected_items as f64) * ln2).ceil() as u32;

        Self {
            bits: bitvec![0; num_bits],
            num_hashes,
            num_bits,
            count: 0,
        }
    }

    /// Insert a known-bad pattern into the filter.
    pub fn insert(&mut self, item: &[u8]) {
        for i in 0..self.num_hashes {
            let idx = self.hash(item, i) % self.num_bits;
            self.bits.set(idx, true);
        }
        self.count += 1;
    }

    /// Query whether an item might be known-bad.
    /// Returns true = "possibly bad" (proceed to Stage 2 consensus).
    /// Returns false = "definitely not in the known-bad set."
    pub fn maybe_bad(&self, item: &[u8]) -> bool {
        (0..self.num_hashes).all(|i| {
            let idx = self.hash(item, i) % self.num_bits;
            self.bits[idx]
        })
    }

    /// Current fill ratio. When this exceeds 0.5, the FP rate
    /// degrades faster than the theoretical bound. Rebuild with
    /// a larger m when fill_ratio() > 0.5.
    pub fn fill_ratio(&self) -> f64 {
        self.bits.count_ones() as f64 / self.num_bits as f64
    }

    fn hash(&self, item: &[u8], seed: u32) -> usize {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&seed.to_le_bytes());
        hasher.update(item);
        let hash = hasher.finalize();
        let bytes: [u8; 8] = hash.as_bytes()[..8].try_into().unwrap();
        usize::from_le_bytes(bytes)
    }
}
```

**roko.toml configuration:**

```toml
[safety.bloom_oracle]
expected_items = 5000       # Estimated known-bad patterns. Range: 100..10_000_000.
false_positive_rate = 0.001 # Target FP rate. Range: 0.0001..0.1.
rebuild_threshold = 0.5     # Rebuild when fill_ratio exceeds this. Range: 0.3..0.8.
```

---

## Four-stage ingestion state machine

The ingestion pipeline is a state machine with four states and explicit transition triggers. Each entry carries its current state and transitions forward on success or backward/out on failure.

```
                    +-----------+
                    | Rejected  |
                    +-----^-----+
                          |
        structural fail   | consensus fail   sandbox fail
                          |                       |
  +-----------+     +-----+------+     +----------+-+     +----------+
  | Untrusted | --> | Quarantine | --> |  Consensus | --> |  Sandbox | --> Adopted
  +-----------+     +------------+     +------------+     +----------+
     entry             pass struct       pass layers         pass
                       validation        1/2/3              counterfactual
```

### Transition triggers

| From | To | Trigger |
|------|----|---------|
| Untrusted | Quarantine | Entry received from any external source |
| Quarantine | Consensus | Structural validation passes: valid JSON, numeric bounds within schema, content hash computed, no duplicate |
| Quarantine | Rejected | Structural validation fails, or BloomOracle flags entry as known-bad |
| Consensus | Sandbox | All consensus layers pass: TrustRAG similarity above threshold (default 0.3), A-MemGuard coherence above threshold (default 0.5), multi-validator quorum met |
| Consensus | Rejected | Any consensus layer fails, or entry matches anti-knowledge signature |
| Sandbox | Adopted | Counterfactual replay shows neutral-or-positive outcome (gate pass rate delta >= -0.02) |
| Sandbox | Rejected | Counterfactual replay shows negative outcome (gate pass rate delta < -0.02) |
| Adopted | Anti-knowledge | Post-adoption monitoring detects declining gate pass rates correlated with entry usage (see causal rollback below) |

### Configuration parameters

```toml
[safety.ingestion]
quarantine_timeout_secs = 3600       # Max time in quarantine. Range: 60..86400.
trustrag_similarity_threshold = 0.3  # Minimum HDC similarity. Range: 0.0..1.0.
amemguard_coherence_threshold = 0.5  # Minimum coherence score. Range: 0.0..1.0.
normal_quorum = "2-of-3"             # Validator quorum for normal entries.
safety_quorum = "3-of-5"             # Validator quorum for safety-parameter entries.
sandbox_replay_window = 50           # Number of recent episodes to replay. Range: 10..500.
sandbox_gate_delta_threshold = -0.02 # Gate pass rate delta below which entry is rejected.
causal_rollback_correlation = 0.7    # Min Pearson r between entry usage and declining gates. Range: 0.3..0.95.
```

### Causal rollback state transitions

When an adopted entry proves harmful, the rollback proceeds through three states:

```
  +----------+        +-----------+        +-----------------+
  |  Adopted | -----> | Suspected | -----> | Anti-Knowledge  |
  +----------+  gate  +-----------+  corr  +-----------------+
                decline   window    confirmed     |
                detected  (20 turns)              |
                                                  v
                                        +------------------+
                                        | Dual Memory      |
                                        | (lesson stored)  |
                                        +------------------+
```

| From | To | Trigger |
|------|----|---------|
| Adopted | Suspected | Gate pass rate drops below EMA - 2 sigma for 5 consecutive turns while entry is in active context |
| Suspected | Adopted | Observation window (20 turns) passes without confirming correlation (Pearson r < `causal_rollback_correlation`) |
| Suspected | Anti-Knowledge | Pearson correlation between entry usage frequency and gate failure rate exceeds `causal_rollback_correlation` over 20-turn window |
| Anti-Knowledge | Dual Memory | System stores: (1) the harmful entry, (2) which decisions it affected, (3) what the correct approach was. Downstream decisions optionally replayed without the entry. |

```rust
/// State of an ingested entry in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestionState {
    Untrusted,
    Quarantine,
    Consensus,
    Sandbox,
    Adopted,
    Suspected,
    AntiKnowledge,
    Rejected,
}

/// An entry moving through the ingestion pipeline.
pub struct IngestEntry {
    pub content_hash: blake3::Hash,
    pub state: IngestionState,
    pub trust_level: TrustLevel,
    pub entered_at: std::time::Instant,
    pub state_changed_at: std::time::Instant,
    /// Number of consecutive turns with declining gate pass rate.
    pub decline_streak: u32,
    /// Turn counter for the suspected observation window.
    pub observation_turns: u32,
}
```

---

## ScrubPolicy wiring path

The `ScrubPolicy` (runtime regex-based taint approximation) wires into the agent dispatch pipeline through `orchestrate.rs`:

```
orchestrate.rs: PlanRunner::run_task()
  |
  +--> ExecAgent::run()
         |
         +--> ToolDispatcher::dispatch()
                |
                +--> [pre-execution] SafetyLayer::check_pre_execution()
                |      includes: BashPolicy, GitPolicy, PathPolicy, NetworkPolicy
                |
                +--> [execute tool handler]
                |
                +--> [post-execution] ScrubPolicy::scrub_output()
                |      applies 9 default regex patterns + user extras
                |      replaces matches with "[REDACTED:<pattern_name>]"
                |
                +--> [post-execution] RateLimiter::check()
                |
                +--> emit_audit() --> .roko/signals.jsonl
```

**Integration points:**

| Component | File | Function |
|-----------|------|----------|
| ScrubPolicy definition | `roko-agent/src/safety/scrub.rs` | `ScrubPolicy::new()`, `scrub_output()` |
| SafetyLayer composition | `roko-agent/src/safety/mod.rs` | `SafetyLayer::check_pre_execution()` |
| ToolDispatcher pipeline | `roko-agent/src/dispatcher/mod.rs` | `dispatch()` 7-step pipeline |
| Orchestrator entry point | `roko-cli/src/orchestrate.rs` | `PlanRunner::run_task()` |
| Custom patterns config | `roko.toml` | `[agent.safety.scrub_patterns]` |

**Error handling:** When `ScrubPolicy::scrub_output()` matches a pattern, it replaces the match in-place and logs a `Kind::SecurityEvent` Engram to the audit trail. The tool call succeeds (the scrubbed output is returned to the agent), but the original unscrubbed value is never exposed. If scrubbing fails (regex compilation error on a user-provided pattern), the tool output is blocked entirely and a `Kind::Error` Engram is emitted.

### Test criteria

- `TaintedString::can_flow_to()` blocks every forbidden label/sink pair per the flow rules matrix
- `TaintedString::with_label()` accumulates labels; `merge_labels()` takes the union
- `TaintedString::declassify()` removes a single label without affecting others
- `BloomOracle` achieves measured FP rate within 2x of configured target over 10,000 random queries
- `BloomOracle::maybe_bad()` never returns false for an inserted item (zero false negatives)
- Ingestion state machine transitions match the trigger table: no entry skips a state
- Causal rollback transitions from Suspected back to Adopted when correlation is below threshold
- `ScrubPolicy` catches all 9 default patterns in synthetic tool output
- Scrubbing failure on a bad regex blocks the entire output rather than leaking secrets

---

## Academic References

| Paper | Contribution |
|-------|-------------|
| Pan et al. (ACL 2024) | Compressed/injected context redirects LLM behavior |
| Debenedetti et al. (2025) | CaMeL — separate control flow from data flow |
| Kanerva (2009, Cognitive Computation 1(2)) | HDC similarity for anomaly detection |
| AgentPoison (Chen et al., 2024) | Optimized backdoor triggers for RAG-based agents |
| MINJA (Cheng et al., 2024) | Adversarial injection through normal interactions |
| MemoryGraft (Li et al., 2024) | Gradual behavioral drift via subtle memory bias |
| TrustRAG (Jiang et al., 2025) | Anomaly detection for RAG retrieval poisoning |
| A-MemGuard (Wang et al., 2025) | Memory integrity verification for persistent agents |
| Denning (1976, CACM 19(5):236-243) | Security lattice for information flow control |
| Myers & Liskov (1997, SOSP '97) | Decentralized label model for IFC |
| Costa & Kopf (2025, arXiv:2505.23643) | FIDES -- IFC for agentic systems, zero policy-violating injections |
| Zhong et al. (2025, arXiv:2502.08966) | RTBAS -- dynamic taint tracking, 100% attack prevention |
| Kim et al. (2025, arXiv:2503.15547) | Prompt Flow Integrity -- privilege escalation prevention |
| Palumbo et al. (2026, arXiv:2602.16708) | PCAS -- Datalog policy compiler for agent systems |

---

## Taint Propagation Algebra

Taint tracking in Roko formalizes as lattice-theoretic information flow control. The core result: when labeled data flows through tool call chains, the output label is the join (least upper bound) of all input labels. This guarantees that taint never decreases -- it only accumulates.

The formalization draws on two foundational models:

- **Denning's lattice model** (1976, CACM 19(5):236-243) established that information flow policies form a lattice, with a can-flow-to partial order over security classes. If class A <= class B, data labeled A may flow to a sink labeled B. The join operator computes the least upper bound when data from multiple classes combines.

- **Myers & Liskov's decentralized label model** (1997, SOSP '97) extended Denning's centralized lattice with per-principal ownership. Each label has an owner who controls declassification. This matters for multi-tenant agent deployments where different owners have different confidentiality policies.

### Security lattice

Define Roko's security lattice L = (SC, <=, join) where:
- SC = set of security classes
- <= = can-flow-to partial order
- join = least upper bound operator

```rust
/// Security lattice for information flow control.
/// Based on Denning (1976, CACM 19(5):236-243).
///
/// The lattice defines a partial order over security classes.
/// Information may only flow from class A to class B if A <= B.
/// When data from multiple sources is combined, the result is
/// labeled with the join of all input labels.
pub struct SecurityLattice {
    /// Ordered security levels from lowest to highest.
    pub levels: Vec<SecurityLevel>,
    /// The can-flow-to relation: (from, to) pairs.
    pub flow_relation: HashSet<(SecurityLevel, SecurityLevel)>,
}

/// A two-dimensional security label following Denning's model.
/// Confidentiality restricts who can read; integrity restricts who can write.
pub struct SecurityLabel {
    /// Confidentiality level: restricts read access.
    /// Higher = more restricted.
    pub confidentiality: LatticeLevel,
    /// Integrity level: restricts trust in the data.
    /// Higher = more trusted.
    pub integrity: LatticeLevel,
    /// Owner principal (for DLM-style declassification).
    /// Only the owner can declassify their own data.
    pub owner: Option<Principal>,
}

/// Lattice levels ordered by restrictiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LatticeLevel {
    /// Public / untrusted. Lowest restriction.
    Public = 0,
    /// Internal / semi-trusted.
    Internal = 1,
    /// Confidential / trusted.
    Confidential = 2,
    /// Secret / highly trusted.
    Secret = 3,
}
```

The two dimensions serve complementary purposes. Confidentiality tracks *who can read* the data -- a `Secret` value must not leak to a `Public` sink. Integrity tracks *how much to trust* the data -- a `Public`-integrity value (untrusted external input) must not flow to a `Secret`-integrity sink (safety-critical configuration) without validation.

### Join operator (label combination)

When two labeled values combine (concatenation, function application, tool pipeline), the result carries the join of both labels:

```rust
impl SecurityLabel {
    /// Join operator: combines two labels into the least upper bound.
    /// Result has max(confidentiality) and min(integrity).
    /// This is the fundamental propagation rule.
    pub fn join(&self, other: &SecurityLabel) -> SecurityLabel {
        SecurityLabel {
            confidentiality: self.confidentiality.max(other.confidentiality),
            integrity: self.integrity.min(other.integrity),
            owner: None, // joined data has no single owner
        }
    }

    /// Can data with this label flow to a sink with the given label?
    /// Requires: self.confidentiality <= sink.confidentiality
    ///       AND self.integrity >= sink.integrity
    pub fn can_flow_to(&self, sink: &SecurityLabel) -> bool {
        self.confidentiality <= sink.confidentiality
            && self.integrity >= sink.integrity
    }
}
```

The join follows standard lattice IFC semantics: confidentiality rises to the maximum (most restrictive wins), integrity drops to the minimum (least trusted wins). This means mixing a `Secret`-confidentiality value with a `Public`-confidentiality value produces `Secret` -- the combined data inherits the strictest read restriction. Mixing a `Trusted`-integrity value with an `Untrusted`-integrity value produces `Untrusted` -- the combined data is only as trustworthy as its least trusted component.

### Tool call taint propagation

Taint propagates through agent tool call chains by the same join rule. Each tool call takes labeled inputs and produces a labeled output. The output label is the join of all input labels:

```
For each tool call T with inputs I_1, I_2, ..., I_n:
    label(output(T)) = join_i label(I_i)    // join of all input labels

For an agent turn with tool calls T_1 -> T_2 -> ... -> T_k:
    label(context_window) = join_j label(output(T_j))
```

This means a single tainted input taints the entire downstream chain. If tool T_1 reads a `Secret`-confidentiality file, and T_2 uses T_1's output, T_2's output is also `Secret`. The context window accumulates taint from every tool call in the turn.

```rust
/// Taint-propagating tool call wrapper.
/// Tracks security labels through the tool dispatch pipeline.
pub struct TaintedToolCall {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    /// Labels of all inputs to this tool call.
    pub input_labels: Vec<SecurityLabel>,
    /// Computed output label: join of all input labels.
    pub output_label: SecurityLabel,
}

impl TaintedToolCall {
    /// Compute the output label from input labels.
    pub fn compute_output_label(inputs: &[SecurityLabel]) -> SecurityLabel {
        inputs.iter().fold(
            SecurityLabel {
                confidentiality: LatticeLevel::Public,
                integrity: LatticeLevel::Secret,
                owner: None,
            },
            |acc, label| acc.join(label),
        )
    }
}
```

The fold starts from the lattice bottom for confidentiality (`Public`) and lattice top for integrity (`Secret`). This identity element ensures that a single input produces its own label unchanged, while multiple inputs accumulate via join.

### FIDES integration

Costa & Kopf (2025, arXiv:2505.23643) introduced FIDES, an information flow control planner for agentic systems. FIDES assigns confidentiality and integrity labels to every data source and tool, then plans agent actions to avoid policy violations. Two results are directly applicable to Roko:

- **Zero policy-violating injections on AgentDojo.** FIDES reduced injection attack success to 0% by constraining the LLM's output schema based on the label of its inputs. When the context window contains low-integrity data (e.g., web-scraped content), the planner restricts the LLM to Boolean or enum outputs -- formats too constrained to carry injection payloads.

- **Constrained decoding for quarantined LLM.** When the LLM processes untrusted input, FIDES routes it through a "quarantined LLM" instance with constrained decoding. The output grammar is limited to a predefined schema, preventing free-text generation that could propagate injected instructions.

Roko's `TaintedToolCall` pipeline implements the same principle: tool calls with low-integrity inputs have their output schemas validated against tighter constraints before the output enters the context window.

### RTBAS dynamic taint tracking

Zhong et al. (2025, arXiv:2502.08966) proposed RTBAS (Runtime Taint-Based Access Control), a dynamic taint tracking system for LLM agents. Three design decisions align with Roko's approach:

- **Taint labels at ingestion.** Every piece of data receives a taint label when it enters the system. External API responses, tool outputs, and user inputs each get labels reflecting their trust level. This matches Roko's `TaintLabel` enum and the ingestion pipeline's quarantine stage.

- **Two novel screeners.** RTBAS introduces an LM-Judge screener (a secondary LLM that evaluates whether a proposed action violates taint policy) and an Attention-Based screener (which analyzes the primary LLM's attention patterns to detect when tainted tokens disproportionately influence the output). The Attention-Based screener is particularly interesting -- it detects prompt injection by measuring whether untrusted tokens receive anomalous attention weight.

- **100% attack prevention with <2% utility degradation.** On their benchmark, RTBAS blocked all tested injection attacks while degrading legitimate task completion by less than 2%. This suggests that taint tracking imposes minimal overhead on normal agent operation.

### Prompt Flow Integrity (PFI)

Kim et al. (2025, arXiv:2503.15547) addressed the STRIDE threat category "Elevation of Privilege" in multi-agent systems. Their Prompt Flow Integrity (PFI) framework:

- **Separates agents by trust level.** Trusted agents with access to sensitive tools (file write, code execution) are isolated from untrusted agents that process external input. Data flows between agents are labeled and checked at every boundary.

- **Tracks inter-agent data flow.** When Agent A sends a message to Agent B, PFI tracks which of A's inputs influenced the message. If A processed untrusted web content, the message to B carries that taint. B's safety layer can then decide whether to accept the message for use in privileged operations.

- **Raises alerts on unsafe flow.** PFI detects when tainted data from an untrusted agent reaches a privileged tool call through an intermediate trusted agent. This catches "confused deputy" attacks where a trusted agent is tricked into executing actions on behalf of an attacker.

This maps to Roko's multi-agent topology: when `roko plan run` spawns multiple agents for parallel task execution, inter-agent messages must carry taint labels. An agent processing untrusted external data should not influence a peer agent's safety-critical decisions without explicit declassification.

### PCAS Datalog policy language

Palumbo et al. (2026, arXiv:2602.16708) proposed PCAS, a Datalog-derived policy language for expressing taint rules over agent tool call graphs. Instead of hard-coding flow rules in Rust match statements, PCAS expresses policies as declarative rules that a solver evaluates at runtime.

The advantage: policies can be updated without recompiling the agent. A new taint rule (e.g., "data from source X must not reach tool Y") becomes a Datalog fact, not a code change.

```
% A tool call is tainted if any input is tainted.
tainted(Call) :- input(Call, Data), tainted(Data).

% Transitive taint propagation through tool chains.
tainted(Result) :- generated_by(Result, Call), tainted(Call).

% Policy: block tainted calls to sensitive resources.
blocked(Call) :- tainted(Call), accesses_sensitive(Call).

% Declassification: owner can remove taint from their own data.
declassified(Data) :- owner(Data, Principal), authorizes(Principal, Data).
not_tainted(Data) :- declassified(Data).
```

PCAS also maintains a fine-grained dependency graph over tool calls. When a taint violation is detected, the graph traces the violation back to its origin -- which external input, through which tool chain, caused the tainted data to reach the forbidden sink. This provenance is valuable for debugging and for the causal rollback mechanism described in the ingestion pipeline section above.

### Configuration

```toml
[safety.taint]
# Lattice model: "denning" (centralized) or "dlm" (decentralized label model).
lattice_model = "denning"
# Default confidentiality level for external data. Options: "public", "internal", "confidential", "secret".
external_data_confidentiality = "public"
# Default integrity level for external data. Options: "public", "internal", "confidential", "secret".
external_data_integrity = "public"
# Default integrity level for tool results. Options: "public", "internal", "confidential", "secret".
tool_result_integrity = "internal"
# Whether to block or log taint violations. Options: "enforce" | "monitor" | "off".
enforcement_mode = "monitor"
# Propagation mode: "strict" (always join) or "selective" (per-tool rules).
propagation_mode = "strict"
# Policy backend: "rust" (compiled match statements) or "datalog" (PCAS-style rules).
policy_backend = "rust"
# Path to Datalog policy file (used when policy_backend = "datalog").
datalog_policy_path = ".roko/policies/taint.dl"
```

### Test criteria

- `SecurityLabel::join()` produces `max(confidentiality)`, `min(integrity)` for all level combinations
- `SecurityLabel::can_flow_to()` blocks `Secret` -> `Public` confidentiality flow
- `SecurityLabel::can_flow_to()` blocks `Public` -> `Secret` integrity flow (low-integrity data to high-integrity sink)
- `SecurityLabel::can_flow_to()` allows `Public` -> `Secret` confidentiality flow (reading up is fine)
- `SecurityLabel::can_flow_to()` allows `Secret` -> `Public` integrity flow (trusted data to untrusted sink is fine)
- `TaintedToolCall::compute_output_label()` with mixed inputs produces the correct join
- `TaintedToolCall::compute_output_label()` with empty inputs returns the identity element (`Public` confidentiality, `Secret` integrity)
- Tool chain T1 -> T2 -> T3 propagates taint transitively: if T1's input is `Secret`, T3's output is `Secret`
- Declassification only succeeds when the caller matches the `owner` principal on the label
- Declassification by a non-owner principal is rejected
- PCAS-style transitive taint rule correctly identifies multi-hop taint propagation across 5+ tool calls
- Join is commutative: `a.join(b) == b.join(a)` for all label pairs
- Join is associative: `a.join(b.join(c)) == a.join(b).join(c)` for all label triples
- Join is idempotent: `a.join(a) == a` for all labels

---

## Cross-References

- [00-defense-in-depth.md](00-defense-in-depth.md) — Overall safety architecture
- [07-prompt-security.md](07-prompt-security.md) — Prompt injection defense
- [08-threat-model.md](08-threat-model.md) — Knowledge poisoning attack trees
- [09-adaptive-risk.md](09-adaptive-risk.md) — Adaptive guardrails that respond to trust levels
