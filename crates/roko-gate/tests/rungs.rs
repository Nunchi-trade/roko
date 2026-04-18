//! Integration coverage for the advertised rung pipeline and the newly wired
//! gate types.

use async_trait::async_trait;
use roko_core::{Body, Context, Engram, Gate, Kind, Verdict};
use roko_gate::generated_test_gate::{ArtifactStore, GeneratedTestGate, InMemoryArtifactStore};
use roko_gate::integration_gate::IntegrationGate;
use roko_gate::llm_judge_gate::{JudgeOracle, JudgePayload, LlmJudgeGate};
use roko_gate::property_test_gate::PropertyTestGate;
use roko_gate::rung_dispatch::{RungExecutionConfig, RungExecutionInputs, run_rung};
use roko_gate::symbol_gate::{
    SymbolExpectation, SymbolGate, SymbolKind, SymbolManifest, Visibility,
};
use roko_gate::verify_chain_gate::VerifyChainGate;
use roko_gate::{
    ClippyGate, CompileGate, DiffGate, DiffPayload, FactCheckGate, GatePayload, SearchHit,
    SearchOracle, TestGate,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

fn tempdir() -> TempDir {
    tempfile::TempDir::new().unwrap_or_else(|err| panic!("failed to create tempdir: {err}"))
}

fn write_text(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("failed to create {}: {err}", parent.display()));
    }
    std::fs::write(&path, contents)
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", path.display()));
}

fn scaffold_cargo_project(root: &Path, lib_rs: &str, extra_files: &[(&str, &str)]) {
    write_text(
        root,
        "Cargo.toml",
        r#"[package]
name = "rungs_fixture"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"#,
    );
    write_text(root, "src/lib.rs", lib_rs);
    for (rel, contents) in extra_files {
        write_text(root, rel, contents);
    }
}

fn json_signal<T: Serialize>(body: &T) -> Engram {
    let body = Body::from_json(body)
        .unwrap_or_else(|err| panic!("failed to serialize signal body as JSON: {err}"));
    Engram::builder(Kind::Task).body(body).build()
}

fn cargo_payload(root: &Path) -> GatePayload {
    GatePayload::in_dir(root)
        .with_target_dir(root.join("target"))
        .with_label("rungs-fixture")
}

fn cargo_signal(root: &Path) -> Engram {
    json_signal(&cargo_payload(root))
}

fn text_signal(text: &str) -> Engram {
    Engram::builder(Kind::Task).body(Body::text(text)).build()
}

fn generated_signal(root: &Path, plan: &str) -> Engram {
    Engram::builder(Kind::Task)
        .tag("plan", plan)
        .body(
            Body::from_json(&cargo_payload(root))
                .unwrap_or_else(|err| panic!("failed to serialize generated-test payload: {err}")),
        )
        .build()
}

fn generated_verify_signal(root: &Path, plan: &str, script: &Path) -> Engram {
    Engram::builder(Kind::Task)
        .tag("plan", plan)
        .tag("verify_script", script.to_string_lossy())
        .body(
            Body::from_json(&cargo_payload(root))
                .unwrap_or_else(|err| panic!("failed to serialize gate payload: {err}")),
        )
        .build()
}

fn symbol_signal(manifest: &SymbolManifest) -> Engram {
    json_signal(manifest)
}

fn judge_signal(payload: &JudgePayload) -> Engram {
    json_signal(payload)
}

fn verify_chain_signal(root: &Path, script: &Path) -> Engram {
    Engram::builder(Kind::Task)
        .tag("verify_script", script.to_string_lossy())
        .body(
            Body::from_json(&cargo_payload(root))
                .unwrap_or_else(|err| panic!("failed to serialize verify-chain payload: {err}")),
        )
        .build()
}

fn diff_signal(payload: &DiffPayload) -> Engram {
    json_signal(payload)
}

async fn verify<G: Gate + Sync>(gate: &G, signal: &Engram) -> Verdict {
    let ctx = Context::now();
    gate.verify(signal, &ctx).await
}

struct StaticSearchOracle {
    hits: Vec<SearchHit>,
}

#[async_trait]
impl SearchOracle for StaticSearchOracle {
    async fn search(&self, _query: &str) -> Result<Vec<SearchHit>, String> {
        Ok(self.hits.clone())
    }
}

