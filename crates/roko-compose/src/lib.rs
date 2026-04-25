//! [`Composer`](roko_core::Composer) implementations — assemble signals into
//! structured outputs under resource budgets.
//!
//! This crate provides:
//!
//! - [`PromptSection`] — typed payload for a labeled, priority-tagged prompt fragment
//! - [`PromptComposer`] — combines sections into a final prompt under a token budget
//! - [`SectionScorer`] and [`GoalDirectedHeuristicScorer`] — rank sections by
//!   priority/recency or goal-directed heuristic relevance, respectively
//!
//! # Design principle
//!
//! Composers do not read files. The app layer reads files, converts them
//! into `Engram<PromptSection>`s, and passes them to the composer. This is
//! the strict I/O boundary from `02-anti-patterns.md` (rule #8).

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::pedantic, clippy::nursery)]

pub mod agents_md;
pub mod attention;
pub mod auction;
pub mod budget;
pub mod budget_predictor;
pub mod compaction;
pub mod context_assembler;
pub mod context_mesh;
pub mod context_provider;
pub mod conventions;
pub mod enrichment;
pub mod foraging;
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
pub use attention::{
    ModelAttentionCurves, PositionAttentionModel, dynamic_placement, placement_adjusted_score,
};
pub use auction::{
    AffectModulation, AuctionDiagnostics, FairnessConfig, LearningBidder, SectionAllocation,
    SubsystemId, VcgAllocation, VcgBid, detect_bid_correlation, is_pareto_optimal, vcg_allocate,
};
pub use budget::{AdjustedBudget, Complexity, adjusted_budget_for};
pub use budget_predictor::{BudgetPredictor, SectionInfluence, TaskFeatures};
pub use compaction::{ChatMessage, CompactionPolicy, compact_history};
pub use context_assembler::{ContextAssembler, ContextChunk, PadState};
pub use context_mesh::{ContextMesh, SharedContextEntry};
pub use context_provider::{
    ContextBidder, ContextBudgets, ContextCandidate, ContextInjectionBudget,
    ContextInjectionPolicy, ContextInjectionRecord, ContextProvider, ContextPurpose,
    ContextRejection, ContextRejectionReason, ContextRequest, ContextScope, ContextSection,
    ContextSource, ContextTier, PlanArtifacts, PriorTaskOutput, ReadFileSpec, ResolvedContext,
    SiblingTask, TaskInput, VerifySpec, is_local_model, pheromone_context,
};
pub use conventions::{ProjectConventions, detect_conventions};
pub use foraging::{
    MultiPatchForager, RetrievalSignal, SourceForagingProfile, estimate_context_sufficiency,
    should_stop_searching, social_foraging_boost,
};
pub use prompt::{
    AttentionBidder, CacheLayer, ContextStrategy, Placement, PromptBuild, PromptComposer,
    PromptSection, SectionPriority, estimate_tokens,
};
pub use prompt_hints::prompt_hints_for;
pub use role_prompts::{
    BuiltinRolePolicy, DEFAULT_CONVENTIONS_SUFFIX, MANIFEST_BACKED_CORE_ROLES, RolePromptSource,
    RoleSystemPromptSpec, TaskContext, builtin_prompt_policy_for,
    builtin_role_policy_from_manifest, builtin_role_policy_manifest_for, builtin_role_profile_for,
    manifest_backed_core_roles, role_identity_for, role_prompt_source_for,
    tool_allowlist_instructions,
};
pub use scorer::{ActiveInferenceScorer, GoalDirectedHeuristicScorer, SectionScorer};
pub use system_prompt_builder::SystemPromptBuilder;
pub use templates::{
    ConductorTemplate, PlanSlice, PromptAssembler, PromptBudget, QuickFixInput, QuickFixTemplate,
    QuickReviewerInput, QuickReviewerTemplate, RefactorerTemplate, ResearcherTemplate, Reviewer,
    ReviewerInput, ReviewerTemplate, RolePromptTemplate, ScribeInput, ScribeTemplate, budget_for,
    scribe::ScribeVariant,
};
pub use token_counter::TokenCounter;
