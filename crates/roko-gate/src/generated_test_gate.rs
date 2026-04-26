//! `GeneratedTestGate` — Rung 3 of the verification ladder (§10.9).
//!
//! Runs *behavioral* tests produced during enrichment (by a `TestGenerator`
//! role) against the implementer's worktree. Unlike [`TestGate`] (Rung 1,
//! the project's own test suite), the generated tests encode the plan's
//! acceptance criteria and are therefore the primary defence against the
//! "did nothing" and "did the wrong thing" failure modes that Rungs 0-2
//! cannot see.
//!
//! # Isolation
//!
//! The implementer agent **never sees the test source text**. The gate
//! loads the generated tests from an immutable [`ArtifactStore`], stages
//! them under a gitignored subdir inside the worktree, runs them via a
//! pattern-scoped [`BuildSystem::test_args`] invocation, and then deletes
//! the staging dir (best-effort, RAII). The staging dir name
//! (`tests/__roko_generated__/`) is deliberately obscure so it is unlikely
//! to collide with user code.
//!
//! # Contract (see `tmp/roko-progress/COMPONENTS/gates/gate-generated-test.md`)
//!
//! - Empty artifact list → `Verdict::pass` with detail
//!   `"no generated tests for this plan"`.
//! - Staging directory is always removed when the gate returns or panics.
//! - Only tests whose name begins with the configured prefix (default
//!   `gen_`) are executed via [`TestSelector::Patterns`].
//! - [`TestCount`] is always surfaced on the verdict (even 0/0/0).
//! - Failing tests are summarised into `error_digest` as a short list of
//!   failing test names; the test *source* is never leaked into the
//!   verdict.
//!
//! Mori reference: no direct analog — Rung 3 is new in Roko. The output
//! parsing mirrors [`crate::test_gate::parse_test_counts`].

use crate::payload::{BuildSystem, GatePayload, TestSelector};
use async_trait::async_trait;
use roko_core::{Context, Engram, TestCount, Verdict, Verify};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Tag key used to locate the plan identifier on the input signal.
///
/// The gate checks, in order: `signal.tag("plan")` → `signal.tag("plan_id")`
/// → the `"plan"` field of the signal body (if the body is a JSON object).
pub const PLAN_TAG_KEY: &str = "plan";
/// Alternate tag key for the plan identifier.
pub const PLAN_TAG_KEY_ALT: &str = "plan_id";
/// Default file-name prefix the `TestGenerator` role uses.
pub const DEFAULT_TEST_PREFIX: &str = "gen_";
/// Default timeout (matches [`crate::test_gate::TestGate`]).
const DEFAULT_TIMEOUT_MS: u64 = 15 * 60 * 1000;
/// Staging subdirectory name, nested under `<worktree>/tests/`.
///
/// The leading/trailing underscores make accidental name collisions with
/// user code very unlikely.
pub const STAGING_SUBDIR: &str = "__roko_generated__";

// ─── ArtifactStore ───────────────────────────────────────────────────────

/// Immutable read-only store of enrichment-time artifacts keyed by plan.
///
/// The generated-test-gate reads generated test files from this store and
/// never writes back. An implementation typically wraps `.roko/plans/`
/// on disk, but a test-only in-memory variant ships with this module.
pub trait ArtifactStore: Send + Sync {
    /// List artifact names under `prefix` for the given plan.
    ///
    /// The returned names must be stable relative paths (no leading `/`,
    /// no `..`) that can be safely joined to a staging directory.
    fn list(&self, plan: &str, prefix: &str) -> Vec<String>;

    /// Read an artifact by plan + name. Returns `None` if missing.
    fn read(&self, plan: &str, name: &str) -> Option<Vec<u8>>;
}

/// In-memory [`ArtifactStore`] used by tests and local orchestration.
///
/// The store is keyed by `(plan, path)` where `path` is the artifact's
/// logical name (e.g. `"generated-tests/gen_foo.rs"`). Populate via the
/// builder-style [`InMemoryArtifactStore::with`] helper before sharing.
#[derive(Clone, Debug, Default)]
pub struct InMemoryArtifactStore {
    entries: BTreeMap<(String, String), Vec<u8>>,
}

impl InMemoryArtifactStore {
    /// Construct an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an artifact by `(plan, name)`.
    #[must_use]
    pub fn with(
        mut self,
        plan: impl Into<String>,
        name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        self.entries
            .insert((plan.into(), name.into()), bytes.into());
        self
    }

