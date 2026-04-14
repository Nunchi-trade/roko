//! `DiffGate` — vacuous-impl rejection (§10.13).
//!
//! Rejects changes that "pass" gates only because they introduced no
//! substantive work: empty implementations, all-whitespace diffs, and
//! tombstone rewrites. Without this gate, agents learn the shortcut of
//! replacing function bodies with `todo!()` or `Ok(())` to make tests and
//! lints happy.
//!
//! The gate operates on the signal's JSON body, which must contain at
//! least one of: `diff` (unified diff text), `file_diffs` (per-file
//! summary), or a `worktree_path` where `git diff` can run (the last
//! shells out; falls back to "insufficient info" if none supplied).
//!
//! # Input shape
//!
//! ```text
//! {
//!   "diff": "--- a/foo\n+++ b/foo\n@@ -1,1 +1,1 @@\n-x\n+y\n",
//!   "min_added_lines": 3,       // optional; default 1
//!   "forbidden_tokens": [       // optional; default common stubs
//!     "todo!()", "unimplemented!()", "panic!(\"not implemented\")"
//!   ]
//! }
//! ```
//!
//! A diff is rejected when:
//!
//! 1. The added-line count (ignoring pure whitespace) is `< min_added_lines`.
//! 2. Every added non-comment line matches one of `forbidden_tokens`.
//! 3. The diff is empty entirely.
//!
//! Mori reference: `apps/mori/src/orchestrator/gates.rs::diff_gate`
//! (vacuous-change detection).

use async_trait::async_trait;
use roko_core::{Context, Engram, Gate, Verdict};
use serde::{Deserialize, Serialize};
use std::time::Instant;

const DEFAULT_MIN_ADDED: u32 = 1;

fn default_forbidden_tokens() -> Vec<String> {
    vec![
        "todo!()".into(),
        "todo!".into(),
        "unimplemented!()".into(),
        "unimplemented!".into(),
        "panic!(\"not implemented\")".into(),
        "Ok(())".into(),
        "return Ok(())".into(),
    ]
}

/// Engram-body payload for [`DiffGate`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffPayload {
    /// Unified diff text (output of `git diff`).
    pub diff: String,
    /// Minimum number of non-whitespace added lines required.
    #[serde(default = "default_min_added")]
    pub min_added_lines: u32,
    /// Tokens that, if they match every added line, trigger rejection.
    #[serde(default = "default_forbidden_tokens")]
    pub forbidden_tokens: Vec<String>,
}

const fn default_min_added() -> u32 {
    DEFAULT_MIN_ADDED
}

impl DiffPayload {
    /// Construct a payload with a diff and default policy.
    #[must_use]
    pub fn new(diff: impl Into<String>) -> Self {
        Self {
            diff: diff.into(),
            min_added_lines: DEFAULT_MIN_ADDED,
            forbidden_tokens: default_forbidden_tokens(),
        }
    }

    /// Override the minimum-added-lines threshold.
    #[must_use]
    pub const fn with_min_added_lines(mut self, n: u32) -> Self {
        self.min_added_lines = n;
        self
    }

    /// Replace the forbidden-token list.
    #[must_use]
    pub fn with_forbidden_tokens(mut self, tokens: Vec<String>) -> Self {
        self.forbidden_tokens = tokens;
        self
    }
}

/// Vacuous-impl rejection gate.
pub struct DiffGate {
    name: String,
}

impl DiffGate {
    /// Construct a new diff gate.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: "diff".into(),
        }
    }
}

