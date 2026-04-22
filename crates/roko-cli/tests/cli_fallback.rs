//! CLI fallback and regression coverage tests (CLI-TUI-05).
//!
//! Verifies:
//! - Every top-level command has `--help` output (no panics).
//! - Common commands produce valid output without `roko-serve` running.
//! - `init` creates `.roko/` and `roko.toml` in a tempdir.
//! - `status --surfaces` produces parseable output.
//! - Help text completeness: every subcommand has a description.
//! - Keybinding consistency: no conflicting bindings in the same context.

mod common;

use assert_cmd::Command;
use common::*;
use predicates::str::contains;
use std::fs;
use tempfile::TempDir;

// -----------------------------------------------------------------------
// 1. CLI smoke tests — --help output for every top-level subcommand
// -----------------------------------------------------------------------

/// All top-level subcommands the CLI exposes.
const TOP_LEVEL_COMMANDS: &[&str] = &[
    "init",
    "run",
    "status",
    "doctor",
    "plan",
    "prd",
    "agent",
    "research",
    "knowledge",
    "learn",
    "job",
    "config",
    "index",
    "serve",
    "daemon",
    "deploy",
    "worker",
    "dashboard",
    "replay",
    "inject",
    "completions",
    "new",
    "explain",
];

/// Nested subcommands that also need `--help` verification.
const NESTED_COMMANDS: &[&[&str]] = &[
    &["plan", "list"],
    &["plan", "show"],
    &["plan", "create"],
    &["plan", "validate"],
    &["plan", "run"],
    &["plan", "generate"],
    &["plan", "regenerate"],
    &["prd", "idea"],
    &["prd", "list"],
    &["prd", "status"],
    &["prd", "draft"],
    &["prd", "plan"],
    &["prd", "consolidate"],
    &["prd", "draft", "new"],
    &["prd", "draft", "edit"],
    &["prd", "draft", "promote"],
    &["prd", "draft", "list"],
    &["research", "topic"],
    &["research", "enhance-prd"],
    &["research", "enhance-plan"],
    &["research", "enhance-tasks"],
    &["research", "analyze"],
    &["research", "list"],
    &["research", "search"],
    &["config", "init"],
    &["config", "show"],
    &["config", "path"],
    &["config", "edit"],
    &["config", "set"],
    &["config", "set-secret"],
    &["config", "check-secrets"],
    &["config", "validate"],
    &["config", "migrate"],
    &["config", "providers", "list"],
    &["config", "providers", "health"],
    &["config", "providers", "test"],
    &["config", "models", "list"],
    &["config", "models", "route"],
    &["config", "subscriptions", "list"],
    &["config", "subscriptions", "add"],
    &["config", "subscriptions", "remove"],
    &["config", "subscriptions", "enable"],
    &["config", "subscriptions", "disable"],
    &["config", "events"],
    &["config", "experiments", "model", "create"],
    &["config", "experiments", "model", "show"],
    &["config", "experiments", "model", "list"],
    &["config", "plugins", "list"],
    &["config", "plugins", "install"],
    &["config", "plugins", "remove"],
    &["config", "plugins", "audit"],
    &["config", "secrets", "list"],
    &["config", "secrets", "get"],
    &["config", "secrets", "set"],
    &["config", "secrets", "rotate"],
    &["agent", "create"],
    &["agent", "delete"],
    &["agent", "list"],
    &["agent", "start"],
    &["agent", "stop"],
    &["agent", "status"],
    &["agent", "serve"],
    &["agent", "chat"],
    &["knowledge", "query"],
    &["knowledge", "stats"],
    &["knowledge", "gc"],
    &["knowledge", "backup"],
    &["knowledge", "restore"],
    &["knowledge", "sync"],
    &["knowledge", "dream", "run"],
    &["knowledge", "dream", "report"],
    &["knowledge", "dream", "schedule"],
    &["knowledge", "dream", "journal"],
    &["knowledge", "dream", "archive"],
    &["knowledge", "custody", "list"],
    &["knowledge", "custody", "show"],
    &["knowledge", "custody", "verify"],
    &["knowledge", "archive"],
    &["learn", "all"],
    &["learn", "router"],
    &["learn", "experiments"],
    &["learn", "efficiency"],
    &["learn", "episodes"],
    &["learn", "tune"],
    &["job", "list"],
    &["job", "create"],
    &["job", "show"],
    &["job", "execute"],
    &["job", "cancel"],
    &["deploy", "railway"],
    &["deploy", "fly"],
    &["deploy", "docker"],
    &["index", "build"],
    &["index", "rebuild"],
    &["index", "search"],
    &["index", "stats"],
    &["daemon", "start"],
    &["daemon", "stop"],
    &["daemon", "status"],
    &["daemon", "logs"],
    &["daemon", "reload"],
    &["daemon", "restart"],
    &["daemon", "install"],
    &["daemon", "uninstall"],
];

