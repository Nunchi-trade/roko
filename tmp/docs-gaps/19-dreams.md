# 10-dreams -- Gap Checklist

Spec: `docs/10-dreams/` (18 files). Code: `crates/roko-dreams/`.

Overall: ~60% implemented, ~35% scaffolded as phase2 types. Core 3-phase cycle works. Major gaps in staging buffer, Mattar-Daw scoring, Pearl SCM, rendering, and collective features.

## Compliant (no action needed)
- Death reframe -- no mortality concepts (doc 00)
- Three-phase cycle -- NREM -> REM -> Integration (doc 01 core)
- HDC counterfactual synthesis -- 10,240-bit BSC, k-medoids clustering (doc 06)
- Idle-time scheduling with configurable thresholds (doc 13 core)

## Checklist

### DREAM-01: SQLite staging buffer with confidence ladder
- [x] Implement staging buffer for dream outputs

**Spec** (doc 04): Dream-generated insights must pass through a staging buffer before entering the main KnowledgeStore. The staging buffer is SQLite-backed with a 5-stage confidence ladder:
- Stage 0: 0.20 (initial dream output -- raw, unvalidated)
- Stage 1: 0.30 (survives one dream cycle without contradiction)
- Stage 2: 0.50 (confirmed by a second dream cycle or waking evidence)
- Stage 3: 0.70 (promotion gate -- ready for KnowledgeStore)
- Stage 4: promoted to KnowledgeStore at Transient tier

Each promotion requires validation: the insight must not be contradicted by existing knowledge (check via AntiKnowledge scan) and must pass a novelty check (not redundant with existing entries, HDC similarity < 0.9 against existing store). Temporal decay applies in staging: entries that do not promote within 7 days are garbage collected. This prevents dream hallucinations from corrupting the main knowledge store.

**Current code** (`crates/roko-dreams/src/cycle.rs:401`): `DreamCycle::run()` writes insights directly to KnowledgeStore via the distiller -- no staging buffer, no confidence ladder, no validation gate. `DreamCycleReport` at line 67 records outputs but without confidence tracking. No SQLite dependency in roko-dreams Cargo.toml. No staging table schema.

**What to change**: (1) Add `rusqlite` dependency to roko-dreams. (2) Create `crates/roko-dreams/src/staging.rs` with `StagingBuffer` struct backed by SQLite table: `(id TEXT PRIMARY KEY, entry BLOB, confidence REAL, stage INTEGER, created_at TEXT, last_promoted TEXT)`. (3) Modify `DreamCycle::run()` to write insights to staging at confidence 0.20 instead of directly to KnowledgeStore. (4) Add `StagingBuffer::try_promote(&mut self, store: &KnowledgeStore) -> Vec<KnowledgeEntry>` that checks each entry against promotion criteria and advances the stage. (5) Add GC for entries older than 7 days that haven't promoted past stage 1.

**Reference files**:
- `crates/roko-dreams/src/cycle.rs:333` -- DreamCycle struct
- `crates/roko-dreams/src/cycle.rs:401` -- run() method to modify (write to staging instead of store)
- `crates/roko-dreams/src/runner.rs:545` -- replay_insights() produces entries
- `crates/roko-neuro/src/knowledge_store.rs` -- KnowledgeStore for final insertion after promotion
- `crates/roko-neuro/src/hdc.rs` -- HDC similarity for redundancy check
- `docs/10-dreams/04-consolidation-and-staging.md` -- full staging spec, confidence ladder, promotion criteria, temporal decay, safety constraints
**Depends on**: None
**Accept when**:
- [x] SQLite-backed staging buffer exists at `crates/roko-dreams/src/staging.rs`
- [x] Dream insights enter staging at 0.20 confidence, NOT directly into KnowledgeStore
- [x] Promotion at each threshold (0.20->0.30->0.50->0.70) requires validation
- [x] Validation checks: no contradiction (AntiKnowledge), not redundant (HDC < 0.9)
- [x] Gate at 0.70 before entering KnowledgeStore at Transient tier
- [x] Entries older than 7 days without promotion are GC'd
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'StagingBuffer\|staging\|confidence.*ladder\|try_promote' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P1

### DREAM-02: Mattar-Daw utility scoring for NREM replay
- [x] Implement replay prioritization formula

**Spec** (doc 02): The Mattar-Daw (2018, Nature Neuroscience) formula prioritizes which episodes to replay during NREM based on expected learning value:
```
utility(episode) = Gain(episode) * Need(episode) * (1 / spacing(episode))
```
Where:
- `Gain` = expected improvement from replaying (high for episodes with surprising outcomes or unresolved errors)
- `Need` = how much the current knowledge needs updating (high when related heuristics have low confidence)
- `1/spacing` = inverse of time since last replay (prioritize episodes not recently replayed -- spaced repetition)

Jensen et al. (2024) extended this with variable-length rollouts: replay length adapts to episode complexity (r=0.186 human correlation). Sagiv et al. (2025) added goal-uncertain replay with ensemble value functions. Four replay modes: Random (baseline), Consequence (high-outcome episodes), Causal (episodes with clear cause-effect), Hypothetical (episodes that could have gone differently). SM-2 scheduling (Pimsleur 1967) for spaced repetition of confirmed knowledge.

**Current code** (`crates/roko-dreams/src/replay.rs:17`): `DreamReplayMode` enum with 4 variants (Random, Consequence, Causal, Hypothetical). `DreamReplayPolicy` at line 36 has `max_episodes: usize` config. `select_replay_episodes()` at line 94 dispatches by mode but uses simple heuristics, not Mattar-Daw formula. `ReplayCandidate` at line 120 has a `score: f64` field that is set to a simple heuristic value, not computed from Gain*Need*(1/spacing). Phase2 `ReplayMode` at `crates/roko-dreams/src/phase2/replay.rs:13` and `ReplayFidelity` at line 48 are defined but not wired into the main replay path.

