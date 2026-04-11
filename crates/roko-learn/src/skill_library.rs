//! Skill library — a registry of reusable capabilities the agent can invoke.
//!
//! A [`Skill`] captures a named capability with a prompt template, the tools
//! it depends on, and example input/output pairs. Skills accumulate
//! lightweight usage telemetry (`usage_count`, `success_rate`) each time they
//! are invoked so the library can surface the most reliable patterns to
//! future prompts.
//!
//! The [`SkillLibrary`] is an in-memory, JSON-file-backed registry. Writes
//! to the in-memory map are guarded by a [`parking_lot::RwLock`]; persistence
//! uses [`tokio::fs`] with a tempfile+rename to keep the on-disk store
//! consistent under concurrent writers.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), roko_learn::skill_library::SkillLibraryError> {
//! use roko_learn::skill_library::{Skill, SkillLibrary};
//!
//! let library = SkillLibrary::new("/tmp/skills.json").await?;
//! let skill = Skill::new(
//!     "summarize_diff",
//!     "Summarize a git diff into a short changelog entry.",
//!     "You are given a diff. Produce a 1-2 sentence changelog entry.",
//! );
//! library.register(&skill).await?;
//! library.record_use("summarize_diff", true).await?;
//! # Ok(()) }
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use roko_agent::{Agent, nl_to_format::NlToFormatConverter};
use roko_core::{Body, Context, Kind, Signal};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::sync::Mutex as AsyncMutex;

/// Errors produced by [`SkillLibrary`].
#[derive(Debug, Error)]
pub enum SkillLibraryError {
    /// A skill with the requested name already exists in the library.
    #[error("skill '{0}' is already registered")]
    Duplicate(String),
    /// No skill with the requested name exists.
    #[error("skill '{0}' is not registered")]
    NotFound(String),
    /// I/O error while reading or writing the persistence file.
    #[error("skill library I/O error: {0}")]
    Io(#[from] io::Error),
    /// JSON (de)serialization error.
    #[error("skill library serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// A reusable capability the agent can invoke.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    /// Stable identifier for the skill record.
    #[serde(default)]
    pub id: String,
    /// Human-readable name for the skill.
    /// Unique, human-readable identifier for the skill (`snake_case` preferred).
    pub name: String,
    /// One-line description of what the skill does.
    pub summary: String,
    /// Prompt template injected when the skill is selected.
    pub prompt_template: String,
    /// When to apply this skill.
    #[serde(default)]
    pub precondition: String,
    /// Procedure or tool-call sequence summary.
    #[serde(default)]
    pub procedure: String,
    /// Expected result after following the procedure.
    #[serde(default)]
    pub postcondition: String,
    /// Confidence in `[0.0, 1.0]`, derived from validation outcomes.
    #[serde(default)]
    pub confidence: f64,
    /// Episode IDs this skill was extracted from.
    #[serde(default)]
    pub source_episodes: Vec<String>,
    /// Number of successful validations.
    #[serde(default)]
    pub validations: u64,
    /// Number of failed applications.
    #[serde(default)]
    pub failures: u64,
    /// Task categories where the skill applies.
    #[serde(default)]
    pub task_categories: Vec<String>,
    /// RFC3339 timestamp when the skill was created.
    #[serde(default)]
    pub created_at: String,
    /// RFC3339 timestamp of the last validation outcome.
    #[serde(default)]
    pub last_validated_at: Option<String>,
    /// Names of tools this skill expects the caller to expose.
    #[serde(default)]
    pub required_tools: Vec<String>,
    /// Illustrative inputs the skill was designed for.
    #[serde(default)]
    pub example_inputs: Vec<String>,
    /// Illustrative outputs corresponding to `example_inputs`.
    #[serde(default)]
    pub example_outputs: Vec<String>,
    /// Free-form tags used by [`SkillLibrary::search_by_tag`].
    #[serde(default)]
    pub tags: Vec<String>,
    /// Smoothed success rate in `[0.0, 1.0]`. Starts at `0.0`.
    #[serde(default)]
    pub success_rate: f64,
    /// Number of times [`SkillLibrary::record_use`] has been called.
    #[serde(default)]
    pub usage_count: u64,

    // ── Voyager-style extraction fields (§16.3.2-16.3.4) ───────────
    /// Longer description (1-2 sentences) of the skill's purpose.
    #[serde(default)]
    pub description: String,
    /// Plan identifier where this skill was first extracted.
    #[serde(default)]
    pub plan_id: String,
    /// Files touched in the originating task.
    #[serde(default)]
    pub files: Vec<String>,
    /// Numbered-step recipe extracted from a successful episode (≤750 chars).
    #[serde(default)]
    pub pattern: String,
    /// Eval score from the originating episode, in `[0.0, 1.0]`.
    #[serde(default)]
    pub score: f64,
    /// When the skill was first extracted.
    #[serde(default)]
    pub first_seen: Option<DateTime<Utc>>,
    /// When the skill was last injected into a prompt.
    #[serde(default)]
    pub last_matched: Option<DateTime<Utc>>,
    /// How many prompts have had this skill injected.
    #[serde(default)]
    pub match_count: u32,
    /// Of those injections, how many led to a gate pass.
    #[serde(default)]
    pub validated_count: u32,
    /// Task category for dedup (skills sharing ≥70% tags + same category are duplicates).
    #[serde(default)]
    pub task_category: String,
}

impl Skill {
    /// Construct a new skill with defaults for telemetry + example fields.
    pub fn new(
        name: impl Into<String>,
        summary: impl Into<String>,
        prompt_template: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let summary = summary.into();
        let prompt_template = prompt_template.into();
        Self {
            id: name.clone(),
            name,
            summary: summary.clone(),
            prompt_template: prompt_template.clone(),
            precondition: String::new(),
            procedure: prompt_template,
            postcondition: summary,
            confidence: 0.0,
            source_episodes: Vec::new(),
            validations: 0,
            failures: 0,
            task_categories: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            last_validated_at: None,
            required_tools: Vec::new(),
            example_inputs: Vec::new(),
            example_outputs: Vec::new(),
            tags: Vec::new(),
            success_rate: 0.0,
            usage_count: 0,
            description: String::new(),
            plan_id: String::new(),
            files: Vec::new(),
            pattern: String::new(),
            score: 0.0,
            first_seen: None,
            last_matched: None,
            match_count: 0,
            validated_count: 0,
            task_category: String::new(),
        }
    }

    /// Construct a structured skill contract with explicit applicability and outcome fields.
    pub fn new_structured(
        id: impl Into<String>,
        name: impl Into<String>,
        precondition: impl Into<String>,
        procedure: impl Into<String>,
        postcondition: impl Into<String>,
    ) -> Self {
        let id = id.into();
        let name = name.into();
        let precondition = precondition.into();
        let procedure = procedure.into();
        let postcondition = postcondition.into();

        Self {
            id: id.clone(),
            name,
            summary: postcondition.clone(),
            prompt_template: procedure.clone(),
            precondition,
            procedure,
            postcondition,
            confidence: 0.0,
            source_episodes: Vec::new(),
            validations: 0,
            failures: 0,
            task_categories: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            last_validated_at: None,
            required_tools: Vec::new(),
            example_inputs: Vec::new(),
            example_outputs: Vec::new(),
            tags: Vec::new(),
            success_rate: 0.0,
            usage_count: 0,
            description: String::new(),
            plan_id: String::new(),
            files: Vec::new(),
            pattern: String::new(),
            score: 0.0,
            first_seen: None,
            last_matched: None,
            match_count: 0,
            validated_count: 0,
            task_category: String::new(),
        }
    }

    /// Builder helper: attach required tool names.
    #[must_use]
    pub fn with_required_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_tools = tools.into_iter().map(Into::into).collect();
        self
    }

