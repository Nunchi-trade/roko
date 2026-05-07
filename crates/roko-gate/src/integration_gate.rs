//! `IntegrationGate` — Rung 5 of the 6-rung verification ladder (§10.11).
//!
//! Runs the highest-fidelity check: an end-to-end scenario that
//! exercises the real runtime. For golem-shaped projects this is a
//! golem lifecycle (spawn golem, fire heartbeat, call subsystem,
//! observe outcome); for everything else it is a shell script or a
//! caller-supplied closure. Because the scenario is expensive
//! (~120 s) this rung runs last and only when Rungs 0-4 are green.
//!
//! Mori reference: `apps/mori/src/orchestrator/gates.rs:1404-1438`
//! (`golem_lifecycle_gate`) and `:1200-1239` (`full_loop_gate`).
//!
//! # Scenarios
//!
//! [`IntegrationGate`] dispatches over three [`IntegrationScenario`]
//! variants:
//!
//! - [`IntegrationScenario::BuildTest`] — invoke the project's test
//!   runner (e.g. `cargo test -- <pattern>`), aggregating the
//!   `passed/failed/ignored` summary into [`TestCount`].
//! - [`IntegrationScenario::Script`] — execute a shell script from
//!   disk via `bash`; exit code 0 is the source of truth.
//! - [`IntegrationScenario::Custom`] — run a caller-supplied
//!   [`IntegrationScenarioFn`] that spawns whatever long-lived
//!   dependencies it needs (Anvil, golem, mirage, ...) and returns
//!   its own [`Verdict`].
//!
//! # Invariants
//!
//! 1. **Warmup honored** — the gate sleeps `warmup_ms` before the
//!    scenario starts, so callers who have already primed dependencies
//!    can set it to zero.
//! 2. **Outer timeout is a kill** — if the scenario does not return
//!    within `timeout_ms` the spawned process is dropped
//!    (`kill_on_drop(true)`) and the verdict is a timeout failure.
//! 3. **No retry inside the gate** — this is the expensive rung;
//!    retry policy lives upstream in the pipeline.
//! 4. **Detail bounded** — stdout/stderr truncated to the last 32 `KiB`
//!    (integration logs are bigger and more load-bearing than the
//!    smaller gates' output).
//! 5. **Custom scenarios own their cleanup** — the closure is
//!    responsible for tearing down whatever it spawned.

use crate::payload::{BuildSystem, GatePayload};
use async_trait::async_trait;
use roko_core::{Context, Signal, TestCount, Verdict, Verify};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

/// Maximum bytes of stdout/stderr retained in a verdict's detail.
///
/// Integration scenarios produce much more output than unit tests;
/// Mori's `full_loop_gate` truncates at the same 32 `KiB` ceiling.
const MAX_DETAIL_BYTES: usize = 32 * 1024;

/// Default warm-up period before a scenario runs, in milliseconds.
///
/// Lets dependencies (Anvil, mirage, ...) settle. Callers that have
/// already primed their dependencies should override via
/// [`IntegrationGate::with_warmup_ms`].
const DEFAULT_WARMUP_MS: u64 = 2_000;

/// Default scenario timeout, in milliseconds (120 s).
const DEFAULT_TIMEOUT_MS: u64 = 120_000;

// ─── IntegrationScenario / IntegrationScenarioFn ───────────────────────

/// Async closure describing a custom integration scenario.
///
/// Implementors own every resource they spawn: Anvil nodes, golems,
/// mirages, temp files. They must tear everything down before
/// returning. The gate's only responsibility is to wrap the closure in
/// a timeout.
#[async_trait]
pub trait IntegrationScenarioFn: Send + Sync {
    /// Execute the scenario; returns the verdict to attribute to the gate.
    async fn run(&self, signal: &Signal, ctx: &Context) -> Verdict;
}

type BoxedScenarioFuture = Pin<Box<dyn Future<Output = Verdict> + Send + 'static>>;

/// Internal wrapper that turns a plain `Fn` into an
/// [`IntegrationScenarioFn`].
struct FnScenario<F>
where
    F: Fn(Signal, Context) -> BoxedScenarioFuture + Send + Sync + 'static,
{
    f: F,
}

#[async_trait]
impl<F> IntegrationScenarioFn for FnScenario<F>
where
    F: Fn(Signal, Context) -> BoxedScenarioFuture + Send + Sync + 'static,
{
    async fn run(&self, signal: &Signal, ctx: &Context) -> Verdict {
        (self.f)(signal.clone(), ctx.clone()).await
    }
}