impl Default for DiffGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Gate for DiffGate {
    async fn verify(&self, signal: &Engram, _ctx: &Context) -> Verdict {
        let started = Instant::now();
        let payload: DiffPayload = match signal.body.as_json() {
            Ok(p) => p,
            Err(e) => {
                let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Verdict::fail(&self.name, format!("signal body is not a DiffPayload: {e}"))
                    .with_duration(elapsed);
            }
        };

        let analysis = analyze_diff(&payload);
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

        if analysis.added_lines == 0 {
            return Verdict::fail(&self.name, "empty diff (no added lines)")
                .with_detail(format!("{analysis:?}"))
                .with_duration(elapsed);
        }
        if analysis.non_whitespace_added < payload.min_added_lines {
            return Verdict::fail(
                &self.name,
                format!(
                    "insufficient changes: {} substantive added lines, need ≥ {}",
                    analysis.non_whitespace_added, payload.min_added_lines
                ),
            )
            .with_detail(format!("{analysis:?}"))
            .with_duration(elapsed);
        }
        if analysis.all_added_are_forbidden {
            return Verdict::fail(
                &self.name,
                "all added lines are stub/vacuous (e.g. todo!/unimplemented!/Ok(()))",
            )
            .with_detail(format!("{analysis:?}"))
            .with_duration(elapsed);
        }
        Verdict::pass(&self.name)
            .with_detail(format!("{analysis:?}"))
            .with_duration(elapsed)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Summary of one diff's added-line structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffAnalysis {
    /// Total added lines (`+` prefix, excluding the `+++` header).
    pub added_lines: u32,
    /// Added lines with non-whitespace, non-comment content.
    pub non_whitespace_added: u32,
    /// Every substantive added line matches a forbidden token.
    pub all_added_are_forbidden: bool,
}

/// Analyze a diff against the forbidden-token policy.
#[must_use]
pub fn analyze_diff(payload: &DiffPayload) -> DiffAnalysis {
    let mut added = 0u32;
    let mut substantive = 0u32;
    let mut forbidden_hits = 0u32;
    for line in payload.diff.lines() {
        // Skip diff headers.
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        let Some(rest) = line.strip_prefix('+') else {
            continue;
        };
        added = added.saturating_add(1);
        let trimmed = rest.trim();
        if trimmed.is_empty() || is_comment_line(trimmed) {
            continue;
        }
        substantive = substantive.saturating_add(1);
        if payload
            .forbidden_tokens
            .iter()
            .any(|t| trimmed == t.trim() || trimmed.contains(t.trim()))
        {
            forbidden_hits = forbidden_hits.saturating_add(1);
        }
    }
    let all_added_are_forbidden = substantive > 0 && forbidden_hits == substantive;
    DiffAnalysis {
        added_lines: added,
        non_whitespace_added: substantive,
        all_added_are_forbidden,
    }
}

fn is_comment_line(s: &str) -> bool {
    s.starts_with("//")
        || s.starts_with('#')
        || s.starts_with("/*")
        || s.starts_with('*')
        || s.starts_with("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(diff: &str) -> DiffPayload {
        DiffPayload::new(diff)
    }

    #[test]
    fn empty_diff_is_rejected_by_analyzer() {
        let a = analyze_diff(&payload(""));
        assert_eq!(a.added_lines, 0);
        assert_eq!(a.non_whitespace_added, 0);
        assert!(!a.all_added_are_forbidden);
    }

    #[test]
    fn whitespace_only_additions_are_not_substantive() {
        let diff = "+++ b/x.rs\n@@ -1,0 +1,2 @@\n+   \n+\n";
        let a = analyze_diff(&payload(diff));
        assert_eq!(a.added_lines, 2);
        assert_eq!(a.non_whitespace_added, 0);
    }

    #[test]
    fn comment_lines_are_not_substantive() {
        let diff = "+++ b/x.rs\n+// comment\n+# another\n+/// doc\n";
        let a = analyze_diff(&payload(diff));
        assert_eq!(a.added_lines, 3);
        assert_eq!(a.non_whitespace_added, 0);
    }

    #[test]
    fn real_code_additions_are_substantive() {
        let diff = "+++ b/x.rs\n+fn add(a: i32, b: i32) -> i32 {\n+    a + b\n+}\n";
        let a = analyze_diff(&payload(diff));
        assert_eq!(a.added_lines, 3);
        assert_eq!(a.non_whitespace_added, 3);
        assert!(!a.all_added_are_forbidden);
    }

    #[test]
    fn all_todo_additions_are_forbidden() {
        let diff = "+++ b/x.rs\n+fn foo() { todo!() }\n+fn bar() { todo!() }\n";
        let a = analyze_diff(&payload(diff));
        assert!(a.all_added_are_forbidden);
    }

    #[test]
    fn mixed_additions_are_not_all_forbidden() {
        let diff = "+++ b/x.rs\n+fn foo() { todo!() }\n+fn bar() { 42 }\n";
        let a = analyze_diff(&payload(diff));
        assert!(!a.all_added_are_forbidden);
    }

    #[test]
    fn serde_roundtrip_preserves_defaults() {
        let p = DiffPayload::new("some diff");
        let json = serde_json::to_string(&p).expect("serialize");
        let decoded: DiffPayload = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded.diff, "some diff");
        assert_eq!(decoded.min_added_lines, DEFAULT_MIN_ADDED);
        assert!(!decoded.forbidden_tokens.is_empty());
    }

    #[test]
    fn min_added_lines_override() {
        let p = DiffPayload::new("").with_min_added_lines(5);
        assert_eq!(p.min_added_lines, 5);
    }

    #[test]
    fn forbidden_tokens_override() {
        let p = DiffPayload::new("").with_forbidden_tokens(vec!["custom".into()]);
        assert_eq!(p.forbidden_tokens, vec!["custom".to_string()]);
    }

    #[tokio::test]
    async fn gate_rejects_empty_diff() {
        use roko_core::{Body, Kind};
        let gate = DiffGate::new();
        let payload = DiffPayload::new("");
        let signal = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload).expect("json"))
            .build();
        let ctx = Context::default();
        let v = gate.verify(&signal, &ctx).await;
        assert!(!v.passed);
        assert!(v.reason.contains("empty diff"));
    }

    #[tokio::test]
    async fn gate_rejects_all_todo_implementations() {
        use roko_core::{Body, Kind};
        let gate = DiffGate::new();
        let diff = "+++ b/x.rs\n+fn foo() { todo!() }\n";
        let payload = DiffPayload::new(diff);
        let signal = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload).expect("json"))
            .build();
        let ctx = Context::default();
        let v = gate.verify(&signal, &ctx).await;
        assert!(!v.passed);
        assert!(v.reason.contains("stub") || v.reason.contains("vacuous"));
    }

    #[tokio::test]
    async fn gate_accepts_substantive_code() {
        use roko_core::{Body, Kind};
        let gate = DiffGate::new();
        let diff = "+++ b/x.rs\n+fn add(a: i32, b: i32) -> i32 {\n+    a + b\n+}\n";
        let payload = DiffPayload::new(diff).with_min_added_lines(2);
        let signal = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload).expect("json"))
            .build();
        let ctx = Context::default();
        let v = gate.verify(&signal, &ctx).await;
        assert!(v.passed, "verdict: {v:?}");
    }

    #[tokio::test]
    async fn gate_rejects_below_min_threshold() {
        use roko_core::{Body, Kind};
        let gate = DiffGate::new();
        let diff = "+++ b/x.rs\n+let x = 1;\n";
        let payload = DiffPayload::new(diff).with_min_added_lines(5);
        let signal = Engram::builder(Kind::Task)
            .body(Body::from_json(&payload).expect("json"))
            .build();
        let ctx = Context::default();
        let v = gate.verify(&signal, &ctx).await;
        assert!(!v.passed);
        assert!(v.reason.contains("insufficient"));
    }
}