    /// Builder helper: attach example input/output pairs.
    #[must_use]
    pub fn with_examples<I, S1, S2>(mut self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (S1, S2)>,
        S1: Into<String>,
        S2: Into<String>,
    {
        for (input, output) in pairs {
            self.example_inputs.push(input.into());
            self.example_outputs.push(output.into());
        }
        self
    }

    /// Builder helper: attach tags.
    #[must_use]
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Builder helper: attach task categories.
    #[must_use]
    pub fn with_task_categories<I, S>(mut self, categories: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.task_categories = categories.into_iter().map(Into::into).collect();
        if self.task_category.is_empty() {
            self.task_category = self.task_categories.first().cloned().unwrap_or_default();
        }
        self
    }

    /// Builder helper: attach source episode IDs.
    #[must_use]
    pub fn with_source_episodes<I, S>(mut self, episode_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.source_episodes = episode_ids.into_iter().map(Into::into).collect();
        self
    }

    fn normalize(&mut self) {
        if self.id.is_empty() {
            self.id = self.name.clone();
        }
        if self.name.is_empty() {
            self.name = self.id.clone();
        }
        if self.procedure.is_empty() {
            self.procedure = self.prompt_template.clone();
        }
        if self.prompt_template.is_empty() {
            self.prompt_template = self.procedure.clone();
        }
        if self.postcondition.is_empty() {
            self.postcondition = self.summary.clone();
        }
        if self.summary.is_empty() {
            self.summary = self.postcondition.clone();
        }
        if self.created_at.is_empty() {
            self.created_at = self
                .first_seen
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339());
        }

        if self.task_categories.is_empty() {
            if !self.task_category.is_empty() {
                self.task_categories.push(self.task_category.clone());
            }
        } else {
            self.task_categories = dedup_strings(std::mem::take(&mut self.task_categories));
            if self.task_category.is_empty() {
                self.task_category = self.task_categories.first().cloned().unwrap_or_default();
            } else if !self.task_categories.contains(&self.task_category) {
                self.task_categories.insert(0, self.task_category.clone());
            }
        }

        if self.validations == 0 && self.failures == 0 {
            if self.usage_count > 0 {
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
                let inferred_validations =
                    (self.success_rate.clamp(0.0, 1.0) * self.usage_count as f64).round() as u64;
                self.validations = inferred_validations.min(self.usage_count);
                self.failures = self.usage_count.saturating_sub(self.validations);
            } else if self.validated_count > 0 {
                self.validations = u64::from(self.validated_count);
            }
        }

        if self.last_validated_at.is_none() && (self.validations > 0 || self.failures > 0) {
            self.last_validated_at = self.last_matched.map(|ts| ts.to_rfc3339());
        }

        self.recompute_confidence();
    }

    fn recompute_confidence(&mut self) {
        let total = self.validations.saturating_add(self.failures);
        self.confidence = if total == 0 {
            self.success_rate.clamp(0.0, 1.0)
        } else {
            #[allow(clippy::cast_precision_loss)]
            let total_f = total as f64;
            #[allow(clippy::cast_precision_loss)]
            let validations_f = self.validations as f64;
            (validations_f / total_f).clamp(0.0, 1.0)
        };
    }

    fn matches_task_category(&self, task_category: &str) -> bool {
        if task_category.is_empty() {
            return true;
        }
        self.task_category == task_category
            || self
                .task_categories
                .iter()
                .any(|category| category == task_category)
    }
}

/// Structured query for [`SkillLibrary::select`].
#[derive(Debug, Clone, Default)]
pub struct SkillQuery {
    /// Tags to match against skill tags (used in scoring).
    pub tags: Vec<String>,
    /// Optional category filter — only skills in this category are considered.
    pub category: Option<String>,
    /// File paths that hint at relevance (each match adds +0.2 to relevance).
    pub files_hint: Vec<String>,
}

/// One gate result passed into skill extraction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillGateResult {
    /// Gate name.
    pub gate: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Numeric gate score.
    pub score: f64,
}

impl SkillGateResult {
    /// Construct a new extraction gate result.
    pub fn new(gate: impl Into<String>, passed: bool, score: f64) -> Self {
        Self {
            gate: gate.into(),
            passed,
            score,
        }
    }
}

/// Input payload for [`SkillLibrary::extract_skill`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillExtractionRequest {
    /// Files the task touched.
    pub task_files: Vec<String>,
    /// Complexity tier from `tasks.toml`.
    pub task_tier: String,
    /// Symbol names referenced by the task description.
    pub symbols: Vec<String>,
    /// Model used for the task.
    pub model: String,
    /// Hash of the rendered system prompt.
    pub prompt_hash: String,
    /// Gate results that passed the task.
    pub gate_results: Vec<SkillGateResult>,
}

impl SkillExtractionRequest {
    /// Construct a new request.
    pub fn new(
        task_files: Vec<String>,
        task_tier: impl Into<String>,
        symbols: Vec<String>,
        model: impl Into<String>,
        prompt_hash: impl Into<String>,
        gate_results: Vec<SkillGateResult>,
    ) -> Self {
        Self {
            task_files,
            task_tier: task_tier.into(),
            symbols,
            model: model.into(),
            prompt_hash: prompt_hash.into(),
            gate_results,
        }
    }
}

/// Structured skill extracted from a successful episode before promotion into
/// the persistent [`SkillLibrary`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillCandidate {
    /// Stable candidate identifier.
    #[serde(default)]
    pub id: String,
    /// Source episode identifier (`episode_id` when present, else `id`).
    #[serde(default)]
    pub source_episode_id: String,
    /// Task identifier associated with the episode.
    #[serde(default)]
    pub task_id: String,
    /// Short candidate title.
    #[serde(default)]
    pub title: String,
    /// Task type or category where the skill applies.
    #[serde(default)]
    pub task_category: String,
    /// Complexity band where the skill applies.
    #[serde(default)]
    pub complexity: String,
    /// Files involved in the successful episode.
    #[serde(default)]
    pub files_involved: Vec<String>,
    /// Short summary of the tool call sequence.
    #[serde(default)]
    pub tool_sequence_summary: String,
    /// Natural-language applicability contract.
    #[serde(default)]
    pub precondition: String,
    /// Natural-language success condition.
    #[serde(default)]
    pub postcondition: String,
    /// Natural-language reusable skill description.
    #[serde(default)]
    pub skill_description: String,
    /// Short summary of the successful output.
    #[serde(default)]
    pub output_summary: String,
    /// Gate outcomes attached to the source episode.
    #[serde(default)]
    pub gate_results: Vec<SkillGateResult>,
}

#[derive(Debug, Serialize)]
struct SkillCandidateEpisodeRecord {
    source_episode_id: String,
    task_id: String,
    agent_id: String,
    model: String,
    task_category: String,
    complexity: String,
    files_involved: Vec<String>,
    tool_sequence: Vec<String>,
    gate_results: Vec<SkillGateResult>,
    output_summary: String,
    reasoning_summary: String,
    duration_secs: f64,
    turns: u64,
    usage: crate::episode_logger::Usage,
}

impl SkillCandidateEpisodeRecord {
    fn from_episode(episode: &crate::episode_logger::Episode) -> Self {
        Self {
            source_episode_id: episode_source_id(episode).to_string(),
            task_id: episode.task_id.clone(),
            agent_id: episode.agent_id.clone(),
            model: episode_model(episode),
            task_category: episode_task_category(episode),
            complexity: episode_complexity(episode),
            files_involved: episode_files(episode),
            tool_sequence: episode_tool_sequence(episode),
            gate_results: episode_gate_results(episode),
            output_summary: episode_output_summary(episode),
            reasoning_summary: episode.reasoning_summary.clone().unwrap_or_default(),
            duration_secs: episode.duration_secs,
            turns: episode.turns,
            usage: episode.usage.clone(),
        }
    }
}

/// Extract candidate skills from successful episodes that passed their gates.
///
/// The judge agent receives one episode at a time and returns a structured
/// summary capturing the precondition, postcondition, tool sequence, and
/// reusable skill description. If the judge response is malformed, a
/// deterministic fallback candidate is synthesized from the episode metadata.
#[must_use]
pub async fn extract_skill_candidates(
    episodes: &[crate::episode_logger::Episode],
    judge_agent: &dyn Agent,
) -> Vec<SkillCandidate> {
    let mut out = Vec::new();

    for episode in episodes
        .iter()
        .filter(|episode| episode_is_skill_candidate(episode))
    {
        let prompt = build_skill_candidate_prompt(episode);
        let input = Signal::builder(Kind::Prompt)
            .body(Body::text(prompt))
            .build();
        let result = judge_agent.run(&input, &Context::now()).await;

        let candidate = if result.success {
            result
                .output
                .body
                .as_text()
                .ok()
                .and_then(parse_skill_candidate_response)
                .map(|candidate| normalize_skill_candidate(candidate, episode))
                .unwrap_or_else(|| fallback_skill_candidate(episode))
        } else {
            fallback_skill_candidate(episode)
        };

        out.push(candidate);
    }

    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchSkillOutcome {
    Success,
    Failure,
}

/// Produces the `pattern` text from an [`Episode`](crate::episode_logger::Episode).
///
/// The default [`TemplatePatternGenerator`] uses a simple string template;
/// a v2 LLM-backed generator can replace it for richer step-by-step recipes.
pub trait PatternGenerator: Send + Sync {
    /// Generate a recipe string from episode metadata.
    fn generate(&self, episode: &crate::episode_logger::Episode) -> String;
}

/// Simple template-based [`PatternGenerator`] — no LLM calls, suitable for
/// offline use. Produces a 4-line recipe from episode `extra` fields.
pub struct TemplatePatternGenerator;

impl PatternGenerator for TemplatePatternGenerator {
    fn generate(&self, episode: &crate::episode_logger::Episode) -> String {
        let files = episode
            .extra
            .get("files")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let role = episode
            .extra
            .get("role")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&episode.agent_id);
        let model = episode
            .extra
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let tags = episode
            .extra
            .get("task_tags")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let reflection = episode
            .extra
            .get("verbal_reflection")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("not captured");

        format!(
            "1. Edit files: {files}\n\
             2. Agent role: {role} using {model}\n\
             3. Tags: {tags}\n\
             4. Approach summary: {reflection}"
        )
    }
}

