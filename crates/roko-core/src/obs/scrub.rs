//! Log scrubbing middleware (items 43.15--43.16).
//!
//! [`LogScrubber`] redacts known secret patterns (API keys, tokens, auth
//! headers) from arbitrary text. Used by the tracing layer and tool-output
//! pipeline to prevent secrets from leaking into logs, traces, and signal
//! bodies.
//!
//! Built-in patterns cover the most common leak vectors:
//! - `sk-...` (`API_KEY`)
//! - `ghp_...` / `gho_...` / `ghs_...` / `ghu_...` / `ghr_...` (`GitHub` tokens)
//! - `xoxb-...` (`Slack` bot tokens)
//! - `Bearer ...` (Authorization headers)
//! - `ANTHROPIC_API_KEY=...` / `OPENAI_API_KEY=...` (env-var leaks)
//!
//! Custom patterns can be added at runtime via [`LogScrubber::add_pattern`].

use parking_lot::RwLock;

/// Replacement text inserted in place of scrubbed secrets.
pub const REDACTED: &str = "[REDACTED]";

/// A compiled regex pattern used by the scrubber.
struct ScrubPattern {
    regex: regex::Regex,
    replacement: String,
}

impl ScrubPattern {
    fn new(pattern: &str) -> Result<Self, regex::Error> {
        Self::with_replacement(pattern, REDACTED)
    }

    fn with_replacement(
        pattern: &str,
        replacement: impl Into<String>,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: regex::Regex::new(pattern)?,
            replacement: replacement.into(),
        })
    }

    fn scrub<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        self.regex.replace_all(text, self.replacement.as_str())
    }
}

/// Redacts known secret patterns from log/trace output.
///
/// Thread-safe: custom patterns can be added from any thread via
/// [`LogScrubber::add_pattern`].
pub struct LogScrubber {
    patterns: RwLock<Vec<ScrubPattern>>,
}

impl std::fmt::Debug for LogScrubber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let len = self.patterns.read().len();
        f.debug_struct("LogScrubber")
            .field("pattern_count", &len)
            .finish_non_exhaustive()
    }
}

/// Built-in patterns covering common secret formats.
fn builtin_patterns() -> Vec<ScrubPattern> {
    // Each pattern captures the secret portion and replaces the whole match.
    let raw = [
        // GitHub personal access tokens.
        (r"ghp_[A-Za-z0-9]{36}", "[REDACTED:GITHUB_PAT]"),
        // API keys.
        (r"sk-[A-Za-z0-9-]+", "[REDACTED:API_KEY]"),
        // Slack bot tokens.
        (r"xoxb-[0-9]+-[A-Za-z0-9]+", "[REDACTED:SLACK_BOT_TOKEN]"),
        // Anthropic / OpenAI API keys: sk-ant-..., sk-proj-..., sk-... (20+ chars)
        (r"sk-[A-Za-z0-9_-]{20,}", REDACTED),
        // GitHub tokens beyond the PAT shape above.
        (r"gh[pousr]_[A-Za-z0-9_]{16,}", REDACTED),
        // Bearer tokens in authorization headers (value after "Bearer ")
        (r"(?i)Bearer\s+[A-Za-z0-9_.+/=-]{10,}", REDACTED),
        // Env-var leak: ANTHROPIC_API_KEY=<value>
        (r"ANTHROPIC_API_KEY=[^\s]+", REDACTED),
        // Env-var leak: OPENAI_API_KEY=<value>
        (r"OPENAI_API_KEY=[^\s]+", REDACTED),
    ];
    raw.iter()
        .map(|p| {
            ScrubPattern::with_replacement(p.0, p.1).unwrap_or_else(|e| {
                panic!("built-in scrub pattern {:?} failed to compile: {e}", p.0)
            })
        })
        .collect()
}

impl LogScrubber {
    /// Create a scrubber pre-loaded with built-in patterns.
    #[must_use]
    pub fn new() -> Self {
        Self {
            patterns: RwLock::new(builtin_patterns()),
        }
    }

    /// Create a scrubber with no patterns (useful for tests that add custom
    /// patterns only).
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            patterns: RwLock::new(Vec::new()),
        }
    }

    /// Add a custom regex pattern. Returns an error if the pattern is invalid.
    ///
    /// # Errors
    ///
    /// Returns the underlying `regex::Error` if the pattern fails to compile.
    pub fn add_pattern(&self, pattern: &str) -> Result<(), regex::Error> {
        let compiled = ScrubPattern::new(pattern)?;
        self.patterns.write().push(compiled);
        Ok(())
    }

    /// Add a custom regex pattern with a specific replacement string.
    ///
    /// This is used for `.env` values, where the replacement must identify the
    /// loaded variable name.
    pub fn add_pattern_with_replacement(
        &self,
        pattern: &str,
        replacement: impl Into<String>,
    ) -> Result<(), regex::Error> {
        let compiled = ScrubPattern::with_replacement(pattern, replacement)?;
        self.patterns.write().push(compiled);
        Ok(())
    }

    /// Add a literal value to the scrubber, redacting it as `name`.
    pub fn add_literal_value(&self, value: &str, name: &str) -> Result<(), regex::Error> {
        if value.is_empty() {
            return Ok(());
        }
        let pattern = regex::escape(value);
        let replacement = format!("[REDACTED:{name}]");
        self.add_pattern_with_replacement(&pattern, replacement)
    }

    /// Scrub all known patterns from the input text, replacing matches with
    /// [`REDACTED`].
    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        let patterns = self.patterns.read();
        let mut result = text.to_string();
        for pattern in patterns.iter() {
            let scrubbed = pattern.scrub(&result);
            if let std::borrow::Cow::Owned(s) = scrubbed {
                result = s;
            }
        }
        drop(patterns);
        result
    }

    /// Number of patterns currently registered.
    #[must_use]
    pub fn pattern_count(&self) -> usize {
        self.patterns.read().len()
    }
}

