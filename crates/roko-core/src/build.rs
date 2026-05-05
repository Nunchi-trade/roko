//! Build system abstraction — command descriptors without I/O.
//!
//! [`BuildSystem`] defines how a project compiles, tests, lints, and formats.
//! Implementations produce [`BuildCommand`] descriptors that carry program name,
//! arguments, and environment but never execute anything. The execution layer
//! lives in roko-gate or roko-orchestrator.
//!
//! This design keeps roko-core free of `std::process` and `std::fs` so it
//! remains portable, testable, and embeddable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── BuildCommand ────────────────────────────────────────────────────────

/// A fully-specified command descriptor: program, args, env, and working dir.
///
/// This is a pure data structure — it does **not** execute. Verify or orchestrator
/// code converts it into a `tokio::process::Command` (or similar) at the
/// execution boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildCommand {
    /// The program binary to invoke (e.g. `"cargo"`, `"npm"`).
    pub program: String,
    /// Positional arguments.
    pub args: Vec<String>,
    /// Additional environment variables to set.
    pub env: HashMap<String, String>,
    /// Working directory override. `None` means inherit from the caller.
    pub working_dir: Option<PathBuf>,
}

impl BuildCommand {
    /// Create a new command with just a program name.
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
        }
    }

    /// Append a single argument.
    #[must_use]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Append multiple arguments.
    #[must_use]
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set an environment variable.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env.insert(key.into(), val.into());
        self
    }

    /// Set the working directory.
    #[must_use]
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }
}

// ─── BuildSystem trait ───────────────────────────────────────────────────

/// A build system that can produce compile, test, lint, and format commands.
///
/// Implementors (e.g. `CargoBuildSystem`, `NpmBuildSystem`) fill in the
/// concrete commands for their ecosystem. The `detect` associated function
/// checks for marker files and returns `Some(boxed_impl)` if the project root
/// looks like it belongs to that build system.
///
/// # No I/O
///
/// All methods return [`BuildCommand`] descriptors. They must not touch the
/// filesystem or spawn processes.
pub trait BuildSystem: Send + Sync {
    /// Human-readable name (e.g. `"cargo"`, `"npm"`).
    fn name(&self) -> &str;

    /// Command to compile / type-check the project.
    fn compile_cmd(&self, target_dir: &Path) -> BuildCommand;

    /// Command to run tests, optionally filtered.
    fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand;

    /// Command to run the linter.
    fn lint_cmd(&self, target_dir: &Path) -> BuildCommand;

    /// Command to run the formatter.
    fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand;

    /// Check whether `file_names` (names in the project root) indicate this
    /// build system is present. Returns `true` if a marker file is found.
    ///
    /// This takes a slice of file names rather than touching the filesystem so
    /// that roko-core stays I/O-free.
    fn detect_from_files(&self, file_names: &[&str]) -> bool;
}

// ─── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial build system for testing the trait.
    struct FakeBuild;

    impl BuildSystem for FakeBuild {
        fn name(&self) -> &str {
            "fake"
        }

        fn compile_cmd(&self, target_dir: &Path) -> BuildCommand {
            BuildCommand::new("fake-cc")
                .arg("build")
                .working_dir(target_dir)
        }

        fn test_cmd(&self, target_dir: &Path, filter: Option<&str>) -> BuildCommand {
            let mut cmd = BuildCommand::new("fake-cc")
                .arg("test")
                .working_dir(target_dir);
            if let Some(f) = filter {
                cmd = cmd.arg(f);
            }
            cmd
        }

        fn lint_cmd(&self, target_dir: &Path) -> BuildCommand {
            BuildCommand::new("fake-lint").working_dir(target_dir)
        }

        fn format_cmd(&self, target_dir: &Path, check_only: bool) -> BuildCommand {
            let mut cmd = BuildCommand::new("fake-fmt").working_dir(target_dir);
            if check_only {
                cmd = cmd.arg("--check");
            }
            cmd
        }

        fn detect_from_files(&self, file_names: &[&str]) -> bool {
            file_names.contains(&"fake.toml")
        }
    }

    #[test]
    fn build_command_builder() {
        let cmd = BuildCommand::new("cargo")
            .arg("check")
            .args(["--workspace", "--all-targets"])
            .env("RUST_LOG", "debug")
            .working_dir("/repo");

        assert_eq!(cmd.program, "cargo");
        assert_eq!(cmd.args, vec!["check", "--workspace", "--all-targets"]);
        assert_eq!(cmd.env.get("RUST_LOG").map(String::as_str), Some("debug"));
        assert_eq!(cmd.working_dir, Some(PathBuf::from("/repo")));
    }

    #[test]
    fn build_command_new_defaults() {
        let cmd = BuildCommand::new("gcc");
        assert_eq!(cmd.program, "gcc");
        assert!(cmd.args.is_empty());
        assert!(cmd.env.is_empty());
        assert!(cmd.working_dir.is_none());
    }

    #[test]
    fn fake_build_compile_cmd() {
        let fb = FakeBuild;
        let cmd = fb.compile_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "fake-cc");
        assert_eq!(cmd.args, vec!["build"]);
        assert_eq!(cmd.working_dir, Some(PathBuf::from("/proj")));
    }

    #[test]
    fn fake_build_test_cmd_with_filter() {
        let fb = FakeBuild;
        let cmd = fb.test_cmd(Path::new("/proj"), Some("my_test"));
        assert_eq!(cmd.program, "fake-cc");
        assert!(cmd.args.contains(&"my_test".to_string()));
    }

    #[test]
    fn fake_build_test_cmd_without_filter() {
        let fb = FakeBuild;
        let cmd = fb.test_cmd(Path::new("/proj"), None);
        assert_eq!(cmd.args, vec!["test"]);
    }

    #[test]
    fn fake_build_lint_cmd() {
        let fb = FakeBuild;
        let cmd = fb.lint_cmd(Path::new("/proj"));
        assert_eq!(cmd.program, "fake-lint");
    }

    #[test]
    fn fake_build_format_check_only() {
        let fb = FakeBuild;
        let cmd = fb.format_cmd(Path::new("/proj"), true);
        assert!(cmd.args.contains(&"--check".to_string()));
    }

    #[test]
    fn fake_build_format_write() {
        let fb = FakeBuild;
        let cmd = fb.format_cmd(Path::new("/proj"), false);
        assert!(!cmd.args.contains(&"--check".to_string()));
    }

    #[test]
    fn fake_build_detect_positive() {
        let fb = FakeBuild;
        assert!(fb.detect_from_files(&["README.md", "fake.toml", "src"]));
    }

    #[test]
    fn fake_build_detect_negative() {
        let fb = FakeBuild;
        assert!(!fb.detect_from_files(&["README.md", "Cargo.toml"]));
    }

    #[test]
    fn build_system_trait_name() {
        let fb = FakeBuild;
        assert_eq!(fb.name(), "fake");
    }
}
