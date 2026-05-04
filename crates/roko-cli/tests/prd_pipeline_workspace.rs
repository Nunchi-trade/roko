//! End-to-end coverage for explicit `--repo` PRD pipeline commands.

mod common;

use assert_cmd::Command;
use assert_cmd::assert::Assert;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const FIXTURE: &str = "mock-prd-pipeline-fixture";
const IDEA: &str = "Build a workspace anchored PRD pipeline";
const TITLE: &str = "workspace anchored feature";
const SLUG: &str = "workspace-anchored-feature";

#[test]
fn explicit_repo_prd_pipeline_artifacts_stay_in_selected_workspace() {
    let selected = TempDir::new().expect("selected workspace");
    let decoy = TempDir::new().expect("decoy cwd");
    let selected_root = selected.path();
    let decoy_root = decoy.path();

    common::init_workspace(selected_root);
    common::seed_minimal_rust_project(selected_root);
    common::init_workspace(decoy_root);
    common::seed_minimal_rust_project(decoy_root);

    run_roko_from(decoy_root, selected_root, &["prd", "idea", IDEA]).success();

    let selected_ideas = read(selected_root.join(".roko/prd/ideas.md"));
    assert!(
        selected_ideas.contains(IDEA),
        "selected workspace ideas.md missing idea:\n{selected_ideas}"
    );
    let decoy_ideas = fs::read_to_string(decoy_root.join(".roko/prd/ideas.md")).unwrap_or_default();
    assert!(
        !decoy_ideas.contains(IDEA),
        "decoy workspace received idea despite --repo:\n{decoy_ideas}"
    );

    run_roko_from(decoy_root, selected_root, &["prd", "draft", "new", TITLE]).success();

    let selected_draft = selected_root
        .join(".roko/prd/drafts")
        .join(format!("{SLUG}.md"));
    assert!(
        selected_draft.exists(),
        "draft missing from selected workspace: {}",
        selected_draft.display()
    );
    assert!(
        !decoy_root
            .join(".roko/prd/drafts")
            .join(format!("{SLUG}.md"))
            .exists(),
        "draft was written to decoy workspace"
    );

    run_roko_from(
        decoy_root,
        selected_root,
        &["prd", "draft", "promote", SLUG],
    )
    .success();

    let selected_published = selected_root
        .join(".roko/prd/published")
        .join(format!("{SLUG}.md"));
    assert!(
        selected_published.exists(),
        "published PRD missing from selected workspace: {}",
        selected_published.display()
    );

    run_roko_from(decoy_root, selected_root, &["prd", "plan", SLUG]).success();

    let selected_plan_dir = selected_root.join(".roko/plans").join(SLUG);
    let selected_tasks = selected_plan_dir.join("tasks.toml");
    let selected_plan_md = selected_plan_dir.join("plan.md");
    assert!(
        selected_tasks.exists(),
        "tasks.toml missing from selected workspace: {}",
        selected_tasks.display()
    );
    assert!(
        selected_plan_md.exists(),
        "plan.md missing from selected workspace: {}",
        selected_plan_md.display()
    );
    assert!(
        !decoy_root.join(".roko/plans").join(SLUG).exists(),
        "plan was written to decoy workspace"
    );

    let tasks = read(&selected_tasks);
    assert!(
        tasks.contains("Verify explicit workspace routing"),
        "generated tasks.toml missing fixture task:\n{tasks}"
    );

    run_roko_from(
        decoy_root,
        selected_root,
        &["plan", "validate", ".roko/plans"],
    )
    .success();
}

fn run_roko_from(current_dir: &Path, repo: &Path, args: &[&str]) -> Assert {
    Command::cargo_bin("roko")
        .expect("roko binary")
        .current_dir(current_dir)
        .arg("--repo")
        .arg(repo)
        .args(args)
        .env("HOME", repo)
        .env("ROKO_DISPATCHER", FIXTURE)
        .env("ROKO_MOCK_STATE_PATH", mock_state_path(repo))
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("XDG_CONFIG_HOME")
        .assert()
}

fn mock_state_path(workdir: &Path) -> PathBuf {
    workdir
        .join(".roko")
        .join("state")
        .join("prd-pipeline-fixture-turn.txt")
}

fn read(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}
