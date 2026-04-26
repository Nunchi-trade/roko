//! Prompt construction helpers extracted from `orchestrate.rs`.
//!
//! This module contains:
//! - System prompt assembly (`build_system_prompt*` family)
//! - Tool allowlist generation (`claude_tool_allowlist*`)
//! - Relevant-context layer builder
//! - Code-context for task (index-backed keyword search)
//! - Keyword extraction from task descriptions
//! - Section-effectiveness learning adjustments
//! - Daimon context section builder
//! - Task dispatch conventions

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use roko_agent::translate::{ClaudeTranslator, RenderedTools, Translator};
use roko_compose::{
    AttentionBidder, Complexity as PromptComplexity, PadState, Placement, PromptSection,
    SectionPriority, TaskContext, estimate_tokens,
};
use roko_core::AgentRole;
use roko_core::Engram;
use roko_learn::efficiency::PromptSectionMeta;
use roko_learn::section_effect::{PriorityChange, SectionEffectivenessRegistry};
use roko_learn::skill_library::Skill;
use roko_learn::playbook::Playbook;
use roko_std::StaticToolRegistry;

use crate::config::Config;
use crate::prompting::{
    PromptBuildOptions, build_role_system_prompt, build_role_system_prompt_validated,
};

// ── Task dispatch conventions ─────────────────────────────────────────

