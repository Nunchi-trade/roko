# Provenance and Attestation

> **Abstract:** Every Engram carries provenance — who produced it, how trusted they are,
> and whether the data is tainted. Provenance enables taint analysis (preventing untrusted
> data from reaching privileged contexts), audit trails (tracing decisions back to their
> source inputs), and reputation-weighted scoring. The extended architecture adds an
> Attestation field for cryptographic proof of origin. This document specifies both the
> current Provenance struct and the planned Attestation extension.


> **Implementation**: Shipping

---

## 1. Why Provenance Matters

Agent systems consume data from many sources: LLM outputs, tool results, external APIs,
user input, on-chain state, knowledge from other agents. Not all sources are equally
trustworthy. A compilation gate's verdict is ground truth; an LLM's unverified suggestion
is a hypothesis; user input from an untrusted channel might contain prompt injection.

Provenance answers three questions for every Engram:

1. **Who produced this?** — agent role, model, human user, external chain, gate
2. **How trusted is that producer?** — a trust score in [0, 1]
3. **Is the data tainted?** — from an untrusted external source, needs validation

These answers flow through the entire system:

- **Taint analysis**: Tainted Engrams must be sanitized before they enter prompts or gates.
  This prevents prompt injection (OWASP LLM01:2025) from untrusted sources.
- **Audit trails**: Combined with lineage (see [02-engram-data-type.md](02-engram-data-type.md)),
  provenance creates a complete chain of accountability from any output back to its origins.
- **Reputation scoring**: The Score's reputation axis (see [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md))
  is initialized from provenance trust.

---

## 2. The Provenance Struct

The current implementation (`roko-core/src/provenance.rs`):

```rust
/// Who produced an Engram and how trustworthy they are.
///
/// Roko uses provenance to:
/// 1. Audit: trace decisions back to their source inputs
/// 2. Security: prevent untrusted data from reaching high-privilege contexts
/// 3. Reputation: weight Engrams by their author's track record
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// Identifier of the producer (agent role, user email, chain address, etc.).
    pub author: String,

    /// Trust score [0..1] at time of emission.
    /// 1.0 = fully trusted (local code, verified gates)
    /// 0.5 = unverified but internal
    /// 0.0 = untrusted (user input, external APIs, chain pulls)
    pub trust: f32,

    /// Whether this Engram contains data from an untrusted source.
    /// Tainted Engrams must be sanitized before they enter prompts or gates.
    pub tainted: bool,

    /// Optional: the agent session or run that produced this Engram.
    /// Useful for grouping related Engrams and computing per-run metrics.
    pub session: Option<String>,
}
```

### 2.1 Fields in Detail

#### author: String

The identifier of the Engram's producer. Format is free-form but conventions exist:

| Author Pattern | Meaning |
|---|---|
| `"roko"` | The Roko framework itself (default) |
| `"gate:compile"` | The compilation gate |
| `"gate:test"` | The test gate |
| `"agent:claude-sonnet"` | An LLM agent using Claude Sonnet |
| `"user:alice@example.com"` | A human user |
| `"chain:0x1234..."` | An on-chain source (contract address) |
| `"derived"` | Produced by `Engram.derive()` |
| `"external:webhook"` | An external webhook source |

The author string is included in the ContentHash computation, so the same content from
different authors produces different Engram identities.

#### trust: f32

A trust score in [0, 1] at the time the Engram was created. This is a snapshot — the
producer's trust level at emission time, not a live value.

| Trust Level | Range | Meaning |
|---|---|---|
| **Fully trusted** | 1.0 | Local code, verified gate outputs, the orchestrator itself |
| **Agent trusted** | 0.75 | Internal agent output — trusted but not ground truth |
| **User trusted** | 0.5 | User input — higher trust than external, but tainted for safety |
| **External** | 0.1 | Untrusted external source — needs validation before use |
| **Untrusted** | 0.0 | Known-bad or explicitly untrusted source |

Trust is clamped to [0, 1] via `with_trust()`:

```rust
pub const fn with_trust(mut self, trust: f32) -> Self {
    self.trust = trust.clamp(0.0, 1.0);
    self
}
```

#### tainted: bool

A binary flag indicating whether the Engram contains data from an untrusted source. Tainted
Engrams are treated differently throughout the system:

