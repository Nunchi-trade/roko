# 03-composition -- Gap Checklist

Spec: `docs/03-composition/` (15 files, docs 00-13 + INDEX). Code: `crates/roko-compose/`.

Overall: ~75% compliant. Core prompt composition works. Gaps in advanced features (VCG auction, MVT foraging, HDC dedup, distributed context).

## Compliant (no action needed)

- Composer trait (doc 00)
- PromptComposer with greedy knapsack (doc 01)
- 12 role templates + PromptBudget (doc 03)
- 13-step enrichment pipeline (doc 04) -- core steps all working
- Token budget management -- static + adaptive + context-tier (doc 05)
- U-shape placement (doc 06)
- Cache alignment markers (doc 02, core feature; layer count discrepancy handled in COMP-01)
- Status and gaps self-assessment (doc 13 -- report doc, no new code needed)

## Checklist

### COMP-01: Doc says 7 layers, code has 9

- [x] Update docs OR align code layer count

**Spec** (doc 02 title): "7-Layer SystemPromptBuilder". The doc's table lists 7 layers:
Role identity, Project conventions, Domain context, Task context, Tool instructions, Skills,
Anti-patterns.

**Current code** (`crates/roko-compose/src/system_prompt_builder.rs:1`): Module docstring says
"Composable system prompt builder with 9 layers." The 9 layers are: (1) Role identity,
(2) Project conventions, (3) Domain context, (3b) Assembled context from ContextAssembler,
(3c) Pheromone signals from stigmergy, (4) Task context, (5) Tool instructions, (6) Skills,
(6b) Playbooks from PlaybookRules, (7) Anti-patterns, (8) Affect guidance from PadState.
Extra vs doc: layers 3b (assembled context), 3c (pheromone signals), 6b (playbooks), and
8 (affect guidance).

**Decision**: Code is ahead of doc. Update doc 02 to reflect 9 layers. No code changes.

**What to change**: Edit `docs/03-composition/02-system-prompt-builder-7-layer.md`:
- Change title to "9-Layer SystemPromptBuilder"
- Add rows for layers 3b, 3c, 6b, and 8 to the layer table
- Update layer count references throughout the file

**Reference files**:
- `crates/roko-compose/src/system_prompt_builder.rs:1` -- module docstring with 9 layers
- `crates/roko-compose/src/system_prompt_builder.rs:58-78` -- layer definitions
- `crates/roko-compose/src/system_prompt_builder.rs:543` -- affect guidance (layer 8) implementation
- `docs/03-composition/02-system-prompt-builder-7-layer.md` -- doc to update (rename file too)

**Accept when**:

- [x] Doc title updated to "9-Layer SystemPromptBuilder" — doc 02 title now reads "02 -- SystemPromptBuilder: 9-Layer Prompt Assembly"
- [x] All 9 layers described in doc — layers 1, 2, 3a, 3b, 3c, 4, 5, 6a, 6b, 8 all have dedicated sections with descriptions
- [x] Layer numbering consistent between doc and code — doc matches code's 9-layer scheme with cache tier assignments

**Verify**:
```bash
grep -n 'Layer\|layer' crates/roko-compose/src/system_prompt_builder.rs | head -30
head -5 docs/03-composition/02-system-prompt-builder-7-layer.md
```

**Priority**: P0 (doc update only)

---

### COMP-02: VCG auction not used in live composition

- [x] Wire VCG auction as budget allocation mechanism in PromptComposer

**Spec** (doc 10): VCG determines budget allocation across sections.

**Current code** (`crates/roko-compose/src/auction.rs`): `LearningBidder` at line 32,
`AuctionDiagnostics` at line 87. Full VCG infrastructure with payment rules and Pareto checks
exists but is standalone -- `PromptComposer` at `crates/roko-compose/src/prompt.rs:285` uses
greedy knapsack, not VCG.

**What to change**: Add a VCG allocation mode to `PromptComposer` that creates bidders for
each prompt section, runs the auction, and uses results as section token budgets. Keep greedy
knapsack as fallback.

