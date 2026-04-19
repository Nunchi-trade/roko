//! Demand-driven, tier-aware context provider for agent task prompts.
//!
//! Instead of mori's approach (13-artifact enrichment pipeline dumped into every
//! prompt at fixed per-role budgets), this module assembles context *on demand*
//! based on the model tier the task will run on:
//!
//! - **Surgical** (Haiku / Ollama / Gemma — mechanical tasks): inline files,
//!   symbol signatures, anti-patterns, verification. ~4K token budget. No
//!   enrichment artifacts, no plan context.
//! - **Focused** (Sonnet — focused/integrative tasks): surgical + task-scoped
//!   brief, dependency graph excerpt, prior task outputs. ~12K token budget.
//! - **Full** (Opus — architectural tasks): focused + plan-level brief,
//!   cross-plan context, research memo, invariants/rubric. ~24K token budget.
//!
//! Local models (Ollama/Gemma) always get Surgical tier regardless of task
//! complexity, because they can't reliably use tools and have smaller context
//! windows.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ContextChunk;
use crate::prompt::{AttentionBidder, CacheLayer, Placement, PromptSection, SectionPriority};
use crate::symbol_resolver::SymbolResolver;
use crate::task_brief::TaskBriefGenerator;
use roko_core::{Body, Engram, Kind, OperatingFrequency};
pub use roko_neuro::{ContextSource, ReadFileSpec, TaskInput, VerifySpec};
use serde::{Deserialize, Serialize};
use tracing::info;

// ─── Context tier ──────────────────────────────────────────────────────────

/// Which context tier to use. Derived from the task's tier + the model backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextTier {
    /// Haiku / Ollama / Gemma — mechanical tasks. Everything inline, no tools.
    Surgical,
    /// Sonnet — focused/integrative tasks. Surgical + task brief + deps.
    Focused,
    /// Opus — architectural tasks. Focused + plan brief + research + invariants.
    Full,
}

impl ContextTier {
    /// Derive the context tier from a task tier string and model slug.
    ///
    /// Local models (ollama/*, llama*, gemma*) always get Surgical regardless
    /// of task tier, because they can't reliably handle large contexts or tools.
    #[must_use]
    pub fn from_task_and_model(task_tier: &str, model_slug: &str) -> Self {
        // Local models always get surgical
        if is_local_model(model_slug) {
            return Self::Surgical;
        }
        match task_tier {
            "mechanical" => Self::Surgical,
            "architectural" => Self::Full,
            _ => Self::Focused, // focused, integrative, or unknown
        }
    }

    /// Default token budget for this tier.
    #[must_use]
    pub const fn default_token_budget(self) -> usize {
        match self {
            Self::Surgical => 4_000,
            Self::Focused => 12_000,
            Self::Full => 24_000,
        }
    }
}

impl From<OperatingFrequency> for ContextTier {
    fn from(value: OperatingFrequency) -> Self {
        match value {
            OperatingFrequency::Gamma => Self::Surgical,
            OperatingFrequency::Theta => Self::Focused,
            OperatingFrequency::Delta => Self::Full,
        }
    }
}

impl From<ContextTier> for OperatingFrequency {
    fn from(value: ContextTier) -> Self {
        match value {
            ContextTier::Surgical => Self::Gamma,
            ContextTier::Focused => Self::Theta,
            ContextTier::Full => Self::Delta,
        }
    }
}

/// Check if a model slug refers to a local model (Ollama, Gemma, Llama, etc.)
#[must_use]
pub fn is_local_model(slug: &str) -> bool {
    let lower = slug.to_ascii_lowercase();
    lower.starts_with("ollama/")
        || lower.starts_with("llama")
        || lower.starts_with("gemma")
        || lower.starts_with("qwen")
        || lower.starts_with("mistral")
        || lower.starts_with("codellama")
        || lower.starts_with("deepseek")
        || lower.starts_with("phi")
        || lower.starts_with("starcoder")
        || lower.contains(':')
            && !lower.starts_with("claude")
            && !lower.starts_with("gpt")
            && !lower.starts_with("composer")
            && !lower.starts_with("cursor")
}

// ─── Rolling average context utility ───────────────────────────────────────

/// Minimum average reference rate required to keep `Normal` priority.
const CONTEXT_AVERAGE_DEMOTE_THRESHOLD: f64 = 0.10;

/// Rolling average statistics for one `(task_tier, context_source_type)` pair.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ContextAverageStats {
    /// Exponential moving average of reference rate.
    ema_reference_rate: f64,
    /// Total observations seen for this pair.
    total_observations: u64,
}

/// Loaded rolling averages for task-context demotion.
#[derive(Clone, Debug, Default)]
struct ContextAverageTracker {
    averages: HashMap<String, HashMap<String, ContextAverageStats>>,
}

impl ContextAverageTracker {
    /// Load rolling averages from disk.
    fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let averages = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| {
                serde_json::from_str::<HashMap<String, HashMap<String, ContextAverageStats>>>(&s)
                    .ok()
            })
            .unwrap_or_default();
        Self { averages }
    }

    /// Return the rolling reference rate for a `(task_tier, source_type)` pair.
    #[must_use]
    fn ref_rate(&self, task_tier: &str, source_type: &str) -> f64 {
        self.averages
            .get(task_tier)
            .and_then(|sources| sources.get(source_type))
            .map(|stats| {
                if stats.total_observations == 0 {
                    1.0
                } else {
                    stats.ema_reference_rate
                }
            })
            .unwrap_or(1.0)
    }
}

// ─── Resolved context ──────────────────────────────────────────────────────

/// A resolved context section ready for injection into the prompt.
#[derive(Clone, Debug)]
pub struct ContextSection {
    /// The prompt section to inject.
    pub section: PromptSection,
    /// Where this context came from (for attribution/feedback).
    pub source: ContextSource,
}

impl ContextSection {
    /// Estimated token count.
    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        self.section.estimated_tokens()
    }
}

/// The fully resolved context for a single task dispatch.
#[derive(Clone, Debug)]
pub struct ResolvedContext {
    /// Ordered list of context sections (by priority then cache layer).
    pub sections: Vec<ContextSection>,
    /// Which tier was used.
    pub tier: ContextTier,
    /// Total estimated token count across all sections.
    pub total_tokens_estimate: usize,
    /// Token budget that was applied.
    pub budget_tokens: usize,
}

