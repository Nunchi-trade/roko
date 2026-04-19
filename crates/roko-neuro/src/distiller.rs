//! Episode distillation into durable knowledge candidates.
//!
//! The distiller batches stored episodes, asks a small model to extract
//! reusable insights, heuristics, warnings, causal links, and strategy
//! fragments, then
//! normalizes the structured response into [`KnowledgeEntry`] values.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use anyhow::{Context as AnyhowContext, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use roko_agent::Agent;
use roko_agent::claude_agent::ClaudeAgent;
use roko_agent::nl_to_format::NlToFormatConverter;
use roko_core::{Body, Context as RokoContext, EmotionalTag, Engram, Kind, PadVector, Provenance};
use roko_learn::episode_logger::Episode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EmotionalProvenance, KnowledgeEntry, KnowledgeKind, ValidationArc};

const DEFAULT_MODEL: &str = "claude-haiku-3-5";
const DEFAULT_MAX_TOKENS: u32 = 2_048;
const DEFAULT_CONFIDENCE: f64 = 0.75;
const DISTILLER_SOURCE: &str = "distiller";
const MAX_DISTILLED_TAGS: usize = 12;
const MAX_DISTILLED_SOURCE_EPISODES: usize = 32;
const MIN_MULTI_EPISODE_SUPPORT: usize = 2;

/// Backend contract for episode distillation.
#[async_trait]
pub trait DistillationBackend: Send + Sync + std::fmt::Debug {
    /// Run the backend on a fully rendered distillation prompt and
    /// return the model's raw text response.
    async fn complete(&self, prompt: &str) -> Result<String>;

    /// Human-readable model identifier used by the backend.
    fn model(&self) -> &str;
}

/// Distills episodes into [`KnowledgeEntry`] candidates.
#[derive(Debug, Clone)]
pub struct Distiller {
    backend: Arc<dyn DistillationBackend>,
}

impl Distiller {
    /// Construct a distiller backed by Anthropic Claude Haiku.
    ///
    /// The backend uses a small model by default so knowledge extraction
    /// remains cheaper than a premium reasoning model.
    #[must_use]
    pub fn with_claude(api_key: impl Into<String>) -> Self {
        Self::with_claude_model(api_key, DEFAULT_MODEL)
    }

    /// Construct a distiller backed by Anthropic Claude using an
    /// explicit model slug.
    #[must_use]
    pub fn with_claude_model(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_backend(Arc::new(ClaudeDistillationBackend::new(api_key, model)))
    }

    /// Construct a distiller from an arbitrary backend.
    #[must_use]
    pub fn with_backend(backend: Arc<dyn DistillationBackend>) -> Self {
        Self { backend }
    }

    /// Return the configured model name.
    #[must_use]
    pub fn model(&self) -> &str {
        self.backend.model()
    }

    /// Distill a batch of episodes into knowledge candidates.
    ///
    /// # Errors
    ///
    /// Returns an error if prompt construction fails, the backend fails,
    /// or the model response cannot be parsed as structured output.
    pub async fn distill(&self, episodes: &[Episode]) -> Result<Vec<KnowledgeEntry>> {
        if episodes.is_empty() {
            return Ok(Vec::new());
        }

        let prompt = build_prompt(episodes)?;
        let response = self
            .backend
            .complete(&prompt)
            .await
            .with_context(|| format!("distillation backend {} failed", self.model()))?;
        let envelope = parse_distillation_response(&response)?;
        Ok(envelope.into_entries(episodes))
    }
}

#[derive(Debug)]
struct ClaudeDistillationBackend {
    agent: ClaudeAgent,
    model: String,
}

impl ClaudeDistillationBackend {
    fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        let model = model.into();
        let agent = ClaudeAgent::new(api_key, &model)
            .with_name(format!("roko-neuro:distiller:{model}"))
            .with_max_tokens(DEFAULT_MAX_TOKENS)
            .with_system_prompt(distillation_system_prompt());
        Self { agent, model }
    }
}