struct StaticJudgeOracle {
    score: f32,
}

#[async_trait]
impl JudgeOracle for StaticJudgeOracle {
    async fn judge(&self, _prompt: &str) -> Result<f32, String> {
        Ok(self.score)
    }
}

fn search_oracle(content: &str) -> Arc<dyn SearchOracle> {
    Arc::new(StaticSearchOracle {
        hits: vec![SearchHit {
            content: content.to_string(),
        }],
    })
}

fn empty_search_oracle() -> Arc<dyn SearchOracle> {
    Arc::new(StaticSearchOracle { hits: Vec::new() })
}

fn judge_oracle(score: f32) -> Arc<dyn JudgeOracle> {
    Arc::new(StaticJudgeOracle { score })
}

fn write_script(root: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("failed to create {}: {err}", parent.display()));
    }
    std::fs::write(&path, contents)
        .unwrap_or_else(|err| panic!("failed to write {}: {err}", path.display()));
    path
}

#[tokio::test]
async fn run_rung_invokes_all_seven_rungs_with_expected_gate_mapping() {
    let tmp = tempdir();
    scaffold_cargo_project(
        tmp.path(),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n",
        &[(
            "tests/rung_harness.rs",
            r#"#[test]
fn smoke() {
    assert_eq!(2 + 2, 4);
}

#[test]
fn prop_fixture_passes() {
    assert_eq!(3 + 4, 7);
}

#[test]
fn test_integration_ok() {
    assert_eq!(5 + 5, 10);
}
"#,
        )],
    );
    let script = write_script(
        tmp.path(),
        "verify.sh",
        "#!/usr/bin/env bash\nprintf '[PASS] smoke (1 tests)\\n'\nexit 0\n",
    );
    let base_signal = generated_verify_signal(tmp.path(), "plan-rungs", &script);
    let symbol_manifest = SymbolManifest::new("plan-rungs").with_expectation(SymbolExpectation {
        name: "add".into(),
        kind: SymbolKind::Function,
        visibility: Visibility::Pub,
        module_path: String::new(),
        signature: None,
    });
    let inputs = RungExecutionInputs {
        symbol_signal: Some(symbol_signal(&symbol_manifest)),
        fact_check_signal: Some(text_signal(
            "The library shipped stable APIs in April 2024.",
        )),
        llm_judge_signal: Some(judge_signal(&JudgePayload {
            task_description: "Implement add".into(),
            diff: "pub fn add(a: i32, b: i32) -> i32 { a + b }".into(),
        })),
    };
    let generated_store: Arc<dyn ArtifactStore> = Arc::new(InMemoryArtifactStore::new());
    let config = RungExecutionConfig {
        source_roots: Some(vec![tmp.path().join("src")]),
        generated_test_artifacts: Some(generated_store),
        verify_chain_fallback: None,
        fact_check_oracle: Some(search_oracle(
            "The library shipped stable APIs in April 2024.",
        )),
        fact_check_min_confidence: Some(0.5),
        llm_judge_oracle: Some(judge_oracle(0.9)),
        llm_judge_min_score: Some(0.5),
        integration_test_pattern: Some("test_integration_ok".into()),
        integration_build_system: None,
    };

    let ctx = Context::now();
    let expected = [
        vec!["compile:cargo"],
        vec!["clippy:cargo"],
        vec!["test:cargo"],
        vec!["symbol"],
        vec!["generated_test:cargo", "verify_chain"],
        vec!["property_test:cargo", "fact_check"],
        vec!["llm_judge", "integration:build_test:test_integration_ok"],
    ];

    for (rung, expected_gates) in expected.iter().enumerate() {
        let verdicts = run_rung(&base_signal, &ctx, rung as u32, &inputs, &config).await;
        let actual: Vec<&str> = verdicts
            .iter()
            .map(|verdict| verdict.gate.as_str())
            .collect();
        assert_eq!(
            actual, *expected_gates,
            "unexpected gate mapping for rung {rung}"
        );
    }
}

#[tokio::test]
async fn compile_rung_passes_and_fails_on_invalid_rust() {
    let tmp = tempdir();
    scaffold_cargo_project(tmp.path(), "", &[]);

    let gate = CompileGate::cargo().with_timeout_ms(60_000);
    let signal = cargo_signal(tmp.path());

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "compile:cargo");

    write_text(
        tmp.path(),
        "src/lib.rs",
        "pub fn broken() -> { missing body }\n",
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("error") || fail.reason.contains("failed"));
}

