//! `TestGate` — Rung 1 of the 6-rung verification ladder (§10.5).
//!
//! Runs the project's test suite via [`BuildSystem::test_args`], parses
//! `passed/failed/ignored` counts, and attaches them to the [`Verdict`]
//! via [`Verdict::with_test_count`] so downstream policies can classify
//! "mostly passing" runs (≥90% pass, ≥20 tests, ≥1 failure).
//!
//! Mori reference: `apps/mori/src/orchestrator/gates.rs::test_gate` +
//! `parse_test_counts`.

use crate::compile_errors::{classify_gate_failure, render_failure_classification};
use crate::payload::{BuildSystem, GatePayload, TestSelector};
use async_trait::async_trait;
use roko_core::{Context, Engram, Verify, TestCount, Verdict};
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

impl TestGate {
    /// Construct a test gate for `build_system` running every test.
    #[must_use]
    pub fn new(build_system: BuildSystem) -> Self {
        Self {
            build_system,
            selector: TestSelector::All,
            extra_args: Vec::new(),
            timeout_ms: 15 * 60 * 1000, // 15 minutes, matching Mori
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

impl roko_core::Cell for TestGate {
    fn cell_id(&self) -> &str { "test-gate" }
    fn cell_name(&self) -> &str { "TestGate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }
}

#[async_trait]
impl Verify for TestGate {
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

        let mut cmd = Command::new(self.build_system.program());
        for arg in self.build_system.test_args() {
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
        let counts = parse_test_counts(&combined, self.build_system);

        let mut verdict = if output.status.success() {
            Verdict::pass(&self.name)
                .with_detail(combined)
                .with_duration(elapsed)
        } else {
            let reason = summarize_test_failures(&combined, 3);
            let classification = classify_gate_failure(&self.name, &combined);
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
/// Returns `None` if no summary line is detected. Cargo/nextest format:
/// `test result: ok. 12 passed; 0 failed; 1 ignored; …`
#[must_use]
pub fn parse_test_counts(output: &str, build: BuildSystem) -> Option<TestCount> {
    match build {
        // Go's output markers are distinct; everyone else falls through
        // to the cargo-style `test result:` summary parser, which most
        // xUnit-style runners approximate.
        BuildSystem::Go => parse_go_test_counts(output),
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
}
