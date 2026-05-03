//! PromptAssemblyService — concrete implementation of `PromptAssembler`.
//!
//! Wraps the existing `SystemPromptBuilder` with role resolution, convention
//! detection, and gate feedback injection.

use async_trait::async_trait;
use roko_core::foundation::{PromptAssembler, PromptSpec};
use roko_core::{AgentRole, Result};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::playbook::{PlaybookStore, QueryContext};
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeTier, NeuroStore};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::conventions::detect_conventions;
use crate::role_prompts::role_identity_for;
use crate::system_prompt_builder::SystemPromptBuilder;
use crate::token_counter::TokenCounter;

const SOURCE_SAMPLE_LIMIT: usize = 12;
const WORKSPACE_MAP_LINE_LIMIT: usize = 200;
/// Maximum recursion depth for source directory scanning (§17.1).
const SOURCE_SCAN_MAX_DEPTH: usize = 5;
/// Maximum files to enumerate before cutting off (§17.1).
const SOURCE_SCAN_MAX_FILES: usize = 500;

// TODO(converge): roko_neuro::NeuroStore currently has a `Sized` supertrait,
// so it cannot be stored directly as `dyn NeuroStore`. Keep this object-safe
// adapter local until roko-neuro exposes an object-safe query trait.
trait PromptKnowledgeStore {
    fn query(&self, topic: &str, limit: usize) -> Vec<KnowledgeEntry>;
}

impl<T> PromptKnowledgeStore for T
where
    T: NeuroStore + Send + Sync,
{
    fn query(&self, topic: &str, limit: usize) -> Vec<KnowledgeEntry> {
        NeuroStore::query(self, topic, limit).unwrap_or_default()
    }
}

/// Service that assembles system prompts via the 9-layer `SystemPromptBuilder`.
///
/// This is the canonical way to build prompts in the workflow engine. It:
/// - Resolves role identity from role name
/// - Detects project conventions from the working directory
/// - Injects gate feedback from prior iterations
/// - Applies anti-patterns
pub struct PromptAssemblyService {
    /// Default conventions text used when workdir detection is unavailable.
    default_conventions: Option<String>,
    /// Static domain knowledge injected into layer 3.
    domain_context: Option<String>,
    /// Optional knowledge store for injecting relevant knowledge into prompts.
    knowledge_store: Option<Arc<dyn PromptKnowledgeStore + Send + Sync>>,
    /// IDs of knowledge entries used in the most recent assembly.
    /// Cleared on each `assemble()` call.
    last_knowledge_ids: Mutex<Vec<String>>,
    /// Prompt section IDs used in the most recent assembly.
    /// Cleared on each `assemble()` call.
    last_prompt_section_ids: Mutex<Vec<String>>,
    /// Path to append-only episode history used for recent execution context.
    episodes_path: Option<PathBuf>,
    /// Learned playbook source used for relevant technique injection.
    playbook_store: Option<Arc<PlaybookStore>>,
    /// Tool usage guidance injected into layer 5.
    tool_instructions: Option<String>,
    /// Optional token cap passed through to the system prompt builder.
    token_budget: Option<usize>,
    /// Per-section effectiveness scores used to skip low-value sections and scale budget.
    section_effectiveness: Option<HashMap<String, f64>>,
}

