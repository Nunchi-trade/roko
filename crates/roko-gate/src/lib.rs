//! Concrete [`Gate`](roko_core::Gate) implementations for Roko.
//!
//! A gate verifies a signal against ground truth by shelling out to a tool
//! (compiler, test runner, linter, static analyzer). Gates produce
//! [`Verdict`](roko_core::Verdict)s that flow back into the substrate as
//! signals, feeding the conductor and the router-feedback loop.
//!
//! # Available Gates
//!
//! - [`ShellGate`] — runs an arbitrary shell command; passes if exit code 0
//! - [`CompileGate`] — `cargo check` equivalent; extensible to other languages
//!
//! Future: `TestGate`, `LintGate`, `SymbolGate`, `LlmJudgeGate`.

#![allow(clippy::module_name_repetitions)]

pub mod clippy_gate;
pub mod compile;
pub mod diff_gate;
pub mod gate_pipeline;
pub mod generated_test_gate;
pub mod integration_gate;
pub mod llm_judge_gate;
pub mod payload;
pub mod property_test_gate;
pub mod rung_selector;
pub mod shell;
pub mod symbol_gate;
pub mod test_gate;
pub mod verify_chain_gate;

pub use clippy_gate::ClippyGate;
pub use compile::CompileGate;
pub use diff_gate::{analyze_diff, DiffAnalysis, DiffGate, DiffPayload};
pub use payload::{BuildSystem, GatePayload, TestSelector};
pub use shell::ShellGate;
pub use test_gate::{parse_test_counts, TestGate};
