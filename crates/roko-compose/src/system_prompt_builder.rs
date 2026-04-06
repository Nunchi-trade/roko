//! Composable system prompt builder with 6 layers.
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
//!
//! The builder emits sections in this exact order, with optional cache
//! alignment markers between stability tiers. Layers 1 + 2 + 5 form the
//! prefix-cacheable "system" tier; layers 3 + 6 form the "session" tier;
//! layer 4 is per-task.
//!
//! # Design
//!
//! Inspired by the dynamic prompt generation pipeline in
//! `mori-agents/17-dynamic-prompt-generation.md` section 3. The key insight:
//! system prompts matter enormously (3-4x quality gap per the `--bare`
//! experiment), AND they should be task-specific, not one-size-fits-all.
//!
//! Anti-pattern #8: **no `std::fs`**. All content arrives via builder methods.

use crate::prompt::{CacheLayer, Placement, PromptSection, SectionPriority};

/// A composable system prompt built from 6 layers.
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
    /// Layer 4: Task context — current task details.
    task: Option<String>,
    /// Layer 5: Tool instructions — available tools and how to use them.
    tools: Option<String>,
    /// Layer 6: Anti-patterns — things the agent must NOT do.
    anti_patterns: Vec<String>,
    /// Whether to insert cache alignment markers between tiers.
    cache_markers: bool,
}

impl SystemPromptBuilder {
    /// Start building a system prompt with the role identity (layer 1).
    ///
    /// The role identity is the stable, role-specific opening paragraph
    /// that tells the agent what it is and what its constraints are.
    #[must_use]
    pub fn new(role_identity: impl Into<String>) -> Self {
        Self {
            role_identity: role_identity.into(),
            conventions: None,
            domain: None,
            task: None,
            tools: None,
            anti_patterns: Vec::new(),
            cache_markers: false,
        }
    }

    /// Set layer 2: project conventions (coding standards, naming, etc.).
    #[must_use]
    pub fn with_conventions(mut self, conventions: impl Into<String>) -> Self {
        self.conventions = Some(conventions.into());
        self
    }

    /// Set layer 3: domain context (project-specific knowledge).
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Set layer 4: task context (current task details).
    #[must_use]
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Set layer 5: tool instructions (available tools and usage guidance).
    #[must_use]
    pub fn with_tools(mut self, tools: impl Into<String>) -> Self {
        self.tools = Some(tools.into());
        self
    }

    /// Set layer 6: anti-patterns (things the agent must NOT do).
    #[must_use]
    pub fn with_anti_patterns(mut self, patterns: Vec<String>) -> Self {
        self.anti_patterns = patterns;
        self
    }

    /// Add a single anti-pattern to layer 6.
    #[must_use]
    pub fn add_anti_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.anti_patterns.push(pattern.into());
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

    /// Build the final system prompt as a single string.
    ///
    /// Layers are emitted in order 1-6, with cache markers between
    /// stability tiers if enabled. Empty layers are skipped.
    #[must_use]
    pub fn build(&self) -> String {
        let mut parts: Vec<String> = Vec::with_capacity(10);

        // ── Layer 1: Role Identity (System tier) ──
        parts.push(self.role_identity.clone());

        // ── Layer 2: Conventions (System tier) ──
        if let Some(ref conv) = self.conventions {
            if !conv.is_empty() {
                parts.push(format!("## Project Conventions\n\n{conv}"));
            }
        }

        // ── Layer 5: Tool Instructions (System tier — grouped with 1+2 for cache) ──
        if let Some(ref tools) = self.tools {
            if !tools.is_empty() {
                parts.push(format!("## Tool Instructions\n\n{tools}"));
            }
        }

        // Cache break: end of System tier.
        if self.cache_markers {
            parts.push("<!-- cache:system -->".to_string());
        }

        // ── Layer 3: Domain Context (Session tier) ──
        if let Some(ref domain) = self.domain {
            if !domain.is_empty() {
                parts.push(format!("## Domain Context\n\n{domain}"));
            }
        }

        // ── Layer 6: Anti-Patterns (Session tier — grouped with 3) ──
        if !self.anti_patterns.is_empty() {
            let mut anti = String::from("## Anti-Patterns\n\nDo NOT:\n");
            for pattern in &self.anti_patterns {
                anti.push_str("- ");
                anti.push_str(pattern);
                anti.push('\n');
            }
            parts.push(anti);
        }

        // Cache break: end of Session tier.
        if self.cache_markers && (self.domain.is_some() || !self.anti_patterns.is_empty()) {
            parts.push("<!-- cache:session -->".to_string());
        }

        // ── Layer 4: Task Context (Task tier — most volatile) ──
        if let Some(ref task) = self.task {
            if !task.is_empty() {
                parts.push(format!("## Current Task\n\n{task}"));
            }
        }

        parts.join("\n\n")
    }