impl PromptAssemblyService {
    /// Create a new `PromptAssemblyService`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            default_conventions: None,
            domain_context: None,
            knowledge_store: None,
            last_knowledge_ids: Mutex::new(Vec::new()),
            last_prompt_section_ids: Mutex::new(Vec::new()),
            episodes_path: None,
            playbook_store: None,
            tool_instructions: None,
            token_budget: None,
            section_effectiveness: None,
        }
    }

    /// Create with default conventions text.
    #[must_use]
    pub fn with_conventions(mut self, conventions: String) -> Self {
        self.default_conventions = Some(conventions);
        self
    }

    /// Add static domain context for layer 3.
    #[must_use]
    pub fn with_domain_context(mut self, domain: String) -> Self {
        self.domain_context = Some(domain);
        self
    }

    /// Attach a knowledge store for prompt enrichment.
    #[must_use]
    pub fn with_knowledge_store<T>(mut self, store: Arc<T>) -> Self
    where
        T: NeuroStore + Send + Sync + 'static,
    {
        self.knowledge_store = Some(store);
        self
    }

    /// Return the knowledge entry IDs used in the most recent assembly.
    pub fn last_knowledge_ids(&self) -> Vec<String> {
        self.last_knowledge_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Return the prompt section IDs used in the most recent assembly.
    pub fn last_prompt_section_ids(&self) -> Vec<String> {
        self.last_prompt_section_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Add recent episode history for layer 3b context.
    #[must_use]
    pub fn with_episode_context(mut self, episodes_path: PathBuf) -> Self {
        self.episodes_path = Some(episodes_path);
        self
    }

    /// Add recent episode history for layer 3b context.
    #[must_use]
    pub fn with_episodes(self, episodes_path: PathBuf) -> Self {
        self.with_episode_context(episodes_path)
    }

    /// Add a learned playbook store for layer 6 relevant techniques.
    #[must_use]
    pub fn with_playbook_context(mut self, store: Arc<PlaybookStore>) -> Self {
        self.playbook_store = Some(store);
        self
    }

    /// Add a learned playbook store for layer 6 relevant techniques.
    #[must_use]
    pub fn with_playbooks(self, store: Arc<PlaybookStore>) -> Self {
        self.with_playbook_context(store)
    }

    /// Add tool usage instructions for layer 5.
    #[must_use]
    pub fn with_tool_instructions(mut self, tools: String) -> Self {
        self.tool_instructions = Some(tools);
        self
    }

    /// Set a token budget for assembled prompts.
    #[must_use]
    pub const fn with_token_budget(mut self, budget: usize) -> Self {
        self.token_budget = Some(budget);
        self
    }

    /// Set per-section effectiveness scores.
    ///
    /// Keys are section names matching the 9 builder layers:
    /// `"identity"`, `"conventions"`, `"domain"`, `"context"`, `"task"`,
    /// `"gate_feedback"`, `"tools"`, `"playbooks"`, `"anti_patterns"`.
    ///
    /// Values are effectiveness scores in `[0.0, 1.0]`. Sections with
    /// score < 0.1 are skipped entirely. Scores are used to scale the
    /// section's share of the token budget.
    #[must_use]
    pub fn with_section_effectiveness(mut self, scores: HashMap<String, f64>) -> Self {
        self.section_effectiveness = Some(normalize_section_effectiveness_scores(scores));
        self
    }

    fn should_include(&self, section: &str) -> bool {
        self.section_effectiveness
            .as_ref()
            .and_then(|scores| scores.get(section))
            .is_none_or(|&score| score >= 0.1)
    }

    /// Returns (sum of included section scores, total section count).
    fn effective_budget_ratio(&self) -> (f64, f64) {
        let sections = [
            "identity",
            "conventions",
            "domain",
            "context",
            "task",
            "gate_feedback",
            "tools",
            "playbooks",
            "anti_patterns",
        ];
        let total = sections.len() as f64;
        let included = sections
            .iter()
            .filter(|section| self.should_include(section))
            .map(|section| {
                self.section_effectiveness
                    .as_ref()
                    .and_then(|scores| scores.get(*section).copied())
                    .unwrap_or(1.0)
            })
            .sum();
        (included, total)
    }

    /// Query knowledge store for relevant technique insights.
    fn query_techniques(&self, task: &str) -> Vec<(String, String)> {
        let Some(store) = self.knowledge_store.as_ref() else {
            return Vec::new();
        };
        let entries = store.query(task, 5);
        entries
            .into_iter()
            .filter(|e| e.kind != KnowledgeKind::AntiKnowledge)
            .filter(|e| e.confidence >= 0.3)
            .map(|e| (e.id, e.content))
            .collect()
    }

    /// Query knowledge store for anti-knowledge warnings.
    fn query_anti_patterns(&self, task: &str) -> Vec<(String, String)> {
        let Some(store) = self.knowledge_store.as_ref() else {
            return Vec::new();
        };
        let entries = store.query(&format!("{task} warning anti-pattern"), 5);
        entries
            .into_iter()
            .filter(|e| e.kind == KnowledgeKind::AntiKnowledge)
            .filter(|e| e.confidence >= 0.2)
            .map(|e| (e.id, format!("WARNING: {}", e.content)))
            .collect()
    }

    fn clear_last_knowledge_ids(&self) {
        self.last_knowledge_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn clear_last_prompt_section_ids(&self) {
        self.last_prompt_section_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }

    fn record_knowledge_ids(&self, ids: Vec<String>) {
        if ids.is_empty() {
            return;
        }

        let mut last_ids = self
            .last_knowledge_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for id in ids {
            if !id.is_empty() && !last_ids.contains(&id) {
                last_ids.push(id);
            }
        }
    }

    fn record_prompt_section_id(&self, id: &str) {
        if id.trim().is_empty() {
            return;
        }

        let mut last_ids = self
            .last_prompt_section_ids
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !last_ids.iter().any(|existing| existing == id) {
            last_ids.push(id.to_string());
        }
    }
}

fn normalize_section_effectiveness_scores(scores: HashMap<String, f64>) -> HashMap<String, f64> {
    scores
        .into_iter()
        .map(|(section, score)| (canonical_section_key(&section).to_string(), score))
        .collect()
}

fn canonical_section_key(section: &str) -> &str {
    match section {
        "role_identity" => "identity",
        "domain_context" => "domain",
        "context_layer" => "context",
        "task_context" => "task",
        "tool_instructions" => "tools",
        "relevant_techniques" => "playbooks",
        other => other,
    }
}

impl Default for PromptAssemblyService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptAssembler for PromptAssemblyService {
    async fn assemble(&self, spec: PromptSpec) -> Result<String> {
        self.clear_last_knowledge_ids();
        self.clear_last_prompt_section_ids();

        let role = resolve_role(spec.role.as_deref());
        let identity = role_identity_for(role);

        let mut builder = SystemPromptBuilder::new(identity);
        builder = builder.with_cache_markers();
        self.record_prompt_section_id("role_identity");
        // SystemPromptBuilder's current API is no-arg; this is equivalent to with_cache_markers(true).

        if self.should_include("conventions")
            && let Some(conventions) =
                conventions_for_spec(&spec, self.default_conventions.as_deref())
        {
            builder = builder.with_conventions(conventions);
            self.record_prompt_section_id("conventions");
        }

        let mut context_blocks = Vec::new();
        if self.should_include("context")
            && let Some(ref episodes_path) = self.episodes_path
        {
            // §17.6: Use lossy read to tolerate truncated crash lines rather
            // than failing the entire episode context on a single parse error.
            if let Ok(episodes) = EpisodeLogger::read_all_lossy(episodes_path).await {
                let recent = episodes.into_iter().rev().take(5).collect::<Vec<_>>();
                if !recent.is_empty() {
                    context_blocks.push(format_episode_context(&recent));
                    self.record_prompt_section_id("context_layer");
                }
            }
        }

        if self.should_include("playbooks")
            && let Some(ref store) = self.playbook_store
        {
            let task_text = spec
                .task
                .as_deref()
                .map(str::trim)
                .filter(|task| !task.is_empty())
                .unwrap_or_else(|| role.label());
            let task_id = first_chars(task_text, 80);
            let ctx = QueryContext {
                task_id: task_id.to_string(),
                task_title: task_text.to_string(),
                task_body: task_text.to_string(),
                role: role.label().to_string(),
                recent_episodes: 0,
                max_results: 3,
            };
            if let Ok(playbooks) = store.query(&ctx).await {
                if !playbooks.is_empty() {
                    builder = builder.with_playbooks(&playbooks);
                    self.record_prompt_section_id("relevant_techniques");
                }
            }
        }

        if self.should_include("domain") {
            if let Some((domain, ids)) = domain_context_for_spec(
                &spec,
                self.domain_context.as_deref(),
                self.knowledge_store.as_deref(),
            ) {
                self.record_knowledge_ids(ids);
                builder = builder.with_domain(domain);
                self.record_prompt_section_id("domain_context");
            }
        }

        if self.should_include("context")
            && let Some(workspace_map) = workspace_map_for_spec(&spec)
        {
            context_blocks.push(workspace_map);
            self.record_prompt_section_id("context_layer");
        }

        if self.should_include("task")
            && let Some(task) = spec.task.as_ref()
        {
            builder = builder.with_task(task.clone());
            self.record_prompt_section_id("task_context");
        }

        if self.should_include("tools")
            && let Some(tools) = &self.tool_instructions
        {
            builder = builder.with_tools(tools.clone());
            self.record_prompt_section_id("tool_instructions");
        }

        if self.should_include("gate_feedback") {
            for feedback in &spec.gate_feedback {
                builder = builder.with_gate_feedback_text(feedback);
                self.record_prompt_section_id("gate_feedback");
            }
        }

        let mut anti_patterns = spec.anti_patterns;
        if self.knowledge_store.is_some()
            && let Some(task_text) = spec.task.as_deref()
        {
            let technique_entries = self.query_techniques(task_text);
            let warning_entries = self.query_anti_patterns(task_text);
            let techniques = technique_entries
                .iter()
                .map(|(_, content)| content.clone())
                .collect::<Vec<_>>();
            let warnings = warning_entries
                .iter()
                .map(|(_, content)| content.clone())
                .collect::<Vec<_>>();

            if self.should_include("context") && !techniques.is_empty() {
                self.record_knowledge_ids(
                    technique_entries
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect::<Vec<_>>(),
                );
                context_blocks.push(format_techniques_section(&techniques));
                self.record_prompt_section_id("context_layer");
            }
            if self.should_include("anti_patterns") && !warnings.is_empty() {
                self.record_knowledge_ids(
                    warning_entries
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect::<Vec<_>>(),
                );
                anti_patterns.extend(warnings.clone());
            }

            tracing::debug!(
                techniques = %techniques.len(),
                warnings = %warnings.len(),
                "injected knowledge into prompt assembly"
            );
        }

        if self.should_include("context") && !context_blocks.is_empty() {
            builder = builder.with_context(context_blocks.join("\n\n"));
        }

        if self.should_include("anti_patterns") && !anti_patterns.is_empty() {
            builder = builder.with_anti_patterns(anti_patterns);
            self.record_prompt_section_id("anti_patterns");
        }

        if let Some(base_budget) = self.token_budget {
            let (included_weight, total) = self.effective_budget_ratio();
            let scaled = ((base_budget as f64) * (included_weight / total)).ceil() as usize;
            builder = builder.with_token_budget(scaled.max(256));
            let counter = TokenCounter::Heuristic {
                // §17.2: Conservative estimate for code-heavy prompts.
                chars_per_token: 3.5,
            };
            return Ok(builder.build_with_counter(&counter));
        }

        Ok(builder.build())
    }

    fn last_prompt_section_ids(&self) -> Vec<String> {
        Self::last_prompt_section_ids(self)
    }

    fn last_knowledge_ids(&self) -> Vec<String> {
        Self::last_knowledge_ids(self)
    }
}

fn resolve_role(role: Option<&str>) -> AgentRole {
    let Some(role) = role.map(str::trim).filter(|role| !role.is_empty()) else {
        return AgentRole::Implementer;
    };
    let normalized = role.to_ascii_lowercase().replace('_', "-");

    std::iter::once(AgentRole::Conductor)
        .chain(AgentRole::ALL_AGENTS)
        .find(|candidate| candidate.label() == normalized)
        .or_else(|| serde_json::from_value(serde_json::Value::String(normalized)).ok())
        .unwrap_or(AgentRole::Implementer)
}

fn conventions_for_spec(spec: &PromptSpec, default_conventions: Option<&str>) -> Option<String> {
    spec.workdir
        .as_deref()
        .and_then(detect_workdir_conventions)
        .or_else(|| default_conventions.map(ToOwned::to_owned))
}

fn workspace_map_for_spec(spec: &PromptSpec) -> Option<String> {
    let workdir = spec.workdir.as_deref()?;
    let (_, file_listing) = collect_source_context(workdir);
    workspace_map_from_file_listing(&file_listing)
}

fn domain_context_for_spec(
    spec: &PromptSpec,
    static_domain_context: Option<&str>,
    knowledge_store: Option<&(dyn PromptKnowledgeStore + Send + Sync)>,
) -> Option<(String, Vec<String>)> {
    let knowledge_text = knowledge_store.and_then(|store| relevant_knowledge_for_spec(store, spec));

    match (static_domain_context, knowledge_text) {
        (Some(existing), Some((knowledge, ids))) if !existing.trim().is_empty() => {
            Some((format!("{existing}\n\n{knowledge}"), ids))
        }
        (Some(existing), _) if !existing.trim().is_empty() => Some((existing.to_owned(), vec![])),
        (_, Some((knowledge, ids))) => Some((knowledge, ids)),
        _ => None,
    }
}

fn relevant_knowledge_for_spec(
    store: &(dyn PromptKnowledgeStore + Send + Sync),
    spec: &PromptSpec,
) -> Option<(String, Vec<String>)> {
    let topic = spec
        .task
        .as_deref()
        .map(|task| first_chars(task, 200))
        .or(spec.role.as_deref())?;

    let entries = store.query(topic, 5);
    let facts = entries
        .iter()
        .filter(|entry| entry.confidence >= 0.5 && !matches!(entry.tier, KnowledgeTier::Transient))
        .collect::<Vec<_>>();

    (!facts.is_empty()).then(|| {
        let ids = facts.iter().map(|entry| entry.id.clone()).collect();
        (format_knowledge_section(&facts), ids)
    })
}

fn first_chars(value: &str, max_chars: usize) -> &str {
    value
        .char_indices()
        .nth(max_chars)
        .map_or(value, |(index, _)| &value[..index])
}

fn format_knowledge_section(entries: &[&KnowledgeEntry]) -> String {
    let lines = entries
        .iter()
        .map(|entry| format!("- [{}] {}", knowledge_kind_label(entry.kind), entry.content))
        .collect::<Vec<_>>()
        .join("\n");

    format!("## Relevant Knowledge\n\n{lines}")
}

fn format_techniques_section(techniques: &[String]) -> String {
    let lines = techniques
        .iter()
        .map(|technique| format!("- {technique}"))
        .collect::<Vec<_>>()
        .join("\n");

    // §17.3: Use distinct heading to avoid collision with format_knowledge_section.
    format!("## Relevant Techniques\n\n{lines}")
}

fn knowledge_kind_label(kind: KnowledgeKind) -> &'static str {
    match kind {
        KnowledgeKind::Insight => "Fact",
        KnowledgeKind::Heuristic | KnowledgeKind::StrategyFragment => "Pattern",
        KnowledgeKind::AntiKnowledge => "Antipattern",
        KnowledgeKind::Warning => "Warning",
        KnowledgeKind::CausalLink => "Reference",
    }
}

fn workspace_map_from_file_listing(file_listing: &[String]) -> Option<String> {
    if file_listing.is_empty() {
        return None;
    }

    let lines = file_listing
        .iter()
        .take(WORKSPACE_MAP_LINE_LIMIT.saturating_sub(1))
        .map(|path| format!("- {path}"))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!("## Workspace Map\n{lines}"))
}

