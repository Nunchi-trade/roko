//! Regression coverage for truthful gate verdicts.

use roko_core::{Body, Context, Engram, GateConfig, GateRunner, Kind, ShellGateCommand, Verify};
use roko_gate::{GateService, ShellGate};
use std::path::Path;
use tempfile::TempDir;

fn gate_config(
    workdir: &Path,
    enabled_gates: &[&str],
    shell_gates: Vec<ShellGateCommand>,
) -> GateConfig {
    GateConfig {
        workdir: workdir.to_path_buf(),
        enabled_gates: enabled_gates
            .iter()
            .map(|gate| (*gate).to_string())
            .collect(),
        shell_gates,
        max_rung: None,
    }
}

async fn run_gate_service(
    workdir: &Path,
    enabled_gates: &[&str],
    shell_gates: Vec<ShellGateCommand>,
) -> roko_core::GateReport {
    let svc = GateService::new();
    svc.run_gates(gate_config(workdir, enabled_gates, shell_gates))
        .await
        .expect("gate service should produce a report")
}

fn empty_signal() -> Engram {
    Engram::builder(Kind::Task).body(Body::empty()).build()
}

fn scaffold_cargo_project(root: &Path) {
    std::fs::create_dir_all(root.join("src")).expect("create cargo src dir");
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "roko_gate_truth_fixture"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#,
    )
    .expect("write Cargo.toml");
    std::fs::write(root.join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n")
        .expect("write src/lib.rs");
}

#[tokio::test]
async fn shell_gate_true_passes_and_is_not_skipped() {
    let tmp = TempDir::new().expect("tempdir");
    let report = run_gate_service(
        tmp.path(),
        &["shell"],
        vec![ShellGateCommand {
            program: "true".into(),
            args: vec![],
            timeout_ms: 1_000,
        }],
    )
    .await;

    assert_eq!(report.verdicts.len(), 1);
    let verdict = &report.verdicts[0];
    assert!(verdict.passed);
    assert!(!verdict.skipped);
    assert_eq!(verdict.skip_reason, None);
    assert!(report.all_passed());
}

#[tokio::test]
async fn shell_gate_false_fails_and_is_not_skipped() {
    let tmp = TempDir::new().expect("tempdir");
    let report = run_gate_service(
        tmp.path(),
        &["shell"],
        vec![ShellGateCommand {
            program: "false".into(),
            args: vec![],
            timeout_ms: 1_000,
        }],
    )
    .await;

    assert_eq!(report.verdicts.len(), 1);
    let verdict = &report.verdicts[0];
    assert!(!verdict.passed);
    assert!(!verdict.skipped);
    assert_eq!(verdict.skip_reason, None);
    assert!(verdict.output.contains("exit code: 1"));
    assert!(!report.all_passed());
}

#[tokio::test]
async fn shell_gate_failure_keeps_stderr_in_the_verdict() {
    let gate = ShellGate::new(
        "sh",
        vec!["-c".into(), "printf 'shell stderr\\n' >&2; exit 1".into()],
    )
    .with_timeout_ms(1_000);

    let verdict = gate.verify(&empty_signal(), &Context::now()).await;

    assert!(!verdict.passed);
    assert!(verdict.reason.contains("exit code"));
    let detail = verdict.detail.as_deref().unwrap_or("");
    assert!(detail.contains("shell stderr"));
}

#[tokio::test]
async fn cargo_check_passes_in_a_valid_project_dir() {
    let tmp = TempDir::new().expect("tempdir");
    scaffold_cargo_project(tmp.path());

    let report = run_gate_service(tmp.path(), &["compile"], vec![]).await;

    assert_eq!(report.verdicts.len(), 1);
    let verdict = &report.verdicts[0];
    assert!(verdict.passed);
    assert!(!verdict.skipped);
    assert_eq!(verdict.skip_reason, None);
    assert!(report.all_passed());
}

#[tokio::test]
async fn unknown_gate_returns_a_skipped_verdict() {
    let tmp = TempDir::new().expect("tempdir");
    let report = run_gate_service(tmp.path(), &["nonexistent"], vec![]).await;

    assert_eq!(report.verdicts.len(), 1);
    let verdict = &report.verdicts[0];
    assert!(!verdict.passed);
    assert!(verdict.skipped);
    assert_eq!(verdict.skip_reason.as_deref(), Some("not wired"));
    assert!(verdict.output.contains("Unknown gate: nonexistent"));
    assert!(!report.all_passed());
}

#[tokio::test]
async fn stub_judge_gate_returns_a_skipped_verdict_with_reason() {
    let tmp = TempDir::new().expect("tempdir");
    let report = run_gate_service(tmp.path(), &["judge"], vec![]).await;

    assert_eq!(report.verdicts.len(), 1);
    let verdict = &report.verdicts[0];
    assert!(!verdict.passed);
    assert!(verdict.skipped);
    assert_eq!(verdict.skip_reason.as_deref(), Some("not implemented"));
    assert!(verdict.output.contains("not yet implemented"));
    assert!(!report.all_passed());
}
