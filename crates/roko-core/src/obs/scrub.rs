//! Log scrubbing middleware (items 43.15--43.16).
//!
//! [`LogScrubber`] redacts known secret patterns (API keys, tokens, auth
//! headers) from arbitrary text. Used by the tracing layer and tool-output
//! pipeline to prevent secrets from leaking into logs, traces, and signal
//! bodies.
//!
//! Built-in patterns cover the most common leak vectors:
//! - `sk-...` (Anthropic / `OpenAI` API keys)
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
}

impl ScrubPattern {
    fn new(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            regex: regex::Regex::new(pattern)?,
        })
    }

    fn scrub<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        self.regex.replace_all(text, REDACTED)
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
        // Anthropic / OpenAI API keys: sk-ant-..., sk-proj-..., sk-... (20+ chars)
        r"sk-[A-Za-z0-9_-]{20,}",
        // Slack bot tokens.
        r"xoxb-[A-Za-z0-9-]{10,}",
        // GitHub personal access tokens (classic & fine-grained)
        r"gh[pousr]_[A-Za-z0-9_]{16,}",
        // Bearer tokens in authorization headers (value after "Bearer ")
        r"(?i)Bearer\s+[A-Za-z0-9_.+/=-]{10,}",
        // Env-var leak: ANTHROPIC_API_KEY=<value>
        r"ANTHROPIC_API_KEY=[^\s]+",
        // Env-var leak: OPENAI_API_KEY=<value>
        r"OPENAI_API_KEY=[^\s]+",
    ];
    raw.iter()
        .map(|p| {
            ScrubPattern::new(p)
                .unwrap_or_else(|e| panic!("built-in scrub pattern {p:?} failed to compile: {e}"))
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
        assert!(output.contains(REDACTED));
        assert!(output.contains("Using key"));
        assert!(output.contains("for request"));
    }

    #[test]
    fn scrubs_openai_api_key() {
        let scrubber = LogScrubber::new();
        let input = "key=sk-proj-abcdefghijklmnopqrstuvwxyz";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-proj-"));
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn scrubs_github_token() {
        let scrubber = LogScrubber::new();
        let input = "Authorization: token ghp_ABCDEFGHIJKLMNOPqrstuvwxyz1234567890";
        let output = scrubber.scrub(input);
        assert!(!output.contains("ghp_"));
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn scrubs_slack_bot_token() {
        let scrubber = LogScrubber::new();
        let input = "Authorization: Bearer xoxb-1234567890-abcdefghijklmnopqrstuv";
        let output = scrubber.scrub(input);
        assert!(!output.contains("xoxb-"));
        assert!(output.contains(REDACTED));
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
        assert!(output.contains(REDACTED));
    }

    #[test]
    fn scrubs_openai_env_var() {
        let scrubber = LogScrubber::new();
        let input = "OPENAI_API_KEY=sk-proj-myverysecretkey12345678";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-proj-"));
        assert!(output.contains(REDACTED));
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
            "keys: sk-ant-abcdefghijklmnopqrstuvwxyz and ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234";
        let output = scrubber.scrub(input);
        assert!(!output.contains("sk-ant-"));
        assert!(!output.contains("ghp_"));
        // Should have two REDACTED markers
        assert_eq!(output.matches(REDACTED).count(), 2);
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
            scrubber.pattern_count() >= 5,
            "should have at least 5 built-in patterns"
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
    fn short_sk_prefix_not_scrubbed() {
        // "sk-short" has less than 20 chars after "sk-", should NOT be scrubbed
        let scrubber = LogScrubber::new();
        let input = "key sk-short should stay";
        let output = scrubber.scrub(input);
        assert_eq!(output, input);
    }
}
