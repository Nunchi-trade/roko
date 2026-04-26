//! Sandbox policy & validation (parity §28.10).
//!
//! This module provides **pure policy/validation** logic for subprocess
//! sandboxing. It does **not** spawn processes itself. Callers wrap
//! their own spawn with these checks and honor the verdict.
//!
//! # Model
//!
//! A [`SandboxPolicy`] captures five independent constraints:
//!
//! 1. **Allowed paths**: filesystem roots a subprocess may touch
//!    (recursive — any descendant is allowed).
//! 2. **Denied paths**: filesystem roots the subprocess must **never**
//!    touch. Denies always win over allows when the two overlap
//!    (see [`SandboxEnforcer::check_path`]).
//! 3. **Allowed commands**: the allow-list of program basenames that
//!    may be executed. If the list is empty, everything is denied.
//! 4. **Wall-clock budget** (`max_wall_ms`): hard upper bound on
//!    elapsed real time for a job — checked via
//!    [`SandboxEnforcer::check_wall_ms`].
//! 5. **Environment allow-list**: env-var names the subprocess is
//!    permitted to inherit. Callers use
//!    [`SandboxEnforcer::filter_env`] to scrub a proposed env-map
//!    before spawning.
//!
//! # Design choices
//!
//! - **Deny wins.** If a path sits inside both an allowed root and a
//!   denied root, the denial wins (narrowest-scope-of-deny rule).
//! - **Absolute paths only.** Relative paths are rejected at policy
//!   construction time with [`SandboxError::RelativePath`]. This
//!   prevents `../../etc/passwd`–style escapes.
//! - **Basename matching for commands.** The allow-list stores
//!   program basenames (e.g. `cargo`, `git`), not full paths, so the
//!   same policy works whether the caller invokes `cargo` on `$PATH`
//!   or `/usr/local/bin/cargo`.
//! - **Silent filtering for env vars.** Unlisted env keys are removed
//!   rather than reported as errors; this matches typical `env -i`
//!   sandbox semantics and keeps callers terse.
//!
//! # Non-goals
//!
//! This module does **not** interact with Landlock, sandbox-exec,
//! seccomp, or any kernel mechanism. It is validation-only. Platform
//! enforcement backends live in a separate spec (see
//! `COMPONENTS/orchestrator/sandboxing.md`) and wrap this module.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised while building or enforcing a [`SandboxPolicy`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SandboxError {
    /// A path was not absolute. Policies must use absolute paths so
    /// that `..` traversal cannot escape the declared roots.
    #[error("relative path rejected: {0}")]
    RelativePath(PathBuf),

    /// The requested path is not contained in any allow-root.
    #[error("path not allowed: {0}")]
    PathNotAllowed(PathBuf),

    /// The requested path sits under a deny-root (deny overrides allow).
    #[error("path denied: {0}")]
    PathDenied(PathBuf),

    /// The program's basename is not on the command allow-list.
    #[error("command not allowed: {0}")]
    CommandNotAllowed(String),

    /// The program path had no basename (empty component), which we
    /// cannot match against the allow-list.
    #[error("command has no basename: {0}")]
    CommandMissingBasename(PathBuf),

    /// The elapsed wall-clock time exceeded the policy's budget.
    #[error("wall-clock budget exceeded: {elapsed_ms}ms > {budget_ms}ms")]
    WallClockExceeded {
        /// Observed elapsed time, in milliseconds.
        elapsed_ms: u64,
        /// Budget configured on the policy, in milliseconds.
        budget_ms: u64,
    },

    /// No wall-clock budget is set on this policy; the caller asked
    /// for one via [`SandboxEnforcer::check_wall_ms`].
    #[error("no wall-clock budget configured")]
    NoWallClockBudget,
}

// ---------------------------------------------------------------------------
// React
// ---------------------------------------------------------------------------

