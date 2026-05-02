//! Dispatch and prompt-building helpers extracted from `orchestrate.rs`.
//!
//! Free functions for system prompt assembly, tool allowlists, task
//! conversion, context building, and output handling.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use roko_agent::translate::{ClaudeTranslator, RenderedTools, Translator};
use roko_compose::{
    AttentionBidder, Complexity as PromptComplexity, PadState, Placement, PromptSection,
    SectionPriority, TaskContext, estimate_tokens,
};
use roko_core::{AgentRole, Task, TaskStatus};
use roko_learn::playbook::Playbook;
use roko_learn::section_effect::{PriorityChange, SectionEffectivenessRegistry};
use roko_learn::skill_library::Skill;
use roko_std::StaticToolRegistry;

use crate::config::Config;
use crate::prompting::{
    PromptBuildOptions, build_role_system_prompt, build_role_system_prompt_validated,
};
use crate::task_parser;

// ─── Constants ───────────────────────────────────────────────────────────

/// Maximum output size stored in task outputs and episode context (32 KB).
const MAX_OUTPUT_BYTES: usize = 32_768;
/// Number of output lines to include in task failure logs.
pub(crate) const TASK_FAILURE_OUTPUT_TAIL_LINES: usize =
    roko_core::defaults::DEFAULT_TASK_FAILURE_OUTPUT_TAIL_LINES;

// ─── Prompt budget ───────────────────────────────────────────────────────

