//! Signal body payloads consumed and emitted by gates.
//!
//! These are structured types that round-trip through [`Body::Json`]. They
//! live here (not in a shared `roko-types` crate) until a second crate needs
//! them; then we extract.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The input payload a gate expects to find in a signal's body.
///
/// A `GatePayload` tells a gate where to run and what to check. Concrete
/// gates like [`CompileGate`](crate::CompileGate) read the signal's body
/// as a `GatePayload`, then act on its fields.
///
/// # Example
///
/// ```
/// use roko_core::{Signal, Kind, Body};
/// use roko_gate::GatePayload;
/// use std::path::PathBuf;
///
/// let payload = GatePayload {
///     working_dir: PathBuf::from("/repo"),
///     target_dir: None,
///     extra_env: vec![],
///     label: Some("check-login-module".into()),
///     target_crates: vec!["roko-agent".into()],
/// };
/// let sig = Signal::builder(Kind::Task)
///     .body(Body::from_json(&payload).expect("example payload should serialize"))
///     .build();
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatePayload {
    /// Directory to run the gate in (the repo root or a worktree).
    pub working_dir: PathBuf,

    /// Optional `CARGO_TARGET_DIR` override (for shared-cache setups).
    pub target_dir: Option<PathBuf>,

    /// Additional environment variables to set for the gate's subprocess.
    pub extra_env: Vec<(String, String)>,

    /// Optional identifying label for logging (e.g. plan/task id).
    pub label: Option<String>,

    /// Crates modified by the task. When non-empty, compile and lint gates
    /// use `-p <crate>` instead of `--workspace` so pre-existing errors in
    /// unrelated crates don't cause false gate failures.
    #[serde(default)]
    pub target_crates: Vec<String>,
}

impl GatePayload {
    /// Construct a payload that runs in `working_dir` with no overrides.
    #[must_use]
    pub fn in_dir(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
            target_dir: None,
            extra_env: Vec::new(),
            label: None,
            target_crates: Vec::new(),
        }
    }

    /// Set the `CARGO_TARGET_DIR` override.
    #[must_use]
    pub fn with_target_dir(mut self, target: impl Into<PathBuf>) -> Self {
        self.target_dir = Some(target.into());
        self
    }

    /// Add an environment variable to the subprocess.
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.push((key.into(), value.into()));
        self
    }

    /// Attach a log label.
    #[must_use]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Scope compile/lint gates to specific crates (uses `-p` instead of `--workspace`).
    #[must_use]
    pub fn with_target_crates(mut self, crates: Vec<String>) -> Self {
        self.target_crates = crates;
        self
    }
}

/// Which build system a compile-style gate should use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BuildSystem {
    /// `cargo check`/`cargo test` (Rust).
    Cargo,
    /// `npm run build`/`npm test` (Node).
    Npm,
    /// `go build`/`go test` (Go).
    Go,
    /// `python -m pytest`/`python -c` (Python).
    Python,
    /// `forge build`/`forge test` (Foundry).
    Forge,
    /// `make build`/`make test` (Make).
    Make,
}

impl BuildSystem {
    /// Detect the build system from common root marker files in `workdir`.
    #[must_use]
    pub fn detect(workdir: &Path) -> Self {
        if workdir.join("Cargo.toml").exists() {
            return Self::Cargo;
        }
        if workdir.join("package.json").exists() {
            return Self::Npm;
        }
        if workdir.join("go.mod").exists() {
            return Self::Go;
        }
        if workdir.join("pyproject.toml").exists() || workdir.join("setup.py").exists() {
            return Self::Python;
        }
        if workdir.join("foundry.toml").exists() {
            return Self::Forge;
        }
        Self::Make
    }