**Reference files**:
- `crates/roko-compose/src/auction.rs:32` -- `LearningBidder` struct
- `crates/roko-compose/src/auction.rs:87` -- `AuctionDiagnostics` struct
- `crates/roko-compose/src/prompt.rs:285` -- `PromptComposer` struct (greedy knapsack)
- `docs/03-composition/10-vcg-auction.md` -- spec for VCG budget allocation

**Accept when**:

- [x] PromptComposer uses VCG auction for section budget allocation — prompt.rs:452-504: LearningBidder multipliers applied to bid density, `vcg_payment_summary()` computes VCG payments
- [x] Bidders represent each prompt section — `AttentionBidder` enum per section; `register_bidder()` at prompt.rs:344 maps bidders to `LearningBidder`s
- [x] Auction diagnostics available — output signal tagged with `auction_total_bid`, `auction_total_payments`, `auction_urgency`, `auction_affect_weight`
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'LearningBidder\|AuctionDiagnostics' crates/roko-compose/src/ --include='*.rs'
grep -rn 'PromptComposer' crates/roko-compose/src/prompt.rs
cargo test -p roko-compose
```

**Priority**: P1

---

### COMP-03: MVT foraging not integrated into Stage 1

- [x] Wire predictive foraging stopping rule into context retrieval

**Spec** (doc 09): MVT (Marginal Value Theorem) stopping rule controls when to stop searching.

**Current code** (`crates/roko-compose/src/foraging.rs`): `MultiPatchForager` at line 25,
`should_stop_searching()` at line 172, `social_foraging_boost()` at line 106. All fully
implemented but standalone -- not called during Stage 1 (Query) assembly in the composition
pipeline.

**What to change**: In the context retrieval path (Stage 1 of composition in
`PromptComposer`), integrate the foraging loop:
1. Initialize `MultiPatchForager` with `SourceForagingProfile` entries derived from prior
   episode statistics (source visit count, average relevance per source, travel cost estimated
   from token count of source).
2. Use `optimal_order()` to determine source visitation order.
3. For each source, call `should_visit()` before querying it.
4. After each batch of retrieved chunks, compute `mvt_ratio` (marginal gain of last batch /
   environment average gain) and `sufficiency` via `coverage_sufficiency()`.
5. Call `should_stop_searching(mvt_ratio, sufficiency, 0.8)` to decide whether to continue.
6. The integration point is `PromptComposer::compose()` at
   `crates/roko-compose/src/prompt.rs:285` -- add a pre-pass that computes the retrieval
   budget before the greedy knapsack stage.

**Reference files**:
- `crates/roko-compose/src/foraging.rs:25` -- `MultiPatchForager` struct
- `crates/roko-compose/src/foraging.rs:172` -- `should_stop_searching()` function
- `crates/roko-compose/src/prompt.rs:285` -- `PromptComposer` (Stage 1 integration point)
- `docs/03-composition/09-foraging.md` -- spec for MVT stopping rule

**Accept when**:

- [x] Context retrieval calls `should_stop_searching()` during source iteration — prompt.rs:13 imports `should_stop_searching`, `foraging_prepass()` at line 477-481 uses MVT to limit candidates; `should_stop_searching(mvt_ratio, sufficiency, 0.8)` called at line 998
- [x] Foraging parameters calibrated from prior episodes — `MultiPatchForager` with `SourceForagingProfile` entries, `with_foraging()` builder at prompt.rs:384
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'should_stop_searching\|MultiPatchForager' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-compose
```

**Priority**: P1

---

### COMP-04: Stage 3 HDC deduplication not integrated

- [x] Wire HDC similarity dedup into assembly pipeline

**Spec** (doc 08 §Stage 3 Deduplicate): After Stage 2 scoring ranks candidates by relevance,
Stage 3 removes near-duplicate candidates using HDC fingerprint similarity. Two candidates are
duplicates if their Hamming distance < 0.15 (equivalently, similarity > 0.85). Greedy scan:
for each candidate in rank order, compare against accepted candidates and reject on threshold.
This prevents the prompt from wasting token budget on redundant information.

