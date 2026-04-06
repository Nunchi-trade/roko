//! One-shot mode — execute a single prompt and exit.
//!
//! Activated when a positional `[prompt]` argument is supplied on the command
//! line. The prompt text is dispatched through the universal loop exactly
//! once, then the process exits with an appropriate exit code.

/// One-shot execution context.
#[derive(Debug, Clone)]
pub struct OneshotMode {
    /// The prompt text to execute.
    pub prompt: String,
    /// Whether to emit JSON output instead of human-readable text.
    pub json_output: bool,
    /// Whether to suppress non-essential output.
    pub quiet: bool,
}

/// Result of a one-shot execution.
#[derive(Debug, Clone)]
pub struct OneshotResult {
    /// Whether the execution succeeded (agent + all gates passed).
    pub success: bool,
    /// Human-readable summary of the run.
    pub summary: String,
    /// Exit code to use: 0 = success, 1 = agent failure.
    pub exit_code: i32,
}

impl OneshotMode {
    /// Create a new one-shot execution context.
    #[must_use]
    pub const fn new(prompt: String) -> Self {
        Self {
            prompt,
            json_output: false,
            quiet: false,
        }
    }

    /// Enable JSON output mode.
    #[must_use]
    pub const fn with_json(mut self, json: bool) -> Self {
        self.json_output = json;
        self
    }

    /// Enable quiet mode (suppress non-essential output).
    #[must_use]
    pub const fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Execute the one-shot prompt. This builds a result but does not actually
    /// run the agent — the caller is responsible for wiring the prompt into
    /// the universal loop via `run_once`.
    #[must_use]
    pub fn prepare(&self) -> PreparedOneshot {
        PreparedOneshot {
            prompt: self.prompt.clone(),
            json_output: self.json_output,
            quiet: self.quiet,
        }
    }

    /// Format a result for display.
    #[must_use]
    pub fn format_result(&self, success: bool, details: &str) -> OneshotResult {
        let exit_code = i32::from(!success);
        let summary = if self.json_output {
            format!(
                r#"{{"success":{},"details":"{}"}}"#,
                success,
                details.replace('"', "\\\"")
            )
        } else if self.quiet {
            if success {
                String::new()
            } else {
                format!("error: {details}")
            }
        } else {
            format!(
                "{} {}",
                if success { "ok:" } else { "fail:" },
                details
            )
        };
        OneshotResult {
            success,
            summary,
            exit_code,
        }
    }
}

/// A prepared one-shot execution, ready to be dispatched.
#[derive(Debug, Clone)]
pub struct PreparedOneshot {
    /// The prompt to send to the agent.
    pub prompt: String,
    /// Whether output should be JSON.
    pub json_output: bool,
    /// Whether to suppress output.
    pub quiet: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_sets_prompt() {
        let mode = OneshotMode::new("fix the bug".into());
        assert_eq!(mode.prompt, "fix the bug");
        assert!(!mode.json_output);
        assert!(!mode.quiet);
    }

    #[test]
    fn with_json_enables_json() {
        let mode = OneshotMode::new("test".into()).with_json(true);
        assert!(mode.json_output);
    }

    #[test]
    fn with_quiet_enables_quiet() {
        let mode = OneshotMode::new("test".into()).with_quiet(true);
        assert!(mode.quiet);
    }

    #[test]
    fn format_result_success_default() {
        let mode = OneshotMode::new("test".into());
        let result = mode.format_result(true, "completed in 2s");
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.summary.contains("ok:"));
        assert!(result.summary.contains("completed in 2s"));
    }

    #[test]
    fn format_result_failure_default() {
        let mode = OneshotMode::new("test".into());
        let result = mode.format_result(false, "gate failed");
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
        assert!(result.summary.contains("fail:"));
    }

    #[test]
    fn format_result_json_mode() {
        let mode = OneshotMode::new("test".into()).with_json(true);
        let result = mode.format_result(true, "done");
        assert!(result.summary.contains(r#""success":true"#));
        assert!(result.summary.contains(r#""details":"done""#));
    }

    #[test]
    fn format_result_quiet_success() {
        let mode = OneshotMode::new("test".into()).with_quiet(true);
        let result = mode.format_result(true, "done");
        assert!(result.summary.is_empty());
    }

    #[test]
    fn format_result_quiet_failure() {
        let mode = OneshotMode::new("test".into()).with_quiet(true);
        let result = mode.format_result(false, "gate failed");
        assert!(result.summary.contains("error:"));
    }

    #[test]
    fn prepare_copies_fields() {
        let mode = OneshotMode::new("hello".into()).with_json(true).with_quiet(false);
        let prepared = mode.prepare();
        assert_eq!(prepared.prompt, "hello");
        assert!(prepared.json_output);
        assert!(!prepared.quiet);
    }
}
