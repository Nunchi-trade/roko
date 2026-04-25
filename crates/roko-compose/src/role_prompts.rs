//! Shared role-based system prompt construction helpers.
//!
//! This module centralizes the prompt wiring that was previously duplicated
//! across CLI entrypoints. It uses [`SystemPromptBuilder`] plus the existing
//! role-template identities from [`crate::templates`] and exposes a typed API
//! suitable for both single-shot and orchestrated execution paths.

use crate::ContextChunk;
use crate::PadState;
use crate::budget::{Complexity, adjusted_budget_for};
use crate::prompt::estimate_tokens;
use crate::prompt::{PromptComposer, PromptSection};
use crate::scorer::{GoalDirectedHeuristicScorer, SectionScorer};
use crate::system_prompt_builder::SystemPromptBuilder;
use crate::templates::RolePromptTemplate;
use crate::templates::common::{CONTEXT_LAYOUT_STANZA, MCP_TOOLS_STANZA};
use crate::templates::conductor::ConductorTemplate;
use crate::templates::implementer::ImplementerTemplate;
use crate::templates::integration::IntegrationTemplate;
use crate::templates::quick::{QuickFixTemplate, QuickReviewerTemplate};
use crate::templates::refactorer::RefactorerTemplate;
use crate::templates::researcher::ResearcherTemplate;
use crate::templates::reviewer::{Reviewer, ReviewerTemplate};
use crate::templates::scribe::{ScribeTemplate, ScribeVariant};
use crate::templates::strategist::StrategistTemplate;
use roko_core::error::{Result, RokoError};
use roko_core::{AgentRole, Budget, Composer, Context, Scorer};
use roko_learn::playbook::Playbook;
use roko_learn::section_effect::SectionEffectivenessRegistry;
use roko_learn::skill_library::Skill;
use tracing::warn;

/// Default conventions appended to every constructed system prompt.
pub const DEFAULT_CONVENTIONS_SUFFIX: &str = "\
- Keep changes minimal and prefer wiring existing modules over reimplementation.
- Use the repo's existing patterns before inventing new ones.";

const DEFAULT_ANTI_PATTERNS: [&str; 3] = [
    "Do not reimplement existing modules when wiring existing code will do.",
    "Do not use git checkout, git switch, or git branch -m.",
    "Do not push branches directly.",
];

/// Runtime source metadata for a built-in role prompt.
///
/// This is deliberately small and static: callers can record where role text
/// came from without inspecting prompt bytes or relying on historical comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RolePromptSource {
    /// Stable source identifier for logs and workspace audit records.
    pub source_id: &'static str,
    /// Module or manifest path that owns the runtime prompt content.
    pub location: &'static str,
    /// Whether this source is owned by Roko runtime policy rather than a legacy import.
    pub roko_owned: bool,
}

/// Typed task/domain context for role prompts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskContext {
    /// The current task description.
    pub task: String,
    /// Optional plan id associated with the task.
    pub plan_id: Option<String>,
    /// Optional goal guiding prompt composition and section scoring.
    pub goal: Option<String>,
    /// Optional workspace label/path.
    pub workspace: Option<String>,
    /// Optional structured context assembled for this task.
    pub context_layer: Option<String>,
    /// Optional extra domain notes.
    pub domain_notes: Option<String>,
}

