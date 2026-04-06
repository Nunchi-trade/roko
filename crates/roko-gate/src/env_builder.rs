//! Gate environment builder — constructs [`GateEnv`] for each gate rung.
//!
//! Replaces scattered `apply_gate_env` calls with a single builder that
//! knows how to construct the right environment for each rung and build
//! system. The builder pattern lets callers override individual fields
//! without touching the rest.

use crate::payload::BuildSystem;
use std::collections::HashMap;
use std::path::PathBuf;

// ─── GateEnv ────────────────────────────────────────────────────────────

/// A fully-resolved environment snapshot for running a gate subprocess.
///
/// Produced by [`GateEnvBuilder`] or [`build_for_rung`]. The struct is
/// consumed by gate implementations that translate it into
/// `tokio::process::Command` configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateEnv {
    /// Working directory the gate process runs in.
    pub working_dir: PathBuf,

    /// Optional `CARGO_TARGET_DIR` or equivalent override.
    pub target_dir: Option<PathBuf>,

    /// Environment variables to inject into the subprocess.
    pub env_vars: HashMap<String, String>,

    /// Name of the build system (for logging / diagnostics).
    pub build_system_name: String,

    /// Extra CLI arguments appended after the gate's default args.
    pub extra_args: Vec<String>,
}

// ─── GateEnvBuilder ─────────────────────────────────────────────────────

/// Builder for [`GateEnv`]. All fields have sensible defaults; only
/// `working_dir` is mandatory (set at construction).
#[derive(Clone, Debug)]
pub struct GateEnvBuilder {
    working_dir: PathBuf,
    target_dir: Option<PathBuf>,
    env_vars: HashMap<String, String>,
    build_system_name: String,
    extra_args: Vec<String>,
}

impl GateEnvBuilder {
    /// Start building a `GateEnv` for the given working directory.
    #[must_use]
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
            target_dir: None,
            env_vars: HashMap::new(),
            build_system_name: String::new(),
            extra_args: Vec::new(),
        }
    }

    /// Override the target directory (e.g. `CARGO_TARGET_DIR`).
    #[must_use]
    pub fn target_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.target_dir = Some(dir.into());
        self
    }

    /// Add a single environment variable.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Set the build system name for logging.
    #[must_use]
    pub fn build_system_name(mut self, name: impl Into<String>) -> Self {
        self.build_system_name = name.into();
        self
    }

    /// Add extra CLI arguments.
    #[must_use]
    pub fn extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Append a single extra CLI argument.
    #[must_use]
    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Consume the builder and produce a [`GateEnv`].
    #[must_use]
    pub fn build(self) -> GateEnv {
        GateEnv {
            working_dir: self.working_dir,
            target_dir: self.target_dir,
            env_vars: self.env_vars,
            build_system_name: self.build_system_name,
            extra_args: self.extra_args,
        }
    }
}

// ─── build_for_rung ─────────────────────────────────────────────────────