impl ResolvedContext {
    /// Convert resolved sections into `PromptSection`s for the composer.
    /// Sections are sorted by cache layer (for prefix-cache alignment),
    /// then their placements are reshaped with an attention U-curve so the
    /// highest-value chunks land at the start and end of the final prompt.
    #[must_use]
    pub fn into_prompt_sections(mut self) -> Vec<PromptSection> {
        // Sort: cache layer ascending (stable layers first), then placement, then priority desc
        self.sections.sort_by(|a, b| {
            a.section
                .cache_layer
                .cmp(&b.section.cache_layer)
                .then(placement_ord(a.section.placement).cmp(&placement_ord(b.section.placement)))
                .then((b.section.priority as u8).cmp(&(a.section.priority as u8)))
        });

        apply_attention_curve_placements(&mut self.sections);

        self.sections
            .into_iter()
            .map(|mut cs| {
                cs.section = cs
                    .section
                    .with_bidder(bidder_for_context_source(&cs.source));
                cs.section
            })
            .collect()
    }

    /// Get source attribution for all included sections.
    #[must_use]
    pub fn sources(&self) -> Vec<&ContextSource> {
        self.sections.iter().map(|s| &s.source).collect()
    }
}

const fn placement_ord(p: Placement) -> u8 {
    match p {
        Placement::Start => 0,
        Placement::Middle => 1,
        Placement::End => 2,
    }
}

const fn bidder_for_context_source(source: &ContextSource) -> AttentionBidder {
    match source {
        ContextSource::KnowledgeEntry { .. } => AttentionBidder::Neuro,
        ContextSource::Episode { .. } | ContextSource::PriorTaskOutput { .. } => {
            AttentionBidder::IterationMemory
        }
        ContextSource::InlineFile { .. } | ContextSource::SymbolSignature { .. } => {
            AttentionBidder::CodeIntelligence
        }
        ContextSource::ResearchMemo => AttentionBidder::Research,
        ContextSource::RecentSignal { .. } | ContextSource::Pheromone { .. } => {
            AttentionBidder::Oracles
        }
        ContextSource::AntiPattern
        | ContextSource::Verification
        | ContextSource::TaskBrief
        | ContextSource::PlanBrief
        | ContextSource::Invariants
        | ContextSource::CrossPlanContext
        | ContextSource::PrdExtract
        | ContextSource::Decomposition
        | ContextSource::SiblingTasks => AttentionBidder::TaskContext,
    }
}

fn apply_attention_curve_placements(sections: &mut [ContextSection]) {
    if sections.len() <= 1 {
        return;
    }

    let mut ranked_indices: Vec<usize> = (0..sections.len()).collect();
    ranked_indices.sort_by(|&a, &b| attention_rank_cmp(&sections[a], &sections[b]));

    let edge_slots = ranked_indices.len().div_ceil(2);
    for (rank, idx) in ranked_indices.into_iter().enumerate() {
        sections[idx].section.placement = if rank < edge_slots {
            if rank % 2 == 0 {
                Placement::Start
            } else {
                Placement::End
            }
        } else {
            Placement::Middle
        };
    }
}

fn attention_rank_cmp(a: &ContextSection, b: &ContextSection) -> std::cmp::Ordering {
    (b.section.priority as u8)
        .cmp(&(a.section.priority as u8))
        .then(b.section.cache_layer.cmp(&a.section.cache_layer))
        .then(a.estimated_tokens().cmp(&b.estimated_tokens()))
        .then_with(|| a.section.name.cmp(&b.section.name))
}

// ─── Context provider config ───────────────────────────────────────────────

/// Per-tier token budget overrides.
#[derive(Clone, Debug)]
pub struct ContextBudgets {
    /// Token budget for surgical tier.
    pub surgical: usize,
    /// Token budget for focused tier.
    pub focused: usize,
    /// Token budget for full tier.
    pub full: usize,
}

impl Default for ContextBudgets {
    fn default() -> Self {
        Self {
            surgical: ContextTier::Surgical.default_token_budget(),
            focused: ContextTier::Focused.default_token_budget(),
            full: ContextTier::Full.default_token_budget(),
        }
    }
}

impl ContextBudgets {
    /// Get the budget for a given tier.
    #[must_use]
    pub const fn for_tier(&self, tier: ContextTier) -> usize {
        match tier {
            ContextTier::Surgical => self.surgical,
            ContextTier::Focused => self.focused,
            ContextTier::Full => self.full,
        }
    }

    /// Get the budget for a given operating frequency.
    ///
    /// This wires the 3-speed policy directly into context assembly:
    /// - `Gamma` is reactive and gets no assembled context.
    /// - `Theta` uses the standard deliberative budget.
    /// - `Delta` is reflective and keeps all assembled context.
    #[must_use]
    pub const fn for_frequency(&self, frequency: OperatingFrequency) -> usize {
        match frequency {
            OperatingFrequency::Gamma => 0,
            OperatingFrequency::Theta => self.focused,
            OperatingFrequency::Delta => usize::MAX,
        }
    }
}

// ─── Plan context (artifacts on disk) ──────────────────────────────────────

/// References to enrichment artifacts in the plan directory.
#[derive(Clone, Debug)]
pub struct PlanArtifacts {
    /// Path to plan directory.
    pub plan_dir: PathBuf,
    /// Plan ID / name.
    pub plan_id: String,
}

impl PlanArtifacts {
    /// Create a new `PlanArtifacts` pointing at the given plan directory.
    #[must_use]
    pub const fn new(plan_dir: PathBuf, plan_id: String) -> Self {
        Self { plan_dir, plan_id }
    }

    /// Read a plan artifact if it exists on disk.
    fn read_artifact(&self, filename: &str) -> Option<String> {
        let path = self.plan_dir.join(filename);
        std::fs::read_to_string(&path)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }

    /// Read the plan-level brief (brief.md).
    #[must_use]
    pub fn plan_brief(&self) -> Option<String> {
        self.read_artifact("brief.md")
    }

    /// Read the research memo (research.md).
    #[must_use]
    pub fn research_memo(&self) -> Option<String> {
        self.read_artifact("research.md")
    }

    /// Read the invariants/rubric (rubric.md).
    #[must_use]
    pub fn invariants(&self) -> Option<String> {
        self.read_artifact("rubric.md")
    }

