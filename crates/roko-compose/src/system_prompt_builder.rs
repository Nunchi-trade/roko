//! Composable system prompt builder with 9 layers.
//!
//! Generates cache-aligned, role-specific system prompts from composable
//! fragments. Each layer targets a different stability tier:
//!
//! | Layer | Content | Cache Tier |
//! |-------|---------|------------|
//! | 1. Role identity | Who am I, what's my job | System (stable) |
//! | 2. Conventions | Project coding standards | System (semi-stable) |
//! | 3. Domain context | Project-specific knowledge | Session (semi-stable) |
//! | 3c. Active signals | Pheromone / stigmergic guidance | Session (semi-stable) |
//! | 4. Task context | Current task details | Task (volatile) |
//! | 5. Tool instructions | Available tools and usage | System (stable) |
//! | 6. Relevant techniques | Learned playbooks and skills | Task (volatile) |
//! | 7. Anti-patterns | What NOT to do | Task (volatile) |
//! | 8. Affect guidance | Emotional tone and focus | Dynamic |
//!
//! The builder emits sections in cache-layer order, with optional cache
//! alignment markers between stability tiers. Layers 1 + 2 + 5 form the
//! prefix-cacheable "system" tier; layers 3 and 3c form the "session" tier;
//! layers 4 + 6 + 7 are per-task; layer 8 is dynamic tone/focus guidance.
//!
//! # Design
//!
//! Inspired by the dynamic prompt generation pipeline in
//! `mori-agents/17-dynamic-prompt-generation.md` section 3. The key insight:
//! system prompts matter enormously (3-4x quality gap per the `--bare`
//! experiment), AND they should be task-specific, not one-size-fits-all.
//!
//! Anti-pattern #8: **no `std::fs`**. All content arrives via builder methods.

use crate::prompt::estimate_tokens;
use crate::prompt::{
    AttentionBidder, CacheLayer, Placement, PromptComposer, PromptSection, SectionPriority,
};
use crate::templates::common::PromptBudget;
use crate::token_counter::TokenCounter;
use crate::{ContextChunk, PadState};
use roko_core::tool::ToolDef;
use roko_core::{Budget, Composer, Context, Engram, Result, Scorer};
use roko_learn::playbook::Playbook;
use roko_learn::section_effect::{PriorityChange, SectionEffectivenessRegistry};
use roko_learn::skill_library::Skill;

/// A composable system prompt built from 9 layers.
///
/// Use the builder pattern:
/// ```ignore
/// let prompt = SystemPromptBuilder::new("You are an implementer...")
///     .with_conventions("Use snake_case, thiserror for errors")
///     .with_domain("DeFi protocol context: ...")
///     .with_task("Implement the rate limiter in crates/golem-core")
///     .with_tools("MCP tools available: Read, Write, Bash")
///     .with_anti_patterns(vec!["Never call unwrap in library crates"])
///     .build();
/// ```
pub struct SystemPromptBuilder {
    /// Layer 1: Role identity — who the agent is and what it does.
    role_identity: String,
    /// Layer 2: Project conventions — coding standards, naming, etc.
    conventions: Option<String>,
    /// Layer 3: Domain context — project-specific knowledge.
    domain: Option<String>,
    /// Layer 3b: Relevant assembled context for the current task.
    context: Option<String>,
    /// Layer 3c: Active pheromone/context signals.
    pheromones: Vec<ContextChunk>,
    /// Layer 4: Task context — current task details.
    task: Option<String>,
    /// Layer 5: Tool instructions — available tools and how to use them.
    tools: Option<String>,
    /// Layer 6: Relevant skills — learned techniques to prefer for this task.
    relevant_skills: Vec<Skill>,
    /// Layer 6: Relevant playbooks — reusable prior task sequences.
    relevant_playbooks: Vec<Playbook>,
    /// Layer 6b: Tool usage hints from learned profiles (LEARN-12).
    tool_hints: Option<String>,
    /// Layer 7: Anti-patterns — things the agent must NOT do.
    anti_patterns: Vec<String>,
    /// Layer 8: Affect guidance — current emotional tone and focus.
    affect_state: Option<PadState>,
    /// Whether to insert cache alignment markers between tiers.
    cache_markers: bool,
    /// Optional token budget enforced by [`build_with_counter`](Self::build_with_counter).
    token_budget: Option<usize>,
    /// Optional per-layer section caps derived from role/complexity budget policy.
    budget_profile: Option<PromptBudget>,
    /// Learned section-effectiveness data scoped to one role.
    section_effectiveness: Option<SectionEffectivenessConfig>,
}

#[derive(Clone)]
struct SectionEffectivenessConfig {
    role: String,
    registry: SectionEffectivenessRegistry,
}

const RELEVANT_TECHNIQUES_TOKEN_BUDGET: usize = 500;

/// Normalize prompt text so logically-identical content yields identical bytes.
#[must_use]
pub fn normalize_for_caching(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    normalized
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .replace('\t', "    ")
}

/// Canonicalize tool definition order before rendering prompt/tool payloads.
pub fn canonical_tool_order(tools: &mut [ToolDef]) {
    tools.sort_by(|a, b| a.name.cmp(&b.name));
}

fn normalize_owned(content: impl Into<String>) -> String {
    let content = content.into();
    normalize_for_caching(&content)
}

impl SystemPromptBuilder {
    /// Start building a system prompt with the role identity (layer 1).
    ///
    /// The role identity is the stable, role-specific opening paragraph
    /// that tells the agent what it is and what its constraints are.
    #[must_use]
    pub fn new(role_identity: impl Into<String>) -> Self {
        Self {
            role_identity: normalize_owned(role_identity),
            conventions: None,
            domain: None,
            context: None,
            pheromones: Vec::new(),
            task: None,
            tools: None,
            relevant_skills: Vec::new(),
            relevant_playbooks: Vec::new(),
            tool_hints: None,
            anti_patterns: Vec::new(),
            affect_state: None,
            cache_markers: false,
            token_budget: None,
            budget_profile: None,
            section_effectiveness: None,
        }
    }

    /// Set layer 2: project conventions (coding standards, naming, etc.).
    #[must_use]
    pub fn with_conventions(mut self, conventions: impl Into<String>) -> Self {
        self.conventions = Some(normalize_owned(conventions));
        self
    }