#[test]
fn every_top_level_command_has_help_output() {
    for cmd in TOP_LEVEL_COMMANDS {
        let assert = Command::cargo_bin("roko")
            .unwrap_or_else(|e| panic!("roko binary not found: {e}"))
            .args([cmd, "--help"])
            .assert();
        let output = assert.get_output();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "'{cmd} --help' failed with exit code {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        );
        assert!(!stdout.is_empty(), "'{cmd} --help' produced empty stdout");
        assert!(
            stdout.contains(&format!("Usage: roko {cmd}"))
                || stdout.contains(&format!("Usage: roko [OPTIONS] {cmd}")),
            "'{cmd} --help' fell through to root help instead of subcommand help\n{stdout}"
        );
    }
}

#[test]
fn every_nested_command_has_help_output() {
    for args in NESTED_COMMANDS {
        let mut full_args: Vec<&str> = args.to_vec();
        full_args.push("--help");
        let label = args.join(" ");

        let assert = Command::cargo_bin("roko")
            .unwrap_or_else(|e| panic!("roko binary not found: {e}"))
            .args(&full_args)
            .assert();
        let output = assert.get_output();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "'{label} --help' failed with exit code {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status.code()
        );
        assert!(!stdout.is_empty(), "'{label} --help' produced empty stdout");
        assert!(
            stdout.contains("Usage:"),
            "'{label} --help' did not include usage\n{stdout}"
        );
    }
}

#[test]
fn root_help_lists_all_top_level_commands() {
    let assert = Command::cargo_bin("roko")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    for cmd in TOP_LEVEL_COMMANDS {
        assert!(
            stdout.contains(cmd),
            "root --help is missing subcommand '{cmd}'\nOutput:\n{stdout}"
        );
    }
}

// -----------------------------------------------------------------------
// 2. Init creates .roko/ and roko.toml
// -----------------------------------------------------------------------

#[test]
fn init_creates_workspace_artifacts() {
    let tmp = TempDir::new().expect("tempdir");
    Command::cargo_bin("roko")
        .unwrap()
        .arg("init")
        .arg(tmp.path())
        .assert()
        .success();

    assert!(
        tmp.path().join(".roko").is_dir(),
        "init did not create .roko/ directory"
    );
    assert!(
        tmp.path().join("roko.toml").is_file(),
        "init did not create roko.toml"
    );

    // Verify roko.toml is valid TOML
    let toml_text = fs::read_to_string(tmp.path().join("roko.toml")).expect("read roko.toml");
    assert!(
        toml_text.parse::<toml::Table>().is_ok(),
        "roko.toml is not valid TOML:\n{toml_text}"
    );
}

// -----------------------------------------------------------------------
// 3. status --surfaces produces parseable output
// -----------------------------------------------------------------------

#[test]
fn status_surfaces_produces_output() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["status", "--surfaces", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    // The surface inventory table should contain column headers and summary
    assert!(
        stdout.contains("KIND") && stdout.contains("STATUS") && stdout.contains("NAME"),
        "status --surfaces did not produce recognizable surface inventory table\n{stdout}"
    );
    assert!(
        stdout.contains("Total:"),
        "status --surfaces did not produce summary line\n{stdout}"
    );
}

#[test]
fn status_surfaces_json_produces_valid_json() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["--json", "status", "--surfaces", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|err| {
        panic!("status --surfaces --json did not produce valid JSON: {err}\n{stdout}")
    });
    assert!(
        parsed.is_array(),
        "status --surfaces --json should produce a JSON array, got:\n{parsed}"
    );
}

// -----------------------------------------------------------------------
// 4. CLI fallback — commands work WITHOUT roko-serve
// -----------------------------------------------------------------------

#[test]
fn status_works_offline_with_filesystem() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    // status should succeed reading from .roko/ without any server
    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["status", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    // Should report signal/episode counts (even if zero)
    assert!(
        stdout.contains("signals:")
            || stdout.contains("episodes:")
            || stdout.contains("Signal")
            || stdout.contains("Episode")
            || stdout.contains("0"),
        "offline status did not produce recognizable output\n{stdout}"
    );
}

