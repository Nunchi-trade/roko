//! Composable system prompt builder with 7 layers.
//!
//! Generates cache-aligned, role-specific system prompts from composable
//! fragments. Each layer targets a different stability tier:
//!
//! | Layer | Content | Cache Tier |
//! |-------|---------|------------|
//! | 1. Role identity | Who am I, what's my job | System (stable) |
//! | 2. Conventions | Project coding standards | System (semi-stable) |
//! | 3. Domain context | Project-specific knowledge | Session (semi-stable) |
//! | 4. Task context | Current task details | Task (volatile) |
//! | 5. Tool instructions | Available tools and usage | System (stable) |
//! | 6. Anti-patterns | What NOT to do | Session (semi-stable) |
//! | 7. Affect guidance | Emotional tone and focus | Dynamic |
//!
//! The builder emits sections in cache-layer order, with optional cache
//! alignment markers between stability tiers. Layers 1 + 2 + 5 form the
//! prefix-cacheable "system" tier; layers 3 + 6 form the "session" tier;
//! layer 4 is per-task; layer 7 is dynamic tone/focus guidance.
//!
//! # Design
//!
//! Inspired by the dynamic prompt generation pipeline in
//! `mori-agents/17-dynamic-prompt-generation.md` section 3. The key insight:
//! system prompts matter enormously (3-4x quality gap per the `--bare`
//! experiment), AND they should be task-specific, not one-size-fits-all.
//!
//! Anti-pattern #8: **no `std::fs`**. All content arrives via builder methods.

use crate::PadState;
use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};
use crate::token_counter::TokenCounter;
use roko_core::tool::ToolDef;
use roko_learn::section_effect::{PriorityChange, SectionEffectivenessRegistry};

/// A composable system prompt built from 7 layers.
///
/// Use the builder pattern:
/// ```ignore
/// let prompt = SystemPromptBuilder::new("You are an implementer...")
///     .with_conventions("Use snake_case, thiserror for errors")
///     .with_domain("DeFi protocol context: ...")
///     .with_task("Implement the rate limiter in crates/golem-core")
///     .with_tools("MCP tools available: Read, Write, Bash")
///     .with_anti_patterns(vec!["Never use unwrap() in library crates"])
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
    /// Layer 4: Task context — current task details.
    task: Option<String>,
    /// Layer 5: Tool instructions — available tools and how to use them.
    tools: Option<String>,
    /// Layer 6: Anti-patterns — things the agent must NOT do.
    anti_patterns: Vec<String>,
    /// Layer 7: Affect guidance — current emotional tone and focus.
    affect_state: Option<PadState>,
    /// Whether to insert cache alignment markers between tiers.
    cache_markers: bool,
    /// Optional token budget enforced by [`build_with_counter`](Self::build_with_counter).
    token_budget: Option<usize>,
    /// Learned section-effectiveness data scoped to one role.
    section_effectiveness: Option<SectionEffectivenessConfig>,
}

