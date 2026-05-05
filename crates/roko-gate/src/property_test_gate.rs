//! `PropertyTestGate` — Rung 4 of the 6-rung verification ladder (§10.10).
//!
//! Runs property/invariant tests (proptest, hypothesis, fast-check) against
//! the implementer's worktree. Property tests assert invariants over random
//! inputs; they catch boundary bugs, overflow, and timing races that point
//! tests miss ("for all N and w, `RateLimiter(N, w)` never allows more than
//! N per window").
//!
//! Unlike the Rung-3 `GeneratedTestGate`, property tests are keyed by a
//! name **prefix** (default `"prop_"`) rather than materialized from an
//! artifact store — the gate simply passes a pattern selector to the
//! configured [`BuildSystem`] and layers proptest-specific environment
//! variables onto the subprocess. Persistence of regression files is
//! **disabled** by default to keep runs hermetic.
//!
//! Parity: `tmp/mori-agents/20-verification-first-architecture.md` §Rung 4.

use crate::payload::{BuildSystem, GatePayload, TestSelector};
use async_trait::async_trait;
use roko_core::{Context, Engram, TestCount, Verdict, Verify};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Maximum size, in bytes, of the extracted counterexample digest.
///
/// Property-test counterexamples can be large (generator output uses
/// `Debug`). The event bus is bounded, so we truncate to avoid OOM while
/// keeping the "minimal input" line intact.
pub const COUNTEREXAMPLE_DIGEST_LIMIT: usize = 2048;

/// Default per-test case count (matches proptest's built-in default).
///
/// Sourced from [`roko_core::defaults::DEFAULT_PROPTEST_CASES`].
pub const DEFAULT_PROPTEST_CASES: u32 = roko_core::defaults::DEFAULT_PROPTEST_CASES;

/// Default shrink iteration ceiling (matches proptest's built-in default).
///
/// Sourced from [`roko_core::defaults::DEFAULT_MAX_SHRINK_ITERS`].
pub const DEFAULT_MAX_SHRINK_ITERS: u32 = roko_core::defaults::DEFAULT_MAX_SHRINK_ITERS;

/// Rung 4 gate: run property/invariant tests and capture counterexamples.
///
/// The gate spawns the configured [`BuildSystem`]'s test runner with a
/// name-prefix selector (default `"prop_"`), layers in proptest
/// environment variables (`PROPTEST_CASES`, persistence off,
/// `PROPTEST_MAX_SHRINK_ITERS`, and optionally `PROPTEST_RNG_SEED`), and
/// on failure extracts the minimal shrunken input into
/// [`Verdict::error_digest`].
pub struct PropertyTestGate {
    build_system: BuildSystem,
    /// Test-name prefix (e.g. `prop_`, `invariant_`, `test_prop_`).
    prefix: String,
    /// Per-test case count, threaded through as `PROPTEST_CASES`.
    cases: u32,
    /// Optional fixed RNG seed for reproducibility.
    seed: Option<u64>,
    /// Maximum shrink iterations.
    max_shrink_iters: u32,
    /// Persist failure regression files between runs?
    persist_failures: bool,
    timeout_ms: u64,
    name: String,
}

fn timeout_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

fn default_timeout_ms() -> u64 {
    timeout_ms(roko_core::config::TimeoutConfig::default().gate_test())
}

impl PropertyTestGate {
    /// Construct a property-test gate for `build_system` matching tests
    /// whose name begins with `"prop_"`, running 256 cases per property
    /// with persistence disabled.
    ///
    /// # Panics (debug)
    ///
    /// Debug-asserts `cases > 0`; a property with zero cases is
    /// meaningless and most likely a wiring bug.
    #[must_use]
    pub fn new(build_system: BuildSystem) -> Self {
        Self {
            build_system,
            prefix: "prop_".into(),
            cases: DEFAULT_PROPTEST_CASES,
            seed: None,
            max_shrink_iters: DEFAULT_MAX_SHRINK_ITERS,
            persist_failures: false,
            timeout_ms: default_timeout_ms(),
            name: format!("property_test:{}", build_system.program()),
        }
    }

