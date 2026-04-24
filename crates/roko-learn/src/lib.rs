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
// The learning crate is numerics- and telemetry-heavy; several pedantic lints
// create high-churn noise here without improving correctness for the current
// implementation style.
#![allow(
    clippy::assigning_clones,
    clippy::bool_to_int_with_if,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::clone_on_copy,
    clippy::collapsible_if,
    clippy::collection_is_never_read,
    clippy::derivable_impls,
    clippy::doc_markdown,
    clippy::expect_used,
    clippy::explicit_iter_loop,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::implicit_hasher,
    clippy::items_after_statements,
    clippy::iter_cloned_collect,
    clippy::manual_let_else,
    clippy::many_single_char_names,
    clippy::map_unwrap_or,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::missing_panics_doc,
    clippy::needless_borrow,
    clippy::needless_collect,
    clippy::needless_continue,
    clippy::needless_pass_by_value,
    clippy::needless_range_loop,
    clippy::option_if_let_else,
    clippy::or_fun_call,
    clippy::question_mark,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::return_self_not_must_use,
    clippy::significant_drop_tightening,
    clippy::similar_names,
    clippy::struct_field_names,
    clippy::suboptimal_flops,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unnecessary_map_or,
    clippy::unused_self,
    clippy::unwrap_used,
    clippy::use_self
)]

/// Active inference helpers for tier routing support.
pub mod active_inference;
/// Adaptive Design of AI Systems autocatalytic optimization (LEARN-08).
pub mod adas;
/// HDC-based adversarial signal detection with attack prototype library (TA-10).
pub mod adversarial;
/// Efficiency trend aggregation helpers for JSONL telemetry.
pub mod aggregate;
pub mod anomaly;
/// Research-oriented bandit shells used to match the learning docs.
pub mod bandit_research;
pub mod bandits;
pub mod baseline;
/// Bayesian confidence updating using Beta-Binomial conjugate model (AS-07).
pub mod bayesian_confidence;
/// Budget tracking and enforcement guardrails for routing decisions.
pub mod budget;
/// Bus-backed calibration policy for predict-publish-correct loop (LEARN-09).
pub mod calibration_policy;
pub mod cascade_router;
/// Causal microstructure discovery: Granger causality, PC algorithm,
/// and formal causal DAG construction from time series data (TA-08).
pub mod causal;
pub mod cfactor;
/// Learned intervention policy for conductor retries and aborts.
pub mod conductor;
pub mod context_pack_cache;
pub mod cost_table;
pub mod costs_db;
pub mod costs_log;
/// Curriculum ordering helpers for task scheduling.
pub mod curriculum;
pub mod drift;
pub mod efficiency;
pub mod episode_logger;
/// Cheap pre-processing of noisy gate failures into retry-ready diagnoses.
pub mod error_enrichment;
/// Persistent storage for error patterns discovered during plan execution.
pub mod error_pattern_store;
/// Event subscriber that fans runtime events into learning subsystems.
pub mod event_subscriber;
/// Unified learning events emitted by routing, evaluation, and runtime feedback.
pub mod events;
/// Forensic replay API for debugging failed tasks (GATE-07).
pub mod forensic_replay;
pub mod hdc_clustering;
/// HDC fingerprint helpers for episode memory.
pub mod hdc_fingerprint;
/// Heuristic, worldview, and research-provenance shells for learning parity.
pub mod heuristics;
/// Kalman filter for online signal smoothing in oracle predictions (P2-10).
pub mod kalman;
/// Rolling latency EMAs and percentiles for routing feedback.
pub mod latency;
pub mod local_reward;
pub mod model_experiment;
pub mod model_router;
/// Domain-specific Oracle implementations (Chain, Coding, Research) and witness verification.
pub mod oracles;
pub mod pareto;
pub mod pattern_discovery;
pub mod playbook;
pub mod playbook_rules;
pub mod prediction;
pub mod prompt_experiment;
pub mod provider_health;
pub mod quality_judge;
pub mod regression;
/// Typed reinforcement signal categories for the learning pipeline (AS-11).
pub mod reinforce_kind;
/// Research-to-runtime pipeline: Paper -> Claim -> Trial -> Ledger (LEARN-11).
pub mod research_pipeline;
/// Evolutionary resonant pattern organisms with Lotka-Volterra dynamics,
/// Price equation tracking, and HDC genomes (TA-09).
pub mod resonant_patterns;
/// Lookahead and calibration shells around the shipped cascade router.
pub mod routing_extras;
/// Append-only routing-decision audit log for explainability and dashboards.
pub mod routing_log;
pub mod runtime_feedback;
pub mod section_effect;
/// Shapley-value attribution for fair credit distribution among agents (P1-08).
pub mod shapley;
/// Evolutionary signal population dynamics: replicator dynamics, Hebbian
/// learning, and Fisher variance monitoring (TA-07).
pub mod signal_metabolism;
pub mod skill_library;
pub mod task_metric;
/// Verdict-aware scoring and routing history for gate-verdict re-entry (GATE-05).
pub mod verdict_scorer;
