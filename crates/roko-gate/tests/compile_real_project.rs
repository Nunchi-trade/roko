//! Integration test: `CompileGate` against a real cargo project.
//!
//! Creates a tiny cargo crate in a tempdir, runs `CompileGate` against it,
//! verifies the verdict. Then corrupts the source and runs again to verify
//! the failure path works too.

use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Gate, Kind, Substrate};
use roko_gate::{BuildSystem, CompileGate, GatePayload};
use roko_std::MemorySubstrate;
use std::path::Path;
use tempfile::TempDir;
use tokio::fs;

async fn scaffold_cargo_project(root: &Path, lib_rs: &str) {
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "roko_gate_fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .await
    .unwrap();
    fs::create_dir_all(root.join("src")).await.unwrap();
    fs::write(root.join("src/lib.rs"), lib_rs).await.unwrap();
}

fn payload_signal(working_dir: &Path) -> Engram {
    let payload = GatePayload::in_dir(working_dir).with_label("test-fixture");
    Engram::builder(Kind::Task)
        .body(Body::from_json(&payload).unwrap())
        .build()
}

#[tokio::test]
async fn compile_gate_passes_on_valid_project() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(tmp.path(), "pub fn greet() -> &'static str { \"hello\" }\n").await;

    let gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(120_000);
    let signal = payload_signal(tmp.path());

    let verdict = gate.verify(&signal, &Context::now()).await;

    assert!(
        verdict.passed,
        "valid project should pass; reason={} detail={:?}",
        verdict.reason, verdict.detail
    );
    assert_eq!(verdict.gate, "compile:cargo");
    assert!(verdict.duration_ms > 0);
}

#[tokio::test]
async fn compile_gate_fails_on_syntax_error() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(tmp.path(), "pub fn broken() -> { missing body }\n").await;

    let gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(120_000);
    let signal = payload_signal(tmp.path());

    let verdict = gate.verify(&signal, &Context::now()).await;

    assert!(!verdict.passed, "syntax error should fail the gate");
    assert!(
        verdict.reason.contains("error"),
        "reason should mention error: {}",
        verdict.reason
    );
    // Detail should contain stderr with the actual rustc error.
    let detail = verdict.detail.as_deref().unwrap_or("");
    assert!(
        detail.contains("error"),
        "detail should contain compiler error: {detail}"
    );
}

#[tokio::test]
async fn compile_gate_fails_on_missing_dir() {
    let gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(10_000);
    let payload = GatePayload::in_dir("/nonexistent/path/xyz");
    let signal = Engram::builder(Kind::Task)
        .body(Body::from_json(&payload).unwrap())
        .build();

    let verdict = gate.verify(&signal, &Context::now()).await;
    assert!(!verdict.passed);
}

#[tokio::test]
async fn compile_gate_rejects_malformed_payload() {
    let gate = CompileGate::new(BuildSystem::Cargo);
    // A signal with no body, or a non-GatePayload body, should be rejected.
    let signal = Engram::builder(Kind::Task)
        .body(Body::text("not a payload"))
        .build();

    let verdict = gate.verify(&signal, &Context::now()).await;
    assert!(!verdict.passed);
    assert!(verdict.reason.contains("not a GatePayload"));
}

/// End-to-end: gate → verdict → persist as Engram → re-query.
///
/// This demonstrates the architectural flow: a gate's verdict becomes a
/// signal that lives in a substrate, ready to be consumed by policies.
#[tokio::test]
async fn verdict_flows_back_into_substrate_as_signal() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(tmp.path(), "pub fn x() -> i32 { 1 }\n").await;

    let substrate = MemorySubstrate::named("verdicts");
    let gate = CompileGate::new(BuildSystem::Cargo).with_timeout_ms(120_000);

    // 1. Task signal.
    let task_signal = payload_signal(tmp.path());
    substrate.put(task_signal.clone()).await.unwrap();

    // 2. Gate verifies.
    let verdict = gate.verify(&task_signal, &Context::now()).await;
    assert!(verdict.passed);

    // 3. Wrap the verdict as a GateVerdict signal derived from the task.
    let verdict_signal = task_signal
        .derive(Kind::GateVerdict, Body::from_json(&verdict).unwrap())
        .tag("gate", &verdict.gate)
        .tag("passed", &verdict.passed.to_string())
        .build();
    substrate.put(verdict_signal.clone()).await.unwrap();

    // 4. Query for gate verdicts — the verdict is now substrate-queryable.
    let verdicts = substrate
        .query(
            &roko_core::Query::of_kind(Kind::GateVerdict),
            &Context::now(),
        )
        .await
        .unwrap();

    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].lineage, vec![task_signal.id]);
    assert_eq!(verdicts[0].tag("passed"), Some("true"));
}

/// A wrapper that upgrades a Gate into a Substrate-writing policy — showing
/// how gates and substrates naturally compose.
struct PersistingGate<G: Gate> {
    inner: G,
    substrate: std::sync::Arc<dyn Substrate>,
}

#[async_trait]
impl<G: Gate> Gate for PersistingGate<G> {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> roko_core::Verdict {
        let verdict = self.inner.verify(signal, ctx).await;
        let verdict_signal = signal
            .derive(
                Kind::GateVerdict,
                Body::from_json(&verdict).unwrap_or(Body::Empty),
            )
            .tag("gate", &verdict.gate)
            .tag("passed", &verdict.passed.to_string())
            .build();
        let _ = self.substrate.put(verdict_signal).await;
        verdict
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

#[tokio::test]
async fn gate_composes_with_substrate_as_adapter() {
    let tmp = TempDir::new().unwrap();
    scaffold_cargo_project(tmp.path(), "pub fn y() -> bool { true }\n").await;

    let substrate: std::sync::Arc<dyn Substrate> = std::sync::Arc::new(MemorySubstrate::new());
    let gate = PersistingGate {
        inner: CompileGate::new(BuildSystem::Cargo).with_timeout_ms(120_000),
        substrate: substrate.clone(),
    };

    let signal = payload_signal(tmp.path());
    let verdict = gate.verify(&signal, &Context::now()).await;
    assert!(verdict.passed);

    // The gate persisted its own verdict as a signal.
    let stored = substrate
        .query(
            &roko_core::Query::of_kind(Kind::GateVerdict),
            &Context::now(),
        )
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].tag("gate"), Some("compile:cargo"));
}