    /// Shortcut: cargo-based property gate.
    #[must_use]
    pub fn cargo() -> Self {
        Self::new(BuildSystem::Cargo)
    }

    /// Override the test-name prefix (e.g. `"invariant_"`).
    #[must_use]
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// Override the per-property case count. `n` must be non-zero.
    ///
    /// # Panics (debug)
    ///
    /// Debug-asserts `n > 0`.
    #[must_use]
    pub fn with_cases(mut self, n: u32) -> Self {
        debug_assert!(n > 0, "PropertyTestGate: cases=0 is meaningless");
        self.cases = n;
        self
    }

    /// Fix the RNG seed for deterministic reproduction of failures.
    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Override the shrink-iteration ceiling.
    #[must_use]
    pub const fn with_max_shrink_iters(mut self, n: u32) -> Self {
        self.max_shrink_iters = n;
        self
    }

    /// Allow proptest to persist regression files between runs (off by
    /// default — keep runs hermetic unless the caller knows better).
    #[must_use]
    pub const fn with_persisted_failures(mut self, persist: bool) -> Self {
        self.persist_failures = persist;
        self
    }

    /// Override the wall-clock timeout in milliseconds (default: 15 min).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Build the set of environment variables the gate layers on top of
    /// [`GatePayload::extra_env`] when spawning the test runner.
    #[must_use]
    fn proptest_env(&self) -> Vec<(String, String)> {
        let mut vars = vec![
            ("PROPTEST_CASES".into(), self.cases.to_string()),
            (
                "PROPTEST_MAX_SHRINK_ITERS".into(),
                self.max_shrink_iters.to_string(),
            ),
        ];
        if !self.persist_failures {
            vars.push(("PROPTEST_DISABLE_FAILURE_PERSISTENCE".into(), "1".into()));
        }
        if let Some(seed) = self.seed {
            vars.push(("PROPTEST_RNG_SEED".into(), seed.to_string()));
        }
        vars
    }
}

impl roko_core::Cell for PropertyTestGate {
    fn cell_id(&self) -> &str {
        "property-test-gate"
    }
    fn cell_name(&self) -> &str {
        "PropertyTestGate"
    }
    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for PropertyTestGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: GatePayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("signal body is not a GatePayload: {e}"))
                    .with_duration(elapsed);
            }
        };

        // Empty prefix → matches "all tests". We special-case it as a
        // pass-through "no property tests declared" verdict, matching the
        // spec's "empty = pass" contract.
        if self.prefix.trim().is_empty() {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            return Verdict::pass(&self.name)
                .with_detail("no property tests declared (empty prefix)")
                .with_duration(elapsed);
        }

        let selector = TestSelector::Patterns(vec![self.prefix.clone()]);

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
        // Payload env first, proptest env second so gate config wins on
        // collision (deterministic gate behavior, even if the caller sets
        // a stray PROPTEST_CASES in extra_env).
        for (k, v) in &payload.extra_env {
            cmd.env(k, v);
        }
        for (k, v) in self.proptest_env() {
            cmd.env(k, v);
        }

        let output = match timeout(Duration::from_millis(self.timeout_ms), cmd.output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("spawn failed: {e}"))
                    .with_duration(elapsed);
            }
            Err(_) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(
                    &self.name,
                    format!("timed out after {} ms", self.timeout_ms),
                )
                .with_duration(elapsed);
            }
        };

        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}\n{stderr}");
        let counts = parse_property_test_counts(&combined);

        let mut verdict = if output.status.success() {
            Verdict::pass(&self.name)
                .with_detail(combined)
                .with_duration(elapsed)
        } else {
            let digest = extract_counterexample(&combined);
            let reason = digest
                .as_deref()
                .and_then(|d| d.lines().find(|l| !l.trim().is_empty()))
                .map_or_else(|| "property failed".to_string(), str::to_string);
            let mut v = Verdict::fail(&self.name, reason)
                .with_detail(combined)
                .with_duration(elapsed);
            if let Some(d) = digest {
                v = v.with_error_digest(d);
            }
            v
        };
        if let Some(tc) = counts {
            verdict = verdict.with_test_count(tc);
        }
        verdict
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Parse a cargo-style `test result:` summary from property-test output.
///
/// Identical format to [`crate::parse_test_counts`] on Cargo; duplicated
/// here so `PropertyTestGate` can stand alone without depending on
/// Rung-1 parsing internals (they may diverge — property runners can
/// print extra shrink-progress noise between summary lines).
#[must_use]
pub fn parse_property_test_counts(output: &str) -> Option<TestCount> {
    let mut total = TestCount::default();
    let mut saw_summary = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("test result:") {
            continue;
        }
        saw_summary = true;
        total.passed += extract_count(trimmed, "passed");
        total.failed += extract_count(trimmed, "failed");
        total.ignored += extract_count(trimmed, "ignored");
    }
    if saw_summary { Some(total) } else { None }
}