    /// Number of artifacts currently held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when the store has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ArtifactStore for InMemoryArtifactStore {
    fn list(&self, plan: &str, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = self
            .entries
            .keys()
            .filter(|(p, n)| p == plan && n.starts_with(prefix))
            .map(|(_, n)| n.clone())
            .collect();
        out.sort();
        out
    }

    fn read(&self, plan: &str, name: &str) -> Option<Vec<u8>> {
        self.entries
            .get(&(plan.to_string(), name.to_string()))
            .cloned()
    }
}

// ─── StagingGuard ────────────────────────────────────────────────────────

/// RAII guard that best-effort removes a staging directory on drop.
///
/// Cleanup failures are swallowed — the gate still reports its verdict.
/// Leaving a stale dir is acceptable for a single gate invocation; the
/// next run replaces the contents wholesale.
struct StagingGuard {
    path: PathBuf,
}

impl StagingGuard {
    const fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

// ─── GeneratedTestGate ───────────────────────────────────────────────────

/// Rung 3 gate: run tests generated during enrichment.
///
/// On each invocation, the gate:
///   1. Resolves the plan id from the signal.
///   2. Lists `generated-tests/<test_prefix>*` from the [`ArtifactStore`].
///   3. Stages each file into `<worktree>/tests/__roko_generated__/`.
///   4. Shells out to the build system with a pattern-scoped selector.
///   5. Parses test counts + failures and returns a [`Verdict`].
///   6. Best-effort deletes the staging dir (RAII).
pub struct GeneratedTestGate {
    build_system: BuildSystem,
    artifacts: Arc<dyn ArtifactStore>,
    test_prefix: String,
    artifact_prefix: String,
    timeout_ms: u64,
    name: String,
}

impl GeneratedTestGate {
    /// Construct a new generated-test gate using the cargo build system
    /// and the default `"gen_"` file/name prefix.
    #[must_use]
    pub fn new(artifacts: Arc<dyn ArtifactStore>) -> Self {
        Self::for_build_system(BuildSystem::Cargo, artifacts)
    }

    /// Construct a generated-test gate for an explicit build system.
    #[must_use]
    pub fn for_build_system(build_system: BuildSystem, artifacts: Arc<dyn ArtifactStore>) -> Self {
        Self {
            build_system,
            artifacts,
            test_prefix: DEFAULT_TEST_PREFIX.into(),
            artifact_prefix: "generated-tests/".into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            name: format!("generated_test:{}", build_system.program()),
        }
    }

    /// Override the test-name prefix (also used as the file-name prefix
    /// when listing artifacts).
    #[must_use]
    pub fn with_test_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.test_prefix = prefix.into();
        self
    }