**Current code**: `HdcVector` at `crates/roko-primitives/src/hdc.rs:30` provides 1024-bit
hyperdimensional vectors with `similarity()` at line 223 (cosine via Hamming distance).
`ContextAssembler::compress()` at `crates/roko-neuro/src/context.rs:366` does basic
compression but not HDC dedup. `PromptComposer` at `crates/roko-compose/src/prompt.rs:285`
uses greedy knapsack but has no dedup stage between scoring and budgeting.

**What to change**: In `ContextAssembler::compress()` or as a new dedup pass in `PromptComposer`,
add HDC dedup between scoring and budget fitting. For each candidate in rank order: compute
HDC fingerprint, compare against accepted candidates, reject if similarity > 0.85.

**Reference files**:
- `crates/roko-primitives/src/hdc.rs:30` -- `HdcVector` struct, 1024-bit vectors
- `crates/roko-primitives/src/hdc.rs:223` -- `similarity()` cosine via Hamming
- `crates/roko-neuro/src/context.rs:366` -- `ContextAssembler::compress()` integration point
- `crates/roko-compose/src/prompt.rs:285` -- `PromptComposer` alternative integration point
- `docs/03-composition/08-5-stage-assembly-pipeline.md` -- §Stage 3, threshold = 0.15 Hamming

**Accept when**:

- [x] HDC fingerprints computed for each scored candidate — `hdc_dedup_candidates()` at prompt.rs:1016 creates `HdcVector::from_seed(content.as_bytes())` per candidate (behind `hdc` feature flag)
- [x] Greedy dedup removes candidates with similarity > threshold vs any accepted candidate — prompt.rs:1038: `fingerprint.similarity(accepted) > threshold`; configurable via `with_hdc_dedup(threshold)` at line 398
- [x] Dedup runs between scoring (Stage 2) and budget fitting (Stage 4) — prompt.rs:471-474: runs after bid density computation, before `select_optional_candidates()`
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'HdcVector\|similarity' crates/roko-primitives/src/hdc.rs | head -10
grep -rn 'dedup\|deduplicate\|hamming' crates/roko-compose/src/ --include='*.rs'
grep -rn 'dedup\|deduplicate' crates/roko-neuro/src/context.rs
cargo test -p roko-compose
```

**Priority**: P1

---

### COMP-05: Active inference epistemic value partial -- RESOLVED

- [x] Complete Bayesian belief change computation in EFE scoring

**Spec** (doc 07): Full EFE with epistemic value (belief divergence).

**Current code** (`crates/roko-compose/src/scorer.rs:232`): `ActiveInferenceScorer` is a
type alias for `GoalDirectedHeuristicScorer`. Uses HDC-approximate similarity, not proper
Bayesian divergence for epistemic value.

**What to change**: Two options:
(a) **Implement proper epistemic value**: Add a `compute_epistemic_value()` method that
computes KL divergence `D_KL(posterior || prior)` where prior is the agent's belief state
before seeing the section and posterior is the belief state after. In practice, approximate
this as the change in confidence on task-relevant predictions: `epistemic_value =
|confidence_after - confidence_before|`. This requires maintaining a belief state vector
(e.g., confidence on each subtask) and simulating the update from each candidate section.
(b) **Document HDC approximation**: Add a doc comment on the `ActiveInferenceScorer` type
alias at `crates/roko-compose/src/scorer.rs:232` explaining that the HDC-based similarity
score is an intentional approximation of EFE epistemic value. The justification: HDC cosine
similarity correlates with information gain for text sections (high-similarity sections are
redundant, low-similarity sections provide novel information), and the computational cost
of proper Bayesian belief update is prohibitive per-section during prompt assembly.
Option (b) is simpler and honest about the current architecture.

**Reference files**:
- `crates/roko-compose/src/scorer.rs:232` -- `ActiveInferenceScorer` type alias
- `crates/roko-compose/src/lib.rs:70` -- re-export of `ActiveInferenceScorer`
- `docs/03-composition/07-active-inference.md` -- spec for EFE scoring

**Accept when**:

- [ ] Epistemic value computed from actual belief change (KL divergence or similar) — not implemented; `epistemic_value()` at scorer.rs:215 uses HDC similarity, not proper KL divergence
- [x] OR: HDC approximation documented as intentional design choice — extensive doc comments at scorer.rs:102-123 and 266-293 explain the HDC approximation rationale
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'ActiveInferenceScorer\|GoalDirectedHeuristicScorer' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-compose
```