    /// Read the PRD extract (prd-extract.md).
    #[must_use]
    pub fn prd_extract(&self) -> Option<String> {
        self.read_artifact("prd-extract.md")
    }

    /// Read the decomposition (decomposition.md).
    #[must_use]
    pub fn decomposition(&self) -> Option<String> {
        self.read_artifact("decomposition.md")
    }

    /// Read cross-plan context (context.md).
    #[must_use]
    pub fn cross_plan_context(&self) -> Option<String> {
        self.read_artifact("context.md")
    }

    /// Read the plan document itself (plan.md).
    #[must_use]
    pub fn plan_doc(&self) -> Option<String> {
        self.read_artifact("plan.md")
    }
}

// ─── Sibling task info ─────────────────────────────────────────────────────

/// Minimal info about a sibling task in the same plan.
#[derive(Clone, Debug)]
pub struct SiblingTask {
    /// Task ID.
    pub id: String,
    /// Task title.
    pub title: String,
    /// Task status (ready, running, completed, etc.).
    pub status: String,
}

// ─── Prior task output ─────────────────────────────────────────────────────

/// Output summary from a completed dependency task.
#[derive(Clone, Debug)]
pub struct PriorTaskOutput {
    /// The dependency task's ID.
    pub task_id: String,
    /// Truncated summary of the task's output.
    pub summary: String,
}

// ─── The context provider ──────────────────────────────────────────────────

/// Assembles context for agent tasks based on model tier.
///
/// This is the main entry point. Create one per orchestration run, then call
/// [`resolve`](Self::resolve) for each task dispatch.
pub struct ContextProvider {
    /// Working directory (repo root).
    workdir: PathBuf,
    /// Per-tier token budgets.
    budgets: ContextBudgets,
    /// Symbol resolver instance.
    symbol_resolver: SymbolResolver,
    /// Task brief generator instance.
    brief_generator: TaskBriefGenerator,
    /// Rolling averages of context reference rates, loaded from `.roko/learn/`.
    context_average_tracker: ContextAverageTracker,
    /// Recent pheromone signals available for enrichment.
    pheromone_signals: Vec<Engram>,
}

impl ContextProvider {
    /// Create a new context provider rooted at `workdir`.
    #[must_use]
    pub fn new(workdir: PathBuf) -> Self {
        let symbol_resolver = SymbolResolver::new(workdir.clone());
        let brief_generator = TaskBriefGenerator::new();
        let context_average_tracker = ContextAverageTracker::load(
            workdir
                .join(".roko")
                .join("learn")
                .join("context-averages.json"),
        );
        Self {
            workdir,
            budgets: ContextBudgets::default(),
            symbol_resolver,
            brief_generator,
            context_average_tracker,
            pheromone_signals: Vec::new(),
        }
    }

    /// Override the per-tier token budgets.
    #[must_use]
    pub const fn with_budgets(mut self, budgets: ContextBudgets) -> Self {
        self.budgets = budgets;
        self
    }

    /// Attach a snapshot of recent pheromone signals to enrich future context.
    #[must_use]
    pub fn with_pheromone_signals(mut self, pheromone_signals: Vec<Engram>) -> Self {
        self.pheromone_signals = pheromone_signals;
        self
    }

    /// Resolve context for a task at the given operating frequency.
    ///
    /// This is the main entry point — called from `dispatch_agent` in
    /// orchestrate.rs between task parsing and prompt composition.
    pub fn resolve(
        &self,
        frequency: OperatingFrequency,
        task: &TaskInput,
        model_slug: &str,
        plan_artifacts: &PlanArtifacts,
        siblings: &[SiblingTask],
        prior_outputs: &[PriorTaskOutput],
    ) -> ResolvedContext {
        let tier = ContextTier::from_task_and_model(&task.tier, model_slug);
        let budget = self.budgets.for_frequency(frequency);

        if budget == 0 {
            return ResolvedContext {
                sections: Vec::new(),
                tier,
                total_tokens_estimate: 0,
                budget_tokens: budget,
            };
        }

        let mut sections = Vec::new();

        // ── Tier 1: Surgical (always included) ─────────────────────
        self.add_surgical_context(&mut sections, task, budget);

        // ── Tier 2: Focused (Sonnet+) ──────────────────────────────
        if tier == ContextTier::Focused || tier == ContextTier::Full {
            self.add_focused_context(
                &mut sections,
                task,
                plan_artifacts,
                siblings,
                prior_outputs,
                budget,
            );
        }

        // ── Tier 3: Full (Opus) ────────────────────────────────────
        if tier == ContextTier::Full {
            add_full_context(&mut sections, plan_artifacts, budget);
        }

        // ── Rolling-average demotion ────────────────────────────────
        self.apply_average_based_demotions(&mut sections, &task.tier);

        // ── Budget enforcement: drop lowest-priority sections ──────
        let sections = self.enforce_budget(sections, budget);

        let total_tokens_estimate = sections.iter().map(ContextSection::estimated_tokens).sum();

        ResolvedContext {
            sections,
            tier,
            total_tokens_estimate,
            budget_tokens: budget,
        }
    }

    /// Demote `Normal` sections to `Low` when their rolling reference rate is too small.
    fn apply_average_based_demotions(&self, sections: &mut [ContextSection], task_tier: &str) {
        for section in sections {
            let source_type = context_source_type(&section.source);
            let ref_rate = self
                .context_average_tracker
                .ref_rate(task_tier, source_type);
            let decision = if ref_rate < CONTEXT_AVERAGE_DEMOTE_THRESHOLD {
                if section.section.priority == SectionPriority::Normal {
                    section.section.priority = SectionPriority::Low;
                }
                "dropped"
            } else {
                "included"
            };

            info!(
                "[context] {}: {} (ref_rate={ref_rate:.2})",
                section.section.name, decision
            );
        }
    }

    // ── Tier 1: Surgical context ───────────────────────────────────────

    fn add_surgical_context(
        &self,
        sections: &mut Vec<ContextSection>,
        task: &TaskInput,
        _budget: usize,
    ) {
        // 1. Inline file contents
        self.add_inline_files(sections, task);

        // 2. Resolved symbol signatures
        self.add_symbol_signatures(sections, task);

        // 3. Anti-patterns
        add_anti_patterns(sections, task);

        // 4. Prior failures
        add_prior_failures(sections, task);

        // 5. Verification commands
        add_verification(sections, task);
    }