    /// Override the artifact subpath prefix (default `"generated-tests/"`).
    #[must_use]
    pub fn with_artifact_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.artifact_prefix = prefix.into();
        self
    }

    /// Override the gate timeout in milliseconds.
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl roko_core::Cell for GeneratedTestGate {
    fn cell_id(&self) -> &str {
        "generated-test-gate"
    }
    fn cell_name(&self) -> &str {
        "GeneratedTestGate"
    }
    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for GeneratedTestGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();

        // 1. Resolve plan id.
        let Some(plan) = resolve_plan_id(signal) else {
            let elapsed = elapsed_ms(started);
            return Verdict::fail(
                &self.name,
                "signal is missing a plan identifier (tag `plan` or `plan_id`)",
            )
            .with_duration(elapsed);
        };

        // 2. Resolve GatePayload for working_dir + env.
        let payload: GatePayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                let elapsed = elapsed_ms(started);
                return Verdict::fail(&self.name, format!("signal body is not a GatePayload: {e}"))
                    .with_duration(elapsed);
            }
        };

        // 3. Fetch generated test artifacts.
        let artifact_names = self.artifacts.list(&plan, &self.artifact_prefix);
        let matching: Vec<String> = artifact_names
            .into_iter()
            .filter(|n| artifact_matches_prefix(n, &self.artifact_prefix, &self.test_prefix))
            .collect();

        if matching.is_empty() {
            let elapsed = elapsed_ms(started);
            return Verdict::pass(&self.name)
                .with_detail("no generated tests for this plan")
                .with_test_count(TestCount::default())
                .with_duration(elapsed);
        }

        // 4. Stage files into <worktree>/tests/<STAGING_SUBDIR>/.
        let staging_dir = payload.working_dir.join("tests").join(STAGING_SUBDIR);
        let _guard = StagingGuard::new(staging_dir.clone());

        if let Err(e) = stage_files(&self.artifacts, &plan, &matching, &staging_dir) {
            let elapsed = elapsed_ms(started);
            return Verdict::fail(&self.name, format!("staging failed: {e}"))
                .with_duration(elapsed);
        }

        // 5. Build and run the test command with a pattern selector.
        let selector = TestSelector::Patterns(vec![self.test_prefix.clone()]);
        let mut cmd = Command::new(self.build_system.program());
        for arg in self.build_system.test_args() {
            cmd.arg(arg);
        }
        for arg in selector.extra_args(self.build_system) {
            cmd.arg(arg);
        }
        cmd.current_dir(&payload.working_dir);
        cmd.kill_on_drop(true);
        if let Some(ref tgt) = payload.target_dir {
            cmd.env("CARGO_TARGET_DIR", tgt);
        }
        for (k, v) in &payload.extra_env {
            cmd.env(k, v);
        }

        let output = match timeout(Duration::from_millis(self.timeout_ms), cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let elapsed = elapsed_ms(started);
                return Verdict::fail(&self.name, format!("spawn failed: {e}"))
                    .with_duration(elapsed);
            }
            Err(_) => {
                let elapsed = elapsed_ms(started);
                return Verdict::fail(
                    &self.name,
                    format!("timed out after {} ms", self.timeout_ms),
                )
                .with_duration(elapsed);
            }
        };

        // 6. Parse results into a Verdict.
        let elapsed = elapsed_ms(started);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}\n{stderr}");
        let counts =
            crate::test_gate::parse_test_counts(&combined, self.build_system).unwrap_or_default();

        let mut verdict = if output.status.success() {
            Verdict::pass(&self.name)
                .with_detail(format!("ran {} generated test file(s)", matching.len()))
                .with_test_count(counts)
                .with_duration(elapsed)
        } else {
            let failing = extract_failing_test_names(&combined, self.build_system, 5);
            let reason = if failing.is_empty() {
                classify_nonzero_exit(&combined)
            } else {
                format!("{} generated test(s) failed", failing.len())
            };
            let digest = build_error_digest(&failing, &combined);
            Verdict::fail(&self.name, reason)
                .with_detail(format!("ran {} generated test file(s)", matching.len()))
                .with_test_count(counts)
                .with_error_digest(digest)
                .with_duration(elapsed)
        };
        // Preserve determinism: drop staging dir *after* we've captured output.
        drop(_guard);
        verdict.duration_ms = elapsed;
        verdict
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn resolve_plan_id(signal: &Engram) -> Option<String> {
    if let Some(v) = signal.tag(PLAN_TAG_KEY) {
        return Some(v.to_string());
    }
    if let Some(v) = signal.tag(PLAN_TAG_KEY_ALT) {
        return Some(v.to_string());
    }
    // Fall back: look inside a JSON body for a "plan" field.
    let val: serde_json::Value = signal.body.as_json().ok()?;
    let plan = val
        .get(PLAN_TAG_KEY)
        .or_else(|| val.get(PLAN_TAG_KEY_ALT))?;
    plan.as_str().map(std::string::ToString::to_string)
}

fn artifact_matches_prefix(name: &str, subpath: &str, test_prefix: &str) -> bool {
    let rest = name.strip_prefix(subpath).unwrap_or(name);
    // Reject paths that escape the staging dir.
    if rest.contains("..") || rest.starts_with('/') {
        return false;
    }
    // Take the trailing file name component and check its prefix.
    let file = rest.rsplit('/').next().unwrap_or(rest);
    file.starts_with(test_prefix)
}

fn stage_files(
    artifacts: &Arc<dyn ArtifactStore>,
    plan: &str,
    names: &[String],
    staging_dir: &Path,
) -> std::io::Result<()> {
    // If a collision already exists with a file (not a dir), bail.
    if staging_dir.exists() && !staging_dir.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "staging path exists and is not a directory: {}",
                staging_dir.display()
            ),
        ));
    }
    // Start from a clean slate to avoid stale files from previous runs.
    if staging_dir.exists() {
        std::fs::remove_dir_all(staging_dir)?;
    }
    std::fs::create_dir_all(staging_dir)?;

    for name in names {
        let Some(bytes) = artifacts.read(plan, name) else {
            continue;
        };
        // Derive the file path relative to the staging dir by stripping
        // the artifact subpath component. We've already sanitized against
        // `..` in `artifact_matches_prefix`.
        let rel: &str = name.rsplit('/').next().unwrap_or(name);
        let dest = staging_dir.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&dest, bytes)?;
    }
    Ok(())
}