- They must be sanitized before entering prompts (preventing prompt injection)
- They cannot be used as ground truth by gates
- They are flagged in audit trails
- The `is_trusted()` method always returns `false` for tainted Engrams, regardless of trust level

```rust
pub fn is_trusted(&self, min_trust: f32) -> bool {
    self.trust >= min_trust && !self.tainted
}
```

This design ensures that even a high-trust tainted Engram (trust = 0.9 but tainted = true)
is never silently treated as trusted. Taint is a hard barrier, not a soft one.

#### session: Option<String>

An optional session identifier for grouping related Engrams. All Engrams produced during a
single agent run share a session ID, enabling:

- Per-run episode reconstruction
- Per-run metric computation
- Session-scoped Substrate queries (`Query.with_session()`)

---

## 3. Provenance Constructors

Four convenience constructors establish default trust levels for common producer types:

```rust
impl Provenance {
    /// Produced by trusted internal code (gates, composers, orchestrator).
    pub fn trusted(author: impl Into<String>) -> Self {
        Self { author: author.into(), trust: 1.0, tainted: false, session: None }
    }

    /// Produced by an internal agent — trusted but not ground truth.
    pub fn agent(author: impl Into<String>) -> Self {
        Self { author: author.into(), trust: 0.75, tainted: false, session: None }
    }

    /// From an external/untrusted source — needs sanitization before use.
    pub fn external(author: impl Into<String>) -> Self {
        Self { author: author.into(), trust: 0.1, tainted: true, session: None }
    }

    /// From a user (higher trust than external, but still tainted for safety).
    pub fn user(author: impl Into<String>) -> Self {
        Self { author: author.into(), trust: 0.5, tainted: true, session: None }
    }
}
```

The default provenance (`Provenance::default()`) is `trusted("roko")` — fully trusted,
not tainted, no session. This is used by the EngramBuilder when no explicit provenance is
specified.

---

## 4. Taint Analysis

Taint tracking in Roko implements a simplified form of information flow control. The rules:

1. **User input is always tainted**: Even trusted users can inadvertently introduce prompt
   injection or malformed data.

2. **External data is always tainted**: Webhooks, API responses, chain state pulled from
   external RPCs — all tainted until validated.

3. **Gate verdicts are never tainted**: Gates verify against ground truth (compilation,
   tests, simulation). Their output is trusted.

4. **Taint propagates through derivation**: If a Composer combines tainted and untainted
   Engrams, the output should be treated with caution. The provenance's taint flag on the
   composed output is set by the Composer implementation.

5. **Taint is cleared by verification**: A tainted Engram that passes a Gate can produce
   an untainted derivative. The Gate verdict Engram has `tainted: false`.

This is not a formal information flow type system (like Jif or FlowCaml). It is a practical
safety measure that prevents the most common failure mode: untrusted external data reaching
an LLM prompt without sanitization.

---

## 5. Provenance in the ContentHash

The `author` and `tainted` fields are included in the ContentHash computation:

```rust
hasher.update(self.provenance.author.as_bytes());
hasher.update(b"|");
hasher.update(&[u8::from(self.provenance.tainted)]);
```

This means the same content from different authors produces different Engram identities.
This is intentional: an insight from a trusted gate and the same text from an untrusted
webhook should be tracked separately because they have different trust properties.

The `trust` and `session` fields are NOT included in the ContentHash. Trust can be updated
as the author's reputation evolves, and session is routing metadata, not identity.

---

## 6. Attestation (Planned Extension)

The extended Engram adds cryptographic attestation — proof that a specific entity produced
the Engram and that the content has not been tampered with.

### 6.1 Attestation Struct (Specified)

```rust
/// Cryptographic proof of Engram origin.
pub struct Attestation {
    /// Ed25519 signature over the Engram's ContentHash.
    pub signature: Ed25519Signature,

    /// Public key of the signer (DID or TEE identity).
    pub public_key: PublicKey,

    /// Optional on-chain attestation (ContentHash posted to chain).
    pub chain_attestation: Option<ChainAttestation>,
}

/// On-chain proof that this Engram's ContentHash was witnessed by a chain.
pub struct ChainAttestation {
    /// Chain ID (e.g., Korai mainnet).
    pub chain_id: u64,

    /// Transaction hash where the ContentHash was posted.
    pub tx_hash: [u8; 32],

    /// Block number at which the attestation was recorded.
    pub block_number: u64,
}
```

