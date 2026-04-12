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
/// };
/// let sig = Signal::builder(Kind::Task)
///     .body(Body::from_json(&payload).unwrap())
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
    #[must_use]
    #[allow(clippy::match_same_arms)]
    pub const fn check_args(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &["check", "--workspace", "--all-targets"],
            Self::Npm => &["run", "build"],
            Self::Go => &["build", "./..."],
            Self::Python => &["-c", "import ast; ast.parse(open('.').read())"],
            Self::Forge => &["build"],
            Self::Make => &["build"],
        }
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

    /// The default "lint" command for this build system.
    ///
    /// For Cargo this is `cargo clippy --workspace --all-targets
    /// -- -D warnings`. Callers that want a softer lint (warnings → warnings)
    /// can append their own args via the gate's `with_extra_args`.
    #[must_use]
    pub const fn lint_args(self) -> &'static [&'static str] {
        match self {
            Self::Cargo => &[
                "clippy",
                "--workspace",
                "--all-targets",
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
        let json = serde_json::to_string(&p).unwrap();
        let parsed: GatePayload = serde_json::from_str(&json).unwrap();
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
}