    /// Set layer 3: domain context (project-specific knowledge).
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(normalize_owned(domain));
        self
    }

    /// Set layer 3b: relevant assembled context for the current task.
    #[must_use]
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(normalize_owned(context));
        self
    }

    /// Set layer 3c: active pheromone/context signals.
    #[must_use]
    pub fn with_pheromones(mut self, pheromones: &[ContextChunk]) -> Self {
        self.pheromones = pheromones.to_vec();
        self
    }

    /// Set layer 4: task context (current task details).
    #[must_use]
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(normalize_owned(task));
        self
    }

    /// Set layer 5: tool instructions (available tools and usage guidance).
    #[must_use]
    pub fn with_tools(mut self, tools: impl Into<String>) -> Self {
        self.tools = Some(normalize_owned(tools));
        self
    }

    /// Set layer 6: relevant skills (learned techniques to inject).
    #[must_use]
    pub fn with_skills(mut self, skills: &[Skill]) -> Self {
        self.relevant_skills = skills.to_vec();
        self
    }

    /// Set layer 6: relevant playbooks for this task.
    #[must_use]
    pub fn with_playbooks(mut self, playbooks: &[Playbook]) -> Self {
        self.relevant_playbooks = playbooks.to_vec();
        self
    }

    /// Set layer 6b: tool usage hints from learned profiles (LEARN-12).
    ///
    /// These hints are injected between Skills and Anti-patterns to guide
    /// the agent toward effective tool sequences for the current task type.
    #[must_use]
    pub fn with_tool_hints(mut self, hints: impl Into<String>) -> Self {
        let hints = normalize_owned(hints);
        if !hints.is_empty() {
            self.tool_hints = Some(hints);
        }
        self
    }

    /// Set layer 7: anti-patterns (things the agent must NOT do).
    #[must_use]
    pub fn with_anti_patterns(mut self, patterns: Vec<String>) -> Self {
        self.anti_patterns = patterns.into_iter().map(normalize_owned).collect();
        self
    }

    /// Add a single anti-pattern to layer 7.
    #[must_use]
    pub fn add_anti_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.anti_patterns.push(normalize_owned(pattern));
        self
    }

    /// Set layer 8: affect guidance (current emotional tone and focus).
    #[must_use]
    pub const fn with_affect_state(mut self, affect_state: Option<PadState>) -> Self {
        self.affect_state = affect_state;
        self
    }

    /// Enable cache alignment markers between stability tiers.
    ///
    /// When enabled, the builder inserts `<!-- cache:TIER -->` markers
    /// between layers so downstream renderers can set `cache_control`
    /// breakpoints on API calls.
    #[must_use]
    pub const fn with_cache_markers(mut self) -> Self {
        self.cache_markers = true;
        self
    }

    /// Enforce a token budget when building via [`build_with_counter`](Self::build_with_counter).
    #[must_use]
    pub const fn with_token_budget(mut self, token_budget: usize) -> Self {
        self.token_budget = Some(token_budget);
        self
    }

    /// Apply per-layer section caps from the shared role budget table.
    #[must_use]
    pub const fn with_budget_profile(mut self, budget_profile: PromptBudget) -> Self {
        self.budget_profile = Some(budget_profile);
        self
    }

    /// Apply learned section-effectiveness adjustments for `role`.
    #[must_use]
    pub fn with_section_effectiveness(
        mut self,
        role: impl Into<String>,
        registry: &SectionEffectivenessRegistry,
    ) -> Self {
        self.section_effectiveness = Some(SectionEffectivenessConfig {
            role: role.into(),
            registry: registry.clone(),
        });
        self
    }

    /// Build the final system prompt as a single string.
    ///
    /// Sections are emitted in cache-layer order, with markers between
    /// stability tiers if enabled. Empty layers are skipped.
    #[must_use]
    pub fn build(&self) -> String {
        normalize_for_caching(&assemble_sections(
            self.build_sections(),
            self.cache_markers,
        ))
    }

    /// Build the final system prompt under the configured token budget.
    ///
    /// If no budget was configured with [`with_token_budget`](Self::with_token_budget),
    /// this falls back to [`build`](Self::build).
    #[must_use]
    pub fn build_with_counter(&self, counter: &TokenCounter) -> String {
        let Some(token_budget) = self.token_budget else {
            return self.build();
        };

        let rendered_sections = self
            .tuned_sections(token_budget, counter)
            .into_iter()
            .map(|section| {
                let section = section.enforce_hard_cap();
                RenderedSection {
                    rendered: render_section(&section),
                    section,
                }
            })
            .collect::<Vec<_>>();

        let mut kept = vec![None; rendered_sections.len()];
        let mut selection_order = (0..rendered_sections.len()).collect::<Vec<_>>();
        selection_order.sort_by(|&a, &b| {
            rendered_sections[b]
                .section
                .priority
                .cmp(&rendered_sections[a].section.priority)
                .then_with(|| {
                    rendered_sections[a]
                        .section
                        .cache_layer
                        .cmp(&rendered_sections[b].section.cache_layer)
                })
                .then_with(|| a.cmp(&b))
        });

        for index in selection_order {
            let rendered = &rendered_sections[index].rendered;
            if candidate_fits(
                &rendered_sections,
                &kept,
                index,
                rendered,
                self.cache_markers,
                token_budget,
                counter,
            ) {
                kept[index] = Some(rendered.clone());
                continue;
            }

            if rendered_sections[index].section.priority == SectionPriority::Critical {
                kept[index] = truncate_to_fit(
                    &rendered_sections,
                    &kept,
                    index,
                    rendered,
                    self.cache_markers,
                    token_budget,
                    counter,
                );
            }
        }

        assemble_selected_sections(&rendered_sections, &kept, self.cache_markers)
    }

    /// Build the system prompt as a vector of [`PromptSection`]s.
    ///
    /// Each layer becomes a separate section with appropriate priority,
    /// cache layer, and placement metadata. This is useful when feeding
    /// directly into [`PromptComposer`](crate::PromptComposer) or
    /// [`PromptAssembler`](crate::templates::assembly::PromptAssembler).
    #[must_use]
    pub fn build_sections(&self) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(10);

        // Layer 1: Role Identity
        if let Some(section) = self.apply_budget_profile(
            PromptSection::new("role_identity", &self.role_identity)
                .with_priority(self.effective_priority("role_identity", SectionPriority::Critical))
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::Start),
        ) {
            sections.push(section);
        }

        // Layer 2: Conventions
        if let Some(ref conv) = self.conventions {
            if !conv.is_empty() {
                if let Some(section) = self.apply_budget_profile(
                    PromptSection::new("conventions", conv)
                        .with_priority(
                            self.effective_priority("conventions", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Start),
                ) {
                    sections.push(section);
                }
            }
        }

        // Layer 5: Tool Instructions (grouped in System cache tier)
        if let Some(ref tools) = self.tools {
            if !tools.is_empty() {
                if let Some(section) = self.apply_budget_profile(
                    PromptSection::new("tool_instructions", tools)
                        .with_priority(
                            self.effective_priority("tool_instructions", SectionPriority::Normal),
                        )
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Middle),
                ) {
                    sections.push(section);
                }
            }
        }

        // Layer 3: Domain Context
        if let Some(ref domain) = self.domain {
            if !domain.is_empty() {
                if let Some(section) = self.apply_budget_profile(
                    PromptSection::new("domain_context", domain)
                        .with_priority(
                            self.effective_priority("domain_context", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                ) {
                    sections.push(section);
                }
            }
        }

        // Layer 3b: Relevant Context
        if let Some(ref context) = self.context {
            if !context.is_empty() {
                if let Some(section) = self.apply_budget_profile(
                    PromptSection::new("context_layer", format!("## Relevant Context\n{context}"))
                        .with_priority(
                            self.effective_priority("context_layer", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                ) {
                    sections.push(section);
                }
            }
        }

        // Layer 3c: Active pheromone signals
        if let Some(pheromones) = self.pheromone_section() {
            sections.push(pheromones);
        }

        // Layer 4: Task Context
        if let Some(ref task) = self.task {
            if !task.is_empty() {
                if let Some(section) = self.apply_budget_profile(
                    PromptSection::new("task_context", task)
                        .with_priority(
                            self.effective_priority("task_context", SectionPriority::Critical),
                        )
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::End),
                ) {
                    sections.push(section);
                }
            }
        }

        // Layer 6: Relevant Techniques
        if let Some(skills) = self.relevant_techniques_section() {
            sections.push(skills);
        }

        // Layer 6b: Tool Usage Hints (LEARN-12)
        if let Some(ref hints) = self.tool_hints {
            if let Some(section) = self.apply_budget_profile(
                PromptSection::new("tool_hints", hints.clone())
                    .with_priority(
                        self.effective_priority("tool_hints", SectionPriority::Low),
                    )
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::Middle),
            ) {
                sections.push(section);
            }
        }

        // Layer 7: Anti-Patterns
        if !self.anti_patterns.is_empty() {
            let anti_text: String = self
                .anti_patterns
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n");
            if let Some(section) = self.apply_budget_profile(
                PromptSection::new("anti_patterns", format!("Do NOT:\n{anti_text}"))
                    .with_priority(
                        self.effective_priority("anti_patterns", SectionPriority::Normal),
                    )
                    .with_cache_layer(CacheLayer::Plan)
                    .with_placement(Placement::End),
            ) {
                sections.push(section);
            }
        }

        // Layer 8: Affect Guidance
        if let Some(affect) = self.affect_guidance() {
            if let Some(section) = self.apply_budget_profile(
                PromptSection::new("affect_guidance", affect)
                    .with_priority(
                        self.effective_priority("affect_guidance", SectionPriority::Normal),
                    )
                    .with_cache_layer(CacheLayer::Volatile)
                    .with_placement(Placement::End),
            ) {
                sections.push(section);
            }
        }

        sort_sections(&mut sections);
        sections
    }

    /// Count how many layers are populated (non-empty).
    #[must_use]
    pub fn layer_count(&self) -> usize {
        let mut count = 1; // Layer 1 is always present.
        if self.conventions.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if self.domain.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if self.context.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if !self.pheromones.is_empty() {
            count += 1;
        }
        if self.task.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if self.tools.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if !self.relevant_skills.is_empty() || !self.relevant_playbooks.is_empty() {
            count += 1;
        }
        if !self.anti_patterns.is_empty() {
            count += 1;
        }
        if self.affect_guidance().is_some() {
            count += 1;
        }
        count
    }

    fn affect_guidance(&self) -> Option<String> {
        let affect = self.affect_state?;
        let mut guidance = Vec::new();

        if affect.arousal >= 0.35 {
            guidance.push("You are under time pressure, focus on the most critical path.");
        } else if affect.arousal <= -0.35 {
            guidance.push("You have time to explore thoroughly.");
        }

        if affect.pleasure <= -0.25 {
            guidance.push(
                "Prefer proven approaches, verify early, and surface uncertainty explicitly.",
            );
        } else if affect.pleasure >= 0.35 {
            guidance.push("Keep the solution lean and avoid over-engineering.");
        }

        if affect.dominance <= -0.20 {
            guidance.push("Reduce scope until the next concrete checkpoint is clear.");
        } else if affect.dominance >= 0.30 {
            guidance.push("Execute decisively, but keep claims grounded in evidence.");
        }

        if affect.somatic_intensity >= 0.25 {
            if affect.somatic_valence <= -0.25 {
                guidance.push(
                    "This task resembles prior failure territory; favor conservative validation and known-safe sequences.",
                );
            } else if affect.somatic_valence >= 0.25 {
                guidance.push(
                    "This task resembles prior success territory; reuse known-good patterns, but keep verification intact.",
                );
            }
        }

        if guidance.is_empty() {
            None
        } else {
            Some(guidance.join(" "))
        }
    }

    fn effective_priority(&self, section: &str, base_priority: SectionPriority) -> SectionPriority {
        let Some(config) = &self.section_effectiveness else {
            return base_priority;
        };
        section_priority_from_u8(adjusted_priority(
            base_priority as u8,
            section,
            &config.role,
            &config.registry,
        ))
    }

    fn section_lift_weight(&self, section: &str) -> f64 {
        let Some(config) = &self.section_effectiveness else {
            return 1.0;
        };

        config
            .registry
            .get(section, &config.role)
            .map(|effect| (1.0 + effect.lift()).clamp(0.5, 1.5))
            .unwrap_or(1.0)
    }

    fn relevant_techniques_section(&self) -> Option<PromptSection> {
        if self.relevant_skills.is_empty() && self.relevant_playbooks.is_empty() {
            return None;
        }

        let mut rendered = String::from("## Relevant Techniques");
        let mut kept_playbooks = 0usize;
        let mut kept_skills = 0usize;
        let mut total_tokens = estimate_tokens(&rendered);

        for playbook in self.relevant_playbooks.iter().take(3) {
            let block = render_playbook(playbook);
            let candidate = format!("{rendered}\n\n{block}");
            let candidate_tokens = estimate_tokens(&candidate);
            if candidate_tokens > RELEVANT_TECHNIQUES_TOKEN_BUDGET {
                break;
            }
            rendered = candidate;
            kept_playbooks += 1;
            total_tokens = candidate_tokens;
        }

        for skill in &self.relevant_skills {
            let block = render_skill(skill);
            let candidate = format!("{rendered}\n\n{block}");
            let candidate_tokens = estimate_tokens(&candidate);
            if candidate_tokens > RELEVANT_TECHNIQUES_TOKEN_BUDGET {
                break;
            }
            rendered = candidate;
            kept_skills += 1;
            total_tokens = candidate_tokens;
        }

        if kept_playbooks < self.relevant_playbooks.len().min(3)
            || kept_skills < self.relevant_skills.len()
        {
            tracing::info!(
                kept_playbooks,
                dropped_playbooks = self.relevant_playbooks.len().min(3) - kept_playbooks,
                kept_skills,
                dropped_skills = self.relevant_skills.len() - kept_skills,
                token_budget = RELEVANT_TECHNIQUES_TOKEN_BUDGET,
                used_tokens = total_tokens,
                "trimmed relevant techniques to fit the prompt budget"
            );
        } else {
            tracing::info!(
                kept_playbooks,
                kept_skills,
                token_budget = RELEVANT_TECHNIQUES_TOKEN_BUDGET,
                used_tokens = total_tokens,
                "included relevant techniques in the prompt"
            );
        }

        self.apply_budget_profile(
            PromptSection::new("relevant_techniques", rendered)
                .with_priority(SectionPriority::High)
                .with_cache_layer(CacheLayer::Plan)
                .with_placement(Placement::End)
                .with_bidder(AttentionBidder::PlaybookRules)
                .with_hard_cap(RELEVANT_TECHNIQUES_TOKEN_BUDGET),
        )
    }

    fn apply_budget_profile(&self, mut section: PromptSection) -> Option<PromptSection> {
        let Some(cap) = self.section_budget_cap(&section.name) else {
            return Some(section);
        };
        if cap == 0 {
            return None;
        }
        section.hard_cap = Some(match section.hard_cap {
            Some(existing) => existing.min(cap),
            None => cap,
        });
        Some(section)
    }

    fn section_budget_cap(&self, section_name: &str) -> Option<usize> {
        let budget = self.budget_profile?;
        match section_name {
            "conventions" | "tool_instructions" | "anti_patterns" => Some(budget.instructions),
            "domain_context" | "context_layer" | "pheromone_signals" => Some(budget.context),
            "relevant_techniques" => Some(budget.skills),
            _ => None,
        }
    }
}

