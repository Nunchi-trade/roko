//! `VerifyChainGate` — Rung 4 of the 6-rung verification ladder (§10.7).
//!
//! Plan-specific `verify.sh` scripts authored by humans (or generated from
//! PRD acceptance criteria) run the custom oracle pipeline a plan needs.
//! This gate shells out to `bash <script>`, parses the
//! `[PASS]`/`[FAIL]` line protocol emitted by the script, and translates
//! the result into a structured [`Verdict`].
//!
//! When no script is provided for a signal, the gate either fails ("strict")
//! or delegates to a fallback gate (typically [`TestGate`](crate::TestGate))
//! — mirroring Mori's behaviour in
//! `apps/mori/src/orchestrator/gates.rs::verify_chain_gate`.
//!
//! # Script resolution
//!
//! The path to the script is read from the input [`Engram`]:
//!
//! 1. If `signal.tag("verify_script")` is set, that path is used.
//! 2. Otherwise the gate treats it as "no script for this signal" and
//!    either falls back (if configured) or fails.
//!
//! The path may be absolute or relative to the [`GatePayload::working_dir`]
//! decoded from the signal body. When relative, it is joined against the
//! working dir so scripts can sit next to the plan they verify.
//!
//! # Script line protocol
//!
//! ```text
//! [PASS] step-name (12 tests)
//! [FAIL] step-name (missing symbol RateLimiter)
//! running 0 tests                 # ← zero-test step; flagged iff
//!                                 #   zero_test_guard is enabled
//! ```
//!
//! Exit code is authoritative: `passed = exit == 0 && !zero_test_failure`.
//! If the first run fails and `retry_once` is enabled (default), the gate
//! sleeps 2s and re-runs once before reporting failure.

use crate::payload::GatePayload;
use async_trait::async_trait;
use roko_core::{Context, Engram, Verify, Verdict};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::{sleep, timeout};

/// Tag key the gate reads from [`Engram::tag`] to find the verify script.
pub const VERIFY_SCRIPT_TAG: &str = "verify_script";

/// Maximum number of bytes of combined stdout/stderr retained in
/// [`Verdict::detail`] — keeps verdicts small on the event bus.
const DETAIL_TAIL_BYTES: usize = 16 * 1024;

/// Verify that runs a plan-specific `verify.sh` and parses its output.
///
/// See the module-level documentation for the script resolution and line
/// protocol. Construct with [`VerifyChainGate::strict`] (no fallback) or
/// [`VerifyChainGate::with_fallback`] (delegate on missing script).
pub struct VerifyChainGate {
    fallback: Option<Arc<dyn Verify>>,
    timeout_ms: u64,
    retry_once: bool,
    zero_test_guard: bool,
    retry_delay_ms: u64,
    name: String,
}

impl VerifyChainGate {
    /// Construct a gate that fails when no `verify.sh` is provided.
    ///
    /// Suitable for plans whose acceptance criteria require a script —
    /// the absence of one is itself a failure condition.
    #[must_use]
    pub fn strict() -> Self {
        Self {
            fallback: None,
            timeout_ms: 20 * 60 * 1000, // 20 minutes, matching Mori
            retry_once: true,
            zero_test_guard: false,
            retry_delay_ms: 2_000,
            name: "verify_chain".into(),
        }
    }

    /// Construct a gate that delegates to `fallback` when no script is set.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `fallback.name()` equals this gate's own
    /// name — delegating to another `VerifyChainGate` would risk a cycle
    /// when the inner gate also fails to find a script.
    #[must_use]
    pub fn with_fallback(fallback: Arc<dyn Verify>) -> Self {
        debug_assert!(
            fallback.name() != "verify_chain",
            "verify_chain fallback must not be another verify_chain gate (cycle risk)",
        );
        Self {
            fallback: Some(fallback),
            ..Self::strict()
        }
    }

    /// Retry once on failure with a 2s sleep. Default: true.
    #[must_use]
    pub const fn with_retry(mut self, retry: bool) -> Self {
        self.retry_once = retry;
        self
    }

