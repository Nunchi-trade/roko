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
//! ## Standalone gates
//!
//! These gates are invoked outside the rung pipeline for specific scenarios:
//!
//! - [`DiffGate`] -- diff analysis (post-task review)
//! - [`CodeExecutionGate`] -- sandboxed code execution
//! - [`BenchmarkGate`](benchmark_gate::BenchmarkGate) -- performance benchmarks
//! - [`FormatCheckGate`](format_check_gate::FormatCheckGate) -- code formatting
//! - [`SecurityScanGate`](security_scan_gate::SecurityScanGate) -- security scanning
//!
//! The crate root re-exports the stable verification API so callers can build
//! selectors, pipelines, thresholds, dispatchers, and feedback transforms
//! without reaching into submodules.

#![allow(clippy::module_name_repetitions)]

pub mod adaptive_threshold;

pub mod artifact_store;
pub mod clippy_gate;
pub mod code_exec;
pub mod compile;
pub mod diff_gate;
pub mod env_builder;
pub mod eval_generator;
pub mod fact_check;
pub mod feedback;
pub mod gate_pipeline;
pub mod generated;
pub mod generated_test_gate;
pub mod integration_gate;
pub mod llm_judge_gate;
pub mod payload;
pub mod process_reward;
pub mod property_test_gate;
pub mod ratchet;
pub mod rung_dispatch;
pub mod rung_selector;
pub mod shell;
pub mod symbol_gate;
pub mod test_gate;
pub mod verdict_publisher;
pub mod verify_chain_gate;

pub use adaptive_threshold::{AdaptiveThresholds, RungStats};
pub use artifact_store::ArtifactStore;
pub use clippy_gate::ClippyGate;
pub use code_exec::{
    CodeExecutionBackend, CodeExecutionGate, CodeExecutionOutcome, CodeExecutionPayload,
};
pub use compile::CompileGate;
pub use diff_gate::{DiffAnalysis, DiffGate, DiffPayload, analyze_diff};
pub use env_builder::{GateEnv, GateEnvBuilder, build_for_rung};
pub use eval_generator::{EvalGenerator, EvalStrategy, EvalTemplate, Evaluation};
pub use fact_check::{FactCheckGate, SearchHit, SearchOracle};
pub use feedback::{FeedbackItem, GateFeedback, Severity, feedback_for_agent};
pub use gate_pipeline::{ComposedGatePipeline, GateComposition, GatePipeline};
pub use generated::{GateError, GateGenerator, GeneratedCheck};
pub use payload::{BuildSystem, GatePayload, TestSelector};
pub use process_reward::{
    AggregateMethod, ProcessRewardModel, ReasoningStep, StepVerdict, TurnSnapshot,
};
pub use ratchet::GateRatchet;
pub use rung_dispatch::{RungExecutionConfig, RungExecutionInputs, run_canonical_rung, run_rung};
pub use rung_selector::{PlanComplexity, Rung, RungCaps, is_selected, select_rungs};
pub use shell::ShellGate;
pub use test_gate::{TestGate, parse_test_counts};
pub use verdict_publisher::VerdictPublisher;