/// How an [`IntegrationGate`] runs its scenario.
pub enum IntegrationScenario {
    /// Run the project's test binary filtered by a pattern
    /// (e.g. `cargo test -- test_golem`).
    BuildTest {
        /// Which build system to dispatch through.
        build: BuildSystem,
        /// Pattern filter appended after `--`.
        pattern: String,
    },
    /// Execute a shell script on disk (via `bash`).
    Script {
        /// Absolute path to the script.
        path: PathBuf,
    },
    /// A fully custom scenario: caller supplies an async closure.
    Custom(Arc<dyn IntegrationScenarioFn>),
}

impl std::fmt::Debug for IntegrationScenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BuildTest { build, pattern } => f
                .debug_struct("BuildTest")
                .field("build", build)
                .field("pattern", pattern)
                .finish(),
            Self::Script { path } => f.debug_struct("Script").field("path", path).finish(),
            Self::Custom(_) => f.debug_struct("Custom").finish(),
        }
    }
}

// ─── IntegrationGate ────────────────────────────────────────────────────

/// Rung 5 gate: run an end-to-end scenario; pass iff it completes cleanly.
///
/// Construct via the dedicated builders ([`IntegrationGate::build_test`],
/// [`IntegrationGate::script`], [`IntegrationGate::custom`]) and tune
/// warmup/timeouts with [`IntegrationGate::with_warmup_ms`] and
/// [`IntegrationGate::with_timeout_ms`].
pub struct IntegrationGate {
    scenario: IntegrationScenario,
    timeout_ms: u64,
    warmup_ms: u64,
    name: String,
}

impl IntegrationGate {
    /// Construct an integration gate that runs a filtered test pattern
    /// through the given build system (e.g. `cargo test -- test_golem`).
    #[must_use]
    pub fn build_test(build: BuildSystem, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        let name = format!("integration:build_test:{pattern}");
        Self {
            scenario: IntegrationScenario::BuildTest { build, pattern },
            timeout_ms: DEFAULT_TIMEOUT_MS,
            warmup_ms: DEFAULT_WARMUP_MS,
            name,
        }
    }

    /// Construct an integration gate that runs a shell script.
    #[must_use]
    pub fn script(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("script");
        let name = format!("integration:script:{label}");
        Self {
            scenario: IntegrationScenario::Script { path },
            timeout_ms: DEFAULT_TIMEOUT_MS,
            warmup_ms: DEFAULT_WARMUP_MS,
            name,
        }
    }

    /// Construct an integration gate from a caller-supplied scenario
    /// trait object.
    #[must_use]
    pub fn custom(f: Arc<dyn IntegrationScenarioFn>) -> Self {
        Self {
            scenario: IntegrationScenario::Custom(f),
            timeout_ms: DEFAULT_TIMEOUT_MS,
            warmup_ms: DEFAULT_WARMUP_MS,
            name: "integration:custom".into(),
        }
    }

    /// Construct an integration gate from a plain async closure.
    ///
    /// Convenience wrapper that avoids the caller needing to define a
    /// separate `IntegrationScenarioFn` impl for one-off scenarios.
    #[must_use]
    pub fn from_fn<F, Fut>(f: F) -> Self
    where
        F: Fn(Signal, Context) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Verdict> + Send + 'static,
    {
        let scenario = Arc::new(FnScenario {
            f: move |sig, ctx| -> BoxedScenarioFuture { Box::pin(f(sig, ctx)) },
        });
        Self::custom(scenario)
    }

    /// Override the pre-scenario warmup in milliseconds (default
    /// [`DEFAULT_WARMUP_MS`]).
    #[must_use]
    pub const fn with_warmup_ms(mut self, ms: u64) -> Self {
        self.warmup_ms = ms;
        self
    }

    /// Override the scenario timeout in milliseconds (default
    /// [`DEFAULT_TIMEOUT_MS`]).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the verdict's gate name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Effective warmup — clamped strictly below the timeout.
    const fn effective_warmup_ms(&self) -> u64 {
        if self.timeout_ms == 0 {
            0
        } else if self.warmup_ms >= self.timeout_ms {
            self.timeout_ms.saturating_sub(1)
        } else {
            self.warmup_ms
        }
    }
}

