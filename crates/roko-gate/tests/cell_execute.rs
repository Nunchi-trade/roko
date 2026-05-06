//! Integration tests for Cell::execute() on the 4 main gates:
//! CompileGate, TestGate, ClippyGate, DiffGate.
//!
//! Each test proves that execute() delegates to verify(), wraps the
//! resulting Verdict in a Kind::GateVerdict signal, and tags the output
//! with gate name and pass/fail status.

use std::path::Path;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use roko_core::traits::Substrate;
use roko_core::{Body, BusErased, Cell, CellContext, Kind, MemoryBus, Signal, Verdict};
use roko_gate::{ClippyGate, CompileGate, DiffGate, DiffPayload, GatePayload, TestGate};
use roko_std::MemorySubstrate;
use tempfile::TempDir;

// ─── Helpers ─────────────────────────────────────────────────────────────

fn cell_context() -> CellContext {
    let bus: Arc<dyn BusErased> = Arc::new(MemoryBus::new(16));
    let store: Arc<dyn Substrate> = Arc::new(MemorySubstrate::new());
    CellContext::new(bus, store, CancellationToken::new())
}

fn scaffold_cargo_project(root: &Path, lib_rs: &str, extra_files: &[(&str, &str)]) {
    let cargo_toml = r#"[package]
name = "cell_execute_fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#;
    std::fs::create_dir_all(root.join("src")).expect("create src/");
    std::fs::write(root.join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");
    std::fs::write(root.join("src/lib.rs"), lib_rs).expect("write src/lib.rs");
    for (rel, contents) in extra_files {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(path, contents).expect("write extra file");
    }
}

fn gate_payload_signal(root: &Path) -> Signal {
    let payload = GatePayload::in_dir(root).with_target_dir(root.join("target"));
    Signal::builder(Kind::Task)
        .body(Body::from_json(&payload).expect("serialize GatePayload"))
        .build()
}

/// Assert an output signal is a well-formed gate verdict.
fn assert_verdict_signal(output: &Signal, expected_gate: &str, expected_passed: bool) {
    assert_eq!(
        output.kind,
        Kind::GateVerdict,
        "output signal kind should be GateVerdict"
    );

    // Tags carry gate name and pass status.
    assert_eq!(output.tag("gate"), Some(expected_gate), "gate tag mismatch");
    let expected_passed_str = expected_passed.to_string();
    assert_eq!(
        output.tag("passed"),
        Some(expected_passed_str.as_str()),
        "passed tag mismatch"
    );

    // Body should deserialize back to a Verdict.
    let verdict: Verdict = output
        .body
        .as_json()
        .expect("output body should deserialize to Verdict");
    assert_eq!(verdict.gate, expected_gate);
    assert_eq!(verdict.passed, expected_passed);
}

// ─── CompileGate ─────────────────────────────────────────────────────────

#[tokio::test]
async fn compile_gate_execute_returns_verdict_signal() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(
        tmp.path(),
        "pub fn greet() -> &'static str { \"hello\" }\n",
        &[],
    );

    let gate = CompileGate::cargo().with_timeout_ms(120_000);
    let input = vec![gate_payload_signal(tmp.path())];
    let ctx = cell_context();

    let result = gate.execute(input, &ctx).await;
    assert!(result.is_ok(), "execute() failed: {:?}", result.err());

    let signals = result.unwrap();
    assert_eq!(signals.len(), 1, "expected exactly 1 output signal");
    assert_verdict_signal(&signals[0], "compile:cargo", true);
}

// ─── TestGate ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_gate_execute_returns_verdict_signal() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(
        tmp.path(),
        r#"pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_add() {
        assert_eq!(add(1, 2), 3);
    }
}
"#,
        &[],
    );

    let gate = TestGate::cargo().with_timeout_ms(120_000);
    let input = vec![gate_payload_signal(tmp.path())];
    let ctx = cell_context();

    let result = gate.execute(input, &ctx).await;
    assert!(result.is_ok(), "execute() failed: {:?}", result.err());

    let signals = result.unwrap();
    assert_eq!(signals.len(), 1, "expected exactly 1 output signal");
    assert_verdict_signal(&signals[0], "test:cargo", true);
}

// ─── ClippyGate ──────────────────────────────────────────────────────────

#[tokio::test]
async fn clippy_gate_execute_returns_verdict_signal() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(tmp.path(), "pub fn clean() -> i32 { 42 }\n", &[]);

    let gate = ClippyGate::cargo().with_timeout_ms(120_000);
    let input = vec![gate_payload_signal(tmp.path())];
    let ctx = cell_context();

    let result = gate.execute(input, &ctx).await;
    assert!(result.is_ok(), "execute() failed: {:?}", result.err());

    let signals = result.unwrap();
    assert_eq!(signals.len(), 1, "expected exactly 1 output signal");
    assert_verdict_signal(&signals[0], "clippy:cargo", true);
}

// ─── DiffGate ────────────────────────────────────────────────────────────

#[tokio::test]
async fn diff_gate_execute_returns_verdict_signal() {
    let gate = DiffGate::new();
    let diff_payload = DiffPayload::new("+++ b/src/lib.rs\n+pub fn x() -> i32 { 1 }\n");
    let input_signal = Signal::builder(Kind::Task)
        .body(Body::from_json(&diff_payload).expect("serialize DiffPayload"))
        .build();

    let ctx = cell_context();
    let result = gate.execute(vec![input_signal], &ctx).await;
    assert!(result.is_ok(), "execute() failed: {:?}", result.err());

    let signals = result.unwrap();
    assert_eq!(signals.len(), 1, "expected exactly 1 output signal");
    assert_verdict_signal(&signals[0], "diff", true);
}