impl TaskContext {
    /// Create a new context from the required task text.
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            ..Self::default()
        }
    }

    /// Attach a plan id.
    #[must_use]
    pub fn with_plan_id(mut self, plan_id: impl Into<String>) -> Self {
        self.plan_id = Some(plan_id.into());
        self
    }

    /// Attach a goal for active-inference scoring.
    #[must_use]
    pub fn with_goal(mut self, goal: impl Into<String>) -> Self {
        self.goal = Some(goal.into());
        self
    }

    /// Attach a workspace label/path.
    #[must_use]
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    /// Attach a structured relevant-context section.
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context_layer = Some(context.into());
        self
    }

    /// Attach extra domain notes.
    #[must_use]
    pub fn with_domain_notes(mut self, notes: impl Into<String>) -> Self {
        self.domain_notes = Some(notes.into());
        self
    }

    fn goal_text(&self) -> Option<&str> {
        self.goal.as_deref().filter(|goal| !goal.trim().is_empty())
    }

    fn task_layer(&self) -> String {
        self.plan_id.as_ref().map_or_else(
            || {
                self.goal.as_ref().map_or_else(
                    || self.task.clone(),
                    |goal| format!("Goal: {goal}\nTask: {}", self.task),
                )
            },
            |plan_id| {
                let mut parts = vec![format!("Plan: {plan_id}")];
                if let Some(goal) = &self.goal {
                    parts.push(format!("Goal: {goal}"));
                }
                parts.push(format!("Task: {}", self.task));
                parts.join("\n")
            },
        )
    }

    fn domain_layer(&self) -> String {
        let mut parts = Vec::new();
        if let Some(plan_id) = &self.plan_id {
            parts.push(format!("Plan: {plan_id}"));
        }
        if !self.task.is_empty() {
            parts.push(format!("Task: {}", self.task));
        }
        if let Some(goal) = &self.goal {
            parts.push(format!("Goal: {goal}"));
        }
        if let Some(workspace) = &self.workspace {
            parts.push(format!("Workspace: {workspace}"));
        }
        if let Some(notes) = &self.domain_notes {
            parts.push(notes.clone());
        }
        parts.join("\n")
    }

    fn context_layer(&self) -> Option<&str> {
        self.context_layer
            .as_deref()
            .filter(|text| !text.trim().is_empty())
    }
}

/// Typed specification for building a role-scoped system prompt.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleSystemPromptSpec {
    /// Role/persona that will run the task.
    pub role: AgentRole,
    /// Task/domain context.
    pub task_context: TaskContext,
    /// Comma-separated hosted-backend tool allowlist.
    pub tool_allowlist_csv: String,
    /// Optional model slug hint for future model-specific prompt formatting.
    pub model_hint: Option<String>,
    /// Optional extra conventions appended after defaults.
    pub extra_conventions: Option<String>,
    /// Optional extra anti-patterns appended after defaults.
    pub extra_anti_patterns: Vec<String>,
    /// Optional relevant skills injected into the system prompt.
    pub relevant_skills: Vec<Skill>,
    /// Optional relevant playbooks injected into the system prompt.
    pub relevant_playbooks: Vec<Playbook>,
    /// Optional pheromone / active-signal chunks injected into the prompt.
    pub pheromones: Vec<ContextChunk>,
    /// Optional affect state used to tune tone and focus.
    pub affect_state: Option<PadState>,
    /// Complexity band used for static per-layer budget shaping.
    pub complexity: Complexity,
    /// Whether to include cache markers between stability tiers.
    pub cache_markers: bool,
}

impl RoleSystemPromptSpec {
    /// Create a new spec with required role/task/tool context.
    #[must_use]
    pub fn new(role: AgentRole, task_context: TaskContext, tool_csv: impl Into<String>) -> Self {
        Self {
            role,
            task_context,
            tool_allowlist_csv: tool_csv.into(),
            model_hint: None,
            extra_conventions: None,
            extra_anti_patterns: Vec::new(),
            relevant_skills: Vec::new(),
            relevant_playbooks: Vec::new(),
            pheromones: Vec::new(),
            affect_state: None,
            complexity: Complexity::Standard,
            cache_markers: false,
        }
    }

    /// Append extra conventions text.
    #[must_use]
    pub fn with_extra_conventions(mut self, text: impl Into<String>) -> Self {
        self.extra_conventions = Some(text.into());
        self
    }

    /// Attach an optional model slug hint for future prompt adaptation.
    #[must_use]
    pub fn with_model_hint(mut self, model_hint: impl Into<String>) -> Self {
        self.model_hint = Some(model_hint.into());
        self
    }

    /// Append one anti-pattern rule.
    #[must_use]
    pub fn add_anti_pattern(mut self, rule: impl Into<String>) -> Self {
        self.extra_anti_patterns.push(rule.into());
        self
    }

    /// Attach relevant learned skills to the prompt.
    #[must_use]
    pub fn with_relevant_skills(mut self, skills: &[Skill]) -> Self {
        self.relevant_skills = skills.to_vec();
        self
    }

    /// Attach relevant playbooks to the prompt.
    #[must_use]
    pub fn with_relevant_playbooks(mut self, playbooks: &[Playbook]) -> Self {
        self.relevant_playbooks = playbooks.to_vec();
        self
    }