#[tokio::test]
async fn test_rung_passes_and_fails_on_a_real_test_failure() {
    let tmp = tempdir();
    scaffold_cargo_project(
        tmp.path(),
        "",
        &[(
            "tests/test_harness.rs",
            r#"#[test]
fn smoke() {
    assert_eq!(1, 1);
}
"#,
        )],
    );

    let gate = TestGate::cargo().with_timeout_ms(60_000);
    let signal = cargo_signal(tmp.path());

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "test:cargo");

    write_text(
        tmp.path(),
        "tests/test_harness.rs",
        r#"#[test]
fn smoke() {
    assert_eq!(1, 2);
}
"#,
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("FAILED") || fail.reason.contains("failed"));
}

#[tokio::test]
async fn clippy_rung_passes_and_rejects_lint_noise() {
    let tmp = tempdir();
    scaffold_cargo_project(tmp.path(), "", &[]);

    let gate = ClippyGate::cargo().with_timeout_ms(60_000);
    let signal = cargo_signal(tmp.path());

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "clippy:cargo");

    write_text(
        tmp.path(),
        "src/lib.rs",
        r#"fn lint_me() {
    let unused_value = 1;
    let _ = 2;
}
"#,
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("unused") || fail.reason.contains("warning"));
}

#[tokio::test]
async fn diff_gate_accepts_real_diffs_and_rejects_stub_tokens() {
    let gate = DiffGate::new();

    let pass = verify(
        &gate,
        &diff_signal(&DiffPayload::new(
            "+++ b/src/lib.rs\n+pub fn answer() -> i32 {\n+    42\n+}\n",
        )),
    )
    .await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "diff");

    let fail = verify(
        &gate,
        &diff_signal(&DiffPayload::new("+++ b/src/lib.rs\n+Ok(())\n")),
    )
    .await;
    assert!(!fail.passed);
    assert!(
        fail.reason.contains("stub")
            || fail.reason.contains("vacuous")
            || fail.reason.contains("insufficient")
    );
}

#[tokio::test]
async fn symbol_rung_passes_and_reports_missing_symbols() {
    let tmp = tempdir();
    write_text(tmp.path(), "src/lib.rs", "pub struct Present;\n");

    let gate = SymbolGate::new(vec![tmp.path().to_path_buf()]);
    let manifest = SymbolManifest::new("plan-symbol").with_expectation(SymbolExpectation {
        name: "Present".into(),
        kind: SymbolKind::Struct,
        visibility: Visibility::Pub,
        module_path: String::new(),
        signature: None,
    });
    let signal = symbol_signal(&manifest);

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "symbol");

    let fail_manifest = SymbolManifest::new("plan-symbol").with_expectation(SymbolExpectation {
        name: "Missing".into(),
        kind: SymbolKind::Struct,
        visibility: Visibility::Pub,
        module_path: String::new(),
        signature: None,
    });
    let fail = verify(&gate, &symbol_signal(&fail_manifest)).await;
    assert!(!fail.passed);
    assert!(
        fail.error_digest
            .as_deref()
            .is_some_and(|digest| digest.contains("MISSING"))
    );
}

#[tokio::test]
async fn generated_test_rung_passes_and_fails_with_explicit_artifacts() {
    let tmp = tempdir();
    scaffold_cargo_project(
        tmp.path(),
        "",
        &[(
            "tests/generated_harness.rs",
            r#"#[path = "__roko_generated__/gen_fixture.rs"]
mod gen_fixture;
"#,
        )],
    );

    let pass_store: Arc<dyn ArtifactStore> = Arc::new(
        InMemoryArtifactStore::new().with(
            "plan-generated",
            "generated-tests/gen_fixture.rs",
            br#"#[test]
fn gen_fixture_passes() {
    assert_eq!(1, 1);
}
"#
            .to_vec(),
        ),
    );
    let gate = GeneratedTestGate::new(pass_store).with_timeout_ms(60_000);
    let signal = generated_signal(tmp.path(), "plan-generated");

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "generated_test:cargo");

    let fail_store: Arc<dyn ArtifactStore> = Arc::new(
        InMemoryArtifactStore::new().with(
            "plan-generated",
            "generated-tests/gen_fixture.rs",
            br#"#[test]
fn gen_fixture_fails() {
    assert_eq!(1, 2);
}
"#
            .to_vec(),
        ),
    );
    let failing_gate = GeneratedTestGate::new(fail_store).with_timeout_ms(60_000);
    let fail = verify(&failing_gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("generated") || fail.reason.contains("FAILED"));
}