/// Maximum pattern length in characters (~250 tokens at 3 chars/token).
const MAX_PATTERN_CHARS: usize = 750;

fn skill_candidate_schema() -> Value {
    json!({
        "type": "object",
        "required": [
            "title",
            "tool_sequence_summary",
            "precondition",
            "postcondition",
            "skill_description"
        ],
        "properties": {
            "id": { "type": "string" },
            "source_episode_id": { "type": "string" },
            "task_id": { "type": "string" },
            "title": { "type": "string" },
            "task_category": { "type": "string" },
            "complexity": { "type": "string" },
            "files_involved": {
                "type": "array",
                "items": { "type": "string" }
            },
            "tool_sequence_summary": { "type": "string" },
            "precondition": { "type": "string" },
            "postcondition": { "type": "string" },
            "skill_description": { "type": "string" },
            "output_summary": { "type": "string" },
            "gate_results": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "gate": { "type": "string" },
                        "passed": { "type": "boolean" },
                        "score": { "type": "number" }
                    }
                }
            }
        }
    })
}

fn build_skill_candidate_prompt(episode: &crate::episode_logger::Episode) -> String {
    let record = SkillCandidateEpisodeRecord::from_episode(episode);
    let record_json = serde_json::to_string_pretty(&record).unwrap_or_else(|_| "{}".to_string());
    let extractor = NlToFormatConverter::new();

    format!(
        "You are Roko's skill extractor.\n\
         Read one successful execution episode and extract a reusable skill candidate.\n\
         Be concrete and conservative. Use only evidence present in the episode.\n\
         Summarize the tool sequence, identify the precondition, identify the postcondition, and write a reusable natural-language skill description.\n\
         Assume this is a cheap haiku-class summarization pass, so keep fields concise and specific.\n\n\
         {}\n\n\
         Episode:\n```json\n{}\n```\n",
        extractor.extraction_prompt(&skill_candidate_schema()),
        record_json
    )
}

fn parse_skill_candidate_response(response: &str) -> Option<SkillCandidate> {
    let extractor = NlToFormatConverter::new();
    let extracted = extractor
        .convert(response, &skill_candidate_schema())
        .ok()?;
    serde_json::from_value(extracted).ok()
}

