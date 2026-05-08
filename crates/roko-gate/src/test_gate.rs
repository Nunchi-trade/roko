//! `TestGate` — Rung 1 of the 6-rung verification ladder (§10.5).
//!
//! Runs the project's test suite via [`BuildSystem::test_args`], parses
//! `passed/failed/ignored` counts, and attaches them to the [`Verdict`]
//! via [`Verdict::with_test_count`] so downstream policies can classify
//! "mostly passing" runs (≥90% pass, ≥20 tests, ≥1 failure).
//!
//! Mori reference: `apps/mori/src/orchestrator/gates.rs::test_gate` +
//! `parse_test_counts`.

use crate::compile_errors::{render_failure_classification, structured_gate_failure};
use crate::payload::{BuildSystem, GatePayload, TestSelector};
use async_trait::async_trait;
use roko_core::{Body, CellContext, Context, Kind, Provenance, Signal, TestCount, Verdict, Verify};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Rung 1 gate: run the project's test suite; pass iff every test passed.
///
/// The gate is build-system-aware: it looks up the test command via
/// [`BuildSystem::test_args`] and appends selector-specific args from
/// [`TestSelector::extra_args`].
pub struct TestGate {
    build_system: BuildSystem,
    selector: TestSelector,
    extra_args: Vec<String>,
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

impl TestGate {
    /// Construct a test gate for `build_system` running every test.
    #[must_use]
    pub fn new(build_system: BuildSystem) -> Self {
        Self {
            build_system,
            selector: TestSelector::All,
            extra_args: Vec::new(),
            timeout_ms: default_timeout_ms(),
            name: format!("test:{}", build_system.program()),
        }
    }

    /// Shortcut: a cargo-based test gate running every test.
    #[must_use]
    pub fn cargo() -> Self {
        Self::new(BuildSystem::Cargo)
    }

    /// Run only the quick (lib/unit) tier.
    #[must_use]
    pub fn quick(build_system: BuildSystem) -> Self {
        Self {
            selector: TestSelector::Quick,
            ..Self::new(build_system)
        }
    }

    /// Run only tests matching the given patterns.
    #[must_use]
    pub fn filtered(build_system: BuildSystem, patterns: Vec<String>) -> Self {
        Self {
            selector: TestSelector::Patterns(patterns),
            ..Self::new(build_system)
        }
    }

    /// Override the gate's selector.
    #[must_use]
    pub fn with_selector(mut self, selector: TestSelector) -> Self {
        self.selector = selector;
        self
    }

    /// Append additional arguments (applied after selector-specific args).
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args.extend(args);
        self
    }

    /// Override the timeout in milliseconds (default: 15 minutes).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

#[async_trait]
impl roko_core::Cell for TestGate {
    fn cell_id(&self) -> &str {
        "test-gate"
    }
    fn cell_name(&self) -> &str {
        "TestGate"
    }
    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }

    async fn execute(
        &self,
        input: Vec<Signal>,
        _ctx: &CellContext,
    ) -> roko_core::error::Result<Vec<Signal>> {
        let fallback = Signal::builder(Kind::Task)
            .body(Body::empty())
            .provenance(Provenance::agent(self.name()))
            .build();
        let signal = input.first().unwrap_or(&fallback);
        let verify_ctx = Context::now();
        let verdict = self.verify(signal, &verify_ctx).await;
        let body = Body::from_json(&verdict)?;
        let output = signal
            .derive_verdict(body)
            .provenance(Provenance::agent(self.name()))
            .tag("gate", verdict.gate.clone())
            .tag("passed", verdict.passed.to_string())
            .build();
        Ok(vec![output])
    }
}

