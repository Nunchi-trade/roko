//! Concrete verification gates and orchestration primitives for Roko.
//!
//! This crate ships the verification stack described in `docs/04-verification`:
//! concrete gate implementations, the rung selector, the sequential pipeline,
//! adaptive thresholds, runtime dispatch, and the agent feedback filter.
//! Gates verify a signal against ground truth and produce
//! [`Verdict`](roko_core::Verdict)s that flow back into the substrate as
//! signals.
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
pub mod fact_check;
pub mod feedback;
pub mod gate_pipeline;
pub mod generated;
pub mod generated_test_gate;
pub mod integration_gate;
pub mod llm_judge_gate;
pub mod payload;
pub mod property_test_gate;
pub mod ratchet;
pub mod rung_dispatch;
pub mod rung_selector;
pub mod shell;
pub mod symbol_gate;
pub mod test_gate;
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
pub use fact_check::{FactCheckGate, SearchHit, SearchOracle};
pub use feedback::{FeedbackItem, GateFeedback, Severity, feedback_for_agent};
pub use gate_pipeline::GatePipeline;
pub use generated::{GateError, GateGenerator, GeneratedCheck};
pub use payload::{BuildSystem, GatePayload, TestSelector};
pub use ratchet::GateRatchet;
pub use rung_dispatch::{RungExecutionConfig, RungExecutionInputs, run_canonical_rung, run_rung};
pub use rung_selector::{PlanComplexity, Rung, RungCaps, is_selected, select_rungs};
pub use shell::ShellGate;
pub use test_gate::{TestGate, parse_test_counts};