fn extract_failing_test_names(output: &str, build: BuildSystem, max: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        let candidate: Option<&str> = match build {
            BuildSystem::Go => t
                .strip_prefix("--- FAIL:")
                .map(str::trim)
                .and_then(|s| s.split_whitespace().next()),
            _ => {
                if t.starts_with("test ") && t.ends_with(" ... FAILED") {
                    let mid = t
                        .trim_start_matches("test ")
                        .trim_end_matches(" ... FAILED");
                    Some(mid.trim())
                } else if t.starts_with("---- ") && t.ends_with(" stdout ----") {
                    let mid = t
                        .trim_start_matches("---- ")
                        .trim_end_matches(" stdout ----");
                    Some(mid.trim())
                } else {
                    None
                }
            }
        };
        if let Some(n) = candidate {
            if !n.is_empty() && !names.iter().any(|existing| existing == n) {
                names.push(n.to_string());
            }
        }
        if names.len() >= max {
            break;
        }
    }
    names
}

fn classify_nonzero_exit(output: &str) -> String {
    if output.contains("error[E") || output.contains("error:") {
        return "test compilation failed".to_string();
    }
    output
        .lines()
        .find(|l| l.contains("FAILED") || l.contains("FAIL"))
        .unwrap_or("generated tests failed")
        .trim()
        .to_string()
}

