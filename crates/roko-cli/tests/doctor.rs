//! Integration tests for `roko doctor`.

use assert_cmd::Command;
use predicates::str::contains;
use roko_fs::{LayoutVersion, RokoLayout};
use tempfile::tempdir;

fn bootstrap_layout(workdir: &std::path::Path) {
    let layout = RokoLayout::for_project(workdir);
    std::fs::create_dir_all(layout.root()).unwrap();
    for dir in layout.top_level_dirs() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(
        layout.version_file(),
        format!("{}\n", LayoutVersion::CURRENT.as_u32()),
    )
    .unwrap();
}

#[test]
fn doctor_json_exits_nonzero_when_workspace_is_not_bootstrapped() {
    let temp = tempdir().unwrap();

    Command::cargo_bin("roko")
        .unwrap()
        .args(["doctor", "--workdir"])
        .arg(temp.path())
        .arg("--json")
        .assert()
        .failure()
        .stdout(contains("\"healthy\": false"))
        .stdout(contains("\"id\": \"config\""))
        .stdout(contains("\"id\": \"layout\""));
}

#[test]
fn doctor_human_output_succeeds_for_bootstrapped_workspace() {
    let temp = tempdir().unwrap();
    std::fs::write(
        temp.path().join("roko.toml"),
        "[agent]\ncommand = \"echo\"\n",
    )
    .unwrap();
    bootstrap_layout(temp.path());

    Command::cargo_bin("roko")
        .unwrap()
        .args(["doctor", "--workdir"])
        .arg(temp.path())
        .assert()
        .success()
        .stdout(contains("doctor: ok"))
        .stdout(contains("[ok] workdir"))
        .stdout(contains("[ok] layout"))
        .stdout(contains("[skipped] serve_health"));
}