    /// Build the system prompt as a vector of [`PromptSection`]s.
    ///
    /// Each layer becomes a separate section with appropriate priority,
    /// cache layer, and placement metadata. This is useful when feeding
    /// directly into [`PromptComposer`](crate::PromptComposer) or
    /// [`PromptAssembler`](crate::templates::assembly::PromptAssembler).
    #[must_use]
    pub fn build_sections(&self) -> Vec<PromptSection> {
        let mut sections = Vec::with_capacity(6);

        // Layer 1: Role Identity
        sections.push(
            PromptSection::new("role_identity", &self.role_identity)
                .with_priority(SectionPriority::Critical)
                .with_cache_layer(CacheLayer::System)
                .with_placement(Placement::Start),
        );

        // Layer 2: Conventions
        if let Some(ref conv) = self.conventions {
            if !conv.is_empty() {
                sections.push(
                    PromptSection::new("conventions", conv)
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::System)
                        .with_placement(Placement::Start),
                );
            }
        }

        // Layer 5: Tool Instructions (grouped in System cache tier)
        if let Some(ref tools) = self.tools {
            if !tools.is_empty() {
                sections.push(
                    PromptSection::new("tool_instructions", tools)
                        .with_priority(SectionPriority::Normal)
                        .with_cache_layer(CacheLayer::System)
                        .with_placement(Placement::Middle),
                );
            }
        }

        // Layer 3: Domain Context
        if let Some(ref domain) = self.domain {
            if !domain.is_empty() {
                sections.push(
                    PromptSection::new("domain_context", domain)
                        .with_priority(SectionPriority::High)
                        .with_cache_layer(CacheLayer::Session)
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
                    .with_priority(SectionPriority::High)
                    .with_cache_layer(CacheLayer::Session)
                    .with_placement(Placement::End),
            );
        }

        // Layer 4: Task Context
        if let Some(ref task) = self.task {
            if !task.is_empty() {
                sections.push(
                    PromptSection::new("task_context", task)
                        .with_priority(SectionPriority::Critical)
                        .with_cache_layer(CacheLayer::Task)
                        .with_placement(Placement::End),
                );
            }
        }

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
        if self.task.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if self.tools.as_ref().is_some_and(|s| !s.is_empty()) {
            count += 1;
        }
        if !self.anti_patterns.is_empty() {
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_with_all_layers() {
        let prompt = SystemPromptBuilder::new("You are an implementer.")
            .with_conventions("Use snake_case. Use thiserror.")
            .with_domain("DeFi protocol: Uniswap v4 hooks")
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
            .with_task("LAYER4_TASK")
            .with_tools("LAYER5_TOOLS")
            .add_anti_pattern("LAYER6_ANTI")
            .build();

        let pos_role = prompt.find("LAYER1_ROLE").unwrap();
        let pos_conv = prompt.find("LAYER2_CONV").unwrap();
        let pos_tools = prompt.find("LAYER5_TOOLS").unwrap();
        let pos_domain = prompt.find("LAYER3_DOMAIN").unwrap();
        let pos_anti = prompt.find("LAYER6_ANTI").unwrap();
        let pos_task = prompt.find("LAYER4_TASK").unwrap();

        // Order: role(1) -> conv(2) -> tools(5) -> domain(3) -> anti(6) -> task(4)
        assert!(pos_role < pos_conv, "role before conventions");
        assert!(pos_conv < pos_tools, "conventions before tools");
        assert!(pos_tools < pos_domain, "tools before domain");
        assert!(pos_domain < pos_anti, "domain before anti-patterns");
        assert!(pos_anti < pos_task, "anti-patterns before task");
    }

    #[test]
    fn cache_markers_inserted_between_tiers() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .with_task("Task")
            .with_cache_markers()
            .build();

        assert!(prompt.contains("<!-- cache:system -->"));
        assert!(prompt.contains("<!-- cache:session -->"));

        // System marker comes after conventions, before domain.
        let sys_marker = prompt.find("<!-- cache:system -->").unwrap();
        let domain_pos = prompt.find("Domain").unwrap();
        assert!(sys_marker < domain_pos);

        // Session marker comes after domain, before task.
        let sess_marker = prompt.find("<!-- cache:session -->").unwrap();
        let task_pos = prompt.find("Task").unwrap();
        assert!(sess_marker < task_pos);
    }

    #[test]
    fn cache_markers_omitted_when_disabled() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("Conv")
            .with_domain("Domain")
            .build();

        assert!(!prompt.contains("<!-- cache:"));
    }

