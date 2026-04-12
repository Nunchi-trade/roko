# 15 — Regulatory Moat & Current Status

> Roko's content-addressed provenance architecture creates a natural regulatory
> compliance moat. The Forensic AI capability enables causal replay of any agent
> decision — answering "why did the agent do this?" with cryptographically verifiable
> evidence. This document specifies the forensic replay engine, regulatory pre-compliance
> capabilities, pre-certified agent templates, the competitive moat analysis, and the
> current implementation status of the entire identity-economy layer.

---

## 1. Forensic AI — Causal Replay Engine

### 1.1 The Capability Nobody Else Has

When an agent takes an action that causes harm — a bad trade, a security
vulnerability introduced, a biased recommendation — regulators and auditors need to
answer three questions:

1. **Why** did the agent do this?
2. **What information** led to this decision?
3. **Who is accountable?**

Every other agent framework treats these as unsolvable problems. Roko solves them
natively through the Synapse Architecture's content-addressed provenance chain.

### 1.2 Causal Replay Protocol

Take any agent action and replay the exact decision context:

```
Causal Replay Steps:

1. IDENTIFY the action
   → Every agent action produces an Engram with a BLAKE3 hash
   → Engram contains: kind, body, author, tags, lineage
   → lineage = Vec<Blake3Hash> pointing to parent Engrams

2. TRACE the lineage DAG
   → Follow lineage hashes backward through the Engram graph
   → Each Engram in the chain is content-addressed
   → If any Engram has been modified, its BLAKE3 hash won't match
   → The replay is cryptographically tamper-evident

3. RECONSTRUCT the decision context
   → Which Engrams were in the Substrate at the time of the decision?
   → Which Scores were computed by which Scorers?
   → Which Router selected which candidate, with what confidence?
   → Which Composer assembled the context, under what budget?
   → Which Gate verified the output, with what verdict?
   → Which Policy fired, emitting what Engrams?

4. VERIFY integrity
   → Recompute BLAKE3 hashes for every Engram in the chain
   → Any mismatch → tamper detected → flag for investigation
   → Hash verification is O(n) where n = chain length, ~1μs per hash

5. PRODUCE audit trail
   → Generate human-readable report of the decision path
   → Include: all input data, all intermediate computations,
     all model calls (with prompts and responses), all gate verdicts
   → Report is itself an Engram (content-addressed, tamper-evident)
```

### 1.3 What Gets Recorded

Every Synapse trait operation produces auditable records:

| Synapse Trait | What's Recorded | Retention |
|---|---|---|
| **Substrate** | Engram reads and writes, query parameters, results | Permanent (content-addressed) |
| **Scorer** | Score computation inputs, weights, outputs, confidence | Per-episode (JSONL) |
| **Gate** | Verdict (pass/fail/skip), evidence, threshold used, gate type | Permanent (tamper-evident) |
| **Router** | Candidate list, selection, confidence, model used | Per-episode |
| **Composer** | Context budget, sections included/excluded, VCG bids | Per-episode |
| **Policy** | Rule triggered, decision (permit/deny/modify), context | Permanent (compliance-critical) |

### 1.4 Engram Lineage DAG

Every Engram contains a lineage field that creates a directed acyclic graph:

```rust
pub struct Engram {
    pub hash: Blake3Hash,             // BLAKE3(kind + body + author + tags)
    pub kind: Kind,                   // 28 variants
    pub body: Vec<u8>,                // content
    pub author: AgentId,
    pub tags: Vec<String>,
    pub lineage: Vec<Blake3Hash>,     // parent Engram hashes
    pub score: [f64; 7],              // 7-axis quality score
    pub tier: Tier,                   // Transient → Permanent
    pub created_at: u64,
    pub provenance: Provenance,       // origin metadata
}

pub struct Provenance {
    pub source: ProvenanceSource,     // HumanInput, AgentGenerated, Restored, etc.
    pub original_author: Option<AgentId>,
    pub original_timestamp: Option<u64>,
    pub chain_of_custody: Vec<CustodyEntry>,
}

pub struct CustodyEntry {
    pub agent: AgentId,
    pub action: CustodyAction,       // Created, Modified, Shared, Restored
    pub timestamp: u64,
    pub hash_at_action: Blake3Hash,   // hash when this custody event occurred
}
```

The lineage DAG enables both forward tracing ("what did this Engram influence?") and
backward tracing ("what inputs produced this Engram?").

---

## 2. Regulatory Pre-Compliance

### 2.1 Mapping to Regulatory Requirements

