# 06-neuro -- Gap Checklist

Spec: `docs/06-neuro/` (17 files). Code: `crates/roko-neuro/`, `crates/roko-primitives/`.

Overall: ~40-50% implemented. Core knowledge types, storage, distillation, HDC operations work. Major gaps in cross-domain transfer, structured HDC encoding, backup/restore, and collective knowledge flow.

## Compliant (no action needed)
- Vision/grimoire rename -- crate correctly named roko-neuro (doc 00)
- Six knowledge types -- all variants present with correct half-lives (doc 01)
- Type half-lives -- all constants correct (doc 03)
- HDC/VSA foundations -- 10,240-bit BSC, all 4 operations (doc 04)
- Core HDC operations -- bind, bundle, permute, similarity (doc 05 core)
- Knowledge query API -- NeuroStore trait, KnowledgeStore impl (doc 10)
- AntiKnowledge -- refutation fields, confidence floor 0.3, GC exemption (doc 11 core)

## Checklist

### NEURO-01: Explicit tier field on KnowledgeEntry
- [x] Wire promotion/demotion logic for the existing tier field

**Spec** (doc 02): Four validation tiers with multipliers that shape the effective half-life of a knowledge entry:
- Transient (0.1x) -- unvalidated observations, fast decay (~3 days for a Warning)
- Working (0.5x) -- partially confirmed, moderate decay
- Consolidated (1.0x) -- fully validated through repeated use
- Persistent (5.0x) -- battle-tested, very slow decay
Promotion rules: Transient->Working requires 2+ independent confirmations from different episodes. Working->Consolidated requires 3+ distinct contexts (different tasks/domains). Consolidated->Persistent requires explicit marking. Demotion: Persistent entries cannot be demoted without an explicit deprecation flag; other tiers demote on repeated contradiction.

**Current code**: `KnowledgeTier` enum at `crates/roko-neuro/src/lib.rs:128` with all 4 tiers and correct multipliers (0.1, 0.5, 1.0, 5.0). `KnowledgeEntry` already has `tier: KnowledgeTier` field at line 263. `TierProgression` at `crates/roko-neuro/src/tier_progression.rs:176` with `evaluate_promotion()` at line 240 and `evaluate_tier_progression()` at line 249. `promote_tier()` at line 864 and `demote_tier()` at line 872 exist. Missing: automatic promotion triggers (2+ confirmations for Transient->Working, 3+ distinct contexts for Working->Consolidated). Persistent demotion requires explicit deprecation but this isn't enforced. The `InsightRecord` at line 41 tracks `source_episodes: Vec<String>` which could provide the confirmation count, but nothing calls back to promote based on that count.

**What to change**: In `KnowledgeStore::ingest()` at `crates/roko-neuro/src/knowledge_store.rs:207`, after adding new entries, check if the new entry confirms an existing one (same `KnowledgeKind`, HDC similarity > 0.526). If so, increment a `confirmation_count` field on the existing entry and call `promote_tier()` when thresholds are met. Track distinct context IDs (e.g., plan/task combos) on each entry to gate Working->Consolidated. Add a `deprecated: bool` field to `KnowledgeEntry` (or check for one) and enforce that `demote_tier()` returns `Err` for Persistent entries without `deprecated = true`.

**Reference files**:
- `crates/roko-neuro/src/lib.rs:128` -- KnowledgeTier enum with multiplier constants
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry struct, tier field at line 263
- `crates/roko-neuro/src/tier_progression.rs:176` -- TierProgression orchestrator
- `crates/roko-neuro/src/tier_progression.rs:864` -- promote_tier() / demote_tier() methods
- `crates/roko-neuro/src/knowledge_store.rs:207` -- ingest() method to modify
- `docs/06-neuro/02-four-validation-tiers.md` -- full promotion/demotion spec
**Depends on**: None
**Accept when**:
- [x] `ingest()` detects confirmations via HDC similarity and increments count
- [x] Promotion logic: Transient->Working (2+ confirmations), Working->Consolidated (3+ distinct contexts)
- [x] Persistent requires explicit deprecation for demotion; `demote_tier()` errors without it
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'promote_tier\|demote_tier\|confirmation_count\|deprecated' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P1

### NEURO-02: Cross-domain HDC resonance detection
- [x] Implement cross-domain similarity scanning on knowledge ingestion