/// Extract a string from `episode.extra[key]`, defaulting to `""`.
fn extra_str(episode: &crate::episode_logger::Episode, key: &str) -> String {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Extract a `Vec<String>` from `episode.extra[key]` (expects a JSON array of strings).
fn extra_strings(episode: &crate::episode_logger::Episode, key: &str) -> Vec<String> {
    episode
        .extra
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn episode_source_id(episode: &crate::episode_logger::Episode) -> &str {
    if episode.episode_id.trim().is_empty() {
        &episode.id
    } else {
        &episode.episode_id
    }
}

fn episode_gate_passed(episode: &crate::episode_logger::Episode) -> bool {
    if let Some(passed) = episode.extra.get("gate_passed").and_then(Value::as_bool) {
        return passed;
    }

    if !episode.gate_verdicts.is_empty() {
        episode.gate_verdicts.iter().all(|verdict| verdict.passed)
    } else {
        episode.success
    }
}

fn episode_is_skill_candidate(episode: &crate::episode_logger::Episode) -> bool {
    episode.success && episode_gate_passed(episode)
}

fn episode_task_category(episode: &crate::episode_logger::Episode) -> String {
    extra_str(episode, "task_category")
}

fn episode_complexity(episode: &crate::episode_logger::Episode) -> String {
    extra_str(episode, "complexity_band")
}

fn episode_files(episode: &crate::episode_logger::Episode) -> Vec<String> {
    dedup_strings(extra_strings(episode, "files"))
}

fn episode_model(episode: &crate::episode_logger::Episode) -> String {
    let model = extra_str(episode, "model");
    if model.is_empty() {
        episode.model.clone()
    } else {
        model
    }
}

fn episode_gate_results(episode: &crate::episode_logger::Episode) -> Vec<SkillGateResult> {
    episode
        .gate_verdicts
        .iter()
        .map(|verdict| {
            SkillGateResult::new(
                &verdict.gate,
                verdict.passed,
                f64::from(u8::from(verdict.passed)),
            )
        })
        .collect()
}

fn episode_output_summary(episode: &crate::episode_logger::Episode) -> String {
    if let Some(summary) = episode
        .extra
        .get("output_summary")
        .and_then(Value::as_str)
        .filter(|summary| !summary.trim().is_empty())
    {
        return summary.to_string();
    }

    if let Some(summary) = episode.reasoning_summary.as_deref()
        && !summary.trim().is_empty()
    {
        return summary.to_string();
    }

    if !episode.task_id.trim().is_empty() {
        return format!("Completed task {}", episode.task_id);
    }

    "Completed successfully".to_string()
}

fn episode_tool_sequence(episode: &crate::episode_logger::Episode) -> Vec<String> {
    for key in ["tool_calls", "tool_sequence", "tools_used", "tools"] {
        if let Some(arr) = episode.extra.get(key).and_then(Value::as_array) {
            let tools: Vec<String> = arr.iter().filter_map(extract_tool_name).collect();
            if !tools.is_empty() {
                return tools;
            }
        }
    }

    episode
        .external_actions
        .iter()
        .filter_map(extract_tool_name)
        .collect()
}

fn extract_tool_name(value: &Value) -> Option<String> {
    if let Some(name) = value.as_str().filter(|name| !name.trim().is_empty()) {
        return Some(name.to_string());
    }

    let object = value.as_object()?;
    for key in ["tool_name", "tool", "name", "action_type", "kind"] {
        if let Some(name) = object.get(key).and_then(Value::as_str)
            && !name.trim().is_empty()
        {
            return Some(name.to_string());
        }
    }

    object
        .get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn summarize_tool_sequence(episode: &crate::episode_logger::Episode) -> String {
    let tools = episode_tool_sequence(episode);
    if tools.is_empty() {
        "No tool sequence captured.".to_string()
    } else {
        tools.join(" -> ")
    }
}

fn fallback_skill_candidate(episode: &crate::episode_logger::Episode) -> SkillCandidate {
    let gate_results = episode_gate_results(episode);
    let output_summary = episode_output_summary(episode);
    let task_category = episode_task_category(episode);
    let complexity = episode_complexity(episode);
    let files_involved = episode_files(episode);
    let tool_sequence_summary = summarize_tool_sequence(episode);
    let precondition = build_precondition(&task_category, &complexity, &files_involved);
    let postcondition = build_postcondition(&gate_results, &output_summary);
    let title = if !task_category.is_empty() {
        format!("{task_category} episode pattern")
    } else {
        "successful episode pattern".to_string()
    };
    let skill_description = format!(
        "Use {} to complete the task and reach {}.",
        tool_sequence_summary, postcondition
    );

    SkillCandidate {
        id: format!("candidate_{}", episode_source_id(episode)),
        source_episode_id: episode_source_id(episode).to_string(),
        task_id: episode.task_id.clone(),
        title,
        task_category,
        complexity,
        files_involved,
        tool_sequence_summary,
        precondition,
        postcondition,
        skill_description,
        output_summary,
        gate_results,
    }
}

fn normalize_skill_candidate(
    mut candidate: SkillCandidate,
    episode: &crate::episode_logger::Episode,
) -> SkillCandidate {
    let fallback = fallback_skill_candidate(episode);

    if candidate.id.trim().is_empty() {
        candidate.id = fallback.id;
    }
    if candidate.source_episode_id.trim().is_empty() {
        candidate.source_episode_id = fallback.source_episode_id;
    }
    if candidate.task_id.trim().is_empty() {
        candidate.task_id = fallback.task_id;
    }
    if candidate.title.trim().is_empty() {
        candidate.title = fallback.title;
    }
    if candidate.task_category.trim().is_empty() {
        candidate.task_category = fallback.task_category;
    }
    if candidate.complexity.trim().is_empty() {
        candidate.complexity = fallback.complexity;
    }
    if candidate.files_involved.is_empty() {
        candidate.files_involved = fallback.files_involved;
    } else {
        candidate.files_involved = dedup_strings(candidate.files_involved);
    }
    if candidate.tool_sequence_summary.trim().is_empty() {
        candidate.tool_sequence_summary = fallback.tool_sequence_summary;
    }
    if candidate.precondition.trim().is_empty() {
        candidate.precondition = fallback.precondition;
    }
    if candidate.postcondition.trim().is_empty() {
        candidate.postcondition = fallback.postcondition;
    }
    if candidate.skill_description.trim().is_empty() {
        candidate.skill_description = fallback.skill_description;
    }
    if candidate.output_summary.trim().is_empty() {
        candidate.output_summary = fallback.output_summary;
    }
    if candidate.gate_results.is_empty() {
        candidate.gate_results = fallback.gate_results;
    }

    candidate
}

fn build_precondition(task_category: &str, complexity: &str, files_involved: &[String]) -> String {
    let mut parts = Vec::new();
    if !task_category.is_empty() {
        parts.push(format!("{task_category} task"));
    }
    if !complexity.is_empty() {
        parts.push(format!("{complexity} complexity"));
    }
    if !files_involved.is_empty() {
        parts.push(format!("touching {}", files_involved.join(", ")));
    }

    if parts.is_empty() {
        "Apply when a similar successful task recurs.".to_string()
    } else {
        format!("Apply when the task is a {}.", parts.join(" with "))
    }
}

fn build_postcondition(gate_results: &[SkillGateResult], output_summary: &str) -> String {
    let gate_summary = if gate_results.is_empty() {
        "completed successfully".to_string()
    } else {
        let passed = gate_results
            .iter()
            .filter(|result| result.passed)
            .map(|result| result.gate.clone())
            .collect::<Vec<_>>();
        if passed.is_empty() {
            "completed without a recorded passing gate".to_string()
        } else {
            format!("passed gates: {}", passed.join(", "))
        }
    };

    format!("{gate_summary}; output: {output_summary}")
}

/// In-memory, JSON-backed registry of [`Skill`] records.
#[derive(Debug)]
pub struct SkillLibrary {
    path: PathBuf,
    skills: RwLock<BTreeMap<String, Skill>>,
    write_lock: AsyncMutex<()>,
}

impl SkillLibrary {
    /// Open (or create) a skill library at `path`. If the file exists it is
    /// deserialized; if it does not, an empty library is returned and will
    /// be created on the next mutating call.
    pub async fn new(path: impl AsRef<Path>) -> Result<Self, SkillLibraryError> {
        let path = path.as_ref().to_path_buf();
        let skills = match tokio::fs::read(&path).await {
            Ok(bytes) if bytes.is_empty() => BTreeMap::new(),
            Ok(bytes) => {
                let list: Vec<Skill> = serde_json::from_slice(&bytes)?;
                list.into_iter()
                    .map(|mut skill| {
                        skill.normalize();
                        (skill.name.clone(), skill)
                    })
                    .collect()
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(err) => return Err(SkillLibraryError::Io(err)),
        };
        Ok(Self {
            path,
            skills: RwLock::new(skills),
            write_lock: AsyncMutex::new(()),
        })
    }

    /// Register a new skill. Returns [`SkillLibraryError::Duplicate`] if a
    /// skill with the same name is already present.
    pub async fn register(&self, skill: &Skill) -> Result<(), SkillLibraryError> {
        let mut skill = skill.clone();
        skill.normalize();
        {
            let mut guard = self.skills.write();
            if guard.contains_key(&skill.name) {
                return Err(SkillLibraryError::Duplicate(skill.name.clone()));
            }
            guard.insert(skill.name.clone(), skill);
        }
        self.persist().await
    }

    /// Retrieve a cloned snapshot of the skill with the given name.
    pub fn get(&self, name: &str) -> Option<Skill> {
        self.skills.read().get(name).cloned()
    }

    /// Return all skills in the library, sorted by name.
    pub fn list(&self) -> Vec<Skill> {
        self.skills.read().values().cloned().collect()
    }

    /// Number of registered skills.
    pub fn len(&self) -> usize {
        self.skills.read().len()
    }

    /// Whether the library has zero registered skills.
    pub fn is_empty(&self) -> bool {
        self.skills.read().is_empty()
    }

    /// Record an invocation of a skill. Updates `usage_count` and folds the
    /// outcome into a rolling mean `success_rate`.
    ///
    /// Returns [`SkillLibraryError::NotFound`] if `name` is not registered.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn record_use(&self, name: &str, success: bool) -> Result<(), SkillLibraryError> {
        {
            let mut guard = self.skills.write();
            let Some(skill) = guard.get_mut(name) else {
                return Err(SkillLibraryError::NotFound(name.to_string()));
            };
            let prior = skill.usage_count;
            let outcome = f64::from(u8::from(success));
            // Running mean: new_mean = (prior_mean * n + outcome) / (n + 1).
            // f64 is wide enough for all realistic counters; cast is lossy
            // only beyond 2^53 which we clamp at below.
            #[allow(clippy::cast_precision_loss)]
            let prior_f = prior as f64;
            skill.success_rate = (skill.success_rate.mul_add(prior_f, outcome)) / (prior_f + 1.0);
            skill.usage_count = prior.saturating_add(1);
            Self::record_validation_fields(skill, success);
        }
        self.persist().await
    }

    /// Return all skills that carry `tag` in their `tags` vector.
    pub fn search_by_tag(&self, tag: &str) -> Vec<Skill> {
        self.skills
            .read()
            .values()
            .filter(|s| s.tags.iter().any(|t| t == tag))
            .cloned()
            .collect()
    }

    /// On-disk path backing this library.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Query for dispatch-time skill suggestions using task metadata.
    ///
    /// The inputs map to [`SkillQuery`] as follows:
    /// - `task_files` -> file hints
    /// - `task_tier` -> category filter
    /// - `symbols` -> tags
    ///
    /// Returns the top matching skills so callers can pick the most suitable
    /// one for prompt injection.
    pub fn query(&self, task_files: &[String], task_tier: &str, symbols: &[String]) -> Vec<Skill> {
        let query = SkillQuery {
            tags: symbols.to_vec(),
            category: (!task_tier.is_empty()).then(|| task_tier.to_string()),
            files_hint: task_files.to_vec(),
        };
        self.select(&query, usize::MAX)
    }

    /// Return all skills applicable to `task_category`, ordered by confidence,
    /// validation count, then name.
    pub fn query_by_task_category(&self, task_category: &str) -> Vec<Skill> {
        let mut skills: Vec<Skill> = self
            .skills
            .read()
            .values()
            .filter(|skill| skill.matches_task_category(task_category))
            .cloned()
            .collect();
        skills.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.validations.cmp(&a.validations))
                .then_with(|| a.name.cmp(&b.name))
        });
        skills
    }

    // ── Voyager-style extraction & selection (§16.3.2-16.3.4) ──────

    /// Extract a skill from a successful episode that passed gates on the
    /// first attempt (Voyager discipline). Returns `None` if the episode
    /// does not qualify or a near-duplicate skill already exists.
    ///
    /// Extraction criteria (§16.3.2):
    /// 1. `episode.success == true`
    /// 2. `extra["iteration"] == 1` (first-attempt success)
    /// 3. `extra["complexity_band"]` in `["standard", "complex"]`
    /// 4. No existing skill shares ≥70% tag overlap AND same `task_category`
    #[allow(clippy::too_many_lines)]
    pub async fn extract(
        &self,
        episode: &crate::episode_logger::Episode,
        generator: &dyn PatternGenerator,
    ) -> Option<Skill> {
        if !Self::episode_qualifies(episode) {
            return None;
        }

        let pattern = generator.generate(episode);
        if pattern.is_empty() {
            return None;
        }
        let pattern = if pattern.len() > MAX_PATTERN_CHARS {
            pattern[..MAX_PATTERN_CHARS].to_string()
        } else {
            pattern
        };

        let tags = extra_strings(episode, "task_tags");
        let category = extra_str(episode, "task_category");

        if self.is_duplicate(&tags, &category) {
            return None;
        }

        let skill = Self::build_skill_from_episode(episode, pattern, tags, category);

        {
            let mut guard = self.skills.write();
            if guard.contains_key(&skill.name) {
                return None;
            }
            guard.insert(skill.name.clone(), skill.clone());
        }
        self.persist().await.ok()?;
        Some(skill)
    }

    /// Extract a skill from a successful task dispatch.
    ///
    /// The request carries the task's touched files, complexity tier,
    /// referenced symbols, model, prompt hash, and gate results. The
    /// extraction is intentionally compact: it persists one reusable skill
    /// record and returns `None` if the request is incomplete or a near
    /// duplicate already exists.
    pub async fn extract_skill(&self, request: SkillExtractionRequest) -> Option<Skill> {
        self.record_dispatch_skill(request, DispatchSkillOutcome::Success)
            .await
    }

    /// Record a failed task dispatch using the same structural inputs as
    /// [`SkillLibrary::extract_skill`]. Failure records are stored as
    /// low-score skills tagged with `outcome:failure` so they can be kept
    /// for later analysis without affecting normal selection.
    pub async fn record_failure(&self, request: SkillExtractionRequest) -> Option<Skill> {
        self.record_dispatch_skill(request, DispatchSkillOutcome::Failure)
            .await
    }

    async fn record_dispatch_skill(
        &self,
        request: SkillExtractionRequest,
        outcome: DispatchSkillOutcome,
    ) -> Option<Skill> {
        if request.task_files.is_empty() && request.symbols.is_empty() {
            return None;
        }
        if request.task_tier.is_empty()
            || request.model.is_empty()
            || request.prompt_hash.is_empty()
        {
            return None;
        }

        let SkillExtractionRequest {
            task_files,
            task_tier,
            symbols,
            model,
            prompt_hash,
            gate_results,
        } = request;

        let task_files = dedup_strings(task_files);
        let symbols = dedup_strings(symbols);
        let mut tags = Vec::with_capacity(symbols.len() + 3);
        tags.extend(symbols.iter().cloned());
        tags.push(format!("model:{}", model));
        tags.push(format!("prompt:{}", short_hash(&prompt_hash)));
        if matches!(outcome, DispatchSkillOutcome::Failure) {
            tags.push("outcome:failure".into());
        }
        let category = task_tier.clone();

        if matches!(outcome, DispatchSkillOutcome::Success) {
            if gate_results.is_empty() || gate_results.iter().any(|g| !g.passed) {
                return None;
            }
            if self.is_duplicate(&tags, &category) {
                return None;
            }
        }

        let (name, summary, description, score, pattern) = match outcome {
            DispatchSkillOutcome::Success => {
                let passed_scores: Vec<f64> = gate_results
                    .iter()
                    .filter(|gate| gate.passed)
                    .map(|gate| gate.score.clamp(0.0, 1.0))
                    .collect();
                let score = if passed_scores.is_empty() {
                    1.0
                } else {
                    passed_scores.iter().sum::<f64>() / passed_scores.len() as f64
                };

                let gate_summary = gate_results
                    .iter()
                    .map(|gate| {
                        format!(
                            "{}:{}:{:.2}",
                            gate.gate,
                            if gate.passed { "pass" } else { "fail" },
                            gate.score
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                let pattern = format!(
                    "1. Touched files: {}\n\
                     2. Symbols: {}\n\
                     3. Model: {}\n\
                     4. Prompt hash: {}\n\
                     5. Gates: {}",
                    task_files.join(", "),
                    symbols.join(", "),
                    model,
                    prompt_hash,
                    gate_summary
                );
                let pattern = if pattern.len() > MAX_PATTERN_CHARS {
                    pattern[..MAX_PATTERN_CHARS].to_string()
                } else {
                    pattern
                };

                (
                    format!(
                        "skill_{}_{}",
                        sanitize_component(&task_tier),
                        short_hash(&prompt_hash)
                    ),
                    format!("Successful {} task on {}", task_tier, model),
                    format!(
                        "Extracted from a successful {} task using {}.",
                        task_tier, model
                    ),
                    score,
                    pattern,
                )
            }
            DispatchSkillOutcome::Failure => {
                let gate_summary = if gate_results.is_empty() {
                    "none".to_string()
                } else {
                    gate_results
                        .iter()
                        .map(|gate| {
                            format!(
                                "{}:{}:{:.2}",
                                gate.gate,
                                if gate.passed { "pass" } else { "fail" },
                                gate.score
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };

                let pattern = format!(
                    "1. Outcome: failure\n\
                     2. Touched files: {}\n\
                     3. Symbols: {}\n\
                     4. Model: {}\n\
                     5. Prompt hash: {}\n\
                     6. Gates: {}",
                    task_files.join(", "),
                    symbols.join(", "),
                    model,
                    prompt_hash,
                    gate_summary
                );
                let pattern = if pattern.len() > MAX_PATTERN_CHARS {
                    pattern[..MAX_PATTERN_CHARS].to_string()
                } else {
                    pattern
                };

                (
                    format!(
                        "failure_{}_{}",
                        sanitize_component(&task_tier),
                        short_hash(&prompt_hash)
                    ),
                    format!("Failed {} task on {}", task_tier, model),
                    format!("Failure pattern from a {} task using {}.", task_tier, model),
                    0.0,
                    pattern,
                )
            }
        };

        let mut skill = Skill::new(name, summary, pattern.clone());
        skill.description = description;
        skill.tags = tags;
        skill.plan_id = String::new();
        skill.files = task_files;
        skill.pattern = pattern;
        skill.score = score;
        skill.first_seen = Some(Utc::now());
        skill.task_category = category;
        skill.task_categories = vec![skill.task_category.clone()];
        skill.precondition = format!("Apply for {} tasks matching these symbols.", task_tier);
        skill.procedure = skill.pattern.clone();
        skill.postcondition = skill.summary.clone();
        skill.created_at = Utc::now().to_rfc3339();

        {
            let mut guard = self.skills.write();
            if guard.contains_key(&skill.name) {
                return None;
            }
            guard.insert(skill.name.clone(), skill.clone());
        }
        self.persist().await.ok()?;
        Some(skill)
    }

    /// Check whether an episode qualifies for skill extraction (§16.3.2).
    fn episode_qualifies(episode: &crate::episode_logger::Episode) -> bool {
        if !episode.success {
            return false;
        }
        let iteration = episode
            .extra
            .get("iteration")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1);
        if iteration != 1 {
            return false;
        }
        let band = episode
            .extra
            .get("complexity_band")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("standard");
        band == "standard" || band == "complex"
    }

    /// Check whether `tags`/`category` would be a near-duplicate of an existing skill.
    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::significant_drop_tightening)]
    fn is_duplicate(&self, tags: &[String], category: &str) -> bool {
        if category.is_empty() || tags.is_empty() {
            return false;
        }
        let guard = self.skills.read();
        for existing in guard.values() {
            if existing.matches_task_category(category) {
                let overlap = tags.iter().filter(|t| existing.tags.contains(t)).count();
                let denom = tags.len().max(1);
                if overlap as f64 / denom as f64 >= 0.7 {
                    return true;
                }
            }
        }
        false
    }

    /// Assemble a [`Skill`] from episode metadata.
    fn build_skill_from_episode(
        episode: &crate::episode_logger::Episode,
        pattern: String,
        tags: Vec<String>,
        category: String,
    ) -> Skill {
        let files = extra_strings(episode, "files");
        let plan_id = extra_str(episode, "plan_id");
        let eval_score = episode
            .extra
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);

        let name = format!("skill_{}", episode.id);
        let mut skill = Skill::new(&name, &episode.task_id, pattern.clone());
        skill.description.clone_from(&episode.task_id);
        skill.tags = tags;
        skill.plan_id = plan_id;
        skill.files = files;
        skill.pattern = pattern;
        skill.score = eval_score;
        skill.first_seen = Some(Utc::now());
        skill.task_category = category;
        skill.task_categories = vec![skill.task_category.clone()];
        skill.precondition = format!("Apply for {} tasks.", skill.task_category);
        skill.procedure = skill.pattern.clone();
        skill.postcondition = skill.summary.clone();
        skill.source_episodes = vec![episode.id.clone()];
        skill.created_at = Utc::now().to_rfc3339();
        skill
    }

    /// Retrieve the top-`limit` skills matching `query`, scored by:
    ///
    /// ```text
    /// tag_overlap = |query.tags ∩ skill.tags| / max(|query.tags|, 1)
    /// file_hint   = 0.2 if any files_hint matches skill.files, else 0.0
    /// relevance   = tag_overlap + file_hint
    /// final       = skill.score × relevance × (1 + 0.1 × √validated_count)
    /// ```
    ///
    /// An empty query (no tags, no category, no `files_hint`) returns an
    /// empty `Vec`. Ties are broken by `name` lexicographic ascending.
    #[allow(clippy::cast_precision_loss, clippy::significant_drop_tightening)]
    pub fn select(&self, query: &SkillQuery, limit: usize) -> Vec<Skill> {
        if query.tags.is_empty() && query.category.is_none() && query.files_hint.is_empty() {
            return Vec::new();
        }

        let guard = self.skills.read();
        let mut scored: Vec<(f64, &Skill)> = guard
            .values()
            .filter(|skill| {
                if let Some(ref cat) = query.category {
                    if !skill.matches_task_category(cat) {
                        return false;
                    }
                }
                true
            })
            .filter_map(|skill| {
                let tag_overlap = if query.tags.is_empty() {
                    0.0
                } else {
                    let overlap = query.tags.iter().filter(|t| skill.tags.contains(t)).count();
                    overlap as f64 / query.tags.len().max(1) as f64
                };

                let file_hint = if query.files_hint.iter().any(|f| skill.files.contains(f)) {
                    0.2
                } else {
                    0.0
                };

                let relevance = tag_overlap + file_hint;
                if relevance <= 0.0 {
                    return None;
                }

                let validated_bonus = 0.1_f64.mul_add(f64::from(skill.validated_count).sqrt(), 1.0);
                let final_score = skill.score * relevance * validated_bonus;

                Some((final_score, skill))
            })
            .collect();

        // Deterministic: score desc, name asc for tie-breaking
        scored.sort_by(|(sa, a), (sb, b)| {
            sb.partial_cmp(sa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.name.cmp(&b.name))
        });

        scored
            .into_iter()
            .take(limit)
            .map(|(_, s)| s.clone())
            .collect()
    }

    /// Record that a skill was injected into a prompt and whether the
    /// subsequent gate check passed. Increments `match_count` (always) and
    /// `validated_count` (if `gate_passed`), and updates `last_matched`.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn record_validation(
        &self,
        skill_name: &str,
        success: bool,
    ) -> Result<(), SkillLibraryError> {
        {
            let mut guard = self.skills.write();
            let Some(skill) = guard.get_mut(skill_name) else {
                return Err(SkillLibraryError::NotFound(skill_name.to_string()));
            };
            Self::record_validation_fields(skill, success);
        }
        self.persist().await
    }

    /// Record that a skill was injected into a prompt and whether the
    /// subsequent gate check passed. Increments `match_count` (always) and
    /// `validated_count` (if `gate_passed`), and updates `last_matched`.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn record_outcome(
        &self,
        skill_name: &str,
        gate_passed: bool,
    ) -> Result<(), SkillLibraryError> {
        {
            let mut guard = self.skills.write();
            let Some(skill) = guard.get_mut(skill_name) else {
                return Err(SkillLibraryError::NotFound(skill_name.to_string()));
            };
            skill.match_count = skill.match_count.saturating_add(1);
            if gate_passed {
                skill.validated_count = skill.validated_count.saturating_add(1);
            }
            skill.last_matched = Some(Utc::now());
            Self::record_validation_fields(skill, gate_passed);
        }
        self.persist().await
    }

    /// Remove skills not matched (or seen) within `days`. Keeps at least
    /// 10 skills regardless — the newest by `first_seen` survive.
    ///
    /// Returns the number of skills pruned.
    #[allow(clippy::significant_drop_tightening)]
    pub async fn prune_stale(&self, days: u32) -> usize {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(days));
        let mut pruned = 0;
        {
            let mut guard = self.skills.write();
            let total = guard.len();

            // Collect stale skills (last activity before cutoff)
            let mut stale: Vec<(String, DateTime<Utc>)> = guard
                .iter()
                .filter(|(_, s)| {
                    let last_active = s
                        .last_matched
                        .or(s.first_seen)
                        .unwrap_or_else(|| cutoff - chrono::Duration::seconds(1));
                    last_active < cutoff
                })
                .map(|(name, s)| {
                    let ts = s.first_seen.unwrap_or(cutoff);
                    (name.clone(), ts)
                })
                .collect();

            // Sort oldest-first so we remove the oldest ones
            stale.sort_by_key(|(_, ts)| *ts);

            // Never drop below 10 total skills
            let max_removable = total.saturating_sub(10);
            let to_remove = stale.len().min(max_removable);

            for (name, _) in stale.into_iter().take(to_remove) {
                guard.remove(&name);
                pruned += 1;
            }
        }
        if pruned > 0 {
            let _ = self.persist().await;
        }
        pruned
    }

    /// Serialize the in-memory map to the on-disk file atomically.
    ///
    /// Writes are serialized via an async mutex so that the tempfile+rename
    /// dance never races against itself under concurrent writers.
    async fn persist(&self) -> Result<(), SkillLibraryError> {
        let _guard = self.write_lock.lock().await;
        let snapshot: Vec<Skill> = self.skills.read().values().cloned().collect();
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    fn record_validation_fields(skill: &mut Skill, success: bool) {
        if success {
            skill.validations = skill.validations.saturating_add(1);
        } else {
            skill.failures = skill.failures.saturating_add(1);
        }
        skill.last_validated_at = Some(Utc::now().to_rfc3339());
        skill.recompute_confidence();
    }
}