#[derive(Clone)]
struct SectionEffectivenessConfig {
    role: String,
    registry: SectionEffectivenessRegistry,
}

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
            task: None,
            tools: None,
            anti_patterns: Vec::new(),
            affect_state: None,
            cache_markers: false,
            token_budget: None,
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

    /// Set layer 6: anti-patterns (things the agent must NOT do).
    #[must_use]
    pub fn with_anti_patterns(mut self, patterns: Vec<String>) -> Self {
        self.anti_patterns = patterns.into_iter().map(normalize_owned).collect();
        self
    }

    /// Add a single anti-pattern to layer 6.
    #[must_use]
    pub fn add_anti_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.anti_patterns.push(normalize_owned(pattern));
        self
    }

    /// Set layer 7: affect guidance (current emotional tone and focus).
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
            .build_sections()
            .into_iter()
            .map(|section| RenderedSection {
                rendered: render_section(&section),
                section,
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
        let mut sections = Vec::with_capacity(8);

        // Layer 1: Role Identity
        sections.push(
            PromptSection::new("role_identity", &self.role_identity)
                .with_priority(self.effective_priority("role_identity", SectionPriority::Critical))
                .with_cache_layer(CacheLayer::Role)
                .with_placement(Placement::Start),
        );

        // Layer 2: Conventions
        if let Some(ref conv) = self.conventions {
            if !conv.is_empty() {
                sections.push(
                    PromptSection::new("conventions", conv)
                        .with_priority(
                            self.effective_priority("conventions", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Start),
                );
            }
        }

        // Layer 5: Tool Instructions (grouped in System cache tier)
        if let Some(ref tools) = self.tools {
            if !tools.is_empty() {
                sections.push(
                    PromptSection::new("tool_instructions", tools)
                        .with_priority(
                            self.effective_priority("tool_instructions", SectionPriority::Normal),
                        )
                        .with_cache_layer(CacheLayer::Role)
                        .with_placement(Placement::Middle),
                );
            }
        }

        // Layer 3: Domain Context
        if let Some(ref domain) = self.domain {
            if !domain.is_empty() {
                sections.push(
                    PromptSection::new("domain_context", domain)
                        .with_priority(
                            self.effective_priority("domain_context", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                );
            }
        }

        // Layer 3b: Relevant Context
        if let Some(ref context) = self.context {
            if !context.is_empty() {
                sections.push(
                    PromptSection::new("context_layer", format!("## Relevant Context\n{context}"))
                        .with_priority(
                            self.effective_priority("context_layer", SectionPriority::High),
                        )
                        .with_cache_layer(CacheLayer::Workspace)
                        .with_placement(Placement::Middle),
                );
            }
        }

        // Layer 6: Anti-Patterns
        if !self.anti_patterns.is_empty() {
            let anti_text: String = self
                .anti_patterns
                .iter()
                .map(|p| format!("- {p}"))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(
                PromptSection::new("anti_patterns", format!("Do NOT:\n{anti_text}"))
                    .with_priority(self.effective_priority("anti_patterns", SectionPriority::High))
                    .with_cache_layer(CacheLayer::Workspace)
                    .with_placement(Placement::End),
            );
        }

        // Layer 7: Affect Guidance
        if let Some(affect) = self.affect_guidance() {
            sections.push(
                PromptSection::new("affect_guidance", affect)
                    .with_priority(
                        self.effective_priority("affect_guidance", SectionPriority::Normal),
                    )
                    .with_cache_layer(CacheLayer::Volatile)
                    .with_placement(Placement::End),
            );
        }

        // Layer 4: Task Context
        if let Some(ref task) = self.task {
            if !task.is_empty() {
                sections.push(
                    PromptSection::new("task_context", task)
                        .with_priority(
                            self.effective_priority("task_context", SectionPriority::Critical),
                        )
                        .with_cache_layer(CacheLayer::Plan)
                        .with_placement(Placement::End),
                );
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
        if self.task.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if self.tools.as_ref().is_some_and(|s| !s.is_empty()) {
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
            guidance.push("Prefer proven approaches, verify early, and surface uncertainty explicitly.");
        } else if affect.pleasure >= 0.35 {
            guidance.push("Keep the solution lean and avoid over-engineering.");
        }

        if affect.dominance <= -0.20 {
            guidance.push("Reduce scope until the next concrete checkpoint is clear.");
        } else if affect.dominance >= 0.30 {
            guidance.push("Execute decisively, but keep claims grounded in evidence.");
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
    });
}

fn assemble_sections(mut sections: Vec<PromptSection>, cache_markers: bool) -> String {
    sort_sections(&mut sections);

    let rendered = sections
        .into_iter()
        .map(|section| RenderedSection {
            rendered: render_section(&section),
            section,
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

const fn cache_marker(layer: CacheLayer) -> Option<&'static str> {
    match layer {
        CacheLayer::Role => Some("<!-- cache:system -->"),
        CacheLayer::Workspace => Some("<!-- cache:session -->"),
        CacheLayer::Plan | CacheLayer::Volatile => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::tool::{ToolCategory, ToolPermission};
    use roko_learn::section_effect::SectionEffectivenessRegistry;

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
                "Never use unwrap() in library crates".to_string(),
                "No hardcoded paths".to_string(),
            ])
            .build();

        assert!(prompt.contains("You are an implementer."));
        assert!(prompt.contains("snake_case"));
        assert!(prompt.contains("DeFi protocol"));
        assert!(prompt.contains("Knowledge about execution flow."));
        assert!(prompt.contains("rate limiter"));
        assert!(prompt.contains("MCP tools"));
        assert!(prompt.contains("Never use unwrap()"));
        assert!(prompt.contains("No hardcoded paths"));
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
    fn layer_order_is_correct() {
        let prompt = SystemPromptBuilder::new("LAYER1_ROLE")
            .with_conventions("LAYER2_CONV")
            .with_domain("LAYER3_DOMAIN")
            .with_context("LAYER3B_CONTEXT")
            .with_task("LAYER4_TASK")
            .with_tools("LAYER5_TOOLS")
            .add_anti_pattern("LAYER6_ANTI")
            .with_affect_state(Some(PadState::new(0.0, 0.8, 0.0)))
            .build();

        let pos_role = prompt.find("LAYER1_ROLE").unwrap();
        let pos_conv = prompt.find("LAYER2_CONV").unwrap();
        let pos_tools = prompt.find("LAYER5_TOOLS").unwrap();
        let pos_domain = prompt.find("LAYER3_DOMAIN").unwrap();
        let pos_context = prompt.find("LAYER3B_CONTEXT").unwrap();
        let pos_anti = prompt.find("LAYER6_ANTI").unwrap();
        let pos_task = prompt.find("LAYER4_TASK").unwrap();
        let pos_affect = prompt.find("time pressure").unwrap();

        // Cache order: role -> workspace -> plan -> volatile.
        assert!(pos_role < pos_conv, "role before conventions");
        assert!(pos_conv < pos_tools, "conventions before tools");
        assert!(pos_tools < pos_domain, "tools before domain");
        assert!(pos_domain < pos_context, "domain before context");
        assert!(pos_context < pos_anti, "context before anti-patterns");
        assert!(pos_anti < pos_task, "workspace sections before task");
        assert!(pos_task < pos_affect, "task before affect guidance");
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
        let sys_marker = prompt.find("<!-- cache:system -->").unwrap();
        let domain_pos = prompt.find("Domain").unwrap();
        assert!(sys_marker < domain_pos);

        // Session marker comes after the session-tier context, before task.
        let sess_marker = prompt.find("<!-- cache:session -->").unwrap();
        let task_pos = prompt.find("Task").unwrap();
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
            .add_anti_pattern("No unwrap()")
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

        // Layer 6: anti_patterns
        assert_eq!(sections[5].name, "anti_patterns");
        assert_eq!(sections[5].cache_layer, CacheLayer::Workspace);
        assert_eq!(sections[5].placement, Placement::End);

        // Layer 4: task_context
        assert_eq!(sections[6].name, "task_context");
        assert_eq!(sections[6].priority, SectionPriority::Critical);
        assert_eq!(sections[6].cache_layer, CacheLayer::Plan);
        assert_eq!(sections[6].placement, Placement::End);

        // Layer 7: affect_guidance
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

        assert!(prompt.find("Role").unwrap() < prompt.find("Task").unwrap());
        assert!(prompt.find("Domain").unwrap() < prompt.find("Task").unwrap());
        assert!(prompt.find("Task").unwrap() < prompt.find("time pressure").unwrap());
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
            .add_anti_pattern("No unwrap()")
            .with_section_effectiveness("Implementer", &registry)
            .build_sections();

        let anti_patterns = sections
            .iter()
            .find(|section| section.name == "anti_patterns");
        assert_eq!(
            anti_patterns.map(|section| section.priority),
            Some(SectionPriority::Normal)
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
