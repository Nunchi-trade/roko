//! Learning subsystems for Roko — episodic memory, playbooks, skills, caches,
//! and pattern discovery.
//!
//! These modules consume the signal stream produced by the orchestrator and
//! agents, persist durable records of what worked, and surface reusable
//! knowledge back to the composer/router feedback loop.
//!
//! # Modules
//!
//! - [`episode_logger`] — append-only JSONL record of agent turns
//! - [`playbook`] — reusable patterns extracted from episodes
//! - [`skill_library`] — structured skills agents can invoke
//! - [`context_pack_cache`] — cached composed prompts keyed by task fingerprint
//! - [`pattern_discovery`] — mining episodes for recurring shapes
//! - [`provider_health`] — per-provider circuit breaker for LLM routing
//! - [`latency`] — rolling latency EMAs and percentiles for routing feedback
//! - [`anomaly`] — runaway loop, cost spike, and quality degradation detection
//! - [`pareto`] — cost-quality Pareto frontier computation for models

#![deny(missing_docs)]
#![allow(clippy::module_name_repetitions)]

pub mod anomaly;
pub mod bandits;
pub mod baseline;
/// Budget tracking and enforcement guardrails for routing decisions.
pub mod budget;
pub mod cascade_router;
pub mod cfactor;
pub mod context_pack_cache;
pub mod cost_table;
pub mod costs_db;
pub mod costs_log;
pub mod drift;
pub mod efficiency;
pub mod episode_logger;
/// Cheap pre-processing of noisy gate failures into retry-ready diagnoses.
pub mod error_enrichment;
/// Unified learning events emitted by routing, evaluation, and runtime feedback.
pub mod events;
pub mod hdc_clustering;
/// Rolling latency EMAs and percentiles for routing feedback.
pub mod latency;
pub mod model_experiment;
pub mod model_router;
pub mod pareto;
pub mod pattern_discovery;
pub mod playbook;
pub mod playbook_rules;
pub mod prediction;
pub mod prompt_experiment;
pub mod provider_health;
pub mod quality_judge;
pub mod regression;
pub mod runtime_feedback;
pub mod section_effect;
pub mod skill_library;
pub mod task_metric;
