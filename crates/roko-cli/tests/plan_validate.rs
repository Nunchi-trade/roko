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

fn write_model_registry(root: &Path) {
    fs::write(
        root.join("roko.toml"),
        r#"
[models.claude-haiku-4-5]
provider = "claude_cli"
slug = "claude-haiku-4-5"

[models.claude-sonnet-4-6]
provider = "claude_cli"
slug = "claude-sonnet-4-6"

[models.claude-opus-4-6]
provider = "claude_cli"
slug = "claude-opus-4-6"
"#,
    )
    .unwrap();
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
fn plan_validate_warns_on_known_model_aliases() {
    let temp = TempDir::new().unwrap();
    write_model_registry(temp.path());
    write_plan(
        temp.path(),
        "aliases",
        r#"
[meta]
plan = "aliases"

[[task]]
id = "T1"
title = "Mechanical alias"
role = "implementer"
model_hint = "haiku"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]

[[task]]
id = "T2"
title = "Focused alias"
role = "implementer"
model_hint = "sonnet"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]

[[task]]
id = "T3"
title = "Architectural alias"
role = "implementer"
model_hint = "opus"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("3 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
    assert!(stdout.contains("PLAN_009"), "missing PLAN_009: {stdout}");
    assert!(
        stdout.contains("uses model alias 'haiku'; use full name 'claude-haiku-4-5' instead"),
        "missing haiku alias warning: {stdout}"
    );
    assert!(
        stdout.contains("uses model alias 'sonnet'; use full name 'claude-sonnet-4-6' instead"),
        "missing sonnet alias warning: {stdout}"
    );
    assert!(
        stdout.contains("uses model alias 'opus'; use full name 'claude-opus-4-6' instead"),
        "missing opus alias warning: {stdout}"
    );
}

#[test]
fn plan_validate_preserves_unknown_model_warning() {
    let temp = TempDir::new().unwrap();
    write_model_registry(temp.path());
    write_plan(
        temp.path(),
        "mystery-model",
        r#"
[meta]
plan = "mystery-model"

[[task]]
id = "T1"
title = "Unknown model"
role = "implementer"
model_hint = "definitely-not-a-model"
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("1 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
    assert!(stdout.contains("PLAN_009"), "missing PLAN_009: {stdout}");
    assert!(
        stdout.contains("uses model 'definitely-not-a-model' which is not configured in roko.toml"),
        "missing original mystery-model warning: {stdout}"
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

#[test]
fn plan_validate_requires_parity_rows_for_architecture_queue_packets() {
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
title = "Architecture packet without parity closure"
role = "implementer"
files = ["crates/roko-gate/src/acceptance_contract.rs"]
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-gate" }]

[task.context]
read_files = [
  { path = "tmp/architecture-plans/06-architecture-implementation.md", why = "source plan" },
]

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
command = "cargo check -p roko-gate"

[task.acceptance_contract.agent_output]
schema = "roko.architecture_packet.v1"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("PLAN_025"), "missing PLAN_025: {stdout}");
}

#[test]
fn plan_validate_requires_complete_architecture_deferral_metadata() {
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
title = "Deferred advanced packet"
role = "implementer"
files = ["plans/architecture-core-queue/tasks.toml"]
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]

[task.context]
read_files = [
  { path = "docs/08-chain/INDEX.md", why = "advanced source inventory" },
]

[task.deferral]
rationale = "Advanced surface must wait for trustworthy execution."

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
command = "cargo check -p roko-cli"

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
requirement_id = "ARCH-DEFER"
source_ref = "docs/08-chain/INDEX.md"
evidence_ref = "plans/architecture-core-queue/tasks.toml"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).failure();
    assert_eq!(assert.get_output().status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("PLAN_026"), "missing PLAN_026: {stdout}");
    assert!(
        stdout.contains("deferral.prerequisite_runtime_policy_gates"),
        "missing prerequisite field diagnostic: {stdout}"
    );
}

#[test]
fn plan_validate_accepts_complete_architecture_deferral_metadata() {
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
title = "Deferred advanced packet"
role = "implementer"
files = ["plans/architecture-core-queue/tasks.toml"]
depends_on = []
verify = [{ phase = "compile", command = "cargo check -p roko-cli" }]

[task.context]
read_files = [
  { path = "docs/08-chain/INDEX.md", why = "advanced source inventory" },
]

[task.deferral]
rationale = "Advanced surface must wait for trustworthy execution."
prerequisite_runtime_policy_gates = ["structured verdicts pass"]
acceptance_gates = ["cargo check -p roko-cli"]
risk_notes = ["Do not ship behavior before runtime gates are durable."]
parity_requirements = ["ARCH-DEFER"]

[task.acceptance_contract]
version = 1

[[task.acceptance_contract.gates]]
id = "compile"
kind = "compile"
command = "cargo check -p roko-cli"

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
requirement_id = "ARCH-DEFER"
source_ref = "docs/08-chain/INDEX.md"
evidence_ref = "plans/architecture-core-queue/tasks.toml"
"#,
    );

    let assert = run_validate(&temp, &["plans"]).success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stdout.contains("0 diagnostics in 1 plan"),
        "unexpected stdout: {stdout}"
    );
}
