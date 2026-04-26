//! Compile failure repeat watcher: detects repeated identical compile errors.
//!
//! When the same compile diagnostic appears [`MAX_IDENTICAL_COMPILE_FAILURES`]
//! consecutive times, the agent is stuck on the same error and needs a restart.

use roko_core::{Body, Context, Engram, Kind, React};

/// Maximum consecutive identical compile failures before firing.
pub const MAX_IDENTICAL_COMPILE_FAILURES: usize = 3;
/// Docs-compatible alias for [`MAX_IDENTICAL_COMPILE_FAILURES`].
pub const MAX_COMPILE_FAIL_REPEAT: usize = MAX_IDENTICAL_COMPILE_FAILURES;

/// Tag key marking signals from this watcher.
pub const WATCHER_NAME: &str = "compile-fail-repeat";

/// Detects 3+ consecutive identical compile failures.
///
/// Scans `CompileDiagnostic` signals from the end of the stream and
/// fires when the same error text appears consecutively at least
/// [`MAX_COMPILE_FAIL_REPEAT`] times.
#[derive(Debug, Clone)]
pub struct CompileFailRepeatWatcher {
    /// Consecutive identical compile failures before firing.
    max_failures: usize,
}

impl Default for CompileFailRepeatWatcher {
    fn default() -> Self {
        Self {
            max_failures: MAX_IDENTICAL_COMPILE_FAILURES,
        }
    }
}

impl CompileFailRepeatWatcher {
    /// Create with a custom threshold.
    #[must_use]
    pub const fn new(max_failures: usize) -> Self {
        Self { max_failures }
    }
}

/// Extract a normalized error key from a compile diagnostic signal.
fn diagnostic_key(signal: &Engram) -> Option<String> {
    match &signal.body {
        Body::Text(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Body::Json(v) => {
            // Try "message" field, else stringify the whole value.
            v.get("message")
                .and_then(|m| m.as_str())
                .map(|s| s.trim().to_owned())
                .or_else(|| Some(v.to_string()))
        }
        _ => None,
    }
}

impl React for CompileFailRepeatWatcher {
    fn decide(&self, stream: &[Engram], _ctx: &Context) -> Vec<Engram> {
        // Collect compile diagnostic signals in order.
        let diagnostics: Vec<&Engram> = stream
            .iter()
            .filter(|s| s.kind == Kind::CompileDiagnostic)
            .collect();

        if diagnostics.len() < self.max_failures {
            return Vec::new();
        }

        // Check the last N diagnostics for identical content.
        let tail = &diagnostics[diagnostics.len() - self.max_failures..];
        let Some(first_key) = diagnostic_key(tail[0]) else {
            return Vec::new();
        };

        let all_same = tail[1..]
            .iter()
            .all(|s| diagnostic_key(s).as_deref() == Some(first_key.as_str()));

        if all_same {
            vec![
                Engram::builder(Kind::Custom("conductor.intervention".into()))
                    .body(Body::text(format!(
                        "{} consecutive identical compile failures: {}",
                        self.max_failures,
                        truncate(&first_key, 100)
                    )))
                    .tag("watcher", WATCHER_NAME)
                    .tag("severity", "warning")
                    .tag("consecutive", self.max_failures.to_string())
                    .build(),
            ]
        } else {
            Vec::new()
        }
    }

    fn name(&self) -> &str {
        WATCHER_NAME
    }
}

/// Truncate a string for display.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_owned()
    } else {
        let mut t = s[..max_len].to_owned();
        t.push_str("...");
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_error(msg: &str) -> Engram {
        Engram::builder(Kind::CompileDiagnostic)
            .body(Body::text(msg))
            .build()
    }

    #[test]
    fn empty_stream_no_fire() {
        let w = CompileFailRepeatWatcher::default();
        assert!(w.decide(&[], &Context::at(0)).is_empty());
    }

    #[test]
    fn different_errors_no_fire() {
        let w = CompileFailRepeatWatcher::default();
        let stream = vec![
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0433]: failed to resolve"),
            compile_error("error[E0599]: no method named `foo`"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn identical_errors_fires() {
        let w = CompileFailRepeatWatcher::default();
        let stream = vec![
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0308]: mismatched types"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag("watcher"), Some(WATCHER_NAME));
    }

    #[test]
    fn below_threshold_no_fire() {
        let w = CompileFailRepeatWatcher::default();
        let stream = vec![
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0308]: mismatched types"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn non_consecutive_at_end_no_fire() {
        let w = CompileFailRepeatWatcher::default();
        let stream = vec![
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0308]: mismatched types"),
            compile_error("error[E0433]: failed to resolve"),
        ];
        assert!(w.decide(&stream, &Context::at(0)).is_empty());
    }

    #[test]
    fn interleaved_non_compile_signals_filtered() {
        let w = CompileFailRepeatWatcher::default();
        let stream = vec![
            compile_error("same error"),
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("trying to fix..."))
                .build(),
            compile_error("same error"),
            Engram::builder(Kind::AgentOutput)
                .body(Body::text("trying again..."))
                .build(),
            compile_error("same error"),
        ];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1); // All 3 compile diagnostics are the same
    }

    #[test]
    fn custom_threshold() {
        let w = CompileFailRepeatWatcher::new(2);
        let stream = vec![compile_error("same"), compile_error("same")];
        let out = w.decide(&stream, &Context::at(0));
        assert_eq!(out.len(), 1);
    }
}