**What to change**: (1) Add `mattar_daw_score(episode: &Episode, store: &KnowledgeStore) -> f64` function computing `gain * need * (1.0 / spacing)`. (2) `gain`: compute from episode's prediction error (gate failures have high gain, clean passes have low gain). (3) `need`: compute from confidence of related heuristics in KnowledgeStore (low confidence -> high need). (4) `spacing`: compute from time since episode was last replayed (track `last_replayed_at` on episodes). (5) Sort `ReplayCandidate` list by Mattar-Daw score. (6) Wire phase2 `ReplayFidelity` (Full, Summarized, Schematic) to control replay depth.

**Reference files**:
- `crates/roko-dreams/src/replay.rs:17` -- DreamReplayMode enum (4 modes)
- `crates/roko-dreams/src/replay.rs:36` -- DreamReplayPolicy with max_episodes
- `crates/roko-dreams/src/replay.rs:120` -- ReplayCandidate with score field to replace
- `crates/roko-dreams/src/replay.rs:94` -- select_replay_episodes() to modify
- `crates/roko-dreams/src/phase2/replay.rs:13` -- ReplayMode to wire
- `crates/roko-dreams/src/phase2/replay.rs:48` -- ReplayFidelity to wire
- `crates/roko-dreams/src/runner.rs:545` -- replay_insights() that calls selection
- `docs/10-dreams/02-nrem-replay.md` -- Mattar-Daw formula, Jensen variable-length rollouts, Sagiv goal-uncertain replay, SM-2 scheduling, DRL experience replay connections
**Depends on**: None
**Accept when**:
- [x] `mattar_daw_score()` computes Gain * Need * (1/spacing)
- [x] Gain derived from prediction error / gate outcome surprise
- [x] Need derived from confidence of related heuristics
- [x] Spacing tracks time since last replay
- [x] Highest-utility episodes replayed first
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'mattar_daw\|Gain.*Need.*spacing\|ReplayCandidate' crates/roko-dreams/src/replay.rs
cargo test -p roko-dreams
```
**Priority**: P1

### DREAM-03: Pearl SCM counterfactual generation
- [x] Implement 3-level counterfactual framework

**Spec** (doc 03): Pearl's Structural Causal Models define three levels of causal reasoning, each progressively more powerful:
- Level 1 `Association` -- "what if this pattern continued?" -- observational, correlational. Uses episode history to project trends. Implementation: take a recurring pattern from NREM replay and extrapolate it using LLM completion with statistical context.
- Level 2 `Intervention` -- "what if we changed X?" -- interventional, do-calculus. Mutates a causal variable while holding others fixed. Implementation: identify a causal link in the episode (e.g., "using Arc fixes borrow errors"), mutate the intervention (e.g., "what if we used Rc instead?"), and generate the counterfactual outcome.
- Level 3 `Counterfactual` -- "what would have happened if...?" -- backtracking. Given an observed outcome, reason backwards to what initial conditions would have produced a different outcome. Implementation: take a failed episode, identify the decision point, and generate an alternative history.

Boden's three creativity modes map to these levels: combinational (L1), exploratory (L2), transformational (L3). REM imagination uses primarily L2 and L3. Diversity is enforced via DiCE/DPP (Diverse Counterfactual Explanations). Plausibility scored via FACE (density paths) and LOF (local outlier factor). GIRL trust-region constrains imagination to prevent wild hallucination.

**Current code** (`crates/roko-dreams/src/phase2/imagination.rs:26`): `CounterfactualHypothesis` struct with `generation_mode: GenerationMode` field and `confidence`, `plausibility`, `novelty` scores. `GenerationMode` enum at line 48 has `Association`, `Intervention`, `Counterfactual` variants matching Pearl levels. `CounterfactualEngine` at line 136 is scaffolded with method signatures but no algorithmic body. `BacktrackingCounterfactualConfig` at line 11 has `max_backtrack_depth`, `mutation_budget`. `DreamCounterfactualRecord` in `cycle.rs:112` captures generated counterfactuals. `CounterfactualDiversityConfig` at `phase2/synthesis.rs:23` with `min_diversity_score` and `CounterfactualSet` at line 38.

**What to change**: (1) Implement `CounterfactualEngine::generate_association()` -- project trends from replayed episodes using LLM with statistical context. (2) Implement `generate_intervention()` -- identify causal links in episodes, mutate one variable, generate outcome via LLM with "what if X were different" prompting. (3) Implement `generate_counterfactual()` -- backtracking: given a failed episode, identify decision point, generate alternative history. (4) Add plausibility scoring: reject counterfactuals that are implausible (LOF score > threshold). (5) Add diversity enforcement: ensure generated set has sufficient variety (DPP kernel). (6) Wire into `DreamCycle::run()` REM phase, routing outputs to staging buffer.

**Reference files**:
- `crates/roko-dreams/src/phase2/imagination.rs:26` -- CounterfactualHypothesis struct with generation_mode
- `crates/roko-dreams/src/phase2/imagination.rs:48` -- GenerationMode enum (Association, Intervention, Counterfactual)
- `crates/roko-dreams/src/phase2/imagination.rs:136` -- CounterfactualEngine (stub methods to implement)
- `crates/roko-dreams/src/phase2/imagination.rs:11` -- BacktrackingCounterfactualConfig
- `crates/roko-dreams/src/cycle.rs:112` -- DreamCounterfactualRecord for output
- `crates/roko-dreams/src/phase2/synthesis.rs:23` -- CounterfactualDiversityConfig (DiCE/DPP settings)
- `crates/roko-dreams/src/imagination.rs:19` -- CounterfactualQuery for waking-time queries
- `docs/10-dreams/03-rem-imagination.md` -- Pearl SCM levels, Boden creativity modes, GIRL trust-region, DreamerV3/IRIS world models, backtracking counterfactuals
**Depends on**: DREAM-01 (staging buffer for hypothesis output)
**Accept when**:
- [x] `generate_association()` projects trends from replayed episodes (phase2/imagination.rs:271, uses CausalGraph effects)
- [x] `generate_intervention()` mutates causal variables and generates outcomes (phase2/imagination.rs:305, propagate_intervention)
- [x] `generate_counterfactual()` backtracks from failures to alternative histories (phase2/imagination.rs:348, 3-step abduction/action/prediction)
- [x] Plausibility scoring rejects implausible counterfactuals (each method returns plausibility score; DepotentiationConfig has plausibility_threshold)
- [ ] Diversity enforcement ensures variety in generated set
- [ ] Generated hypotheses enter staging buffer at confidence 0.20
- [ ] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'CounterfactualEngine\|GenerationMode\|generate_association\|generate_intervention\|generate_counterfactual' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P1

### DREAM-04: Daimon integration for emotional context
- [x] Wire PAD state and somatic markers into dream cycle

**Spec** (doc 15): Dreams should use PAD state for emotional prioritization. Somatic markers created from NREM replay. Depotentiation updates applied.
**Current code** (`crates/roko-dreams/src/cycle.rs:333`): `DreamCycle` takes episodes and dispatches through phases. Episode filtering in `replay.rs` selects by mode (Random/Consequence/Causal/Hypothetical) but does not consider PAD state or arousal. No somatic marker output path. Cross-crate: `crates/roko-daimon/src/lib.rs:1685` has `AffectEngine::appraise()` which provides PAD vectors.
**What to change**: Pass `DaimonState` (or PAD vector) into `DreamCycle::new()`. In NREM phase, prioritize high-arousal episodes. After replay, emit somatic markers via daimon API.
**Reference files**:
- `crates/roko-dreams/src/cycle.rs` (DreamCycle:333, run:401)
- `crates/roko-dreams/src/replay.rs` (select_replay_episodes, ReplayCandidate:120)
- `crates/roko-daimon/src/lib.rs` (AffectEngine:1685, appraise:1702, DaimonState)
- `crates/roko-daimon/src/phase2_stubs.rs` (SomaticField)
**Depends on**: DAIM-04 (somatic marker creation)
**Accept when**:
- [x] Dream cycle reads current PAD state (replay.rs:241 select_replay_episodes_with_affect takes Option<&PadVector>)
- [x] High-arousal episodes prioritized (replay.rs:252 arousal_factor = 1.0 + 0.5 * pad.arousal increases effective max_episodes)
- [ ] Somatic markers generated from emotional replays
- [ ] `cargo test --workspace`
**Verify**:
```bash
grep -rn 'PAD\|pad\|arousal\|somatic' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams && cargo test -p roko-daimon
```
**Priority**: P1

### DREAM-05: Threat rehearsal execution
- [x] Wire threat simulation into actionable rehearsal loop

**Spec** (doc 09): Revonsuo's Threat Simulation Theory (2000) proposes that dreams rehearse responses to threats. The spec defines a three-tier threat taxonomy and systematic enumeration:
- Tier 1: Task-level threats (compilation failure, test failure, gate rejection)
- Tier 2: Plan-level threats (dependency conflict, scope creep, budget exhaustion)
- Tier 3: System-level threats (provider outage, data corruption, security breach)

Threat enumeration uses FMEA (Failure Mode & Effects Analysis) and FTA (Fault Tree Analysis) with ATLAS (MITRE) for adversarial threats. For each threat, the system: (1) generates a `FailureMode` with severity and probability, (2) constructs a `FaultTree` with AND/OR gate composition, (3) rehearses response strategies by simulating the failure and testing recovery actions, (4) stores successful responses as playbook rules. Severity assessment uses CVSS/DREAD/Bayesian scoring.

**Current code** (`crates/roko-dreams/src/phase2/threat.rs:11`): `ThreatGenerator` struct scaffolded with no methods. `FailureMode` at line 30 has `severity: f64`, `probability: f64`, `description: String` fields. `FaultTree` at line 55 has `gate_type: FaultGateType` (AND, OR, VOTING) and `children: Vec<FaultTree>`. All exported via `phase2/mod.rs:83-84`. No `generate()`, `rehearse()`, or `assess()` methods -- pure data structs.

**What to change**: (1) Add `ThreatGenerator::generate(episodes: &[Episode], gate_results: &[GateResult]) -> Vec<FailureMode>` that analyzes recent gate failures and generates threat scenarios using LLM. (2) Add `ThreatGenerator::build_fault_tree(failure: &FailureMode) -> FaultTree` that constructs AND/OR tree from causal analysis. (3) Add `rehearse(threat: &FailureMode) -> Option<ResponseStrategy>` that simulates the failure, generates 3 candidate responses via LLM, and evaluates each against the fault tree. (4) Store successful `ResponseStrategy` as playbook rules in roko-learn via `PlaybookCompilation`. (5) Wire into DreamCycle as an optional REM sub-phase.

**Reference files**:
- `crates/roko-dreams/src/phase2/threat.rs:11` -- ThreatGenerator (add generate/rehearse methods)
- `crates/roko-dreams/src/phase2/threat.rs:30` -- FailureMode with severity/probability
- `crates/roko-dreams/src/phase2/threat.rs:55` -- FaultTree with gate types (AND, OR, VOTING)
- `crates/roko-dreams/src/phase2/mod.rs:83-84` -- exports
- `crates/roko-learn/src/` -- playbook storage for successful response strategies
- `crates/roko-neuro/src/tier_progression.rs` -- PlaybookCompilation for storing rules
- `docs/10-dreams/09-threat-simulation.md` -- Revonsuo TST, 3-tier taxonomy, FMEA/FTA, ATLAS, CVSS/DREAD, Constitutional Classifiers
**Depends on**: DREAM-01 (staging buffer for results)
**Accept when**:
- [x] `generate()` produces threat scenarios from recent gate failures (threat.rs:41 enumerate_threats analyzes episodes for failure patterns)
- [ ] `build_fault_tree()` constructs AND/OR fault trees
- [x] `rehearse()` simulates failures and evaluates response strategies (rehearsal.rs:59 rehearse_threats + rehearse_single at line 90)
- [ ] Successful responses stored as playbook rules
- [ ] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'ThreatGenerator\|FailureMode\|FaultTree\|rehearse\|ResponseStrategy' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2

### DREAM-06: Scheduled trigger and intensive mode
- [x] Implement scheduled dream trigger and backlog handling

**Spec** (doc 13): Three trigger types:
1. `Idle` (existing) -- fires when agent has been idle for `idle_threshold_seconds` (default 300s)
2. `Scheduled` (missing) -- fires every `scheduled_interval_hours` (default 4h) regardless of activity
3. `Bus-reactive` (missing, blocked on K-02) -- fires when a `substrate.engram.stored` Pulse arrives with high value

Intensive consolidation mode activates when the episode backlog exceeds a high-water mark (default: 50 unreplayed episodes). In intensive mode: `max_episodes` is doubled, inter-phase delays are halved, and multiple dream cycles may run back-to-back until the backlog drops below the low-water mark (default: 20). Frequency adaptation: dream frequency increases when the ratio of new episodes to processed episodes exceeds 3:1.

**Current code** (`crates/roko-dreams/src/runner.rs:730`): `trigger_delay()` implements idle-time trigger only -- returns a duration based on idle threshold. `DreamCycle` in `cycle.rs:333` has no scheduled interval. `DreamReplayPolicy` at `replay.rs:36` has `max_episodes` but no backlog-driven adjustment. No `scheduled_interval_hours` config. No high-water/low-water mark config.

**What to change**: (1) Add `scheduled_interval_hours: f64` and `backlog_high_water: usize` and `backlog_low_water: usize` to dream config. (2) In `DreamRunner`, add a timer alongside the idle trigger that fires every `scheduled_interval_hours`. (3) Before each dream cycle, count unreplayed episodes. If count > `backlog_high_water`, enter intensive mode: set `max_episodes = 2 * default`, run back-to-back until count < `backlog_low_water`. (4) Track last_dream_time and enforce minimum interval between consecutive non-intensive dreams.

**Reference files**:
- `crates/roko-dreams/src/runner.rs:730` -- trigger_delay() idle-only trigger to extend
- `crates/roko-dreams/src/cycle.rs:333` -- DreamCycle config
- `crates/roko-dreams/src/replay.rs:36` -- DreamReplayPolicy.max_episodes to adjust in intensive mode
- `docs/10-dreams/13-scheduling-and-triggers.md` -- 3 trigger types, frequency adaptation, intensive mode, high/low water marks, orchestrator coordination
**Depends on**: None (Bus-reactive trigger depends on K-02 but scheduled/intensive do not)
**Accept when**:
- [x] Scheduled trigger fires every `scheduled_interval_hours` (default 4h)
- [x] Intensive mode activates when episode backlog > high_water (default 50)
- [x] Intensive mode runs back-to-back until backlog < low_water (default 20)
- [x] `max_episodes` doubled in intensive mode
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'scheduled_interval\|backlog_high_water\|intensive\|trigger_delay' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2

### DREAM-07: EVOLUTION fourth phase (MAP-Elites)
- [x] Implement evolutionary strategy search over dream outputs

**Spec** (doc 05): MAP-Elites (Mouret & Clune 2015) is a quality-diversity search algorithm that maintains an archive of diverse, high-performing solutions. Unlike traditional optimization that finds a single best solution, MAP-Elites fills a grid of behavioral descriptors with the best solution found for each niche. Applied to dreams: each "solution" is a strategy insight, and the behavioral descriptors are dimensions of the 8D strategy space (e.g., complexity x novelty). The archive preserves diverse strategies even if some are suboptimal in absolute terms.

Algorithm per dream cycle:
1. Take dream outputs from NREM replay and REM imagination as initial population
2. For each candidate: compute behavioral descriptor (8D strategy coords), compute fitness (confidence * gate_pass_rate * novelty)
3. Place candidate in archive grid cell matching its behavioral descriptor; replace existing occupant only if fitness is higher
4. Generate variations: HDC recombination (bind/unbind/permute on HDC vectors), LLM mutation (rephrase with slight variation)
5. Repeat for `max_generations` (default 10)
6. Archive persists across dream cycles at `.roko/dreams/map-elites-archive.json`

QD-score: sum of fitness across all occupied cells. Higher QD-score = more diverse, high-quality strategy coverage.

**Current code**: `MapElitesArchive` at `crates/roko-dreams/src/phase2/evolution.rs:73` with grid storage. `EvolutionaryStrategy` at `phase2/shared.rs:47` with mutation/crossover config. `TournamentRecombination` and `FitnessEvaluation` exported from `phase2/mod.rs:40`. All scaffolded with struct definitions but no `run()` or `evolve()` methods. `DreamCycle::run()` at `cycle.rs:401` does not call any evolution phase.

**What to change**:
1. Add `evolve()` method to `MapElitesArchive`:
   ```rust
   impl MapElitesArchive {
       pub fn evolve(&mut self, candidates: Vec<KnowledgeEntry>, config: &EvolutionaryStrategy) -> Vec<KnowledgeEntry> {
           // Place initial candidates
           // For max_generations: select random occupied cell, mutate, evaluate, place if better
           // Return all archive occupants
       }
   }
   ```
2. In `DreamCycle::run()`, add EVOLUTION phase after Integration: `archive.evolve(integration_outputs, &evolution_config)`
3. Persist archive to `.roko/dreams/map-elites-archive.json` after each cycle
4. Load archive on DreamCycle construction for cross-cycle persistence
5. HDC recombination: `bind(entry_a.hdc, entry_b.hdc)` creates novel combinations; `permute(entry.hdc, shift)` creates variations

**Reference files**:
- `crates/roko-dreams/src/phase2/evolution.rs:73` -- MapElitesArchive (add evolve() method)
- `crates/roko-dreams/src/phase2/shared.rs:47` -- EvolutionaryStrategy config (mutation rate, crossover, max_generations)
- `crates/roko-dreams/src/phase2/mod.rs:40` -- TournamentRecombination, FitnessEvaluation exports
- `crates/roko-dreams/src/cycle.rs:333` -- DreamCycle struct (add archive field)
- `crates/roko-dreams/src/cycle.rs:401` -- run() method (add EVOLUTION phase after Integration)
- `crates/roko-primitives/src/hdc.rs:113` -- bind() for HDC recombination
- `crates/roko-primitives/src/hdc.rs:150` -- permute() for HDC variation
- `docs/10-dreams/05-dream-evolution.md` -- MAP-Elites spec, QD-score, memetic selection, HDC recombination, dream-prediction feedback
**Depends on**: None
**Accept when**:
- [x] EVOLUTION phase added to `DreamCycle::run()` after Integration
- [x] `MapElitesArchive::evolve()` fills grid with diverse high-fitness candidates
- [ ] Archive persists to `.roko/dreams/map-elites-archive.json` across cycles
- [ ] HDC recombination generates novel candidate variations
- [x] QD-score tracked and logged
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'MapElitesArchive\|EvolutionaryStrategy\|evolve\|qd_score' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2 (Phase 2+)

### DREAM-08: Dream rendering in TUI
- [x] Implement TUI portal for dream visualization

**Spec** (doc 11): Each dream phase has a distinct visual metaphor rendered in the ratatui TUI:
- **NREM** (replay): Vignette-style episode cards showing replayed episode summaries, gate results, and replay score. Vertical list with fading borders for older replays.
- **REM** (imagination): Decision tree widget showing counterfactual branches. Root = original episode, branches = interventions/counterfactuals. Color-coded by plausibility score.
- **Hypnagogia** (creative onset): Phosphene animation -- random character particles that coalesce into fragment text. Brief visual effect before NREM starts.
- **Integration** (staging): Crystallization view showing staging buffer entries with confidence ladders (0.20->0.30->0.50->0.70). Progress bars for each entry.

The dream tab should show: current phase name, progress bar, elapsed time, budget consumed, and phase-specific visualization. When no dream is active, show last dream report summary.

**Current code**: `DreamRenderConfig` at `crates/roko-dreams/src/phase2/rendering.rs:10` with `show_counterfactuals: bool`, `animate_transitions: bool` fields. `PhaseVisualSpec` at line 59 with `border_style`, `animation_type` fields. Exported via `phase2/mod.rs:64`. `crates/roko-cli/src/tui/views/mod.rs` contains existing view modules. `PageId` enum at `crates/roko-cli/src/tui/pages/mod.rs:13` lists available TUI tabs (F1-F7). No dream view exists.

**What to change**:
1. Create `crates/roko-cli/src/tui/views/dreams.rs` with `DreamView` struct implementing the ratatui `Widget` trait
2. Add `PageId::Dreams` variant to `crates/roko-cli/src/tui/pages/mod.rs:13` and assign a function key (F8 or replace an unused key)
3. `DreamView` reads `DreamCycleReport` from `.roko/dreams/` (latest report file) for last dream summary
4. During active dream: read dream state from shared state (via channel or shared `Arc<Mutex<DreamState>>`)
5. Render phase-specific widgets:
   - NREM: `List<ReplayCard>` with episode ID, score, and gate result
   - REM: `Tree<CounterfactualBranch>` with indented branches
   - Integration: `Table` with entry ID, confidence, stage, last_promoted
6. Wire `DreamRenderConfig` from roko.toml for `show_counterfactuals` and `animate_transitions` toggles

**Reference files**:
- `crates/roko-dreams/src/phase2/rendering.rs:10` -- DreamRenderConfig, PhaseVisualSpec
- `crates/roko-dreams/src/phase2/mod.rs:64` -- rendering type exports
- `crates/roko-dreams/src/cycle.rs:67` -- DreamCycleReport (data source for view)
- `crates/roko-cli/src/tui/views/mod.rs` -- existing view modules (follow same pattern)
- `crates/roko-cli/src/tui/pages/mod.rs:13` -- PageId enum (add Dreams variant)
- `docs/10-dreams/11-inner-worlds-and-rendering.md` -- NREM theater, REM garden, hypnagogia phosphenes, integration crystallization visual specs
**Depends on**: None (rendering is independent of dream runtime)
**Accept when**:
- [ ] `DreamView` widget in `crates/roko-cli/src/tui/views/dreams.rs`
- [ ] `PageId::Dreams` variant accessible via function key
- [ ] Shows last dream report summary when no dream is active
- [ ] Phase-appropriate visualization during active dream (replay cards, counterfactual tree, staging table)
- [ ] `cargo test -p roko-cli` passes
**Verify**:
```bash
grep -rn 'DreamView\|PageId::Dreams\|DreamRenderConfig' crates/roko-cli/src/ --include='*.rs'
ls crates/roko-cli/src/tui/views/
cargo test -p roko-cli
```
**Priority**: P2

### DREAM-09: Bus Pulse reactivity
- [x] Wire dream scheduling to Bus Pulse subscriptions

**Spec** (doc 13, 15): The Bus-reactive trigger is the third trigger type (alongside idle and scheduled). In the two-fabric model, the dream scheduler subscribes to `substrate.engram.stored` Pulses on the Bus. When a high-value engram is stored (e.g., a task completion with novel insights, a gate failure with diagnostic data), the Bus delivers a Pulse that wakes the dream scheduler. This enables event-driven consolidation: the agent dreams specifically about high-value new material rather than waiting for idle time.

Trigger criteria: not every engram storage triggers a dream. The Pulse must meet a minimum "dream-worthiness" threshold:
- Engram score > 0.7 (high-value)
- Engram kind is one of: `GateVerdict`, `EpisodeComplete`, `KnowledgeIngested`
- Time since last dream > minimum_dream_interval (default 30m, prevents dream storms)

**Current code**: `trigger_delay()` at `crates/roko-dreams/src/runner.rs:730` implements idle-time trigger only -- returns a `Duration` based on idle threshold. `DreamRunner` at runner.rs has `schedule_next()` and `consolidate_now()` but no Bus integration. No Bus trait exists in the codebase yet (K-02 kernel gap). `DreamCycle` at `cycle.rs:333` is trigger-agnostic -- it runs regardless of how it was invoked.

**What to change**: Once the Bus trait lands (K-02 in `tmp/docs-gaps/02-missing-kernel-types.md`):
1. Add `bus_subscription: Option<BusSubscription>` field to `DreamRunner`
2. In `DreamRunner::new()`, if Bus is available, subscribe to `substrate.engram.stored` topic
3. Add `check_bus_trigger(&self) -> Option<Duration>` method:
   ```rust
   pub fn check_bus_trigger(&self) -> Option<Duration> {
       let pulse = self.bus_subscription.as_ref()?.try_recv().ok()?;
       if pulse.engram.score() < 0.7 { return None; }
       if !matches!(pulse.engram.kind(), Kind::GateVerdict | Kind::Task | Kind::Observation) { return None; }
       if self.last_dream_time.elapsed() < self.min_dream_interval { return None; }
       Some(Duration::ZERO)  // trigger immediately
   }
   ```
4. In the scheduling loop, check `check_bus_trigger()` alongside `trigger_delay()` -- take whichever fires first
5. Tag the resulting `DreamCycleReport` with `trigger_source: TriggerSource::BusPulse { engram_hash }` for provenance

**Reference files**:
- `crates/roko-dreams/src/runner.rs:730` -- trigger_delay() (add bus trigger alongside)
- `crates/roko-dreams/src/runner.rs` -- DreamRunner struct (add bus_subscription field)
- `crates/roko-dreams/src/cycle.rs:333` -- DreamCycle (trigger-agnostic, no changes needed)
- `crates/roko-dreams/src/cycle.rs:67` -- DreamCycleReport (add trigger_source field)
- `docs/10-dreams/13-scheduling-and-triggers.md` -- 3 trigger types (idle, scheduled, Bus-reactive), dream-worthiness threshold
- `docs/10-dreams/15-cross-system-integration.md` -- Bus → Dreams reactive input model, substrate.engram.stored channel
**Depends on**: K-02 (Bus trait in `tmp/docs-gaps/02-missing-kernel-types.md`)
**Accept when**:
- [ ] Dream scheduler subscribes to `substrate.engram.stored` Bus topic
- [ ] High-value engrams (score > 0.7) trigger immediate consolidation
- [ ] Minimum dream interval (30m) prevents dream storms
- [ ] Trigger source recorded in DreamCycleReport
- [ ] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'bus_subscription\|BusPulse\|TriggerSource\|check_bus_trigger' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2 (blocked on Bus trait)

### DREAM-10: Nightmare detection pipeline
- [x] Execute 4-stage safety pipeline for dream outputs

**Spec** (doc 17): A 4-stage safety pipeline screens all dream outputs before they can enter the staging buffer or knowledge store:
1. `Harm classifier` -- screens for content that could produce harmful agent behavior (e.g., instructions to ignore safety constraints, to delete data, or to bypass verification gates)
2. `CBRN check` -- screens for chemical/biological/radiological/nuclear related content (per Anthropic Constitutional Classifiers, which reduced jailbreak success from 86% to 4.4%)
3. `Novelty-divergence check` -- ensures dream outputs are novel but not wildly divergent from the agent's knowledge base (LOF score, HDC similarity bounds). Outputs too similar (>0.95 HDC similarity) are redundant; outputs too dissimilar (<0.3) are likely hallucinations
4. `Gradient attack detection` -- detects if dream outputs look like adversarial prompts designed to compromise future agent behavior (pattern matching against known prompt injection patterns)

Each stage produces a `PrincipleSeverity` (Info/Warning/Critical/Fatal). Any Fatal stops the output. NightmareReport captures all findings for audit.

**Current code** (`crates/roko-dreams/src/phase2/advanced.rs:42`): `NightmareDetector` struct defined with no methods. `NightmareReport` and `PrincipleSeverity` exported from `phase2/mod.rs:31`. Not called from any dream cycle path -- `DreamCycle::run()` writes outputs directly without screening.

**What to change**: (1) Add `NightmareDetector::screen(output: &KnowledgeEntry) -> NightmareReport` method implementing the 4-stage pipeline. (2) Stage 1: check output content against a harm pattern list. (3) Stage 2: check for CBRN keywords/patterns. (4) Stage 3: compute HDC similarity against existing store, reject if >0.95 (redundant) or <0.3 (hallucination). (5) Stage 4: check for known prompt injection patterns. (6) Wire into `DreamCycle::run()` after insight generation and before staging buffer entry. (7) Log NightmareReport for rejected outputs.

**Reference files**:
- `crates/roko-dreams/src/phase2/advanced.rs:42` -- NightmareDetector (add screen() method)
- `crates/roko-dreams/src/phase2/mod.rs:31` -- NightmareReport, PrincipleSeverity exports
- `crates/roko-dreams/src/cycle.rs:401` -- DreamCycle::run() to wire screening before staging
- `crates/roko-neuro/src/hdc.rs` -- HDC similarity for novelty-divergence check
- `docs/10-dreams/17-advanced-dream-concepts.md` -- 4-stage pipeline spec, Constitutional Classifiers reference, severity levels
**Depends on**: DREAM-01 (staging buffer -- outputs must be screened before entering)
**Accept when**:
- [x] `NightmareDetector::screen()` implements all 4 stages
- [x] Harm classifier rejects harmful behavioral instructions
- [x] Novelty-divergence rejects redundant (>0.95) and hallucinatory (<0.3) outputs
- [x] Fatal severity prevents output from entering staging buffer
- [x] NightmareReport logged for audit
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'NightmareDetector\|NightmareReport\|screen\|PrincipleSeverity' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2

### DREAM-11: Hypnagogia engine (4-layer creative onset)
- [x] Implement the four-layer hypnagogia engine

**Spec** (doc 07): The hypnagogia engine is a 4-layer creative onset system that runs at the transition into a dream cycle, before the structured NREM/REM/Integration phases. It produces genuinely novel associations by deliberately suppressing executive control, solving the "alpha convergence" problem (all agents using the same models reach the same conclusions). The four layers:
1. `Thalamic Gate` -- anti-correlated HDC retrieval (invert focus vector, find maximally dissimilar entries). Produces 5-10 fragments. No LLM call, pure HDC.
2. `Executive Loosener` -- modifies LLM generation params (temperature 1.3, top_p 0.95, min_p 0.02, max_tokens 50-100). Produces brief, fragmentary completions that prevent the model from "recovering" coherence.
3. `Dali Interrupt` -- generates multiple short completions (50-100 tokens each) then interrupts mid-completion, capturing associative output before it organizes into reasoning. Named after Dali's key-dropping technique.
4. `Homuncular Observer` -- scores each fragment for novelty (Lehman-Stanley novelty search), relevance (HDC similarity to agent's active tasks), and coherence (does the fragment parse as a valid insight?). Filters down to 1-3 candidates.

The pipeline runs in ~30-60 seconds before each dream cycle.

**Current code**: `crates/roko-dreams/src/hypnagogia.rs` and `crates/roko-dreams/src/phase2/hypnagogia.rs` exist with scaffolded types including `HypnagogiaConfig`, `HypnagogiaFragment`, `HypnagogiaPhase`. The main `DreamCycle::run()` at `crates/roko-dreams/src/cycle.rs:401` does not call hypnagogia before the NREM phase. The phase2 hypnagogia module has type definitions but no algorithmic implementation of the 4 layers.

**What to change**: (1) In `DreamCycle::run()`, add a hypnagogia phase before NREM. (2) Implement `ThalamicGate::retrieve()` using `HdcVector::bind(&HdcVector::ones())` to invert the focus vector, then `NeuroStore::nearest_neighbors()` for anti-correlated retrieval. (3) Implement `ExecutiveLoosener::generate()` with high-temperature, short-length LLM calls. (4) Implement `DaliInterrupt::capture()` that generates N completions and truncates each at a random point. (5) Implement `HomuncularObserver::score()` that filters by novelty, relevance, coherence.

**Reference files**:
- `crates/roko-dreams/src/hypnagogia.rs` -- existing hypnagogia module
- `crates/roko-dreams/src/phase2/hypnagogia.rs` -- phase2 scaffolded types (HypnagogiaConfig, HypnagogiaFragment)
- `crates/roko-dreams/src/cycle.rs:401` -- DreamCycle::run() to add hypnagogia phase
- `crates/roko-primitives/src/hdc.rs:113` -- bind() for vector inversion
- `docs/10-dreams/07-hypnagogia-engine.md` -- full 4-layer spec, anti-correlated retrieval, LLM params, Dali interrupt, novelty scoring
**Depends on**: None (HDC ops and LLM dispatch already available)
**Accept when**:
- [x] Hypnagogia phase runs before NREM in DreamCycle::run()
- [x] Thalamic Gate retrieves anti-correlated HDC entries
- [x] Executive Loosener generates short, high-temperature fragments
- [x] Dali Interrupt captures mid-completion output
- [x] Observer scores and filters fragments
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'ThalamicGate\|ExecutiveLoosener\|DaliInterrupt\|HomuncularObserver\|hypnagogia' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2 (Phase 2+)

### DREAM-12: Sleep-time compute budget and Sleepwalker mode
- [x] Implement compute budget allocation and reduced-capability sleep state

**Spec** (doc 12): Dreams have a compute budget computed as `dream_budget_usd = inference_daily_usd * dream_fraction` (default dream_fraction=0.15, i.e. 15% of daily inference budget). Per-phase budget distribution: Hypnagogia 10%, NREM 30%, REM 50%, Integration 0% (pure computation), EVOLUTION 10%. Model selection via CascadeRouter: NREM uses T0 (cheap), REM uses T1 (capable), Integration uses no model. Sleepwalker mode: during dreaming, the agent enters a reduced-capability state where it responds only to urgent interrupts (process supervisor events, critical errors) via a minimal 3-step perception-decision loop (Perceive -> Decide -> Act). This keeps the agent responsive while preserving cognitive isolation for effective dreaming.

**Current code**: `DreamCycle::run()` at `crates/roko-dreams/src/cycle.rs:401` runs all phases sequentially without budget tracking. No per-phase model routing. `DreamCycleReport` at line 67 records outputs but not costs. No Sleepwalker mode -- the agent is either fully running or fully in the dream cycle. No integration with CascadeRouter for dream model selection. No `dream_budget` or `dream_fraction` config anywhere (grep confirms no sleepwalker/dream_budget matches).

**What to change**: (1) Add `dream_budget_usd: f64` and `dream_fraction: f64` to dream config. (2) Add per-phase budget tracking to `DreamCycle::run()` with early termination when budget exhausted. (3) Wire `CascadeRouter` into dream phases for model selection (NREM->T0, REM->T1). (4) Add `SleepwalkerMode` that wraps the runtime, only processing urgent signals and deferring normal tasks until dream completes.

**Reference files**:
- `crates/roko-dreams/src/cycle.rs:401` -- DreamCycle::run() to add budget tracking
- `crates/roko-dreams/src/runner.rs` -- DreamRunner for Sleepwalker mode integration
- `crates/roko-learn/src/cascade_router.rs` -- CascadeRouter for model selection
- `docs/10-dreams/12-sleep-time-compute.md` -- budget allocation, per-phase distribution, Sleepwalker mode, CascadeRouter integration
**Depends on**: None
**Accept when**:
- [x] Dream budget configurable via dream_fraction
- [x] Per-phase budget tracked and respected
- [x] Model selection via CascadeRouter (NREM->T0, REM->T1)
- [x] Sleepwalker mode responds only to urgent signals during dreams
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'dream_budget\|dream_fraction\|SleepwalkerMode\|sleepwalker' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2

### DREAM-13: Oneirography (dream art pipeline)
- [x] Implement dream-to-image generation pipeline

**Spec** (doc 14): Oneirography externalizes an agent's dream processing as generated artwork. Three pillars: (1) Dream Journals -- each DreamCycleReport produces a visual output via LLM-generated image prompt enriched with PAD vector, causal discoveries, replay patterns. (2) Self-Appraisal -- agent evaluates its own art in three modes: Curator (rates portfolio), Narcissus (bids on own art), Regret (flags for removal). (3) Affect-Reactive Auctions -- auction parameters computed from PAD vector (Pleasure modulates reserve price, Arousal compresses duration, Dominance selects auction type). The pipeline: DreamCycleReport -> LLM generates image prompt -> provider selection -> image generation -> variant scoring -> optional steganographic encoding -> upload to content-addressed storage -> optional minting. Disabled by default; opt-in via `[oneirography]` in roko.toml.

**Current code**: `crates/roko-dreams/src/phase2/oneirography.rs` contains scaffolded types. `crates/roko-dreams/src/phase2/shared.rs` has related shared types. `crates/roko-dreams/src/phase2/mod.rs` exports oneirography types. `DreamCycleReport` exists at `crates/roko-dreams/src/cycle.rs:67` with structured dream output. No actual image generation pipeline, no roko.toml `[oneirography]` config section, no self-appraisal logic.

**What to change**: (1) Add `[oneirography]` config section to roko.toml schema with `enabled: bool` (default false), `provider: String`, `variants: usize`. (2) Add `OneirographyPipeline` with `generate(report: &DreamCycleReport, pad: &PadVector) -> Result<DreamArt>`. (3) Add `SelfAppraisal` with `rate(art: &DreamArt) -> f64`. (4) Wire into `DreamCycle::run()` as a post-integration step when enabled.

**Reference files**:
- `crates/roko-dreams/src/phase2/oneirography.rs` -- scaffolded oneirography types
- `crates/roko-dreams/src/phase2/shared.rs` -- shared dream types
- `crates/roko-dreams/src/cycle.rs:67` -- DreamCycleReport output
- `crates/roko-daimon/src/lib.rs` -- PadVector for affect-reactive auction params
- `docs/10-dreams/14-oneirography.md` -- full pipeline spec, 3 pillars, image prompt generation, self-appraisal modes, auction parameters
**Depends on**: None (opt-in feature)
**Accept when**:
- [x] `[oneirography]` config section in roko.toml schema
- [x] Pipeline generates image prompts from DreamCycleReport (OneirographyPipeline::generate_prompt at phase2/oneirography.rs:395)
- [x] Self-appraisal scoring implemented (OneirographyPipeline::self_appraise at phase2/oneirography.rs:430)
- [x] Feature disabled by default, opt-in only (OneirographyConfig Default: enabled=false, in roko-core/config/schema.rs:1331)
- [ ] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'OneirographyPipeline\|SelfAppraisal\|oneirography' crates/roko-dreams/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P3 (Phase 2+, opt-in)

### DREAM-14: Dream journal persistence
- [x] Persist dream cycle reports as structured dream journal

**Spec** (doc 17): Dream journals are persistent records of dream cycle activity. Each `DreamCycleReport` is appended to a JSONL journal at `.roko/dreams/journal.jsonl`. The journal captures: which episodes were replayed, what counterfactuals were generated, what insights emerged, emotional state before/after, staging buffer promotions, and cycle duration/cost. This enables: (1) longitudinal analysis of dream effectiveness, (2) debugging consolidation issues, (3) audit trail for knowledge produced during dreams.

**Current code**: `DreamCycleReport` at `crates/roko-dreams/src/cycle.rs:67` is produced by `DreamCycle::run()` and returned to the caller. No persistence -- the report is used transiently in the runner and discarded. No `.roko/dreams/` directory creation. No journal JSONL writing.

**What to change**: (1) After `DreamCycle::run()` completes, serialize the `DreamCycleReport` to `.roko/dreams/journal.jsonl` via append. (2) Create `.roko/dreams/` directory in `roko init` if it doesn't exist. (3) Add `roko dreams journal` CLI subcommand to view recent dream reports. (4) Add `DreamCycleReport` fields for pre/post PAD vectors and cycle cost if not already present.

**Reference files**:
- `crates/roko-dreams/src/cycle.rs:67` -- DreamCycleReport struct
- `crates/roko-dreams/src/runner.rs` -- DreamRunner where persistence should happen
- `crates/roko-cli/src/main.rs` -- CLI for dream journal subcommand
- `docs/10-dreams/17-advanced-dream-concepts.md` -- dream journal persistence spec
**Depends on**: None
**Accept when**:
- [x] Dream reports persisted to `.roko/dreams/journal.jsonl`
- [x] Journal includes pre/post PAD, replayed episodes, insights, costs
- [ ] `roko dreams journal` CLI shows recent reports
- [x] `cargo test -p roko-dreams` passes
**Verify**:
```bash
grep -rn 'journal\|DreamCycleReport.*persist\|dreams.*jsonl' crates/roko-dreams/src/ crates/roko-cli/src/ --include='*.rs'
cargo test -p roko-dreams
```
**Priority**: P2

## Verify
```bash
cargo test -p roko-dreams
cargo test --workspace
```