fn extract_count(line: &str, label: &str) -> u32 {
    for part in line.split(';') {
        let p = part.trim();
        if let Some(rest) = p.strip_suffix(label).map(str::trim_end) {
            if let Some(num_str) = rest.split_whitespace().last() {
                if let Ok(n) = num_str.parse::<u32>() {
                    return n;
                }
            }
        }
    }
    0
}

/// Extract a shrunken counterexample from property-test output.
///
/// Handles two styles:
/// * proptest: `minimal failing input:` (also tolerates the alternate
///   `Minimal failing input:` casing) + surrounding context.
/// * hypothesis: `Falsifying example:` + subsequent indented lines.
///
/// The returned digest is truncated to at most
/// [`COUNTEREXAMPLE_DIGEST_LIMIT`] bytes (preserving the marker line).
/// Returns `None` if no recognizable counterexample marker is present.
#[must_use]
pub fn extract_counterexample(output: &str) -> Option<String> {
    let lines: Vec<&str> = output.lines().collect();
    let marker_idx = lines.iter().position(|l| is_counterexample_marker(l))?;

    // Start a few lines back for context (property name, assertion).
    let start = marker_idx.saturating_sub(6);
    // Capture the marker line plus subsequent indented / continuation
    // lines up to a blank gap — typical proptest shrinking blocks are
    // 4-20 lines. We cap at 32 to guard against runaway Debug output.
    let mut end = marker_idx + 1;
    let cap = lines.len().min(marker_idx + 32);
    while end < cap {
        let l = lines[end];
        let trimmed = l.trim();
        if trimmed.is_empty() {
            break;
        }
        // Stop at the next independent section header.
        if l.starts_with("thread '") || trimmed.starts_with("test result:") {
            break;
        }
        end += 1;
    }

    let selected = &lines[start..end];
    let body = selected.join("\n");
    Some(truncate_digest(&body, marker_idx - start))
}

fn is_counterexample_marker(line: &str) -> bool {
    let t = line.trim_start();
    // proptest
    t.starts_with("minimal failing input:")
        || t.starts_with("Minimal failing input:")
        // hypothesis
        || t.starts_with("Falsifying example:")
        // proptest shrink-exhausted fallback
        || t.starts_with("unable to shrink further")
}

