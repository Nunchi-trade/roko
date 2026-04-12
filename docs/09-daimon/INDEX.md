# Topic 09: Daimon — Affect Engine

> The agent's internal cognitive-emotional state tracker: PAD vector, ALMA temporal model, OCC/Scherer appraisal, somatic markers, behavioral states, and compute allocation feedback loops.

---

## What This Topic Covers

The Daimon models the agent's internal cognitive state using a PAD (Pleasure-Arousal-Dominance) vector (Mehrabian 1996). Every measurable event — gate pass/fail, task success/failure, time pressure, blockers — produces an appraisal that updates the PAD vector. The PAD state then drives four systems simultaneously: behavioral state selection, tier routing bias, VCG auction bidding, and somatic landscape querying. The result is an agent that automatically adjusts compute allocation, model selection, exploration rate, and strategy preference based on its ongoing experience.

The Daimon is **not cosmetic**. It is not a chatbot personality layer. It is a control signal that directly modulates how much compute the agent spends, which models it uses, and what context it retrieves. An agent with a healthy Daimon integration saves money when things go well (cheaper models, fewer retries) and spends more when things go badly (stronger models, more retries, richer context).

The behavioral states are **cyclical** — Engaged, Struggling, Coasting, Exploring, Focused, Resting — with no terminal state. The agent never "dies." It encounters harder problems, runs low on budget, or accumulates failures — all recoverable conditions.

---

## Sub-documents

| # | File | Summary |
|---|---|---|
| 00 | [00-vision-and-mortality-incompatibility.md](./00-vision-and-mortality-incompatibility.md) | What the Daimon is and is not. Explicit removal of mortality framing. Why `04-mortality-daimon.md` and `05-death-daimon.md` are skipped. |
| 01 | [01-pad-vector.md](./01-pad-vector.md) | Mehrabian 1996 PAD model. Three dimensions (Pleasure, Arousal, Dominance), each [-1, 1]. Eight octant states. Plutchik mapping. Rust structs. PAD cosine similarity. Decay mechanics. |
| 02 | [02-alma-three-layer-temporal.md](./02-alma-three-layer-temporal.md) | Gebhard 2005 ALMA model. Three temporal layers: Emotion (seconds), Mood (hours), Personality (lifetime). Layer interactions. Comparison with alternatives. |
| 03 | [03-occ-scherer-appraisal.md](./03-occ-scherer-appraisal.md) | OCC and Scherer appraisal theory. Full 8-step appraisal pipeline. Appraisal rules for all 6 event types with Rust code. Rung scaling. Asymmetric valence (prospect theory). |
| 04 | [04-six-behavioral-states.md](./04-six-behavioral-states.md) | Six cyclical states: Engaged, Struggling, Coasting, Exploring, Focused, Resting. PAD thresholds. Behavioral modulation parameters. Tier bias table. Dispatch strategies. |
| 05 | [05-behavioral-state-to-tier-routing.md](./05-behavioral-state-to-tier-routing.md) | How behavioral state modulates CascadeRouter prediction error thresholds. Three-tier cognitive architecture (T0/T1/T2). Cost implications. Feedback loop stability. |
| 06 | [06-somatic-markers-damasio.md](./06-somatic-markers-damasio.md) | Damasio 1994 somatic marker hypothesis. k-d tree over 8D strategy space. SomaticLandscape and SomaticMarker structs. Sub-1ms query latency. Marker creation and consolidation. |
| 07 | [07-15-percent-contrarian-retrieval.md](./07-15-percent-contrarian-retrieval.md) | Bower 1981 mood-congruent echo chamber prevention. Rolling 200-tick window. Contrarian tracker. Three complementary loop-breaking mechanisms. Mind wandering. |
| 08 | [08-8-dimensional-strategy-space.md](./08-8-dimensional-strategy-space.md) | Domain-configurable 8D coordinate system. Coding dimensions (Complexity, Risk, Novelty, Confidence, Time Pressure, Scope, Reversibility, Dependency Depth). Chain dimensions. Cross-domain transfer. |
| 09 | [09-mood-congruent-memory.md](./09-mood-congruent-memory.md) | Emotional state biases retrieval. Four-factor scoring (recency × importance × relevance × emotional congruence). EmotionalTag on Engrams. Emotional provenance. Diversity as quality signal. Dream-memory-emotion triangle. |
| 10 | [10-integration-points.md](./10-integration-points.md) | Four integration points: behavioral state selection, tier routing bias, VCG auction bidding (full formula), somatic landscape querying. Integration map. Event emission. |
| 11 | [11-coding-agent-integration.md](./11-coding-agent-integration.md) | Per-crate confidence. Error pattern sensitivity with familiarity scaling. Fatigue detection (consecutive failure monitoring). SystemPromptBuilder integration. |
| 12 | [12-collective-emotional-contagion.md](./12-collective-emotional-contagion.md) | Emotional contagion across agent mesh. P/A attenuation 0.3, D attenuation 0.0. Arousal cap +0.3 per sync. Anti-cascade design. Somatic field formation. C-Factor. |
| 13 | [13-current-status-and-gaps.md](./13-current-status-and-gaps.md) | Implementation status: roko-daimon vs. roko-golem overlap. Consolidation plan (Tier 0C). What's done, what's scaffolded, what's specified but unbuilt. Skipped legacy files. Priority path. |

---

## Key Academic Citations