impl Composer for SystemPromptBuilder {
    fn compose(
        &self,
        signals: &[Engram],
        budget: &Budget,
        scorer: &dyn Scorer,
        ctx: &Context,
    ) -> Result<Engram> {
        let mut built_sections = self
            .build_sections()
            .into_iter()
            .map(PromptSection::into_signal)
            .collect::<Result<Vec<_>>>()?;
        built_sections.extend(signals.iter().cloned());
        PromptComposer::new().compose(&built_sections, budget, scorer, ctx)
    }

    fn name(&self) -> &str {
        "system_prompt_builder"
    }
}

fn adjusted_priority(
    base_priority: u8,
    section: &str,
    role: &str,
    registry: &SectionEffectivenessRegistry,
) -> u8 {
    match registry.recommend_priority_change(section, role) {
        PriorityChange::Increase => base_priority.saturating_add(1),
        PriorityChange::Decrease => base_priority.saturating_sub(1),
        PriorityChange::NoChange | PriorityChange::InsufficientData => base_priority,
    }
}

const fn section_priority_from_u8(priority: u8) -> SectionPriority {
    match priority {
        0 => SectionPriority::Low,
        1 => SectionPriority::Normal,
        2 => SectionPriority::High,
        _ => SectionPriority::Critical,
    }
}

