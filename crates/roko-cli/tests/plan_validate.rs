//! Integration coverage for `roko plan validate`.

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn write_plan(root: &Path, plan_id: &str, tasks_toml: &str) -> PathBuf {
    let plan_dir = root.join("plans").join(plan_id);
    fs::create_dir_all(&plan_dir).unwrap();
    fs::write(plan_dir.join("tasks.toml"), tasks_toml).unwrap();
    plan_dir
}

fn run_validate(temp: &TempDir, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("roko")
        .unwrap()
        .current_dir(temp.path())
        .arg("plan")
        .arg("validate")
        .args(args)
        .assert()
}

#[test]
fn plan_validate_help_shows_new_flags() {
    let assert = Command::cargo_bin("roko")
        .unwrap()
        .arg("plan")
        .arg("validate")
        .arg("--help")
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("--strict"),
        "missing --strict flag: {stdout}"
    );
    assert!(stdout.contains("--json"), "missing --json flag: {stdout}");
    assert!(
        stdout.contains("[DIR]") || stdout.contains("[dir]"),
        "missing directory argument in help: {stdout}"
    );
}

#[test]
fn plan_validate_succeeds_for_well_formed_plan() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "good",
        r#"
[meta]
plan = "good"

[[task]]
id = "T1"
title = "Implement the validator"
role = "implementer"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("0 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
}

#[test]
fn plan_validate_reports_cycles() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "cycle",
        r#"
[meta]
plan = "cycle"

[[task]]
id = "T1"
title = "First"
role = "implementer"
depends_on = ["T2"]
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]

[[task]]
id = "T2"
title = "Second"
role = "implementer"
depends_on = ["T1"]
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("PLAN_006"), "missing PLAN_006: {stdout}");
}

#[test]
fn plan_validate_reports_missing_role_templates() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "missing-template",
        r#"
[meta]
plan = "missing-template"

[[task]]
id = "T1"
title = "Validate a missing template"
role = "researcher"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans", "--strict"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("PLAN_008"), "missing PLAN_008: {stdout}");
}

#[test]
fn plan_validate_reports_invalid_gate_rungs() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "bad-rung",
        r#"
[meta]
plan = "bad-rung"

[[task]]
id = "T1"
title = "Use an invalid rung"
role = "implementer"
gate_rung = 9
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("PLAN_007"), "missing PLAN_007: {stdout}");
}

#[test]
fn plan_validate_accepts_typed_acceptance_contract() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "contract",
        r#"
[meta]
plan = "contract"

[[task]]
id = "T1"
title = "Define a done gate"
role = "implementer"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-gate" }]

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
command = "cargo check -p roko-gate"

[task.acceptance_contract.no_stub]
production_paths = ["crates/roko-gate/src"]

[task.acceptance_contract.agent_output]
schema = "roko.acceptance.agent_output.v1"

[task.acceptance_contract.review_verdict]
reviewer_role_id = "quick-reviewer"
min_confidence = 0.6

[task.acceptance_contract.recovery]
retry = true
reflection = true
replan = true

[task.acceptance_contract.parity_ledger]

[[task.acceptance_contract.parity_ledger.rows]]
requirement_id = "RT00.done-gate"
source_ref = "tmp/architecture-plans/08-end-to-end-acceptance.md"
evidence_ref = "crates/roko-gate/src/acceptance_contract.rs"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("0 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
}

#[test]
fn plan_validate_fails_closed_for_malformed_acceptance_contract() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "bad-contract",
        r#"
[meta]
plan = "bad-contract"

[[task]]
id = "T1"
title = "Define a bad done gate"
role = "implementer"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-gate" }]

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("ACCEPT_003"),
        "missing ACCEPT_003: {stdout}"
    );
}

#[test]
fn plan_validate_accepts_architecture_queue_packets() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "architecture",
        r#"
[meta]
plan = "architecture"
queue_kind = "architecture_implementation"

[[task]]
id = "Q1"
title = "Implement one architecture packet"
role = "implementer"
files = ["crates/roko-core/src/config/schema.rs"]
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-core" }]

[task.context]
read_files = [
  { path = "tmp/architecture-plans/06-architecture-implementation.md", why = "source plan" },
]

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
command = "cargo check -p roko-core"

[task.acceptance_contract.agent_output]
schema = "roko.architecture_packet.v1"

[task.acceptance_contract.review_verdict]
reviewer_role_id = "quick-reviewer"
min_confidence = 0.6

[task.acceptance_contract.recovery]
retry = true
reflection = true

[task.acceptance_contract.parity_ledger]

[[task.acceptance_contract.parity_ledger.rows]]
requirement_id = "ARCH-Q1"
source_ref = "tmp/architecture-plans/06-architecture-implementation.md"
evidence_ref = "crates/roko-core/src/config/schema.rs"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("0 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
}

#[test]
fn plan_validate_fails_closed_for_incomplete_architecture_queue_packets() {
    let temp = TempDir::new().unwrap();
    write_plan(
        temp.path(),
        "architecture",
        r#"
[meta]
plan = "architecture"
queue_kind = "architecture_implementation"

[[task]]
id = "Q1"
title = "Incomplete architecture packet"
role = "implementer"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    for rule in ["PLAN_020", "PLAN_021", "PLAN_022", "PLAN_023", "PLAN_024"] {
        assert!(stdout.contains(rule), "missing {rule}: {stdout}");
    }
}