**Priority**: P2

---

### COMP-06: Dominance affect modulation not wired -- RESOLVED

- [x] Wire PAD dominance axis to affect guidance

**Spec** (doc 12): Dominance modulates retrieval and prompt tone.

**Current code** (`crates/roko-neuro/src/context.rs:148`): `PadState` struct with `pleasure`,
`arousal`, `dominance` fields. In `crates/roko-compose/src/system_prompt_builder.rs:543`:
dominance is partially read (`if affect.dominance <= -0.20` and `>= 0.30`) for affect
guidance text, but the retrieval bias modulation from the spec is absent.

**What to change**: Wire dominance to retrieval bias (e.g., high dominance = prefer authoritative
sources, low dominance = prefer diverse sources). Ensure the existing dominance thresholds in
the builder cover the full spec.

**Reference files**:
- `crates/roko-neuro/src/context.rs:148` -- `PadState` struct
- `crates/roko-compose/src/system_prompt_builder.rs:543` -- existing dominance checks
- `docs/03-composition/12-affect.md` -- spec for dominance modulation

**Accept when**:

- [x] Dominance axis influences retrieval bias or prompt tone
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'dominance' crates/roko-compose/src/ --include='*.rs'
grep -rn 'PadState' crates/roko-neuro/src/context.rs
cargo test -p roko-compose
```

**Priority**: P2

---

### COMP-07: PAD state decay and persistence

- [x] Implement PAD state time-based decay and disk persistence

**Spec** (doc 12 §4): PAD state should decay over time and persist to `.roko/daimon/`.

**Current code** (`crates/roko-neuro/src/context.rs:148`): `PadState` struct exists with
`pleasure`, `arousal`, `dominance`, `somatic_valence` fields and a `new()` constructor.
No decay logic. No disk persistence.

**What to change**: Add a `decay(&mut self, elapsed_ms: u64, half_life_ms: u64)` method to
`PadState` that exponentially decays each axis toward 0.0. Add serde serialization and a
persistence path in `.roko/daimon/pad-state.json`.

**Reference files**:
- `crates/roko-neuro/src/context.rs:148` -- `PadState` struct
- `crates/roko-daimon/src/lib.rs` -- daimon crate (where persistence could live)
- `docs/03-composition/12-affect.md` -- spec for PAD decay and persistence

**Accept when**:

- [x] PAD state decays toward neutral over configurable half-life — `PadState::decay()` at context.rs:212 with exponential decay formula `x * 2^(-elapsed/half_life)`; `DEFAULT_PAD_HALF_LIFE_MS = 30min`; snap-to-zero at `PAD_SNAP_TO_ZERO = 0.01`
- [x] PAD state persisted to disk between sessions — serde `Serialize`/`Deserialize` derived at context.rs:157; `PAD_STATE_FILENAME = "pad-state.json"` at line 178; `persist()` and `load()` methods tested in `pad_persist_and_load_round_trips` at line 2814
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'PadState' crates/roko-neuro/src/context.rs
grep -rn 'decay' crates/roko-neuro/src/ --include='*.rs'
cargo test -p roko-compose
```

**Priority**: P2

---

### COMP-08: Level 3 network context engineering

- [x] Implement agent mesh context sharing

**Spec** (doc 11 §Level 3): Network-level context engineering for agent mesh.

**Current code**: No agent mesh infrastructure. The closest existing concept is
`MultiAgentPool` at `crates/roko-agent/src/multi_pool.rs:48` but it manages lifecycle,
not context sharing.

**What to change**: Add a shared context bus or registry that agents in a mesh can publish
sections to and subscribe from. Implement cross-agent context deduplication.

**Reference files**:
- `crates/roko-agent/src/multi_pool.rs:48` -- `MultiAgentPool` (lifecycle, not context)
- `crates/roko-compose/src/prompt.rs:285` -- `PromptComposer` (where shared context would be consumed)
- `docs/03-composition/11-context-engineering.md` -- spec for Level 3 network context