    /// The default "check" command for this build system (args after the program).
    ///
    /// For Cargo, uses `--workspace --lib` (no `--all-targets`) so test-only
    /// compilation errors do not block the compile gate. Test compilation is
    /// handled by [`TestGate`] instead.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn check_args(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["check", "--workspace", "--lib"],
            Self::Npm => &["run", "build"],
            Self::Go => &["build", "./..."],
            Self::Python => &["-c", "import ast; ast.parse(open('.').read())"],
            Self::Forge => &["build"],
            Self::Make => &["build"],
        }
    }

    /// Build a scoped check command targeting specific crates.
    ///
    /// When `crates` is non-empty, emits `-p <name>` for each crate instead
    /// of `--workspace`. Falls back to [`check_args`](Self::check_args) when
    /// the list is empty or the build system is not Cargo.
    #[must_use]
    pub fn scoped_check_args(self, crates: &[String]) -> Vec<String> {
        let crates = cargo_package_scope(crates);
        if self != Self::Cargo || crates.is_empty() {
            return self.check_args().iter().map(|s| (*s).to_owned()).collect();
        }
        let mut args = vec!["check".to_owned()];
        for krate in crates {
            args.push("-p".to_owned());
            args.push(krate.to_owned());
        }
        args.push("--lib".to_owned());
        args
    }

    /// The default "test" command for this build system.
    ///
    /// For Cargo, prefers plain `cargo test` (nextest support is added by
    /// extra-args / config; detection of nextest at build-time ships with §10.5).
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn test_args(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["test", "--workspace"],
            Self::Npm => &["test"],
            Self::Go => &["test", "./..."],
            Self::Python => &["-m", "pytest"],
            Self::Forge => &["test"],
            Self::Make => &["test"],
        }
    }

    /// Build a scoped test command targeting specific crates.
    ///
    /// When `crates` is non-empty, emits `-p <name>` for each crate instead
    /// of `--workspace`. Falls back to [`test_args`](Self::test_args) when
    /// the list is empty or the build system is not Cargo.
    #[must_use]
    pub fn scoped_test_args(self, crates: &[String]) -> Vec<String> {
        let crates = cargo_package_scope(crates);
        if self != Self::Cargo || crates.is_empty() {
            return self.test_args().iter().map(|s| (*s).to_owned()).collect();
        }
        let mut args = vec!["test".to_owned()];
        for krate in crates {
            args.push("-p".to_owned());
            args.push(krate.to_owned());
        }
        args
    }

    /// The default "lint" command for this build system.
    ///
    /// For Cargo this is `cargo clippy --workspace --lib --no-deps -- -D warnings`.
    /// Uses `--lib` (not `--all-targets`) so test-only lint errors do not
    /// block the gate, and `--no-deps` to skip linting third-party code.
    /// Callers that want a softer lint can append their own args via the
    /// gate's `with_extra_args`.
    #[must_use]
    pub const fn lint_args(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &[
                "clippy",
                "--workspace",
                "--lib",
                "--no-deps",
                "--",
                "-D",
                "warnings",
            ],
            Self::Npm => &["run", "lint"],
            Self::Go => &["vet", "./..."],
            Self::Python => &["-m", "ruff", "check", "."],
            Self::Forge => &["fmt", "--check"],
            Self::Make => &["lint"],
        }
    }

    /// Build a scoped lint command targeting specific crates.
    ///
    /// When `crates` is non-empty, emits `-p <name>` for each crate instead
    /// of `--workspace`. Falls back to [`lint_args`](Self::lint_args) when
    /// the list is empty or the build system is not Cargo.
    #[must_use]
    pub fn scoped_lint_args(self, crates: &[String]) -> Vec<String> {
        let crates = cargo_package_scope(crates);
        if self != Self::Cargo || crates.is_empty() {
            return self.lint_args().iter().map(|s| (*s).to_owned()).collect();
        }
        let mut args = vec!["clippy".to_owned()];
        for krate in crates {
            args.push("-p".to_owned());
            args.push(krate.to_owned());
        }
        args.push("--lib".to_owned());
        args.push("--no-deps".to_owned());
        args.push("--".to_owned());
        args.push("-D".to_owned());
        args.push("warnings".to_owned());
        args
    }

    /// Human-readable name for this build system (for log messages).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cargo => "Cargo",
            Self::Npm => "npm",
            Self::Go => "Go",
            Self::Python => "Python",
            Self::Forge => "Forge",
            Self::Make => "Make",
        }
    }

    /// The program (binary) invoked for this build system.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Go => "go",
            Self::Python => "python3",
            Self::Forge => "forge",
            Self::Make => "make",
        }
    }

    /// Returns `true` when the build system's program binary exists on `PATH`.
    #[must_use]
    pub fn is_available(&self) -> bool {
        let program = self.program();
        let path_var = std::env::var("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path_var) {
            if dir.join(program).is_file() {
                return true;
            }
        }
        false
    }
}

// ─── TestSelector ────────────────────────────────────────────────────────

/// Which tests the `TestGate` should run.
///
/// Mirrors Mori's `TestSelector` (see
/// `apps/mori/src/orchestrator/gates.rs`) and the §10.5 `TestGate` spec.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode", content = "data")]
#[non_exhaustive]
pub enum TestSelector {
    /// Every test in the project.
    All,
    /// Lib/unit tests only (fast iteration loops).
    Quick,
    /// Tests matching the given glob/name patterns.
    Patterns(Vec<String>),
    /// Tests touching changed files / affected crates.
    AffectedOnly,
}

impl TestSelector {
    /// Extra arguments to append to `test_args` for this selector.
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub fn extra_args(&self, build: BuildSystem) -> Vec<String> {
        match (self, build) {
            (Self::All | Self::AffectedOnly, _) => Vec::new(),
            (Self::Quick, BuildSystem::Cargo) => vec!["--lib".into()],
            (Self::Quick, BuildSystem::Npm) => vec![
                "--".into(),
                "--testPathIgnorePatterns".into(),
                "integration".into(),
            ],
            (Self::Quick, BuildSystem::Go) => vec!["-short".into()],
            (Self::Quick, BuildSystem::Python) => vec![
                "-m".into(),
                "pytest".into(),
                "-m".into(),
                "not integration".into(),
            ],
            (Self::Quick, _) => Vec::new(),
            (Self::Patterns(ps), BuildSystem::Npm) => {
                let mut v = vec!["--".into()];
                v.extend(ps.clone());
                v
            }
            (Self::Patterns(ps), BuildSystem::Go) => {
                if ps.is_empty() {
                    Vec::new()
                } else {
                    vec!["-run".into(), ps.join("|")]
                }
            }
            (Self::Patterns(ps), BuildSystem::Python) => {
                let mut v = vec!["-k".into()];
                v.push(ps.join(" or "));
                v
            }
            (Self::Patterns(ps), _) => ps.clone(),
        }
    }
}

