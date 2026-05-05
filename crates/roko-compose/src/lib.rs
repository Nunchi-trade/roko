//! [`Compose`](roko_core::Compose) implementations — assemble signals into
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
//! into `Signal<PromptSection>`s, and passes them to the composer. This is
//! the strict I/O boundary from `02-anti-patterns.md` (rule #8).

#![allow(clippy::module_name_repetitions)]
#![allow(clippy::pedantic, clippy::nursery)]

pub mod error;

pub mod agents_md;
pub mod attention;
pub mod auction;
pub mod budget;
pub mod budget_predictor;
pub mod cognitive_workspace;
pub mod compaction;
pub mod context_assembler;
pub mod context_mesh;
pub mod context_provider;
pub mod conventions;
pub mod cost_attribution;
pub mod enrichment;
pub mod foraging;
pub mod gate_feedback;
pub mod prompt;
pub mod prompt_assembly_service;
pub mod prompt_hints;
pub mod role_prompts;
pub mod scorer;
pub mod strategy;
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
    SectionCostStats, SubsystemId, VcgAllocation, VcgBid, detect_bid_correlation,
    is_pareto_optimal, vcg_allocate,
};
pub use budget::{AdjustedBudget, Complexity, adjusted_adaptive_budget_for, adjusted_budget_for};
pub use budget_predictor::{BudgetPredictor, SectionInfluence, TaskFeatures};
pub use cognitive_workspace::{
    CognitiveWorkspaceInput, build_cognitive_workspace, task_contract_from_prompt_context,
};
pub use compaction::{ChatMessage, CompactionPolicy, compact_history};
pub use context_assembler::{ContextAssembler, ContextChunk, PadState};
pub use context_mesh::{ContextMesh, SharedContextEntry};
pub use context_provider::{
    ContextBidder, ContextBidderRegistry, ContextBudgets, ContextCandidate, ContextInjectionBudget,
    ContextInjectionPolicy, ContextInjectionRecord, ContextProvider, ContextPurpose,
    ContextRejection, ContextRejectionReason, ContextRequest, ContextScope, ContextSection,
    ContextSource, ContextTier, DocsSourceMapBidder, LearningAttentionConfig,
    LearningContextBidder, LearningPosterior, PlanArtifacts, PriorTaskOutput, ReadFileSpec,
    RecentFailurePatternsBidder, ResolvedContext, RolePromptPolicyBidder, SiblingTask, TaskInput,
    TaskRequirementsBidder, VerifySpec, is_local_model, pheromone_context,
};
pub use conventions::{ProjectConventions, detect_conventions};
pub use cost_attribution::{CostAttribution, SectionCost};
pub use error::ComposeError;
pub use foraging::{
    MultiPatchForager, RetrievalSignal, SourceForagingProfile, estimate_context_sufficiency,
    should_stop_searching, social_foraging_boost,
};
pub use gate_feedback::{GateFeedback, MAX_GATE_FEEDBACK_LINES};
pub use prompt::{
    AttentionBidder, COMPOSITION_MANIFEST_TAG, CacheLayer, CompositionManifest, ContextStrategy,
    ExcludedSectionMeta, IncludedSectionMeta, Placement, PromptBuild, PromptComposer,
    PromptSection, SectionPriority, estimate_tokens,
};
pub use prompt_assembly_service::PromptAssemblyService;
pub use prompt_hints::prompt_hints_for;
pub use role_prompts::{
    BuiltinRolePolicy, DEFAULT_CONVENTIONS_SUFFIX, MANIFEST_BACKED_CORE_ROLES, RolePromptSource,
    RoleSystemPromptSpec, TaskContext, builtin_prompt_policy_for,
    builtin_role_policy_from_manifest, builtin_role_policy_manifest_for, builtin_role_profile_for,
    manifest_backed_core_roles, role_identity_for, role_prompt_source_for,
    tool_allowlist_instructions,
};
pub use scorer::{ActiveInferenceScorer, GoalDirectedHeuristicScorer, SectionScorer};
pub use strategy::{CompositionStrategy, DEFAULT_VCG_WARMUP_OBSERVATIONS};
pub use system_prompt_builder::SystemPromptBuilder;
pub use templates::{
    ConductorTemplate, PlanSlice, PromptAssembler, PromptBudget, QuickFixInput, QuickFixTemplate,
    QuickReviewerInput, QuickReviewerTemplate, RefactorerTemplate, ResearcherTemplate, Reviewer,
    ReviewerInput, ReviewerTemplate, RolePromptTemplate, ScribeInput, ScribeTemplate, budget_for,
    scribe::ScribeVariant,
};
pub use token_counter::TokenCounter;