**Spec** (doc 08, 09): Structural analogy detection via HDC similarity across knowledge domains. When a new entry is ingested, its HDC fingerprint is compared against entries from *different* domains. Abstract role vectors identify structural patterns (e.g., "retry logic" in a networking crate is structurally similar to "retry logic" in a database crate). The 0.526 threshold (doc 09) guarantees <1% false positive rate after Bonferroni correction across a 100K vocabulary (Z-score 5.26, per-comparison FP rate 7.3e-8). Multi-agent confirmation further reduces joint FP rate to ~5.3e-15. Matches become analogy hypotheses that must be confirmed by 2+ independent agents before promotion.

**Current code**: `KnowledgeHdcEncoder` at `crates/roko-neuro/src/hdc.rs:9` with `encode_entry()` at line 21 and `encode_query()` at line 29. `HdcVector::similarity()` at `crates/roko-primitives/src/hdc.rs:223` returns normalized Hamming similarity in [0,1]. `KnowledgeStore::ingest()` at `crates/roko-neuro/src/knowledge_store.rs:207` does not scan for cross-domain resonance. `ItemMemory::top_k()` at `crates/roko-primitives/src/hdc.rs:431` provides nearest-neighbor scanning over an in-memory item memory. The resonance scan should be fast (~13ns per comparison, ~1.3ms for 100K entries per doc 09).

**What to change**: In `KnowledgeStore::ingest()`, after adding new entries, scan each against existing entries from different domains using `HdcVector::similarity()`. For entries where similarity > 0.526, create a `KnowledgeKind::Insight` entry with `source: "cross_domain_resonance"` and initial confidence 0.3. Track confirmation count on resonance-generated entries; require 2+ confirmations before promoting above Transient. Add a `domain` field or use existing category/tag to partition entries by domain for the cross-domain check.

**Reference files**:
- `crates/roko-neuro/src/hdc.rs:9` -- KnowledgeHdcEncoder with encode_entry/encode_query
- `crates/roko-neuro/src/knowledge_store.rs:207` -- ingest() method to modify
- `crates/roko-primitives/src/hdc.rs:223` -- similarity() for threshold comparison
- `crates/roko-primitives/src/hdc.rs:431` -- ItemMemory::top_k() for nearest-neighbor scanning
- `docs/06-neuro/08-cross-domain-hdc-transfer.md` -- cross-domain resonance spec (abstract role vectors, analogy detection)
- `docs/06-neuro/09-false-positive-math.md` -- 0.526 threshold derivation, Bonferroni correction, JL validation
**Depends on**: NEURO-03 (structured role-filler encoding for better domain extraction)
**Accept when**:
- [x] On ingest, new entries scanned against entries from other domains
- [x] Similarity > 0.526 triggers analogy hypothesis with confidence 0.3
- [ ] Hypothesis requires 2+ agent confirmation before promotion above Transient
- [ ] Scan completes in <5ms for 100K entries
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'cross_domain\|resonance\|analogy\|0.526' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P1

### NEURO-03: Structured role-filler HDC encoding
- [x] Implement KnowledgeHdcEncoder with role-filler bindings

**Spec** (doc 06): The HDC knowledge encoding spec defines role-filler bindings: each entry is encoded as a superposition of `bind(role_vector, filler_vector)` pairs. Standard roles include: `role:kind` (the KnowledgeKind), `role:domain` (coding/chain/research/etc.), `role:content` (text fingerprint of the content body), `role:source` (agent or episode that produced it), and `role:tier` (current validation tier). This structured encoding enables *analogy by query*: `unbind(composite, role:kind)` retrieves the kind-independent content fingerprint, enabling cross-kind similarity (e.g., a Heuristic and an Insight that share the same conceptual content). The `query_by_role(role, filler)` method finds entries where a specific role is bound to a specific filler, e.g., "find all entries where role:domain = 'roko-gate'."

**Current code**: `KnowledgeHdcEncoder` at `crates/roko-neuro/src/hdc.rs:9` currently uses `text_fingerprint` from `crates/roko-primitives/src/hdc.rs:521` and `HdcVector::from_seed()` at line 205 for symbol fingerprinting. `encode_generic_entry()` at line 36 bundles kind + content fingerprints as an unstructured superposition. `encode_causal_link()` at line 61 uses `CausalLinkParts` (line 12) with `permute()` for directional encoding (CAUSE_SHIFT=1, EFFECT_SHIFT=2) -- this part is done correctly. `ItemMemory` at `crates/roko-primitives/src/hdc.rs:402` provides a named-vector store that can hold role vectors. Missing: the role vector dictionary (an `ItemMemory` of role vectors like "kind", "domain", "content") and role-filler bind at encode time.