    /// Flag zero-test steps as failures even when the script exits 0.
    /// Default: false (Mori trusts the script's own `has_positive` guard).
    #[must_use]
    pub const fn with_zero_test_guard(mut self, guard: bool) -> Self {
        self.zero_test_guard = guard;
        self
    }

    /// Override the timeout in milliseconds (default: 20 minutes).
    #[must_use]
    pub const fn with_timeout_ms(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Override the retry sleep delay (default: 2000 ms).
    #[must_use]
    pub const fn with_retry_delay_ms(mut self, ms: u64) -> Self {
        self.retry_delay_ms = ms;
        self
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Resolve the script path from the signal + payload, or return `None`.
    fn resolve_script(signal: &Engram, payload: Option<&GatePayload>) -> Option<PathBuf> {
        let raw = signal.tag(VERIFY_SCRIPT_TAG)?;
        let p = Path::new(raw);
        if p.is_absolute() {
            Some(p.to_path_buf())
        } else {
            Some(payload.map_or_else(|| p.to_path_buf(), |pl| pl.working_dir.join(p)))
        }
    }

    async fn run_once(
        &self,
        script: &Path,
        payload: Option<&GatePayload>,
    ) -> Result<std::process::Output, std::io::Error> {
        let mut cmd = Command::new("bash");
        cmd.arg(script);
        cmd.kill_on_drop(true);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        if let Some(p) = payload {
            cmd.current_dir(&p.working_dir);
            cmd.env("REPO_ROOT", &p.working_dir);
            if let Some(ref tgt) = p.target_dir {
                cmd.env("CARGO_TARGET_DIR", tgt);
            }
            for (k, v) in &p.extra_env {
                cmd.env(k, v);
            }
        }
        cmd.output().await
    }
}

impl roko_core::Cell for VerifyChainGate {
    fn cell_id(&self) -> &str { "verify-chain-gate" }
    fn cell_name(&self) -> &str { "VerifyChainGate" }
    fn protocols(&self) -> &[&str] { &["Verify"] }
}

#[async_trait]
impl Verify for VerifyChainGate {
    async fn verify(&self, signal: &Engram, ctx: &Context) -> Verdict {
        let started = Instant::now();
        // GatePayload is optional: some callers may rely on absolute
        // script paths and the current process's cwd.
        let payload: Option<GatePayload> = signal.body.as_json().ok();
        let script = Self::resolve_script(signal, payload.as_ref());

        let script = match script {
            Some(s) if s.exists() => s,
            _ => {
                if let Some(ref fb) = self.fallback {
                    return fb.verify(signal, ctx).await;
                }
                let reason = script.as_ref().map_or_else(
                    || {
                        format!(
                            "signal is missing '{VERIFY_SCRIPT_TAG}' tag (no fallback configured)"
                        )
                    },
                    |s| {
                        format!(
                            "verify script not found at {} (no fallback configured)",
                            s.display()
                        )
                    },
                );
                let elapsed = elapsed_ms(started);
                return Verdict::fail(&self.name, reason).with_duration(elapsed);
            }
        };

        // First attempt.
        let attempt = self.attempt(&script, payload.as_ref(), started).await;

        let mut attempt = match attempt {
            Ok(a) => a,
            Err(v) => return v,
        };

        let mut attempts_used: u32 = 1;
        if !attempt.passed && self.retry_once {
            sleep(Duration::from_millis(self.retry_delay_ms)).await;
            match self.attempt(&script, payload.as_ref(), started).await {
                Ok(retry) => {
                    attempt = retry;
                    attempts_used = 2;
                }
                Err(v) => return v,
            }
        }

        let elapsed = elapsed_ms(started);
        let pass_count = attempt.pass_count;
        let fail_count = attempt.fail_count;
        let zero_test_steps = attempt.zero_test_steps;
        let exit_code = attempt.exit_code;
        let detail = truncate_tail(&attempt.combined, DETAIL_TAIL_BYTES);

        let base_reason = if attempt.passed {
            String::new()
        } else if fail_count > 0 || !zero_test_steps.is_empty() {
            if zero_test_steps.is_empty() {
                format!("verify-chain: {fail_count} FAIL marker(s) in script output")
            } else {
                format!(
                    "verify-chain: zero-test step(s) rejected: {}",
                    zero_test_steps.join(", "),
                )
            }
        } else {
            exit_code.map_or_else(
                || "verify-chain script terminated by signal".into(),
                |c| format!("verify-chain script exit {c}"),
            )
        };

        let reason = if attempts_used == 2 {
            if attempt.passed {
                format!("passed after retry; {pass_count} PASS, {fail_count} FAIL")
            } else {
                format!("{base_reason} (failed both attempts)")
            }
        } else {
            base_reason
        };

        let mut verdict = if attempt.passed {
            Verdict::pass(&self.name)
        } else {
            Verdict::fail(&self.name, reason.clone())
        };
        verdict = verdict.with_detail(detail).with_duration(elapsed);

        if !attempt.passed {
            let digest = if zero_test_steps.is_empty() {
                parse_verify_chain_failure(&attempt.stripped)
            } else {
                format!(
                    "verify-chain matched zero tests for step(s): {}",
                    zero_test_steps.join(", ")
                )
            };
            if !digest.is_empty() {
                verdict = verdict.with_error_digest(digest);
            }
        } else if attempts_used == 2 {
            // Record the retry-success note where the pass reason lives.
            verdict.reason = reason;
        }

        verdict
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Result of a single bash invocation + parse.
struct Attempt {
    passed: bool,
    exit_code: Option<i32>,
    pass_count: u32,
    fail_count: u32,
    zero_test_steps: Vec<String>,
    combined: String,
    stripped: String,
}

impl VerifyChainGate {
    async fn attempt(
        &self,
        script: &Path,
        payload: Option<&GatePayload>,
        started: Instant,
    ) -> Result<Attempt, Verdict> {
        let remaining = self.timeout_ms.saturating_sub(elapsed_ms(started)).max(1);
        let fut = self.run_once(script, payload);
        let output = match timeout(Duration::from_millis(remaining), fut).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let elapsed = elapsed_ms(started);
                return Err(
                    Verdict::fail(&self.name, format!("spawn failed: {e}")).with_duration(elapsed)
                );
            }
            Err(_) => {
                let elapsed = elapsed_ms(started);
                return Err(Verdict::fail(
                    &self.name,
                    format!("verify-chain timed out after {} ms", self.timeout_ms),
                )
                .with_duration(elapsed));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let combined = format!("{stdout}\n{stderr}");
        let stripped = strip_ansi(&combined);
        let (pass_count, fail_count) = parse_verify_chain_counts(&stripped);
        let zero_test_steps = if self.zero_test_guard {
            parse_zero_test_steps(&stripped)
        } else {
            Vec::new()
        };
        let exit_code = output.status.code();
        let passed = output.status.success() && zero_test_steps.is_empty();

        Ok(Attempt {
            passed,
            exit_code,
            pass_count,
            fail_count,
            zero_test_steps,
            combined,
            stripped,
        })
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Count `[PASS]` / `[FAIL]` markers in the (already-ANSI-stripped) output.
#[must_use]
pub fn parse_verify_chain_counts(output: &str) -> (u32, u32) {
    let mut pass = 0u32;
    let mut fail = 0u32;
    for line in output.lines() {
        let t = line.trim();
        if t.contains("[PASS]") || t.contains("] PASS") || t.ends_with(" PASS") {
            pass = pass.saturating_add(1);
        }
        if t.contains("[FAIL]") || t.contains("] FAIL") || t.ends_with(" FAIL") {
            fail = fail.saturating_add(1);
        }
    }
    (pass, fail)
}

/// Detect steps that emitted `running 0 tests` without positive test output.
#[must_use]
pub fn parse_zero_test_steps(output: &str) -> Vec<String> {
    // Walk the output, tracking the current bracketed step label. When we
    // see `running 0 tests` or `0 passed; 0 failed`, flag the step unless a
    // later line within the same step shows positive tests.
    let mut flagged: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut saw_zero = false;
    let mut saw_positive = false;
    for raw in output.lines() {
        let line = raw.trim();
        if let Some(label) = parse_step_tag(line) {
            let is_terminal = line.contains("] PASS")
                || line.contains("] FAIL")
                || line.ends_with(" PASS")
                || line.ends_with(" FAIL");
            if is_terminal {
                if line.contains("PASS")
                    && saw_zero
                    && !saw_positive
                    && current.as_deref() == Some(label)
                    && !flagged.iter().any(|s| s == label)
                {
                    flagged.push(label.to_string());
                }
                current = None;
            } else {
                current = Some(label.to_string());
            }
            saw_zero = false;
            saw_positive = false;
            continue;
        }
        if current.is_some() {
            if let Some(rest) = line.strip_prefix("running ") {
                if let Some(n_str) = rest.strip_suffix(" tests") {
                    match n_str.trim().parse::<u32>() {
                        Ok(0) => saw_zero = true,
                        Ok(_) => saw_positive = true,
                        Err(_) => {}
                    }
                }
            } else if line.starts_with("test result:") {
                if line.contains(" 0 passed; 0 failed;") {
                    saw_zero = true;
                } else {
                    saw_positive = true;
                }
            }
        }
    }
    flagged
}

fn parse_step_tag(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let close = rest.find(']')?;
    Some(&rest[..close])
}

/// Extract a short digest of failing lines from verify-chain output.
#[must_use]
pub fn parse_verify_chain_failure(output: &str) -> String {
    let mut result = String::new();
    let mut in_fail = false;
    let mut captured: u32 = 0;
    for line in output.lines() {
        if (line.contains("[FAIL]") || line.contains("] FAIL")) && !in_fail {
            in_fail = true;
            result.push_str(line);
            result.push('\n');
            captured = 1;
            continue;
        }
        if in_fail {
            result.push_str(line);
            result.push('\n');
            captured += 1;
            if captured >= 30 {
                break;
            }
        }
    }
    if result.len() > 2000 {
        result.truncate(2000);
        result.push_str("\n... (truncated)");
    }
    result
}

/// Remove ANSI escape sequences from a string.
#[must_use]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI (and other) escapes: consume until final byte in `@`..=`~`.
            // Minimal parser: accept `ESC [ ... <final>` and `ESC <final>`.
            if chars.peek().copied() == Some('[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else if chars.peek().is_some() {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn truncate_tail(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let start = s.len() - max_bytes;
    // Walk forward to a char boundary so we never slice mid-codepoint.
    let mut cut = start;
    while cut < s.len() && !s.is_char_boundary(cut) {
        cut += 1;
    }
    let mut out = String::with_capacity(max_bytes + 32);
    out.push_str("... (truncated)\n");
    out.push_str(&s[cut..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn scaled_test_timeout_ms(ms: u64) -> u64 {
        if std::env::var("CI").is_ok_and(|value| value == "true") {
            ms.saturating_mul(10)
        } else {
            ms
        }
    }

    fn signal_with_script(dir: &std::path::Path, script: &str) -> Engram {
        let payload = GatePayload::in_dir(dir);
        let body = Body::from_json(&payload).expect("serialize payload");
        Engram::builder(Kind::Task)
            .body(body)
            .tag(VERIFY_SCRIPT_TAG, script)
            .build()
    }

    fn signal_without_tag(dir: &std::path::Path) -> Engram {
        let payload = GatePayload::in_dir(dir);
        let body = Body::from_json(&payload).expect("serialize payload");
        Engram::builder(Kind::Task).body(body).build()
    }

    fn write_script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, body).expect("write script");
        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("chmod");
        path
    }

    #[tokio::test]
    async fn passing_script_yields_pass_verdict() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\n\
             echo '[PASS] build'\n\
             echo '[PASS] tests'\n\
             echo '[PASS] lint'\n\
             exit 0\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict().with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed, "reason: {}", v.reason);
        assert_eq!(v.gate, "verify_chain");
        assert!(v.detail.is_some());
    }

    #[tokio::test]
    async fn failing_script_yields_fail_verdict_with_digest() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\n\
             echo '[PASS] build'\n\
             echo '[PASS] tests'\n\
             echo '[FAIL] lint: missing symbol RateLimiter'\n\
             exit 1\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict().with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(
            v.error_digest.as_deref().unwrap_or("").contains("FAIL"),
            "digest: {:?}",
            v.error_digest
        );
    }

    #[tokio::test]
    async fn script_exit_nonzero_without_markers_fails() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\necho 'silent failure'\nexit 3\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict().with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("exit 3"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn missing_script_with_strict_fails() {
        let tmp = TempDir::new().expect("tmpdir");
        let sig = signal_with_script(tmp.path(), "does-not-exist.sh");
        let gate = VerifyChainGate::strict();
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("not found"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn missing_tag_with_strict_fails() {
        let tmp = TempDir::new().expect("tmpdir");
        let sig = signal_without_tag(tmp.path());
        let gate = VerifyChainGate::strict();
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("verify_script"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn missing_script_with_fallback_delegates() {
        use roko_std::noop::NoOpGate;
        let tmp = TempDir::new().expect("tmpdir");
        let sig = signal_without_tag(tmp.path());
        let gate = VerifyChainGate::with_fallback(Arc::new(NoOpGate));
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed);
        assert_eq!(v.gate, "noop_gate");
    }

    #[tokio::test]
    async fn existing_script_does_not_delegate_to_fallback() {
        use roko_std::noop::NoOpGate;
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\necho '[FAIL] x'\nexit 1\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::with_fallback(Arc::new(NoOpGate)).with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        // The real script ran and failed; fallback (which would pass) was not used.
        assert!(!v.passed);
        assert_eq!(v.gate, "verify_chain");
    }

    #[tokio::test]
    async fn retry_succeeds_on_second_attempt() {
        let tmp = TempDir::new().expect("tmpdir");
        let counter = tmp.path().join("count");
        write_script(
            tmp.path(),
            "verify.sh",
            &format!(
                "#!/usr/bin/env bash\n\
                 n=$(cat {path} 2>/dev/null || echo 0)\n\
                 echo $((n+1)) > {path}\n\
                 if [ \"$n\" = \"0\" ]; then\n\
                   echo '[FAIL] flaky'\n\
                   exit 1\n\
                 fi\n\
                 echo '[PASS] flaky'\n\
                 exit 0\n",
                path = counter.display(),
            ),
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict()
            .with_retry(true)
            .with_retry_delay_ms(10);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed, "reason: {}", v.reason);
        assert!(v.reason.contains("retry"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn retry_disabled_keeps_first_failure() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\necho '[FAIL] nope'\nexit 1\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict().with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(!v.reason.contains("both attempts"));
    }

    #[tokio::test]
    async fn retry_fails_both_attempts_marks_reason() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\necho '[FAIL] always'\nexit 1\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict()
            .with_retry(true)
            .with_retry_delay_ms(10);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("both attempts"), "reason: {}", v.reason,);
    }

    #[tokio::test]
    async fn zero_test_guard_trips_on_empty_run() {
        let tmp = TempDir::new().expect("tmpdir");
        // Script passes at the exit-code level but a step reports "running 0 tests".
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\n\
             echo '[doc-tests]'\n\
             echo 'running 0 tests'\n\
             echo '[doc-tests] PASS'\n\
             exit 0\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict()
            .with_retry(false)
            .with_zero_test_guard(true);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed, "reason: {}", v.reason);
        assert!(v.reason.contains("zero-test"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn zero_test_guard_off_trusts_exit_code() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(
            tmp.path(),
            "verify.sh",
            "#!/usr/bin/env bash\n\
             echo '[doc-tests]'\n\
             echo 'running 0 tests'\n\
             echo '[doc-tests] PASS'\n\
             exit 0\n",
        );
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict()
            .with_retry(false)
            .with_zero_test_guard(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed, "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn timeout_kills_long_running_script() {
        let tmp = TempDir::new().expect("tmpdir");
        write_script(tmp.path(), "verify.sh", "#!/usr/bin/env bash\nsleep 10\n");
        let sig = signal_with_script(tmp.path(), "verify.sh");
        let gate = VerifyChainGate::strict()
            .with_retry(false)
            .with_timeout_ms(scaled_test_timeout_ms(150));
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(!v.passed);
        assert!(v.reason.contains("timed out"), "reason: {}", v.reason);
    }

    #[tokio::test]
    async fn absolute_script_path_resolves() {
        let tmp = TempDir::new().expect("tmpdir");
        let abs = write_script(
            tmp.path(),
            "abs-verify.sh",
            "#!/usr/bin/env bash\necho '[PASS] ok'\nexit 0\n",
        );
        let abs_str = abs.to_string_lossy().into_owned();
        // Working dir is a different directory; script is resolved via abs path.
        let other = TempDir::new().expect("tmpdir");
        let sig = signal_with_script(other.path(), &abs_str);
        let gate = VerifyChainGate::strict().with_retry(false);
        let v = gate.verify(&sig, &Context::at(0)).await;
        assert!(v.passed, "reason: {}", v.reason);
    }

    // ─── unit tests for parsers (no process spawning) ────────────────────

    #[test]
    fn parse_counts_pass_fail_markers() {
        let out = "[PASS] build\n[PASS] tests\n[FAIL] lint\n";
        let (p, f) = parse_verify_chain_counts(out);
        assert_eq!(p, 2);
        assert_eq!(f, 1);
    }

    #[test]
    fn parse_counts_ignores_unrelated_lines() {
        let out = "starting\nrunning tests\ndone\n";
        let (p, f) = parse_verify_chain_counts(out);
        assert_eq!(p, 0);
        assert_eq!(f, 0);
    }

    #[test]
    fn strip_ansi_removes_color_codes() {
        let colored = "\x1b[31m[FAIL]\x1b[0m lint\n";
        let plain = strip_ansi(colored);
        assert_eq!(plain, "[FAIL] lint\n");
    }

    #[test]
    fn strip_ansi_leaves_plain_text_alone() {
        assert_eq!(strip_ansi("hello world"), "hello world");
    }

    #[test]
    fn parse_zero_test_steps_flags_empty_pass() {
        let out = "[docs]\nrunning 0 tests\n[docs] PASS\n";
        assert_eq!(parse_zero_test_steps(out), vec!["docs".to_string()]);
    }

    #[test]
    fn parse_zero_test_steps_ignores_positive_runs() {
        let out =
            "[lib]\nrunning 5 tests\ntest result: ok. 5 passed; 0 failed; 0 ignored\n[lib] PASS\n";
        assert!(parse_zero_test_steps(out).is_empty());
    }

    #[test]
    fn parse_zero_test_steps_ignores_failed_steps() {
        // A FAILED step is already a FAIL; don't double-count it as a zero-test.
        let out = "[docs]\nrunning 0 tests\n[docs] FAIL\n";
        assert!(parse_zero_test_steps(out).is_empty());
    }

    #[test]
    fn parse_verify_chain_failure_captures_fail_block() {
        let out = "[PASS] a\n[FAIL] b\ncontext-line-1\ncontext-line-2\n";
        let d = parse_verify_chain_failure(out);
        assert!(d.contains("[FAIL] b"));
        assert!(d.contains("context-line-1"));
    }

    #[test]
    fn truncate_tail_preserves_tail_and_adds_prefix() {
        let s = "a".repeat(100);
        let t = truncate_tail(&s, 20);
        assert!(t.starts_with("... (truncated)"));
        assert!(t.ends_with(&"a".repeat(20)));
    }

    #[test]
    fn truncate_tail_identity_when_small_enough() {
        assert_eq!(truncate_tail("short", 100), "short");
    }

    #[test]
    fn with_fallback_rejects_self() {
        // The debug_assert! fires in debug builds when wiring a
        // verify_chain gate as its own fallback. We verify the name
        // equality check that triggers it.
        struct FakeCycle;
        #[async_trait]
        impl Verify for FakeCycle {
            async fn verify(&self, _: &Engram, _: &Context) -> Verdict {
                Verdict::pass("verify_chain")
            }
            #[allow(clippy::unnecessary_literal_bound)]
            fn name(&self) -> &str {
                "verify_chain"
            }
        }
        let hit = std::panic::catch_unwind(|| {
            let _ = VerifyChainGate::with_fallback(Arc::new(FakeCycle));
        });
        // debug_assert only fires in debug builds; assert it actually panics
        // there, otherwise accept the release-build no-op.
        if cfg!(debug_assertions) {
            assert!(hit.is_err());
        } else {
            assert!(hit.is_ok());
        }
    }
}