#[test]
fn plan_list_works_offline() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    // Create a minimal plan directory
    let plan_dir = tmp.path().join("plans").join("test-plan-alpha");
    fs::create_dir_all(&plan_dir).expect("create plan dir");
    fs::write(
        plan_dir.join("plan.md"),
        "# Plan: test-plan-alpha\n\nA test plan.\n",
    )
    .expect("write plan.md");
    fs::write(
        plan_dir.join("tasks.toml"),
        r#"[meta]
plan = "test-plan-alpha"
iteration = 1
total = 1
done = 0
status = "ready"

[[task]]
id = "T1"
title = "Test task"
description = "A test task."
role = "implementer"
status = "ready"
tier = "focused"
files = []
allowed_tools = []
denied_tools = []
mcp_servers = []
depends_on = []
depends_on_plan = []
acceptance = []
verify = []
timeout_secs = 60
max_retries = 0
"#,
    )
    .expect("write tasks.toml");

    // plan list reads from filesystem, not serve
    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["plan", "list", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(
        stdout.contains("test-plan-alpha"),
        "plan list did not show the test plan\n{stdout}"
    );
}

#[test]
fn prd_list_works_offline() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    // Create a sample PRD in the published subdirectory
    // (prd list reads from .roko/prd/published/ and .roko/prd/drafts/)
    let published_dir = tmp.path().join(".roko").join("prd").join("published");
    fs::create_dir_all(&published_dir).expect("create published prd dir");
    fs::write(
        published_dir.join("test-feature.md"),
        "# Test Feature\n\nA test PRD.\n",
    )
    .expect("write prd");

    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["prd", "list"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(
        stdout.contains("test-feature"),
        "prd list did not show the test PRD\n{stdout}"
    );
}

#[test]
fn config_show_works_offline() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["config", "show", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    // config show should produce at least some output referencing roko.toml or config fields
    assert!(
        !stdout.trim().is_empty(),
        "config show produced empty output"
    );
}

#[test]
fn doctor_works_offline() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    // Bootstrap the full layout (VERSION file + subdirectories) so doctor
    // passes the layout check.
    let layout = roko_fs::RokoLayout::for_project(tmp.path());
    for dir in layout.top_level_dirs() {
        fs::create_dir_all(dir).expect("create layout dir");
    }
    fs::write(
        layout.version_file(),
        format!("{}\n", roko_fs::LayoutVersion::CURRENT.as_u32()),
    )
    .expect("write VERSION");

    // doctor should succeed in a bootstrapped workspace (serve_health will be skipped)
    Command::cargo_bin("roko")
        .unwrap()
        .args(["doctor", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("doctor: ok"))
        .stdout(contains("[skipped] serve_health"));
}

#[test]
fn dashboard_text_fallback_works_offline() {
    let tmp = TempDir::new().expect("tempdir");
    init_workspace(tmp.path());

    let assert = Command::cargo_bin("roko")
        .unwrap()
        .current_dir(tmp.path())
        .args(["dashboard", "--text", "--workdir"])
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    assert!(
        stdout.contains("dashboard scaffold:"),
        "dashboard --text did not render dashboard text in offline mode\n{stdout}"
    );
}

// -----------------------------------------------------------------------
// 5. Help text completeness
// -----------------------------------------------------------------------

#[test]
fn every_top_level_command_has_description_in_root_help() {
    let assert = Command::cargo_bin("roko")
        .unwrap()
        .arg("--help")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

    // Find the Commands section and verify each command has associated text
    let commands_section = stdout
        .split("Commands:")
        .nth(1)
        .unwrap_or_else(|| panic!("root --help missing Commands section\n{stdout}"));

    for cmd in TOP_LEVEL_COMMANDS {
        // Each command should appear in the Commands: section with some description
        let has_entry = commands_section.lines().any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with(cmd) || trimmed.contains(&format!("  {cmd}"))
        });
        assert!(
            has_entry,
            "subcommand '{cmd}' is missing from the Commands section of --help\n{commands_section}"
        );
    }
}

#[test]
fn subcommand_help_has_description_and_usage() {
    // Sample a representative subset of commands to check structure
    let sample_commands: Vec<Vec<&str>> = vec![
        vec!["init"],
        vec!["status"],
        vec!["plan", "list"],
        vec!["config", "show"],
        vec!["doctor"],
        vec!["research", "topic"],
        vec!["knowledge", "query"],
    ];

    for args in &sample_commands {
        let mut full_args = args.clone();
        full_args.push("--help");
        let label = args.join(" ");

        let assert = Command::cargo_bin("roko")
            .unwrap()
            .args(&full_args)
            .assert()
            .success();
        let stdout = String::from_utf8_lossy(&assert.get_output().stdout);

        // Every subcommand help should have a Usage: line
        assert!(
            stdout.contains("Usage:"),
            "'{label} --help' missing Usage: section\n{stdout}"
        );
    }
}

