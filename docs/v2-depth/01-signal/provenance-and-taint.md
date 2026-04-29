# Provenance and Taint

> Depth for [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;8. Redesigns provenance as a lattice-based information flow control (IFC) system where taint propagation is join in a security lattice and custody records act as dependent-type witnesses.

---

## 1. The Information Flow Problem

Every Signal in Roko carries data from *somewhere*. A task description comes from a user. An insight comes from an LLM. A verdict comes from a gate. A causal link comes from a consolidation algorithm. These origins have different trust levels, and trust must propagate conservatively through the lineage DAG.

The naive approach -- a boolean `tainted: bool` -- is too coarse. It cannot distinguish between user input (intentional, low-risk) and LLM hallucination (unintentional, high-risk). It cannot express that a Signal is tainted *because* it consumed tainted input (propagated taint) vs. tainted *intrinsically* (source taint).

The right framework is **information flow control (IFC)**: a lattice of security labels where information can flow from lower to higher labels but never from higher to lower without explicit declassification. Taint propagation is join in this lattice.

---

## 2. The Taint Lattice

Define a lattice of taint levels ordered by trust (lower = more trusted):

```
                    Propagated
                   /          \
        LlmGenerated      ExternalFetch
                   \          /
                  UserInput
                      |
                    Clean
```

In lattice notation:

```
Clean < UserInput < {LlmGenerated, ExternalFetch} < Propagated
```

Where `LlmGenerated` and `ExternalFetch` are incomparable (neither is strictly more trusted than the other), and `Propagated` is the top of the lattice (most tainted).

```rust
/// Taint levels forming a security lattice.
///
/// The lattice ordering is: Clean < UserInput < {LlmGenerated, ExternalFetch} < Propagated
/// Where LlmGenerated and ExternalFetch are incomparable.
///
/// Taint propagation is join (least upper bound) in this lattice.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Taint {
    /// No taint. Data originates from a verified, trusted source.
    /// Gates, system internals, deterministic computations.
    Clean,

    /// User-provided input: prompts, file uploads, inline instructions.
    /// Intentional but unverified. Low risk for most operations.
    UserInput { detail: String },

    /// External data fetch: API response, web scrape, webhook payload.
    /// Unverified provenance. Moderate risk.
    ExternalFetch { url: Option<String>, detail: String },

    /// LLM-generated content that may contain hallucinated facts.
    /// High risk for factual claims. Low risk for code generation
    /// (where gates can verify).
    LlmGenerated { model: String, detail: String },

    /// Taint inherited from upstream Signals in the lineage DAG.
    /// The `max_taint` field records the highest taint level
    /// among all contributing ancestors.
    Propagated {
        max_upstream: Box<Taint>,
        inherited_from: Vec<ContentHash>,
    },

    /// Stale data: exceeded a configured freshness window.
    /// A temporal taint that can be cleared by re-fetching.
    StaleData { threshold_ms: i64 },

    /// Explicitly flagged by a human operator.
    /// Cannot be automatically cleared.
    UserFlagged { reason: String },

    /// Tool failure: the producing tool reported an error or
    /// returned suspect data.
    ToolFailure { tool: String, detail: String },

    /// Extension point for domain-specific taint.
    Custom(String),
}
```

### 2.1 The Join Operation

Taint propagation is join (least upper bound) in the lattice. When a Compose Cell combines Signals with different taint levels, the output's taint is the join of all input taints.

```rust
impl Taint {
    /// Lattice ordering: returns true if self <= other.
    /// Clean is bottom. Propagated is top.
    pub fn flows_to(&self, other: &Taint) -> bool {
        match (self, other) {
            // Clean flows to everything
            (Taint::Clean, _) => true,
            // Everything flows to Propagated
            (_, Taint::Propagated { .. }) => true,
            // UserInput flows to LlmGenerated, ExternalFetch
            (Taint::UserInput { .. }, Taint::LlmGenerated { .. }) => true,
            (Taint::UserInput { .. }, Taint::ExternalFetch { .. }) => true,
            // Same level flows to same level
            (a, b) if std::mem::discriminant(a) == std::mem::discriminant(b) => true,
            // Nothing else flows
            _ => false,
        }
    }

    /// Lattice join: least upper bound of two taint levels.
    /// This is the taint of a Signal derived from two inputs.
    pub fn join(a: &Taint, b: &Taint) -> Taint {
        if a.flows_to(b) {
            b.clone()
        } else if b.flows_to(a) {
            a.clone()
        } else {
            // Incomparable: promote to Propagated
            Taint::Propagated {
                max_upstream: Box::new(if a.severity() >= b.severity() {
                    a.clone()
                } else {
                    b.clone()
                }),
                inherited_from: Vec::new(),
            }
        }
    }

    /// Join across multiple taint levels.
    pub fn join_all(taints: &[Taint]) -> Taint {
        taints.iter().fold(Taint::Clean, |acc, t| Taint::join(&acc, t))
    }

    /// Numeric severity for tie-breaking when elements are incomparable.
    fn severity(&self) -> u8 {
        match self {
            Taint::Clean => 0,
            Taint::UserInput { .. } => 1,
            Taint::ExternalFetch { .. } => 2,
            Taint::LlmGenerated { .. } => 2,
            Taint::StaleData { .. } => 2,
            Taint::ToolFailure { .. } => 3,
            Taint::UserFlagged { .. } => 4,
            Taint::Propagated { .. } => 5,
            Taint::Custom(_) => 3,
        }
    }
}
```

### 2.2 Propagation Rules

The lattice join enforces conservative taint propagation automatically:

1. **Compose preserves taint**: When a Compose Cell assembles a prompt from multiple Signals, the prompt Signal's taint is `Taint::join_all(inputs.map(|s| s.provenance.taint))`. If any input is LlmGenerated, the prompt is at least LlmGenerated.

2. **Derivation preserves taint**: When a new Signal is derived from an existing one (via `Signal::derive()`), the derived Signal inherits the parent's taint unless explicitly declassified.

3. **Taint only increases**: In the lattice, information flows upward. A Clean Signal that consumes LlmGenerated input becomes at least LlmGenerated. It cannot become Clean again without explicit human action.

4. **Verify does not clear taint**: A Verify verdict can *validate* a tainted Signal (increasing confidence), but validation does not erase the taint. The Signal's provenance still records that tainted material participated. Taint is a *historical fact*, not a current assessment.

```rust
/// Taint-aware composition.
///
/// The Compose Cell propagates taint from all input Signals
/// to the output Signal via lattice join.
pub fn compose_with_taint(
    inputs: &[Signal],
    compose_fn: impl Fn(&[Signal]) -> Value,
) -> Signal {
    let composed_payload = compose_fn(inputs);
    let composed_taint = Taint::join_all(
        &inputs.iter().map(|s| s.provenance.taint.clone()).collect::<Vec<_>>()
    );
    let inherited_from: Vec<ContentHash> = inputs.iter()
        .filter(|s| !matches!(s.provenance.taint, Taint::Clean))
        .map(|s| s.content_hash)
        .collect();

    let taint = if inherited_from.is_empty() {
        composed_taint
    } else {
        Taint::Propagated {
            max_upstream: Box::new(composed_taint),
            inherited_from,
        }
    };

    Signal::builder(Kind::Text)
        .payload(composed_payload)
        .provenance(Provenance {
            taint,
            ..Provenance::system()
        })
        .source(inputs.iter().map(|s| s.ref_()).collect())
        .build()
}
```

---

## 3. Declassification: The Only Way Down

Taint can only decrease through **explicit declassification** -- a human-initiated action recorded in the audit trail. Declassification is not automatic; it is a policy decision.

```rust
/// Declassify a tainted Signal.
///
/// This is the ONLY path that reduces taint.
/// It requires a Custody record proving that a human
/// reviewed and approved the declassification.
pub fn declassify(
    signal: &mut Signal,
    new_taint: Taint,
    custody: Custody,
    store: &dyn Store,
) -> Result<()> {
    // Validate: new_taint must be strictly lower than current
    if !new_taint.flows_to(&signal.provenance.taint) {
        return Err(Error::DeclassificationNotLower {
            current: signal.provenance.taint.clone(),
            requested: new_taint,
        });
    }

    // Validate: custody must have human authorization
    if !custody.authorized.is_human() {
        return Err(Error::DeclassificationRequiresHuman);
    }

    // Record the declassification as a Custody Signal
    let custody_signal = Signal::builder(Kind::Evidence {
        kind: EvidenceKind::Declassification,
    })
        .payload(json!({
            "target": signal.content_hash.to_hex(),
            "from_taint": format!("{:?}", signal.provenance.taint),
            "to_taint": format!("{:?}", new_taint),
        }))
        .provenance(Provenance {
            taint: Taint::Clean,
            author: Author::System,
            ..Default::default()
        })
        .source(vec![signal.ref_()])
        .build();

    store.put(custody_signal).await?;

    // Apply the declassification
    signal.provenance.taint = new_taint;
    store.update_metadata(signal).await?;

    Ok(())
}
```

---

## 4. Custody Records as Dependent Types

A `Custody` record is a witness that a specific action was authorized. In type-theoretic terms, Custody is a **dependent type**: the type `Custody<A>` depends on the action `A` it authorizes. You cannot call `Store.put()` for a privileged Signal without holding a valid `Custody` witness.

```rust
/// Custody is a dependent witness: it proves that a specific action
/// was authorized by a specific principal at a specific time.
///
/// Custody is itself a Signal (persisted in Store for audit).
/// It is the bridge between "someone decided to do X" and
/// "the system has proof that X was authorized."
pub struct Custody {
    /// What action was authorized.
    pub action: ActionId,

    /// Who authorized it.
    pub principal: Principal,

    /// When the authorization was granted.
    pub when: DateTime<Utc>,

    /// What evidence supports the authorization.
    pub authorized: AuthzEvidence,

    /// Which heuristics informed the decision.
    pub why_heuristics: Vec<SignalRef>,

    /// Which claims or findings justified it.
    pub why_claims: Vec<SignalRef>,

    /// Optional dry-run evidence.
    pub simulation: Option<SignalRef>,

    /// Which Verify Cells passed before the action.
    pub gates_passed: Vec<SignalRef>,

    /// Pointer to the result Signal (filled after execution).
    pub result: Option<SignalRef>,

    /// Optional external witness (e.g., chain witness).
    pub witness: Option<ExternalWitness>,
}

/// Authorization evidence: what allowed this action?
pub enum AuthzEvidence {
    /// Role-based: the principal holds the required role.
    RoleGrant { role: String, scope: String },

    /// Human confirmation: a human explicitly approved.
    HumanConfirmation { confirmer: String, channel: String },

    /// Escalation: a lower-level denial was overridden.
    Escalation { original_denial: SignalRef, override_by: String },

    /// Session approval: approved for the duration of the session.
    SessionApproval { session_id: String },

    /// Automatic: policy allows without confirmation.
    Automatic { policy: String },
}
```

### 4.1 Custody-Gated Store

The key architectural insight: certain Store operations should be *gated on custody*. You cannot persist a destructive action's result without a Custody witness proving authorization.

```rust
/// A Store wrapper that requires Custody for privileged operations.
///
/// This is the type-level enforcement of "privileged actions
/// require authorization proof."
pub struct CustodyGatedStore<S: Store> {
    inner: S,
    /// Kinds that require Custody for Store.put().
    privileged_kinds: HashSet<Kind>,
}

impl<S: Store> CustodyGatedStore<S> {
    /// Put a Signal into Store.
    ///
    /// If the Signal's kind is privileged, a valid Custody
    /// record must be provided. The Custody record is itself
    /// stored alongside the Signal.
    pub async fn put(
        &self,
        signal: Signal,
        custody: Option<Custody>,
    ) -> Result<SignalRef> {
        if self.privileged_kinds.contains(&signal.kind) {
            let custody = custody.ok_or(Error::CustodyRequired {
                kind: signal.kind.clone(),
            })?;

            // Validate custody
            self.validate_custody(&custody, &signal)?;

            // Store the custody record first
            let custody_signal = custody.to_signal(&signal);
            self.inner.put(custody_signal).await?;
        }

        // Store the Signal itself
        self.inner.put(signal).await
    }

    fn validate_custody(
        &self,
        custody: &Custody,
        signal: &Signal,
    ) -> Result<()> {
        // 1. Custody must reference this action
        // 2. Principal must have required role
        // 3. Custody must not be expired
        // 4. All referenced gates must have passed
        Ok(())
    }
}

/// Default privileged kinds: anything that modifies external state
/// or makes irreversible decisions.
pub fn default_privileged_kinds() -> HashSet<Kind> {
    let mut kinds = HashSet::new();
    kinds.insert(Kind::Evidence { kind: EvidenceKind::Declassification });
    kinds.insert(Kind::Evidence { kind: EvidenceKind::Deployment });
    kinds.insert(Kind::Evidence { kind: EvidenceKind::ExternalWrite });
    kinds.insert(Kind::Evidence { kind: EvidenceKind::NetworkEgress });
    // Destructive file operations
    kinds.insert(Kind::Evidence { kind: EvidenceKind::FileDelete });
    kinds
}
```

---

## 5. Attestation Levels

Attestation is the cryptographic layer above custody. It proves that a specific signer committed to a specific Signal.

```rust
/// Attestation: cryptographic proof of signer and integrity.
///
/// Three levels with increasing assurance:
///   LocalAgent < OrgRole < ChainWitness
pub struct Attestation {
    /// Who signed.
    pub signer: PublicKey,
    /// Ed25519 signature over the Signal's content_hash.
    pub signature: Ed25519Signature,
    /// Which content_hash was signed.
    pub signed_hash: ContentHash,
    /// When the attestation was created.
    pub timestamp: DateTime<Utc>,
    /// Assurance level.
    pub level: AttestationLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttestationLevel {
    /// Signed by the agent's ephemeral session key.
    /// Low friction, low assurance.
    LocalAgent,
    /// Signed by a human-held organizational key.
    /// Medium friction, medium assurance.
    OrgRole,
    /// Witnessed on-chain by an independent verifier.
    /// High friction, high assurance.
    ChainWitness,
}
```

### 5.1 Attestation and Taint Independence

Attestation and taint are orthogonal:

- A Signal can be `Taint::LlmGenerated` AND attested at `LocalAgent` level. The attestation proves *who generated it*, not *whether the content is trustworthy*.
- A Signal can be `Taint::Clean` AND unattested. Clean taint means the source is trusted; attestation means the integrity is cryptographically provable. These are different properties.

```
                Attestation
                    |
      Integrity     |     Trust
      (who signed)  |     (where data came from)
                    |
                  Taint
```

The design principle: **taint tracks trust; attestation tracks integrity**. Both are needed for full provenance, but they serve different purposes and must not be conflated.

---

## 6. Cross-Space Taint

When Signals move between Spaces (workspaces), their taint must be considered in the receiving Space's context. A Signal that is `Taint::Clean` in Space A (where the agent is trusted) may need to be `Taint::ExternalFetch` in Space B (where the agent is unknown).

```rust
/// Cross-Space taint policy.
///
/// When a Signal enters a new Space, its taint is re-evaluated
/// based on the receiving Space's trust policy.
pub struct CrossSpaceTaintPolicy {
    /// Trust map: source Space -> taint level for imported Signals.
    /// Spaces not in the map use `default_import_taint`.
    pub trust_map: BTreeMap<SpaceId, Taint>,
    /// Default taint for Signals from unknown Spaces.
    pub default_import_taint: Taint,
}

impl CrossSpaceTaintPolicy {
    /// Re-taint a Signal for import into this Space.
    ///
    /// The imported Signal's taint is the join of:
    ///   1. Its original taint
    ///   2. The trust level assigned to its source Space
    pub fn import_taint(
        &self,
        signal: &Signal,
        source_space: &SpaceId,
    ) -> Taint {
        let space_taint = self.trust_map
            .get(source_space)
            .unwrap_or(&self.default_import_taint);

        Taint::join(&signal.provenance.taint, space_taint)
    }
}
```

### 6.1 Trust Domains

Cross-Space taint naturally creates **trust domains**: clusters of Spaces that trust each other's Signals. Within a trust domain, Signals flow freely with their original taint. Across trust domain boundaries, Signals are re-tainted to the boundary's trust level.

```
Trust Domain A          |  Trust Domain B
(Spaces X, Y, Z)       |  (Spaces P, Q)
                        |
  X --Clean--> Y        |  P --Clean--> Q
  Y --Clean--> Z        |
                        |
  Z --ExternalFetch-----|----> P
                        |  (re-tainted at boundary)
```

This is directly analogous to network security zones: a DMZ (trust domain boundary) re-classifies traffic from external (untrusted) to internal (trusted) based on explicit policy.

---

## 7. The Provenance Struct

Provenance bundles taint, authorship, and generation metadata into a single durable audit context on every Signal.

```rust
/// Provenance: durable audit context for a Signal.
///
/// Answers four questions:
///   1. Who produced this? (author)
///   2. How trusted is the producer? (trust)
///   3. Is the data tainted, and why? (taint)
///   4. What generation metadata exists? (generation, source_files, etc.)
pub struct Provenance {
    /// Durable producer identity.
    pub author: Author,

    /// Snapshot trust score at emission time.
    /// Later reputation changes do not rewrite history.
    pub trust: f64,

    /// Typed taint classification.
    pub taint: Taint,

    /// Optional session grouping for replay and scoping.
    pub session: Option<String>,

    /// Source code locations relevant to this Signal.
    pub source_files: Vec<SourceFileRange>,

    /// LLM generation metadata (model, temperature, prompt hash).
    pub generation: Option<GenerationProvenance>,

    /// Web fetch metadata (URL, timestamp, HTTP status).
    pub web_fetch: Option<WebFetchProvenance>,

    /// Claimed citations/references.
    pub citations: Vec<Citation>,
}

/// Author: who produced the Signal.
pub enum Author {
    /// A human user.
    User(String),
    /// An agent within the system.
    Agent(AgentId),
    /// A Verify Cell (gate).
    Gate(String),
    /// The system itself (internal computations).
    System,
    /// An external service or API.
    External(String),
    /// A wallet address (for chain interactions).
    Wallet(Address),
}

/// Generation provenance: how was this LLM output produced?
pub struct GenerationProvenance {
    pub model: String,
    pub prompt_hash: ContentHash,
    pub temperature: f64,
    pub seed: Option<u64>,
    pub tokens_used: usize,
}
```

---

## 8. Taint in the Seven-Step Loop

See [01-SIGNAL.md](../../unified/01-SIGNAL.md) &sect;13 for the Signal lifecycle. Taint participates in every step of the cognitive loop:

```rust
/// Taint's role at each loop step.
///
/// The key principle: taint is checked at ACTION time, not at
/// PERCEPTION time. Tainted data is allowed into the system
/// (it would be censorship to block it at intake). But tainted
/// data triggers additional safety checks before it can drive
/// external actions.
pub enum LoopStepTaintRole {
    /// PERCEIVE: Record taint on incoming Signals.
    /// External I/O enters with taint assigned by source.
    /// No blocking -- all data is accepted.
    Perceive,

    /// EVALUATE: Score Cells may consult taint.
    /// Tainted Signals can receive lower confidence scores,
    /// but scoring does not change or clear taint.
    Evaluate,

    /// COMPOSE: Taint propagates via lattice join.
    /// The composed prompt Signal carries the join of all
    /// input Signals' taint levels. This is where taint
    /// becomes dangerous (tainted context -> tainted prompt).
    Compose,

    /// ACT: Taint gates action.
    /// High-risk actions (network egress, file delete, chain tx)
    /// require confirmation if the context is tainted.
    /// Low-risk actions (file read, log write) proceed without
    /// extra confirmation even with tainted context.
    Act,

    /// VERIFY: Verdicts record taint state.
    /// A passing verdict on a tainted Signal does NOT clear the taint.
    /// It validates the output, not the provenance.
    Verify,

    /// PERSIST: Taint is stored as part of provenance.
    /// Custody records reference taint state at action time.
    Persist,

    /// REACT: Policy uses taint for follow-up decisions.
    /// Quarantine, escalation, re-verification triggered
    /// by taint state + action outcome.
    React,
}
```

### 8.1 Action-Time Taint Check

The critical safety check occurs at ACT time. The action's risk level determines how taint is handled:

```rust
/// Action-time taint check.
///
/// Returns the required authorization level based on
/// the action's risk and the context's taint level.
pub fn taint_gate(
    action_risk: ActionRisk,
    context_taint: &Taint,
) -> AuthorizationRequirement {
    match (action_risk, context_taint.severity()) {
        // Low-risk action + any taint: proceed
        (ActionRisk::Low, _) => AuthorizationRequirement::None,

        // Medium-risk action + clean: proceed
        (ActionRisk::Medium, 0) => AuthorizationRequirement::None,

        // Medium-risk action + tainted: require session approval
        (ActionRisk::Medium, 1..=2) => AuthorizationRequirement::SessionApproval,

        // Medium-risk action + highly tainted: require human confirmation
        (ActionRisk::Medium, 3..) => AuthorizationRequirement::HumanConfirmation,

        // High-risk action + clean: require session approval
        (ActionRisk::High, 0) => AuthorizationRequirement::SessionApproval,

        // High-risk action + any taint: require human confirmation
        (ActionRisk::High, 1..) => AuthorizationRequirement::HumanConfirmation,

        // Critical action: always require human confirmation + attestation
        (ActionRisk::Critical, _) => AuthorizationRequirement::HumanConfirmationWithAttestation,
    }
}

pub enum ActionRisk {
    Low,       // Read-only operations, logging
    Medium,    // File writes, local state changes
    High,      // Network egress, deployment, chain tx
    Critical,  // Destructive operations, key management
}

pub enum AuthorizationRequirement {
    None,
    SessionApproval,
    HumanConfirmation,
    HumanConfirmationWithAttestation,
}
```

---

## 9. What This Enables

1. **Lattice-based taint propagation**: Taint flows conservatively through the lineage DAG via join in a well-defined lattice. No taint can be silently dropped. The only path downward is explicit human declassification with a custody record.

2. **Action-gated safety**: Tainted data is not blocked at intake (that would be censorship). Instead, taint gates *actions*: the riskier the action and the more tainted the context, the stronger the authorization required. This is proportional response, not blanket denial.

3. **Cross-Space trust boundaries**: When Signals move between Spaces, taint is re-evaluated at the boundary. Trust domains enable cooperative multi-Space architectures without sacrificing provenance integrity.

4. **Custody as proof**: Custody records are dependent-type witnesses that prove authorization. They are persisted in Store alongside the Signals they authorize, creating an unforgeable audit trail.

5. **Attestation layering**: Cryptographic attestation can be applied independently of taint. A Signal can be both tainted (provenance) and attested (integrity), providing two orthogonal dimensions of trust.

---

## 10. Feedback Loops

1. **Taint -> Verify -> Confidence -> Score -> Compose -> Taint**: Tainted Signals that pass verification get higher confidence scores, which makes them more likely to be included in future compositions. But inclusion propagates their taint, creating a natural pressure to eventually declassify (if legitimate) or demote (if not).

2. **Custody -> Store -> Replay -> Audit -> Trust**: Custody records accumulate in Store. Audit replays traverse them. Successful audits increase trust in the principals who authorized actions. This creates a reputation feedback loop: principals who make good decisions earn higher trust for future authorizations.

3. **Cross-Space import -> Re-taint -> Verify -> Promote -> Export**: Signals imported from untrusted Spaces are re-tainted. If they pass local verification and earn local reinforcement, their local taint can be reduced (via declassification). Eventually they may be exported back with higher trust. This is how inter-Space trust is built incrementally.

4. **Declassification -> Audit -> Policy -> Declassification**: Each declassification is audited. If a principal declassifies too aggressively (declassified Signals later turn out to be harmful), their trust decreases and future declassification requests require stronger evidence. The system learns who is a reliable declassifier.

---

## 11. Open Questions

1. **Taint granularity**: The current lattice has ~8 levels. Is this sufficient? Some domains may need finer distinctions (e.g., `LlmGenerated` should distinguish between different models, since GPT-4 and a fine-tuned distillation have different hallucination rates). But finer granularity makes the lattice harder to reason about.

2. **Taint decay**: Should taint decay over time? A Signal that was `LlmGenerated` 6 months ago and has been verified 50 times might be treated differently from a fresh hallucination. The current design says no -- taint is a historical fact. But operational pragmatism may require time-weighted taint.

3. **Automated declassification**: The current design requires human action for declassification. Could automated declassification be safe? For example: "if a Signal passes 10 independent Verify Cells across 3 sessions, automatically declassify from LlmGenerated to Clean." This reduces operational burden but creates an attack surface (adversarial Signals designed to pass verification).

4. **Taint and HDC fingerprints**: Should taint level be encoded in the HDC fingerprint? If so, tainted Signals would be dissimilar to clean Signals even with identical content. This provides a safety margin in similarity search (tainted results are naturally down-ranked) but at the cost of complicating cross-taint-level deduplication.

5. **Custody witness storage**: Custody records are themselves Signals stored in Store. This creates recursive provenance: the Custody Signal has its own provenance, which could itself require custody for modification. Is there a fixed point? The pragmatic answer is that Custody Signals are always `Taint::Clean` and `Author::System`, breaking the recursion.