#[async_trait]
impl Verify for TestGate {
    async fn verify(&self, signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: GatePayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("signal body is not a GatePayload: {e}"))
                    .with_duration(elapsed);
            }
        };

        if !self.build_system.is_available() {
            let reason = format!(
                "{} not available: '{}' not found on PATH",
                self.build_system.name(),
                self.build_system.program()
            );
            tracing::warn!(gate = %self.name, "{reason}");
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            return Verdict::pass(&self.name)
                .with_detail(format!("skipped: {reason}"))
                .with_duration(elapsed);
        }

        let mut cmd = Command::new(self.build_system.program());
        for arg in self.build_system.scoped_test_args(&payload.target_crates) {
            cmd.arg(arg);
        }
        for arg in self.selector.extra_args(self.build_system) {
            cmd.arg(arg);
        }
        for arg in &self.extra_args {
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
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let reason = format!("spawn failed: {e}");
                let classification =
                    structured_gate_failure(&self.name, &reason, reason.clone(), elapsed);
                return Verdict::fail(&self.name, reason)
                    .with_error_digest(render_failure_classification(&classification))
                    .with_duration(elapsed);
            }
            Err(_) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                let reason = format!("timed out after {} ms", self.timeout_ms);
                let classification =
                    structured_gate_failure(&self.name, &reason, reason.clone(), elapsed);
                return Verdict::fail(&self.name, reason)
                    .with_error_digest(render_failure_classification(&classification))
                    .with_duration(elapsed);
            }
        };

        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}\n{stderr}");
        let counts = parse_test_counts(&combined, self.build_system);

        let mut verdict = if output.status.success() {
            Verdict::pass(&self.name)
                .with_detail(combined)
                .with_duration(elapsed)
        } else {
            let reason = summarize_test_failures(&combined, 3);
            let classification =
                structured_gate_failure(&self.name, &combined, reason.clone(), elapsed);
            Verdict::fail(&self.name, reason)
                .with_detail(combined)
                .with_error_digest(render_failure_classification(&classification))
                .with_duration(elapsed)
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

/// Parse `passed/failed/ignored` counts from test-runner output.
///
/// Returns `None` if no summary line is detected. Dispatches to a
/// format-specific parser based on the build system:
/// - Cargo/nextest: `test result: ok. 12 passed; 0 failed; 1 ignored; …`
/// - Go: `--- PASS: TestX (0.01s)` / `--- FAIL:` / `--- SKIP:`
/// - Npm (Jest/Vitest/Mocha): `Tests: N passed, N failed, N skipped, N total`
/// - Python (pytest): `N passed, N failed, N skipped`
/// - Forge (Foundry): `Test result: ok. N passed; 0 failed;`
#[must_use]
pub fn parse_test_counts(output: &str, build: BuildSystem) -> Option<TestCount> {
    match build {
        BuildSystem::Go => parse_go_test_counts(output),
        BuildSystem::Npm => {
            parse_npm_test_counts(output).or_else(|| parse_cargo_test_counts(output))
        }
        BuildSystem::Python => {
            parse_pytest_counts(output).or_else(|| parse_cargo_test_counts(output))
        }
        _ => parse_cargo_test_counts(output),
    }
}

fn parse_cargo_test_counts(output: &str) -> Option<TestCount> {
    // Aggregate across all `test result:` lines (cargo prints one per target).
    let mut total = TestCount::default();
    let mut saw_summary = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("test result:") {
            continue;
        }
        saw_summary = true;
        let passed = extract_count(trimmed, "passed");
        let failed = extract_count(trimmed, "failed");
        let ignored = extract_count(trimmed, "ignored");
        total.passed += passed;
        total.failed += failed;
        total.ignored += ignored;
    }
    if saw_summary { Some(total) } else { None }
}

fn parse_go_test_counts(output: &str) -> Option<TestCount> {
    // Go's `go test` prints `PASS` / `FAIL` / `ok  path  duration` lines.
    // Count the ---PASS/---FAIL/---SKIP markers for per-test resolution.
    let mut passed = 0;
    let mut failed = 0;
    let mut ignored = 0;
    let mut saw_any = false;
    for line in output.lines() {
        let t = line.trim();
        if t.starts_with("--- PASS:") {
            passed += 1;
            saw_any = true;
        } else if t.starts_with("--- FAIL:") {
            failed += 1;
            saw_any = true;
        } else if t.starts_with("--- SKIP:") {
            ignored += 1;
            saw_any = true;
        }
    }
    if saw_any {
        Some(TestCount::new(passed, failed, ignored))
    } else {
        None
    }
}