fn build_error_digest(failing: &[String], combined: &str) -> String {
    if failing.is_empty() {
        // Fall back to the first compile error line, if any.
        return combined
            .lines()
            .find(|l| l.trim_start().starts_with("error"))
            .unwrap_or("generated tests failed without parsable names")
            .trim()
            .to_string();
    }
    failing.join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn signal_with_plan(plan: &str, payload: &GatePayload) -> Engram {
        Engram::builder(Kind::Task)
            .tag("plan", plan)
            .body(Body::from_json(payload).unwrap())
            .build()
    }

    fn store() -> Arc<InMemoryArtifactStore> {
        Arc::new(InMemoryArtifactStore::new())
    }

    #[test]
    fn in_memory_store_lists_and_reads() {
        let s = InMemoryArtifactStore::new()
            .with("p1", "generated-tests/gen_a.rs", b"fn a() {}".to_vec())
            .with("p1", "generated-tests/gen_b.rs", b"fn b() {}".to_vec())
            .with("p1", "other/x.rs", b"fn x() {}".to_vec())
            .with("p2", "generated-tests/gen_c.rs", b"fn c() {}".to_vec());

        let mut listed = s.list("p1", "generated-tests/");
        listed.sort();
        assert_eq!(
            listed,
            vec![
                "generated-tests/gen_a.rs".to_string(),
                "generated-tests/gen_b.rs".into(),
            ]
        );
        assert_eq!(
            s.read("p1", "generated-tests/gen_a.rs").unwrap(),
            b"fn a() {}".to_vec()
        );
        assert!(s.read("p1", "missing.rs").is_none());
        assert_eq!(s.len(), 4);
        assert!(!s.is_empty());
    }

    #[test]
    fn resolve_plan_id_prefers_tag() {
        let payload = GatePayload::in_dir("/nowhere");
        let sig = Engram::builder(Kind::Task)
            .tag("plan", "plan-42")
            .body(Body::from_json(&payload).unwrap())
            .build();
        assert_eq!(resolve_plan_id(&sig).as_deref(), Some("plan-42"));
    }

    #[test]
    fn resolve_plan_id_falls_back_to_body() {
        let sig = Engram::builder(Kind::Task)
            .body(Body::from_json(&serde_json::json!({ "plan": "from-body" })).unwrap())
            .build();
        assert_eq!(resolve_plan_id(&sig).as_deref(), Some("from-body"));
    }

    #[test]
    fn resolve_plan_id_none_when_missing() {
        let sig = Engram::builder(Kind::Task).body(Body::empty()).build();
        assert!(resolve_plan_id(&sig).is_none());
    }

    #[test]
    fn artifact_matches_prefix_happy_and_rejects_escape() {
        assert!(artifact_matches_prefix(
            "generated-tests/gen_x.rs",
            "generated-tests/",
            "gen_"
        ));
        assert!(!artifact_matches_prefix(
            "generated-tests/other.rs",
            "generated-tests/",
            "gen_"
        ));
        assert!(!artifact_matches_prefix(
            "generated-tests/../etc/gen_evil.rs",
            "generated-tests/",
            "gen_"
        ));
        assert!(!artifact_matches_prefix(
            "generated-tests//gen_absolute.rs",
            "generated-tests/",
            "gen_"
        ));
    }

    #[test]
    fn classify_nonzero_exit_detects_compile_failure() {
        let compile = "error[E0599]: no method `foo`";
        assert_eq!(classify_nonzero_exit(compile), "test compilation failed");
        let bare_fail = "somewhere\ntest gen_x ... FAILED\nend";
        assert!(classify_nonzero_exit(bare_fail).contains("FAILED"));
        let empty = "";
        assert_eq!(classify_nonzero_exit(empty), "generated tests failed");
    }

    #[test]
    fn extract_failing_test_names_cargo_format() {
        let out = "running 3 tests\n\
                   test gen_a ... ok\n\
                   test gen_b ... FAILED\n\
                   test gen_c ... FAILED\n\
                   failures:\n---- gen_b stdout ----\nassertion failed\n";
        let names = extract_failing_test_names(out, BuildSystem::Cargo, 5);
        assert!(names.contains(&"gen_b".to_string()));
        assert!(names.contains(&"gen_c".to_string()));
        assert!(names.len() <= 5);
    }

    #[test]
    fn extract_failing_test_names_go_format() {
        let out = "--- FAIL: TestGenA (0.00s)\n    gen_a_test.go:10: boom\n--- FAIL: TestGenB (0.00s)\nFAIL";
        let names = extract_failing_test_names(out, BuildSystem::Go, 5);
        assert_eq!(names, vec!["TestGenA".to_string(), "TestGenB".into()]);
    }

    #[test]
    fn extract_failing_test_names_caps_at_max() {
        use std::fmt::Write as _;
        let mut out = String::new();
        for i in 0..20 {
            writeln!(&mut out, "test gen_{i} ... FAILED").unwrap();
        }
        let names = extract_failing_test_names(&out, BuildSystem::Cargo, 3);
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn build_error_digest_joins_failing_names() {
        let digest = build_error_digest(&["gen_a".into(), "gen_b".into()], "unused");
        assert_eq!(digest, "gen_a, gen_b");
    }

    #[test]
    fn build_error_digest_falls_back_to_error_line() {
        let digest = build_error_digest(&[], "stuff\nerror: bad\nmore");
        assert!(digest.contains("error: bad"));
    }

    #[test]
    fn staging_guard_removes_directory_on_drop() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("tests").join(STAGING_SUBDIR);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("gen_x.rs"), b"// test").unwrap();
        assert!(staging.exists());
        {
            let _g = StagingGuard::new(staging.clone());
        }
        assert!(!staging.exists(), "guard must remove staging dir");
    }

    #[test]
    fn staging_guard_noop_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("tests").join(STAGING_SUBDIR);
        // Never created.
        {
            let _g = StagingGuard::new(staging.clone());
        }
        assert!(!staging.exists());
    }

    #[test]
    fn stage_files_copies_artifact_bodies() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("tests").join(STAGING_SUBDIR);

        let s = InMemoryArtifactStore::new()
            .with("p", "generated-tests/gen_a.rs", b"fn a(){}".to_vec())
            .with("p", "generated-tests/gen_b.rs", b"fn b(){}".to_vec());
        let arc: Arc<dyn ArtifactStore> = Arc::new(s);
        let names = vec![
            "generated-tests/gen_a.rs".to_string(),
            "generated-tests/gen_b.rs".into(),
        ];
        stage_files(&arc, "p", &names, &staging).unwrap();
        assert!(staging.join("gen_a.rs").exists());
        assert!(staging.join("gen_b.rs").exists());
        let content = std::fs::read(staging.join("gen_a.rs")).unwrap();
        assert_eq!(content, b"fn a(){}".to_vec());
    }

    #[test]
    fn stage_files_cleans_prior_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("tests").join(STAGING_SUBDIR);
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("stale.rs"), b"// stale").unwrap();

        let s = InMemoryArtifactStore::new().with(
            "p",
            "generated-tests/gen_x.rs",
            b"fn x(){}".to_vec(),
        );
        let arc: Arc<dyn ArtifactStore> = Arc::new(s);
        let names = vec!["generated-tests/gen_x.rs".to_string()];
        stage_files(&arc, "p", &names, &staging).unwrap();
        assert!(staging.join("gen_x.rs").exists());
        assert!(
            !staging.join("stale.rs").exists(),
            "prior files must be pruned"
        );
    }

    #[test]
    fn stage_files_rejects_file_collision() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = tmp.path().join("tests");
        std::fs::create_dir_all(&parent).unwrap();
        let staging = parent.join(STAGING_SUBDIR);
        // Place a *file* where the staging *directory* should live.
        std::fs::write(&staging, b"not a dir").unwrap();

        let s = InMemoryArtifactStore::new();
        let arc: Arc<dyn ArtifactStore> = Arc::new(s);
        let err = stage_files(&arc, "p", &[], &staging).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    }

    #[tokio::test]
    async fn empty_artifacts_returns_pass_with_zero_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = GatePayload::in_dir(tmp.path());
        let gate = GeneratedTestGate::new(store());
        let sig = signal_with_plan("plan-empty", &payload);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed, "empty artifact list must pass");
        assert_eq!(
            v.detail.as_deref(),
            Some("no generated tests for this plan")
        );
        assert_eq!(v.test_count, Some(TestCount::default()));
        assert_eq!(v.gate, "generated_test:cargo");
    }

    #[tokio::test]
    async fn missing_plan_id_fails_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = GatePayload::in_dir(tmp.path());
        let sig = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload).unwrap())
            .build();
        let gate = GeneratedTestGate::new(store());
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("plan identifier"));
    }

    #[tokio::test]
    async fn malformed_body_fails_cleanly() {
        let sig = Engram::builder(Kind::Task)
            .tag("plan", "p")
            .body(Body::text("not json"))
            .build();
        let gate = GeneratedTestGate::new(store());
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("GatePayload"));
    }

    #[tokio::test]
    async fn non_matching_artifacts_filtered_by_prefix() {
        // Artifacts exist but none start with the configured prefix.
        let tmp = tempfile::tempdir().unwrap();
        let payload = GatePayload::in_dir(tmp.path());

        let s = InMemoryArtifactStore::new()
            .with("plan-x", "generated-tests/other.rs", b"// noise".to_vec())
            .with(
                "plan-x",
                "generated-tests/garbage_y.rs",
                b"// noise".to_vec(),
            );

        let gate = GeneratedTestGate::new(Arc::new(s)).with_test_prefix("gen_");
        let sig = signal_with_plan("plan-x", &payload);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed);
        assert_eq!(
            v.detail.as_deref(),
            Some("no generated tests for this plan")
        );
    }

    #[tokio::test]
    async fn custom_prefix_and_name_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = GatePayload::in_dir(tmp.path());
        let gate = GeneratedTestGate::new(store())
            .with_test_prefix("test_gen_")
            .with_name("rung3");
        let sig = signal_with_plan("plan-c", &payload);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert_eq!(v.gate, "rung3");
        assert!(v.passed);
        assert_eq!(gate.name(), "rung3");
        assert_eq!(gate.test_prefix, "test_gen_");
    }

    #[test]
    fn gate_builder_defaults() {
        let g = GeneratedTestGate::new(store());
        assert_eq!(g.test_prefix, DEFAULT_TEST_PREFIX);
        assert_eq!(g.artifact_prefix, "generated-tests/");
        assert_eq!(g.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(g.name(), "generated_test:cargo");
    }

    #[test]
    fn gate_builder_overrides() {
        let g = GeneratedTestGate::for_build_system(BuildSystem::Go, store())
            .with_test_prefix("gen_go_")
            .with_artifact_prefix("integration-tests/")
            .with_timeout_ms(5_000)
            .with_name("generated_test:custom");
        assert_eq!(g.test_prefix, "gen_go_");
        assert_eq!(g.artifact_prefix, "integration-tests/");
        assert_eq!(g.timeout_ms, 5_000);
        assert_eq!(g.name(), "generated_test:custom");
    }
}