impl roko_core::Cell for IntegrationGate {
    fn cell_id(&self) -> &str {
        "integration-gate"
    }
    fn cell_name(&self) -> &str {
        "IntegrationGate"
    }
    fn protocols(&self) -> &[&str] {
        &["Verify"]
    }
}

#[async_trait]
impl Verify for IntegrationGate {
    async fn verify(&self, signal: &Signal, ctx: &Context) -> Verdict {
        let started = Instant::now();

        let warmup = self.effective_warmup_ms();
        if warmup > 0 {
            tokio::time::sleep(Duration::from_millis(warmup)).await;
        }

        let total_budget = Duration::from_millis(self.timeout_ms);
        let remaining = total_budget
            .checked_sub(started.elapsed())
            .unwrap_or_else(|| Duration::from_millis(0));

        let verdict = match &self.scenario {
            IntegrationScenario::BuildTest { build, pattern } => {
                run_build_test(&self.name, *build, pattern, signal, remaining).await
            }
            IntegrationScenario::Script { path } => {
                run_script(&self.name, path, signal, remaining).await
            }
            IntegrationScenario::Custom(f) => {
                run_custom(&self.name, f.as_ref(), signal, ctx, remaining).await
            }
        };

        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        verdict.with_duration(elapsed)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// ─── Scenario runners ───────────────────────────────────────────────────

async fn run_build_test(
    gate_name: &str,
    build: BuildSystem,
    pattern: &str,
    signal: &Signal,
    remaining: Duration,
) -> Verdict {
    if remaining.is_zero() {
        return Verdict::fail(gate_name, "timed out before scenario started");
    }

    let payload: Option<GatePayload> = signal.body.as_json().ok();
    let mut cmd = Command::new(build.program());
    let target_crates = payload
        .as_ref()
        .map(|p| p.target_crates.as_slice())
        .unwrap_or(&[]);
    for arg in build.scoped_test_args(target_crates) {
        cmd.arg(arg);
    }
    // Append `-- <pattern>` (or build-system equivalent).
    match build {
        BuildSystem::Go => {
            cmd.arg("-run").arg(pattern);
        }
        BuildSystem::Forge | BuildSystem::Make => {
            cmd.arg(pattern);
        }
        BuildSystem::Cargo | BuildSystem::Npm | BuildSystem::Python => {
            cmd.arg("--").arg(pattern);
        }
    }

    if let Some(ref p) = payload {
        cmd.current_dir(&p.working_dir);
        if let Some(ref tgt) = p.target_dir {
            cmd.env("CARGO_TARGET_DIR", tgt);
        }
        for (k, v) in &p.extra_env {
            cmd.env(k, v);
        }
    }
    cmd.kill_on_drop(true);

    let output = match timeout(remaining, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Verdict::fail(gate_name, format!("spawn failed: {e}"));
        }
        Err(_) => {
            return Verdict::fail(
                gate_name,
                format!(
                    "timed out after {} ms",
                    remaining.as_millis().min(u128::from(u64::MAX))
                ),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = truncate_detail(&stdout, &stderr);
    let counts = parse_cargo_style_counts(&combined);
    let digest = extract_failure_digest(&combined);

    let mut verdict = if output.status.success() {
        Verdict::pass(gate_name).with_detail(combined)
    } else {
        let code = output
            .status
            .code()
            .map_or_else(|| "terminated by signal".to_string(), |c| c.to_string());
        Verdict::fail(gate_name, format!("scenario exit code: {code}")).with_detail(combined)
    };
    if let Some(tc) = counts {
        verdict = verdict.with_test_count(tc);
    }
    if !digest.is_empty() {
        verdict = verdict.with_error_digest(digest);
    }
    verdict
}

async fn run_script(
    gate_name: &str,
    path: &std::path::Path,
    signal: &Signal,
    remaining: Duration,
) -> Verdict {
    if remaining.is_zero() {
        return Verdict::fail(gate_name, "timed out before scenario started");
    }

    let payload: Option<GatePayload> = signal.body.as_json().ok();
    let mut cmd = Command::new("bash");
    cmd.arg(path);
    if let Some(ref p) = payload {
        cmd.current_dir(&p.working_dir);
        for (k, v) in &p.extra_env {
            cmd.env(k, v);
        }
    }
    cmd.kill_on_drop(true);

    let output = match timeout(remaining, cmd.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Verdict::fail(gate_name, format!("spawn failed: {e}"));
        }
        Err(_) => {
            return Verdict::fail(
                gate_name,
                format!(
                    "timed out after {} ms",
                    remaining.as_millis().min(u128::from(u64::MAX))
                ),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let combined = truncate_detail(&stdout, &stderr);

    if output.status.success() {
        Verdict::pass(gate_name).with_detail(combined)
    } else {
        let code = output
            .status
            .code()
            .map_or_else(|| "terminated by signal".to_string(), |c| c.to_string());
        Verdict::fail(gate_name, format!("script exit code: {code}")).with_detail(combined)
    }
}

async fn run_custom(
    gate_name: &str,
    scenario: &dyn IntegrationScenarioFn,
    signal: &Signal,
    ctx: &Context,
    remaining: Duration,
) -> Verdict {
    if remaining.is_zero() {
        return Verdict::fail(gate_name, "timed out before scenario started");
    }

    let fut = scenario.run(signal, ctx);
    timeout(remaining, fut).await.unwrap_or_else(|_| {
        Verdict::fail(
            gate_name,
            format!(
                "timed out after {} ms",
                remaining.as_millis().min(u128::from(u64::MAX))
            ),
        )
    })
}

// ─── Helpers ────────────────────────────────────────────────────────────

fn truncate_detail(stdout: &str, stderr: &str) -> String {
    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        format!("---stderr---\n{stderr}")
    } else {
        format!("{stdout}\n---stderr---\n{stderr}")
    };
    if combined.len() <= MAX_DETAIL_BYTES {
        return combined;
    }
    // Keep the last MAX_DETAIL_BYTES bytes, aligned to a char
    // boundary so String is valid.
    let split_at = combined.len() - MAX_DETAIL_BYTES;
    let mut idx = split_at;
    while idx < combined.len() && !combined.is_char_boundary(idx) {
        idx += 1;
    }
    format!("...[truncated {} bytes]...\n{}", idx, &combined[idx..])
}

fn parse_cargo_style_counts(output: &str) -> Option<TestCount> {
    let mut total = TestCount::default();
    let mut saw_summary = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("test result:") {
            continue;
        }
        saw_summary = true;
        let passed = extract_number_before(trimmed, "passed");
        let failed = extract_number_before(trimmed, "failed");
        let ignored = extract_number_before(trimmed, "ignored");
        total.passed += passed;
        total.failed += failed;
        total.ignored += ignored;
    }
    if saw_summary { Some(total) } else { None }
}

fn extract_number_before(line: &str, label: &str) -> u32 {
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

fn extract_failure_digest(output: &str) -> String {
    let mut names: Vec<&str> = Vec::new();
    for line in output.lines() {
        let t = line.trim();
        if (t.starts_with("test ") && t.ends_with(" ... FAILED"))
            || t.starts_with("--- FAIL:")
            || t.starts_with("FAIL\t")
        {
            names.push(t);
        }
        if names.len() >= 5 {
            break;
        }
    }
    names.join("; ")
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use std::sync::atomic::{AtomicU32, Ordering};

    fn scaled_test_timeout_ms(ms: u64) -> u64 {
        if std::env::var("CI").is_ok_and(|value| value == "true") {
            ms.saturating_mul(10)
        } else {
            ms
        }
    }

    fn empty_signal() -> Signal {
        Signal::builder(Kind::Task).body(Body::empty()).build()
    }

    fn script_signal(dir: &std::path::Path) -> Signal {
        let payload = GatePayload::in_dir(dir);
        Signal::builder(Kind::Task)
            .body(Body::from_json(&payload).unwrap())
            .build()
    }

    // --- constructor / builder tests ---

    #[test]
    fn build_test_constructor_sets_name_and_defaults() {
        let g = IntegrationGate::build_test(BuildSystem::Cargo, "test_golem");
        assert_eq!(g.name(), "integration:build_test:test_golem");
        assert_eq!(g.timeout_ms, DEFAULT_TIMEOUT_MS);
        assert_eq!(g.warmup_ms, DEFAULT_WARMUP_MS);
        match &g.scenario {
            IntegrationScenario::BuildTest { build, pattern } => {
                assert_eq!(*build, BuildSystem::Cargo);
                assert_eq!(pattern, "test_golem");
            }
            _ => panic!("expected BuildTest variant"),
        }
    }

    #[test]
    fn script_constructor_names_from_filename() {
        let g = IntegrationGate::script("/tmp/generated-integration.sh");
        assert_eq!(g.name(), "integration:script:generated-integration.sh");
        match &g.scenario {
            IntegrationScenario::Script { path } => {
                assert_eq!(path, std::path::Path::new("/tmp/generated-integration.sh"));
            }
            _ => panic!("expected Script variant"),
        }
    }

    #[test]
    fn custom_constructor_has_generic_name() {
        struct Noop;
        #[async_trait]
        impl IntegrationScenarioFn for Noop {
            async fn run(&self, _s: &Signal, _c: &Context) -> Verdict {
                Verdict::pass("noop")
            }
        }
        let g = IntegrationGate::custom(Arc::new(Noop));
        assert_eq!(g.name(), "integration:custom");
    }

    #[test]
    fn with_name_overrides_display_name() {
        let g = IntegrationGate::build_test(BuildSystem::Cargo, "foo").with_name("lifecycle");
        assert_eq!(g.name(), "lifecycle");
    }

    #[test]
    fn builder_overrides_warmup_and_timeout() {
        let g = IntegrationGate::build_test(BuildSystem::Cargo, "foo")
            .with_warmup_ms(500)
            .with_timeout_ms(9_000);
        assert_eq!(g.warmup_ms, 500);
        assert_eq!(g.timeout_ms, 9_000);
    }

    #[test]
    fn effective_warmup_clamps_below_timeout() {
        let g = IntegrationGate::build_test(BuildSystem::Cargo, "foo")
            .with_warmup_ms(10_000)
            .with_timeout_ms(1_000);
        assert!(g.effective_warmup_ms() < g.timeout_ms);
        assert_eq!(g.effective_warmup_ms(), 999);
    }

    #[test]
    fn effective_warmup_zero_when_timeout_zero() {
        let g = IntegrationGate::build_test(BuildSystem::Cargo, "foo")
            .with_warmup_ms(500)
            .with_timeout_ms(0);
        assert_eq!(g.effective_warmup_ms(), 0);
    }

    // --- helper tests ---

    #[test]
    fn truncate_detail_keeps_short_output() {
        let out = truncate_detail("hello", "");
        assert_eq!(out, "hello");
    }

    #[test]
    fn truncate_detail_merges_stderr_tag() {
        let out = truncate_detail("stdout-data", "stderr-data");
        assert!(out.contains("stdout-data"));
        assert!(out.contains("---stderr---"));
        assert!(out.contains("stderr-data"));
    }

    #[test]
    fn truncate_detail_caps_large_output() {
        let big = "a".repeat(64 * 1024);
        let out = truncate_detail(&big, "");
        assert!(out.len() <= MAX_DETAIL_BYTES + 64);
        assert!(out.starts_with("...[truncated"));
    }

    #[test]
    fn parse_counts_recognises_cargo_summary() {
        let out = "test result: ok. 7 passed; 0 failed; 2 ignored";
        let tc = parse_cargo_style_counts(out).expect("summary present");
        assert_eq!(tc, TestCount::new(7, 0, 2));
    }

    #[test]
    fn parse_counts_returns_none_when_absent() {
        assert!(parse_cargo_style_counts("nothing here").is_none());
    }

    #[test]
    fn extract_digest_picks_first_failed_tests() {
        let out = "\
test foo::a ... FAILED
test foo::b ... FAILED
test foo::c ... ok";
        let d = extract_failure_digest(out);
        assert!(d.contains("foo::a"));
        assert!(d.contains("foo::b"));
        assert!(!d.contains("foo::c"));
    }

    // --- Verify::verify behaviour (custom scenarios) ---

    #[tokio::test]
    async fn custom_pass_scenario_passes_through() {
        let gate =
            IntegrationGate::from_fn(|_sig, _ctx| async move { Verdict::pass("scenario-ok") })
                .with_warmup_ms(0)
                .with_timeout_ms(5_000)
                .with_name("custom-pass");
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(v.passed, "verdict should pass; got {v:?}");
        assert_eq!(v.gate, "scenario-ok");
    }

    #[tokio::test]
    async fn custom_fail_scenario_passes_through() {
        let gate = IntegrationGate::from_fn(|_sig, _ctx| async move {
            Verdict::fail("scenario-bad", "deliberate failure")
        })
        .with_warmup_ms(0)
        .with_timeout_ms(5_000);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("deliberate failure"));
    }

    #[tokio::test]
    async fn custom_timeout_kills_scenario() {
        let gate = IntegrationGate::from_fn(|_sig, _ctx| async move {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Verdict::pass("never")
        })
        .with_warmup_ms(0)
        .with_timeout_ms(scaled_test_timeout_ms(80));
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("timed out"), "reason was: {}", v.reason);
    }

    #[tokio::test]
    async fn warmup_is_applied_before_custom_scenario() {
        let fire_count = Arc::new(AtomicU32::new(0));
        let fire_clone = fire_count.clone();
        let gate = IntegrationGate::from_fn(move |_s, _c| {
            let counter = fire_clone.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Verdict::pass("warm")
            }
        })
        .with_warmup_ms(120)
        .with_timeout_ms(2_000);
        let start = Instant::now();
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        let elapsed = start.elapsed();
        assert!(v.passed);
        assert_eq!(fire_count.load(Ordering::SeqCst), 1);
        assert!(
            elapsed >= Duration::from_millis(100),
            "warmup should take at least ~120ms, actually {elapsed:?}"
        );
        assert!(v.duration_ms >= 100, "duration_ms was {}", v.duration_ms);
    }

    #[tokio::test]
    async fn zero_warmup_skips_sleep() {
        let gate = IntegrationGate::from_fn(|_s, _c| async move { Verdict::pass("fast") })
            .with_warmup_ms(0)
            .with_timeout_ms(5_000);
        let start = Instant::now();
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        let elapsed = start.elapsed();
        assert!(v.passed);
        assert!(
            elapsed < Duration::from_millis(500),
            "with zero warmup should be quick, got {elapsed:?}"
        );
    }

    // --- Verify::verify behaviour (script scenarios) ---

    #[tokio::test]
    async fn script_passes_when_exit_zero() {
        let tmp = std::env::temp_dir();
        let path = tmp.join(format!("roko-integration-pass-{}.sh", std::process::id()));
        std::fs::write(&path, "#!/usr/bin/env bash\necho ALL-OK\nexit 0\n").unwrap();
        let gate = IntegrationGate::script(&path)
            .with_warmup_ms(0)
            .with_timeout_ms(5_000);
        let v = gate.verify(&script_signal(&tmp), &Context::at(0)).await;
        let _ = std::fs::remove_file(&path);
        assert!(v.passed, "script should pass; got: {v:?}");
        assert!(v.detail.as_deref().unwrap_or("").contains("ALL-OK"));
    }

    #[tokio::test]
    async fn script_fails_when_exit_nonzero() {
        let tmp = std::env::temp_dir();
        let path = tmp.join(format!("roko-integration-fail-{}.sh", std::process::id()));
        std::fs::write(&path, "#!/usr/bin/env bash\necho going-bad 1>&2\nexit 7\n").unwrap();
        let gate = IntegrationGate::script(&path)
            .with_warmup_ms(0)
            .with_timeout_ms(5_000);
        let v = gate.verify(&script_signal(&tmp), &Context::at(0)).await;
        let _ = std::fs::remove_file(&path);
        assert!(!v.passed);
        assert!(v.reason.contains("exit code"));
        assert!(
            v.detail.as_deref().unwrap_or("").contains("going-bad"),
            "stderr should be captured in detail"
        );
    }

    #[tokio::test]
    async fn script_missing_file_fails_gracefully() {
        let gate = IntegrationGate::script("/definitely/not/a/real/script/xyz.sh")
            .with_warmup_ms(0)
            .with_timeout_ms(1_000);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        // Either a spawn failure or an exit-code failure is acceptable.
        assert!(
            v.reason.contains("spawn failed") || v.reason.contains("exit code"),
            "unexpected reason: {}",
            v.reason
        );
    }

    // --- Verify::verify behaviour (build-test scenarios) ---

    #[tokio::test]
    async fn build_test_spawn_failure_yields_verdict() {
        // Point at a working dir that exists but has no Cargo project.
        let tmp = std::env::temp_dir();
        let payload = GatePayload::in_dir(&tmp);
        let sig = Signal::builder(Kind::Task)
            .body(Body::from_json(&payload).unwrap())
            .build();
        let gate = IntegrationGate::build_test(BuildSystem::Cargo, "__no_such_test")
            .with_warmup_ms(0)
            .with_timeout_ms(30_000);
        let v = gate.verify(&sig, &Context::at(0)).await;
        // Either cargo is installed and reports no manifest (fail), or
        // not installed and we get a spawn failure. Both satisfy the
        // contract: the gate does not hang and returns a failing verdict.
        assert!(!v.passed);
    }
}
