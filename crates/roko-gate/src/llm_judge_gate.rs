//! `LlmJudgeGate` — Rung 5 of the 6-rung verification ladder (§10.12).
//!
//! An advisory quality gate: asks a "judge oracle" (typically a cheap LLM
//! like Haiku) to score a diff and passes iff the score clears a
//! configurable threshold. Non-blocking by default — routine API flakes
//! should never fail a pipeline. Callers opt into hard-stop behaviour via
//! [`LlmJudgeGate::blocking`].
//!
//! # Why a local trait
//!
//! The judge delegates to an `Agent`-like oracle. `roko-gate` deliberately
//! does **not** depend on `roko-agent` (cycle), so this module defines a
//! minimal [`JudgeOracle`] trait that callers implement — typically with a
//! thin wrapper around an `Agent`. Tests use a mock oracle.
//!
//! # Signal body contract
//!
//! The gate reads a [`JudgePayload`] from the signal body. Supported shapes:
//!
//! - a JSON body matching [`JudgePayload`] (`task_description` + `diff`)
//! - a text body — treated as the full prompt (no truncation)
//! - an empty body — fails immediately with reason "no diff to judge"
//!
//! # Verdict
//!
//! - `passed = score >= min_score` (and the diff is non-empty)
//! - `Verdict.score` carries the raw judge score clamped to `[0, 1]`
//! - oracle errors under `non_blocking` → pass with `detail` explaining why

use async_trait::async_trait;
use roko_core::{Context, Gate, Signal, Verdict};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

/// A minimal, `Agent`-agnostic oracle interface the judge delegates to.
///
/// Implementors typically wrap an LLM client (Anthropic Haiku, `OpenAI`,
/// Ollama, …) or an existing `roko_agent::Agent`. The oracle receives an
/// already-assembled prompt and returns a single quality score.
///
/// # Score convention
///
/// The returned `f32` is a normalized quality score in `[0, 1]`. Values
/// outside the range are clamped by the gate.
///
/// # Errors
///
/// Any transport, parsing, or timeout failure should be surfaced as
/// `Err(String)`. The gate decides whether the error is fatal based on
/// its `non_blocking` setting.
#[async_trait]
pub trait JudgeOracle: Send + Sync {
    /// Score the given prompt, returning a quality score.
    async fn judge(&self, prompt: &str) -> Result<f32, String>;
}

/// Structured input the gate expects in a signal body.
///
/// When absent (text body, empty body), the gate degrades gracefully.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgePayload {
    /// The task/goal description the diff is supposed to satisfy.
    pub task_description: String,
    /// The unified diff (or blob of changed code) being judged.
    pub diff: String,
}

/// LLM-as-judge gate.
///
/// See the [module docs](self) for the full behavioural contract.
pub struct LlmJudgeGate {
    oracle: Arc<dyn JudgeOracle>,
    min_score: f32,
    non_blocking: bool,
    max_diff_bytes: usize,
    name: String,
}

impl LlmJudgeGate {
    /// Default maximum diff size sent to the oracle: 30 `KiB`.
    ///
    /// Mirrors Mori's default. Configurable via
    /// [`LlmJudgeGate::with_max_diff_bytes`].
    pub const DEFAULT_MAX_DIFF_BYTES: usize = 30 * 1024;

    /// Construct a judge gate that passes iff the oracle returns at least
    /// `min_score`. `min_score` is clamped to `[0, 1]`.
    #[must_use]
    pub fn new(oracle: Arc<dyn JudgeOracle>, min_score: f32) -> Self {
        Self {
            oracle,
            min_score: min_score.clamp(0.0, 1.0),
            non_blocking: true,
            max_diff_bytes: Self::DEFAULT_MAX_DIFF_BYTES,
            name: "llm_judge".to_string(),
        }
    }

    /// Switch the gate from advisory (default) to hard-stop mode.
    ///
    /// In blocking mode, oracle errors fail the verdict instead of passing
    /// with a warning detail.
    #[must_use]
    pub const fn blocking(mut self) -> Self {
        self.non_blocking = false;
        self
    }

    /// Override the diff truncation cap (bytes).
    #[must_use]
    pub const fn with_max_diff_bytes(mut self, n: usize) -> Self {
        self.max_diff_bytes = n;
        self
    }

    /// Override the gate's display name.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// True if the gate is currently in advisory (non-blocking) mode.
    #[must_use]
    pub const fn is_non_blocking(&self) -> bool {
        self.non_blocking
    }