pub(crate) fn task_dispatch_conventions(
    task_def: Option<&crate::task_parser::TaskDef>,
) -> Option<String> {
    let task_def = task_def?;
    let mut sections = Vec::new();

    if !task_def.files.is_empty() {
        let mut write_scope = String::from(
            "Honor the declared write scope strictly. Only create, edit, move, or delete files in this allowlist unless the user explicitly expands it:",
        );
        for path in &task_def.files {
            write_scope.push_str("\n- ");
            write_scope.push_str(path);
        }
        sections.push(write_scope);
    }

    if let Some(max_loc) = task_def.max_loc {
        sections.push(format!(
            "Keep the total code delta within roughly {max_loc} lines of change unless verification requires a tightly scoped follow-up."
        ));
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

// ── System prompt builders ────────────────────────────────────────────

pub(crate) fn build_system_prompt(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> String {
    build_system_prompt_with_context(role, plan_id, task, tools_csv, None, None, task_def)
}

pub(crate) fn build_system_prompt_with_context(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    context_layer: Option<&str>,
    affect_state: Option<PadState>,
    task_def: Option<&crate::task_parser::TaskDef>,
) -> String {
    let mut task_context = TaskContext::new(task)
        .with_plan_id(plan_id)
        .with_workspace("roko-cli orchestration");
    if let Some(context) = context_layer.filter(|context| !context.trim().is_empty()) {
        task_context = task_context.with_context(context);
    }
    build_role_system_prompt(
        role,
        task_context,
        tools_csv,
        PromptBuildOptions {
            affect_state,
            complexity: Some(prompt_budget_complexity(task_def)),
            extra_conventions: task_dispatch_conventions(task_def),
            ..PromptBuildOptions::default()
        },
    )
}

pub(crate) fn build_system_prompt_with_context_validated(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    context_layer: Option<&str>,
    affect_state: Option<PadState>,
    task_def: Option<&crate::task_parser::TaskDef>,
    relevant_skills: &[Skill],
    relevant_playbooks: &[Playbook],
    context_window_tokens: usize,
    section_effectiveness: Option<&SectionEffectivenessRegistry>,
    code_context: Vec<String>,
    pheromones: Vec<roko_compose::ContextChunk>,
    extra_anti_patterns: Vec<String>,
) -> Result<String> {
    let mut task_context = TaskContext::new(task)
        .with_plan_id(plan_id)
        .with_workspace("roko-cli orchestration");
    if let Some(context) = context_layer.filter(|context| !context.trim().is_empty()) {
        task_context = task_context.with_context(context);
    }
    build_role_system_prompt_validated(
        role,
        task_context,
        tools_csv,
        PromptBuildOptions {
            affect_state,
            complexity: Some(prompt_budget_complexity(task_def)),
            extra_conventions: task_dispatch_conventions(task_def),
            extra_anti_patterns,
            relevant_skills: relevant_skills.to_vec(),
            relevant_playbooks: relevant_playbooks.to_vec(),
            code_context,
            pheromones,
        },
        context_window_tokens,
        section_effectiveness,
    )
}

// ── Prompt budget ─────────────────────────────────────────────────────

pub(crate) fn prompt_budget_complexity(
    task_def: Option<&crate::task_parser::TaskDef>,
) -> PromptComplexity {
    match task_def.map(|task| task.tier.as_str()) {
        Some("fast" | "mechanical") => PromptComplexity::Trivial,
        Some("complex" | "premium" | "architectural") => PromptComplexity::Complex,
        _ => PromptComplexity::Standard,
    }
}

pub(crate) fn effective_context_window_tokens(config: &Config) -> usize {
    config.prompt.token_budget
}

// ── Relevant context layer ────────────────────────────────────────────

pub(crate) fn build_relevant_context_layer(context_sections: &[PromptSection]) -> Option<String> {
    let non_empty_sections = context_sections
        .iter()
        .map(|section| section.content.trim())
        .filter(|section| !section.is_empty())
        .count();
    let content = context_sections
        .iter()
        .map(|section| section.content.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if content.is_empty() {
        return None;
    }

    let token_estimate = estimate_tokens(&content);
    if non_empty_sections < 2 && token_estimate < 48 {
        tracing::debug!(
            non_empty_sections,
            token_estimate,
            "skipping underspecified relevant-context layer"
        );
        None
    } else {
        Some(format!("## Relevant Context\n\n{content}"))
    }
}

// ── Code context for task ─────────────────────────────────────────────

/// Extract code-intelligence context chunks for a task description.
///
/// When `cached_index` is `Some`, uses the pre-built index instead of loading
/// from disk. This avoids a full workspace scan on every dispatch when the
/// caller maintains a `code_index_cache` (see `PlanRunner::cached_code_index`).
///
/// Falls back to loading from `workdir` when no cached index is available.
/// Returns an empty vec if the index cannot be built or yields no results.
pub(crate) fn code_context_for_task(
    workdir: &Path,
    task_description: &str,
    cached_index: Option<&roko_index::WorkspaceIndex>,
) -> Vec<String> {
    const MAX_RESULTS: usize = 15;
    const MAX_TOKENS: usize = 3000;
    const TOKENS_PER_RESULT: usize = 200;

    // Use the cached index if provided, otherwise load fresh.
    let owned_index;
    let index: &roko_index::WorkspaceIndex = if let Some(idx) = cached_index {
        idx
    } else {
        match roko_index::WorkspaceIndex::load(workdir) {
            Ok(idx) => {
                owned_index = idx;
                &owned_index
            }
            Err(err) => {
                tracing::debug!(error = %err, "code-context: skipping (index unavailable)");
                return Vec::new();
            }
        }
    };

    let keywords = extract_task_keywords(task_description);
    if keywords.is_empty() {
        return Vec::new();
    }

    let query_text = keywords.join(" ");
    let strategy = roko_index::SearchStrategy::Hybrid {
        keyword: Some(roko_index::KeywordQuery {
            text: query_text,
            scope: roko_index::SearchScope::Both,
            case_sensitive: false,
            whole_word: false,
        }),
        structural: None,
        hdc: None,
    };

    let results = index.search(strategy, MAX_RESULTS);
    if results.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut total_tokens = 0;
    for result in &results {
        let chunk = format!(
            "- `{}` ({:?}) in `{}` line {} (score: {:.3})",
            result.symbol.id.symbol_name,
            result.symbol.id.kind,
            result.symbol.id.file_path,
            result.symbol.line,
            result.score,
        );
        let est = estimate_tokens(&chunk);
        if total_tokens + est > MAX_TOKENS {
            break;
        }
        total_tokens += est.min(TOKENS_PER_RESULT);
        chunks.push(chunk);
    }

    if chunks.is_empty() {
        return Vec::new();
    }

    vec![format!(
        "### Relevant Code Symbols\n\n{}",
        chunks.join("\n")
    )]
}

// ── Keyword extraction ────────────────────────────────────────────────

/// Extract meaningful keywords from a task description for code search.
pub(crate) fn extract_task_keywords(description: &str) -> Vec<String> {
    static STOP_WORDS: &[&str] = &[
        "the",
        "a",
        "an",
        "in",
        "on",
        "at",
        "to",
        "for",
        "of",
        "and",
        "or",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "being",
        "have",
        "has",
        "had",
        "do",
        "does",
        "did",
        "will",
        "would",
        "could",
        "should",
        "may",
        "might",
        "shall",
        "can",
        "need",
        "must",
        "it",
        "its",
        "this",
        "that",
        "these",
        "those",
        "with",
        "from",
        "by",
        "as",
        "into",
        "not",
        "no",
        "if",
        "then",
        "else",
        "when",
        "where",
        "which",
        "who",
        "what",
        "how",
        "all",
        "each",
        "every",
        "both",
        "few",
        "more",
        "most",
        "other",
        "some",
        "such",
        "only",
        "own",
        "same",
        "so",
        "than",
        "too",
        "very",
        "just",
        "but",
        "also",
        "about",
        "above",
        "after",
        "before",
        "between",
        "through",
        "during",
        "up",
        "down",
        "out",
        "over",
        "under",
        "again",
        "further",
        "implement",
        "add",
        "create",
        "make",
        "use",
        "update",
        "fix",
        "change",
        "ensure",
    ];

    description
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect::<Vec<_>>()
        .into_iter()
        .take(10)
        .collect()
}

// ── Section effectiveness ─────────────────────────────────────────────

pub(crate) fn adjust_priority_from_section_learning(
    priority: SectionPriority,
    section_name: &str,
    role: &str,
    registry: &SectionEffectivenessRegistry,
) -> SectionPriority {
    let next = match registry.recommend_priority_change(section_name, role) {
        PriorityChange::Increase => (priority as u8).saturating_add(1),
        PriorityChange::Decrease => (priority as u8).saturating_sub(1),
        PriorityChange::NoChange | PriorityChange::InsufficientData => priority as u8,
    };
    match next {
        0 => SectionPriority::Low,
        1 => SectionPriority::Normal,
        2 => SectionPriority::High,
        _ => SectionPriority::Critical,
    }
}

pub(crate) fn apply_section_effectiveness_to_prompt_section(
    mut section: PromptSection,
    role: &str,
    registry: &SectionEffectivenessRegistry,
) -> PromptSection {
    section.priority =
        adjust_priority_from_section_learning(section.priority, &section.name, role, registry);
    section
}

pub(crate) fn prompt_section_meta_from_sections(
    sections: &[Engram],
    prompt: &Engram,
) -> Vec<PromptSectionMeta> {
    let included = prompt.lineage.iter().copied().collect::<HashSet<_>>();
    sections
        .iter()
        .filter_map(|signal| {
            PromptSection::from_signal(signal)
                .ok()
                .map(|section| (signal, section))
        })
        .map(|(signal, original)| {
            let rendered = original.clone().enforce_hard_cap();
            let is_included = included.contains(&signal.id);
            let was_truncated = rendered.content != original.content;
            let tokens = if is_included {
                rendered.estimated_tokens() as u64
            } else {
                0
            };
            PromptSectionMeta {
                name: rendered.name,
                tokens,
                priority: rendered.priority as u8,
                was_truncated: is_included && was_truncated,
                was_dropped: !is_included,
            }
        })
        .collect()
}

// ── Daimon context ────────────────────────────────────────────────────

pub(crate) fn build_daimon_context_section(
    affect_state: PadState,
    behavioral_state: roko_core::BehavioralState,
) -> Option<PromptSection> {
    let pad_magnitude = affect_state.pleasure.abs()
        + affect_state.arousal.abs()
        + affect_state.dominance.abs()
        + affect_state.somatic_intensity;
    if pad_magnitude < 0.35 {
        return None;
    }

    let mut content = format!(
        "## Daimon state\nBehavioral state: {behavioral_state:?}\nPAD: pleasure={:.2}, arousal={:.2}, dominance={:.2}\n",
        affect_state.pleasure, affect_state.arousal, affect_state.dominance
    );
    if affect_state.somatic_intensity >= 0.15 {
        content.push_str(&format!(
            "Somatic intensity: {:.2} (valence: {:+.2})\n",
            affect_state.somatic_intensity, affect_state.somatic_valence
        ));
        if affect_state.somatic_valence <= -0.2 {
            content
                .push_str("Interpretation: slow down, prefer caution, and verify risky moves.\n");
        } else if affect_state.somatic_valence >= 0.2 {
            content.push_str(
                "Interpretation: this strategy region has positive prior outcomes; keep momentum without skipping checks.\n",
            );
        }
    }

    Some(
        PromptSection::new("daimon-state", content)
            .with_priority(SectionPriority::Normal)
            .with_placement(Placement::Middle)
            .with_bidder(AttentionBidder::Daimon)
            .with_hard_cap(256),
    )
}

// ── Tool allowlists ───────────────────────────────────────────────────

pub(crate) fn default_task_category(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("Implementer") || role.eq_ignore_ascii_case("AutoFixer") {
        "implementation"
    } else if role.eq_ignore_ascii_case("Strategist") {
        "planning"
    } else if role.eq_ignore_ascii_case("Auditor") {
        "review"
    } else if role.eq_ignore_ascii_case("Scribe") {
        "documentation"
    } else {
        "implementation"
    }
}

pub(crate) fn claude_tool_allowlist(role: AgentRole) -> String {
    claude_tool_allowlist_with(role, None)
}

pub(crate) fn claude_tool_allowlist_with(
    role: AgentRole,
    dynamic_registry: Option<&roko_agent::mcp::DynamicToolRegistry>,
) -> String {
    use roko_core::tool::ToolRegistry;
    let tools: Vec<roko_core::tool::ToolDef> = if let Some(registry) = dynamic_registry {
        registry.for_role(role).into_iter().cloned().collect()
    } else {
        let registry = StaticToolRegistry::new();
        registry.for_role(role).into_iter().cloned().collect()
    };
    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}

pub(crate) fn claude_task_tool_allowlist_with(
    role: AgentRole,
    allowed_tools: Option<&[String]>,
    denied_tools: Option<&[String]>,
    dynamic_registry: Option<&roko_agent::mcp::DynamicToolRegistry>,
) -> String {
    use roko_core::tool::ToolRegistry;

    let allowed: Option<HashSet<&str>> =
        allowed_tools.map(|tools| tools.iter().map(String::as_str).collect());
    let denied: Option<HashSet<&str>> =
        denied_tools.map(|tools| tools.iter().map(String::as_str).collect());
    let tools: Vec<roko_core::tool::ToolDef> = if let Some(registry) = dynamic_registry {
        registry
            .for_role(role)
            .into_iter()
            .filter(|tool| {
                allowed
                    .as_ref()
                    .is_none_or(|set| set.contains(tool.name.as_str()))
            })
            .filter(|tool| {
                denied
                    .as_ref()
                    .is_none_or(|set| !set.contains(tool.name.as_str()))
            })
            .cloned()
            .collect()
    } else {
        let registry = StaticToolRegistry::new();
        registry
            .for_role(role)
            .into_iter()
            .filter(|tool| {
                allowed
                    .as_ref()
                    .is_none_or(|set| set.contains(tool.name.as_str()))
            })
            .filter(|tool| {
                denied
                    .as_ref()
                    .is_none_or(|set| !set.contains(tool.name.as_str()))
            })
            .cloned()
            .collect()
    };

    match ClaudeTranslator.render_tools(&tools) {
        RenderedTools::CliFlag(csv) => csv,
        _ => String::new(),
    }
}