fn dedup_strings(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        if seen.insert(value.clone()) {
            out.push(value);
        }
    }
    out
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn sanitize_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "unknown".to_string()
    } else {
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use roko_agent::MockAgent;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn sample_skill(name: &str) -> Skill {
        Skill::new(
            name,
            "sample summary",
            "You are a helpful assistant. Do the thing.",
        )
        .with_required_tools(["read", "write"])
        .with_tags(["sample", "test"])
        .with_examples([("in-a", "out-a"), ("in-b", "out-b")])
    }

    #[tokio::test]
    async fn register_and_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let skill = sample_skill("alpha");
        library.register(&skill).await.unwrap();

        let fetched = library.get("alpha").unwrap();
        assert_eq!(fetched.name, "alpha");
        assert_eq!(fetched.id, "alpha");
        assert_eq!(fetched.required_tools, vec!["read", "write"]);
        assert_eq!(fetched.example_inputs.len(), 2);
        assert_eq!(fetched.example_outputs.len(), 2);
        assert_eq!(fetched.usage_count, 0);
        assert!((fetched.success_rate - 0.0).abs() < f64::EPSILON);
        assert!(!fetched.created_at.is_empty());
    }

    #[tokio::test]
    async fn register_rejects_duplicates() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        library.register(&sample_skill("dup")).await.unwrap();
        let err = library
            .register(&sample_skill("dup"))
            .await
            .expect_err("duplicate should fail");
        assert!(matches!(err, SkillLibraryError::Duplicate(name) if name == "dup"));
        assert_eq!(library.len(), 1);
    }

    #[tokio::test]
    async fn list_returns_all_sorted() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        library.register(&sample_skill("gamma")).await.unwrap();
        library.register(&sample_skill("alpha")).await.unwrap();
        library.register(&sample_skill("beta")).await.unwrap();

        let names: Vec<String> = library.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["alpha", "beta", "gamma"]);
        assert_eq!(library.len(), 3);
        assert!(!library.is_empty());
    }

    #[tokio::test]
    async fn record_use_updates_counters_and_rate() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        library.register(&sample_skill("s1")).await.unwrap();

        library.record_use("s1", true).await.unwrap();
        library.record_use("s1", true).await.unwrap();
        library.record_use("s1", false).await.unwrap();
        library.record_use("s1", true).await.unwrap();

        let s = library.get("s1").unwrap();
        assert_eq!(s.usage_count, 4);
        // 3 successes / 4 attempts
        assert!((s.success_rate - 0.75).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_use_missing_returns_not_found() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let err = library
            .record_use("ghost", true)
            .await
            .expect_err("missing skill should fail");
        assert!(matches!(err, SkillLibraryError::NotFound(name) if name == "ghost"));
    }

    #[tokio::test]
    async fn persist_and_reload_preserves_state() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nested").join("skills.json");

        {
            let library = SkillLibrary::new(&path).await.unwrap();
            library.register(&sample_skill("persist")).await.unwrap();
            library.record_use("persist", true).await.unwrap();
            library.record_use("persist", false).await.unwrap();
        }

        let reloaded = SkillLibrary::new(&path).await.unwrap();
        let s = reloaded.get("persist").unwrap();
        assert_eq!(s.usage_count, 2);
        assert!((s.success_rate - 0.5).abs() < 1e-9);
        assert_eq!(s.required_tools, vec!["read", "write"]);
        assert_eq!(s.tags, vec!["sample", "test"]);
    }

    #[tokio::test]
    async fn search_by_tag_filters_correctly() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let a = Skill::new("a", "", "").with_tags(["rust", "fs"]);
        let b = Skill::new("b", "", "").with_tags(["rust"]);
        let c = Skill::new("c", "", "").with_tags(["python"]);
        library.register(&a).await.unwrap();
        library.register(&b).await.unwrap();
        library.register(&c).await.unwrap();

        let rust = library.search_by_tag("rust");
        let names: Vec<String> = rust.into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["a", "b"]);

        let fs = library.search_by_tag("fs");
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].name, "a");

        let none = library.search_by_tag("ruby");
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn missing_skill_returns_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        assert!(library.get("nope").is_none());
        assert!(library.is_empty());
        assert_eq!(library.len(), 0);
        assert!(library.list().is_empty());
    }

    #[tokio::test]
    async fn new_on_missing_file_returns_empty_library() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.json");

        let library = SkillLibrary::new(&path).await.unwrap();
        assert!(library.is_empty());
        assert_eq!(library.path(), path.as_path());
    }

    #[tokio::test]
    async fn concurrent_register_produces_consistent_state() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = Arc::new(SkillLibrary::new(&path).await.unwrap());

        let mut handles = Vec::new();
        for i in 0..16 {
            let lib = Arc::clone(&library);
            handles.push(tokio::spawn(async move {
                let s = Skill::new(format!("skill_{i:02}"), "summary", "template");
                lib.register(&s).await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }

        assert_eq!(library.len(), 16);
        let names: Vec<String> = library.list().into_iter().map(|s| s.name).collect();
        assert_eq!(names.first().map(String::as_str), Some("skill_00"));
        assert_eq!(names.last().map(String::as_str), Some("skill_15"));

        // Reload from disk to confirm every concurrent write reached the file.
        let reloaded = SkillLibrary::new(&path).await.unwrap();
        assert_eq!(reloaded.len(), 16);
    }

    #[tokio::test]
    async fn concurrent_record_use_tracks_every_outcome() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = Arc::new(SkillLibrary::new(&path).await.unwrap());
        library
            .register(&Skill::new("race", "sum", "tmpl"))
            .await
            .unwrap();

        let mut handles = Vec::new();
        for i in 0..20 {
            let lib = Arc::clone(&library);
            handles.push(tokio::spawn(async move {
                lib.record_use("race", i % 2 == 0).await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }
        let s = library.get("race").unwrap();
        assert_eq!(s.usage_count, 20);
        // 10 of 20 were successes → 0.5
        assert!((s.success_rate - 0.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn corrupt_file_surfaces_serde_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("broken.json");
        tokio::fs::write(&path, b"not valid json").await.unwrap();

        let err = SkillLibrary::new(&path)
            .await
            .expect_err("corrupt file should error");
        assert!(matches!(err, SkillLibraryError::Serde(_)));
    }

    #[tokio::test]
    async fn builder_helpers_compose() {
        let skill = Skill::new("builder", "sum", "tmpl")
            .with_required_tools(vec!["a", "b", "c"])
            .with_tags(vec!["x"])
            .with_examples(vec![("q", "r")]);
        assert_eq!(skill.required_tools.len(), 3);
        assert_eq!(skill.tags, vec!["x"]);
        assert_eq!(skill.example_inputs, vec!["q"]);
        assert_eq!(skill.example_outputs, vec!["r"]);
    }

    #[test]
    fn structured_skill_contract_fields_are_preserved() {
        let skill = Skill::new_structured(
            "skill.contract",
            "Contract Skill",
            "When the task requires a compile-test-fix loop",
            "Run cargo check, fix compiler errors, then run targeted tests",
            "The crate compiles and the failing test scope passes",
        )
        .with_task_categories(["implementation", "verification"])
        .with_source_episodes(["ep-001", "ep-002"]);

        assert_eq!(skill.id, "skill.contract");
        assert_eq!(skill.name, "Contract Skill");
        assert_eq!(
            skill.precondition,
            "When the task requires a compile-test-fix loop"
        );
        assert_eq!(
            skill.procedure,
            "Run cargo check, fix compiler errors, then run targeted tests"
        );
        assert_eq!(
            skill.postcondition,
            "The crate compiles and the failing test scope passes"
        );
        assert_eq!(
            skill.task_categories,
            vec!["implementation", "verification"]
        );
        assert_eq!(skill.source_episodes, vec!["ep-001", "ep-002"]);
        assert!(!skill.created_at.is_empty());
    }

    #[tokio::test]
    async fn record_validation_updates_confidence_and_category_query() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let implementation = Skill::new_structured(
            "impl.loop",
            "Compile Loop",
            "Compilation is broken",
            "Check compiler output, patch code, re-run cargo check",
            "Build passes",
        )
        .with_task_categories(["implementation"]);
        let docs = Skill::new_structured(
            "docs.skill",
            "Docs Skill",
            "Docs task",
            "Edit markdown",
            "Docs updated",
        )
        .with_task_categories(["docs"]);

        library.register(&implementation).await.unwrap();
        library.register(&docs).await.unwrap();

        library
            .record_validation("Compile Loop", true)
            .await
            .unwrap();
        library
            .record_validation("Compile Loop", true)
            .await
            .unwrap();
        library
            .record_validation("Compile Loop", false)
            .await
            .unwrap();

        let skill = library.get("Compile Loop").unwrap();
        assert_eq!(skill.validations, 2);
        assert_eq!(skill.failures, 1);
        assert!((skill.confidence - (2.0 / 3.0)).abs() < 1e-9);
        assert!(skill.last_validated_at.is_some());

        let queried = library.query_by_task_category("implementation");
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].name, "Compile Loop");
    }

    // ── Voyager extraction / selection tests (§16.3.2-16.3.4) ──────

    fn make_episode(
        task_id: &str,
        success: bool,
        iteration: u64,
        band: &str,
        tags: &[&str],
        category: &str,
    ) -> crate::episode_logger::Episode {
        let mut ep = crate::episode_logger::Episode::new("test-agent", task_id);
        ep.success = success;
        ep.extra
            .insert("iteration".into(), serde_json::json!(iteration));
        ep.extra
            .insert("complexity_band".into(), serde_json::json!(band));
        ep.extra.insert("task_tags".into(), serde_json::json!(tags));
        ep.extra
            .insert("task_category".into(), serde_json::json!(category));
        ep.extra
            .insert("files".into(), serde_json::json!(["src/main.rs"]));
        ep.extra
            .insert("role".into(), serde_json::json!("implementer"));
        ep.extra
            .insert("model".into(), serde_json::json!("claude-4"));
        ep.extra
            .insert("verbal_reflection".into(), serde_json::json!("worked well"));
        ep.extra.insert("score".into(), serde_json::json!(0.9));
        ep
    }

    #[tokio::test]
    async fn extract_from_successful_episode() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        let pg = TemplatePatternGenerator;

        let ep = make_episode("task-1", true, 1, "standard", &["rust", "async"], "backend");
        let skill = library.extract(&ep, &pg).await;
        assert!(skill.is_some());
        let skill = skill.unwrap();
        assert!(skill.name.starts_with("skill_"));
        assert!(!skill.pattern.is_empty());
        assert_eq!(skill.tags, vec!["rust", "async"]);
        assert_eq!(skill.task_category, "backend");
        assert_eq!(skill.task_categories, vec!["backend"]);
        assert!((skill.score - 0.9).abs() < f64::EPSILON);
        assert!(skill.first_seen.is_some());
        assert_eq!(skill.source_episodes.len(), 1);
        assert_eq!(library.len(), 1);
    }

    #[tokio::test]
    async fn extract_rejects_non_success() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        let pg = TemplatePatternGenerator;

        let ep = make_episode("task-1", false, 1, "standard", &["rust"], "backend");
        assert!(library.extract(&ep, &pg).await.is_none());
        assert_eq!(library.len(), 0);
    }

    #[tokio::test]
    async fn extract_rejects_iteration_gt_1() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        let pg = TemplatePatternGenerator;

        let ep = make_episode("task-1", true, 2, "standard", &["rust"], "backend");
        assert!(library.extract(&ep, &pg).await.is_none());
        assert_eq!(library.len(), 0);
    }

    #[tokio::test]
    async fn extract_rejects_trivial_complexity() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        let pg = TemplatePatternGenerator;

        let ep = make_episode("task-1", true, 1, "trivial", &["rust"], "backend");
        assert!(library.extract(&ep, &pg).await.is_none());

        let ep2 = make_episode("task-2", true, 1, "simple", &["rust"], "backend");
        assert!(library.extract(&ep2, &pg).await.is_none());
        assert_eq!(library.len(), 0);
    }

    #[tokio::test]
    async fn extract_dedup_by_tag_overlap() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        let pg = TemplatePatternGenerator;

        // First extraction succeeds
        let ep1 = make_episode(
            "task-1",
            true,
            1,
            "standard",
            &["rust", "async", "tokio"],
            "backend",
        );
        assert!(library.extract(&ep1, &pg).await.is_some());

        // 2/3 tags overlap = 66.7% < 70% — should succeed
        let mut ep2 = make_episode(
            "task-2",
            true,
            1,
            "standard",
            &["rust", "async", "serde"],
            "backend",
        );
        ep2.id = "ep_dedup_test_002".into();
        assert!(library.extract(&ep2, &pg).await.is_some());
        assert_eq!(library.len(), 2);

        // 3/3 tags overlap = 100% ≥ 70% + same category — rejected
        let mut ep3 = make_episode(
            "task-3",
            true,
            1,
            "standard",
            &["rust", "async", "tokio"],
            "backend",
        );
        ep3.id = "ep_dedup_test_003".into();
        assert!(library.extract(&ep3, &pg).await.is_none());
        assert_eq!(library.len(), 2);

        // Same tags but different category — should succeed (dedup requires
        // BOTH tag overlap ≥70% AND same category)
        let mut ep4 = make_episode(
            "task-4",
            true,
            1,
            "standard",
            &["rust", "async", "tokio"],
            "frontend",
        );
        ep4.id = "ep_dedup_test_004".into();
        assert!(library.extract(&ep4, &pg).await.is_some());
        assert_eq!(library.len(), 3);
    }

    #[tokio::test]
    async fn select_scoring_formula() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        // High score + validated
        let mut s1 = Skill::new("skill_a", "sum-a", "tmpl-a");
        s1.tags = vec!["rust".into(), "async".into()];
        s1.score = 0.9;
        s1.validated_count = 4; // bonus = 1 + 0.1*2 = 1.2
        s1.task_category = "backend".into();
        s1.files = vec!["src/main.rs".into()];
        library.register(&s1).await.unwrap();

        // Lower score, zero validated
        let mut s2 = Skill::new("skill_b", "sum-b", "tmpl-b");
        s2.tags = vec!["rust".into(), "async".into()];
        s2.score = 0.8;
        s2.validated_count = 0; // bonus = 1.0
        s2.task_category = "backend".into();
        library.register(&s2).await.unwrap();

        let query = SkillQuery {
            tags: vec!["rust".into(), "async".into()],
            category: None,
            files_hint: vec!["src/main.rs".into()],
        };

        let results = library.select(&query, 2);
        assert_eq!(results.len(), 2);
        // s1: 0.9 * (1.0+0.2) * 1.2 = 1.296
        // s2: 0.8 * 1.0 * 1.0 = 0.8
        assert_eq!(results[0].name, "skill_a");
        assert_eq!(results[1].name, "skill_b");
    }

    #[tokio::test]
    async fn select_empty_query_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let mut s = Skill::new("s", "sum", "tmpl");
        s.tags = vec!["rust".into()];
        s.score = 0.9;
        library.register(&s).await.unwrap();

        assert!(library.select(&SkillQuery::default(), 10).is_empty());
    }

    #[tokio::test]
    async fn select_limit_caps_output() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        for i in 0..5 {
            let mut s = Skill::new(format!("sk_{i}"), "sum", "tmpl");
            s.tags = vec!["rust".into()];
            s.score = 0.9;
            library.register(&s).await.unwrap();
        }

        let query = SkillQuery {
            tags: vec!["rust".into()],
            ..Default::default()
        };
        assert_eq!(library.select(&query, 2).len(), 2);
    }

    #[tokio::test]
    async fn query_maps_task_metadata_to_selection() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let mut matched = Skill::new("matched", "sum", "tmpl");
        matched.tags = vec!["resolve_symbols".into()];
        matched.task_category = "implementation".into();
        matched.files = vec!["src/lib.rs".into()];
        matched.score = 0.9;
        library.register(&matched).await.unwrap();

        let mut other = Skill::new("other", "sum", "tmpl");
        other.tags = vec!["unrelated".into()];
        other.task_category = "docs".into();
        other.files = vec!["README.md".into()];
        other.score = 0.9;
        library.register(&other).await.unwrap();

        let results = library.query(
            &["src/lib.rs".into()],
            "implementation",
            &["resolve_symbols".into()],
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "matched");
    }

    #[tokio::test]
    async fn record_outcome_updates_counters() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();
        library
            .register(&Skill::new("sk", "sum", "tmpl"))
            .await
            .unwrap();

        library.record_outcome("sk", true).await.unwrap();
        library.record_outcome("sk", false).await.unwrap();
        library.record_outcome("sk", true).await.unwrap();

        let s = library.get("sk").unwrap();
        assert_eq!(s.match_count, 3);
        assert_eq!(s.validated_count, 2);
        assert!(s.last_matched.is_some());
    }

    #[tokio::test]
    async fn extract_skill_persists_to_disk() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        let request = SkillExtractionRequest::new(
            vec!["src/lib.rs".into()],
            "implementation",
            vec!["extract_skill".into()],
            "gpt-5".to_string(),
            "prompt-hash-1234".to_string(),
            vec![
                SkillGateResult::new("compile", true, 0.98),
                SkillGateResult::new("test", true, 0.93),
            ],
        );

        let extracted = library.extract_skill(request).await;
        assert!(extracted.is_some());

        let reloaded = SkillLibrary::new(&path).await.unwrap();
        assert_eq!(reloaded.len(), 1);
        assert!(
            reloaded
                .get(extracted.as_ref().unwrap().name.as_str())
                .is_some()
        );
    }

    #[tokio::test]
    async fn prune_stale_removes_old_keeps_minimum() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("skills.json");
        let library = SkillLibrary::new(&path).await.unwrap();

        // Register 12 skills all with old first_seen, no last_matched
        let old_time = Utc::now() - chrono::Duration::days(90);
        for i in 0..12u32 {
            let mut s = Skill::new(format!("skill_{i:02}"), "sum", "tmpl");
            s.first_seen = Some(old_time + chrono::Duration::hours(i64::from(i)));
            library.register(&s).await.unwrap();
        }
        assert_eq!(library.len(), 12);

        // All 12 are stale (>60 days) but keep at least 10
        let pruned = library.prune_stale(60).await;
        assert_eq!(pruned, 2);
        assert_eq!(library.len(), 10);

        // The two oldest (skill_00, skill_01) were removed
        assert!(library.get("skill_00").is_none());
        assert!(library.get("skill_01").is_none());
        assert!(library.get("skill_02").is_some());
    }

    #[tokio::test]
    async fn template_pattern_generator_produces_nonempty() {
        let pg = TemplatePatternGenerator;
        let ep = make_episode("task-1", true, 1, "standard", &["rust"], "backend");
        let pattern = pg.generate(&ep);
        assert!(!pattern.is_empty());
        assert!(pattern.contains("Edit files:"));
        assert!(pattern.contains("Agent role:"));
        assert!(pattern.contains("Tags:"));
        assert!(pattern.contains("Approach summary:"));
    }

    #[tokio::test]
    async fn skill_extraction_extracts_candidates_from_successful_gate_passed_episodes() {
        let extractor = NlToFormatConverter::new();
        let response = extractor.wrap(
            r#"{
                "title": "Rust compile-test repair",
                "task_category": "implementation",
                "complexity": "complex",
                "files_involved": ["src/lib.rs", "src/parser.rs"],
                "tool_sequence_summary": "Read src/lib.rs, Edit src/parser.rs, Bash cargo test -p roko-learn",
                "precondition": "Implementation task touching Rust parser files with failing checks",
                "postcondition": "Compile and test gates pass with parser changes applied",
                "skill_description": "Inspect the affected Rust files, patch the parser, and rerun targeted tests until the gates pass.",
                "output_summary": "Parser update landed and the targeted tests passed"
            }"#,
        );
        let agent = MockAgent::reply(response);

        let mut passing = make_episode(
            "task-1",
            true,
            1,
            "complex",
            &["rust", "parser"],
            "implementation",
        );
        passing.episode_id = "ep-success-1".into();
        passing.reasoning_summary = Some("Patched the parser and reran tests".into());
        passing.gate_verdicts = vec![
            crate::episode_logger::GateVerdict::new("compile", true),
            crate::episode_logger::GateVerdict::new("test", true),
        ];
        passing.extra.insert(
            "tool_calls".into(),
            serde_json::json!(["Read", "Edit", "Bash"]),
        );
        passing.extra.insert(
            "files".into(),
            serde_json::json!(["src/lib.rs", "src/parser.rs"]),
        );

        let mut failing = make_episode("task-2", false, 1, "complex", &["rust"], "implementation");
        failing.episode_id = "ep-fail-1".into();
        failing.gate_verdicts = vec![crate::episode_logger::GateVerdict::new("compile", false)];

        let candidates = extract_skill_candidates(&[passing, failing], &agent).await;
        assert_eq!(candidates.len(), 1);

        let candidate = &candidates[0];
        assert_eq!(candidate.source_episode_id, "ep-success-1");
        assert_eq!(candidate.task_category, "implementation");
        assert_eq!(candidate.complexity, "complex");
        assert_eq!(
            candidate.files_involved,
            vec!["src/lib.rs".to_string(), "src/parser.rs".to_string()]
        );
        assert_eq!(candidate.gate_results.len(), 2);
        assert!(candidate.gate_results.iter().all(|result| result.passed));
        assert!(candidate.skill_description.contains("patch the parser"));
    }
}