/// Parse Jest/Vitest/Mocha summary output.
///
/// Common patterns:
/// - Jest/Vitest: `Tests:       3 passed, 1 failed, 4 total`
/// - Mocha:      `  3 passing (12ms)\n  1 failing`
/// - TAP:        `# pass  3\n# fail  1\n# skip  0`
fn parse_npm_test_counts(output: &str) -> Option<TestCount> {
    let mut total = TestCount::default();
    let mut saw_summary = false;

    for line in output.lines() {
        let t = line.trim();

        // Jest/Vitest format: "Tests:  N passed, N failed, N skipped, N total"
        if t.starts_with("Tests:") || t.starts_with("Test Suites:") {
            if t.starts_with("Tests:") {
                saw_summary = true;
                total.passed += extract_jest_count(t, "passed");
                total.failed += extract_jest_count(t, "failed");
                total.ignored += extract_jest_count(t, "skipped")
                    + extract_jest_count(t, "pending")
                    + extract_jest_count(t, "todo");
            }
        }
        // TAP format: "# pass  N" / "# fail  N" / "# skip  N"
        else if t.starts_with("# pass") || t.starts_with("# fail") || t.starts_with("# skip") {
            if let Some(n) = t
                .split_whitespace()
                .last()
                .and_then(|s| s.parse::<u32>().ok())
            {
                saw_summary = true;
                if t.starts_with("# pass") {
                    total.passed += n;
                } else if t.starts_with("# fail") {
                    total.failed += n;
                } else {
                    total.ignored += n;
                }
            }
        }
        // Mocha format: "  4 passing (12ms)" / "  1 failing" / "  2 pending"
        // Pattern: first token is a number, second is passing/failing/pending.
        else {
            let words: Vec<&str> = t.split_whitespace().collect();
            if words.len() >= 2 {
                if let Ok(n) = words[0].parse::<u32>() {
                    let keyword = words[1].trim_end_matches(|c: char| !c.is_alphabetic());
                    match keyword {
                        "passing" => {
                            saw_summary = true;
                            total.passed += n;
                        }
                        "failing" => {
                            saw_summary = true;
                            total.failed += n;
                        }
                        "pending" => {
                            saw_summary = true;
                            total.ignored += n;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if saw_summary { Some(total) } else { None }
}

/// Extract `N <label>` from Jest/Vitest summary (comma-separated segments).
fn extract_jest_count(line: &str, label: &str) -> u32 {
    for segment in line.split(',') {
        let s = segment.trim();
        if s.ends_with(label) {
            if let Some(num_str) = s.split_whitespace().rev().nth(1) {
                if let Ok(n) = num_str.parse::<u32>() {
                    return n;
                }
            }
        }
    }
    0
}

/// Parse pytest summary output.
///
/// Patterns:
/// - `=== 3 passed, 1 failed, 2 skipped in 0.5s ===`
/// - `=== 5 passed in 1.2s ===`
/// - Short: `3 passed, 1 failed`
fn parse_pytest_counts(output: &str) -> Option<TestCount> {
    let mut total = TestCount::default();
    let mut saw_summary = false;

    for line in output.lines() {
        let t = line.trim();

        // pytest's "=== ... ===" summary line
        if t.starts_with('=') && t.ends_with('=') && t.contains("passed") {
            saw_summary = true;
            total.passed += extract_pytest_label(t, "passed");
            total.failed += extract_pytest_label(t, "failed");
            total.ignored +=
                extract_pytest_label(t, "skipped") + extract_pytest_label(t, "deselected");
        }
        // Also look for summary without === decoration (e.g. short mode)
        else if (t.contains(" passed") || t.contains(" failed")) && t.contains(',') {
            let has_pytest_keywords = t.contains("passed")
                || t.contains("failed")
                || t.contains("skipped")
                || t.contains("error");
            if has_pytest_keywords && !t.starts_with("test result:") {
                saw_summary = true;
                total.passed += extract_pytest_label(t, "passed");
                total.failed += extract_pytest_label(t, "failed");
                total.ignored += extract_pytest_label(t, "skipped");
            }
        }
    }
    if saw_summary { Some(total) } else { None }
}

/// Extract `N label` from pytest summary (space-separated tokens).
fn extract_pytest_label(line: &str, label: &str) -> u32 {
    let words: Vec<&str> = line.split_whitespace().collect();
    for (i, &word) in words.iter().enumerate() {
        if (word == label || word.trim_end_matches(',') == label) && i > 0 {
            if let Ok(n) = words[i - 1].parse::<u32>() {
                return n;
            }
        }
    }
    0
}

fn extract_count(line: &str, label: &str) -> u32 {
    // Look for `<digits> <label>` or `<digits>; <label>`.
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

/// Summarize up to `max` failing test names/lines for a concise reason.
fn summarize_test_failures(output: &str, max: usize) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        let is_failure = (t.starts_with("---- ") && t.ends_with(" stdout ----"))
            || (t.starts_with("test ") && t.ends_with(" ... FAILED"))
            || t.starts_with("FAIL\t")
            || t.starts_with("--- FAIL:");
        if is_failure {
            lines.push(t);
        }
        if lines.len() >= max {
            break;
        }
    }
    if !lines.is_empty() {
        return lines.join("; ");
    }
    output
        .lines()
        .find(|l| l.contains("FAILED") || l.contains("FAIL"))
        .unwrap_or("tests failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cargo_counts_single_summary() {
        let out = "running 3 tests\ntest result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out";
        let counts = parse_cargo_test_counts(out).expect("summary present");
        assert_eq!(counts, TestCount::new(3, 0, 0));
    }

    #[test]
    fn parse_cargo_counts_aggregates_multiple_summaries() {
        let out = "test result: ok. 10 passed; 0 failed; 2 ignored\n\
                   test result: FAILED. 5 passed; 3 failed; 0 ignored";
        let counts = parse_cargo_test_counts(out).expect("summary present");
        assert_eq!(counts, TestCount::new(15, 3, 2));
    }

    #[test]
    fn parse_cargo_counts_none_when_no_summary() {
        let out = "running 5 tests\ntest foo ... ok\n";
        assert!(parse_cargo_test_counts(out).is_none());
    }

    #[test]
    fn parse_go_counts_per_marker() {
        let out = "--- PASS: TestAdd (0.00s)\n\
                   --- FAIL: TestSub (0.01s)\n\
                   --- SKIP: TestMul (0.00s)\n\
                   FAIL";
        let counts = parse_go_test_counts(out).expect("markers present");
        assert_eq!(counts, TestCount::new(1, 1, 1));
    }

    #[test]
    fn parse_dispatches_by_build_system() {
        let cargo_out = "test result: ok. 1 passed; 0 failed; 0 ignored";
        assert!(parse_test_counts(cargo_out, BuildSystem::Cargo).is_some());
        let go_out = "--- PASS: TestX (0.00s)";
        assert!(parse_test_counts(go_out, BuildSystem::Go).is_some());
    }

    #[test]
    fn extract_count_reads_labeled_numbers() {
        assert_eq!(
            extract_count("test result: ok. 5 passed; 2 failed", "passed"),
            5
        );
        assert_eq!(
            extract_count("test result: ok. 5 passed; 2 failed", "failed"),
            2
        );
        assert_eq!(
            extract_count("test result: ok. 0 passed; 0 failed", "passed"),
            0
        );
    }

    #[test]
    fn summarize_failures_joins_failed_tests() {
        let out = "test foo::bar ... FAILED\ntest foo::baz ... FAILED\nok";
        let s = summarize_test_failures(out, 2);
        assert!(s.contains("foo::bar"));
        assert!(s.contains("foo::baz"));
    }

    #[test]
    fn summarize_falls_back_when_no_failed_markers() {
        let out = "generic FAIL message here";
        let s = summarize_test_failures(out, 2);
        assert!(s.contains("FAIL"));
    }

    #[test]
    fn test_gate_builder() {
        let g = TestGate::cargo()
            .with_selector(TestSelector::Quick)
            .with_extra_args(vec!["--nocapture".into()])
            .with_timeout_ms(60_000);
        assert_eq!(g.name(), "test:cargo");
        assert_eq!(g.timeout_ms, 60_000);
    }

    #[test]
    fn test_gate_quick_constructor_sets_quick_selector() {
        let g = TestGate::quick(BuildSystem::Cargo);
        assert!(matches!(g.selector, TestSelector::Quick));
    }

    #[test]
    fn test_gate_filtered_constructor_sets_patterns() {
        let g = TestGate::filtered(BuildSystem::Cargo, vec!["auth".into(), "login".into()]);
        match &g.selector {
            TestSelector::Patterns(p) => assert_eq!(p, &vec!["auth".to_string(), "login".into()]),
            _ => panic!("expected Patterns selector"),
        }
    }

    #[test]
    fn test_selector_quick_cargo_adds_lib_flag() {
        let args = TestSelector::Quick.extra_args(BuildSystem::Cargo);
        assert_eq!(args, vec!["--lib".to_string()]);
    }

    #[test]
    fn test_selector_patterns_go_joins_with_pipe() {
        let sel = TestSelector::Patterns(vec!["TestFoo".into(), "TestBar".into()]);
        let args = sel.extra_args(BuildSystem::Go);
        assert_eq!(
            args,
            vec!["-run".to_string(), "TestFoo|TestBar".to_string()]
        );
    }

    // ── npm / Jest / Vitest parser tests ──────────────────────────────────

    #[test]
    fn parse_jest_summary() {
        let out = "Test Suites: 2 passed, 2 total\nTests:       5 passed, 1 failed, 2 skipped, 8 total\nTime:        3.2s";
        let counts = parse_test_counts(out, BuildSystem::Npm).expect("jest summary present");
        assert_eq!(counts, TestCount::new(5, 1, 2));
    }

    #[test]
    fn parse_vitest_summary() {
        let out = " ✓ tests/math.test.ts (3)\n ✗ tests/api.test.ts (1)\nTests:  3 passed, 1 failed, 4 total";
        let counts = parse_test_counts(out, BuildSystem::Npm).expect("vitest summary");
        assert_eq!(counts, TestCount::new(3, 1, 0));
    }

    #[test]
    fn parse_mocha_summary() {
        let out = "  4 passing (12ms)\n  1 failing\n  2 pending";
        let counts = parse_test_counts(out, BuildSystem::Npm).expect("mocha summary");
        assert_eq!(counts, TestCount::new(4, 1, 2));
    }

    #[test]
    fn parse_tap_summary() {
        let out = "# tests 10\n# pass  7\n# fail  2\n# skip  1";
        let counts = parse_test_counts(out, BuildSystem::Npm).expect("TAP summary");
        assert_eq!(counts, TestCount::new(7, 2, 1));
    }

    #[test]
    fn npm_falls_back_to_cargo_format() {
        // Some npm test runners (e.g. cargo-like) produce "test result:" lines
        let out = "test result: ok. 3 passed; 0 failed; 0 ignored";
        let counts = parse_test_counts(out, BuildSystem::Npm).expect("fallback");
        assert_eq!(counts, TestCount::new(3, 0, 0));
    }

    // ── pytest parser tests ───────────────────────────────────────────────

    #[test]
    fn parse_pytest_summary_decorated() {
        let out = "collected 10 items\n\ntest_math.py ..F..\ntest_api.py .s\n\n======== 7 passed, 1 failed, 2 skipped in 1.23s ========";
        let counts = parse_test_counts(out, BuildSystem::Python).expect("pytest summary");
        assert_eq!(counts, TestCount::new(7, 1, 2));
    }

    #[test]
    fn parse_pytest_all_passed() {
        let out = "============================= 12 passed in 0.5s =============================";
        let counts = parse_test_counts(out, BuildSystem::Python).expect("pytest all passed");
        assert_eq!(counts, TestCount::new(12, 0, 0));
    }

    #[test]
    fn parse_pytest_with_deselected() {
        let out = "======== 5 passed, 3 deselected in 0.8s ========";
        let counts = parse_test_counts(out, BuildSystem::Python).expect("pytest deselected");
        assert_eq!(counts.passed, 5);
        assert_eq!(counts.ignored, 3);
    }

    #[test]
    fn python_falls_back_to_cargo_format() {
        let out = "test result: ok. 2 passed; 1 failed; 0 ignored";
        let counts = parse_test_counts(out, BuildSystem::Python).expect("fallback");
        assert_eq!(counts, TestCount::new(2, 1, 0));
    }

    // ── Forge still uses Cargo format ────────────────────────────────────

    #[test]
    fn parse_forge_uses_cargo_format() {
        let out = "Running 5 tests for src/Counter.sol:CounterTest\n[PASS] testIncrement()\ntest result: ok. 5 passed; 0 failed; 0 ignored";
        let counts = parse_test_counts(out, BuildSystem::Forge).expect("forge format");
        assert_eq!(counts, TestCount::new(5, 0, 0));
    }
}