fn format_episode_context(episodes: &[Episode]) -> String {
    let lines = episodes
        .iter()
        .map(|episode| {
            let status = if episode.success { "SUCCESS" } else { "FAILED" };
            let task = if episode.task_id.trim().is_empty() {
                "-"
            } else {
                episode.task_id.trim()
            };
            let model = if episode.model.trim().is_empty() {
                "-"
            } else {
                episode.model.trim()
            };
            let tokens = episode.tokens_used.max(
                episode
                    .usage
                    .input_tokens
                    .saturating_add(episode.usage.output_tokens),
            );
            let failed_gate = episode
                .gate_verdicts
                .iter()
                .find(|verdict| !verdict.passed && !verdict.gate.trim().is_empty())
                .map(|verdict| verdict.gate.trim());

            let mut line = format!(
                "- [{status}] task={task} model={model} duration={:.1}s tokens={tokens}",
                episode.duration_secs
            );
            if let Some(gate) = failed_gate {
                line.push_str(&format!(" gate={gate}"));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!("## Recent Execution History (last 5 runs)\n{lines}")
}

fn detect_workdir_conventions(workdir: &Path) -> Option<String> {
    let cargo_toml = read_to_string_if_exists(&workdir.join("Cargo.toml")).unwrap_or_default();
    let (source_samples, file_listing) = collect_source_context(workdir);

    if cargo_toml.is_empty() && source_samples.is_empty() && file_listing.is_empty() {
        return None;
    }

    let source_refs = source_samples
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let file_refs = file_listing.iter().map(String::as_str).collect::<Vec<_>>();
    let conventions = detect_conventions(&cargo_toml, &source_refs, &file_refs);
    let fragment = conventions.to_prompt_fragment();

    (!fragment.trim().is_empty()).then_some(fragment)
}

fn collect_source_context(workdir: &Path) -> (Vec<String>, Vec<String>) {
    let mut source_samples = Vec::new();
    let mut file_listing = Vec::new();
    collect_source_context_from(
        &workdir.join("src"),
        workdir,
        &mut source_samples,
        &mut file_listing,
        0,
    );
    (source_samples, file_listing)
}

fn collect_source_context_from(
    dir: &Path,
    root: &Path,
    source_samples: &mut Vec<String>,
    file_listing: &mut Vec<String>,
    depth: usize,
) {
    if depth > SOURCE_SCAN_MAX_DEPTH || file_listing.len() >= SOURCE_SCAN_MAX_FILES {
        return;
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if file_listing.len() >= SOURCE_SCAN_MAX_FILES {
            return;
        }

        let path = entry.path();
        if path.is_dir() {
            collect_source_context_from(&path, root, source_samples, file_listing, depth + 1);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        if let Some(relative) = relative_path_string(&path, root) {
            file_listing.push(relative);
        }

        if source_samples.len() < SOURCE_SAMPLE_LIMIT {
            if let Some(source) = read_to_string_if_exists(&path) {
                source_samples.push(source);
            }
        }
    }
}

fn read_to_string_if_exists(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn relative_path_string(path: &Path, root: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok().unwrap_or(path);
    path_to_string(relative)
}

fn path_to_string(path: &Path) -> Option<String> {
    path.to_str()
        .map(str::to_owned)
        .or_else(|| Some(path.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_neuro::KnowledgeStore;

    #[tokio::test]
    async fn basic_assembly() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the login bug".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn assembly_with_gate_feedback() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the build".into()),
                gate_feedback: vec!["error[E0308]: mismatched types".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[tokio::test]
    async fn default_role_is_implementer() {
        let svc = PromptAssemblyService::new();
        let prompt = svc
            .assemble(PromptSpec {
                task: Some("Do something".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
    }

    #[test]
    fn resolves_role_labels_and_serde_names() {
        assert_eq!(
            resolve_role(Some("quick-reviewer")),
            AgentRole::QuickReviewer
        );
        assert_eq!(
            resolve_role(Some("quick_reviewer")),
            AgentRole::QuickReviewer
        );
        assert_eq!(
            resolve_role(Some("dep-validator")),
            AgentRole::DependencyValidator
        );
        assert_eq!(resolve_role(Some("unknown")), AgentRole::Implementer);
    }

    #[test]
    fn uses_default_conventions_without_workdir() {
        let spec = PromptSpec::default();
        assert_eq!(
            conventions_for_spec(&spec, Some("Use workspace conventions")),
            Some("Use workspace conventions".to_string())
        );
    }

    #[tokio::test]
    async fn assembly_includes_domain_context() {
        let svc = PromptAssemblyService::new()
            .with_domain_context("DeFi context: Uniswap v4 hooks".into());
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Implement hook routing".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("DeFi context: Uniswap v4 hooks"));
    }

    #[tokio::test]
    async fn knowledge_store_facts_injected_into_domain_layer() {
        let tempdir = tempfile::tempdir().unwrap();
        let store = Arc::new(KnowledgeStore::new(
            tempdir.path().join("neuro").join("knowledge.jsonl"),
        ));
        store
            .add(KnowledgeEntry {
                id: "k-rate-limit-fact".into(),
                kind: KnowledgeKind::Insight,
                content: "The rate limiter uses token buckets for burst control.".into(),
                confidence: 0.9,
                tags: vec!["rate".into(), "limiter".into()],
                tier: KnowledgeTier::Consolidated,
                ..KnowledgeEntry::default()
            })
            .unwrap();

        let svc = PromptAssemblyService::new().with_knowledge_store(store);
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the rate limiter".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("## Domain Context"));
        assert!(prompt.contains("## Relevant Knowledge"));
        assert!(prompt.contains("The rate limiter uses token buckets for burst control."));
        assert_eq!(
            svc.last_knowledge_ids(),
            vec!["k-rate-limit-fact".to_string()]
        );
        assert!(
            svc.last_prompt_section_ids()
                .contains(&"domain_context".to_string())
        );
    }

    #[tokio::test]
    async fn knowledge_store_skips_low_confidence_entries() {
        let tempdir = tempfile::tempdir().unwrap();
        let store = Arc::new(KnowledgeStore::new(
            tempdir.path().join("neuro").join("knowledge.jsonl"),
        ));
        store
            .add(KnowledgeEntry {
                id: "k-low-confidence".into(),
                kind: KnowledgeKind::Insight,
                content: "Low confidence limiter advice should stay out.".into(),
                confidence: 0.1,
                tags: vec!["rate".into(), "limiter".into()],
                tier: KnowledgeTier::Consolidated,
                ..KnowledgeEntry::default()
            })
            .unwrap();

        let svc = PromptAssemblyService::new().with_knowledge_store(store);
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix the rate limiter".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.contains("Low confidence limiter advice should stay out."));
    }

    #[tokio::test]
    async fn assembly_includes_episode_context() {
        let tempdir = tempfile::tempdir().unwrap();
        let episodes_path = tempdir.path().join("episodes.jsonl");
        let logger = EpisodeLogger::new(&episodes_path);

        for index in 0..3 {
            let mut episode = Episode::new("implementer", format!("T-04{index}"));
            episode.success = index != 1;
            episode.model = "sonnet".into();
            episode.duration_secs = 12.0 + f64::from(index);
            episode.usage = roko_learn::episode_logger::Usage::tokens(1_000, 500);
            if !episode.success {
                episode
                    .gate_verdicts
                    .push(roko_learn::episode_logger::GateVerdict::new(
                        "compile", false,
                    ));
            }
            logger.append(&episode).await.unwrap();
        }

        let svc = PromptAssemblyService::new().with_episode_context(episodes_path);
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix recent regressions".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("Recent Execution History"));
        assert!(prompt.contains("gate=compile"));
    }

    #[tokio::test]
    async fn assembly_includes_playbook_context() {
        let tempdir = tempfile::tempdir().unwrap();
        let store = Arc::new(PlaybookStore::new(tempdir.path().join("playbooks")));
        let mut playbook =
            roko_learn::playbook::Playbook::new("fix-compile", "Fix Rust compile failures");
        playbook.success_count = 2;
        playbook.steps.push(roko_learn::playbook::PlaybookStep::new(
            0,
            "Run cargo check and address the first compiler diagnostic",
            "run_command",
            vec!["compile_ok".into()],
        ));
        store.save(&playbook).await.unwrap();

        let svc = PromptAssemblyService::new().with_playbook_context(Arc::clone(&store));
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Fix compile failures in the Rust crate".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.is_empty());
        assert!(prompt.contains("Relevant Techniques"));
        assert!(prompt.contains("Fix Rust compile failures"));
    }

    #[tokio::test]
    async fn assembly_includes_tool_instructions() {
        let svc = PromptAssemblyService::new()
            .with_tool_instructions("Use cargo check for Rust verification".into());
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("Verify the crate".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("## Tool Instructions"));
        assert!(prompt.contains("Use cargo check for Rust verification"));
        assert!(
            svc.last_prompt_section_ids()
                .contains(&"tool_instructions".to_string())
        );
    }

    #[tokio::test]
    async fn effectiveness_skips_low_scoring_sections() {
        let scores = HashMap::from([
            ("conventions".to_string(), 0.05),
            ("anti_patterns".to_string(), 0.0),
        ]);
        let svc = PromptAssemblyService::new()
            .with_conventions("VISIBLE_CONVENTIONS".into())
            .with_section_effectiveness(scores);
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                anti_patterns: vec!["VISIBLE_ANTI_PATTERN".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(!prompt.contains("VISIBLE_CONVENTIONS"));
        assert!(!prompt.contains("VISIBLE_ANTI_PATTERN"));
        assert!(prompt.contains("You are the Implementer"));
    }

    #[tokio::test]
    async fn effectiveness_includes_high_scoring_sections() {
        let scores = HashMap::from([("task".to_string(), 0.9)]);
        let svc = PromptAssemblyService::new().with_section_effectiveness(scores);
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("BUILD_THE_FEATURE".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("BUILD_THE_FEATURE"));
    }

    #[tokio::test]
    async fn no_effectiveness_includes_everything() {
        let svc = PromptAssemblyService::new().with_conventions("VISIBLE_CONVENTIONS".into());
        let prompt = svc
            .assemble(PromptSpec {
                role: Some("implementer".into()),
                task: Some("BUILD_THE_FEATURE".into()),
                anti_patterns: vec!["VISIBLE_ANTI_PATTERN".into()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(prompt.contains("VISIBLE_CONVENTIONS"));
        assert!(prompt.contains("BUILD_THE_FEATURE"));
        assert!(prompt.contains("VISIBLE_ANTI_PATTERN"));
    }

    #[test]
    fn workspace_map_is_capped_to_two_hundred_lines() {
        let listing = (0..250)
            .map(|index| format!("src/file_{index}.rs"))
            .collect::<Vec<_>>();
        let map = workspace_map_from_file_listing(&listing).unwrap();

        assert_eq!(map.lines().count(), WORKSPACE_MAP_LINE_LIMIT);
        assert!(map.contains("src/file_198.rs"));
        assert!(!map.contains("src/file_199.rs"));
    }
}
