//! PromptAssemblyService — concrete implementation of `PromptAssembler`.
//!
//! Wraps the existing `SystemPromptBuilder` with role resolution, convention
//! detection, and gate feedback injection.

use async_trait::async_trait;
use roko_core::foundation::{PromptAssembler, PromptSpec};
use roko_core::{AgentRole, Result};
use roko_learn::episode_logger::{Episode, EpisodeLogger};
use roko_learn::playbook::{PlaybookStore, QueryContext};
use roko_neuro::{KnowledgeEntry, KnowledgeKind, KnowledgeStore, KnowledgeTier};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::conventions::detect_conventions;
use crate::role_prompts::role_identity_for;
use crate::system_prompt_builder::SystemPromptBuilder;
use crate::token_counter::TokenCounter;

const SOURCE_SAMPLE_LIMIT: usize = 12;
const WORKSPACE_MAP_LINE_LIMIT: usize = 200;

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
    /// Durable knowledge source injected into layer 3 when relevant.
    knowledge_store: Option<Arc<KnowledgeStore>>,
    /// Path to append-only episode history used for recent execution context.
    episodes_path: Option<PathBuf>,
    /// Learned playbook source used for relevant technique injection.
    playbook_store: Option<Arc<PlaybookStore>>,
    /// Tool usage guidance injected into layer 5.
    tool_instructions: Option<String>,
    /// Optional token cap passed through to the system prompt builder.
    token_budget: Option<usize>,
}

impl PromptAssemblyService {
    /// Create a new `PromptAssemblyService`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            default_conventions: None,
            domain_context: None,
            knowledge_store: None,
            episodes_path: None,
            playbook_store: None,
            tool_instructions: None,
            token_budget: None,
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

    /// Add a durable knowledge store for layer 3 domain context.
    #[must_use]
    pub fn with_knowledge_store(mut self, store: Arc<KnowledgeStore>) -> Self {
        self.knowledge_store = Some(store);
        self
    }

    /// Add recent episode history for layer 3b context.
    #[must_use]
    pub fn with_episode_context(mut self, episodes_path: PathBuf) -> Self {
        self.episodes_path = Some(episodes_path);
        self
    }

    /// Add a learned playbook store for layer 6 relevant techniques.
    #[must_use]
    pub fn with_playbook_context(mut self, store: Arc<PlaybookStore>) -> Self {
        self.playbook_store = Some(store);
        self
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
}

impl Default for PromptAssemblyService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PromptAssembler for PromptAssemblyService {
    async fn assemble(&self, spec: PromptSpec) -> Result<String> {
        let role = resolve_role(spec.role.as_deref());
        let identity = role_identity_for(role);

        let mut builder = SystemPromptBuilder::new(identity);
        builder = builder.with_cache_markers();
        // SystemPromptBuilder's current API is no-arg; this is equivalent to with_cache_markers(true).

        if let Some(conventions) = conventions_for_spec(&spec, self.default_conventions.as_deref())
        {
            builder = builder.with_conventions(conventions);
        }

        let mut context_blocks = Vec::new();
        if let Some(ref episodes_path) = self.episodes_path {
            if let Ok(episodes) = EpisodeLogger::read_all(episodes_path).await {
                let recent = episodes.into_iter().rev().take(5).collect::<Vec<_>>();
                if !recent.is_empty() {
                    context_blocks.push(format_episode_context(&recent));
                }
            }
        }

        if let Some(ref store) = self.playbook_store {
            let task_text = spec.task.as_deref().unwrap_or("");
            let ctx = QueryContext {
                task_id: String::new(),
                task_title: task_text.to_string(),
                task_body: String::new(),
                role: spec.role.clone().unwrap_or_default(),
                recent_episodes: 0,
                max_results: 3,
            };
            if let Ok(playbooks) = store.query(&ctx).await {
                if !playbooks.is_empty() {
                    builder = builder.with_playbooks(&playbooks);
                }
            }
        }

        if let Some(domain) = domain_context_for_spec(
            &spec,
            self.domain_context.as_deref(),
            self.knowledge_store.as_deref(),
        ) {
            builder = builder.with_domain(domain);
        }

        if let Some(workspace_map) = workspace_map_for_spec(&spec) {
            context_blocks.push(workspace_map);
        }

        if !context_blocks.is_empty() {
            builder = builder.with_context(context_blocks.join("\n\n"));
        }

        if let Some(task) = spec.task {
            builder = builder.with_task(task);
        }

        if let Some(tools) = &self.tool_instructions {
            builder = builder.with_tools(tools.clone());
        }

        for feedback in spec.gate_feedback {
            builder = builder.with_gate_feedback_text(feedback);
        }

        if !spec.anti_patterns.is_empty() {
            builder = builder.with_anti_patterns(spec.anti_patterns);
        }

        if let Some(budget) = self.token_budget {
            builder = builder.with_token_budget(budget);
            let counter = TokenCounter::Heuristic {
                chars_per_token: 4.0,
            };
            return Ok(builder.build_with_counter(&counter));
        }

        Ok(builder.build())
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
    knowledge_store: Option<&KnowledgeStore>,
) -> Option<String> {
    let knowledge_text = knowledge_store.and_then(|store| relevant_knowledge_for_spec(store, spec));

    match (static_domain_context, knowledge_text) {
        (Some(existing), Some(knowledge)) if !existing.trim().is_empty() => {
            Some(format!("{existing}\n\n{knowledge}"))
        }
        (Some(existing), _) if !existing.trim().is_empty() => Some(existing.to_owned()),
        (_, Some(knowledge)) => Some(knowledge),
        _ => None,
    }
}

fn relevant_knowledge_for_spec(store: &KnowledgeStore, spec: &PromptSpec) -> Option<String> {
    let topic = spec
        .task
        .as_deref()
        .map(|task| first_chars(task, 200))
        .or(spec.role.as_deref())?;

    let entries = store.query(topic, 5).ok()?;
    let facts = entries
        .iter()
        .filter(|entry| entry.confidence >= 0.5 && !matches!(entry.tier, KnowledgeTier::Transient))
        .collect::<Vec<_>>();

    (!facts.is_empty()).then(|| format_knowledge_section(&facts))
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
    );
    (source_samples, file_listing)
}

fn collect_source_context_from(
    dir: &Path,
    root: &Path,
    source_samples: &mut Vec<String>,
    file_listing: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_source_context_from(&path, root, source_samples, file_listing);
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