#[tokio::test]
async fn property_test_rung_passes_and_fails_on_prefix_matched_tests() {
    let tmp = tempdir();
    scaffold_cargo_project(
        tmp.path(),
        "",
        &[(
            "tests/prop_harness.rs",
            r#"#[test]
fn prop_fixture_passes() {
    assert_eq!(2 + 2, 4);
}
"#,
        )],
    );

    let gate = PropertyTestGate::cargo().with_timeout_ms(60_000);
    let signal = cargo_signal(tmp.path());

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "property_test:cargo");

    write_text(
        tmp.path(),
        "tests/prop_harness.rs",
        r#"#[test]
fn prop_fixture_fails() {
    assert_eq!(1, 2);
}
"#,
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("FAILED") || fail.reason.contains("property"));
}

#[tokio::test]
async fn verify_chain_rung_passes_and_fails_on_script_exit_status() {
    let tmp = tempdir();
    let script = write_script(
        tmp.path(),
        "verify.sh",
        "#!/usr/bin/env bash\nprintf '[PASS] smoke (1 tests)\\n'\nexit 0\n",
    );
    let gate = VerifyChainGate::strict()
        .with_retry(false)
        .with_timeout_ms(60_000);
    let signal = verify_chain_signal(tmp.path(), &script);

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "verify_chain");

    write_script(
        tmp.path(),
        "verify.sh",
        "#!/usr/bin/env bash\nprintf '[FAIL] smoke (missing symbol)\\n' >&2\nexit 1\n",
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(
        fail.reason.contains("verify-chain")
            || fail.reason.contains("exit")
            || fail.reason.contains("FAILED")
    );
}

#[tokio::test]
async fn fact_check_rung_passes_and_fails_with_mock_oracles() {
    let signal = text_signal("The library shipped stable APIs in April 2024.");

    let pass_gate = FactCheckGate::new(
        search_oracle("The library shipped stable APIs in April 2024."),
        0.5,
    );
    let pass = verify(&pass_gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "fact_check");

    let fail_gate = FactCheckGate::new(empty_search_oracle(), 0.5);
    let fail = verify(&fail_gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("claims verified"));
}

#[tokio::test]
async fn llm_judge_rung_passes_and_fails_against_score_threshold() {
    let payload = JudgePayload {
        task_description: "Implement safe division".into(),
        diff: "fn div(a: i32, b: i32) -> Option<i32> { ... }".into(),
    };
    let signal = judge_signal(&payload);

    let pass_gate = LlmJudgeGate::new(judge_oracle(0.9), 0.5);
    let pass = verify(&pass_gate, &signal).await;
    assert!(pass.passed);
    assert_eq!(pass.gate, "llm_judge");

    let fail_gate = LlmJudgeGate::new(judge_oracle(0.1), 0.5);
    let fail = verify(&fail_gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("below threshold"));
}

#[tokio::test]
async fn integration_rung_passes_and_fails_with_scripts() {
    let tmp = tempdir();
    let script = write_script(
        tmp.path(),
        "integration.sh",
        "#!/usr/bin/env bash\nprintf 'integration ok\\n'\nexit 0\n",
    );
    let gate = IntegrationGate::script(&script)
        .with_warmup_ms(0)
        .with_timeout_ms(60_000);
    let signal = cargo_signal(tmp.path());

    let pass = verify(&gate, &signal).await;
    assert!(pass.passed);
    assert!(pass.gate.starts_with("integration:script:"));

    write_script(
        tmp.path(),
        "integration.sh",
        "#!/usr/bin/env bash\nprintf 'integration failed\\n' >&2\nexit 1\n",
    );
    let fail = verify(&gate, &signal).await;
    assert!(!fail.passed);
    assert!(fail.reason.contains("exit") || fail.reason.contains("failed"));
}