/// Construct an appropriate [`GateEnv`] for a specific rung and build system.
///
/// Rung semantics:
///
/// | Rung | Gate kind      | Notable env tweaks                        |
/// |------|---------------|-------------------------------------------|
/// | 0    | Compile       | Warnings suppressed for fast feedback      |
/// | 1    | Lint          | Warnings promoted to errors                |
/// | 2    | Test          | `RUST_BACKTRACE=1` for diagnostics         |
/// | 3    | Symbol        | No extra env                               |
/// | 4    | Generated test| Same as rung 2 + `NEXTEST_RETRIES=0`       |
/// | 5    | Property test | Higher timeout tolerance                   |
/// | 6    | Integration   | `INTEGRATION=1` marker                     |
#[must_use]
pub fn build_for_rung(
    rung: u8,
    build_system: &str,
    working_dir: impl Into<PathBuf>,
) -> GateEnv {
    let wd = working_dir.into();
    let mut builder = GateEnvBuilder::new(wd).build_system_name(build_system);

    match rung {
        // Compile: suppress warnings for speed
        0 => {
            if build_system == BuildSystem::Cargo.program() {
                builder = builder.env("RUSTFLAGS", "-Awarnings");
            }
        }
        // Lint: promote warnings to errors
        1 => {
            if build_system == BuildSystem::Cargo.program() {
                builder = builder.env("RUSTFLAGS", "-Dwarnings");
            } else if build_system == BuildSystem::Go.program() {
                builder = builder.env("GOFLAGS", "-v");
            }
        }
        // Test: enable backtraces
        2 => {
            if build_system == BuildSystem::Cargo.program() {
                builder = builder.env("RUST_BACKTRACE", "1");
            }
        }
        // Symbol: no special env
        3 => {}
        // Generated test: backtraces + no retries
        4 => {
            if build_system == BuildSystem::Cargo.program() {
                builder = builder
                    .env("RUST_BACKTRACE", "1")
                    .env("NEXTEST_RETRIES", "0");
            }
        }
        // Property test: higher iteration count hint
        5 => {
            if build_system == BuildSystem::Cargo.program() {
                builder = builder.env("PROPTEST_CASES", "256");
            }
        }
        // Integration: marker flag
        6 => {
            builder = builder.env("INTEGRATION", "1");
        }
        // Unknown rung: no special env
        _ => {}
    }

    builder.build()
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_builder_basic_construction() {
        let env = GateEnvBuilder::new("/repo")
            .build_system_name("cargo")
            .build();
        assert_eq!(env.working_dir, PathBuf::from("/repo"));
        assert_eq!(env.build_system_name, "cargo");
        assert!(env.target_dir.is_none());
        assert!(env.env_vars.is_empty());
        assert!(env.extra_args.is_empty());
    }

    #[test]
    fn env_builder_full_chain() {
        let env = GateEnvBuilder::new("/project")
            .target_dir("/tmp/target")
            .build_system_name("cargo")
            .env("RUST_LOG", "debug")
            .env("CI", "true")
            .extra_args(vec!["--release".into()])
            .extra_arg("--verbose")
            .build();
        assert_eq!(env.working_dir, PathBuf::from("/project"));
        assert_eq!(env.target_dir, Some(PathBuf::from("/tmp/target")));
        assert_eq!(env.build_system_name, "cargo");
        assert_eq!(env.env_vars.get("RUST_LOG").unwrap(), "debug");
        assert_eq!(env.env_vars.get("CI").unwrap(), "true");
        assert_eq!(env.extra_args, vec!["--release", "--verbose"]);
    }

    #[test]
    fn env_builder_env_overwrites_previous() {
        let env = GateEnvBuilder::new("/repo")
            .env("KEY", "first")
            .env("KEY", "second")
            .build();
        assert_eq!(env.env_vars.get("KEY").unwrap(), "second");
        assert_eq!(env.env_vars.len(), 1);
    }

    #[test]
    fn env_builder_default_fields() {
        let env = GateEnvBuilder::new("/x").build();
        assert_eq!(env.working_dir, PathBuf::from("/x"));
        assert!(env.target_dir.is_none());
        assert!(env.env_vars.is_empty());
        assert!(env.build_system_name.is_empty());
        assert!(env.extra_args.is_empty());
    }

    #[test]
    fn env_builder_clone_independence() {
        let builder = GateEnvBuilder::new("/repo")
            .env("A", "1")
            .build_system_name("cargo");
        let clone = builder.clone();
        let env1 = builder.build();
        let env2 = clone.build();
        assert_eq!(env1, env2);
    }

    #[test]
    fn build_for_rung_compile_suppresses_warnings() {
        let env = build_for_rung(0, "cargo", "/repo");
        assert_eq!(env.env_vars.get("RUSTFLAGS").unwrap(), "-Awarnings");
        assert_eq!(env.build_system_name, "cargo");
    }

    #[test]
    fn build_for_rung_lint_promotes_warnings() {
        let env = build_for_rung(1, "cargo", "/repo");
        assert_eq!(env.env_vars.get("RUSTFLAGS").unwrap(), "-Dwarnings");
    }

    #[test]
    fn build_for_rung_test_enables_backtrace() {
        let env = build_for_rung(2, "cargo", "/repo");
        assert_eq!(env.env_vars.get("RUST_BACKTRACE").unwrap(), "1");
    }

    #[test]
    fn build_for_rung_symbol_has_no_extra_env() {
        let env = build_for_rung(3, "cargo", "/repo");
        assert!(env.env_vars.is_empty());
    }

    #[test]
    fn build_for_rung_generated_test_sets_retries() {
        let env = build_for_rung(4, "cargo", "/repo");
        assert_eq!(env.env_vars.get("RUST_BACKTRACE").unwrap(), "1");
        assert_eq!(env.env_vars.get("NEXTEST_RETRIES").unwrap(), "0");
    }

    #[test]
    fn build_for_rung_property_test_sets_cases() {
        let env = build_for_rung(5, "cargo", "/repo");
        assert_eq!(env.env_vars.get("PROPTEST_CASES").unwrap(), "256");
    }

    #[test]
    fn build_for_rung_integration_sets_marker() {
        let env = build_for_rung(6, "cargo", "/repo");
        assert_eq!(env.env_vars.get("INTEGRATION").unwrap(), "1");
    }

    #[test]
    fn build_for_rung_non_cargo_compile_no_rustflags() {
        let env = build_for_rung(0, "npm", "/repo");
        assert!(!env.env_vars.contains_key("RUSTFLAGS"));
        assert_eq!(env.build_system_name, "npm");
    }

    #[test]
    fn build_for_rung_go_lint_sets_goflags() {
        let env = build_for_rung(1, "go", "/repo");
        assert_eq!(env.env_vars.get("GOFLAGS").unwrap(), "-v");
    }

    #[test]
    fn build_for_rung_unknown_rung_has_no_extra_env() {
        let env = build_for_rung(99, "cargo", "/repo");
        assert!(env.env_vars.is_empty());
    }

    #[test]
    fn build_for_rung_integration_applies_to_all_build_systems() {
        for bs in ["cargo", "npm", "go", "python3", "forge", "make"] {
            let env = build_for_rung(6, bs, "/repo");
            assert_eq!(
                env.env_vars.get("INTEGRATION").unwrap(),
                "1",
                "integration marker missing for {bs}"
            );
        }
    }
}
