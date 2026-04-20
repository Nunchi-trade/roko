//! Enrichment pipeline — generate plan artifacts (briefs, tasks, verification
//! checklists, research memos, etc.) from plan documents.
//!
//! This module orchestrates a 13-step pipeline where each step either extracts
//! content from existing artifacts (non-LLM steps) or calls an LLM via the
//! [`LlmClient`] trait.
//!
//! # Design principles
//!
//! - **Config-injected env** (anti-pattern #3): all configuration comes via
//!   [`EnrichmentConfig`]; no environment variable reads.
//! - **I/O at boundary** (anti-pattern #8): prompt builders and validators are
//!   pure functions taking/returning strings. Only the pipeline itself does
//!   filesystem I/O.
//! - **No cross-step state**: each step reads its inputs fresh from disk.
//! - **Continue on failure**: [`EnrichmentPipeline::run_all`] logs per-step
//!   errors and continues.

pub mod batch_client;
pub mod client;
pub mod config;
pub mod direct_client;
pub mod estimate;
pub mod inputs;
pub mod outcome;
pub mod pipeline;
pub mod prompts;
pub mod select;
pub mod step;
pub mod validate;

pub use batch_client::{
    BatchClient, BatchId, BatchRequest, BatchResponse, BatchStatus, BatchTransport, BatchUsage,
};
pub use client::LlmClient;
pub use config::EnrichmentConfig;
pub use direct_client::{
    DirectClient, DirectRequest, DirectResponse, DirectTransport, DirectUsage, Message, StreamChunk,
};
pub use estimate::{EnrichmentEstimate, PlanInfo, estimate_enrichment};
pub use inputs::step_dependency_paths;
pub use outcome::{SkipReason, StepCost, StepOutcome};
pub use pipeline::{EnrichmentPipeline, StepOutcomeHistory};
pub use prompts::{StepInputs, build_prompt, build_repair_prompt, generate_without_llm};
pub use select::StepSelector;
pub use step::{ALL_ORDERED, EnrichStep, LlmBackend};
pub use validate::{normalize_step_output, repair_toml_output, validate_step_output};