#[async_trait]
impl DistillationBackend for ClaudeDistillationBackend {
    async fn complete(&self, prompt: &str) -> Result<String> {
        let signal = Engram::builder(Kind::Prompt)
            .body(Body::text(prompt))
            .provenance(Provenance::agent("roko-neuro:distiller"))
            .build();
        let result = self.agent.run(&signal, &RokoContext::now()).await;
        if !result.success {
            let reason = result
                .output
                .body
                .as_text()
                .map(str::to_owned)
                .unwrap_or_else(|_| "distillation model returned a non-text failure".to_string());
            return Err(anyhow!(reason));
        }

        result
            .output
            .body
            .as_text()
            .map(str::to_owned)
            .map_err(|err| anyhow!("distillation response was not text: {err}"))
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[derive(Debug, Deserialize)]
struct DistillationEnvelope {
    #[serde(default, alias = "knowledge", alias = "candidates", alias = "items")]
    entries: Vec<DistillationCandidate>,
}

impl DistillationEnvelope {
    fn into_entries(self, episodes: &[Episode]) -> Vec<KnowledgeEntry> {
        let fallback_source = batch_source_episodes(episodes);
        let valid_source_ids = fallback_source
            .as_ref()
            .map(|ids| ids.iter().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let episode_models = episode_models_by_source(episodes);
        let episode_affect = episode_affect_by_source(episodes);

        self.entries
            .into_iter()
            .filter_map(|candidate| {
                candidate.into_entry(
                    fallback_source.as_deref(),
                    &valid_source_ids,
                    &episode_models,
                    &episode_affect,
                )
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
struct DistillationCandidate {
    #[serde(default)]
    kind: KnowledgeKind,
    #[serde(default)]
    content: String,
    #[serde(default = "default_candidate_confidence")]
    confidence: f64,
    #[serde(default, alias = "episode_ids", alias = "source_episode_ids")]
    source_episodes: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    source_model: Option<String>,
    #[serde(default = "default_candidate_model_generality")]
    model_generality: f64,
    #[serde(default)]
    half_life_days: Option<f64>,
}

impl DistillationCandidate {
    fn into_entry(
        mut self,
        fallback_source: Option<&[String]>,
        valid_source_ids: &BTreeSet<String>,
        episode_models: &BTreeMap<String, String>,
        episode_affect: &BTreeMap<String, EpisodeAffectSnapshot>,
    ) -> Option<KnowledgeEntry> {
        let content = self.content.trim();
        if content.is_empty() {
            return None;
        }

        let source_episodes_were_explicit = !self.source_episodes.is_empty();
        if self.source_episodes.is_empty()
            && let Some(source) = fallback_source
        {
            self.source_episodes.extend(source.iter().cloned());
        }

        self.source_episodes = sanitize_source_episodes(self.source_episodes, valid_source_ids);
        if source_episodes_were_explicit && self.source_episodes.is_empty() {
            return None;
        }
        self.tags = sanitize_tags(self.tags);

        if requires_multi_episode_support(self.kind)
            && self.source_episodes.len() < MIN_MULTI_EPISODE_SUPPORT
        {
            return None;
        }

        let kind_tag = knowledge_kind_tag(self.kind);
        if !self.tags.iter().any(|tag| tag == kind_tag) {
            self.tags.push(kind_tag.to_string());
        }
        self.tags = sanitize_tags(self.tags);

        let confidence = self.confidence.clamp(0.0, 1.0);
        let (source_model, model_generality) = inferred_model_scope(
            self.kind,
            self.source_model.take(),
            self.model_generality,
            &self.source_episodes,
            &self.tags,
            episode_models,
        );
        let half_life_days = self
            .half_life_days
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(|| self.kind.default_half_life_days());
        let (emotional_tag, emotional_provenance) =
            aggregate_emotional_metadata(&self.source_episodes, episode_affect, self.kind);

        Some(KnowledgeEntry {
            id: derive_knowledge_id(self.kind, content, &self.source_episodes, &self.tags),
            kind: self.kind,
            source: Some(DISTILLER_SOURCE.to_string()),
            content: content.to_string(),
            confidence,
            confidence_weight: confidence,
            refuted_insight_id: None,
            refutation_evidence: None,
            source_episodes: self.source_episodes,
            tags: self.tags,
            source_model,
            model_generality,
            created_at: Utc::now(),
            half_life_days,
            tier: Default::default(),
            emotional_tag,
            emotional_provenance,
            hdc_vector: None,
            confirmation_count: 0,
            distinct_contexts: Vec::new(),
            deprecated: false,
        })
    }
}

fn sanitize_source_episodes(
    source_episodes: Vec<String>,
    valid_source_ids: &BTreeSet<String>,
) -> Vec<String> {
    let mut sanitized = source_episodes
        .into_iter()
        .map(|source| source.trim().to_string())
        .filter(|source| !source.is_empty())
        .filter(|source| valid_source_ids.is_empty() || valid_source_ids.contains(source))
        .collect::<Vec<_>>();
    sanitized.sort();
    sanitized.dedup();
    sanitized.truncate(MAX_DISTILLED_SOURCE_EPISODES);
    sanitized
}

const fn requires_multi_episode_support(kind: KnowledgeKind) -> bool {
    matches!(
        kind,
        KnowledgeKind::Heuristic | KnowledgeKind::StrategyFragment
    )
}

fn sanitize_tags(tags: Vec<String>) -> Vec<String> {
    let mut sanitized = tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    sanitized.sort();
    sanitized.dedup();
    sanitized.truncate(MAX_DISTILLED_TAGS);
    sanitized
}

#[derive(Debug, Clone)]
struct EpisodeAffectSnapshot {
    tag: EmotionalTag,
    success: bool,
    completed_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct EpisodePromptRecord {
    source_id: String,
    id: String,
    episode_id: String,
    kind: String,
    agent_id: String,
    task_id: String,
    input_signal_hash: String,
    output_signal_hash: String,
    model: String,
    trigger_kind: String,
    success: bool,
    turns: u64,
    tokens_used: u64,
    duration_secs: f64,
    failure_reason: Option<String>,
    emotional_tag: Option<roko_core::EmotionalTag>,
    gate_verdicts: Vec<roko_learn::episode_logger::GateVerdict>,
    usage: roko_learn::episode_logger::Usage,
    external_actions: Vec<Value>,
    headline: bool,
    extra: Value,
    timestamp: chrono::DateTime<Utc>,
    started_at: chrono::DateTime<Utc>,
    completed_at: chrono::DateTime<Utc>,
}

impl EpisodePromptRecord {
    fn from_episode(episode: &Episode) -> Self {
        let source_id = episode_source_id(episode).to_string();
        Self {
            source_id,
            id: episode.id.clone(),
            episode_id: episode.episode_id.clone(),
            kind: episode.kind.clone(),
            agent_id: episode.agent_id.clone(),
            task_id: episode.task_id.clone(),
            input_signal_hash: episode.input_signal_hash.clone(),
            output_signal_hash: episode.output_signal_hash.clone(),
            model: episode.model.clone(),
            trigger_kind: episode.trigger_kind.clone(),
            success: episode.success,
            turns: episode.turns,
            tokens_used: episode.tokens_used,
            duration_secs: episode.duration_secs,
            failure_reason: episode.failure_reason.clone(),
            emotional_tag: episode.emotional_tag.clone(),
            gate_verdicts: episode.gate_verdicts.clone(),
            usage: episode.usage.clone(),
            external_actions: episode.external_actions.clone(),
            headline: episode.headline,
            extra: json!(&episode.extra),
            timestamp: episode.timestamp.clone(),
            started_at: episode.started_at.clone(),
            completed_at: episode.completed_at.clone(),
        }
    }
}

fn build_prompt(episodes: &[Episode]) -> Result<String> {
    let corpus: Vec<EpisodePromptRecord> = episodes
        .iter()
        .map(EpisodePromptRecord::from_episode)
        .collect();
    let corpus_json = serde_json::to_string_pretty(&corpus)?;
    Ok(format!(
        "Episode corpus:\n```json\n{corpus_json}\n```\n\n\
         Extract reusable knowledge from the corpus.\n\
         Return only structured JSON that matches the schema in the system prompt.\n\
         Be conservative: emit entries only when the episodes support them.\n\
         Treat emotional tags as supporting provenance: high-arousal repeated episodes deserve more weight, but do not infer durable knowledge from mood alone.\n\
         Target categories:\n\
         - insight: declarative observations such as file structure, function arity, or stable project facts\n\
         - heuristic: an empirical rule inferred from repeated episode patterns\n\
         - warning: a recurring failure mode, guardrail, or risk to avoid\n\
         - causal_link: a cause-and-effect observation grounded in the episodes\n\
         - strategy_fragment: a reusable approach or recipe that solved a task\n\
         Prefer concise content strings with concrete wording.\n"
    ))
}

fn distillation_system_prompt() -> String {
    let schema = distillation_schema();
    let extractor = NlToFormatConverter::new();
    format!(
        "You are Roko's knowledge distiller.\n\
         Read the episode corpus and synthesize durable knowledge.\n\
         Merge duplicate ideas across episodes.\n\
         Insights should be direct observations.\n\
         Heuristics should only appear when the episodes show a recurring pattern.\n\
         Warnings should capture recurring failures, guardrails, or things to avoid.\n\
         Causal links should state what caused what when the evidence is strong enough.\n\
         Strategy fragments should describe a reusable fix, recipe, or approach.\n\
         For heuristics and strategy fragments, set source_model plus a low model_generality only when the guidance depends on one model's behavior, formatting, or tool syntax.\n\
         Use model_generality near 1.0 for heuristics that work across models and omit source_model for those general rules.\n\
         Avoid legacy category names like fact, procedure, playbook, or constraint in the output.\n\n\
         {}\n",
        extractor.extraction_prompt(&schema)
    )
}

fn distillation_schema() -> Value {
    json!({
        "type": "object",
        "required": ["entries"],
        "properties": {
            "entries": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string" },
                        "content": { "type": "string" },
                        "confidence": { "type": "number" },
                        "source_episodes": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "tags": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "source_model": { "type": ["string", "null"] },
                        "model_generality": { "type": "number" },
                        "half_life_days": { "type": "number" }
                    }
                }
            }
        }
    })
}

fn parse_distillation_response(response: &str) -> Result<DistillationEnvelope> {
    let extractor = NlToFormatConverter::new();
    let extracted = extractor
        .convert(response, &distillation_schema())
        .context("extract distillation JSON from model response")?;
    serde_json::from_value(extracted).context("decode distillation JSON envelope")
}

fn derive_knowledge_id(
    kind: KnowledgeKind,
    content: &str,
    source_episodes: &[String],
    tags: &[String],
) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    knowledge_kind_tag(kind).hash(&mut hasher);
    content.hash(&mut hasher);
    for source in source_episodes {
        source.hash(&mut hasher);
    }
    for tag in tags {
        tag.hash(&mut hasher);
    }
    format!("kn_{:016x}", hasher.finish())
}

fn knowledge_kind_tag(kind: KnowledgeKind) -> &'static str {
    kind.as_str()
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn batch_source_episodes(episodes: &[Episode]) -> Option<Vec<String>> {
    let mut source_episodes: Vec<String> = episodes
        .iter()
        .map(episode_source_id)
        .filter(|source| !source.trim().is_empty())
        .map(ToOwned::to_owned)
        .collect();
    source_episodes.sort();
    source_episodes.dedup();

    if source_episodes.is_empty() {
        None
    } else {
        Some(source_episodes)
    }
}

fn episode_models_by_source(episodes: &[Episode]) -> BTreeMap<String, String> {
    let mut models = BTreeMap::new();
    for episode in episodes {
        let model = episode.model.trim();
        if model.is_empty() {
            continue;
        }
        models.insert(episode_source_id(episode).to_string(), model.to_string());
    }
    models
}

fn episode_affect_by_source(episodes: &[Episode]) -> BTreeMap<String, EpisodeAffectSnapshot> {
    let mut affect = BTreeMap::new();
    for episode in episodes {
        let Some(tag) = episode.emotional_tag.clone() else {
            continue;
        };
        affect.insert(
            episode_source_id(episode).to_string(),
            EpisodeAffectSnapshot {
                tag,
                success: episode.success,
                completed_at: episode.completed_at,
            },
        );
    }
    affect
}

fn aggregate_emotional_metadata(
    source_episodes: &[String],
    episode_affect: &BTreeMap<String, EpisodeAffectSnapshot>,
    kind: KnowledgeKind,
) -> (Option<EmotionalTag>, Option<EmotionalProvenance>) {
    let mut snapshots = Vec::new();
    let mut pads = Vec::new();
    let mut moods = Vec::new();
    let mut intensity_sum = 0.0_f64;

    for source in source_episodes {
        let Some(snapshot) = episode_affect.get(source) else {
            continue;
        };
        pads.push(snapshot.tag.pad);
        moods.push(snapshot.tag.mood_snapshot);
        intensity_sum += f64::from(snapshot.tag.intensity).clamp(0.0, 1.0);
        snapshots.push(snapshot.clone());
    }

    if pads.is_empty() {
        return (None, None);
    }

    let average_pad = average_pad_vector(&pads);
    let average_mood = average_pad_vector(&moods);
    let mean_intensity = (intensity_sum / pads.len() as f64).clamp(0.0, 1.0) as f32;
    snapshots.sort_by_key(|snapshot| snapshot.completed_at);

    let emotional_tag = EmotionalTag::new(
        average_pad,
        mean_intensity,
        format!("distilled:{}", kind.as_str()),
        average_mood,
    );
    let emotional_provenance = EmotionalProvenance {
        average_pad,
        discovery_emotion: snapshots
            .first()
            .map(|snapshot| EmotionalProvenance::coarse_emotion_label(snapshot.tag.pad))
            .unwrap_or_else(|| "neutral_mid_arousal".to_string()),
        validation_arc: infer_validation_arc(&snapshots),
        emotional_diversity: emotional_diversity(&snapshots),
    };

    (Some(emotional_tag), Some(emotional_provenance))
}

fn average_pad_vector(vectors: &[PadVector]) -> PadVector {
    if vectors.is_empty() {
        return PadVector::neutral();
    }

    let len = vectors.len() as f64;
    let pleasure = vectors.iter().map(|pad| pad.pleasure).sum::<f64>() / len;
    let arousal = vectors.iter().map(|pad| pad.arousal).sum::<f64>() / len;
    let dominance = vectors.iter().map(|pad| pad.dominance).sum::<f64>() / len;
    PadVector::new(pleasure, arousal, dominance).clamped()
}

fn emotional_diversity(snapshots: &[EpisodeAffectSnapshot]) -> f64 {
    if snapshots.is_empty() {
        return 0.0;
    }

    let mut counts: HashMap<String, u32> = HashMap::new();
    for snapshot in snapshots {
        *counts
            .entry(EmotionalProvenance::coarse_emotion_label(snapshot.tag.pad))
            .or_insert(0) += 1;
    }

    let total = counts.values().copied().sum::<u32>() as f64;
    if total <= 0.0 {
        return 0.0;
    }

    let mut entropy = 0.0_f64;
    for count in counts.values().copied() {
        let p = count as f64 / total;
        if p > 0.0 {
            entropy -= p * p.log2();
        }
    }

    let max_entropy = (counts.len() as f64).log2();
    if max_entropy > 0.0 {
        (entropy / max_entropy).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn infer_validation_arc(snapshots: &[EpisodeAffectSnapshot]) -> Option<ValidationArc> {
    if snapshots.len() < 2 {
        return None;
    }

    let first = episode_sentiment(snapshots.first()?);
    let last = episode_sentiment(snapshots.last()?);
    let all_same_outcome = snapshots
        .windows(2)
        .all(|pair| pair[0].success == pair[1].success);

    if first <= -0.15 && last >= 0.15 {
        return Some(ValidationArc::Redemptive);
    }
    if first >= 0.15 && last <= -0.15 {
        return Some(ValidationArc::Contaminating);
    }
    if last > first + 0.20 {
        return Some(ValidationArc::Progressive);
    }
    if all_same_outcome {
        return Some(ValidationArc::Stable);
    }

    Some(ValidationArc::Stable)
}

fn episode_sentiment(snapshot: &EpisodeAffectSnapshot) -> f64 {
    let outcome_bias = if snapshot.success { 0.20 } else { -0.20 };
    (snapshot.tag.pad.pleasure + outcome_bias).clamp(-1.0, 1.0)
}

fn inferred_model_scope(
    kind: KnowledgeKind,
    source_model: Option<String>,
    model_generality: f64,
    source_episodes: &[String],
    tags: &[String],
    episode_models: &BTreeMap<String, String>,
) -> (Option<String>, f64) {
    if kind != KnowledgeKind::Heuristic && kind != KnowledgeKind::StrategyFragment {
        return (None, 1.0);
    }

    let explicit_model = source_model
        .and_then(normalize_model_slug)
        .or_else(|| tagged_model(tags, "target-model:"))
        .or_else(|| tagged_model(tags, "source-model:"));
    let explicit_generality = sanitize_model_generality(model_generality);

    if explicit_model.is_some() || explicit_generality <= 0.7 {
        return (explicit_model, explicit_generality);
    }

    let models: BTreeSet<String> = source_episodes
        .iter()
        .filter_map(|episode_id| episode_models.get(episode_id))
        .filter_map(|model| normalize_model_slug(model.to_string()))
        .collect();

    if models.len() == 1 && explicit_generality < 1.0 {
        return (models.into_iter().next(), explicit_generality);
    }

    (None, explicit_generality)
}

fn tagged_model(tags: &[String], prefix: &str) -> Option<String> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn normalize_model_slug(model: String) -> Option<String> {
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

fn sanitize_model_generality(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        default_candidate_model_generality()
    }
}

fn default_candidate_confidence() -> f64 {
    DEFAULT_CONFIDENCE
}

fn default_candidate_model_generality() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockBackend {
        response: String,
        prompt: Mutex<Option<String>>,
        model: String,
    }

    impl MockBackend {
        fn new(response: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                response: response.into(),
                prompt: Mutex::new(None),
                model: "mock-haiku".to_string(),
            })
        }

        fn prompt(&self) -> Option<String> {
            self.prompt.lock().ok().and_then(|guard| guard.clone())
        }
    }

    #[async_trait]
    impl DistillationBackend for MockBackend {
        async fn complete(&self, prompt: &str) -> Result<String> {
            if let Ok(mut guard) = self.prompt.lock() {
                *guard = Some(prompt.to_owned());
            }
            Ok(self.response.clone())
        }

        fn model(&self) -> &str {
            &self.model
        }
    }

    fn episode(id: &str, episode_id: &str, success: bool) -> Episode {
        let mut episode = Episode::new("agent-a", "task-a");
        episode.id = id.to_string();
        episode.episode_id = episode_id.to_string();
        episode.kind = "agent_turn".to_string();
        episode.model = "claude-sonnet-4-5".to_string();
        episode.success = success;
        episode
    }

    #[tokio::test]
    async fn distiller_maps_structured_response_into_entries() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"insight","content":"file src/lib.rs contains struct Widget","confidence":0.9,"source_episodes":["ep-a"],"tags":["rust","struct"],"half_life_days":45},{"kind":"warning","content":"never modify file X without also updating Y","confidence":0.8,"source_episodes":["ep-b"],"tags":["guardrail"],"half_life_days":60}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend.clone());
        let episodes = vec![
            episode("signal-a", "ep-a", true),
            episode("signal-b", "ep-b", false),
        ];

        let entries = distiller.distill(&episodes).await.expect("distill");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, KnowledgeKind::Insight);
        assert_eq!(entries[0].source_episodes, vec!["ep-a"]);
        assert!(entries[0].tags.iter().any(|tag| tag == "insight"));
        assert_eq!(entries[1].kind, KnowledgeKind::Warning);
        assert_eq!(entries[1].source_episodes, vec!["ep-b"]);
        assert!(entries[1].id.starts_with("kn_"));