**What to change**: (1) Add a `RoleVectors` struct to `KnowledgeHdcEncoder` containing an `ItemMemory` with deterministically-seeded role vectors for each standard role (kind, domain, content, source, tier). (2) In `encode_generic_entry()`, replace the current flat `bundle([kind_fp, content_fp])` with `bundle([bind(role_kind, kind_fp), bind(role_domain, domain_fp), bind(role_content, content_fp)])`. (3) Add `query_by_role(role: &str, filler: &HdcVector) -> HdcVector` that returns `bind(role_vector, filler)` for use as a similarity probe. (4) Add `unbind_role(composite: &HdcVector, role: &str) -> HdcVector` that returns `bind(composite, role_vector)` (XOR is self-inverse). (5) Wire these into `KnowledgeStore` as query methods.

**Reference files**:
- `crates/roko-neuro/src/hdc.rs:9` -- KnowledgeHdcEncoder, encode_generic_entry at line 36
- `crates/roko-neuro/src/hdc.rs:61` -- encode_causal_link with permute (pattern to follow)
- `crates/roko-primitives/src/hdc.rs:113` -- bind() (XOR operation)
- `crates/roko-primitives/src/hdc.rs:402` -- ItemMemory for storing named role vectors
- `crates/roko-primitives/src/hdc.rs:521` -- text_fingerprint for content encoding
- `crates/roko-neuro/src/knowledge_store.rs:207` -- ingest() to enhance with role-filler encoding
- `docs/06-neuro/06-hdc-knowledge-encoding.md` -- full role-filler binding spec, query API, analogy by unbinding
**Depends on**: None
**Accept when**:
- [x] `KnowledgeHdcEncoder` has `RoleVectors` with deterministic seeds for kind, domain, content, source, tier
- [x] `encode_generic_entry()` produces role-filler bound composite vectors
- [x] `query_by_role(role, filler) -> HdcVector` method exists on `KnowledgeStore`
- [x] `unbind_role(composite, role) -> HdcVector` method exists for analogy extraction
- [x] CausalLink uses permute for directional encoding (already done)
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'query_by_role\|unbind_role\|RoleVectors\|role_vectors' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P1

### NEURO-04: Reactive AntiKnowledge checking
- [x] Check new candidates against existing AntiKnowledge on ingest