impl SystemPromptBuilder {
    fn tuned_sections(&self, token_budget: usize, counter: &TokenCounter) -> Vec<PromptSection> {
        let mut sections = self.build_sections();
        if self.section_effectiveness.is_some() {
            self.apply_learned_budget_tuning(&mut sections, token_budget, counter);
        }
        sections
    }

    fn apply_learned_budget_tuning(
        &self,
        sections: &mut [PromptSection],
        token_budget: usize,
        counter: &TokenCounter,
    ) {
        if token_budget == 0 {
            return;
        }

        let mut data = Vec::with_capacity(sections.len());
        let mut weighted_total = 0.0_f64;

        for section in sections.iter() {
            let rendered = render_section(&section.clone().enforce_hard_cap());
            let base_tokens = counter.count(&rendered);
            let weight = self.section_lift_weight(&section.name);
            let weighted = base_tokens as f64 * weight;
            weighted_total += weighted;
            data.push((base_tokens, weight, weighted));
        }

        if weighted_total <= 0.0 {
            return;
        }

        let mut caps = data
            .iter()
            .map(|(_, _, weighted)| {
                ((token_budget as f64 * *weighted) / weighted_total).floor() as usize
            })
            .collect::<Vec<_>>();

        let mut assigned = caps.iter().sum::<usize>();
        let mut remainders = data
            .iter()
            .enumerate()
            .map(|(index, (_, _, weighted))| {
                let exact = (token_budget as f64 * *weighted) / weighted_total;
                (exact.fract(), index)
            })
            .collect::<Vec<_>>();
        remainders.sort_by(|lhs, rhs| rhs.0.total_cmp(&lhs.0).then_with(|| lhs.1.cmp(&rhs.1)));

        for (_, index) in remainders {
            if assigned >= token_budget {
                break;
            }
            caps[index] = caps[index].saturating_add(1);
            assigned += 1;
        }

        for (index, section) in sections.iter_mut().enumerate() {
            let tuned_cap = caps[index];
            section.hard_cap = Some(match section.hard_cap {
                Some(existing) => existing.min(tuned_cap),
                None => tuned_cap,
            });

            tracing::info!(
                section = %section.name,
                base_tokens = data[index].0,
                weight = data[index].1,
                tuned_cap = section.hard_cap.unwrap_or(0),
                token_budget,
                "applied learned section budget tuning"
            );
        }
    }
}

fn render_skill(skill: &Skill) -> String {
    let title = if skill.name.is_empty() {
        "Unnamed skill"
    } else {
        skill.name.as_str()
    };
    let when_to_use = skill
        .precondition
        .trim()
        .strip_prefix("Apply for ")
        .map_or_else(
            || {
                if skill.precondition.trim().is_empty() {
                    skill.summary.trim()
                } else {
                    skill.precondition.trim()
                }
            },
            |rest| rest.trim_end_matches(" tasks.").trim(),
        );
    let how_to_apply = if skill.procedure.trim().is_empty() {
        skill.prompt_template.trim()
    } else {
        skill.procedure.trim()
    };

    format!(
        "### {title}\n\nWhen to use: {when_to_use}\nHow to apply: {how_to_apply}\nSuccess rate: {:.0}%\n",
        (skill.success_rate.clamp(0.0, 1.0) * 100.0)
    )
}

fn render_playbook(playbook: &Playbook) -> String {
    let title = if playbook.name.is_empty() {
        playbook.id.as_str()
    } else {
        playbook.name.as_str()
    };

    let mut rendered = format!(
        "### Playbook: {title} ({})\n\nGoal: {}\nSuccess rate: {:.0}%\n",
        playbook.id,
        playbook.goal,
        playbook.success_rate().unwrap_or(0.0).clamp(0.0, 1.0) * 100.0
    );

    if !playbook.steps.is_empty() {
        rendered.push_str("Steps:");
        for step in &playbook.steps {
            rendered.push_str(&format!("\n- [{}] {}", step.action_kind, step.description));
        }
        rendered.push('\n');
    }

    rendered
}

#[derive(Clone)]
struct RenderedSection {
    section: PromptSection,
    rendered: String,
}

fn sort_sections(sections: &mut [PromptSection]) {
    sections.sort_by(|a, b| {
        a.cache_layer
            .cmp(&b.cache_layer)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| section_order_rank(&a.name).cmp(&section_order_rank(&b.name)))
            .then_with(|| a.name.cmp(&b.name))
    });
}

fn assemble_sections(mut sections: Vec<PromptSection>, cache_markers: bool) -> String {
    sort_sections(&mut sections);

    let rendered = sections
        .into_iter()
        .map(|section| {
            let section = section.enforce_hard_cap();
            RenderedSection {
                rendered: render_section(&section),
                section,
            }
        })
        .collect::<Vec<_>>();
    let kept = rendered
        .iter()
        .map(|section| Some(section.rendered.clone()))
        .collect::<Vec<_>>();
    assemble_selected_sections(&rendered, &kept, cache_markers)
}

fn render_section(section: &PromptSection) -> String {
    match section.name.as_str() {
        "role_identity" => section.content.clone(),
        "conventions" => format!("## Project Conventions\n\n{}", section.content),
        "tool_instructions" => format!("## Tool Instructions\n\n{}", section.content),
        "domain_context" => format!("## Domain Context\n\n{}", section.content),
        "relevant_techniques" => section.content.clone(),
        "pheromone_signals" => format!("## Active Signals\n\n{}", section.content),
        "anti_patterns" => format!("## Anti-Patterns\n\n{}", section.content),
        "affect_guidance" => format!("## Affect Guidance\n\n{}", section.content),
        "task_context" => format!("## Current Task\n\n{}", section.content),
        _ => section.content.clone(),
    }
}

