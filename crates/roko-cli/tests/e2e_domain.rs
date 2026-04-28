//! Integration tests for domain-aware orchestration (ORCH-09).
//!
//! These tests validate that roko can parse and validate tasks.toml files
//! with the new `domain` field, and that the `roko run` path respects
//! domain configuration. We use `cat` as the mock agent backend and `true`
//! as the gate — no LLM needed.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

/// Helper: run a roko CLI command against the given workdir.
///
/// Isolates from the user's global config and API keys so tests use only
/// the config written into the test workdir.
fn roko(workdir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("roko")
        .unwrap()
        .current_dir(workdir)
        .args(args)
        .env("ROKO_LOG", "warn")
        .env("HOME", workdir)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("XDG_CONFIG_HOME")
        .assert()
}

/// Write a roko.toml with `cat` backend and domain-specific config.
fn write_domain_config(workdir: &Path, default_domain: Option<&str>, extra_gates: &str) {
    let domain_line = default_domain
        .map(|d| format!("default_domain = \"{d}\""))
        .unwrap_or_default();
    let config = format!(
        r#"
[project]
name = "domain-test"
{domain_line}

[agent]
command = "cat"
args = []
timeout_ms = 30000

[prompt]
token_budget = 1000
role = "You are a Roko agent."

[gates]
{extra_gates}

[[gate]]
kind = "shell"
program = "true"
args = []
timeout_ms = 5000
"#
    );
    fs::write(workdir.join("roko.toml"), config).unwrap();
}

// -----------------------------------------------------------------------
// Test 1: roko.toml with default_domain parses correctly
// -----------------------------------------------------------------------

#[test]
fn config_with_default_domain_parses() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    roko(workdir, &["init", &workdir.display().to_string()]).success();
    write_domain_config(workdir, Some("research"), "");

    // `roko config show` should succeed — the config is valid.
    roko(workdir, &["config", "show"]).success();
}

// -----------------------------------------------------------------------
// Test 2: roko.toml with domain_gates parses correctly
// -----------------------------------------------------------------------

#[test]
fn config_with_domain_gates_parses() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    roko(workdir, &["init", &workdir.display().to_string()]).success();
    write_domain_config(
        workdir,
        Some("research"),
        "[gates.domain_gates]\nresearch = [\"shell:true\"]\ndocs = [\"shell:markdownlint .\"]\n",
    );

    // Config should parse without errors.
    roko(workdir, &["config", "show"]).success();
}

// -----------------------------------------------------------------------
// Test 3: tasks.toml with domain field validates successfully
// -----------------------------------------------------------------------

#[test]
fn tasks_with_domain_field_validates() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    roko(workdir, &["init", &workdir.display().to_string()]).success();
    write_domain_config(workdir, None, "");

    let plan_dir = workdir.join("plans").join("domain-validate");
    fs::create_dir_all(&plan_dir).unwrap();

    fs::write(
        plan_dir.join("tasks.toml"),
        r#"[meta]
plan = "domain-validate"
total = 3

[[task]]
id = "t1"
title = "Research task"
description = "A research-domain task"
domain = "research"
role = "researcher"
depends_on = []

[[task]]
id = "t2"
title = "Docs task"
description = "A docs-domain task"
domain = "docs"
role = "scribe"
depends_on = []

[[task]]
id = "t3"
title = "Code task"
description = "A code-domain task"
domain = "code"
role = "implementer"
depends_on = ["t1"]
"#,
    )
    .unwrap();
    fs::write(plan_dir.join("plan.md"), "# Domain validation test\n").unwrap();

    // Plan validation should pass with the domain fields present.
    roko(workdir, &["plan", "validate", "plans"]).success();
}

// -----------------------------------------------------------------------
// Test 4: roko run with research default_domain uses shell gate (not cargo)
// -----------------------------------------------------------------------

#[test]
fn run_with_research_domain_uses_shell_gate() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    // Init workspace — no Cargo.toml, so cargo check would fail.
    roko(workdir, &["init", &workdir.display().to_string()]).success();
    write_domain_config(workdir, Some("research"), "");

    // `roko run` with cat should succeed even without Cargo.toml,
    // because the [[gate]] in the config is `true`.
    roko(
        workdir,
        &[
            "run",
            "--engine",
            "legacy",
            "summarize the codebase",
            "--workdir",
            &workdir.display().to_string(),
        ],
    )
    .success();
}

// -----------------------------------------------------------------------
// Test 5: tasks.toml with custom domain validates
// -----------------------------------------------------------------------

#[test]
fn tasks_with_custom_domain_validates() {
    let tmp = TempDir::new().unwrap();
    let workdir = tmp.path();

    roko(workdir, &["init", &workdir.display().to_string()]).success();
    write_domain_config(workdir, None, "");

    let plan_dir = workdir.join("plans").join("custom-domain");
    fs::create_dir_all(&plan_dir).unwrap();

    fs::write(
        plan_dir.join("tasks.toml"),
        r#"[meta]
plan = "custom-domain"
total = 1

[[task]]
id = "t1"
title = "Custom domain task"
description = "A task with a user-defined domain"
domain = "blockchain-audit"
role = "implementer"
depends_on = []
"#,
    )
    .unwrap();
    fs::write(plan_dir.join("plan.md"), "# Custom domain test\n").unwrap();

    roko(workdir, &["plan", "validate", "plans"]).success();
}