fn cargo_package_scope(crates: &[String]) -> Vec<&str> {
    crates
        .iter()
        .map(String::as_str)
        .filter(|name| !name.is_empty() && *name != "workspace")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_marker(dir: &Path, marker: &str) {
        std::fs::write(dir.join(marker), "").expect("write marker");
    }

    #[test]
    fn builder_chain() {
        let p = GatePayload::in_dir("/repo")
            .with_target_dir("/tmp/target")
            .with_env("RUST_LOG", "debug")
            .with_label("plan-42");
        assert_eq!(p.working_dir, PathBuf::from("/repo"));
        assert_eq!(
            p.target_dir.as_deref(),
            Some(std::path::Path::new("/tmp/target"))
        );
        assert_eq!(p.extra_env, vec![("RUST_LOG".into(), "debug".into())]);
        assert_eq!(p.label.as_deref(), Some("plan-42"));
    }

    #[test]
    fn serde_roundtrip() {
        let p = GatePayload::in_dir("/x").with_label("y");
        let json = serde_json::to_string(&p)
            .expect("invariant: gate payload should serialize in round-trip test");
        let parsed: GatePayload = serde_json::from_str(&json)
            .expect("invariant: serialized gate payload should deserialize");
        assert_eq!(p, parsed);
    }

    #[test]
    fn build_system_programs() {
        assert_eq!(BuildSystem::Cargo.program(), "cargo");
        assert_eq!(BuildSystem::Npm.program(), "npm");
    }

    #[test]
    fn build_system_check_args() {
        assert!(BuildSystem::Cargo.check_args().contains(&"check"));
    }

    #[test]
    fn cargo_scoped_test_args_use_packages_not_workspace() {
        let args =
            BuildSystem::Cargo.scoped_test_args(&["roko-acp".to_string(), "roko-gate".to_string()]);

        assert_eq!(
            args,
            vec![
                "test".to_string(),
                "-p".to_string(),
                "roko-acp".to_string(),
                "-p".to_string(),
                "roko-gate".to_string(),
            ]
        );
    }

    #[test]
    fn cargo_workspace_sentinel_uses_workspace_args() {
        let crates = ["workspace".to_string()];

        assert_eq!(
            BuildSystem::Cargo.scoped_check_args(&crates),
            BuildSystem::Cargo
                .check_args()
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            BuildSystem::Cargo.scoped_test_args(&crates),
            BuildSystem::Cargo
                .test_args()
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            BuildSystem::Cargo.scoped_lint_args(&crates),
            BuildSystem::Cargo
                .lint_args()
                .iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_system_detect_rust() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "Cargo.toml");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Cargo);
    }

    #[test]
    fn build_system_detect_node() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "package.json");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Npm);
    }

    #[test]
    fn build_system_detect_go() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "go.mod");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Go);
    }

    #[test]
    fn build_system_detect_python_pyproject() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "pyproject.toml");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Python);
    }

    #[test]
    fn build_system_detect_python_setup_py() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "setup.py");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Python);
    }

    #[test]
    fn build_system_detect_solidity() {
        let dir = TempDir::new().expect("tempdir");
        write_marker(dir.path(), "foundry.toml");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Forge);
    }

    #[test]
    fn build_system_detect_falls_back_to_make() {
        let dir = TempDir::new().expect("tempdir");
        assert_eq!(BuildSystem::detect(dir.path()), BuildSystem::Make);
    }

    #[test]
    fn build_system_name_returns_human_readable() {
        assert_eq!(BuildSystem::Cargo.name(), "Cargo");
        assert_eq!(BuildSystem::Npm.name(), "npm");
        assert_eq!(BuildSystem::Go.name(), "Go");
        assert_eq!(BuildSystem::Python.name(), "Python");
        assert_eq!(BuildSystem::Forge.name(), "Forge");
        assert_eq!(BuildSystem::Make.name(), "Make");
    }

    #[test]
    fn cargo_is_available_on_dev_machine() {
        // On any machine running `cargo test`, cargo must be on PATH.
        assert!(BuildSystem::Cargo.is_available());
    }

    #[test]
    fn is_available_walks_path_directories() {
        // Verify the lookup logic works by checking that programs
        // on PATH are found. `cargo` must exist since we're running
        // cargo test; a made-up program should not.
        let path_var = std::env::var("PATH").unwrap_or_default();
        let cargo_found = std::env::split_paths(&path_var)
            .any(|dir| dir.join("cargo").is_file());
        // cargo must be findable on PATH if we got here
        assert!(cargo_found);
    }
}