**Depends on**: AGT-07 (MultiAgentPool wired to orchestrator)

**Accept when**:

- [x] Agents can share context sections across mesh — `ContextMesh` at context_mesh.rs:45 with `publish()`, `query()`, `query_all()`, `to_prompt_sections()` methods; thread-safe via `Arc<Mutex<_>>`
- [x] Context deduplication across agents — `ContextMesh::deduplicate()` at context_mesh.rs:203 removes similar entries; tested in `deduplicate_removes_similar_entries` and `deduplicate_keeps_different_topics`

**Verify**:
```bash
grep -rn 'mesh\|shared_context\|context_bus' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-compose
```

**Priority**: P2 (Phase 2+, depends on mesh)

---

### COMP-09: Enrichment pipeline learning paths -- RESOLVED

- [x] Add adaptive step selection, parallel execution, cost tracking

**Spec** (doc 04 §9, doc 13): Enrichment steps should be adaptively selected (learned from outcomes), run in parallel where possible, and track cost per step.

**Current code** (`crates/roko-compose/src/enrichment/`): `EnrichmentPipeline<C>` at
`pipeline.rs:30` runs 13 enrichment steps. `EnrichmentConfig` at `config.rs:18` controls
which steps are enabled. `EnrichmentEstimate` at `estimate.rs:118` estimates cost/tokens
before running. Steps are defined in `step.rs` and selected by complexity/role via
`estimate_enrichment()` in `estimate.rs`. Steps run sequentially via the pipeline's
`run()` method. No adaptive selection (learning from which steps actually helped), no
parallel execution (independent steps like "gather imports" and "gather tests" could run
concurrently), no per-step cost tracking (the estimate is pre-computed but actual cost
per step is not recorded).

**What to change**: Add a step selection mechanism that learns from prior episode outcomes
(via efficiency events). Identify independent steps and run them concurrently. Track
per-step token/time cost and emit efficiency events.

**Reference files**:
- `crates/roko-compose/src/` -- enrichment pipeline implementation
- `crates/roko-learn/src/efficiency.rs` -- efficiency event format for cost tracking
- `docs/03-composition/04-enrichment.md` -- spec for enrichment steps
- `docs/03-composition/13-learning.md` -- spec for adaptive learning

**Accept when**:

- [x] Step selection influenced by prior outcomes
- [x] Independent steps can run in parallel
- [x] Per-step cost tracked in efficiency events
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'enrichment\|enrich' crates/roko-compose/src/ --include='*.rs' | head -20
cargo test -p roko-compose
```

**Priority**: P2

---

### COMP-10: Budget prediction and section influence learning

- [x] Implement TALE budget prediction and leave-one-out influence

**Spec** (doc 05 §11-12): Budget prediction from task features. Leave-one-out section influence measurement.

**Current code** (`crates/roko-compose/src/`): Neither budget prediction from task features
nor leave-one-out section influence measurement is implemented. Token budgets are set
statically or via the budget policy.

**What to change**: Add a budget predictor that takes task features (complexity, role, domain)
and predicts optimal token budget from historical efficiency data. Add leave-one-out influence
scoring that measures each section's impact on task success.

**Reference files**:
- `crates/roko-compose/src/prompt.rs:285` -- `PromptComposer` (budget allocation)
- `crates/roko-compose/src/budget.rs` -- budget management
- `crates/roko-learn/src/efficiency.rs` -- historical efficiency data
- `docs/03-composition/05-token-budget.md` -- spec for TALE budget prediction

**Accept when**:

- [x] Budget predicted from task features before composition
- [x] Section influence measured via leave-one-out or approximation
- [x] Results fed back into section weights
- [x] `cargo test -p roko-compose` passes

**Verify**:
```bash
grep -rn 'budget_predict\|influence\|leave_one_out' crates/roko-compose/src/ --include='*.rs'
cargo test -p roko-compose
```

**Priority**: P2

---

## Verify

```bash
cargo test -p roko-compose
```
