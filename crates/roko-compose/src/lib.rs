//! [`Composer`](roko_core::Composer) implementations — assemble signals into
//! structured outputs under resource budgets.
//!
//! This crate provides:
//!
//! - [`PromptSection`] — typed payload for a labeled, priority-tagged prompt fragment
//! - [`PromptComposer`] — combines sections into a final prompt under a token budget
//! - [`SectionScorer`] — ranks sections by priority × recency × relevance
//!
//! # Design principle
//!
//! Composers do not read files. The app layer reads files, converts them
//! into `Signal<PromptSection>`s, and passes them to the composer. This is
//! the strict I/O boundary from `02-anti-patterns.md` (rule #8).

#![allow(clippy::module_name_repetitions)]

pub mod agents_md;
pub mod budget;
pub mod conventions;
pub mod enrichment;
pub mod prompt;
pub mod prompt_hints;
pub mod role_prompts;
pub mod scorer;
pub mod system_prompt_builder;
pub mod templates;

pub use agents_md::AgentsMd;
pub use budget::{AdjustedBudget, Complexity, adjusted_budget_for};
pub use conventions::{ProjectConventions, detect_conventions};
pub use prompt::{
    CacheLayer, ContextStrategy, Placement, PromptBuild, PromptComposer, PromptSection,
    SectionPriority, estimate_tokens,
};
pub use prompt_hints::prompt_hints_for;
pub use role_prompts::{
    DEFAULT_CONVENTIONS_SUFFIX, RoleSystemPromptSpec, TaskContext, role_identity_for,
    tool_allowlist_instructions,
};
pub use scorer::SectionScorer;
pub use system_prompt_builder::SystemPromptBuilder;
pub use templates::{
    PromptAssembler, PromptBudget, QuickFixInput, QuickFixTemplate, QuickReviewerInput,
    QuickReviewerTemplate, budget_for,
};