/// Truncate `s` to [`COUNTEREXAMPLE_DIGEST_LIMIT`] bytes while keeping
/// the marker line (identified by its line offset into `s`) intact.
fn truncate_digest(s: &str, marker_line_offset: usize) -> String {
    if s.len() <= COUNTEREXAMPLE_DIGEST_LIMIT {
        return s.to_string();
    }
    // Walk forward one line at a time from the start, stopping before we
    // exceed the budget. If the marker line would be cut off, drop
    // leading context to preserve it.
    let lines: Vec<&str> = s.lines().collect();
    let mut out_lines: Vec<&str> = Vec::with_capacity(lines.len());
    let budget = COUNTEREXAMPLE_DIGEST_LIMIT.saturating_sub(16); // slack for trailing note
    let marker = lines.get(marker_line_offset).copied().unwrap_or("");

    // First, guarantee the marker line fits.
    let marker_cost = marker.len() + 1;
    if marker_cost >= budget {
        // Pathological: marker alone is too long; hard-truncate it.
        return char_boundary_truncate(marker, budget);
    }
    // Greedy forward fill, but ensure we always include the marker. We
    // do this by tracking the budget minus marker_cost as our "pre" slot.
    let pre_budget = budget.saturating_sub(marker_cost);
    let mut pre_used: usize = 0;
    for (i, line) in lines.iter().enumerate().take(marker_line_offset) {
        let cost = line.len() + 1;
        if pre_used + cost > pre_budget {
            // Drop the oldest context we've buffered.
            while pre_used + cost > pre_budget && !out_lines.is_empty() {
                let dropped = out_lines.remove(0);
                pre_used = pre_used.saturating_sub(dropped.len() + 1);
            }
            if pre_used + cost > pre_budget {
                // Even with nothing buffered we can't fit this line; skip.
                let _ = i;
                continue;
            }
        }
        out_lines.push(line);
        pre_used += cost;
    }
    out_lines.push(marker);
    let mut used = pre_used + marker_cost;

    // Then append trailing lines until we run out of budget.
    for line in lines.iter().skip(marker_line_offset + 1) {
        let cost = line.len() + 1;
        if used + cost > budget {
            break;
        }
        out_lines.push(line);
        used += cost;
    }

    let mut body = out_lines.join("\n");
    body.push_str("\n[truncated]");
    body
}