Roko's architecture natively satisfies requirements that other frameworks must
bolt on after the fact:

| Regulation | Requirement | Roko's Native Capability |
|---|---|---|
| **EU AI Act (Article 14)** | Human oversight mechanisms | Cognitive Signals (Pause, Resume, Escalate) + Gate architecture. Any agent can be paused mid-execution without data loss. |
| **EU AI Act (FRIA)** | Fundamental rights impact assessment | Pre-deployment simulation through synthetic scenarios. Gate pipeline produces quantified risk assessment. |
| **EU AI Act (Article 13)** | Transparency and information provision | Full Engram lineage DAG provides complete decision provenance. Every output is traceable to its inputs. |
| **SEC/CFTC** | Trading decision reconstruction | Complete Engram lineage from market data → analysis → trade. Content-addressed — cannot be altered after the fact. |
| **MiFID II** | Best execution documentation | Router decisions logged with candidate set, selection rationale, and confidence. Demonstrates systematic best-execution process. |
| **HIPAA** | Audit trail for clinical decisions | Content-addressed provenance chain. PHI-aware Gate prevents data leakage. |
| **SOX** | Financial control documentation | Tamper-proof Gate verdict history. Every financial decision passes through auditable gates. |
| **GDPR (Article 22)** | Right to explanation of automated decisions | Causal replay produces human-readable explanation of any decision. |
| **GDPR (Article 17)** | Right to erasure | Knowledge backup/restore architecture supports selective deletion with provenance tracking. |

### 2.2 Content-Addressed Compliance

The key insight: **content-addressing makes compliance verifiable by default**.

Traditional AI audit trails rely on mutable databases — logs can be altered, timestamps
can be faked, evidence can be deleted. Roko's BLAKE3 content-addressing means:

- Every Engram's hash depends on its content — any modification changes the hash
- The lineage DAG creates an immutable chain — modifying any intermediate Engram
  breaks all downstream hashes
- On-chain anchoring (when enabled) provides timestamping via Korai block headers
- TEE attestation proves computation integrity

A regulator can independently verify the entire decision chain by recomputing hashes.
No trust in the operator is required — the math is the proof.

### 2.3 Compliance Cost Analysis

```
Traditional AI compliance:
  - Manual audit trail construction: $200-500K/year
  - Third-party auditor fees: $100-300K/year
  - Compliance team: 3-5 FTEs × $150K = $450-750K/year
  - Total: $750K-1.5M/year per regulated entity

Roko-native compliance:
  - Automated audit trails: $0 (built into architecture)
  - Report generation: ~$100/report (LLM + template)
  - Compliance monitoring: 1 FTE × $150K/year (review, not build)
  - Total: $200-400K/year per regulated entity

  Savings: 60-75% reduction in compliance costs
```

**Enterprise value**: $100-500K/month per regulated enterprise. A single compliance
failure costs $10M-$1B in fines. Roko is insurance against catastrophic regulatory
penalties.

---

## 3. Pre-Certified Agent Templates

### 3.1 Concept

Build agent configurations for specific regulatory regimes with compliance encoded
in Policy traits. Once a configuration is certified by a regulator or auditor, the
configuration becomes a moat — switching costs are astronomical because
re-certification takes months.

### 3.2 Template Catalog

#### SEC-Compliant Trading Agent

```rust
pub struct SecTradingTemplate {
    // Required Policy traits
    pub best_execution_policy: BestExecutionPolicy,    // MiFID II / Reg NMS
    pub position_limit_policy: PositionLimitPolicy,    // concentration limits
    pub wash_trading_detector: WashTradingDetector,    // market manipulation check
    pub insider_trading_screen: InsiderTradingScreen,  // information barrier
    pub audit_trail_policy: AuditTrailPolicy,          // full decision capture

    // Required Gate traits
    pub compliance_gate: ComplianceGate,               // pre-trade compliance
    pub risk_gate: RiskGate,                           // position risk check
    pub reporting_gate: ReportingGate,                 // regulatory reporting

    // Configuration
    pub max_position_pct: f64,     // max % of portfolio in single asset
    pub max_daily_turnover: u64,   // USDC daily trading limit
    pub mandatory_cooling: u64,    // seconds between correlated trades
}
```

#### HIPAA-Compliant Clinical Agent

