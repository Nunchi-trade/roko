//! [`Composer`](roko_core::Composer) implementations — assemble signals into
//! structured outputs under resource budgets.
//!
//! This crate provides:
//!
//! - [`PromptSection`] — typed payload for a labeled, priority-tagged prompt fragment
//! - [`PromptComposer`] — combines sections into a final prompt under a token budget
//! - [`SectionScorer`] and [`ActiveInferenceScorer`] — rank sections by
//!   priority/recency or goal-directed expected free energy, respectively
//!
//! # Design principle
//!
//! Composers do not read files. The app layer reads files, converts them
//! into `Engram<PromptSection>`s, and passes them to the composer. This is
//! the strict I/O boundary from `02-anti-patterns.md` (rule #8).

#![allow(clippy::module_name_repetitions)]

pub mod agents_md;
pub mod budget;
pub mod compaction;
pub mod context_assembler;
pub mod context_provider;
pub mod conventions;
pub mod enrichment;
pub mod prompt;
pub mod prompt_hints;
pub mod role_prompts;
pub mod scorer;
pub mod symbol_resolver;
pub mod system_prompt_builder;
pub mod task_brief;
pub mod templates;
pub mod token_counter;

pub use agents_md::AgentsMd;
pub use budget::{AdjustedBudget, Complexity, adjusted_budget_for};
pub use compaction::{ChatMessage, CompactionPolicy, compact_history};
pub use context_assembler::{ContextAssembler, ContextChunk, PadState};
pub use context_provider::{
    ContextBudgets, ContextProvider, ContextSection, ContextSource, ContextTier, PlanArtifacts,
    PriorTaskOutput, ReadFileSpec, ResolvedContext, SiblingTask, TaskInput, VerifySpec,
    is_local_model, pheromone_context,
};
pub use conventions::{ProjectConventions, detect_conventions};
pub use prompt::{
    AttentionBidder, CacheLayer, ContextStrategy, Placement, PromptBuild, PromptComposer,
    PromptSection, SectionPriority, estimate_tokens,
};
pub use prompt_hints::prompt_hints_for;
pub use role_prompts::{
    DEFAULT_CONVENTIONS_SUFFIX, RoleSystemPromptSpec, TaskContext, role_identity_for,
    tool_allowlist_instructions,
};
pub use scorer::{ActiveInferenceScorer, SectionScorer};
pub use system_prompt_builder::SystemPromptBuilder;
pub use templates::{
    PlanSlice, PromptAssembler, PromptBudget, QuickFixInput, QuickFixTemplate, QuickReviewerInput,
    QuickReviewerTemplate, Reviewer, ReviewerInput, ReviewerTemplate, RolePromptTemplate,
    ScribeInput, ScribeTemplate, budget_for, scribe::ScribeVariant,
};
pub use token_counter::TokenCounter;
