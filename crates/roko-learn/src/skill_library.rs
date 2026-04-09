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
use serde::{Deserialize, Serialize};
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
    /// Unique, human-readable identifier for the skill (`snake_case` preferred).
    pub name: String,
    /// One-line description of what the skill does.
    pub summary: String,
    /// Prompt template injected when the skill is selected.
    pub prompt_template: String,
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
        Self {
            name: name.into(),
            summary: summary.into(),
            prompt_template: prompt_template.into(),
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
                list.into_iter().map(|s| (s.name.clone(), s)).collect()
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
        {
            let mut guard = self.skills.write();
            if guard.contains_key(&skill.name) {
                return Err(SkillLibraryError::Duplicate(skill.name.clone()));
            }
            guard.insert(skill.name.clone(), skill.clone());
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

    /// Alias for [`SkillLibrary::select`] to match the public API used by the
    /// orchestration layer docs.
    pub fn query(&self, query: &SkillQuery, limit: usize) -> Vec<Skill> {
        self.select(query, limit)
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
                    format!("Extracted from a successful {} task using {}.", task_tier, model),
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
            if existing.task_category == category {
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
                    if !skill.task_category.is_empty() && skill.task_category != *cat {
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
    if out.is_empty() { "unknown".to_string() } else { out }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
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
        assert_eq!(fetched.required_tools, vec!["read", "write"]);
        assert_eq!(fetched.example_inputs.len(), 2);
        assert_eq!(fetched.example_outputs.len(), 2);
        assert_eq!(fetched.usage_count, 0);
        assert!((fetched.success_rate - 0.0).abs() < f64::EPSILON);
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
        assert!((skill.score - 0.9).abs() < f64::EPSILON);
        assert!(skill.first_seen.is_some());
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
}
