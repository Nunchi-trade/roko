//! Phase 1 protocol wiring tests -- verifies CLI-level wiring of v2 abstractions.
//!
//! These tests run `roko doctor` as a subprocess and verify that the
//! v2_abstractions check appears in the output, proving the protocol
//! traits (Cell, Observe, Connect, Trigger) are compiled into and
//! reachable from the CLI binary.

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

// ============================================================================
// Doctor v2_abstractions check
// ============================================================================

#[test]
fn doctor_human_output_contains_v2_abstractions() {
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
        .stdout(contains("[ok] v2_abstractions"))
        .stdout(contains("phase 1 protocol abstractions are reachable"));
}

#[test]
fn doctor_json_output_contains_v2_abstractions() {
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
        .arg("--json")
        .assert()
        .success()
        .stdout(contains("\"v2_abstractions\""))
        .stdout(contains("\"ok\""));
}

#[test]
fn doctor_v2_abstractions_passes_even_when_workspace_unhealthy() {
    // The v2_abstractions check is compile-time and should pass even
    // when the workspace itself is not bootstrapped (missing .roko/, etc.).
    let temp = tempdir().unwrap();

    let output = Command::cargo_bin("roko")
        .unwrap()
        .args(["doctor", "--workdir"])
        .arg(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[ok] v2_abstractions"),
        "v2_abstractions should pass even in unhealthy workspace, got:\n{stdout}"
    );
}