fn assemble_selected_sections(
    sections: &[RenderedSection],
    kept: &[Option<String>],
    cache_markers: bool,
) -> String {
    let selected_indices = kept
        .iter()
        .enumerate()
        .filter_map(|(index, rendered)| rendered.as_ref().map(|_| index))
        .collect::<Vec<_>>();

    let mut parts = Vec::with_capacity(selected_indices.len().saturating_mul(2));

    for (position, index) in selected_indices.iter().copied().enumerate() {
        if let Some(rendered) = &kept[index] {
            parts.push(rendered.clone());
        }

        if !cache_markers {
            continue;
        }

        let current = sections[index].section.cache_layer;
        let next = selected_indices
            .get(position + 1)
            .map(|next_index| sections[*next_index].section.cache_layer);
        if next != Some(current) {
            if let Some(marker) = cache_marker(current) {
                parts.push(marker.to_string());
            }
        }
    }

    normalize_for_caching(&parts.join("\n\n"))
}

fn candidate_fits(
    sections: &[RenderedSection],
    kept: &[Option<String>],
    index: usize,
    candidate: &str,
    cache_markers: bool,
    token_budget: usize,
    counter: &TokenCounter,
) -> bool {
    let mut next = kept.to_vec();
    next[index] = Some(candidate.to_string());
    counter.count(&assemble_selected_sections(sections, &next, cache_markers)) <= token_budget
}