    #[test]
    fn empty_layers_skipped() {
        let prompt = SystemPromptBuilder::new("Role")
            .with_conventions("")
            .with_domain("")
            .with_task("")
            .with_tools("")
            .build();

        assert!(!prompt.contains("Conventions"));
        assert!(!prompt.contains("Domain"));
        assert!(!prompt.contains("Task"));
        assert!(!prompt.contains("Tool"));
    }

    #[test]
    fn build_sections_produces_correct_metadata() {
        let sections = SystemPromptBuilder::new("Role identity text")
            .with_conventions("Use snake_case")
            .with_domain("DeFi context")
            .with_task("Implement feature X")
            .with_tools("Use MCP tools")
            .add_anti_pattern("No unwrap()")
            .build_sections();

        assert_eq!(sections.len(), 6);

        // Layer 1: role_identity
        assert_eq!(sections[0].name, "role_identity");
        assert_eq!(sections[0].priority, SectionPriority::Critical);
        assert_eq!(sections[0].cache_layer, CacheLayer::System);
        assert_eq!(sections[0].placement, Placement::Start);

        // Layer 2: conventions
        assert_eq!(sections[1].name, "conventions");
        assert_eq!(sections[1].priority, SectionPriority::High);
        assert_eq!(sections[1].cache_layer, CacheLayer::System);

        // Layer 5: tool_instructions (before domain in output order)
        assert_eq!(sections[2].name, "tool_instructions");
        assert_eq!(sections[2].cache_layer, CacheLayer::System);

        // Layer 3: domain_context
        assert_eq!(sections[3].name, "domain_context");
        assert_eq!(sections[3].cache_layer, CacheLayer::Session);

        // Layer 6: anti_patterns
        assert_eq!(sections[4].name, "anti_patterns");
        assert_eq!(sections[4].cache_layer, CacheLayer::Session);
        assert_eq!(sections[4].placement, Placement::End);

        // Layer 4: task_context
        assert_eq!(sections[5].name, "task_context");
        assert_eq!(sections[5].priority, SectionPriority::Critical);
        assert_eq!(sections[5].cache_layer, CacheLayer::Task);
        assert_eq!(sections[5].placement, Placement::End);
    }

    #[test]
    fn build_sections_skips_empty_layers() {
        let sections = SystemPromptBuilder::new("Role").build_sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "role_identity");
    }

    #[test]
    fn layer_count_reflects_populated_layers() {
        assert_eq!(
            SystemPromptBuilder::new("Role").layer_count(),
            1
        );
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
                .with_task("task")
                .with_tools("tools")
                .add_anti_pattern("anti")
                .layer_count(),
            6
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
            .with_task("Write unit tests.")
            .with_tools("MCP: Read, Bash.")
            .add_anti_pattern("No panics.");

        let text = builder.build();
        let sections = builder.build_sections();

        // Every section's content should appear in the built text.
        for section in &sections {
            assert!(
                text.contains(&section.content),
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
        assert!(middle_sections.iter().any(|s| s.name == "tool_instructions"));
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
}