    /// Add inline file contents as context sections.
    fn add_inline_files(&self, sections: &mut Vec<ContextSection>, task: &TaskInput) {
        for rf in &task.read_files {
            let full_path = self.workdir.join(&rf.path);
            if !full_path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&full_path) else {
                continue;
            };

            let lines_to_show = rf.lines.as_ref().map_or_else(
                || content.lines().take(100).collect::<Vec<_>>().join("\n"),
                |range| extract_line_range(&content, range),
            );

            if lines_to_show.trim().is_empty() {
                continue;
            }

            let label = rf.lines.as_ref().map_or_else(
                || format!("file:{}", rf.path),
                |range| format!("file:{}:{}", rf.path, range),
            );

            let formatted = format!(
                "### `{}` {}\nWhy: {}\n```\n{}\n```",
                rf.path,
                rf.lines
                    .as_deref()
                    .map(|l| format!("(lines {l})"))
                    .unwrap_or_default(),
                rf.why,
                lines_to_show,
            );

            sections.push(ContextSection {
                section: PromptSection::new(&label, &formatted)
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::Middle),
                source: ContextSource::InlineFile {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                },
            });
        }
    }

    /// Add resolved symbol signatures as context sections.
    fn add_symbol_signatures(&self, sections: &mut Vec<ContextSection>, task: &TaskInput) {
        use std::fmt::Write;

        if !task.symbols.is_empty() {
            let resolved = self.symbol_resolver.resolve_symbols(&task.symbols);
            if !resolved.is_empty() {
                let mut content = String::from("## Key symbols\n");
                for sig in &resolved {
                    let _ = write!(
                        content,
                        "\n### `{}`\nFrom: `{}`\n```rust\n{}\n```\n",
                        sig.symbol, sig.file, sig.signature
                    );
                }

                sections.push(ContextSection {
                    section: PromptSection::new("symbols", &content)
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::Middle),
                    source: ContextSource::SymbolSignature {
                        symbol: task.symbols.join(", "),
                        file: resolved.first().map(|r| r.file.clone()).unwrap_or_default(),
                    },
                });
            }
        }
    }
}

/// Add anti-patterns as context sections.
fn add_anti_patterns(sections: &mut Vec<ContextSection>, task: &TaskInput) {
    if !task.anti_patterns.is_empty() {
        let content = task
            .anti_patterns
            .iter()
            .map(|ap| format!("- {ap}"))
            .collect::<Vec<_>>()
            .join("\n");
        let formatted = format!("## Do NOT\n{content}");

        sections.push(ContextSection {
            section: PromptSection::new("anti_patterns", &formatted)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End),
            source: ContextSource::AntiPattern,
        });
    }
}

/// Add prior failures as context sections.
fn add_prior_failures(sections: &mut Vec<ContextSection>, task: &TaskInput) {
    if !task.prior_failures.is_empty() {
        let content = task
            .prior_failures
            .iter()
            .enumerate()
            .map(|(i, f)| format!("### Attempt {}\n{f}", i + 1))
            .collect::<Vec<_>>()
            .join("\n\n");
        let formatted = format!("## Prior failures (learn from these)\n{content}");

        sections.push(ContextSection {
            section: PromptSection::new("prior_failures", &formatted)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Volatile)
                .with_placement(Placement::End),
            source: ContextSource::AntiPattern, // reusing for failures
        });
    }
}

/// Add verification commands or acceptance criteria as context sections.
fn add_verification(sections: &mut Vec<ContextSection>, task: &TaskInput) {
    if !task.verify_commands.is_empty() {
        let content = task
            .verify_commands
            .iter()
            .map(|v| {
                let msg = v.fail_msg.as_deref().unwrap_or("must succeed");
                format!("- `{}` — {msg}", v.command)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let formatted =
            format!("## Verification (these commands must pass after your changes)\n{content}");

        sections.push(ContextSection {
            section: PromptSection::new("verification", &formatted)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End),
            source: ContextSource::Verification,
        });
    } else if !task.acceptance.is_empty() {
        let content = task
            .acceptance
            .iter()
            .map(|a| format!("- {a}"))
            .collect::<Vec<_>>()
            .join("\n");
        let formatted = format!("## Acceptance criteria\n{content}");

        sections.push(ContextSection {
            section: PromptSection::new("acceptance", &formatted)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End),
            source: ContextSource::Verification,
        });
    }
}

/// Convert a snapshot of pheromone engrams into context chunks.
///
/// The `scope` filter accepts an exact plan/scope identifier or `all`.
/// Signals without explicit scope metadata are treated as globally visible.
#[must_use]
pub fn pheromone_context(field: &[Engram], scope: &str) -> Vec<ContextChunk> {
    let requested_scope = scope.to_ascii_lowercase();
    let mut chunks = field
        .iter()
        .filter(|signal| signal.kind == Kind::Pheromone)
        .filter(|signal| pheromone_matches_scope(signal, &requested_scope))
        .map(|signal| pheromone_chunk(signal, &requested_scope))
        .collect::<Vec<_>>();
    chunks.sort_by(|left, right| {
        right
            .relevance
            .partial_cmp(&left.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.content.cmp(&right.content))
    });
    chunks
}

impl ContextProvider {
    // ── Tier 2: Focused context ────────────────────────────────────────