    /// Attach active pheromone/context signals to the prompt.
    #[must_use]
    pub fn with_pheromones(mut self, pheromones: &[ContextChunk]) -> Self {
        self.pheromones = pheromones.to_vec();
        self
    }

    /// Attach affect state for tone/focus guidance.
    #[must_use]
    pub const fn with_affect_state(mut self, affect_state: Option<PadState>) -> Self {
        self.affect_state = affect_state;
        self
    }

    /// Apply one complexity band to the shared role-budget profile.
    #[must_use]
    pub const fn with_complexity(mut self, complexity: Complexity) -> Self {
        self.complexity = complexity;
        self
    }

    /// Enable cache-marker emission in the underlying builder.
    #[must_use]
    pub const fn with_cache_markers(mut self) -> Self {
        self.cache_markers = true;
        self
    }

    fn conventions_text(&self) -> String {
        let mut out = format!("{CONTEXT_LAYOUT_STANZA}\n\n{DEFAULT_CONVENTIONS_SUFFIX}");
        if let Some(extra) = &self.extra_conventions {
            let trimmed = extra.trim();
            if !trimmed.is_empty() {
                out.push_str("\n\n");
                out.push_str(trimmed);
            }
        }
        out
    }

    fn anti_patterns(&self) -> Vec<String> {
        let mut out: Vec<String> = DEFAULT_ANTI_PATTERNS
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        for rule in &self.extra_anti_patterns {
            if !out.iter().any(|seen| seen == rule) {
                out.push(rule.clone());
            }
        }
        out
    }

    fn builder_with_section_effectiveness(
        &self,
        section_effectiveness: Option<&SectionEffectivenessRegistry>,
    ) -> SystemPromptBuilder {
        let mut builder = SystemPromptBuilder::new(role_identity_for(self.role))
            .with_conventions(self.conventions_text())
            .with_tools(tool_allowlist_instructions(&self.tool_allowlist_csv))
            .with_anti_patterns(self.anti_patterns())
            .with_affect_state(self.affect_state);

        if self.complexity != Complexity::Standard {
            builder =
                builder.with_budget_profile(adjusted_budget_for(self.role, self.complexity).budget);
        }

        if let Some(registry) = section_effectiveness {
            builder = builder.with_section_effectiveness(format!("{:?}", self.role), registry);
        }
        if !self.relevant_skills.is_empty() {
            builder = builder.with_skills(&self.relevant_skills);
        }
        if !self.relevant_playbooks.is_empty() {
            builder = builder.with_playbooks(&self.relevant_playbooks);
        }
        if !self.pheromones.is_empty() {
            builder = builder.with_pheromones(&self.pheromones);
        }

        let domain = self.task_context.domain_layer();
        if !domain.is_empty() {
            builder = builder.with_domain(domain);
        }
        if let Some(context) = self.task_context.context_layer() {
            builder = builder.with_context(context);
        }
        let task = self.task_context.task_layer();
        if !task.is_empty() {
            builder = builder.with_task(task);
        }
        if self.cache_markers {
            builder = builder.with_cache_markers();
        }
        builder
    }

    /// Build the raw prompt text via [`SystemPromptBuilder`].
    #[must_use]
    pub fn build(&self) -> String {
        self.builder_with_section_effectiveness(None).build()
    }

    /// Build the raw prompt text with learned section-effectiveness applied.
    #[must_use]
    pub fn build_with_section_effectiveness(
        &self,
        section_effectiveness: &SectionEffectivenessRegistry,
    ) -> String {
        self.builder_with_section_effectiveness(Some(section_effectiveness))
            .build()
    }

    /// Build structured prompt sections via [`SystemPromptBuilder`].
    #[must_use]
    pub fn build_sections(&self) -> Vec<PromptSection> {
        self.builder_with_section_effectiveness(None)
            .build_sections()
    }

    /// Build structured prompt sections with learned section-effectiveness applied.
    #[must_use]
    pub fn build_sections_with_section_effectiveness(
        &self,
        section_effectiveness: &SectionEffectivenessRegistry,
    ) -> Vec<PromptSection> {
        self.builder_with_section_effectiveness(Some(section_effectiveness))
            .build_sections()
    }