impl Default for LogScrubber {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_anthropic_api_key() {
        let scrubber = LogScrubber::new();
        let input = "Using key sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890 for request";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-ant-api03"));
        assert!(output.contains("[REDACTED:API_KEY]"));
        assert!(output.contains("Using key"));
        assert!(output.contains("for request"));
    }

    #[test]
    fn scrubs_openai_api_key() {
        let scrubber = LogScrubber::new();
        let input = "key=sk-proj-abcdefghijklmnopqrstuvwxyz";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-proj-"));
        assert!(output.contains("[REDACTED:API_KEY]"));
    }

    #[test]
    fn scrubs_github_token() {
        let scrubber = LogScrubber::new();
        let input = "Authorization: token ghp_ABCDEFGHIJKLMNOPqrstuvwxyz1234567890";
        let output = scrubber.scrub(input);
        assert!(!output.contains("ghp_"));
        assert!(output.contains("[REDACTED:GITHUB_PAT]"));
    }

    #[test]
    fn scrubs_slack_bot_token() {
        let scrubber = LogScrubber::new();
        let input = "Authorization: xoxb-1234567890-abcdefghijklmnopqrstuv";
        let output = scrubber.scrub(input);
        assert!(!output.contains("xoxb-"));
        assert!(output.contains("[REDACTED:SLACK_BOT_TOKEN]"));
    }

    #[test]
    fn scrubs_bearer_token() {
        let scrubber = LogScrubber::new();
        let input = "Header: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test.signature";
        let output = scrubber.scrub(input);
        assert!(!output.contains("eyJhbGciOi"));
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn scrubs_anthropic_env_var() {
        let scrubber = LogScrubber::new();
        let input = "export ANTHROPIC_API_KEY=sk-ant-secret-key-12345";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-ant-secret-key"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn scrubs_openai_env_var() {
        let scrubber = LogScrubber::new();
        let input = "OPENAI_API_KEY=sk-proj-myverysecretkey12345678";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-proj-"));
        assert!(output.contains("[REDACTED]"));
    }

    #[test]
    fn leaves_clean_text_unchanged() {
        let scrubber = LogScrubber::new();
        let input = "Just a normal log line with no secrets at all.";
        let output = scrubber.scrub(input);
        assert_eq!(output, input);
    }

    #[test]
    fn scrubs_multiple_secrets_in_one_line() {
        let scrubber = LogScrubber::new();
        let input =
            "keys: sk-ant-abcdefghijklmnopqrstuvwxyz and ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-ant-"));
        assert!(!output.contains("ghp_"));
        assert_eq!(
            output.matches("[REDACTED:API_KEY]").count()
                + output.matches("[REDACTED:GITHUB_PAT]").count(),
            2
        );
    }

    #[test]
    fn custom_pattern_works() {
        let scrubber = LogScrubber::empty();
        scrubber.add_pattern(r"my-secret-\d+").unwrap();
        let input = "found my-secret-42 in config";
        let output = scrubber.scrub(input);
        assert!(!output.contains("my-secret-42"));
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn invalid_custom_pattern_returns_error() {
        let scrubber = LogScrubber::empty();
        let result = scrubber.add_pattern(r"[invalid");
        assert!(result.is_err());
    }

    #[test]
    fn pattern_count_tracks_additions() {
        let scrubber = LogScrubber::empty();
        assert_eq!(scrubber.pattern_count(), 0);
        scrubber.add_pattern(r"foo").unwrap();
        assert_eq!(scrubber.pattern_count(), 1);
        scrubber.add_pattern(r"bar").unwrap();
        assert_eq!(scrubber.pattern_count(), 2);
    }

    #[test]
    fn default_has_builtin_patterns() {
        let scrubber = LogScrubber::default();
        assert!(
            scrubber.pattern_count() >= 8,
            "should have at least 8 built-in patterns"
        );
    }

    #[test]
    fn scrubs_github_org_token() {
        let scrubber = LogScrubber::new();
        let input = "org token: gho_ABCDEFGHIJKLMNOPqrstuvwx";
        let output = scrubber.scrub(input);
        assert!(!output.contains("gho_"));
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn bearer_case_insensitive() {
        let scrubber = LogScrubber::new();
        let input = "header: bearer abcdefghijklmnopqrstuvwxyz12";
        let output = scrubber.scrub(input);
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn empty_string_stays_empty() {
        let scrubber = LogScrubber::new();
        assert_eq!(scrubber.scrub(""), "");
    }

    #[test]
    fn short_sk_prefix_is_scrubbed() {
        let scrubber = LogScrubber::new();
        let input = "key sk-short should stay";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-short"));
        assert!(output.contains("[REDACTED:API_KEY]"));
    }

    #[test]
    fn literal_value_uses_named_redaction() {
        let scrubber = LogScrubber::empty();
        scrubber
            .add_literal_value("super-secret-value", "TEST_ENV")
            .unwrap();
        let output = scrubber.scrub("value super-secret-value leaked");
        assert!(output.contains("[REDACTED:TEST_ENV]"));
        assert!(!output.contains("super-secret-value"));
    }
}