    fn add_focused_context(
        &self,
        sections: &mut Vec<ContextSection>,
        task: &TaskInput,
        plan_artifacts: &PlanArtifacts,
        siblings: &[SiblingTask],
        prior_outputs: &[PriorTaskOutput],
        _budget: usize,
    ) {
        // 1. Active pheromone field summary.
        self.add_pheromone_context(sections, &plan_artifacts.plan_id);

        // 2. Task-scoped brief (What/Why/How)
        let plan_doc = plan_artifacts.plan_doc();
        let brief = self
            .brief_generator
            .generate(task, plan_doc.as_deref(), siblings);
        if !brief.is_empty() {
            sections.push(ContextSection {
                section: PromptSection::new("task_brief", &brief)
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(3_000),
                source: ContextSource::TaskBrief,
            });
        }

        // 3. Sibling tasks (just IDs + titles for orientation)
        if !siblings.is_empty() {
            let content = siblings
                .iter()
                .map(|s| {
                    let marker = if task.depends_on.contains(&s.id) {
                        " ← depends on"
                    } else if sibling_depends_on_me(s, task) {
                        " → blocks"
                    } else {
                        ""
                    };
                    let status = match s.status.as_str() {
                        "done" | "completed" => " ✅",
                        "running" | "in_progress" => " ⏳",
                        _ => "",
                    };
                    format!("- **{}**: {}{}{}", s.id, s.title, status, marker)
                })
                .collect::<Vec<_>>()
                .join("\n");
            let formatted = format!("## Sibling tasks in this plan\n{content}");

            sections.push(ContextSection {
                section: PromptSection::new("siblings", &formatted)
                    .with_priority(SectionPriority::Low)
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(1_500),
                source: ContextSource::SiblingTasks,
            });
        }

        // 4. Prior task outputs (from completed dependencies)
        let relevant_outputs: Vec<_> = prior_outputs
            .iter()
            .filter(|o| task.depends_on.contains(&o.task_id))
            .collect();
        if !relevant_outputs.is_empty() {
            let content = relevant_outputs
                .iter()
                .map(|o| format!("### {} output\n{}", o.task_id, o.summary))
                .collect::<Vec<_>>()
                .join("\n\n");
            let formatted = format!("## Completed dependency outputs\n{content}");

            sections.push(ContextSection {
                section: PromptSection::new("prior_outputs", &formatted)
                    .with_priority(SectionPriority::Normal)
                    .with_cache_layer(CacheLayer::Volatile)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(4_000),
                source: ContextSource::PriorTaskOutput {
                    task_id: relevant_outputs
                        .iter()
                        .map(|o| o.task_id.clone())
                        .collect::<Vec<_>>()
                        .join(","),
                },
            });
        }

        // 5. PRD extract (scoped: only paragraphs mentioning this task's files)
        if let Some(prd) = plan_artifacts.prd_extract() {
            let scoped = scope_text_to_files(&prd, &task.files);
            if !scoped.is_empty() {
                sections.push(ContextSection {
                    section: PromptSection::new("prd_extract", format!("## PRD context\n{scoped}"))
                        .with_priority(SectionPriority::Low)
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle)
                        .with_hard_cap(2_000),
                    source: ContextSource::PrdExtract,
                });
            }
        }
    }

    fn add_pheromone_context(&self, sections: &mut Vec<ContextSection>, scope: &str) {
        let pheromones = pheromone_context(&self.pheromone_signals, scope);
        if pheromones.is_empty() {
            return;
        }

        for (index, chunk) in pheromones.into_iter().enumerate() {
            let priority = pheromone_priority(&chunk);
            sections.push(ContextSection {
                section: PromptSection::new(format!("pheromone_signal_{index}"), chunk.content)
                    .with_priority(priority)
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::Middle)
                    .with_hard_cap(800),
                source: ContextSource::RecentSignal {
                    signal_id: format!("pheromone-{scope}-{index}"),
                    plan_id: scope.to_string(),
                    kind: "pheromone".to_string(),
                },
            });
        }
    }

    // ── Budget enforcement ─────────────────────────────────────────────

    /// Drop lowest-priority sections until total fits within budget.
    /// Within the same priority level, drop largest sections first.
    #[allow(clippy::unused_self)] // method form for test ergonomics
    fn enforce_budget(
        &self,
        mut sections: Vec<ContextSection>,
        budget: usize,
    ) -> Vec<ContextSection> {
        // First, enforce per-section hard caps
        for section in &mut sections {
            section.section = section.section.clone().enforce_hard_cap();
        }

        let total: usize = sections.iter().map(ContextSection::estimated_tokens).sum();
        if total <= budget {
            return sections;
        }

        // Sort by priority ascending (lowest first = dropped first),
        // then by size descending (within same priority, drop biggest first)
        sections.sort_by(|a, b| {
            (a.section.priority as u8)
                .cmp(&(b.section.priority as u8))
                .then(b.estimated_tokens().cmp(&a.estimated_tokens()))
        });

        let mut running_total: usize = sections.iter().map(ContextSection::estimated_tokens).sum();
        let mut to_drop = Vec::new();

        for (i, section) in sections.iter().enumerate() {
            if running_total <= budget {
                break;
            }
            // Never drop Critical sections
            if section.section.priority == SectionPriority::Critical {
                continue;
            }
            running_total -= section.estimated_tokens();
            to_drop.push(i);
        }

        // Remove dropped sections (reverse order to preserve indices)
        to_drop.reverse();
        for i in to_drop {
            sections.remove(i);
        }

        sections
    }
}

// ── Tier 3: Full context ───────────────────────────────────────────

/// Add full-tier context sections (plan brief, research, invariants, etc.).
fn add_full_context(
    sections: &mut Vec<ContextSection>,
    plan_artifacts: &PlanArtifacts,
    _budget: usize,
) {
    // 1. Plan-level brief (full, not scoped)
    if let Some(brief) = plan_artifacts.plan_brief() {
        sections.push(ContextSection {
            section: PromptSection::new("plan_brief", format!("## Plan brief\n{brief}"))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(6_000),
            source: ContextSource::PlanBrief,
        });
    }

    // 2. Research memo
    if let Some(research) = plan_artifacts.research_memo() {
        sections.push(ContextSection {
            section: PromptSection::new("research", format!("## Research memo\n{research}"))
                .with_priority(SectionPriority::Low)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(4_000),
            source: ContextSource::ResearchMemo,
        });
    }

    // 3. Invariants / rubric
    if let Some(rubric) = plan_artifacts.invariants() {
        sections.push(ContextSection {
            section: PromptSection::new("invariants", format!("## Invariants & rubric\n{rubric}"))
                .with_priority(SectionPriority::Normal)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(3_000),
            source: ContextSource::Invariants,
        });
    }

    // 4. Cross-plan context
    if let Some(cross) = plan_artifacts.cross_plan_context() {
        sections.push(ContextSection {
            section: PromptSection::new("cross_plan", format!("## Cross-plan context\n{cross}"))
                .with_priority(SectionPriority::Low)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(3_000),
            source: ContextSource::CrossPlanContext,
        });
    }

    // 5. Decomposition
    if let Some(decomp) = plan_artifacts.decomposition() {
        sections.push(ContextSection {
            section: PromptSection::new("decomposition", format!("## Decomposition\n{decomp}"))
                .with_priority(SectionPriority::Low)
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(3_000),
            source: ContextSource::Decomposition,
        });
    }
}