```rust
pub struct HipaaClinicalTemplate {
    // Required Policy traits
    pub phi_detection_policy: PhiDetectionPolicy,      // detect PHI in outputs
    pub minimum_necessary_policy: MinNecessaryPolicy,   // minimum necessary standard
    pub consent_tracking_policy: ConsentTrackingPolicy, // patient consent verification
    pub break_glass_policy: BreakGlassPolicy,          // emergency override with logging

    // Required Gate traits
    pub phi_leakage_gate: PhiLeakageGate,              // block PHI in outputs
    pub audit_gate: AuditGate,                         // HIPAA audit trail
    pub access_control_gate: AccessControlGate,        // role-based access

    // Privacy
    pub privacy_tier: PrivacyTier,                     // minimum Tier 2.5 or 3
    pub data_retention: Duration,                      // max retention period
}
```

#### GDPR-Compliant Data Agent

```rust
pub struct GdprDataTemplate {
    // Required Policy traits
    pub purpose_limitation_policy: PurposeLimitationPolicy,
    pub data_minimization_policy: DataMinimizationPolicy,
    pub consent_verification_policy: ConsentVerificationPolicy,
    pub erasure_policy: ErasurePolicy,                 // right to be forgotten
    pub portability_policy: PortabilityPolicy,         // data portability

    // Required Gate traits
    pub cross_border_gate: CrossBorderGate,            // data transfer checks
    pub retention_gate: RetentionGate,                 // auto-delete expired data
    pub explanation_gate: ExplanationGate,             // Article 22 explanations

    // Configuration
    pub data_categories: Vec<DataCategory>,
    pub legal_bases: Vec<LegalBasis>,
    pub retention_periods: HashMap<DataCategory, Duration>,
}
```

### 3.3 Certification Moat

The certification process creates a durable competitive moat:

```
1. DEVELOP template with regulatory expert input
   → 6-12 months of development and testing

2. AUDIT by accredited third party
   → 3-6 months of security and compliance review

3. CERTIFY with regulatory body (where applicable)
   → Varies by jurisdiction and regulation

4. MAINTAIN certification
   → Annual re-audit
   → Continuous monitoring updates
   → Regulatory change tracking

5. MOAT EFFECT
   → Competitor must replicate steps 1-4
   → Customer switching requires re-certification
   → Certified configuration becomes the standard
   → Network effects: more users → more auditor familiarity → easier certification
```

---

## 4. Competitive Landscape

### 4.1 What Competitors Lack

| Capability | Roko | LangChain | AutoGPT | CrewAI | Microsoft Autogen |
|---|---|---|---|---|---|
| Content-addressed provenance | Native (BLAKE3) | None | None | None | None |
| Causal replay | Full DAG traversal | Log files only | None | None | Log files only |
| Regulatory pre-compliance | Built-in templates | Manual | None | None | Azure compliance (cloud-only) |
| Tamper-evident audit trail | Cryptographic | None | None | None | None |
| On-chain anchoring | Korai chain | None | None | None | None |
| Privacy tiers | 4 tiers (Valhalla) | None | None | None | Azure AD |
| Economic accountability | Staking + slashing | None | None | None | None |

### 4.2 Why This Moat Is Durable

1. **Architectural** — compliance is woven into the Synapse Architecture traits, not
   bolted on. Competitors would need to redesign their core abstractions.

2. **Economic** — staking and slashing create financial accountability that pure-software
   frameworks cannot replicate without a token economy.

3. **Regulatory** — once certified, switching costs are enormous. Re-certification
   takes months and costs hundreds of thousands of dollars.

4. **Network** — collective calibration (C-Factor) means more agents on Roko make
   every agent better. Competitors with fewer agents have worse knowledge quality.

5. **Data** — the Korai knowledge chain accumulates verified knowledge over time.
   This data flywheel is defensible and compounds.

---

## 5. Implementation Status — Full Identity-Economy Layer

### 5.1 Status Overview

> **Overall status (2026-04-12)**: The identity-economy layer is fully designed and
> documented but not yet implemented in code. The Roko framework currently operates
> as a CLI-based agent toolkit with direct dispatch. Chain infrastructure (Korai,
> Daeji, mirage-rs), on-chain identity (ERC-8004), and the job marketplace are
> deferred to Tier 5-6 in the implementation plan. The current development focus
> is on Tier 1 (model routing) and Tier 2 (cognitive integration).

### 5.2 Component Status Matrix