/// Constraints for subprocess execution.
///
/// Construct via [`SandboxPolicy::builder`] for validation; the public
/// fields are kept accessible so tests & inspectors can read back the
/// declared policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    /// Absolute paths the subprocess may read/write (recursive).
    pub allowed_paths: Vec<PathBuf>,
    /// Absolute paths the subprocess must never touch (recursive).
    /// Wins over `allowed_paths` on overlap.
    pub denied_paths: Vec<PathBuf>,
    /// Allow-list of program basenames. Empty = no command allowed.
    pub allowed_commands: BTreeSet<String>,
    /// Hard upper bound on wall-clock runtime, in milliseconds.
    pub max_wall_ms: Option<u64>,
    /// Env-var names that survive [`SandboxEnforcer::filter_env`].
    pub env_allow_list: BTreeSet<String>,
}

impl SandboxPolicy {
    /// A fully-closed policy: denies all paths, all commands, and all
    /// env vars. Callers extend from here.
    #[must_use]
    pub fn closed() -> Self {
        Self::default()
    }

    /// Start building a validated policy.
    #[must_use]
    pub fn builder() -> SandboxPolicyBuilder {
        SandboxPolicyBuilder::default()
    }
}

/// Builder for [`SandboxPolicy`]. Validates absoluteness on every add.
#[derive(Debug, Default)]
pub struct SandboxPolicyBuilder {
    allowed_paths: Vec<PathBuf>,
    denied_paths: Vec<PathBuf>,
    allowed_commands: BTreeSet<String>,
    max_wall_ms: Option<u64>,
    env_allow_list: BTreeSet<String>,
}

impl SandboxPolicyBuilder {
    /// Append an allow-root. Must be absolute.
    ///
    /// # Errors
    /// Returns [`SandboxError::RelativePath`] if `path` is not absolute.
    pub fn allow_path<P: AsRef<Path>>(mut self, path: P) -> Result<Self, SandboxError> {
        let p = path.as_ref();
        if !p.is_absolute() {
            return Err(SandboxError::RelativePath(p.to_path_buf()));
        }
        self.allowed_paths.push(normalize(p));
        Ok(self)
    }

    /// Append a deny-root. Must be absolute.
    ///
    /// # Errors
    /// Returns [`SandboxError::RelativePath`] if `path` is not absolute.
    pub fn deny_path<P: AsRef<Path>>(mut self, path: P) -> Result<Self, SandboxError> {
        let p = path.as_ref();
        if !p.is_absolute() {
            return Err(SandboxError::RelativePath(p.to_path_buf()));
        }
        self.denied_paths.push(normalize(p));
        Ok(self)
    }

    /// Add a program basename to the command allow-list.
    #[must_use]
    pub fn allow_command(mut self, name: impl Into<String>) -> Self {
        self.allowed_commands.insert(name.into());
        self
    }

    /// Configure the wall-clock budget (milliseconds).
    #[must_use]
    pub const fn max_wall_ms(mut self, ms: u64) -> Self {
        self.max_wall_ms = Some(ms);
        self
    }

    /// Permit a specific environment-variable name.
    #[must_use]
    pub fn allow_env(mut self, key: impl Into<String>) -> Self {
        self.env_allow_list.insert(key.into());
        self
    }

    /// Finalize the builder into an immutable policy.
    #[must_use]
    pub fn build(self) -> SandboxPolicy {
        SandboxPolicy {
            allowed_paths: self.allowed_paths,
            denied_paths: self.denied_paths,
            allowed_commands: self.allowed_commands,
            max_wall_ms: self.max_wall_ms,
            env_allow_list: self.env_allow_list,
        }
    }
}

// ---------------------------------------------------------------------------
// Enforcer
// ---------------------------------------------------------------------------

/// Validates proposed subprocess actions against a [`SandboxPolicy`].
///
/// The enforcer holds a reference to its policy and performs no I/O —
/// callers feed it the candidate path / command / elapsed-time and
/// receive a verdict.
#[derive(Debug)]
pub struct SandboxEnforcer<'p> {
    policy: &'p SandboxPolicy,
}

impl<'p> SandboxEnforcer<'p> {
    /// Build an enforcer bound to `policy`.
    #[must_use]
    pub const fn new(policy: &'p SandboxPolicy) -> Self {
        Self { policy }
    }

    /// Expose the underlying policy (for logging / audit).
    #[must_use]
    pub const fn policy(&self) -> &SandboxPolicy {
        self.policy
    }