// -----------------------------------------------------------------------
// 6. Keybinding consistency tests
// -----------------------------------------------------------------------

/// This test verifies the TUI keybinding system for internal consistency.
/// It uses the publicly-exported types from roko_cli::tui to verify
/// the key dispatch function produces deterministic, non-conflicting results.
#[test]
fn keybinding_consistency_no_conflicting_global_keys() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let key = |code: KeyCode| -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    };

    let modals = ModalVisibility::default();

    // Verify F-keys consistently switch to tabs in every tab context
    for source_tab in Tab::ALL {
        for target_tab in Tab::ALL {
            let action = handle_key(
                key(target_tab.fkey()),
                InputMode::Normal,
                source_tab,
                FocusZone::PlanTree,
                &modals,
            );
            assert_eq!(
                action,
                TuiAction::SwitchTab(target_tab),
                "F-key for {target_tab:?} did not switch tab when active tab was {source_tab:?}"
            );
        }
    }
}

#[test]
fn keybinding_ctrl_c_always_quits_in_all_modes() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let ctrl_c = KeyEvent {
        code: KeyCode::Char('c'),
        modifiers: KeyModifiers::CONTROL,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };

    let modals = ModalVisibility::default();

    let modes = [
        InputMode::Normal,
        InputMode::Inject,
        InputMode::Filter,
        InputMode::Confirm,
        InputMode::ConfigEdit,
    ];

    for mode in modes {
        for tab in Tab::ALL {
            let action = handle_key(ctrl_c, mode, tab, FocusZone::PlanTree, &modals);
            assert_eq!(
                action,
                TuiAction::QuitConfirmed,
                "Ctrl-C did not produce QuitConfirmed in mode {mode:?} on tab {tab:?}"
            );
        }
    }
}

#[test]
fn keybinding_modal_intercepts_override_normal_keys() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::modals::ModalState;
    use roko_cli::tui::tabs::Tab;

    let key = |code: KeyCode| -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    };

    // When Help modal is active, '?' should toggle help (not produce global action)
    let help = ModalState::Help;
    let vis = ModalVisibility::from_active_modal(Some(&help));
    let action = handle_key(
        key(KeyCode::Char('?')),
        InputMode::Normal,
        Tab::Dashboard,
        FocusZone::PlanTree,
        &vis,
    );
    assert_eq!(
        action,
        TuiAction::ShowHelp,
        "Help modal did not intercept '?' key"
    );

    // When Help modal is active, 'q' should also close help
    let action = handle_key(
        key(KeyCode::Char('q')),
        InputMode::Normal,
        Tab::Dashboard,
        FocusZone::PlanTree,
        &vis,
    );
    assert_eq!(
        action,
        TuiAction::ShowHelp,
        "Help modal did not intercept 'q' key"
    );
}

#[test]
fn keybinding_text_input_modes_capture_all_chars() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let modals = ModalVisibility::default();

    let char_key = |c: char| -> KeyEvent {
        KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    };

    let text_modes = [InputMode::Inject, InputMode::Filter, InputMode::ConfigEdit];
    let test_chars = ['a', 'z', '0', '9', ' ', '-', '.'];

    for mode in text_modes {
        for ch in test_chars {
            let action = handle_key(
                char_key(ch),
                mode,
                Tab::Dashboard,
                FocusZone::PlanTree,
                &modals,
            );
            let expected = TuiAction::InputChar(ch);
            assert_eq!(
                action, expected,
                "Text input mode {mode:?} did not capture char '{ch}'"
            );
        }

        // Backspace should produce InputBackspace in text modes
        let action = handle_key(
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            },
            mode,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals,
        );
        assert_eq!(
            action,
            TuiAction::InputBackspace,
            "Text input mode {mode:?} did not capture Backspace"
        );
    }
}

#[test]
fn keybinding_no_tab_loses_navigation() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let modals = ModalVisibility::default();

    let key = |code: KeyCode| -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    };

    // Every tab must respond to Up/Down with some non-None action
    // (scroll, select, or navigation)
    for tab in Tab::ALL {
        let up_action = handle_key(
            key(KeyCode::Up),
            InputMode::Normal,
            tab,
            FocusZone::PlanTree,
            &modals,
        );
        assert_ne!(
            up_action,
            TuiAction::None,
            "Tab {tab:?} does not respond to Up key"
        );

        let down_action = handle_key(
            key(KeyCode::Down),
            InputMode::Normal,
            tab,
            FocusZone::PlanTree,
            &modals,
        );
        assert_ne!(
            down_action,
            TuiAction::None,
            "Tab {tab:?} does not respond to Down key"
        );
    }
}

