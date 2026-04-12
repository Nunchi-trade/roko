# Taint-Aware Ingestion and Data Flow Control

> **Layer**: L2 Scaffold (context engineering), L3 Harness (ingestion gates), Cross-cut (Neuro)
>
> **Crate**: `roko-agent` (ScrubPolicy), target: `roko-neuro` (taint labels), `roko-compose` (context assembly)
>
> **Synapse traits**: `Gate` (verify taint compliance before data flows to sinks), `Scorer` (rate trust level of ingested data)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), Neuro knowledge store documentation

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

## Bloom Oracle (Design Target)

The Bloom Oracle is a probabilistic filter for rapid initial screening of ingested data. Using a Bloom filter trained on known-bad patterns (malicious payloads, known attack signatures, previously rejected entries), it provides O(1) rejection of data that has a high probability of being harmful.

False positive rate: configurable, default 0.1% (one legitimate entry in a thousand incorrectly flagged — flagged entries go to Stage 2 consensus, not rejected outright). False negative rate: zero by construction (Bloom filters never produce false negatives).

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

---

## Related Topics

- [00-defense-in-depth.md](00-defense-in-depth.md) — Overall safety architecture
- [07-prompt-security.md](07-prompt-security.md) — Prompt injection defense
- [08-threat-model.md](08-threat-model.md) — Knowledge poisoning attack trees
- [09-adaptive-risk.md](09-adaptive-risk.md) — Adaptive guardrails that respond to trust levels