    /// Check whether `path` may be accessed.
    ///
    /// # Errors
    ///
    /// - [`SandboxError::RelativePath`] if `path` is not absolute.
    /// - [`SandboxError::PathDenied`] if `path` is under a deny-root.
    /// - [`SandboxError::PathNotAllowed`] if `path` is outside every
    ///   allow-root. (Denies are checked first — "deny wins".)
    pub fn check_path<P: AsRef<Path>>(&self, path: P) -> Result<(), SandboxError> {
        let p = path.as_ref();
        if !p.is_absolute() {
            return Err(SandboxError::RelativePath(p.to_path_buf()));
        }
        let candidate = normalize(p);

        // Deny wins — check denies first.
        if self
            .policy
            .denied_paths
            .iter()
            .any(|root| is_contained(&candidate, root))
        {
            return Err(SandboxError::PathDenied(candidate));
        }

        if self
            .policy
            .allowed_paths
            .iter()
            .any(|root| is_contained(&candidate, root))
        {
            Ok(())
        } else {
            Err(SandboxError::PathNotAllowed(candidate))
        }
    }

    /// Check whether `program` is on the command allow-list.
    ///
    /// `program` may be a bare name (`cargo`) or an absolute path
    /// (`/usr/local/bin/cargo`); only the file basename is matched.
    ///
    /// # Errors
    ///
    /// - [`SandboxError::CommandMissingBasename`] if `program` has no
    ///   trailing component we can extract.
    /// - [`SandboxError::CommandNotAllowed`] if the basename is not on
    ///   the allow-list.
    pub fn check_command<P: AsRef<Path>>(&self, program: P) -> Result<(), SandboxError> {
        let p = program.as_ref();
        let basename = p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| SandboxError::CommandMissingBasename(p.to_path_buf()))?;
        if self.policy.allowed_commands.contains(basename) {
            Ok(())
        } else {
            Err(SandboxError::CommandNotAllowed(basename.to_owned()))
        }
    }

    /// Verify that `elapsed_ms` has not blown the policy's wall-clock
    /// budget.
    ///
    /// # Errors
    ///
    /// - [`SandboxError::NoWallClockBudget`] if the policy has none.
    /// - [`SandboxError::WallClockExceeded`] if `elapsed_ms` exceeds
    ///   the budget.
    pub fn check_wall_ms(&self, elapsed_ms: u64) -> Result<(), SandboxError> {
        let budget = self
            .policy
            .max_wall_ms
            .ok_or(SandboxError::NoWallClockBudget)?;
        if elapsed_ms > budget {
            Err(SandboxError::WallClockExceeded {
                elapsed_ms,
                budget_ms: budget,
            })
        } else {
            Ok(())
        }
    }

    /// Strip environment variables not on the allow-list.
    ///
    /// Returns a fresh map containing only the keys listed in
    /// `env_allow_list`. Unknown keys are silently dropped so callers
    /// can pipe `std::env::vars()` straight through.
    #[must_use]
    pub fn filter_env<I, K, V>(&self, env: I) -> BTreeMap<String, String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        env.into_iter()
            .filter_map(|(k, v)| {
                let key = k.into();
                if self.policy.env_allow_list.contains(&key) {
                    Some((key, v.into()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Convenience: whether `key` is permitted by the env allow-list.
    #[must_use]
    pub fn env_allows(&self, key: &str) -> bool {
        self.policy.env_allow_list.contains(key)
    }
}

// ---------------------------------------------------------------------------
// Helpers (pure)
// ---------------------------------------------------------------------------

/// Canonicalize the path *logically*: collapse `.` and `..` components
/// without touching the filesystem. We intentionally do NOT call
/// [`std::fs::canonicalize`] here — the target may not exist yet
/// (think: a build output that the subprocess is about to create).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => { /* skip */ }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Returns true when `candidate` is contained in `root` (or equal to
/// it). Both arguments are assumed already normalized.
fn is_contained(candidate: &Path, root: &Path) -> bool {
    candidate.starts_with(root)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{SandboxEnforcer, SandboxError, SandboxPolicy};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn ap(p: &str) -> PathBuf {
        // tests only — absolute path literal for cross-platform test paths
        #[cfg(windows)]
        {
            PathBuf::from(format!("C:\\{}", p.trim_start_matches('/')))
        }
        #[cfg(not(windows))]
        {
            PathBuf::from(p)
        }
    }

    fn base_policy() -> SandboxPolicy {
        SandboxPolicy::builder()
            .allow_path(ap("/workspace"))
            .expect("absolute")
            .allow_path(ap("/tmp/roko"))
            .expect("absolute")
            .deny_path(ap("/workspace/.ssh"))
            .expect("absolute")
            .allow_command("cargo")
            .allow_command("git")
            .max_wall_ms(30_000)
            .allow_env("HOME")
            .allow_env("PATH")
            .build()
    }

    #[test]
    fn path_allow_inside_root() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        assert!(e.check_path(ap("/workspace/src/lib.rs")).is_ok());
        assert!(e.check_path(ap("/tmp/roko/job-1/out.log")).is_ok());
        // root itself is allowed
        assert!(e.check_path(ap("/workspace")).is_ok());
    }

    #[test]
    fn path_deny_outside_allow_roots() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        let err = e.check_path(ap("/etc/passwd")).unwrap_err();
        assert!(matches!(err, SandboxError::PathNotAllowed(_)));
    }

    #[test]
    fn path_deny_wins_over_allow() {
        // /workspace/.ssh sits inside /workspace (allowed) but is
        // explicitly denied — the deny must win.
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        let err = e.check_path(ap("/workspace/.ssh/id_rsa")).unwrap_err();
        assert!(
            matches!(err, SandboxError::PathDenied(_)),
            "expected PathDenied, got {err:?}"
        );
    }

    #[test]
    fn relative_paths_rejected_on_build() {
        let err = SandboxPolicy::builder()
            .allow_path("relative/path")
            .unwrap_err();
        assert!(matches!(err, SandboxError::RelativePath(_)));

        let err2 = SandboxPolicy::builder()
            .deny_path("also/relative")
            .unwrap_err();
        assert!(matches!(err2, SandboxError::RelativePath(_)));
    }

    #[test]
    fn relative_path_rejected_on_check() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        // `check_path` refuses relative input even if the policy is
        // otherwise permissive.
        let err = e.check_path("relative/foo").unwrap_err();
        assert!(matches!(err, SandboxError::RelativePath(_)));
    }

    #[test]
    fn path_traversal_normalized() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        // /workspace/../etc/passwd normalizes to /etc/passwd, which is
        // not in the allow list.
        let err = e.check_path(ap("/workspace/../etc/passwd")).unwrap_err();
        assert!(matches!(err, SandboxError::PathNotAllowed(_)));
    }

    #[test]
    fn command_whitelist_basename_match() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        assert!(e.check_command("cargo").is_ok());
        assert!(e.check_command(ap("/usr/local/bin/cargo")).is_ok());
        assert!(e.check_command("git").is_ok());
    }

    #[test]
    fn command_whitelist_rejects_unknown() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        let err = e.check_command("rm").unwrap_err();
        match err {
            SandboxError::CommandNotAllowed(name) => assert_eq!(name, "rm"),
            other => panic!("expected CommandNotAllowed, got {other:?}"),
        }
    }

    #[test]
    fn empty_command_allow_list_denies_everything() {
        let pol = SandboxPolicy::closed();
        let e = SandboxEnforcer::new(&pol);
        assert!(matches!(
            e.check_command("cargo"),
            Err(SandboxError::CommandNotAllowed(_))
        ));
    }

    #[test]
    fn env_allow_list_filters_unlisted_keys() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        assert!(e.env_allows("HOME"));
        assert!(!e.env_allows("AWS_SECRET_ACCESS_KEY"));

        let src: Vec<(String, String)> = vec![
            ("HOME".into(), "/home/u".into()),
            ("PATH".into(), "/usr/bin".into()),
            ("AWS_SECRET_ACCESS_KEY".into(), "leak".into()),
            ("TERM".into(), "xterm".into()),
        ];
        let filtered: BTreeMap<String, String> = e.filter_env(src);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered.get("HOME").map(String::as_str), Some("/home/u"));
        assert_eq!(filtered.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert!(!filtered.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!filtered.contains_key("TERM"));
    }

    #[test]
    fn wall_ms_within_budget_passes() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        assert!(e.check_wall_ms(0).is_ok());
        assert!(e.check_wall_ms(30_000).is_ok()); // boundary is inclusive
        assert!(e.check_wall_ms(29_999).is_ok());
    }

    #[test]
    fn wall_ms_over_budget_fails() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        let err = e.check_wall_ms(30_001).unwrap_err();
        match err {
            SandboxError::WallClockExceeded {
                elapsed_ms,
                budget_ms,
            } => {
                assert_eq!(elapsed_ms, 30_001);
                assert_eq!(budget_ms, 30_000);
            }
            other => panic!("expected WallClockExceeded, got {other:?}"),
        }
    }

    #[test]
    fn wall_ms_without_budget_errors() {
        let pol = SandboxPolicy::builder().build();
        let e = SandboxEnforcer::new(&pol);
        assert!(matches!(
            e.check_wall_ms(10),
            Err(SandboxError::NoWallClockBudget)
        ));
    }

    #[test]
    fn closed_policy_is_fully_locked_down() {
        let pol = SandboxPolicy::closed();
        let e = SandboxEnforcer::new(&pol);
        assert!(matches!(
            e.check_path(ap("/any/path")),
            Err(SandboxError::PathNotAllowed(_))
        ));
        assert!(matches!(
            e.check_command("ls"),
            Err(SandboxError::CommandNotAllowed(_))
        ));
        assert!(!e.env_allows("HOME"));
    }

    #[test]
    fn policy_round_trips_through_builder() {
        let pol = base_policy();
        assert_eq!(pol.allowed_paths.len(), 2);
        assert_eq!(pol.denied_paths.len(), 1);
        assert!(pol.allowed_commands.contains("cargo"));
        assert!(pol.allowed_commands.contains("git"));
        assert_eq!(pol.max_wall_ms, Some(30_000));
        assert!(pol.env_allow_list.contains("HOME"));
        assert!(pol.env_allow_list.contains("PATH"));

        // Enforcer exposes the same policy for inspection.
        let e = SandboxEnforcer::new(&pol);
        let pol_ptr: *const SandboxPolicy = &raw const pol;
        assert!(std::ptr::eq(e.policy(), pol_ptr));
    }

    #[test]
    fn command_missing_basename_errors() {
        let pol = base_policy();
        let e = SandboxEnforcer::new(&pol);
        // A path that has no basename — e.g. root.
        let err = e.check_command(ap("/")).unwrap_err();
        assert!(
            matches!(err, SandboxError::CommandMissingBasename(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn path_prefix_of_root_does_not_confuse_containment() {
        // `/a/bomb` must NOT be treated as contained in root `/a/b`
        // (string-prefix match would falsely say yes; component-wise
        // `starts_with` on `Path` rejects it).
        let pol = SandboxPolicy::builder()
            .allow_path(ap("/a/b"))
            .expect("absolute")
            .build();
        let e = SandboxEnforcer::new(&pol);
        let err = e.check_path(ap("/a/bomb/x")).unwrap_err();
        assert!(
            matches!(err, SandboxError::PathNotAllowed(_)),
            "prefix-only match must not grant access, got {err:?}"
        );
        assert!(e.check_path(ap("/a/b/x")).is_ok());
    }

    #[test]
    fn overlapping_allow_and_deny_deny_wins_on_nested_root() {
        // Allow /a, deny /a/secret, allow /a/secret/public (nested
        // allow should NOT un-deny — deny wins at any matching
        // ancestor).
        let pol = SandboxPolicy::builder()
            .allow_path(ap("/a"))
            .expect("absolute")
            .deny_path(ap("/a/secret"))
            .expect("absolute")
            .allow_path(ap("/a/secret/public"))
            .expect("absolute")
            .build();
        let e = SandboxEnforcer::new(&pol);
        // /a/secret/public/foo is inside an allow root AND inside a
        // deny root. Deny wins.
        let err = e.check_path(ap("/a/secret/public/foo")).unwrap_err();
        assert!(matches!(err, SandboxError::PathDenied(_)));
        // Sibling path outside the deny root should be fine.
        assert!(e.check_path(ap("/a/open/file")).is_ok());
    }
}