**Spec** (doc 11): When new non-AntiKnowledge entries are ingested, they must be scanned against existing AntiKnowledge entries for HDC similarity conflicts. This is the reverse of the existing forward path (where new AntiKnowledge caps the target's confidence). Without this check, an agent can re-learn knowledge that has already been explicitly refuted, defeating the purpose of AntiKnowledge. The doc describes AntiKnowledge as an "epistemic parasite" that survives by attaching to its refuted target -- the reverse check ensures the parasite also blocks new entries that match the refuted pattern. Price equation (1970) analogy: AntiKnowledge provides selective pressure against recurring bad patterns.

**Current code**: `KnowledgeKind::AntiKnowledge` handled in `crates/roko-neuro/src/knowledge_store.rs:225` -- when AntiKnowledge is ingested, it looks up the `refuted_insight_id` and caps the original's confidence at 0.3 (line 253). However, the reverse check is missing: when a new non-AntiKnowledge entry is ingested, existing AntiKnowledge entries are not scanned for HDC similarity conflicts. Confidence floor 0.3 enforced at line 497. GC exemption for AntiKnowledge at line 521. `KnowledgeHdcEncoder::encode_entry()` at `crates/roko-neuro/src/hdc.rs:21` provides HDC vectors for comparison.

**What to change**: In `KnowledgeStore::ingest()` at line 207, after adding new entries:
1. For each new entry where `kind != AntiKnowledge`, collect all existing AntiKnowledge entries
2. Encode the new entry via `KnowledgeHdcEncoder::encode_entry()`
3. Compare against each AntiKnowledge entry's HDC vector using `HdcVector::similarity()`
4. If similarity > 0.5: log a warning with both entry IDs and the similarity score
5. If similarity > 0.7: discount the new entry's confidence by 0.5x (strong conflict)
6. If similarity > 0.9: reject the entry entirely (near-duplicate of refuted knowledge)
7. Add a `conflicts: Vec<(String, f64)>` field to the ingest result to surface detected conflicts

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs:207` -- ingest() method (add reverse check here)
- `crates/roko-neuro/src/knowledge_store.rs:225` -- existing forward AntiKnowledge handling (pattern to follow)
- `crates/roko-neuro/src/knowledge_store.rs:497` -- confidence floor 0.3 enforcement
- `crates/roko-neuro/src/knowledge_store.rs:521` -- GC exemption for AntiKnowledge
- `crates/roko-neuro/src/hdc.rs:9` -- KnowledgeHdcEncoder with encode_entry() at line 21
- `crates/roko-primitives/src/hdc.rs:223` -- similarity() for threshold comparison
- `docs/06-neuro/11-antiknowledge-challenge.md` -- AntiKnowledge spec, epistemic parasite detection, Price equation, confidence floor mechanics
**Depends on**: None
**Accept when**:
- [x] Ingest path checks each new non-AntiKnowledge entry against all existing AntiKnowledge entries
- [x] Similarity > 0.5 logs warning with entry IDs and similarity score
- [x] Similarity > 0.7 discounts new entry confidence by 0.5x
- [x] Similarity > 0.9 rejects entry entirely
- [x] Conflicts surfaced in ingest result
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'AntiKnowledge.*similarity\|check.*anti\|conflicts' crates/roko-neuro/src/knowledge_store.rs
cargo test -p roko-neuro
```
**Priority**: P1

### NEURO-05: Backup/restore CLI commands
- [x] Implement `roko neuro backup` and `roko neuro restore`

**Spec** (doc 15): Knowledge backup/restore follows a 4-step protocol: BACKUP (export all entries with full metadata), DELETE (optional -- clear local store), CREATE (provision fresh store), RESTORE (import with confidence discounting and tier reset). The doc specifies that restored entries always start at Transient tier regardless of their original tier, and confidence is multiplied by 0.85x per generation (so knowledge inherited twice has 0.85^2 = 0.7225x confidence). Export format is JSONL of `KnowledgeEntry` with all fields including HDC vectors, provenance, and lineage. Filter options allow selective export/import by knowledge type, minimum confidence, tags, and date range. This replaces the legacy "succession" concept with user-controlled backup/restore.

**Current code**: `KnowledgeStore` at `crates/roko-neuro/src/knowledge_store.rs:74` has `ingest()` at line 207 but no `export()`/`import()` methods. `KnowledgeEntry` at `crates/roko-neuro/src/lib.rs:216` derives `Serialize`/`Deserialize` so it can already be written to JSONL. No `neuro` subcommand exists in `crates/roko-cli/src/main.rs`. The `source: Option<String>` field on `KnowledgeEntry` at line 224 can be used to record the restore origin.

**What to change**: (1) Add `export(&self, path: &Path, filter: ExportFilter) -> Result<usize>` and `import(&mut self, path: &Path, options: ImportOptions) -> Result<usize>` methods to `KnowledgeStore`. `ExportFilter` has fields: `types: Option<Vec<KnowledgeKind>>`, `min_confidence: Option<f64>`, `tags: Option<Vec<String>>`, `since: Option<DateTime<Utc>>`. `ImportOptions` has: `confidence_discount: f64` (default 0.85), `reset_tier: bool` (default true), `source_label: String`. (2) Add `neuro backup <path>` and `neuro restore <path>` subcommands to CLI with corresponding clap args for `--types`, `--min-confidence`, `--tags`, `--since`. (3) On import, set `entry.tier = KnowledgeTier::Transient`, multiply `entry.confidence *= options.confidence_discount`, set `entry.source = Some(source_label)`.

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs:74` -- KnowledgeStore struct
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry with Serialize/Deserialize
- `crates/roko-neuro/src/lib.rs:128` -- KnowledgeTier (reset to Transient on import)
- `crates/roko-cli/src/main.rs` -- CLI entry point, add `neuro` subcommand group
- `docs/06-neuro/15-knowledge-backup-restore.md` -- 4-step backup/restore spec, confidence discount, filter options
- `docs/06-neuro/14-library-of-babel.md` -- 5 inflow channels, confidence discounts per channel (Self=1.0, Mesh=0.80, Korai=0.60, Restore=0.85, Lethe=0.50)
**Depends on**: None
**Accept when**:
- [x] `roko neuro backup <path>` exports knowledge store as JSONL -- `KnowledgeStore::export()` at knowledge_store.rs:810 + `NeuroCmd::Backup` in CLI main.rs:776
- [x] `roko neuro restore <path>` imports with filter options (--types, --min-confidence, --tags, --since) -- `NeuroCmd::Restore` in CLI main.rs:795 with --types, --min-confidence, --generation flags
- [x] Restored entries start at Transient tier -- `import()` at knowledge_store.rs:889 sets `entry.tier = KnowledgeTier::Transient`
- [x] 0.85x confidence discount applied per generation -- `ImportOptions::confidence_discount` default 0.85, applied at knowledge_store.rs:892
- [x] Source recorded on imported entries -- `entry.source = Some(options.source_label.clone())` at knowledge_store.rs:893
- [ ] `cargo test --workspace` passes
**Verify**:
```bash
grep -rn 'neuro.*backup\|neuro.*restore\|fn export\|fn import\|ExportFilter\|ImportOptions' crates/roko-neuro/src/ crates/roko-cli/src/ --include='*.rs'
cargo test --workspace
```
**Priority**: P1

### NEURO-06: BundleAccumulator for incremental bundling
- [x] Already implemented -- stateful incremental bundling with decay exists

**Spec** (doc 05): BundleAccumulator for adding vectors one at a time with optional decay.
**Current code**: `BundleAccumulator` at `crates/roko-primitives/src/hdc.rs:260` with `new()` at line 275, `add()` at line 283, `add_weighted()` at line 291, `decay()` at line 304, `finish()` at line 313. `DecayingBundleAccumulator` at line 335 with built-in decay on every `add()` and `half_life()` at line 378. This item is fully compliant.
**Reference files**:
- `crates/roko-primitives/src/hdc.rs:260` -- BundleAccumulator
- `crates/roko-primitives/src/hdc.rs:335` -- DecayingBundleAccumulator
**Accept when**: Already done.
**Verify**:
```bash
grep -rn 'BundleAccumulator\|DecayingBundle' crates/roko-primitives/src/hdc.rs
cargo test -p roko-primitives
```
**Priority**: Done

### NEURO-07: Confidence discounting per source channel
- [x] Apply source-specific confidence discounts on ingest

**Spec** (doc 14): The Library of Babel design defines 5 inflow channels for knowledge, each with a different trust discount reflecting the reliability of the source:
- `Self` (1.0x) -- locally generated knowledge (own episodes, own distillation)
- `Mesh` (0.80x) -- knowledge received from another agent via Agent Mesh
- `Korai` (0.60x) -- knowledge read from the Korai chain (less trusted due to potential poisoning)
- `Restore` (0.85x) -- knowledge imported via backup/restore (slight decay per generation, compounding: 0.85^N for N generations)
- `Lethe` (0.50x) -- knowledge inherited from dissolved predecessor agents (highest discount due to context loss)

The discount prevents knowledge from accumulating confidence through circular propagation: Agent A shares with Agent B, B shares back with A, and confidence inflates. Each channel hop applies its discount, ensuring confidence monotonically decreases with distance from the original source.

**Current code**: `KnowledgeStore::ingest()` at `crates/roko-neuro/src/knowledge_store.rs:207` accepts `Vec<KnowledgeEntry>` with no source channel parameter. `KnowledgeEntry` has `source: Option<String>` field at `crates/roko-neuro/src/lib.rs:224` but no discount logic exists. All entries ingested at their declared confidence regardless of origin. The NEURO-05 backup/restore item handles the Restore discount specifically, but the general mechanism is missing.

**What to change**:
1. Add to `crates/roko-neuro/src/lib.rs`:
```rust
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceChannel {
    Local,    // 1.0x -- own episodes
    Mesh,     // 0.80x -- agent mesh peer
    Korai,    // 0.60x -- on-chain knowledge
    Restore,  // 0.85x -- backup/restore
    Lethe,    // 0.50x -- predecessor dissolution
}
impl SourceChannel {
    pub fn discount(&self) -> f64 {
        match self { Local => 1.0, Mesh => 0.80, Korai => 0.60, Restore => 0.85, Lethe => 0.50 }
    }
}
```
2. Add `source_channel: Option<SourceChannel>` parameter to `KnowledgeStore::ingest()` (default `None` = `Local`)
3. Before storage, multiply `entry.confidence *= channel.discount()`
4. Record the channel in `entry.source` for provenance tracking
5. Update all existing callers of `ingest()` to pass appropriate channel

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs:207` -- ingest() method to modify
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry struct, source field at line 224
- `crates/roko-neuro/src/lib.rs:128` -- KnowledgeTier (discount interacts with tier multiplier)
- `crates/roko-cli/src/orchestrate.rs` -- primary ingest() caller (should pass Local)
- `docs/06-neuro/14-library-of-babel.md` -- 5 inflow channels, discount factors, circular propagation prevention
**Depends on**: None
**Accept when**:
- [x] `SourceChannel` enum with 5 variants and `discount()` method exists
- [x] `ingest()` accepts optional `SourceChannel` parameter
- [x] Confidence multiplied by channel-specific discount before storage
- [x] Source channel recorded in entry provenance
- [x] All existing callers updated to pass correct channel
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'SourceChannel\|discount\|source_channel' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2

### NEURO-08: Worldview clustering and cold-tier preservation
- [x] Implement co-citation clustering for knowledge worldviews

**Spec** (doc 12): Heuristics that consistently co-fire (activate in the same episodes) form implicit "worldviews" -- coherent sets of beliefs about how a domain works. Co-citation clustering groups these heuristics so the agent can: (a) recognize which worldview is active, (b) switch worldviews when the current one fails, and (c) preserve inactive worldviews in cold storage rather than losing them to GC. This is analogous to Kuhn's paradigm shifts -- an agent might have a "retry-heavy" worldview and a "fail-fast" worldview, each internally consistent but incompatible.

**Current code**: `HeuristicRule` at `crates/roko-neuro/src/tier_progression.rs:80` with `rule`, `support`, `confidence`, `source_insights` fields. `source_insights: Vec<String>` tracks which insights contributed but not which episodes they co-occurred in. `hdc_clustering.rs` at `crates/roko-learn/src/hdc_clustering.rs` provides k-medoids clustering over HDC vectors. `KnowledgeStore` GC at `crates/roko-neuro/src/knowledge_store.rs:521` deletes entries below confidence threshold with no worldview awareness. No co-citation matrix, no worldview grouping, no cold-tier flag.

**What to change**:
1. Add `co_citation_count: HashMap<String, usize>` to `HeuristicRule` tracking how many times each other heuristic co-fired in the same episode
2. Create `crates/roko-neuro/src/worldview.rs` with:
   ```rust
   pub struct Worldview {
       pub id: String,
       pub heuristic_ids: Vec<String>,
       pub centroid: HdcVector,       // average HDC of member heuristics
       pub last_active: DateTime<Utc>,
       pub cold: bool,                // true = preserved from GC
   }
   pub struct WorldviewClusterer {
       pub worldviews: Vec<Worldview>,
       pub min_co_citation: usize,    // default 3
       pub cold_threshold_days: u64,  // default 30
   }
   ```
3. In `WorldviewClusterer::cluster()`, build co-citation adjacency matrix from heuristic pairs, then run k-medoids from `hdc_clustering.rs` on HDC vectors of co-citing heuristics
4. Mark worldviews inactive for > `cold_threshold_days` as `cold = true`
5. In `KnowledgeStore` GC, exempt entries whose heuristic belongs to a cold worldview
6. Add `WorldviewClusterer::activate(worldview_id)` that sets `cold = false` and resets `last_active`

**Reference files**:
- `crates/roko-neuro/src/tier_progression.rs:80` -- HeuristicRule (add co_citation_count field)
- `crates/roko-neuro/src/tier_progression.rs:41` -- InsightRecord with source_episodes
- `crates/roko-learn/src/hdc_clustering.rs` -- k-medoids clustering to use for grouping
- `crates/roko-neuro/src/knowledge_store.rs:521` -- GC logic to add cold-tier exemption
- `crates/roko-neuro/src/hdc.rs:21` -- encode_entry() for centroid computation
- `docs/06-neuro/12-4-tier-distillation-pipeline.md` -- worldview clustering spec, co-citation semantics, cold-tier preservation, paradigm shift detection
**Depends on**: LEARN-01 (demurrage model), LEARN-10 (full Heuristic type)
**Accept when**:
- [ ] Co-citation counts tracked on HeuristicRule pairs -- not found; HeuristicRule has no co_citation field
- [ ] K-medoids clustering groups co-citing heuristics into worldviews -- tag-based union-find exists but not k-medoids on heuristic co-citation
- [ ] Inactive worldviews (>30 days) marked cold -- no time-based cold marking; WorldviewCluster has no last_active/cold fields
- [x] Cold worldview members exempt from GC -- `gc_with_worldview_preservation()` preserves last representative of each cluster from GC
- [ ] `activate()` re-enables a cold worldview -- no activate() method exists
- [ ] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'Worldview\|co_citation\|cold_tier\|WorldviewClusterer' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2 (Phase 2+)

### NEURO-09: Distillation D2 HDC clustering
- [x] Use HDC similarity-based clustering for D2 promotion

**Spec** (doc 12): D2 (insights -> heuristics) should use HDC similarity clustering, not just pattern matching.
**Current code**: `Distiller::distill()` at `crates/roko-neuro/src/distiller.rs:85` extracts knowledge from episodes. `TierProgression::promote_heuristics()` at `crates/roko-neuro/src/tier_progression.rs:400` promotes `InsightRecord` (line 41) to `HeuristicRule` using pattern matching. HDC clustering at `crates/roko-learn/src/hdc_clustering.rs` provides k-medoids but is not wired to D2 promotion. `KnowledgeHdcEncoder::encode_entry()` at `crates/roko-neuro/src/hdc.rs:21` can produce vectors for similarity comparison.
**What to change**: In `TierProgression::promote_heuristics()`, use HDC vectors to cluster related insights before promotion. Replace or augment the pattern-matching approach with `hdc_clustering` similarity grouping. Cluster representatives become the promoted heuristics.
**Reference files**:
- `crates/roko-neuro/src/tier_progression.rs:400` -- promote_heuristics()
- `crates/roko-neuro/src/tier_progression.rs:41` -- InsightRecord
- `crates/roko-neuro/src/distiller.rs:85` -- distill() entry point
- `crates/roko-learn/src/hdc_clustering.rs` -- k-medoids clustering
- `crates/roko-neuro/src/hdc.rs:21` -- encode_entry()
- `docs/06-neuro/12-*` -- D2 distillation spec
**Depends on**: NEURO-03 (structured HDC encoding for richer vectors)
**Accept when**:
- [x] D2 promotion uses HDC similarity to cluster related insights
- [x] Cluster representatives promoted to heuristics
- [x] `cargo test -p roko-neuro`
**Verify**:
```bash
grep -rn 'hdc_cluster\|similarity.*insight\|cluster.*heuristic' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2

### NEURO-10: Demurrage balance model and reinforcement signals
- [x] Implement balance-based freshness with reinforcement earning

**Spec** (doc 07): Knowledge entries carry a `balance` field that represents their freshness reserve. Balance decreases via demurrage tax over time and increases via 5 reinforcement signals: Retrieved (entry was selected for use), Cited (another entry references it), Gated (survived a verification gate), Surprised (explained a novel outcome), AgentQuoted (agent explicitly reused it). The freshness formula is `freshness(t) = balance(t) * ebbinghaus_weight(age, type_half_life, tier_multiplier)`. Novelty-weighted reinforcement means common entries get small bumps while rare-but-useful entries get larger bumps, scored against top-K HDC neighbors. Spacing effect: reinforcement from distinct episodes matters more than repeated reinforcement within one episode.

**Current code**: `KnowledgeEntry` at `crates/roko-neuro/src/lib.rs:216` has `confidence: f64` and `half_life_days: f64` but no `balance` field. Decay in `KnowledgeStore` at `crates/roko-neuro/src/knowledge_store.rs` uses time-based exponential decay only. No reinforcement signals are tracked. The `hdc_vector` field exists but novelty-weighted reinforcement is not computed. GC at line 521 uses confidence threshold only, not balance.

**What to change**: (1) Add `balance: f64` field to `KnowledgeEntry` (initial value 1.0). (2) Add `ReinforcementSignal` enum with 5 variants (Retrieved, Cited, Gated, Surprised, AgentQuoted). (3) Add `reinforce(&mut self, signal: ReinforcementSignal, novelty: f64)` method that bumps balance by `signal.base_value() * (1.0 + novelty)`. (4) Add `apply_demurrage(&mut self, elapsed_hours: f64)` method that deducts `demurrage_rate * elapsed_hours`. (5) Update GC to use `balance < threshold` instead of just confidence. (6) In `ingest()`, emit reinforcement for confirmed entries.

**Reference files**:
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry, add balance field
- `crates/roko-neuro/src/knowledge_store.rs` -- decay/GC logic to update
- `crates/roko-neuro/src/hdc.rs` -- HDC similarity for novelty scoring
- `docs/06-neuro/07-ebbinghaus-decay-with-tier.md` -- full demurrage spec, balance formula, reinforcement signals, worked examples
**Depends on**: None
**Accept when**:
- [x] `KnowledgeEntry` has `balance: f64` field
- [x] 5 reinforcement signal types implemented
- [x] Balance updated on retrieval, citation, gate survival, surprise, and agent quoting
- [x] Demurrage tax applied periodically
- [x] GC considers balance, not just confidence
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'balance\|demurrage\|ReinforcementSignal\|reinforce' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2 (deferred per INDEX.md)

### NEURO-11: Cold-tier freeze/thaw for knowledge entries
- [x] Implement freeze/thaw mechanics for low-balance entries

**Spec** (doc 07): When an entry's balance falls below a GC floor, it is frozen into cold storage rather than deleted. Frozen entries retain their content address, lineage, and provenance but are removed from the hot retrieval path. Thawing restores a starter balance so the entry can compete again but not indefinitely -- if it fails after thaw, it cools back down. This prevents good-but-quiet knowledge from being permanently lost.

**Current code**: `KnowledgeStore` at `crates/roko-neuro/src/knowledge_store.rs` stores all entries in JSONL and runs GC that deletes entries below a confidence threshold (line 521). No cold/hot distinction. No freeze/thaw methods. `KnowledgeEntry` has no `frozen: bool` or `cold_tier: bool` field. AntiKnowledge is exempt from GC at line 521 but there is no general exemption mechanism.

**What to change**: (1) Add `frozen: bool` field to `KnowledgeEntry` (default false). (2) In GC, instead of deleting entries below threshold, set `frozen = true` and exclude them from query results. (3) Add `thaw(entry_id: &str, starter_balance: f64)` method that resets `frozen = false` and sets `balance = starter_balance`. (4) Add `query_cold(filter)` method to retrieve frozen entries for inspection. (5) Frozen entries should be stored in a separate JSONL file or section for performance.

**Reference files**:
- `crates/roko-neuro/src/knowledge_store.rs` -- GC logic, query methods
- `crates/roko-neuro/src/lib.rs:216` -- KnowledgeEntry, add frozen field
- `docs/06-neuro/07-ebbinghaus-decay-with-tier.md` -- freeze/thaw semantics, cold-tier graduation
**Depends on**: NEURO-10 (balance field needed for freeze threshold)
**Accept when**:
- [x] `KnowledgeEntry` has `frozen: bool` field
- [x] GC freezes instead of deleting entries below balance threshold
- [x] `thaw()` restores starter balance
- [x] Frozen entries excluded from hot queries but accessible via `query_cold()`
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'frozen\|thaw\|cold_tier\|query_cold' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2 (deferred per INDEX.md)

### NEURO-12: Falsifier records for heuristic calibration
- [x] Implement explicit falsifier/contradiction records on heuristics

**Spec** (doc 12): When a heuristic is contradicted by evidence, an explicit falsifier record is created rather than just lowering confidence. The falsifier records: what heuristic was contradicted, which episode(s) provided the contradiction, the nature of the contradiction (full refutation vs. refinement), and whether the heuristic should be narrowed (child heuristic spawned) or cold-tiered. Calibration fields on `HeuristicRule`: trials, confirmations, violations, confidence_interval, receipts. Five calibration actions: Confirm, Violate, Refine, Generalize, Refute.

**Current code**: `HeuristicRule` at `crates/roko-neuro/src/tier_progression.rs:80` has `rule`, `support`, `confidence`, `source_insights` fields. No `trials`, `violations`, `confidence_interval`, or `receipts` fields. `replay_heuristics()` at line signature adjusts confidence up/down but does not create falsifier records. The `Distiller` at `crates/roko-neuro/src/distiller.rs:85` extracts knowledge including contradictions but they are not tracked as falsifiers.

**What to change**: (1) Expand `HeuristicRule` with `trials: usize`, `confirmations: usize`, `violations: usize`, `receipts: Vec<(String, CalibrationAction)>` where `CalibrationAction` is `{Confirm, Violate, Refine, Generalize, Refute}`. (2) Add `FalsifierRecord` struct with `heuristic_id`, `contradicting_episodes`, `contradiction_type`, `action_taken`. (3) In `replay_heuristics()`, create `FalsifierRecord` entries when violations are detected. (4) Store falsifiers alongside heuristics in the knowledge store.

**Reference files**:
- `crates/roko-neuro/src/tier_progression.rs:80` -- HeuristicRule to expand
- `crates/roko-neuro/src/tier_progression.rs` -- replay_heuristics() to enhance
- `crates/roko-neuro/src/distiller.rs:85` -- distill() contradiction extraction
- `docs/06-neuro/12-4-tier-distillation-pipeline.md` -- heuristic calibration spec, 5 calibration actions, falsifier records
**Depends on**: None
**Accept when**:
- [x] `HeuristicRule` has trials, confirmations, violations, receipts fields
- [x] `FalsifierRecord` struct exists with contradiction metadata
- [x] `replay_heuristics()` creates falsifier records on contradiction
- [x] Falsifiers stored in knowledge store
- [x] `cargo test -p roko-neuro` passes
**Verify**:
```bash
grep -rn 'FalsifierRecord\|CalibrationAction\|violations\|receipts' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-neuro
```
**Priority**: P2

## Verify
```bash
cargo test -p roko-neuro
cargo test -p roko-primitives
cargo test --workspace
```