/// Convert a context source into the stable source-type key used by learning data.
const fn context_source_type(source: &ContextSource) -> &'static str {
    match source {
        ContextSource::KnowledgeEntry { .. } => "knowledge",
        ContextSource::Episode { .. } => "episode",
        ContextSource::InlineFile { .. } => "file",
        ContextSource::RecentSignal { .. } => "signal",
        ContextSource::SymbolSignature { .. } => "symbol",
        ContextSource::AntiPattern => "anti_pattern",
        ContextSource::Verification => "verification",
        ContextSource::TaskBrief => "task_brief",
        ContextSource::PriorTaskOutput { .. } => "prior_output",
        ContextSource::PlanBrief => "plan_brief",
        ContextSource::ResearchMemo => "research_memo",
        ContextSource::Invariants => "invariants",
        ContextSource::CrossPlanContext => "cross_plan",
        ContextSource::PrdExtract => "prd_extract",
        ContextSource::Decomposition => "decomposition",
        ContextSource::SiblingTasks => "sibling_tasks",
        ContextSource::Pheromone { .. } => "pheromone",
    }
}

/// Check if this sibling depends on the given task.
///
/// We don't have the sibling's `depends_on` here, so this is best-effort.
/// The orchestrator could populate this if needed.
const fn sibling_depends_on_me(_sibling: &SiblingTask, _task: &TaskInput) -> bool {
    false
}

fn pheromone_chunk(signal: &Engram, scope: &str) -> ContextChunk {
    let kind = pheromone_kind(signal);
    let intensity = signal
        .tag("pheromone_intensity")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let confidence = signal
        .tag("pheromone_confidence")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let decay_rate = signal
        .tag("pheromone_decay_rate")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
        .max(0.0);
    let deposited_by = signal
        .tag("pheromone_deposited_by")
        .or_else(|| signal.tag("author"))
        .unwrap_or(signal.provenance.author.as_str());
    let body = render_signal_body(signal);
    let content = format!(
        "- [{kind}] scope={scope} intensity={intensity:.2} confidence={confidence:.2} decay={decay_rate:.2} by {deposited_by}\n  {body}"
    );

    ContextChunk {
        content,
        source: ContextSource::RecentSignal {
            signal_id: signal.id.to_string(),
            plan_id: scope.to_string(),
            kind: "pheromone".to_string(),
        },
        relevance: intensity.max(confidence),
        track_record: Some(intensity),
        confidence: Some(confidence),
        recency: Some(signal.created_at_ms.max(0) as f64),
        emotional_tag: None,
    }
}

fn pheromone_priority(chunk: &ContextChunk) -> SectionPriority {
    let lower = chunk.content.to_ascii_lowercase();
    if lower.contains("[threat]") || lower.contains("[warning]") || lower.contains("failure") {
        SectionPriority::High
    } else if lower.contains("[opportunity]") || lower.contains("success") {
        SectionPriority::Normal
    } else {
        SectionPriority::Low
    }
}

fn pheromone_kind(signal: &Engram) -> &'static str {
    let from_tag = signal
        .tag("pheromone_kind")
        .or_else(|| signal.tag("kind"))
        .unwrap_or(signal.kind.as_str());
    let lower = from_tag.to_ascii_lowercase();
    if lower.contains("threat") || lower.contains("warning") || lower.contains("failure") {
        "Threat"
    } else if lower.contains("opportunity") || lower.contains("success") {
        "Opportunity"
    } else if lower.contains("resource") {
        "Resource"
    } else {
        "Signal"
    }
}

fn pheromone_matches_scope(signal: &Engram, requested_scope: &str) -> bool {
    let scope = signal
        .tag("pheromone_scope")
        .or_else(|| signal.tag("scope"))
        .or_else(|| signal.tag("plan_id"))
        .unwrap_or("global")
        .to_ascii_lowercase();
    requested_scope == "all" || scope == "global" || scope == requested_scope
}

