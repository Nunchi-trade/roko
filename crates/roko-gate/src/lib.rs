//! Concrete verification gates and orchestration primitives for Roko.
//!
//! This crate ships the verification stack described in `docs/04-verification`:
//! concrete gate implementations, the rung selector, the sequential pipeline,
//! adaptive thresholds, runtime dispatch, and the agent feedback filter.
//! Gates verify a signal against ground truth and produce
//! [`Verdict`](roko_core::Verdict)s that flow back into the substrate as
//! signals.
//!
//! # Gate architecture: two-tier system
//!
//! ## Rung-dispatched gates (7 rungs)
//!
//! The canonical 7-rung pipeline is dispatched via [`rung_dispatch`] based on
//! plan complexity. Each rung maps to one or more concrete gates:
//!
//! | Rung | Index | Gates |
//! |------|-------|-------|
//! | `Compile` | 0 | [`CompileGate`] |
//! | `Lint` | 1 | [`ClippyGate`] |
//! | `Test` | 2 | [`TestGate`] |
//! | `Symbol` | 3 | [`SymbolGate`](symbol_gate::SymbolGate) |
//! | `GeneratedTest` | 4 | [`GeneratedTestGate`](generated_test_gate::GeneratedTestGate) + [`VerifyChainGate`](verify_chain_gate::VerifyChainGate) |
//! | `PropertyTest` | 5 | [`PropertyTestGate`](property_test_gate::PropertyTestGate) + [`FactCheckGate`] |
//! | `Integration` | 6 | [`LlmJudgeGate`](llm_judge_gate::LlmJudgeGate) + [`IntegrationGate`](integration_gate::IntegrationGate) |
//!
//! ## Standalone gates (6 gates)
//!
//! These gates are invoked outside the rung pipeline for specific scenarios:
//!
//! - [`DiffGate`] -- diff analysis (post-task review)
//! - [`CodeExecutionGate`] -- sandboxed code execution
//! - [`ShellGate`] -- arbitrary shell command verification
//! - [`BenchmarkRegressionGate`](benchmark_gate::BenchmarkRegressionGate) -- performance benchmarks
//! - [`FormatCheckGate`](format_check_gate::FormatCheckGate) -- code formatting
//! - [`SecurityScanGate`](security_scan_gate::SecurityScanGate) -- security scanning
//!
//! ## Ad-hoc generated checks
//!
//! - [`GateGenerator`] / [`GeneratedCheck`] -- dynamically generated verification checks
//!
//! ## Composition wrappers
//!
//! - [`ParallelGate`] -- run multiple gates in parallel, collect all verdicts
//! - [`VotingGate`] -- majority-vote across inner gates
//! - [`FallbackGate`] -- try gates in order, use first non-error verdict
//!
//! The crate root re-exports the stable verification API so callers can build
//! selectors, pipelines, thresholds, dispatchers, and feedback transforms
//! without reaching into submodules.

#![allow(clippy::module_name_repetitions)]
// Verification-gate crate: numerics-heavy with many gate structs; pedantic lints
// suppressed to reduce churn without sacrificing correctness.
#![allow(
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::collection_is_never_read,
    clippy::derivable_impls,
    clippy::derive_partial_eq_without_eq,
    clippy::doc_markdown,
    clippy::if_not_else,
    clippy::items_after_statements,
    clippy::literal_string_with_formatting_args,
    clippy::manual_clamp,
    clippy::map_unwrap_or,
    clippy::missing_const_for_fn,
    clippy::missing_fields_in_debug,
    clippy::needless_range_loop,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::redundant_closure_for_method_calls,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::struct_field_names,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::unreadable_literal
)]

pub mod adaptive_threshold;