| Component | Design Status | Code Status | Tier | Priority |
|---|---|---|---|---|
| **ERC-8004 Three Registries** | Complete | Not started | 5 | P2 |
| **Korai Passport** | Complete | AgentEntry stub in mirage-rs | 5 | P2 |
| **Passport Tiers** | Complete | Not started | 5 | P2 |
| **7-Domain EMA Reputation** | Complete | Not started | 5 | P2 |
| **Knowledge Marketplace** | Complete | Not started | 5 | P2 |
| **Commerce Bazaar** | Complete | Not started | 6 | P3 |
| **MPP (Machine Payment Protocol)** | Complete | Cost tracking exists in roko-learn | 5 | P2 |
| **x402 Micropayments** | Complete | Not started | 6 | P3 |
| **Agent Economy** | Complete | Cost/efficiency events wired in roko-learn | 5 | P2 |
| **KORAI Tokenomics** | Complete | GNOS stub in mirage-rs (needs rename) | 6 | P3 |
| **Vickrey Reputation Auction** | Complete (with proof) | Not started | 6 | P3 |
| **Three Hiring Models** | Complete | Direct dispatch in CLI (no auction) | 6 | P3 |
| **ISFR Clearing & Settlement** | Complete | Not started | 6 | P3 |
| **Knowledge Futures Market** | Complete | Not started | 6 | P3 |
| **Forensic AI / Causal Replay** | Complete | Engram lineage DAG exists in roko-core | 5 | P2 |
| **Pre-Certified Templates** | Design only | Not started | 6 | P3 |

### 5.3 What Exists Today

The following components are **built and wired** in the current codebase:

```
roko-core:
  ✅ Signal (will be renamed to Engram) with BLAKE3 hash and lineage
  ✅ 6 Synapse traits (Substrate, Scorer, Gate, Router, Composer, Policy)
  ✅ 28 Kind variants for typed Engrams
  ✅ 7-axis scoring

roko-learn:
  ✅ Episode logger (.roko/episodes.jsonl)
  ✅ Cost tracking per request/plan/run
  ✅ Efficiency events (.roko/learn/efficiency.jsonl)
  ✅ CascadeRouter with persistent model routing

roko-gate:
  ✅ 11 gate types with 6-rung pipeline
  ✅ Adaptive thresholds (EMA per rung)
  ✅ Gate verdicts with evidence hashes

roko-fs:
  ✅ FileSubstrate (JSONL persistence)
  ✅ Content-addressed storage

mirage-rs (separate repo):
  ✅ In-process EVM simulator
  ✅ Fork mode for mainnet Ethereum
  ✅ 141 tests
  ✅ AgentEntry stub (needs Passport expansion)
  ✅ Token stub (needs KORAI/DAEJI rename)
```

### 5.4 What Needs to Be Built

Ordered by implementation tier:

**Tier 5 (Agent Mesh & Chain — P2)**:
- Korai chain deployment (or Daeji testnet)
- ERC-8004 smart contracts (3 registries)
- Korai Passport with full fields
- Agent Mesh (WebSocket + Iroh P2P)
- Reputation system (7-domain EMA)
- Gossip mesh (GossipSub v1.1 or mirage-rs mock)
- Basic payment infrastructure (DAEJI token, escrow)

**Tier 6 (Advanced Economy — P3)**:
- KORAI token with demurrage (WAD arithmetic, Solidity contracts)
- Vickrey auction engine
- Three hiring models (BountySpec, SparrowBid, dispatch)
- Job state machine with timeout fallbacks
- Consortium validation (commit-reveal)
- ISFR collective price discovery
- Cooperative clearing (QP solver, ClearingCertificate)
- x402 micropayment integration
- Knowledge marketplace (3-tier Bazaar)
- Knowledge Futures Market
- Pre-certified agent templates

### 5.5 Dependencies

```
Tier 5 depends on:
  ✅ Tier 1 (Model Routing) — in progress
  ✅ Tier 2 (Cognitive Integration) — partially started
  ⬜ Tier 3 (Agent Platform) — roko-serve, roko-plugin
  ⬜ Tier 4 (Interfaces) — TUI, web portal

Tier 6 depends on:
  ⬜ Tier 5 (Agent Mesh & Chain) — full prerequisite
```

### 5.6 Implementation Approach

The identity-economy layer will be implemented via the Roko self-hosting workflow:

```bash
# 1. Generate PRDs for each component
roko prd draft new "erc-8004-three-registries"
roko prd draft new "korai-passport"
roko prd draft new "reputation-7-domain-ema"
# ... (one PRD per component)

# 2. Research and enhance
roko research enhance-prd erc-8004-three-registries

# 3. Generate implementation plans
roko prd plan erc-8004-three-registries

# 4. Execute (agent-driven, gate-verified)
roko plan run plans/

# 5. Validate
# Each component's verification criteria from 12b-chain-layer.md
# serve as the gate pass conditions
```