### 6.2 What Attestation Provides

| Property | How |
|---|---|
| **Non-repudiation** | Ed25519 signature proves the signer produced the Engram |
| **Integrity** | Signature covers ContentHash, which covers content |
| **Chain witness** | Optional ChainAttestation provides timestamped on-chain proof |
| **Identity binding** | PublicKey links to DID (Decentralized Identifier) or TEE attestation |

### 6.3 Attestation Use Cases

- **Forensic AI**: Regulatory compliance requires proving who generated what output and when.
  Attestation provides cryptographic evidence.
- **Agent Mesh trust**: When agents share knowledge via the Mesh, attestation lets recipients
  verify the origin without trusting the transport layer.
- **Chain witness**: Posting an Engram's ContentHash to the Korai chain creates an immutable
  timestamp. This is the "notary" function — proving that a piece of knowledge existed at a
  specific time.
- **C2PA alignment**: The Content Authenticity Initiative's C2PA standard uses similar
  content-hash-plus-signature provenance. Roko's attestation model is designed to be
  compatible with C2PA's manifest format.

### 6.4 Implementation Status

Attestation is specified but not yet implemented. The current Signal struct does not have an
`attestation` field. This is planned for a future tier of the implementation plan.

---

## 7. Provenance and the Extended Architecture

The refactoring-prd specifies an enhanced Provenance struct with additional fields for
the full architecture:

```rust
pub struct Provenance {
    pub author: String,
    pub trust: f32,
    pub tainted: bool,
    pub session: Option<String>,
    // Extended fields (planned):
    pub model_fingerprint: Option<String>,  // which LLM model produced this
    pub prompt_hash: Option<ContentHash>,   // hash of the prompt that produced this
    pub taint_level: TaintLevel,            // graduated taint (replaces bool)
    pub timestamp: i64,                     // provenance creation time
    pub context: Option<String>,            // provenance context description
}

pub enum TaintLevel {
    Trusted,    // verified ground truth
    Internal,   // internal agent output
    User,       // user-provided input
    External,   // external API/webhook
    Adversarial, // known-hostile source
}
```

The extended Provenance adds model fingerprinting (which specific LLM produced this output),
prompt hashing (what prompt was used), and graduated taint levels (replacing the binary
tainted flag with a spectrum).

---

## Academic Foundations

| Citation | Contribution |
|---|---|
| OWASP LLM01:2025 | Prompt injection ranked #1 LLM security risk. Motivates taint tracking for untrusted data. |
| Debenedetti et al. 2025 (arXiv:2503.18813) | CaMeL: capability-based authorization separating control from data flow. Aligns with provenance-based taint analysis. |
| C2PA (Coalition for Content Provenance and Authenticity) | Content authenticity standard using hash + signature. Roko's attestation model aligns with C2PA manifest format. |
| Sabelfeld & Myers 2003, IEEE S&P | Language-based information flow security. Theoretical foundation for taint tracking. |
| Denning 1976, Communications of the ACM | Lattice model for information flow control. Taint levels form a trust lattice. |

---

## Current Status and Gaps

- **Implemented**: Provenance struct with author, trust, tainted, session. Four constructors
  (trusted, agent, external, user). `is_trusted()` check. `with_trust()`, `with_session()`,
  `with_taint()` builders. Fully tested in `roko-core`.
- **Implemented**: Provenance included in ContentHash (author + tainted).
- **Not yet implemented**: Attestation struct and `attestation` field on Engram.
- **Not yet implemented**: Extended Provenance fields (model_fingerprint, prompt_hash,
  graduated TaintLevel).
- **Not yet implemented**: Taint propagation rules in Composer implementations.

---

## Cross-References

- [02-engram-data-type.md](02-engram-data-type.md) — Provenance as a field on the Engram
- [03-score-7-axis-appraisal.md](03-score-7-axis-appraisal.md) — Reputation axis initialized from trust
- [14-c-factor-collective-intelligence.md](14-c-factor-collective-intelligence.md) — Trust in agent mesh
- [17-design-principles-and-frontier-summary.md](17-design-principles-and-frontier-summary.md) — Forensic AI innovation