    /// Compose the section form under a token budget using [`PromptComposer`].
    ///
    /// This is useful when callers want a budget-aware system prompt string
    /// while preserving the role/system/session/task layering semantics.
    pub fn compose_with_budget(&self, token_budget: usize) -> Result<String> {
        let scorer = self.composition_scorer();
        let ctx = self.composition_context();
        self.compose_with_budget_and_scorer(token_budget, scorer.as_ref(), &ctx)
    }

    /// Compose the section form under a token budget using an explicit scorer/context pair.
    pub fn compose_with_budget_and_scorer(
        &self,
        token_budget: usize,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<String> {
        let sections = self.build_sections();
        let signals = sections
            .into_iter()
            .map(PromptSection::into_signal)
            .collect::<Result<Vec<_>>>()?;
        let composed =
            PromptComposer::new().compose(&signals, &Budget::tokens(token_budget), scorer, ctx)?;
        composed.body.as_text().map(str::to_string)
    }

    /// Build the prompt and validate it against a model context window.
    ///
    /// If the composed prompt exceeds 30% of the available window, the
    /// builder re-runs composition with a tighter budget to drop lower-value
    /// sections. If it exceeds 50% of the window, the prompt is rejected.
    pub fn build_with_context_window(&self, context_window_tokens: usize) -> Result<String> {
        self.build_with_context_window_and_section_effectiveness(context_window_tokens, None)
    }

    /// Build the prompt and validate it against a model context window,
    /// optionally applying learned section-effectiveness.
    pub fn build_with_context_window_and_section_effectiveness(
        &self,
        context_window_tokens: usize,
        section_effectiveness: Option<&SectionEffectivenessRegistry>,
    ) -> Result<String> {
        let prompt = self
            .builder_with_section_effectiveness(section_effectiveness)
            .build();
        let prompt_tokens = estimate_tokens(&prompt);
        let soft_limit = context_window_tokens.saturating_mul(3) / 10;
        let hard_limit = context_window_tokens / 2;
        let soft_limit = soft_limit.max(1);
        let hard_limit = hard_limit.max(1);

        if prompt_tokens > hard_limit {
            return Err(RokoError::BudgetExceeded {
                dimension: "system_prompt_tokens",
                used: prompt_tokens,
                limit: hard_limit,
            });
        }

        if prompt_tokens > soft_limit {
            warn!(
                role = %self.role.label(),
                prompt_tokens,
                soft_limit,
                hard_limit,
                "system prompt exceeds the soft context-window budget; recomposing with tighter budget"
            );

            let sections = if let Some(registry) = section_effectiveness {
                self.build_sections_with_section_effectiveness(registry)
            } else {
                self.build_sections()
            };
            let scorer = self.composition_scorer();
            let ctx = self.composition_context();
            let prompt = self.compose_sections_with_budget_and_scorer(
                sections,
                soft_limit,
                scorer.as_ref(),
                &ctx,
            )?;
            let prompt_tokens = estimate_tokens(&prompt);

            if prompt_tokens > hard_limit {
                return Err(RokoError::BudgetExceeded {
                    dimension: "system_prompt_tokens",
                    used: prompt_tokens,
                    limit: hard_limit,
                });
            }

            return Ok(prompt);
        }

        Ok(prompt)
    }

    fn compose_sections_with_budget_and_scorer(
        &self,
        sections: Vec<PromptSection>,
        token_budget: usize,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<String> {
        let signals = sections
            .into_iter()
            .map(PromptSection::into_signal)
            .collect::<Result<Vec<_>>>()?;
        let composed =
            PromptComposer::new().compose(&signals, &Budget::tokens(token_budget), scorer, ctx)?;
        composed.body.as_text().map(str::to_string)
    }

    fn composition_context(&self) -> Context {
        let mut ctx = Context::now();
        if let Some(goal) = self.task_context.goal_text() {
            ctx = ctx.with_goal(goal);
        }
        ctx
    }

    fn composition_scorer(&self) -> Box<dyn Scorer> {
        if let Some(goal) = self.task_context.goal_text() {
            Box::new(GoalDirectedHeuristicScorer::new(goal))
        } else {
            Box::new(SectionScorer::new())
        }
    }
}

/// Render the tool allowlist guidance block from a comma-separated list.
#[must_use]
pub fn tool_allowlist_instructions(tools_csv: &str) -> String {
    let csv = tools_csv.trim();
    if csv.is_empty() {
        format!(
            "{MCP_TOOLS_STANZA}\nNo hosted-backend tool allowlist was supplied. Use only the minimum tools required for the role."
        )
    } else {
        format!(
            "{MCP_TOOLS_STANZA}\nClaude tool allowlist: {csv}\n\nUse only the tools granted to your role."
        )
    }
}

/// Resolve role identity text from template modules plus typed fallbacks.
#[must_use]
pub fn role_identity_for(role: AgentRole) -> String {
    match role {
        AgentRole::Strategist => StrategistTemplate.role_identity().to_string(),
        AgentRole::Implementer => ImplementerTemplate.role_identity().to_string(),
        AgentRole::Architect => ReviewerTemplate::new(Reviewer::Architect)
            .role_identity()
            .to_string(),
        AgentRole::Auditor => ReviewerTemplate::new(Reviewer::Auditor)
            .role_identity()
            .to_string(),
        AgentRole::QuickReviewer => QuickReviewerTemplate.role_identity().to_string(),
        AgentRole::Scribe => {
            ScribeTemplate::role_identity_for_variant(ScribeVariant::Initial).to_string()
        }
        AgentRole::Critic => {
            ScribeTemplate::role_identity_for_variant(ScribeVariant::Critic).to_string()
        }
        AgentRole::AutoFixer => QuickFixTemplate.role_identity().to_string(),
        AgentRole::IntegrationTester => IntegrationTemplate.role_identity().to_string(),
        AgentRole::Refactorer => RefactorerTemplate.role_identity().to_string(),
        AgentRole::Researcher => ResearcherTemplate.role_identity().to_string(),
        AgentRole::Conductor => ConductorTemplate.role_identity().to_string(),
        other => format!(
            "You are an AI agent in the {} role. Complete your assigned task.",
            other.label()
        ),
    }
}

/// Resolve prompt source metadata for a built-in role.
#[must_use]
pub fn role_prompt_source_for(role: AgentRole) -> RolePromptSource {
    match role {
        AgentRole::Strategist => RolePromptSource {
            source_id: "roko.builtin.role.strategist",
            location: "crates/roko-compose/src/templates/strategist.rs",
            roko_owned: true,
        },
        AgentRole::Implementer => RolePromptSource {
            source_id: "roko.builtin.role.implementer",
            location: "crates/roko-compose/src/templates/implementer.rs",
            roko_owned: true,
        },
        AgentRole::Architect => RolePromptSource {
            source_id: "roko.builtin.role.architect",
            location: "crates/roko-compose/src/templates/reviewer.rs",
            roko_owned: true,
        },
        AgentRole::Auditor => RolePromptSource {
            source_id: "roko.builtin.role.auditor",
            location: "crates/roko-compose/src/templates/reviewer.rs",
            roko_owned: true,
        },
        AgentRole::QuickReviewer => RolePromptSource {
            source_id: "roko.builtin.role.quick-reviewer",
            location: "crates/roko-compose/src/templates/quick.rs",
            roko_owned: true,
        },
        AgentRole::Scribe => RolePromptSource {
            source_id: "roko.builtin.role.scribe",
            location: "crates/roko-compose/src/templates/scribe.rs",
            roko_owned: true,
        },
        AgentRole::Critic => RolePromptSource {
            source_id: "roko.builtin.role.critic",
            location: "crates/roko-compose/src/templates/scribe.rs",
            roko_owned: true,
        },
        AgentRole::AutoFixer => RolePromptSource {
            source_id: "roko.builtin.role.auto-fixer",
            location: "crates/roko-compose/src/templates/quick.rs",
            roko_owned: true,
        },
        AgentRole::IntegrationTester => RolePromptSource {
            source_id: "roko.builtin.role.integration-tester",
            location: "crates/roko-compose/src/templates/integration.rs",
            roko_owned: true,
        },
        AgentRole::Refactorer => RolePromptSource {
            source_id: "roko.builtin.role.refactorer",
            location: "crates/roko-compose/src/templates/refactorer.rs",
            roko_owned: true,
        },
        AgentRole::Researcher => RolePromptSource {
            source_id: "roko.builtin.role.researcher",
            location: "crates/roko-compose/src/templates/researcher.rs",
            roko_owned: true,
        },
        AgentRole::Conductor => RolePromptSource {
            source_id: "roko.builtin.role.conductor",
            location: "crates/roko-compose/src/templates/conductor.rs",
            roko_owned: true,
        },
        other => RolePromptSource {
            source_id: "roko.builtin.role.generic",
            location: other.label(),
            roko_owned: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::SectionPriority;
    use roko_learn::playbook::Playbook;
    use roko_learn::skill_library::Skill;
    use std::collections::HashSet;

    #[test]
    fn role_identities_are_non_empty_and_distinct_for_core_roles() {
        let roles = [
            AgentRole::Strategist,
            AgentRole::Implementer,
            AgentRole::Architect,
            AgentRole::Auditor,
            AgentRole::QuickReviewer,
            AgentRole::Scribe,
            AgentRole::Critic,
            AgentRole::AutoFixer,
            AgentRole::IntegrationTester,
            AgentRole::Refactorer,
            AgentRole::Conductor,
        ];
        let ids: Vec<String> = roles.into_iter().map(role_identity_for).collect();
        assert!(ids.iter().all(|id| !id.trim().is_empty()));
        let unique: HashSet<String> = ids.into_iter().collect();
        assert_eq!(unique.len(), roles.len());
    }

    #[test]
    fn built_in_role_prompt_sources_are_roko_owned() {
        let roles = std::iter::once(AgentRole::Conductor).chain(AgentRole::ALL_AGENTS);
        for role in roles {
            let source = role_prompt_source_for(role);
            assert!(
                source.roko_owned,
                "{} source is not Roko-owned",
                role.label()
            );
            assert!(
                source.source_id.starts_with("roko.builtin.role."),
                "{} source id should be Roko-owned: {}",
                role.label(),
                source.source_id
            );
            assert!(
                !source.location.contains("mori") && !source.location.contains("bardo"),
                "{} source location leaks legacy project name: {}",
                role.label(),
                source.location
            );
        }
    }

    #[test]
    fn built_in_runtime_role_prompts_do_not_emit_legacy_project_tokens() {
        let roles = std::iter::once(AgentRole::Conductor).chain(AgentRole::ALL_AGENTS);
        for role in roles {
            let spec = RoleSystemPromptSpec::new(
                role,
                TaskContext::new("Check prompt provenance.").with_plan_id("RT10"),
                "Read,Edit,Bash",
            );
            let prompt = spec.build().to_lowercase();
            for forbidden in [".mori", "bardo", "mori"] {
                assert!(
                    !prompt.contains(forbidden),
                    "{} prompt leaked forbidden token {forbidden:?}",
                    role.label()
                );
            }
        }
    }

    #[test]
    fn built_prompt_includes_context_and_tool_guidance() {
        let ctx = TaskContext::new("Implement task wiring")
            .with_plan_id("042-golem-mortality")
            .with_goal("keep routing and prompt composition aligned")
            .with_workspace("roko-cli orchestration")
            .with_context(
                "## Relevant Context\n\n### Knowledge\n- [Heuristic] Keep the prompt compact.",
            )
            .with_domain_notes("Focus on runtime prompt-path parity.");
        let spec = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx, "Read,Edit,Bash")
            .with_extra_conventions("Prefer additive changes.");
        let prompt = spec.build();
        assert!(prompt.contains("Plan: 042-golem-mortality"));
        assert!(prompt.contains("Goal: keep routing and prompt composition aligned"));
        assert!(prompt.contains("Task: Implement task wiring"));
        assert!(prompt.contains("Workspace: roko-cli orchestration"));
        assert!(prompt.contains("## Relevant Context"));
        assert!(prompt.contains("Claude tool allowlist: Read,Edit,Bash"));
        assert!(prompt.contains("Prefer additive changes."));
    }

    #[test]
    fn built_prompt_includes_affect_guidance() {
        let ctx = TaskContext::new("Implement affect wiring");
        let spec = RoleSystemPromptSpec::new(AgentRole::Conductor, ctx, "Read,Edit")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)));

        let prompt = spec.build();
        assert!(prompt.contains("You are under time pressure, focus on the most critical path."));
    }