    /// The configured minimum score threshold (`[0, 1]`).
    #[must_use]
    pub const fn min_score(&self) -> f32 {
        self.min_score
    }

    /// Extract the (task, diff) pair from a signal body.
    ///
    /// Returns `None` when the body is empty OR both fields are empty
    /// after decoding.
    fn extract_payload(signal: &Signal) -> Option<JudgePayload> {
        // Prefer a structured JudgePayload if present.
        if let Ok(payload) = signal.body.as_json::<JudgePayload>() {
            return Some(payload);
        }
        // Fall back to a plain text body: treat the entire text as the diff
        // so the oracle still has something concrete to look at.
        if let Ok(text) = signal.body.as_text() {
            if text.is_empty() {
                return None;
            }
            return Some(JudgePayload {
                task_description: String::new(),
                diff: text.to_string(),
            });
        }
        None
    }

    /// Truncate `diff` to at most `max_diff_bytes` UTF-8 bytes.
    ///
    /// Truncates on a char boundary so the output is always valid UTF-8.
    /// Appends a `"\n...[truncated]"` marker when anything was cut.
    fn truncate_diff(diff: &str, max_bytes: usize) -> String {
        if diff.len() <= max_bytes {
            return diff.to_string();
        }
        // Walk back to the nearest char boundary so we never slice a
        // multi-byte codepoint.
        let mut cut = max_bytes;
        while cut > 0 && !diff.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut out = String::with_capacity(cut + 16);
        out.push_str(&diff[..cut]);
        out.push_str("\n...[truncated]");
        out
    }

    /// Assemble the prompt sent to the oracle. Pure, cheap to test.
    fn build_prompt(payload: &JudgePayload, max_diff_bytes: usize) -> String {
        let diff = Self::truncate_diff(&payload.diff, max_diff_bytes);
        if payload.task_description.is_empty() {
            format!("Score this diff on a 0.0-1.0 scale.\n\nDiff:\n{diff}")
        } else {
            format!(
                "Score this implementation on a 0.0-1.0 scale.\n\nTask: {}\n\nDiff:\n{}",
                payload.task_description, diff,
            )
        }
    }
}

#[async_trait]
impl Gate for LlmJudgeGate {
    async fn verify(&self, signal: &Signal, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let elapsed = |t: Instant| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);

        let Some(payload) = Self::extract_payload(signal) else {
            return Verdict::fail(&self.name, "no diff to judge").with_duration(elapsed(started));
        };
        if payload.diff.is_empty() {
            return Verdict::fail(&self.name, "no diff to judge").with_duration(elapsed(started));
        }

        let prompt = Self::build_prompt(&payload, self.max_diff_bytes);