pub(crate) fn prompt_budget_complexity(
    task_def: Option<&task_parser::TaskDef>,
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

// ─── Task dispatch conventions ───────────────────────────────────────────

pub(crate) fn task_dispatch_conventions(task_def: Option<&task_parser::TaskDef>) -> Option<String> {
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

// ─── System prompt builders ──────────────────────────────────────────────

pub(crate) fn build_system_prompt(
    role: AgentRole,
    plan_id: &str,
    task: &str,
    tools_csv: &str,
    task_def: Option<&task_parser::TaskDef>,
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
    task_def: Option<&task_parser::TaskDef>,
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
    task_def: Option<&task_parser::TaskDef>,
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

// ─── Context layer builder ───────────────────────────────────────────────

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

// ─── Daimon context section ──────────────────────────────────────────────

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
            "Somatic hint: valence={:.2}, intensity={:.2}\n",
            affect_state.somatic_valence, affect_state.somatic_intensity
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

// ─── Section effectiveness ───────────────────────────────────────────────

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

// ─── Default task category ───────────────────────────────────────────────

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

// ─── Tool allowlists ────────────────────────────────────────────────────

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

// ─── Task conversions ────────────────────────────────────────────────────

pub(crate) fn task_def_to_input(td: &task_parser::TaskDef) -> roko_compose::TaskInput {
    let (read_files, symbols, anti_patterns, prior_failures) = match &td.context {
        Some(ctx) => (
            ctx.read_files
                .iter()
                .map(|rf| roko_compose::ReadFileSpec {
                    path: rf.path.clone(),
                    lines: rf.lines.clone(),
                    why: rf.why.clone(),
                })
                .collect(),
            ctx.symbols.clone(),
            ctx.anti_patterns.clone(),
            ctx.prior_failures.clone(),
        ),
        None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
    };

    roko_compose::TaskInput {
        id: td.id.clone(),
        title: td.title.clone(),
        description: td.description.clone(),
        tier: td.tier.clone(),
        files: td.files.clone(),
        read_files,
        symbols,
        anti_patterns,
        prior_failures,
        verify_commands: td
            .verify
            .iter()
            .map(|v| roko_compose::VerifySpec {
                phase: v.phase.clone(),
                command: v.command.clone(),
                fail_msg: v.fail_msg.clone(),
            })
            .collect(),
        acceptance: td.acceptance.clone(),
        depends_on: td.depends_on.clone(),
        max_loc: td.max_loc,
    }
}

pub(crate) fn task_def_to_dag_task(task: &task_parser::TaskDef, completed: bool) -> Task {
    let mut dag_task = Task::new(task.id.clone(), task.title.clone());
    dag_task.status = if completed {
        TaskStatus::Done
    } else {
        TaskStatus::Pending
    };
    dag_task.files = task.files.clone();
    dag_task.role = task.role.clone();
    dag_task.acceptance = task.acceptance.clone();
    dag_task.depends_on = task.depends_on.clone();
    dag_task
}

/// Convert declared task context files into Claude CLI `--read` args.
pub(crate) fn task_read_cli_args(task_def: &task_parser::TaskDef) -> Vec<String> {
    task_def
        .context
        .as_ref()
        .map(|ctx| {
            ctx.read_files
                .iter()
                .flat_map(|rf| ["--read".to_string(), rf.path.clone()])
                .collect()
        })
        .unwrap_or_default()
}

// ─── Output and context helpers ──────────────────────────────────────────

pub(crate) fn truncate_doc_snippet(content: &str, max_chars: usize) -> String {
    let mut chars = content.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_none() {
        content.to_string()
    } else {
        format!("{truncated}\n\n[... truncated]")
    }
}

pub(crate) fn truncate_output(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    let tail = &output[output.len() - MAX_OUTPUT_BYTES..];
    let start = tail.find('\n').map_or(0, |i| i + 1);
    format!(
        "[truncated: original {} bytes, showing last {} bytes]\n{}",
        output.len(),
        MAX_OUTPUT_BYTES,
        &tail[start..]
    )
}

pub(crate) fn tail_output_lines(output: &str, line_count: usize) -> String {
    if output.is_empty() || line_count == 0 {
        return String::new();
    }

    let mut lines: Vec<&str> = output.lines().rev().take(line_count).collect();
    lines.reverse();
    lines.join("\n")
}

pub(crate) fn extract_task_symbols(text: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let mut seen = HashSet::new();

    for raw in text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':')) {
        if raw.is_empty() {
            continue;
        }

        for candidate in raw.split("::") {
            let candidate =
                candidate.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_');
            if candidate.len() < 3 {
                continue;
            }
            let has_underscore = candidate.contains('_');
            let has_upper = candidate.chars().any(|ch| ch.is_ascii_uppercase());
            if !has_underscore && !has_upper {
                continue;
            }

            let candidate = candidate.to_string();
            if seen.insert(candidate.clone()) {
                symbols.push(candidate);
            }
        }
    }

    symbols
}

pub(crate) fn with_task_failure_context(
    error: anyhow::Error,
    task_id: &str,
    phase: &str,
    gate: &str,
    output_tail: Option<&str>,
) -> anyhow::Error {
    let error = error
        .context(format!("task_id={task_id}"))
        .context(format!("phase={phase}"))
        .context(format!("gate={gate}"));

    match output_tail {
        Some(tail) if !tail.trim().is_empty() => error.context(format!(
            "agent_output_tail_last_{}_lines:\n{}",
            TASK_FAILURE_OUTPUT_TAIL_LINES, tail
        )),
        _ => error.context(format!(
            "agent_output_tail_last_{}_lines: <unavailable>",
            TASK_FAILURE_OUTPUT_TAIL_LINES
        )),
    }
}

pub(crate) fn save_task_output(
    workdir: &Path,
    task_id: &str,
    output: &str,
    hub: Option<&crate::state_hub::StateHubSender>,
) {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let _ = std::fs::create_dir_all(&output_dir);
    let output_path = output_dir.join(format!("{task_id}.txt"));
    let summary = truncate_output(output);
    let _ = std::fs::write(output_path, &summary);

    if let Some(hub) = hub {
        let lines: Vec<String> = summary.lines().map(String::from).collect();
        hub.publish(roko_core::DashboardEvent::TaskOutputAppended {
            task_id: task_id.to_string(),
            lines,
        });
    }
}

pub(crate) fn file_contains_public_api(path: &str, content: &str) -> bool {
    let normalized = path.replace('\\', "/");
    if normalized.ends_with("/src/lib.rs") || normalized.ends_with("/src/mod.rs") {
        return true;
    }

    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("pub trait ")
            || trimmed.starts_with("pub type ")
            || trimmed.starts_with("pub use ")
            || trimmed.starts_with("pub mod ")
    })
}

pub(crate) fn load_prior_task_outputs(
    workdir: &Path,
    depends_on: &[String],
) -> Vec<roko_compose::PriorTaskOutput> {
    let output_dir = workdir.join(".roko").join("task-outputs");
    let mut outputs = Vec::new();

    for dep_id in depends_on {
        let output_path = output_dir.join(format!("{dep_id}.txt"));
        if let Ok(summary) = std::fs::read_to_string(&output_path) {
            if !summary.trim().is_empty() {
                outputs.push(roko_compose::PriorTaskOutput {
                    task_id: dep_id.clone(),
                    summary,
                });
            }
        }
    }

    outputs
}

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

/// Extract code-intelligence context chunks for a task description.
pub(crate) fn code_context_for_task(
    workdir: &Path,
    task_description: &str,
    cached_index: Option<&roko_index::WorkspaceIndex>,
) -> Vec<String> {
    const MAX_RESULTS: usize = 15;
    const MAX_TOKENS: usize = 3000;
    const TOKENS_PER_RESULT: usize = 200;

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