| Citation | Used In | Contribution |
|---|---|---|
| Mehrabian 1996 | 01, 04, 05, 10, 11 | PAD vector model — the three-dimensional emotional space |
| Russell & Mehrabian 1977 | 01, 04 | Three-factor theory of emotions, empirical validation of PAD |
| Gebhard 2005 | 02 | ALMA layered model — Emotion/Mood/Personality temporal dynamics |
| Ortony, Clore & Collins 1988 | 03 | OCC appraisal model — events/agents/objects classification |
| Scherer 2001 | 03 | Component Process Model — sequential appraisal checks |
| Kahneman & Tversky 1979 | 03 | Prospect theory — loss aversion, asymmetric valence |
| Plutchik 1980 | 01, 09 | Wheel of emotions — categorical emotion classification |
| Damasio 1994 | 06, 08 | Somatic marker hypothesis — emotions as fast heuristics |
| Bechara et al. 1994, 1997 | 06 | Iowa Gambling Task — somatic markers precede conscious awareness |
| Bower 1981 | 07, 09, 12 | Associative network theory — mood-congruent memory |
| Blaney 1986 | 07, 09 | Mood-congruent memory review |
| Faul & LaBar 2022 | 07, 09 | Updated mood-congruent evidence, 5–30% accuracy boost |
| Emotional RAG 2024 | 07, 09 | Empirical validation of emotion-weighted retrieval for LLM agents |
| Walker & van der Helm 2009 | 07, 09 | REM depotentiation — sleep to forget emotion, sleep to remember content |
| McGaugh 2004 | 09 | Amygdala-hippocampal emotional consolidation |
| Ebbinghaus 1885 | 09 | Forgetting curve — exponential memory decay |
| Shinn et al. 2023 (Reflexion) | 09, 11 | Verbal reinforcement learning — self-reflection improves decisions |
| Roediger & Karpicke 2006 | 09 | Testing effect — retrieval strengthens memory trace |
| McAdams 2001 | 09 | Narrative identity — validation arcs (Redemptive, Contaminating, etc.) |
| Abelson 1963 | 09 | Hot cognition — emotionally charged beliefs resist revision |
| Seligman 1967 | 07, 11 | Learned helplessness — sustained failure produces maladaptive passivity |
| Kahneman 2011 | 05, 06 | Dual-process theory — System 1 (fast, heuristic) vs. System 2 (slow, analytical) |
| Chen et al. 2023 (FrugalGPT) | 04, 05 | Cascade architectures for cost-efficient LLM routing |
| Li et al. 2010 (LinUCB) | 05 | Contextual bandits for personalized recommendation |
| Vickrey 1961, Clarke 1971, Groves 1973 | 10 | VCG auction mechanism for truthful bidding |
| Woolley et al. 2010 | 12 | Collective intelligence factor in group performance |
| Hatfield, Cacioppo & Rapson 1993 | 12 | Emotional contagion in social groups |
| Grassé 1959 | 12 | Stigmergy — indirect coordination through environmental traces |
| Kleyko et al. 2022 | 08 | Hyperdimensional computing — vector symbolic architectures |

---

## Key Rust Types

```rust
// roko-daimon/src/lib.rs
pub struct PadVector { pleasure: f64, arousal: f64, dominance: f64 }
pub struct AffectState { pad: PadVector, confidence: f64, updated_at: DateTime<Utc> }
pub enum AffectEvent { GateResult{..}, TaskOutcome{..}, Blocked{..}, TimePressure{..}, QueueWait{..}, DreamFailure{..} }
pub struct DaimonState { state: AffectState, half_life_hours: f64, persistence_path: Option<PathBuf> }
pub trait AffectEngine { fn appraise(&mut self, event: AffectEvent) -> PadVector; fn query(&self) -> AffectState; fn modulate(&self, params: &mut DispatchParams); fn persist(&self, path: &Path) -> Result<()>; }
pub enum DispatchStrategy { Conservative, Balanced, Exploratory, Escalating, Proactive }
pub struct DispatchParams { model: String, turn_limit: u32, strategy: DispatchStrategy, effort: String }

// roko-golem/src/daimon.rs (to be consolidated into roko-daimon)
pub enum AffectOctant { Excited, Surprised, Confident, Relaxed, Angry, Anxious, Bored, Depressed }
pub struct AffectBehaviorModulation { strategy: AffectBehaviorStrategy, exploration_rate: f64, prefer_proven_playbooks: bool, model_tier_escalation: u8, extra_retries: u32, trigger_dream_cycles: bool, run_maintenance_tasks: bool }

// Specified but not yet implemented
pub struct SomaticLandscape { tree: KdTree<f64, SomaticMarker, 8> }
pub struct SomaticMarker { strategy_coords: [f64; 8], valence: f64, intensity: f64, episodes: Vec<ContentHash> }
pub struct EmotionalTag { pad: PadVector, emotion: String, intensity: f32, trigger: String, mood_snapshot: PadVector }
```

---

## Cross-topic Dependencies

| Dependency | Direction | What Flows |
|---|---|---|
| [05-learning](../05-learning/INDEX.md) | Daimon → Learning | Behavioral state modulates CascadeRouter thresholds |
| [03-dreams](../10-dreams/INDEX.md) | Bidirectional | Emotional load → dream urgency; REM → depotentiation; dream outcomes → appraisal |
| [04-knowledge](../06-neuro/INDEX.md) | Daimon → Knowledge | EmotionalTag on Engrams; mood-congruent retrieval; somatic field via mesh |
| [04-verification](../04-verification/INDEX.md) | Verification → Daimon | Gate pass/fail triggers appraisal; rung level scales emotional impact |
| [02-runtime](../01-orchestration/INDEX.md) | Daimon → Runtime | Conversational tone mapping; event emission; TUI/Spectre visualization |