pub mod acceptance_contract;
pub mod artifact_store;
pub mod clippy_gate;
pub mod code_exec;
pub mod compile;
/// Structured compile error classification: parse cargo JSON, classify by category.
pub mod compile_errors;
/// Standalone gate combinators: ParallelGate, VotingGate, FallbackGate (GATE-04).
pub mod composition;
pub mod diff_gate;
pub mod env_builder;
pub mod error_patterns;
pub mod eval_generator;
pub mod fact_check;
pub mod feedback;
/// Forensic causal chain reconstruction from content-addressed artifacts (GATE-07).
pub mod forensic;
pub mod gate_pipeline;
pub mod generated;
pub mod generated_test_gate;
/// Multi-gate joint anomaly detection via Hotelling's T-squared (GATE-08).
pub mod hotelling;
pub mod integration_gate;
pub mod llm_judge_gate;
pub mod payload;
/// PELT (Pruned Exact Linear Time) offline change point detection (P1-13).
pub mod pelt;
pub mod process_reward;
pub mod property_test_gate;
pub mod ratchet;
pub mod review_verdict;
pub mod rung_dispatch;
pub mod rung_selector;
pub mod shell;
/// Statistical Process Control extensions: CUSUM, EWMA Control Chart, BOCPD (GATE-01).
pub mod spc;
pub mod symbol_gate;
pub mod test_gate;
pub mod verdict_publisher;
pub mod verify_chain_gate;

pub use acceptance_contract::{
    AcceptanceContract, AcceptanceDecision, AcceptanceEvidence, AcceptanceIssue, AcceptanceOutcome,
    GateEvidence, GateRequirement, GateRequirementKind, NoStubEvidence, NoStubRequirement,
    ParityLedgerEvidenceRow, ParityLedgerRequirement, ParityLedgerRequirementRow,
    ParityLedgerStatus, RecoveryEvidence, RecoveryRequirement, RequiredNextAction,
    ReviewVerdictEvidence, ReviewVerdictRequirement, StructuredAgentOutputRequirement,
    StructuredOutputEvidence,
};
pub use adaptive_threshold::{AdaptiveThresholds, RungStats};
pub use artifact_store::ArtifactStore;
pub use clippy_gate::ClippyGate;
pub use code_exec::{
    CodeExecutionBackend, CodeExecutionGate, CodeExecutionOutcome, CodeExecutionPayload,
};
pub use compile::CompileGate;
pub use compile_errors::{
    CompileError, CompileErrorSummary, ErrorCategory, FailureClass, GateFailureAction,
    GateFailureClassification, classify_error_code, classify_gate_failure, parse_cargo_json,
    parse_plain_stderr, render_failure_classification,
};
pub use composition::{FallbackGate, ParallelGate, VotingGate};
pub use diff_gate::{DiffAnalysis, DiffGate, DiffPayload, analyze_diff};
pub use env_builder::{GateEnv, GateEnvBuilder, build_for_rung};
pub use error_patterns::{
    FailurePatternRecord, error_key, extract_error_digest, records_from_classification,
    records_from_parsed_review_verdict,
};
pub use eval_generator::{EvalGenerator, EvalStrategy, EvalTemplate, Evaluation};
pub use fact_check::{FactCheckGate, SearchHit, SearchOracle};
pub use feedback::{FeedbackItem, GateFeedback, Severity, feedback_for_agent};
pub use forensic::{
    ArtifactMetadata, CausalChain, ForensicError, ForensicReplayBuilder, TurnRecord,
};
pub use gate_pipeline::{ComposedGatePipeline, GateComposition, GatePipeline};
pub use generated::{GateError, GateGenerator, GeneratedCheck};
pub use hotelling::{HotellingDetector, JointAnomalyResult};
pub use payload::{BuildSystem, GatePayload, TestSelector};
pub use process_reward::{
    AggregateMethod, ProcessRewardModel, ReasoningStep, StepVerdict, TurnSnapshot,
};
pub use ratchet::GateRatchet;
pub use review_verdict::{
    ParsedReviewVerdict, ReviewParseSource, ReviewVerdict, ReviewVerdictContext,
    parse_structured_review_verdict,
};
pub use rung_dispatch::{RungExecutionConfig, RungExecutionInputs, run_canonical_rung, run_rung};
pub use rung_selector::{PlanComplexity, Rung, RungCaps, is_selected, select_rungs};
pub use shell::ShellGate;
pub use spc::{
    BocpdDetector, ChangePoint, ControlStatus, CusumDetector, CusumShift, EwmaControlChart,
    SpcAlert, SpcDetector,
};
pub use test_gate::{TestGate, parse_test_counts};
pub use verdict_publisher::VerdictPublisher;
