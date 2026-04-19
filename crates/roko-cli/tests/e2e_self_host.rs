//! End-to-end self-hosting smoke test.

use assert_cmd::Command;
use roko_orchestrator::ExecutorSnapshot;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use tempfile::TempDir;

const FIXTURE: &str = "mock-self-host-fixture";
const IDEA_TEXT: &str = "Test: wire XYZ";
const SLUG: &str = "test-wire-xyz";

#[test]
fn self_hosting_workflow_with_mock_dispatcher() {
    let tmp = TempDir::new().expect("tempdir");
    let workdir = tmp.path();

    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(workdir)
        .assert()
        .success();

    seed_minimal_rust_project(workdir);
    seed_git_repo(workdir);
    fs::create_dir_all(workdir.join("plans")).expect("create top-level plans dir");
    enable_auto_plan(workdir);

    run_cli(workdir, &["prd", "idea", IDEA_TEXT]).success();
    let ideas_path = workdir.join(".roko").join("prd").join("ideas.md");
    let ideas = fs::read_to_string(&ideas_path).expect("read ideas");
    assert!(
        ideas.contains(IDEA_TEXT),
        "ideas.md missing captured idea: {ideas}"
    );

    run_cli(workdir, &["prd", "draft", "new", SLUG]).success();
    let draft_path = workdir
        .join(".roko")
        .join("prd")
        .join("drafts")
        .join(format!("{SLUG}.md"));
    let draft = fs::read_to_string(&draft_path).expect("read draft");
    assert!(
        draft.contains("status: draft"),
        "draft did not materialize: {draft}"
    );
    assert!(
        draft.contains("REQ-001"),
        "draft missing expected requirements: {draft}"
    );

    run_cli(workdir, &["research", "enhance-prd", SLUG]).success();

    run_cli(workdir, &["prd", "draft", "promote", SLUG]).success();
    let published_path = workdir
        .join(".roko")
        .join("prd")
        .join("published")
        .join(format!("{SLUG}.md"));
    assert!(
        published_path.exists(),
        "published PRD missing at {}",
        published_path.display()
    );

    let plan_dir = workdir.join("plans").join(SLUG);
    let tasks_path = plan_dir.join("tasks.toml");
    assert!(
        tasks_path.exists(),
        "tasks.toml missing at {}",
        tasks_path.display()
    );
    assert!(
        plan_dir.join("plan.md").exists(),
        "plan.md missing at {}",
        plan_dir.display()
    );

    run_cli(workdir, &["plan", "validate", "plans"]).success();
    remove_index_plan_if_present(workdir);
    commit_workspace_state(workdir, "prepare self-host smoke plan");
    seed_plan_branch(workdir, SLUG);
    run_cli(workdir, &["plan", "run", "plans"]).success();

    let root_episodes_path = workdir.join(".roko").join("episodes.jsonl");
    let root_episodes = fs::read_to_string(&root_episodes_path).expect("read root episodes");
    assert!(
        root_episodes
            .lines()
            .any(|line| line.contains(r#""kind":"prd_published""#)),
        "root episodes.jsonl missing prd_published entry: {root_episodes}"
    );

    let memory_episodes_path = workdir.join(".roko").join("memory").join("episodes.jsonl");
    let memory_episodes = fs::read_to_string(&memory_episodes_path).expect("read memory episodes");
    assert!(
        memory_episodes
            .lines()
            .any(|line| line.contains(r#""kind":"agent_turn""#)),
        "memory episodes.jsonl missing agent_turn entry: {memory_episodes}"
    );

    let executor_path = workdir.join(".roko").join("state").join("executor.json");
    let executor_json = fs::read_to_string(&executor_path).expect("read executor snapshot");
    let snapshot = ExecutorSnapshot::from_json(&executor_json).expect("parse executor snapshot");
    assert!(
        snapshot.plan_count() >= 1,
        "executor snapshot did not record any plans"
    );

    let signals_path = workdir.join(".roko").join("engrams.jsonl");
    let signals = fs::read_to_string(&signals_path).expect("read signals");
    assert!(
        signals
            .lines()
            .any(|line| line.contains(r#""gate_verdict""#)),
        "engrams.jsonl missing gate verdict: {signals}"
    );

    let status = run_cli(workdir, &["status"]).success();
    let stdout = String::from_utf8_lossy(&status.get_output().stdout);
    assert!(
        stdout.contains("signal counts"),
        "status output missing signal summary: {stdout}"
    );
}

fn enable_auto_plan(workdir: &Path) {
    let config_path = workdir.join("roko.toml");
    let config = fs::read_to_string(&config_path).expect("read roko.toml");
    let updated = config.replace("auto_plan = false", "auto_plan = true");
    fs::write(&config_path, updated).expect("write roko.toml");
}

fn seed_minimal_rust_project(workdir: &Path) {
    fs::create_dir_all(workdir.join("src")).expect("create src dir");
    fs::write(
        workdir.join("Cargo.toml"),
        r#"[package]
name = "self-host-smoke"
version = "0.1.0"
edition = "2024"

[dependencies]
"#,
    )
    .expect("write Cargo.toml");
    fs::write(
        workdir.join("src").join("main.rs"),
        "fn main() {\n    println!(\"self-host smoke\");\n}\n",
    )
    .expect("write src/main.rs");
}

fn seed_git_repo(workdir: &Path) {
    run_process(workdir, &["git", "init"]);
    run_process(workdir, &["git", "config", "user.name", "Self Host Test"]);
    run_process(
        workdir,
        &["git", "config", "user.email", "self-host@example.com"],
    );
    run_process(workdir, &["git", "add", "."]);
    run_process(workdir, &["git", "commit", "-m", "seed"]);
}

fn remove_index_plan_if_present(workdir: &Path) {
    let index_path = workdir.join("plans").join("INDEX.md");
    if index_path.exists() {
        fs::remove_file(&index_path).expect("remove plans/INDEX.md");
    }
}

fn commit_workspace_state(workdir: &Path, message: &str) {
    run_process(workdir, &["git", "add", "."]);
    run_process(workdir, &["git", "commit", "-m", message]);
}

fn seed_plan_branch(workdir: &Path, plan_id: &str) {
    let main_branch = git_stdout(workdir, &["git", "branch", "--show-current"]);
    let branch = format!("roko/plan/{plan_id}");
    run_process(workdir, &["git", "checkout", "-b", &branch]);
    run_process(workdir, &["git", "checkout", &main_branch]);
}

fn run_process(workdir: &Path, args: &[&str]) {
    let (program, rest) = args.split_first().expect("process command");
    let status = ProcessCommand::new(program)
        .current_dir(workdir)
        .args(rest)
        .status()
        .unwrap_or_else(|err| panic!("spawn {program}: {err}"));
    assert!(
        status.success(),
        "{program} {:?} failed with {status}",
        rest
    );
}

fn git_stdout(workdir: &Path, args: &[&str]) -> String {
    let (program, rest) = args.split_first().expect("process command");
    let output = ProcessCommand::new(program)
        .current_dir(workdir)
        .args(rest)
        .output()
        .unwrap_or_else(|err| panic!("spawn {program}: {err}"));
    assert!(
        output.status.success(),
        "{program} {:?} failed with {}",
        rest,
        output.status
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn run_cli(workdir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    Command::cargo_bin("roko")
        .unwrap()
        .current_dir(workdir)
        .args(args)
        .env("ROKO_DISPATCHER", FIXTURE)
        .env("ROKO_MOCK_STATE_PATH", mock_state_path(workdir))
        .assert()
}

fn mock_state_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("state")
        .join("mock-dispatcher-turn.txt")
}
