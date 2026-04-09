//! Episode distillation into durable knowledge candidates.
//!
//! The distiller batches stored episodes, asks a small model to extract
//! reusable facts, procedures, heuristics, and constraints, then
//! normalizes the structured response into [`KnowledgeEntry`] values.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use anyhow::{Context as AnyhowContext, Result, anyhow};
use async_trait::async_trait;
use chrono::Utc;
use roko_agent::Agent;
use roko_agent::claude_agent::ClaudeAgent;
use roko_agent::nl_to_format::NlToFormatConverter;
use roko_core::{Body, Context as RokoContext, Kind, Provenance, Signal};
use roko_learn::episode_logger::Episode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{KnowledgeEntry, KnowledgeKind};

const DEFAULT_MODEL: &str = "claude-haiku-3-5";
const DEFAULT_MAX_TOKENS: u32 = 2_048;
const DEFAULT_CONFIDENCE: f64 = 0.75;

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
        let signal = Signal::builder(Kind::Prompt)
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
        let fallback_source = match episodes {
            [episode] => Some(episode_source_id(episode).to_string()),
            _ => None,
        };

        self.entries
            .into_iter()
            .filter_map(|candidate| candidate.into_entry(fallback_source.as_deref()))
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
    half_life_days: Option<f64>,
}

impl DistillationCandidate {
    fn into_entry(mut self, fallback_source: Option<&str>) -> Option<KnowledgeEntry> {
        let content = self.content.trim();
        if content.is_empty() {
            return None;
        }

        if self.source_episodes.is_empty()
            && let Some(source) = fallback_source
        {
            self.source_episodes.push(source.to_string());
        }

        self.source_episodes.sort();
        self.source_episodes.dedup();

        let kind_tag = knowledge_kind_tag(self.kind);
        if !self.tags.iter().any(|tag| tag == kind_tag) {
            self.tags.push(kind_tag.to_string());
        }
        self.tags.sort();
        self.tags.dedup();

        let confidence = self.confidence.clamp(0.0, 1.0);
        let half_life_days = self
            .half_life_days
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or_else(|| self.kind.default_half_life_days());

        Some(KnowledgeEntry {
            id: derive_knowledge_id(self.kind, content, &self.source_episodes, &self.tags),
            kind: self.kind,
            source: None,
            content: content.to_string(),
            confidence,
            source_episodes: self.source_episodes,
            tags: self.tags,
            created_at: Utc::now(),
            half_life_days,
            hdc_vector: None,
        })
    }
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
         Target categories:\n\
         - fact: declarative observations such as file structure, function arity, or stable project facts\n\
         - procedure: repeatable steps that fixed a problem or completed a task\n\
         - heuristic: an empirical rule inferred from repeated episode patterns\n\
         - constraint: a hard rule inferred from repeated failures or explicit guardrails\n\
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
         Facts should be direct observations.\n\
         Procedures should describe a fix or recipe.\n\
         Heuristics should only appear when the episodes show a recurring pattern.\n\
         Constraints should only appear when the episodes show repeated failures or guardrails.\n\n\
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
    match kind {
        KnowledgeKind::Fact => "fact",
        KnowledgeKind::Insight => "insight",
        KnowledgeKind::Procedure => "procedure",
        KnowledgeKind::Heuristic => "heuristic",
        KnowledgeKind::Playbook => "playbook",
        KnowledgeKind::Constraint => "constraint",
        KnowledgeKind::AntiKnowledge => "anti_knowledge",
    }
}

fn episode_source_id(episode: &Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn default_candidate_confidence() -> f64 {
    DEFAULT_CONFIDENCE
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
            r#"<|json|>{"entries":[{"kind":"fact","content":"file src/lib.rs contains struct Widget","confidence":0.9,"source_episodes":["ep-a"],"tags":["rust","struct"],"half_life_days":45},{"kind":"constraint","content":"never modify file X without also updating Y","confidence":0.8,"source_episodes":["ep-b"],"tags":["guardrail"],"half_life_days":60}]}<|/json|>"#,
        );
        let distiller = Distiller::with_backend(backend.clone());
        let episodes = vec![
            episode("signal-a", "ep-a", true),
            episode("signal-b", "ep-b", false),
        ];

        let entries = distiller.distill(&episodes).await.expect("distill");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, KnowledgeKind::Fact);
        assert_eq!(entries[0].source_episodes, vec!["ep-a"]);
        assert!(entries[0].tags.iter().any(|tag| tag == "fact"));
        assert_eq!(entries[1].kind, KnowledgeKind::Constraint);
        assert_eq!(entries[1].source_episodes, vec!["ep-b"]);
        assert!(entries[1].id.starts_with("kn_"));

        let prompt = backend.prompt().expect("prompt recorded");
        assert!(prompt.contains("Episode corpus"));
        assert!(prompt.contains("ep-a"));
        assert!(prompt.contains("ep-b"));
        assert!(prompt.contains("Target categories"));
    }

    #[test]
    fn system_prompt_requests_crane_delimiters() {
        let prompt = distillation_system_prompt();
        assert!(prompt.contains("<|json|>"));
        assert!(prompt.contains("entries"));
    }
}