fn truncate_to_fit(
    sections: &[RenderedSection],
    kept: &[Option<String>],
    index: usize,
    rendered: &str,
    cache_markers: bool,
    token_budget: usize,
    counter: &TokenCounter,
) -> Option<String> {
    let mut boundaries = rendered
        .char_indices()
        .map(|(boundary, _)| boundary)
        .collect::<Vec<_>>();
    boundaries.push(rendered.len());

    let mut low = 0usize;
    let mut high = boundaries.len();
    let mut best = None;

    while low < high {
        let mid = (low + high) / 2;
        let candidate = &rendered[..boundaries[mid]];

        if !candidate.is_empty()
            && candidate_fits(
                sections,
                kept,
                index,
                candidate,
                cache_markers,
                token_budget,
                counter,
            )
        {
            best = Some(candidate.to_string());
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    best
}

fn section_order_rank(name: &str) -> u8 {
    match name {
        "role_identity" => 0,
        "conventions" => 1,
        "tool_instructions" => 2,
        "domain_context" => 3,
        "context_layer" => 4,
        "pheromone_signals" => 5,
        "task_context" => 6,
        "relevant_techniques" => 7,
        "anti_patterns" => 8,
        "affect_guidance" => 9,
        _ => 10,
    }
}

impl SystemPromptBuilder {
    fn pheromone_section(&self) -> Option<PromptSection> {
        if self.pheromones.is_empty() {
            return None;
        }

        let mut pheromones = self.pheromones.to_vec();
        pheromones.sort_by(|left, right| {
            right
                .relevance
                .partial_cmp(&left.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| chunk_priority_rank(left).cmp(&chunk_priority_rank(right)))
                .then_with(|| left.content.cmp(&right.content))
        });

        let rendered = pheromones
            .iter()
            .map(render_pheromone_chunk)
            .collect::<Vec<_>>()
            .join("\n");

        Some(
            PromptSection::new("pheromone_signals", rendered)
                .with_priority(self.effective_priority("pheromone_signals", SectionPriority::High))
                .with_cache_layer(CacheLayer::Workspace)
                .with_placement(Placement::Middle)
                .with_hard_cap(1_500),
        )
    }
}

fn render_pheromone_chunk(chunk: &ContextChunk) -> String {
    let label = pheromone_label(chunk);
    let mut parts = vec![format!("- [{label}] {}", chunk.content.trim())];
    if let Some(recency) = chunk.recency {
        parts.push(format!("  recency={recency:.2}"));
    }
    if let Some(confidence) = chunk.confidence {
        parts.push(format!("  confidence={confidence:.2}"));
    }
    if let Some(track_record) = chunk.track_record {
        parts.push(format!("  track_record={track_record:.2}"));
    }
    parts.join("\n")
}

fn pheromone_label(chunk: &ContextChunk) -> &'static str {
    let lower = chunk.content.to_ascii_lowercase();
    if lower.contains("[threat]") || lower.contains("threat") || lower.contains("failure") {
        "Threat"
    } else if lower.contains("[warning]") || lower.contains("warning") || lower.contains("risk") {
        "Warning"
    } else if lower.contains("[opportunity]") || lower.contains("opportunity") {
        "Opportunity"
    } else {
        "Signal"
    }
}

fn chunk_priority_rank(chunk: &ContextChunk) -> u8 {
    let lower = chunk.content.to_ascii_lowercase();
    if lower.contains("threat") || lower.contains("warning") || lower.contains("failure") {
        0
    } else if lower.contains("opportunity") || lower.contains("success") {
        1
    } else {
        2
    }
}

const fn cache_marker(layer: CacheLayer) -> Option<&'static str> {
    match layer {
        CacheLayer::Role => Some("<!-- cache:system -->"),
        CacheLayer::Workspace => Some("<!-- cache:session -->"),
        CacheLayer::Plan => Some("<!-- cache:task -->"),
        CacheLayer::Volatile => Some("<!-- cache:dynamic -->"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolPermission};
    use roko_core::{Budget, Context, Kind, Score, Scorer};
    use roko_learn::playbook::{Playbook, PlaybookStep};
    use roko_learn::section_effect::SectionEffectivenessRegistry;
    use roko_learn::skill_library::Skill;

    struct ConstScorer;

    impl Scorer for ConstScorer {
        fn score(&self, _signal: &Engram, _ctx: &Context) -> Score {
            Score::NEUTRAL
        }
    }

    fn test_tool(name: &str) -> ToolDef {
        ToolDef::new(
            name,
            format!("{name} description"),
            ToolCategory::Read,
            ToolPermission::read_only(),
        )
    }

    #[test]
    fn build_with_all_layers() {
        let prompt = SystemPromptBuilder::new("You are an implementer.")
            .with_conventions("Use snake_case. Use thiserror.")
            .with_domain("DeFi protocol: Uniswap v4 hooks")
            .with_context("Knowledge about execution flow.")
            .with_task("Implement rate limiter in crates/golem-core")
            .with_tools("MCP tools: Read, Write, Bash")
            .with_anti_patterns(vec![
                "Never call unwrap in library crates".to_string(),
                "No hardcoded paths".to_string(),
            ])
            .build();

        assert!(prompt.contains("You are an implementer."));
        assert!(prompt.contains("snake_case"));
        assert!(prompt.contains("DeFi protocol"));
        assert!(prompt.contains("Knowledge about execution flow."));
        assert!(prompt.contains("rate limiter"));
        assert!(prompt.contains("MCP tools"));
        assert!(prompt.contains("Never call unwrap"));
        assert!(prompt.contains("No hardcoded paths"));
    }

    #[test]
    fn build_with_pheromones_includes_active_signals_layer() {
        let pheromones = vec![ContextChunk {
            content: "- [Threat] context assembly is too slow.".to_string(),
            source: roko_neuro::ContextSource::RecentSignal {
                signal_id: "pheromone-1".to_string(),
                plan_id: "plan-1".to_string(),
                kind: "pheromone".to_string(),
            },
            relevance: 0.92,
            track_record: Some(0.9),
            confidence: Some(0.8),
            recency: Some(0.95),
            emotional_tag: None,
        }];

        let prompt = SystemPromptBuilder::new("You are a conductor.")
            .with_pheromones(&pheromones)
            .with_task("Stabilize orchestration")
            .build();

        assert!(prompt.contains("## Active Signals"));
        assert!(prompt.contains("[Threat]"));
        assert!(prompt.contains("context assembly is too slow"));
    }

    #[test]
    fn build_minimal_only_role() {
        let prompt = SystemPromptBuilder::new("You are a reviewer.").build();
        assert!(prompt.contains("You are a reviewer."));
        assert!(!prompt.contains("Conventions"));
        assert!(!prompt.contains("Domain"));
        assert!(!prompt.contains("Anti-Patterns"));
    }

    #[test]
    fn composer_impl_merges_builder_sections_with_input_signals() {
        let builder =
            SystemPromptBuilder::new("You are an implementer.").with_conventions("Use snake_case.");
        let extra = PromptSection::new("extra_context", "Ship the migration.")
            .into_signal()
            .expect("section signal");
        let prompt = builder
            .compose(&[extra], &Budget::default(), &ConstScorer, &Context::now())
            .expect("composed prompt");

        assert_eq!(prompt.kind, Kind::Prompt);
        let rendered = prompt.body.as_text().expect("text prompt");
        assert!(rendered.contains("You are an implementer."));
        assert!(rendered.contains("Use snake_case."));
        assert!(rendered.contains("Ship the migration."));
    }

    #[test]
    fn layer_order_is_correct() {
        let prompt = SystemPromptBuilder::new("LAYER1_ROLE")
            .with_conventions("LAYER2_CONV")
            .with_domain("LAYER3_DOMAIN")
            .with_context("LAYER3B_CONTEXT")
            .with_pheromones(&[ContextChunk {
                content: "- [Opportunity] reuse known-good prompt patterns.".to_string(),
                source: roko_neuro::ContextSource::RecentSignal {
                    signal_id: "pheromone-2".to_string(),
                    plan_id: "plan-2".to_string(),
                    kind: "pheromone".to_string(),
                },
                relevance: 0.75,
                track_record: Some(0.8),
                confidence: Some(0.7),
                recency: Some(0.9),
                emotional_tag: None,
            }])
            .with_task("LAYER4_TASK")
            .with_tools("LAYER5_TOOLS")
            .add_anti_pattern("LAYER6_ANTI")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build();

        let pos_role = prompt
            .find("LAYER1_ROLE")
            .expect("invariant: prompt should contain the role layer marker");
        let pos_conv = prompt
            .find("LAYER2_CONV")
            .expect("invariant: prompt should contain the conventions marker");
        let pos_tools = prompt
            .find("LAYER5_TOOLS")
            .expect("invariant: prompt should contain the tools marker");
        let pos_domain = prompt
            .find("LAYER3_DOMAIN")
            .expect("invariant: prompt should contain the domain marker");
        let pos_context = prompt
            .find("LAYER3B_CONTEXT")
            .expect("invariant: prompt should contain the context marker");
        let pos_anti = prompt
            .find("LAYER6_ANTI")
            .expect("invariant: prompt should contain the anti-pattern marker");
        let pos_task = prompt
            .find("LAYER4_TASK")
            .expect("invariant: prompt should contain the task marker");
        let pos_affect = prompt
            .find("time pressure")
            .expect("invariant: prompt should contain affect guidance text");

        // Cache order: role -> workspace -> plan -> volatile.
        assert!(pos_role < pos_conv, "role before conventions");
        assert!(pos_conv < pos_tools, "conventions before tools");
        assert!(pos_tools < pos_domain, "tools before domain");
        assert!(pos_domain < pos_context, "domain before context");
        assert!(pos_context < pos_task, "context before task");
        assert!(pos_task < pos_anti, "task before anti-patterns");
        assert!(pos_task < pos_affect, "task before affect guidance");
        assert!(prompt.contains("Active Signals"));
    }

    #[test]
    fn cache_markers_inserted_between_tiers() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_context("Context")
            .with_task("Task")
            .with_cache_markers()
            .build();

        assert!(prompt.contains("<!-- cache:system -->"));
        assert!(prompt.contains("<!-- cache:session -->"));

        // System marker comes after conventions, before domain.
        let sys_marker = prompt
            .find("<!-- cache:system -->")
            .expect("invariant: prompt should contain the system cache marker");
        let domain_pos = prompt
            .find("Domain")
            .expect("invariant: prompt should contain the domain section");
        assert!(sys_marker < domain_pos);

        // Session marker comes after the session-tier context, before task.
        let sess_marker = prompt
            .find("<!-- cache:session -->")
            .expect("invariant: prompt should contain the session cache marker");
        let task_pos = prompt
            .find("Task")
            .expect("invariant: prompt should contain the task section");
        assert!(sess_marker < task_pos);
    }

    #[test]
    fn cache_markers_omitted_when_disabled() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_context("Context")
            .build();

        assert!(!prompt.contains("<!-- cache:"));
    }

    #[test]
    fn empty_layers_skipped() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("")
            .with_domain("")
            .with_context("")
            .with_task("")
            .with_tools("")
            .build();

        assert!(!prompt.contains("Conventions"));
        assert!(!prompt.contains("Domain"));
        assert!(!prompt.contains("Relevant Context"));
        assert!(!prompt.contains("Task"));
        assert!(!prompt.contains("Tool"));
    }

    #[test]
    fn build_sections_produces_correct_metadata() {
        let sections = SystemPromptBuilder::new("Role identity text")
            .with_conventions("Use snake_case")
            .with_domain("DeFi context")
            .with_context("Assembly context")
            .with_task("Implement feature X")
            .with_tools("Use MCP tools")
            .add_anti_pattern("No unwrap calls")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build_sections();

        assert_eq!(sections.len(), 8);

        // Layer 1: role_identity
        assert_eq!(sections[0].name, "role_identity");
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[0].cache_layer, CacheLayer::Role);
        assert_eq!(sections[0].placement, Placement::Start);

        // Layer 2: conventions
        assert_eq!(sections[1].name, "conventions");
        assert_eq!(sections[1].priority, SectionPriority::High);
        assert_eq!(sections[1].cache_layer, CacheLayer::Role);

        // Layer 5: tool_instructions (before domain in output order)
        assert_eq!(sections[2].name, "tool_instructions");
        assert_eq!(sections[2].cache_layer, CacheLayer::Role);

        // Layer 3: domain_context
        assert_eq!(sections[3].name, "domain_context");
        assert_eq!(sections[3].cache_layer, CacheLayer::Workspace);

        // Layer 3b: context_layer
        assert_eq!(sections[4].name, "context_layer");
        assert_eq!(sections[4].cache_layer, CacheLayer::Workspace);

        // Layer 4: task_context
        assert_eq!(sections[5].name, "task_context");
        assert_eq!(sections[5].priority, SectionPriority::Critical);
        assert_eq!(sections[5].cache_layer, CacheLayer::Plan);
        assert_eq!(sections[5].placement, Placement::End);

        // Layer 6: relevant_techniques is absent in this test.
        // Layer 7: anti_patterns
        assert_eq!(sections[6].name, "anti_patterns");
        assert_eq!(sections[6].cache_layer, CacheLayer::Plan);
        assert_eq!(sections[6].placement, Placement::End);

        // Layer 8: affect_guidance
        assert_eq!(sections[7].name, "affect_guidance");
        assert_eq!(sections[7].priority, SectionPriority::Normal);
        assert_eq!(sections[7].cache_layer, CacheLayer::Volatile);
        assert_eq!(sections[7].placement, Placement::End);
    }

    #[test]
    fn build_sections_skips_empty_layers() {
        let sections = SystemPromptBuilder::new("Role").build_sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "role_identity");
    }

    #[test]
    fn layer_count_reflects_populated_layers() {
        assert_eq!(SystemPromptBuilder::new("Role").layer_count(), 1);
        assert_eq!(
            SystemPromptBuilder::new("Role")
                .with_conventions("conv")
                .with_task("task")
                .layer_count(),
            3
        );
        assert_eq!(
            SystemPromptBuilder::new("Role")
                .with_conventions("conv")
                .with_domain("domain")
                .with_context("context")
                .with_task("task")
                .with_tools("tools")
                .add_anti_pattern("anti")
                .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
                .layer_count(),
            8
        );
    }

    #[test]
    fn add_anti_pattern_accumulates() {
        let prompt = SystemPromptBuilder::new("Role")
            .add_anti_pattern("No panics")
            .add_anti_pattern("No unwrap")
            .add_anti_pattern("No todo!()")
            .build();

        assert!(prompt.contains("No panics"));
        assert!(prompt.contains("No unwrap"));
        assert!(prompt.contains("No todo!()"));
    }

    #[test]
    fn build_and_build_sections_agree_on_content() {
        let builder = SystemPromptBuilder::new("You are a test agent.")
            .with_conventions("Use thiserror.")
            .with_domain("Blockchain domain.")
            .with_context("Recent context.")
            .with_task("Write unit tests.")
            .with_tools("MCP: Read, Bash.")
            .add_anti_pattern("No panics.");

        let text = builder.build();
        let sections = builder.build_sections();

        // Every section's content should appear in the built text after
        // normalizing consecutive newlines.  `build()` uses "\n\n" between
        // header and body while `build_sections()` sometimes uses "\n",
        // so we collapse runs of whitespace for the comparison.
        let normalize = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let norm_text = normalize(&text);
        for section in &sections {
            let norm_section = normalize(&section.content);
            assert!(
                norm_text.contains(&norm_section),
                "Section '{}' content not found in built text",
                section.name
            );
        }
    }

    #[test]
    fn anti_patterns_format_as_list() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_anti_patterns(vec!["A".to_string(), "B".to_string()])
            .build();

        assert!(prompt.contains("- A\n- B"));
    }

    #[test]
    fn affect_guidance_reflects_arousal() {
        let high = SystemPromptBuilder::new("Role")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build();
        assert!(high.contains("You are under time pressure, focus on the most critical path."));

        let low = SystemPromptBuilder::new("Role")
            .with_affect_state(Some(PadState::new(0.0, -0.8, 0.0)))
            .build();
        assert!(low.contains("You have time to explore thoroughly."));

        let neutral = SystemPromptBuilder::new("Role")
            .with_affect_state(Some(PadState::new(0.0, 0.0, 0.0)))
            .build();
        assert!(!neutral.contains("time pressure"));
        assert!(!neutral.contains("explore thoroughly"));
    }

    #[test]
    fn affect_guidance_mentions_negative_somatic_signal() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_affect_state(Some(
                PadState::new(0.0, 0.0, 0.0).with_somatic_hint(-0.8, 0.7),
            ))
            .build();

        assert!(prompt.contains("prior failure territory"));
        assert!(prompt.contains("known-safe sequences"));
    }

    #[test]
    fn system_prompt_builder_with_real_role_identity() {
        // Use a realistic role identity string.
        let role = "You are the Implementer. Your job is to write production-quality code \
                    that satisfies the plan specification exactly.";
        let prompt = SystemPromptBuilder::new(role)
            .with_conventions("- snake_case for functions\n- thiserror for errors")
            .with_task("Implement the rate limiter module")
            .build();

        assert!(prompt.starts_with("You are the Implementer"));
        assert!(prompt.contains("rate limiter"));
    }

    #[test]
    fn sections_have_correct_placement_ordering() {
        let sections = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_context("Context")
            .with_task("Task")
            .with_tools("Tools")
            .add_anti_pattern("Anti")
            .build_sections();

        // Start sections come before Middle, Middle before End.
        let start_sections: Vec<_> = sections
            .iter()
            .filter(|s| s.placement == Placement::Start)
            .collect();
        let middle_sections: Vec<_> = sections
            .iter()
            .filter(|s| s.placement == Placement::Middle)
            .collect();
        let end_sections: Vec<_> = sections
            .iter()
            .filter(|s| s.placement == Placement::End)
            .collect();

        assert!(!start_sections.is_empty());
        assert!(!middle_sections.is_empty());
        assert!(!end_sections.is_empty());

        // Role identity and conventions at Start.
        assert!(start_sections.iter().any(|s| s.name == "role_identity"));
        assert!(start_sections.iter().any(|s| s.name == "conventions"));
        // Tools at Middle.
        assert!(
            middle_sections
                .iter()
                .any(|s| s.name == "tool_instructions")
        );
        // Task and anti-patterns at End.
        assert!(end_sections.iter().any(|s| s.name == "task_context"));
        assert!(end_sections.iter().any(|s| s.name == "anti_patterns"));
    }

    #[test]
    fn no_cache_session_marker_when_no_session_layers() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_cache_markers()
            .build();

        // System marker should still appear.
        assert!(prompt.contains("<!-- cache:system -->"));
        // No session marker because domain and anti_patterns are empty.
        assert!(!prompt.contains("<!-- cache:session -->"));
    }

    #[test]
    fn section_cache_order_keeps_stable_sections_before_task_content() {
        let sections = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_task("Task")
            .with_tools("Tools")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build_sections();

        let ordered: Vec<_> = sections
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        assert_eq!(
            ordered,
            vec![
                "role_identity",
                "conventions",
                "tool_instructions",
                "domain_context",
                "task_context",
                "affect_guidance",
            ]
        );

        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_task("Task")
            .with_tools("Tools")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build();

        let role_pos = prompt
            .find("Role")
            .expect("invariant: prompt should contain the role text");
        let domain_pos = prompt
            .find("Domain")
            .expect("invariant: prompt should contain the domain text");
        let task_pos = prompt
            .find("Task")
            .expect("invariant: prompt should contain the task text");
        let pressure_pos = prompt
            .find("time pressure")
            .expect("invariant: prompt should contain affect guidance text");

        assert!(role_pos < task_pos);
        assert!(domain_pos < task_pos);
        assert!(task_pos < pressure_pos);
    }

    #[test]
    fn section_priority_adjustment_increases_positive_lift_sections() {
        let mut registry = SectionEffectivenessRegistry::new();
        for _ in 0..24 {
            registry.record_outcome("conventions", "Implementer", true, true);
        }
        for _ in 0..6 {
            registry.record_outcome("conventions", "Implementer", true, false);
        }
        for _ in 0..2 {
            registry.record_outcome("conventions", "Implementer", false, true);
        }
        for _ in 0..8 {
            registry.record_outcome("conventions", "Implementer", false, false);
        }

        let sections = SystemPromptBuilder::new("Role")
            .with_conventions("Use snake_case")
            .with_section_effectiveness("Implementer", &registry)
            .build_sections();

        let conventions = sections
            .iter()
            .find(|section| section.name == "conventions");
        assert_eq!(
            conventions.map(|section| section.priority),
            Some(SectionPriority::Critical)
        );
    }

    #[test]
    fn section_priority_adjustment_decreases_negative_lift_sections() {
        let mut registry = SectionEffectivenessRegistry::new();
        for _ in 0..5 {
            registry.record_outcome("anti_patterns", "Implementer", true, true);
        }
        for _ in 0..20 {
            registry.record_outcome("anti_patterns", "Implementer", true, false);
        }
        for _ in 0..8 {
            registry.record_outcome("anti_patterns", "Implementer", false, true);
        }
        for _ in 0..2 {
            registry.record_outcome("anti_patterns", "Implementer", false, false);
        }

        let sections = SystemPromptBuilder::new("Role")
            .add_anti_pattern("No unwrap calls")
            .with_section_effectiveness("Implementer", &registry)
            .build_sections();

        let anti_patterns = sections
            .iter()
            .find(|section| section.name == "anti_patterns");
        assert_eq!(
            anti_patterns.map(|section| section.priority),
            Some(SectionPriority::Low)
        );
    }

    #[test]
    fn section_priority_adjustment_ignores_insufficient_data() {
        let mut registry = SectionEffectivenessRegistry::new();
        for _ in 0..5 {
            registry.record_outcome("tool_instructions", "Implementer", true, true);
            registry.record_outcome("tool_instructions", "Implementer", false, false);
        }

        let sections = SystemPromptBuilder::new("Role")
            .with_tools("Read, Edit, Bash")
            .with_section_effectiveness("Implementer", &registry)
            .build_sections();

        let tools = sections
            .iter()
            .find(|section| section.name == "tool_instructions");
        assert_eq!(
            tools.map(|section| section.priority),
            Some(SectionPriority::Normal)
        );
    }

    #[test]
    fn relevant_skills_section_is_injected_and_budgeted() {
        let mut skill = Skill::new(
            "git_fixup",
            "Use git fixup workflow for targeted rewrites.",
            "Rebase or fix up the last commit when the patch is focused.",
        );
        skill.precondition = "Apply for focused refactor tasks.".to_string();
        skill.procedure =
            "Use `git commit --fixup` and autosquash to preserve history.".to_string();
        skill.success_rate = 0.92;

        let prompt = SystemPromptBuilder::new("Role")
            .with_task("Task")
            .with_skills(&[skill])
            .build();

        assert!(prompt.contains("## Relevant Techniques"));
        assert!(prompt.contains("When to use: focused refactor"));
        assert!(prompt.contains("How to apply: Use `git commit --fixup`"));
    }

    #[test]
    fn relevant_playbooks_are_injected_and_capped() {
        let mut api = Playbook::new("pb1", "Implement REST API");
        api.name = "implement-api".to_string();
        api.steps.push(PlaybookStep::new(
            0,
            "Wire the route handler",
            "edit_file",
            vec!["compile_ok".to_string()],
        ));

        let mut docs = Playbook::new("pb2", "Review docs");
        docs.name = "review-docs".to_string();

        let mut auth = Playbook::new("pb3", "Implement auth");
        auth.name = "implement-auth".to_string();

        let mut cache = Playbook::new("pb4", "Implement cache");
        cache.name = "implement-cache".to_string();

        let prompt = SystemPromptBuilder::new("Role")
            .with_task("Implement REST API")
            .with_playbooks(&[api, docs, auth, cache])
            .build();

        assert!(prompt.contains("## Relevant Techniques"));
        assert!(prompt.contains("Playbook: implement-api (pb1)"));
        assert!(prompt.contains("Playbook: review-docs (pb2)"));
        assert!(prompt.contains("Playbook: implement-auth (pb3)"));
        assert!(!prompt.contains("Playbook: implement-cache (pb4)"));
    }

    #[test]
    fn prompt_normalization_whitespace_variants_produce_identical_output() {
        let prompt_a = SystemPromptBuilder::new("Role\tIdentity  \r\n")
            .with_conventions("Use snake_case.\t\r\nKeep changes minimal.   ")
            .with_task("Implement cache normalization.\t")
            .build();
        let prompt_b = SystemPromptBuilder::new("Role    Identity\n")
            .with_conventions("Use snake_case.    \nKeep changes minimal.")
            .with_task("Implement cache normalization.")
            .build();

        assert_eq!(prompt_a, prompt_b);
        assert!(!prompt_a.contains('\r'));
        assert!(!prompt_a.contains('\t'));
    }

    #[test]
    fn prompt_normalization_canonical_tool_order_produces_identical_output() {
        let mut tools_a = vec![
            test_tool("write_file"),
            test_tool("bash"),
            test_tool("read_file"),
        ];
        let mut tools_b = vec![
            test_tool("read_file"),
            test_tool("write_file"),
            test_tool("bash"),
        ];
        canonical_tool_order(&mut tools_a);
        canonical_tool_order(&mut tools_b);

        let tools_a = tools_a
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let tools_b = tools_b
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let prompt_a = SystemPromptBuilder::new("Role")
            .with_tools(format!("Available tools:\n{tools_a}\t"))
            .build();
        let prompt_b = SystemPromptBuilder::new("Role")
            .with_tools(format!("Available tools:\r\n{tools_b}"))
            .build();

        assert_eq!(prompt_a, prompt_b);
        assert!(prompt_a.contains("bash, read_file, write_file"));
    }

    #[test]
    fn budget_enforcement_never_exceeds_token_budget() {
        let counter = TokenCounter::Heuristic {
            chars_per_token: 1.0,
        };
        let builder = SystemPromptBuilder::new("ROLE")
            .with_conventions("HIGH")
            .with_tools("normal tools that should be dropped")
            .with_task("critical task")
            .with_token_budget(70);

        let prompt = builder.build_with_counter(&counter);

        assert!(
            counter.count(&prompt) <= 70,
            "prompt exceeded budget: {} > 70",
            counter.count(&prompt)
        );
        assert!(prompt.contains("ROLE"));
        assert!(prompt.contains("HIGH"));
        assert!(prompt.contains("critical task"));
        assert!(!prompt.contains("normal tools"));
    }

    #[test]
    fn budget_enforcement_truncates_critical_sections_to_fit() {
        let counter = TokenCounter::Heuristic {
            chars_per_token: 1.0,
        };
        let builder = SystemPromptBuilder::new("ROLE")
            .with_task("task details that are too long for the remaining budget")
            .with_token_budget(24);

        let prompt = builder.build_with_counter(&counter);

        assert!(
            counter.count(&prompt) <= 24,
            "prompt exceeded budget: {} > 24",
            counter.count(&prompt)
        );
        assert!(prompt.contains("ROLE"));
        assert!(prompt.contains("## Current Task"));
        assert!(!prompt.contains("remaining budget"));
    }
}