    #[test]
    fn model_hint_passthrough() {
        let ctx = TaskContext::new("Implement model hint passthrough");
        let baseline = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx.clone(), "Read,Edit");
        let hinted = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx, "Read,Edit")
            .with_model_hint("glm-5.1");

        assert_eq!(hinted.model_hint.as_deref(), Some("glm-5.1"));
        assert_eq!(baseline.build(), hinted.build());
        assert_eq!(baseline.build_sections(), hinted.build_sections());
    }

    #[test]
    fn relevant_skills_are_forwarded_into_the_prompt_builder() {
        let ctx = TaskContext::new("Implement skill injection");
        let skill = Skill::new(
            "git_fixup",
            "Use fixup commits for focused rewrites.",
            "Use fixup commits and autosquash to keep history tidy.",
        );

        let prompt = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx, "Read,Edit")
            .with_relevant_skills(&[skill])
            .build();

        assert!(prompt.contains("## Relevant Techniques"));
        assert!(prompt.contains("Use fixup commits"));
    }

    #[test]
    fn relevant_playbooks_are_forwarded_into_the_prompt_builder() {
        let ctx = TaskContext::new("Implement REST API");
        let mut playbook = Playbook::new("pb-implement-api", "Implement REST API");
        playbook.name = "implement-api".to_string();

        let prompt = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx, "Read,Edit")
            .with_relevant_playbooks(&[playbook])
            .build();

        assert!(prompt.contains("## Relevant Techniques"));
        assert!(prompt.contains("Playbook: implement-api (pb-implement-api)"));
    }

    #[test]
    fn compose_with_budget_keeps_critical_sections() {
        let ctx = TaskContext::new("Implement prompt wiring")
            .with_plan_id("123")
            .with_workspace("workspace")
            .with_domain_notes(&"X".repeat(8_000));
        let spec = RoleSystemPromptSpec::new(AgentRole::Conductor, ctx, "Read,Edit")
            .with_extra_conventions("Y".repeat(8_000))
            .add_anti_pattern("Z".repeat(8_000));

        let sections = spec.build_sections();
        let critical_tokens = sections
            .iter()
            .filter(|s| s.priority == SectionPriority::Critical)
            .map(PromptSection::estimated_tokens)
            .sum::<usize>();

        let prompt = spec
            .compose_with_budget(critical_tokens)
            .expect("composition should keep critical layers");

        assert!(prompt.contains("--- role_identity ---"));
        assert!(prompt.contains("--- task_context ---"));
        assert!(!prompt.contains("--- conventions ---"));
        assert!(!prompt.contains("--- domain_context ---"));
        assert!(!prompt.contains("--- tool_instructions ---"));
        assert!(!prompt.contains("--- anti_patterns ---"));
    }

    #[test]
    fn compose_with_budget_uses_goal_aware_active_inference_scoring() {
        let ctx = TaskContext::new("Compose goal-aware prompt sections")
            .with_goal("reduce routing latency");
        let spec = RoleSystemPromptSpec::new(AgentRole::Implementer, ctx, "Read,Edit")
            .with_extra_conventions("Prefer the fastest viable path.")
            .with_pheromones(&[ContextChunk {
                content: "- [Threat] context assembly is too slow.".to_string(),
                source: crate::ContextSource::RecentSignal {
                    signal_id: "pheromone-1".to_string(),
                    plan_id: "plan-x".to_string(),
                    kind: "pheromone".to_string(),
                },
                relevance: 0.95,
                track_record: Some(0.9),
                confidence: Some(0.8),
                recency: Some(0.95),
                emotional_tag: None,
            }]);
        let sections = spec.build_sections();
        let critical_budget = sections
            .iter()
            .filter(|section| section.priority == SectionPriority::Critical)
            .map(PromptSection::estimated_tokens)
            .sum::<usize>();
        let active_signal_budget = sections
            .iter()
            .find(|section| section.name == "active_signals")
            .map(PromptSection::estimated_tokens)
            .unwrap_or(0);

        let prompt = spec
            .compose_with_budget(critical_budget + active_signal_budget + 96)
            .expect("composition should succeed");

        assert!(prompt.contains("Goal: reduce routing latency"));
        assert!(prompt.contains("context assembly is too slow"));
    }
}