        let verdict = match self.oracle.judge(&prompt).await {
            Ok(raw_score) => {
                let score = raw_score.clamp(0.0, 1.0);
                if score >= self.min_score {
                    Verdict::pass(&self.name).with_score(score)
                } else {
                    Verdict::fail(
                        &self.name,
                        format!("score {score:.3} below threshold {:.3}", self.min_score),
                    )
                    .with_score(score)
                    .with_error_digest(format!("judge score={score:.3}"))
                }
            }
            Err(err) => {
                if self.non_blocking {
                    Verdict::pass(&self.name)
                        .with_detail(format!("judge unavailable: {err}"))
                        .with_score(self.min_score)
                } else {
                    Verdict::fail(&self.name, format!("judge error: {err}"))
                }
            }
        };
        verdict.with_duration(elapsed(started))
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use roko_core::{Body, Kind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock oracle that returns a canned score.
    struct ConstOracle {
        score: f32,
        calls: AtomicUsize,
    }

    impl ConstOracle {
        fn new(score: f32) -> Arc<Self> {
            Arc::new(Self {
                score,
                calls: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl JudgeOracle for ConstOracle {
        async fn judge(&self, _prompt: &str) -> Result<f32, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.score)
        }
    }

    /// Mock oracle that always errors.
    struct ErrOracle;

    #[async_trait]
    impl JudgeOracle for ErrOracle {
        async fn judge(&self, _prompt: &str) -> Result<f32, String> {
            Err("simulated outage".to_string())
        }
    }

    /// Mock oracle that captures the prompt it received.
    #[allow(clippy::disallowed_types)] // tests use std::sync::Mutex for simplicity
    struct RecordingOracle {
        last_prompt: std::sync::Mutex<String>,
        score: f32,
    }

    impl RecordingOracle {
        #[allow(clippy::disallowed_types)]
        fn new(score: f32) -> Arc<Self> {
            Arc::new(Self {
                last_prompt: std::sync::Mutex::new(String::new()),
                score,
            })
        }

        fn prompt(&self) -> String {
            self.last_prompt
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait]
    impl JudgeOracle for RecordingOracle {
        async fn judge(&self, prompt: &str) -> Result<f32, String> {
            if let Ok(mut g) = self.last_prompt.lock() {
                *g = prompt.to_string();
            }
            Ok(self.score)
        }
    }

    fn payload_signal(task: &str, diff: &str) -> Signal {
        let body = Body::from_json(&JudgePayload {
            task_description: task.to_string(),
            diff: diff.to_string(),
        })
        .expect("serialize JudgePayload");
        Signal::builder(Kind::Task).body(body).build()
    }

    fn empty_signal() -> Signal {
        Signal::builder(Kind::Task).body(Body::empty()).build()
    }

    #[tokio::test]
    async fn passes_when_score_meets_threshold() {
        let gate = LlmJudgeGate::new(ConstOracle::new(0.95), 0.8);
        let v = gate
            .verify(
                &payload_signal("add login", "diff --git a/x b/x\n+fn login() {}"),
                &Context::at(0),
            )
            .await;
        assert!(v.passed);
        assert!(v.score > 0.9);
        assert_eq!(v.gate, "llm_judge");
    }

    #[tokio::test]
    async fn fails_when_score_below_threshold() {
        let gate = LlmJudgeGate::new(ConstOracle::new(0.4), 0.75);
        let v = gate
            .verify(&payload_signal("task", "+some change"), &Context::at(0))
            .await;
        assert!(!v.passed);
        assert!(v.reason.contains("below threshold"));
        assert!((v.score - 0.4).abs() < 1e-6);
        assert!(v.error_digest.is_some());
    }

    #[tokio::test]
    async fn oracle_error_nonblocking_passes() {
        let gate = LlmJudgeGate::new(Arc::new(ErrOracle), 0.8);
        assert!(gate.is_non_blocking());
        let v = gate
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(v.passed, "non-blocking should pass on oracle error");
        let detail = v.detail.as_deref().unwrap_or("");
        assert!(detail.contains("judge unavailable"));
        assert!(detail.contains("simulated outage"));
    }

    #[tokio::test]
    async fn oracle_error_blocking_fails() {
        let gate = LlmJudgeGate::new(Arc::new(ErrOracle), 0.8).blocking();
        assert!(!gate.is_non_blocking());
        let v = gate
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(!v.passed);
        assert!(v.reason.contains("judge error"));
        assert!(v.reason.contains("simulated outage"));
    }

    #[tokio::test]
    async fn empty_diff_fails_regardless_of_blocking() {
        let nb = LlmJudgeGate::new(ConstOracle::new(1.0), 0.5);
        let v = nb.verify(&payload_signal("t", ""), &Context::at(0)).await;
        assert!(!v.passed);
        assert_eq!(v.reason, "no diff to judge");

        let b = LlmJudgeGate::new(ConstOracle::new(1.0), 0.5).blocking();
        let v = b.verify(&payload_signal("t", ""), &Context::at(0)).await;
        assert!(!v.passed);
        assert_eq!(v.reason, "no diff to judge");
    }

    #[tokio::test]
    async fn empty_body_fails() {
        let gate = LlmJudgeGate::new(ConstOracle::new(1.0), 0.5);
        let v = gate.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert_eq!(v.reason, "no diff to judge");
    }

    #[tokio::test]
    async fn diff_is_truncated_to_max_bytes() {
        let oracle = RecordingOracle::new(0.9);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5).with_max_diff_bytes(64);
        let big_diff = "a".repeat(10_000);
        let v = gate
            .verify(&payload_signal("t", &big_diff), &Context::at(0))
            .await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        // Prompt should be far smaller than the original 10 KiB diff.
        assert!(prompt.len() < 500);
        assert!(prompt.contains("[truncated]"));
    }

    #[tokio::test]
    async fn small_diff_is_not_truncated() {
        let oracle = RecordingOracle::new(0.9);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5);
        let v = gate
            .verify(&payload_signal("t", "+short diff"), &Context::at(0))
            .await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        assert!(prompt.contains("+short diff"));
        assert!(!prompt.contains("[truncated]"));
    }

    #[tokio::test]
    async fn text_body_treated_as_diff() {
        let oracle = RecordingOracle::new(0.9);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5);
        let signal = Signal::builder(Kind::Task)
            .body(Body::text("+added line\n-removed line"))
            .build();
        let v = gate.verify(&signal, &Context::at(0)).await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        assert!(prompt.contains("+added line"));
    }

    #[tokio::test]
    async fn score_out_of_range_is_clamped() {
        let high = LlmJudgeGate::new(ConstOracle::new(7.0), 0.5);
        let v = high
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(v.passed);
        assert!((v.score - 1.0).abs() < 1e-6);

        let low = LlmJudgeGate::new(ConstOracle::new(-3.0), 0.5);
        let v = low
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(!v.passed);
        assert!((v.score - 0.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn min_score_constructor_clamps() {
        let lo = LlmJudgeGate::new(ConstOracle::new(0.0), -2.0);
        assert!((lo.min_score() - 0.0).abs() < 1e-6);
        let hi = LlmJudgeGate::new(ConstOracle::new(0.0), 5.0);
        assert!((hi.min_score() - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn custom_name_surfaces_in_verdict() {
        let gate = LlmJudgeGate::new(ConstOracle::new(0.99), 0.5).with_name("haiku_judge");
        assert_eq!(gate.name(), "haiku_judge");
        let v = gate
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert_eq!(v.gate, "haiku_judge");
    }

    #[tokio::test]
    async fn prompt_includes_task_when_present() {
        let oracle = RecordingOracle::new(0.8);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5);
        let v = gate
            .verify(
                &payload_signal("implement login", "+fn login()"),
                &Context::at(0),
            )
            .await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        assert!(prompt.contains("implement login"));
        assert!(prompt.contains("+fn login()"));
    }

    #[tokio::test]
    async fn prompt_omits_task_line_when_empty() {
        let oracle = RecordingOracle::new(0.8);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5);
        let v = gate
            .verify(&payload_signal("", "+only diff"), &Context::at(0))
            .await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        assert!(!prompt.contains("Task:"));
        assert!(prompt.contains("+only diff"));
    }

    #[tokio::test]
    async fn utf8_truncation_respects_char_boundary() {
        // Use 2-byte UTF-8 codepoints so a naive byte cut would split one.
        let oracle = RecordingOracle::new(0.9);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5).with_max_diff_bytes(5);
        let diff = "àààààààààà"; // 10 × 2-byte chars = 20 bytes.
        let v = gate
            .verify(&payload_signal("t", diff), &Context::at(0))
            .await;
        assert!(v.passed);
        let prompt = oracle.prompt();
        // Did not panic; prompt is valid UTF-8 (string).
        assert!(prompt.contains("[truncated]"));
    }

    #[tokio::test]
    async fn exact_threshold_passes() {
        let gate = LlmJudgeGate::new(ConstOracle::new(0.75), 0.75);
        let v = gate
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(v.passed, "score == min_score should pass");
    }

    #[tokio::test]
    async fn verdict_records_duration_ms() {
        // Regression: LlmJudgeGate previously skipped `.with_duration(..)`,
        // making pipeline duration aggregates drop its time. Every code
        // path (ok/fail, blocking/nonblocking, no-diff) must set it.
        let good = LlmJudgeGate::new(ConstOracle::new(0.9), 0.5);
        let v = good
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(v.passed);
        // duration_ms may be 0 on a fast path, but the field must be
        // populated (present and non-sentinel). We assert the sentinel
        // u64::MAX is never set (which would indicate overflow).
        assert_ne!(v.duration_ms, u64::MAX);

        let bad = LlmJudgeGate::new(ConstOracle::new(0.1), 0.5);
        let v = bad
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert!(!v.passed);
        assert_ne!(v.duration_ms, u64::MAX);

        // No-diff path should also set duration.
        let empty = LlmJudgeGate::new(ConstOracle::new(1.0), 0.5);
        let v = empty.verify(&empty_signal(), &Context::at(0)).await;
        assert!(!v.passed);
        assert_ne!(v.duration_ms, u64::MAX);
    }

    #[tokio::test]
    async fn oracle_called_exactly_once_per_verify() {
        let oracle = ConstOracle::new(0.9);
        let gate = LlmJudgeGate::new(oracle.clone(), 0.5);
        let _ = gate
            .verify(&payload_signal("t", "some diff"), &Context::at(0))
            .await;
        assert_eq!(oracle.calls.load(Ordering::SeqCst), 1);
    }
}