fn render_signal_body(signal: &Engram) -> String {
    match &signal.body {
        Body::Text(text) => text.trim().to_string(),
        Body::Json(value) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        }
        Body::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        Body::Empty => String::from("<empty>"),
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

/// Extract lines from content given a range like "40-80" or "10-".
fn extract_line_range(content: &str, range: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let parts: Vec<&str> = range.split('-').collect();
    let start = parts
        .first()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let end = parts
        .get(1)
        .and_then(|s| {
            if s.is_empty() {
                None
            } else {
                s.parse::<usize>().ok()
            }
        })
        .unwrap_or(lines.len())
        .min(lines.len());
    lines[start..end].join("\n")
}

/// Scope a text document to paragraphs that mention any of the given file paths.
/// Returns the full paragraph for each match. If no matches, returns empty string.
fn scope_text_to_files(text: &str, files: &[String]) -> String {
    if files.is_empty() {
        return String::new();
    }

    // Split into paragraphs (double newline separated)
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut matched = Vec::new();

    for para in paragraphs {
        let lower = para.to_ascii_lowercase();
        for file in files {
            // Match the filename or the path
            let basename = Path::new(file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file);
            if lower.contains(&file.to_ascii_lowercase())
                || lower.contains(&basename.to_ascii_lowercase())
            {
                matched.push(para);
                break;
            }
        }
    }

    matched.join("\n\n")
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::OperatingFrequency;

    #[test]
    fn context_tier_from_task_and_model() {
        assert_eq!(
            ContextTier::from_task_and_model("mechanical", "claude-haiku-4-5"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("focused", "claude-sonnet-4-6"),
            ContextTier::Focused
        );
        assert_eq!(
            ContextTier::from_task_and_model("architectural", "claude-opus-4-6"),
            ContextTier::Full
        );
        assert_eq!(
            ContextTier::from_task_and_model("integrative", "claude-sonnet-4-6"),
            ContextTier::Focused
        );
    }

    #[test]
    fn local_models_always_get_surgical() {
        assert_eq!(
            ContextTier::from_task_and_model("architectural", "ollama/gemma4:12b"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("focused", "llama3.1:8b"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("integrative", "gemma4:27b"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("architectural", "qwen2.5-coder:7b"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("focused", "deepseek-coder:6.7b"),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from_task_and_model("focused", "mistral:7b"),
            ContextTier::Surgical
        );
    }

    #[test]
    fn operating_frequency_maps_to_context_tier() {
        assert_eq!(
            ContextTier::from(OperatingFrequency::Gamma),
            ContextTier::Surgical
        );
        assert_eq!(
            ContextTier::from(OperatingFrequency::Theta),
            ContextTier::Focused
        );
        assert_eq!(
            ContextTier::from(OperatingFrequency::Delta),
            ContextTier::Full
        );
        assert_eq!(
            OperatingFrequency::from(ContextTier::Surgical),
            OperatingFrequency::Gamma
        );
        assert_eq!(
            OperatingFrequency::from(ContextTier::Focused),
            OperatingFrequency::Theta
        );
        assert_eq!(
            OperatingFrequency::from(ContextTier::Full),
            OperatingFrequency::Delta
        );
    }

    #[test]
    fn is_local_model_detects_ollama_patterns() {
        assert!(is_local_model("ollama/gemma4:12b"));
        assert!(is_local_model("llama3.1:8b"));
        assert!(is_local_model("gemma4:27b"));
        assert!(is_local_model("qwen2.5-coder:7b"));
        assert!(is_local_model("deepseek-coder:6.7b"));
        assert!(is_local_model("mistral:7b"));
        assert!(is_local_model("phi-3:mini"));
        assert!(is_local_model("starcoder2:3b"));
        assert!(is_local_model("codellama:7b"));

        // Cloud models are NOT local
        assert!(!is_local_model("claude-sonnet-4-6"));
        assert!(!is_local_model("claude-opus-4-6"));
        assert!(!is_local_model("claude-haiku-4-5"));
        assert!(!is_local_model("gpt-5.4"));
        assert!(!is_local_model("composer-2-fast"));
        assert!(!is_local_model("cursor-fast"));
    }

    #[test]
    fn default_budgets() {
        let budgets = ContextBudgets::default();
        assert_eq!(budgets.surgical, 4_000);
        assert_eq!(budgets.focused, 12_000);
        assert_eq!(budgets.full, 24_000);
        assert_eq!(budgets.for_frequency(OperatingFrequency::Gamma), 0);
        assert_eq!(budgets.for_frequency(OperatingFrequency::Theta), 12_000);
        assert_eq!(budgets.for_frequency(OperatingFrequency::Delta), usize::MAX);
    }

    #[test]
    fn rolling_average_demotes_low_value_normal_sections() {
        let workdir = PathBuf::from("/tmp/test");
        let mut provider = ContextProvider::new(workdir);
        provider.context_average_tracker.averages.insert(
            "integrative".to_string(),
            HashMap::from([(
                "task_brief".to_string(),
                ContextAverageStats {
                    ema_reference_rate: 0.05,
                    total_observations: 12,
                },
            )]),
        );

        let mut sections = vec![
            ContextSection {
                section: PromptSection::new("task_brief", "brief content")
                    .with_priority(SectionPriority::Normal),
                source: ContextSource::TaskBrief,
            },
            ContextSection {
                section: PromptSection::new("verification", "verify content")
                    .with_priority(SectionPriority::High),
                source: ContextSource::Verification,
            },
        ];

        provider.apply_average_based_demotions(&mut sections, "integrative");

        assert_eq!(sections[0].section.priority, SectionPriority::Low);
        assert_eq!(sections[1].section.priority, SectionPriority::High);
    }

    #[test]
    fn pheromone_context_filters_by_scope_and_kind() {
        let signals = vec![
            Engram::builder(Kind::Pheromone)
                .body(Body::text("Reduce routing latency"))
                .tag("pheromone_kind", "threat")
                .tag("pheromone_scope", "plan-alpha")
                .tag("pheromone_intensity", "0.92")
                .tag("pheromone_confidence", "0.81")
                .build(),
            Engram::builder(Kind::Pheromone)
                .body(Body::text("Reuse known-good prompt paths"))
                .tag("pheromone_kind", "opportunity")
                .tag("pheromone_scope", "global")
                .tag("pheromone_intensity", "0.72")
                .tag("pheromone_confidence", "0.88")
                .build(),
            Engram::builder(Kind::Task)
                .body(Body::text("Not a pheromone"))
                .build(),
        ];

        let chunks = pheromone_context(&signals, "plan-alpha");

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("[Threat]"));
        assert!(chunks[1].content.contains("[Opportunity]"));
        assert!(
            chunks
                .iter()
                .all(|chunk| matches!(chunk.source, ContextSource::RecentSignal { .. }))
        );
    }

    #[test]
    fn resolve_includes_pheromone_sections_when_signals_are_present() {
        let workdir = PathBuf::from("/tmp/test");
        let provider = ContextProvider::new(workdir).with_pheromone_signals(vec![
            Engram::builder(Kind::Pheromone)
                .body(Body::text("Context assembly is too slow"))
                .tag("pheromone_kind", "warning")
                .tag("pheromone_scope", "plan-42")
                .tag("pheromone_intensity", "0.9")
                .build(),
        ]);
        let task = TaskInput {
            id: "T1".to_string(),
            title: "Make prompt assembly faster".to_string(),
            description: None,
            tier: "focused".to_string(),
            files: vec![],
            read_files: vec![],
            symbols: vec![],
            anti_patterns: vec![],
            prior_failures: vec![],
            verify_commands: vec![],
            acceptance: vec![],
            depends_on: vec![],
            max_loc: None,
        };
        let plan_artifacts = PlanArtifacts::new(PathBuf::from("/tmp/plan"), "plan-42".to_string());

        let resolved = provider.resolve(
            OperatingFrequency::Theta,
            &task,
            "claude-sonnet-4-6",
            &plan_artifacts,
            &[],
            &[],
        );

        assert!(
            resolved
                .sections
                .iter()
                .any(|section| section.section.name.starts_with("pheromone_signal"))
        );
    }

    #[test]
    fn scope_text_to_files_finds_relevant_paragraphs() {
        let text = "This paragraph talks about src/main.rs and how it works.\n\n\
                     This paragraph is about unrelated things.\n\n\
                     Here we discuss config.rs and settings.";
        let files = vec!["src/main.rs".into()];
        let scoped = scope_text_to_files(text, &files);
        assert!(scoped.contains("src/main.rs"));
        assert!(!scoped.contains("unrelated"));
    }

    #[test]
    fn scope_text_to_files_matches_basename() {
        let text = "This talks about main.rs changes.\n\nUnrelated paragraph.";
        let files = vec!["crates/roko-cli/src/main.rs".into()];
        let scoped = scope_text_to_files(text, &files);
        assert!(scoped.contains("main.rs"));
    }

    #[test]
    fn scope_text_empty_files_returns_empty() {
        let text = "Some content here.";
        let scoped = scope_text_to_files(text, &[]);
        assert!(scoped.is_empty());
    }

    #[test]
    fn extract_line_range_works() {
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        assert_eq!(extract_line_range(content, "2-4"), "line 2\nline 3\nline 4");
        assert_eq!(extract_line_range(content, "3-"), "line 3\nline 4\nline 5");
    }

    #[test]
    fn enforce_budget_drops_low_priority_first() {
        let workdir = PathBuf::from("/tmp/test");
        let provider = ContextProvider::new(workdir);

        let sections = vec![
            ContextSection {
                section: PromptSection::new("critical", &"x".repeat(400))
                    .with_priority(SectionPriority::Critical),
                source: ContextSource::Verification,
            },
            ContextSection {
                section: PromptSection::new("high", &"y".repeat(400))
                    .with_priority(SectionPriority::High),
                source: ContextSource::AntiPattern,
            },
            ContextSection {
                section: PromptSection::new("low", &"z".repeat(4000))
                    .with_priority(SectionPriority::Low),
                source: ContextSource::ResearchMemo,
            },
        ];

        // Budget = 300 tokens (~1200 bytes). The low section alone is ~1000 tokens.
        let result = provider.enforce_budget(sections, 300);

        // Low should be dropped, critical and high kept
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|s| s.section.name == "critical"));
        assert!(result.iter().any(|s| s.section.name == "high"));
        assert!(!result.iter().any(|s| s.section.name == "low"));
    }

    #[test]
    fn enforce_budget_never_drops_critical() {
        let workdir = PathBuf::from("/tmp/test");
        let provider = ContextProvider::new(workdir);

        let sections = vec![ContextSection {
            section: PromptSection::new("critical_big", &"x".repeat(8000))
                .with_priority(SectionPriority::Critical),
            source: ContextSource::Verification,
        }];

        // Budget is tiny but critical sections survive
        let result = provider.enforce_budget(sections, 10);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].section.name, "critical_big");
    }

    #[test]
    fn resolved_context_into_prompt_sections_sorts_by_cache_layer() {
        let resolved = ResolvedContext {
            sections: vec![
                ContextSection {
                    section: PromptSection::new("task", "task content")
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::End),
                    source: ContextSource::Verification,
                },
                ContextSection {
                    section: PromptSection::new("session", "session content")
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                    source: ContextSource::PlanBrief,
                },
                ContextSection {
                    section: PromptSection::new("system", "system content")
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Start),
                    source: ContextSource::AntiPattern,
                },
            ],
            tier: ContextTier::Full,
            total_tokens_estimate: 30,
            budget_tokens: 24_000,
        };

        let prompt_sections = resolved.into_prompt_sections();
        assert_eq!(prompt_sections[0].name, "system");
        assert_eq!(prompt_sections[1].name, "session");
        assert_eq!(prompt_sections[2].name, "task");
        assert_eq!(prompt_sections[0].bidder, AttentionBidder::TaskContext);
        assert_eq!(prompt_sections[1].bidder, AttentionBidder::TaskContext);
        assert_eq!(prompt_sections[2].bidder, AttentionBidder::TaskContext);
    }

    #[test]
    fn resolved_context_maps_sources_to_attention_bidders() {
        let resolved = ResolvedContext {
            sections: vec![
                ContextSection {
                    section: PromptSection::new("knowledge", "knowledge"),
                    source: ContextSource::KnowledgeEntry {
                        entry_id: "k1".into(),
                        kind: "heuristic".into(),
                        source: Some("neuro".into()),
                    },
                },
                ContextSection {
                    section: PromptSection::new("file", "file"),
                    source: ContextSource::InlineFile {
                        path: "src/lib.rs".into(),
                        lines: None,
                    },
                },
                ContextSection {
                    section: PromptSection::new("research", "research"),
                    source: ContextSource::ResearchMemo,
                },
            ],
            tier: ContextTier::Focused,
            total_tokens_estimate: 12,
            budget_tokens: 12_000,
        };

        let prompt_sections = resolved.into_prompt_sections();
        assert!(
            prompt_sections
                .iter()
                .any(|section| section.bidder == AttentionBidder::Neuro)
        );
        assert!(
            prompt_sections
                .iter()
                .any(|section| section.bidder == AttentionBidder::CodeIntelligence)
        );
        assert!(
            prompt_sections
                .iter()
                .any(|section| section.bidder == AttentionBidder::Research)
        );
    }

    #[test]
    fn resolved_context_attention_curve_positions_high_value_at_edges() {
        let resolved = ResolvedContext {
            sections: vec![
                ContextSection {
                    section: PromptSection::new("critical", "critical context")
                        .with_priority(SectionPriority::Critical)
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::Middle),
                    source: ContextSource::Verification,
                },
                ContextSection {
                    section: PromptSection::new("high", "high value")
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Volatile)
                        .with_placement(Placement::Middle),
                    source: ContextSource::PriorTaskOutput {
                        task_id: "T1".into(),
                    },
                },
                ContextSection {
                    section: PromptSection::new("normal", "normal value")
                        .with_priority(SectionPriority::Normal)
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                    source: ContextSource::PlanBrief,
                },
                ContextSection {
                    section: PromptSection::new("low", "low value")
                        .with_priority(SectionPriority::Low)
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Middle),
                    source: ContextSource::ResearchMemo,
                },
            ],
            tier: ContextTier::Full,
            total_tokens_estimate: 12,
            budget_tokens: 24_000,
        };

        let prompt_sections = resolved.into_prompt_sections();

        let critical = prompt_sections
            .iter()
            .find(|s| s.name == "critical")
            .expect("critical section present");
        let high = prompt_sections
            .iter()
            .find(|s| s.name == "high")
            .expect("high section present");
        let normal = prompt_sections
            .iter()
            .find(|s| s.name == "normal")
            .expect("normal section present");
        let low = prompt_sections
            .iter()
            .find(|s| s.name == "low")
            .expect("low section present");

        assert_eq!(critical.placement, Placement::Start);
        assert_eq!(high.placement, Placement::End);
        assert_eq!(normal.placement, Placement::Middle);
        assert_eq!(low.placement, Placement::Middle);
    }
}