        let prompt = backend.prompt().expect("prompt recorded");
        assert!(prompt.contains("Episode corpus"));
        assert!(prompt.contains("ep-a"));
        assert!(prompt.contains("ep-b"));
        assert!(prompt.contains("Target categories"));
    }

    #[tokio::test]
    async fn distiller_falls_back_to_batch_confirmation_chain() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"insight","content":"shared insight","confidence":0.9,"tags":["shared"]}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let episodes = vec![
            episode("signal-a", "ep-a", true),
            episode("signal-b", "ep-b", true),
        ];

        let entries = distiller.distill(&episodes).await.expect("distill");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_episodes, vec!["ep-a", "ep-b"]);
    }

    #[tokio::test]
    async fn model_specific_heuristics_preserve_model_metadata() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"heuristic","content":"Use XML tool tags for tool calls.","confidence":0.82,"source_episodes":["ep-a","ep-b"],"source_model":"claude-sonnet-4-5","model_generality":0.2}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let episodes = vec![
            episode("signal-a", "ep-a", true),
            episode("signal-b", "ep-b", true),
        ];

        let entries = distiller.distill(&episodes).await.expect("distill");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, KnowledgeKind::Heuristic);
        assert_eq!(
            entries[0].source_model.as_deref(),
            Some("claude-sonnet-4-5")
        );
        assert!((entries[0].model_generality - 0.2).abs() < f64::EPSILON);
        assert!(entries[0].applies_to_model("claude-sonnet-4-5"));
        assert!(!entries[0].applies_to_model("gpt-5.4"));
    }

    #[tokio::test]
    async fn distiller_bounds_tags_and_marks_entry_source() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"insight","content":"Normalize noisy tags.","confidence":0.9,"source_episodes":[" ep-a ","","ep-a","ep-b"],"tags":[" one ","two","two","three","four","five","six","seven","eight","nine","ten","eleven","twelve","thirteen"]}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let entries = distiller
            .distill(&[
                episode("signal-a", "ep-a", true),
                episode("signal-b", "ep-b", true),
            ])
            .await
            .expect("distill");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source.as_deref(), Some(DISTILLER_SOURCE));
        assert_eq!(entries[0].source_episodes, vec!["ep-a", "ep-b"]);
        assert!(entries[0].tags.iter().any(|tag| tag == "insight"));
        assert!(entries[0].tags.len() <= MAX_DISTILLED_TAGS);
    }

    #[tokio::test]
    async fn single_episode_distillation_does_not_emit_heuristics_or_strategy_fragments() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"heuristic","content":"One run is enough.","confidence":0.8,"source_episodes":["ep-a"]},{"kind":"strategy_fragment","content":"Always do this from one attempt.","confidence":0.9,"source_episodes":["ep-a"]},{"kind":"insight","content":"One concrete observation.","confidence":0.7,"source_episodes":["ep-a"]}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let entries = distiller
            .distill(&[episode("signal-a", "ep-a", true)])
            .await
            .expect("distill");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, KnowledgeKind::Insight);
    }

    #[tokio::test]
    async fn distiller_discards_unknown_source_episode_ids() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"insight","content":"Unknown provenance should drop.","confidence":0.8,"source_episodes":["not-in-batch"]}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let entries = distiller
            .distill(&[episode("signal-a", "ep-a", true)])
            .await
            .expect("distill");

        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn distiller_transfers_emotional_provenance_from_supporting_episodes() {
        let backend = MockBackend::new(
            r#"<|json|>{"entries":[{"kind":"warning","content":"Do not retry the rollout until rollback health is confirmed.","confidence":0.84,"source_episodes":["ep-a","ep-b"],"tags":["deploy","rollback"]}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend);
        let mut first = episode("signal-a", "ep-a", false);
        first.completed_at = Utc::now() - chrono::Duration::minutes(10);
        first.emotional_tag = Some(EmotionalTag::new(
            PadVector::new(-0.8, 0.7, -0.1),
            0.9,
            "rollout_failure",
            PadVector::new(-0.7, 0.6, -0.1),
        ));
        let mut second = episode("signal-b", "ep-b", false);
        second.completed_at = Utc::now();
        second.emotional_tag = Some(EmotionalTag::new(
            PadVector::new(0.7, 0.2, 0.4),
            0.7,
            "rollback_recovered",
            PadVector::new(0.6, 0.1, 0.3),
        ));
        second.success = true;

        let entries = distiller.distill(&[first, second]).await.expect("distill");
        assert_eq!(entries.len(), 1);
        let tag = entries[0]
            .emotional_tag
            .as_ref()
            .expect("emotional provenance");
        assert_eq!(tag.trigger, "distilled:warning");
        assert!((f64::from(tag.intensity) - 0.8).abs() < 0.001);
        assert!((tag.pad.pleasure + 0.05).abs() < 0.001);
        assert!(tag.pad.arousal > 0.44);
        assert!((tag.mood_snapshot.pleasure + 0.05).abs() < 0.001);
        assert!(tag.mood_snapshot.arousal > 0.34);

        let provenance = entries[0]
            .emotional_provenance
            .as_ref()
            .expect("emotional provenance metadata");
        assert_eq!(provenance.discovery_emotion, "negative_high_arousal");
        assert_eq!(provenance.validation_arc, Some(ValidationArc::Redemptive));
        assert!((provenance.emotional_diversity - 1.0).abs() < 0.001);
        assert!((provenance.average_pad.pleasure + 0.05).abs() < 0.001);
    }

    #[test]
    fn system_prompt_requests_crane_delimiters() {
        let prompt = distillation_system_prompt();
        assert!(prompt.contains("<|json|>"));
        assert!(prompt.contains("entries"));
    }
}