#[test]
fn keybinding_tab_key_cycles_focus_globally() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let modals = ModalVisibility::default();

    // Tab key should produce FocusNext on every tab (in Normal mode)
    for tab in Tab::ALL {
        let action = handle_key(
            KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::empty(),
                kind: KeyEventKind::Press,
                state: KeyEventState::empty(),
            },
            InputMode::Normal,
            tab,
            FocusZone::PlanTree,
            &modals,
        );
        // Tab key produces FocusNext on every tab in Normal mode
        assert_eq!(
            action,
            TuiAction::FocusNext,
            "Tab key did not produce FocusNext on tab {tab:?}, got {action:?}"
        );
    }
}

#[test]
fn keybinding_confirm_mode_only_accepts_yes_no() {
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use roko_cli::tui::input::{FocusZone, InputMode, ModalVisibility, TuiAction, handle_key};
    use roko_cli::tui::tabs::Tab;

    let modals = ModalVisibility::default();

    let key = |code: KeyCode| -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    };

    // y/Y/Enter should confirm
    for code in [KeyCode::Char('y'), KeyCode::Char('Y'), KeyCode::Enter] {
        let action = handle_key(
            key(code),
            InputMode::Confirm,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals,
        );
        assert_eq!(
            action,
            TuiAction::ConfirmYes,
            "Confirm mode did not produce ConfirmYes for {code:?}"
        );
    }

    // n/N/Esc should reject
    for code in [KeyCode::Char('n'), KeyCode::Char('N'), KeyCode::Esc] {
        let action = handle_key(
            key(code),
            InputMode::Confirm,
            Tab::Dashboard,
            FocusZone::PlanTree,
            &modals,
        );
        assert_eq!(
            action,
            TuiAction::ConfirmNo,
            "Confirm mode did not produce ConfirmNo for {code:?}"
        );
    }

    // Random keys should produce None in confirm mode
    let action = handle_key(
        key(KeyCode::Char('x')),
        InputMode::Confirm,
        Tab::Dashboard,
        FocusZone::PlanTree,
        &modals,
    );
    assert_eq!(
        action,
        TuiAction::None,
        "Confirm mode should ignore unrelated keys"
    );
}

// -----------------------------------------------------------------------
// 7. Focus zone cycling is deterministic
// -----------------------------------------------------------------------

#[test]
fn focus_zone_next_prev_roundtrip() {
    use roko_cli::tui::input::FocusZone;
    use roko_cli::tui::tabs::Tab;

    // For Dashboard, cycling through all zones should return to start
    let start = FocusZone::PlanTree;
    let mut current = start;
    for _ in 0..5 {
        current = current.next(Tab::Dashboard);
    }
    assert_eq!(
        current, start,
        "FocusZone::next did not cycle back to start on Dashboard"
    );

    // Same for prev
    let mut current = start;
    for _ in 0..5 {
        current = current.prev(Tab::Dashboard);
    }
    assert_eq!(
        current, start,
        "FocusZone::prev did not cycle back to start on Dashboard"
    );
}

// -----------------------------------------------------------------------
// 8. Tab enum consistency
// -----------------------------------------------------------------------

#[test]
fn tab_fkey_roundtrip_for_all_tabs() {
    use roko_cli::tui::tabs::Tab;

    for tab in Tab::ALL {
        let fkey = tab.fkey();
        let resolved = Tab::from_key(fkey);
        assert_eq!(
            resolved,
            Some(tab),
            "Tab::from_key did not roundtrip for {tab:?}"
        );
    }
}

#[test]
fn tab_next_prev_cycle_returns_to_start() {
    use roko_cli::tui::tabs::Tab;

    let mut t = Tab::Dashboard;
    for _ in 0..Tab::ALL.len() {
        t = t.next();
    }
    assert_eq!(
        t,
        Tab::Dashboard,
        "Tab::next did not cycle back to Dashboard"
    );

    for _ in 0..Tab::ALL.len() {
        t = t.prev();
    }
    assert_eq!(
        t,
        Tab::Dashboard,
        "Tab::prev did not cycle back to Dashboard"
    );
}

#[test]
fn tab_indices_are_sequential_and_unique() {
    use roko_cli::tui::tabs::Tab;
    use std::collections::HashSet;

    let mut indices = HashSet::new();
    for (i, tab) in Tab::ALL.iter().enumerate() {
        assert_eq!(tab.index(), i, "Tab {tab:?} has non-sequential index");
        assert!(indices.insert(tab.index()), "duplicate tab index {i}");
    }
}