/// Truncate `s` to at most `limit` bytes, respecting UTF-8 char
/// boundaries.
fn char_boundary_truncate(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push_str("[truncated]");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};

    fn empty_signal() -> Engram {
        Engram::builder(Kind::Task).body(Body::empty()).build()
    }

    fn payload_signal(payload: &GatePayload) -> Engram {
        Engram::builder(Kind::Task)
            .body(Body::from_json(payload).expect("json body"))
            .build()
    }

    // ── construction & builders ──────────────────────────────────────

    #[test]
    fn cargo_shortcut_sets_name_and_defaults() {
        let g = PropertyTestGate::cargo();
        assert_eq!(g.name(), "property_test:cargo");
        assert_eq!(g.prefix, "prop_");
        assert_eq!(g.cases, DEFAULT_PROPTEST_CASES);
        assert!(g.seed.is_none());
        assert!(!g.persist_failures);
    }

    #[test]
    fn builder_chain_threads_all_overrides() {
        let g = PropertyTestGate::new(BuildSystem::Cargo)
            .with_prefix("invariant_")
            .with_cases(50)
            .with_seed(0xDEAD_BEEF)
            .with_max_shrink_iters(64)
            .with_persisted_failures(true)
            .with_timeout_ms(30_000);
        assert_eq!(g.prefix, "invariant_");
        assert_eq!(g.cases, 50);
        assert_eq!(g.seed, Some(0xDEAD_BEEF));
        assert_eq!(g.max_shrink_iters, 64);
        assert!(g.persist_failures);
        assert_eq!(g.timeout_ms, 30_000);
    }

    #[test]
    #[should_panic(expected = "cases=0 is meaningless")]
    fn with_cases_zero_is_debug_rejected() {
        let _ = PropertyTestGate::cargo().with_cases(0);
    }

    // ── env-var assembly ─────────────────────────────────────────────

    #[test]
    fn proptest_env_contains_cases_and_disables_persistence() {
        let g = PropertyTestGate::cargo().with_cases(128);
        let env = g.proptest_env();
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("PROPTEST_CASES").map(String::as_str), Some("128"));
        assert_eq!(
            map.get("PROPTEST_DISABLE_FAILURE_PERSISTENCE")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            map.get("PROPTEST_MAX_SHRINK_ITERS").map(String::as_str),
            Some(DEFAULT_MAX_SHRINK_ITERS.to_string().as_str())
        );
        assert!(!map.contains_key("PROPTEST_RNG_SEED"));
    }

    #[test]
    fn proptest_env_includes_seed_when_set() {
        let g = PropertyTestGate::cargo().with_seed(42);
        let env = g.proptest_env();
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert_eq!(map.get("PROPTEST_RNG_SEED").map(String::as_str), Some("42"));
    }

    #[test]
    fn proptest_env_omits_disable_when_persistence_enabled() {
        let g = PropertyTestGate::cargo().with_persisted_failures(true);
        let env = g.proptest_env();
        let map: std::collections::HashMap<_, _> = env.into_iter().collect();
        assert!(!map.contains_key("PROPTEST_DISABLE_FAILURE_PERSISTENCE"));
    }

    // ── counterexample extraction ────────────────────────────────────

    #[test]
    fn extracts_proptest_minimal_input() {
        let out = "\
running 1 test
test prop_never_exceeds ... FAILED

failures:

---- prop_never_exceeds stdout ----
thread 'prop_never_exceeds' panicked at src/test.rs:42:9:
Test failed: assertion `requests_seen <= limit` failed
  requests_seen: 101
  limit: 100
minimal failing input: ConfigAndRequests { config: (100, 1000), requests: [1, 2, 3] }
  successes: 0
  local rejects: 0
  global rejects: 0

test result: FAILED. 0 passed; 1 failed; 0 ignored";
        let d = extract_counterexample(out).expect("digest present");
        assert!(d.contains("minimal failing input:"));
        assert!(d.contains("ConfigAndRequests"));
        assert!(d.contains("requests_seen: 101"));
    }

    #[test]
    fn extracts_hypothesis_falsifying_example() {
        let out = "\
FAILED test_never_exceeds

Falsifying example: test_never_exceeds(
    config=(100, 1000),
    requests=[1, 2, 3],
)

1 failed in 0.5s";
        let d = extract_counterexample(out).expect("digest present");
        assert!(d.contains("Falsifying example"));
        assert!(d.contains("config=(100, 1000)"));
    }

    #[test]
    fn returns_none_when_no_counterexample_marker() {
        let out = "test result: FAILED. 0 passed; 1 failed; 0 ignored";
        assert!(extract_counterexample(out).is_none());
    }

    #[test]
    fn handles_shrink_exhausted_marker() {
        let out = "Test failed: panic\nunable to shrink further after 2048 iterations\n";
        let d = extract_counterexample(out).expect("digest present");
        assert!(d.contains("unable to shrink further"));
    }

    // ── truncation ───────────────────────────────────────────────────

    #[test]
    fn huge_counterexample_truncated_to_two_kib() {
        let huge_input: String = "x".repeat(10_000);
        let out = format!(
            "Test failed: assertion\nminimal failing input: Payload {{ data: \"{huge_input}\" }}\n"
        );
        let d = extract_counterexample(&out).expect("digest present");
        assert!(d.len() <= COUNTEREXAMPLE_DIGEST_LIMIT);
        // Marker line must be preserved (possibly in truncated form).
        assert!(d.contains("minimal failing input:"));
    }

    #[test]
    fn truncation_preserves_marker_when_context_is_large() {
        // Marker plus many long trailing lines — the extractor captures
        // up to 32 following lines, which we make long enough to blow
        // past the 2 KiB budget. The marker line itself must survive.
        let mut lines: Vec<String> = vec![
            "ctx-0 long preceding context line padded out".into(),
            "ctx-1 long preceding context line padded out".into(),
            "minimal failing input: Payload { value: 42 }".into(),
        ];
        for i in 0..30 {
            lines.push(format!(
                "trailing-line-{i}-with-quite-a-lot-of-padding-so-that-the-total-easily-exceeds-two-kib-aaaaaaaaaaaaaaaaaaaa"
            ));
        }
        let out = lines.join("\n");
        let d = extract_counterexample(&out).expect("digest present");
        assert!(d.len() <= COUNTEREXAMPLE_DIGEST_LIMIT);
        assert!(d.contains("minimal failing input: Payload { value: 42 }"));
        assert!(d.contains("[truncated]"));
    }

    #[test]
    fn small_counterexample_not_modified() {
        let out = "header\nminimal failing input: x\ntail";
        let d = extract_counterexample(out).expect("digest present");
        assert!(!d.contains("[truncated]"));
        assert!(d.contains("minimal failing input: x"));
    }

    // ── count parsing ────────────────────────────────────────────────

    #[test]
    fn parses_cargo_test_result_summary() {
        let out = "running 2 tests\n\
                   test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        let tc = parse_property_test_counts(out).expect("summary present");
        assert_eq!(tc, TestCount::new(2, 0, 0));
    }

    #[test]
    fn parses_aggregated_summaries() {
        let out = "test result: ok. 3 passed; 0 failed; 1 ignored\n\
                   test result: FAILED. 1 passed; 2 failed; 0 ignored";
        let tc = parse_property_test_counts(out).expect("summary present");
        assert_eq!(tc, TestCount::new(4, 2, 1));
    }

    #[test]
    fn returns_none_without_summary() {
        assert!(parse_property_test_counts("no summary here").is_none());
    }

    // ── verify() integration via subprocess ─────────────────────────

    #[tokio::test]
    async fn bad_body_yields_failure_verdict() {
        let gate = PropertyTestGate::cargo();
        // Empty body is not a GatePayload JSON → fail.
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("GatePayload"));
    }

    #[tokio::test]
    async fn empty_prefix_passes_without_running() {
        let gate = PropertyTestGate::cargo().with_prefix("   ");
        let dir = tempfile::tempdir().expect("tempdir");
        let payload = GatePayload::in_dir(dir.path());
        let v = gate
            .verify(&payload_signal(&payload), &Context::at(0))
            .await;
        assert!(v.passed);
        assert_eq!(v.gate, "property_test:cargo");
        assert!(
            v.detail
                .as_deref()
                .unwrap_or("")
                .contains("no property tests")
        );
    }

    #[tokio::test]
    async fn timeout_produces_timeout_verdict() {
        // Use `sleep` as a stand-in "test runner" via a BuildSystem that
        // dispatches to it. Since BuildSystem is an enum with fixed
        // programs, we exercise the timeout path through an impossible
        // working dir on a real runner — spawning `cargo test` in a
        // non-existent dir fails fast, which doesn't exercise timeout.
        //
        // Instead, stub: override timeout to 1ms and point at a real
        // cargo invocation that would take longer. We only need the
        // timeout branch to be reachable; on fast machines cargo takes
        // >>1ms to even open the manifest.
        let dir = tempfile::tempdir().expect("tempdir");
        // Put a minimal Cargo.toml so cargo doesn't bail before spawn.
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.0.1\"\nedition=\"2021\"\n",
        )
        .expect("write manifest");
        std::fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
        std::fs::write(dir.path().join("src/lib.rs"), "").expect("write lib");

        let gate = PropertyTestGate::cargo().with_timeout_ms(1);
        let payload = GatePayload::in_dir(dir.path());
        let v = gate
            .verify(&payload_signal(&payload), &Context::at(0))
            .await;
        assert!(!v.passed);
        // Either the timeout or a spawn failure is acceptable — both
        // are valid failure modes exercised by this path.
        assert!(
            v.reason.contains("timed out") || v.reason.contains("spawn failed"),
            "unexpected reason: {}",
            v.reason
        );
    }
}
