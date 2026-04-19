//! Shared system-prompt assembly helpers for CLI execution paths.

use anyhow::Result;
use roko_compose::{Complexity, PadState, RoleSystemPromptSpec, TaskContext};
use roko_core::AgentRole;
use roko_learn::playbook::Playbook;
use roko_learn::section_effect::SectionEffectivenessRegistry;
use roko_learn::skill_library::Skill;

/// Optional prompt-builder settings shared across CLI dispatch paths.
#[derive(Clone, Debug, Default)]
pub struct PromptBuildOptions {
    /// Optional affect state for tone/focus guidance.
    pub affect_state: Option<PadState>,
    /// Optional prompt-budget complexity band.
    pub complexity: Option<Complexity>,
    /// Optional additional conventions appended to defaults.
    pub extra_conventions: Option<String>,
    /// Optional extra anti-patterns appended to defaults.
    pub extra_anti_patterns: Vec<String>,
    /// Optional relevant skills injected into the system prompt.
    pub relevant_skills: Vec<Skill>,
    /// Optional relevant playbooks injected into the system prompt.
    pub relevant_playbooks: Vec<Playbook>,
    /// Optional code-intelligence context chunks injected as domain context.
    pub code_context: Vec<String>,
}

fn build_spec(
    role: AgentRole,
    task_context: TaskContext,
    tools_csv: impl Into<String>,
    options: PromptBuildOptions,
) -> RoleSystemPromptSpec {
    let task_context = if options.code_context.is_empty() {
        task_context
    } else {
        let combined = options.code_context.join("\n\n");
        task_context.with_domain_notes(combined)
    };
    let mut spec = RoleSystemPromptSpec::new(role, task_context, tools_csv)
        .with_affect_state(options.affect_state)
        .with_cache_markers();
    if let Some(complexity) = options.complexity {
        spec = spec.with_complexity(complexity);
    }
    if let Some(conventions) = options.extra_conventions {
        spec = spec.with_extra_conventions(conventions);
    }
    for anti_pattern in options.extra_anti_patterns {
        spec = spec.add_anti_pattern(anti_pattern);
    }
    if !options.relevant_skills.is_empty() {
        spec = spec.with_relevant_skills(&options.relevant_skills);
    }
    if !options.relevant_playbooks.is_empty() {
        spec = spec.with_relevant_playbooks(&options.relevant_playbooks);
    }
    spec
}

/// Build a role-scoped system prompt from shared task context.
#[must_use]
pub fn build_role_system_prompt(
    role: AgentRole,
    task_context: TaskContext,
    tools_csv: impl Into<String>,
    options: PromptBuildOptions,
) -> String {
    build_spec(role, task_context, tools_csv, options).build()
}

/// Build a role-scoped system prompt and validate it against a context window.
pub fn build_role_system_prompt_validated(
    role: AgentRole,
    task_context: TaskContext,
    tools_csv: impl Into<String>,
    options: PromptBuildOptions,
    context_window_tokens: usize,
    section_effectiveness: Option<&SectionEffectivenessRegistry>,
) -> Result<String> {
    Ok(build_spec(role, task_context, tools_csv, options)
        .build_with_context_window_and_section_effectiveness(
            context_window_tokens,
            section_effectiveness,
        )?)
}