The identity-economy layer is designed to be built by agents using Roko itself —
dogfooding the framework to build its own economic infrastructure.

---

## 6. Roadmap to Series A

### 6.1 What a16z Needs to See

For a Series A raise (see `00-vision-and-a16z-framing.md`), the identity-economy
layer needs to demonstrate:

1. **Working agent identity** — Korai Passport minted, capabilities declared,
   reputation tracked (even on testnet with DAEJI tokens)

2. **Working job market** — at least one hiring model functional (Vickrey auction
   preferred for its game-theoretic properties)

3. **Measurable collective intelligence** — C-Factor > 1.0 consistently, meaning
   agents working together outperform the sum of individuals

4. **Revenue model** — agents earning and spending DAEJI, with fee economics that
   demonstrate sustainable unit economics

5. **Compliance moat** — at least one pre-certified template (SEC trading or GDPR
   data) with third-party audit letter

### 6.2 Timeline Dependencies

```
Now:          Tier 1 (model routing) + Tier 2 (cognitive)
Next:         Tier 3 (platform) + Tier 4 (interfaces)
Then:         Tier 5 (mesh + chain basics)
Series A:     Tier 6 (full economy) — with working demo on Daeji testnet
Post-Series:  Korai mainnet launch, KORAI token, production economy
```

### 6.3 KYA Narrative

The Series A pitch centers on **KYA — Know Your Agent**:

> In a world where AI agents control billions of dollars, operate critical
> infrastructure, and make decisions that affect human lives, the question is not
> "how smart is your agent?" but "how well do you know your agent?"
>
> Roko answers this question with cryptographic certainty. Every agent has a
> verifiable identity (Korai Passport), a quantified track record (7-domain EMA
> reputation), economic accountability (staking and slashing), and complete
> decision provenance (Forensic AI causal replay).
>
> No other framework can answer "why did the agent do this?" with mathematical
> proof. No other framework makes agents economically accountable for their
> actions. No other framework creates a certified compliance moat that turns
> regulatory burden into competitive advantage.
>
> This is not a feature — it is the foundation.

---

## 7. Academic Citations

- EU AI Act 2024 — Regulation (EU) 2024/1689 (artificial intelligence act)
- SEC Rule 17a-4 — Records to be preserved by certain exchange members, brokers,
  and dealers
- MiFID II 2014/65/EU — Markets in Financial Instruments Directive (best execution,
  pre/post trade transparency)
- HIPAA 1996 — Health Insurance Portability and Accountability Act (audit trail,
  minimum necessary standard)
- SOX 2002 — Sarbanes-Oxley Act (internal controls, financial reporting)
- GDPR 2016/679 — General Data Protection Regulation (right to explanation, right
  to erasure, purpose limitation)
- CFTC Rule 1.31 — Books and records, data retention
- Vickrey 1961 — Counterspeculation, Auctions, and Competitive Sealed Tenders
- Woolley et al. 2010 — Evidence for a Collective Intelligence Factor (Science
  330(6004))
- Kanerva 2009 — Hyperdimensional Computing (Cognitive Computation 1(2))
- Chen et al. 2023 — FrugalGPT (arXiv:2305.05176)
- Lee et al. 2026 — Meta-Harness (arXiv:2603.28052)

---

## 8. Cross-References

| Topic | Document |
|---|---|
| Vision and a16z framing | `00-vision-and-a16z-framing.md` |
| ERC-8004 registries | `01-erc-8004-three-registries.md` |
| Korai Passport | `02-korai-passport.md` |
| Passport tiers | `03-passport-tiers.md` |
| Reputation system | `04-reputation-7-domain-ema.md` |
| Knowledge marketplace | `05-knowledge-marketplace.md` |
| Commerce Bazaar | `06-commerce-bazaar.md` |
| Machine Payment Protocol | `07-mpp-machine-payment-protocol.md` |
| x402 micropayments | `08-x402-micropayments.md` |
| Agent economy | `09-agent-economy.md` |
| KORAI tokenomics | `10-korai-tokenomics.md` |
| Vickrey auction | `11-vickrey-reputation-auction.md` |
| Three hiring models | `12-three-hiring-models.md` |
| ISFR clearing & settlement | `13-isfr-clearing-settlement.md` |
| Knowledge futures market | `14-knowledge-futures-market.md` |

---

*Generated from: refactoring-prd/09-innovations.md §IX, refactoring-prd/07-implementation-priorities.md,
tmp/implementation-plans/12b-chain-layer.md (all sections), bardo-backup/prd/10-safety/.
All naming renames applied.*
