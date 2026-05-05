//! Secret scrubbing from tool output (§36.50).
//!
//! Runs over `ToolResult::Ok { content, .. }` before the content is
//! handed to the LLM, replacing secrets with a fixed redaction marker.
//! The real impl ships a default pattern set covering:
//!
//! - API keys: `sk-*`, `sk-ant-*`, `sk-proj-*`, `AKIA…`, `ghp_*`,
//!   `ghs_*`, `glpat-*`, etc.
//! - JWTs: `eyJ…` triple-segment tokens.
//! - Private keys: `-----BEGIN * PRIVATE KEY-----` blocks.
//! - `.env`-style `SECRET=…`, `TOKEN=…`, `PASSWORD=…` lines.
//!
//! The scrubber is **pure**: it allocates a new `String` and never
//! mutates shared state. Callers apply it per-result in the dispatcher.

use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};

// ─── Public constants ──────────────────────────────────────────────────────

/// Marker used to replace detected secrets.
pub const SCRUB_MARKER: &str = "[REDACTED]";

// ─── Default pattern set ───────────────────────────────────────────────────

/// A compiled regex paired with whether the whole match (`None`) or a
/// specific capture group (1-indexed `Some(n)`) should be replaced.
struct Pattern {
    re: Regex,
    /// `None` → replace the whole match. `Some(n)` → replace group `n`
    /// only (leaving the rest of the match in-place).
    replace_group: Option<usize>,
}

/// Lazy-initialised default pattern set. Compiled exactly once per process.
static DEFAULT_PATTERNS: OnceLock<Vec<Pattern>> = OnceLock::new();

/// Return the default pattern list, compiling on first call.
///
/// # Panics
///
/// Panics if any of the hard-coded default regex strings fail to compile
/// (this is a programming error and will surface immediately in tests).
#[allow(clippy::expect_used)] // compile-time constants; compile failures are bugs, not user errors
fn default_patterns() -> &'static Vec<Pattern> {
    DEFAULT_PATTERNS.get_or_init(|| {
        vec![
            // 1. Anthropic API keys
            Pattern {
                re: Regex::new(r"\bsk-ant-api\d{2}-[A-Za-z0-9_-]{80,}\b")
                    .expect("Anthropic key regex compiles"),
                replace_group: None,
            },
            // 2. OpenAI API keys (sk-proj-… and bare sk-…)
            Pattern {
                re: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b")
                    .expect("OpenAI key regex compiles"),
                replace_group: None,
            },
            // 3a. AWS access key AKIA
            Pattern {
                re: Regex::new(r"\bAKIA[0-9A-Z]{16}\b")
                    .expect("AWS AKIA regex compiles"),
                replace_group: None,
            },
            // 3b. AWS access key ASIA (STS temporary)
            Pattern {
                re: Regex::new(r"\bASIA[0-9A-Z]{16}\b")
                    .expect("AWS ASIA regex compiles"),
                replace_group: None,
            },
            // 4a. GitHub PAT ghp_
            Pattern {
                re: Regex::new(r"\bghp_[A-Za-z0-9]{36}\b")
                    .expect("GitHub ghp_ regex compiles"),
                replace_group: None,
            },
            // 4b. GitHub PAT ghs_
            Pattern {
                re: Regex::new(r"\bghs_[A-Za-z0-9]{36}\b")
                    .expect("GitHub ghs_ regex compiles"),
                replace_group: None,
            },
            // 4c. GitHub PAT gho_
            Pattern {
                re: Regex::new(r"\bgho_[A-Za-z0-9]{36}\b")
                    .expect("GitHub gho_ regex compiles"),
                replace_group: None,
            },
            // 4d. GitHub PAT ghu_
            Pattern {
                re: Regex::new(r"\bghu_[A-Za-z0-9]{36}\b")
                    .expect("GitHub ghu_ regex compiles"),
                replace_group: None,
            },
            // 4e. GitHub PAT ghr_
            Pattern {
                re: Regex::new(r"\bghr_[A-Za-z0-9]{36}\b")
                    .expect("GitHub ghr_ regex compiles"),
                replace_group: None,
            },
            // 5. GitLab PAT
            Pattern {
                re: Regex::new(r"\bglpat-[A-Za-z0-9_-]{20,}\b")
                    .expect("GitLab PAT regex compiles"),
                replace_group: None,
            },
            // 6. Slack tokens
            Pattern {
                re: Regex::new(r"\bxox[abpsr]-[A-Za-z0-9-]{10,}\b")
                    .expect("Slack token regex compiles"),
                replace_group: None,
            },
            // 7. JWTs (three base64url segments starting with eyJ)
            Pattern {
                re: Regex::new(r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b")
                    .expect("JWT regex compiles"),
                replace_group: None,
            },
            // 8. Generic private key blocks (multiline)
            Pattern {
                re: RegexBuilder::new(
                    r"-----BEGIN (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----[\s\S]*?-----END (?:RSA |EC |DSA |OPENSSH |PGP )?PRIVATE KEY-----",
                )
                .multi_line(true)
                .dot_matches_new_line(true)
                .build()
                .expect("private key block regex compiles"),
                replace_group: None,
            },
            // 9. Env-file assignments for high-risk keys (replace value only)
            // Group 1: key  Group 2: value
            Pattern {
                re: RegexBuilder::new(
                    r"^\s*(PASSWORD|SECRET|TOKEN|API_KEY|APIKEY|PRIVATE_KEY|DATABASE_URL)\s*=\s*(\S+)",
                )
                .case_insensitive(true)
                .multi_line(true)
                .build()
                .expect("env assignment regex compiles"),
                replace_group: Some(2),
            },
        ]
    })
}

// ─── React ────────────────────────────────────────────────────────────────

/// Rules describing which patterns to scrub.
#[derive(Debug, Clone, Default)]
pub struct ScrubPolicy {
    /// Additional regex patterns the caller wants to treat as secrets.
    /// Each is compiled per-call; invalid patterns are silently skipped.
    pub extra_patterns: Vec<String>,
    /// When `true`, the default pattern set (patterns 1-9) is skipped.
    /// Only `extra_patterns` are applied. Rarely used; primarily for tests.
    pub disable_defaults: bool,
}

// ─── Core function ─────────────────────────────────────────────────────────

/// Scrub secrets from a content string, returning a new `String`.
///
/// Applies every enabled pattern in order, replacing matches with
/// [`SCRUB_MARKER`]. For env-file assignment patterns, only the VALUE
/// portion is replaced so that `KEY=[REDACTED]` remains readable.
///
/// If no pattern matches, a fresh allocation of `content` is returned.
#[must_use]
pub fn scrub_secrets(content: &str, policy: &ScrubPolicy) -> String {
    let mut result = content.to_string();

    if !policy.disable_defaults {
        for pattern in default_patterns() {
            result = apply_pattern(&result, pattern);
        }
    }

    for raw in &policy.extra_patterns {
        let Ok(re) = Regex::new(raw) else {
            // Silently skip invalid user-supplied patterns.
            continue;
        };
        let extra = Pattern {
            re,
            replace_group: None,
        };
        result = apply_pattern(&result, &extra);
    }

    result
}

/// Apply a single `Pattern` to `text`, returning the result.
fn apply_pattern(text: &str, pattern: &Pattern) -> String {
    let Some(group) = pattern.replace_group else {
        // No target group — replace the whole match.
        return pattern.re.replace_all(text, SCRUB_MARKER).into_owned();
    };

    // Replace only the specified capture group, leaving everything
    // else in the match verbatim (e.g. `KEY=VALUE` → `KEY=[REDACTED]`).
    let mut out = String::with_capacity(text.len());
    let mut last = 0usize;
    for caps in pattern.re.captures_iter(text) {
        // Group 0 (whole match) is always present for a successful match.
        let Some(whole) = caps.get(0) else { continue };
        out.push_str(&text[last..whole.start()]);

        // Group `group` must exist for our hard-coded patterns; skip if not.
        let Some(group_match) = caps.get(group) else {
            out.push_str(&text[whole.start()..whole.end()]);
            last = whole.end();
            continue;
        };

        // text between start of whole match and start of group
        out.push_str(&text[whole.start()..group_match.start()]);
        out.push_str(SCRUB_MARKER);
        // text between end of group and end of whole match
        out.push_str(&text[group_match.end()..whole.end()]);

        last = whole.end();
    }
    out.push_str(&text[last..]);
    out
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> ScrubPolicy {
        ScrubPolicy::default()
    }

    // ── 1. Anthropic API key ────────────────────────────────────────────────

    #[test]
    fn redacts_anthropic_key() {
        let key = "sk-ant-api03-".to_string() + &"A".repeat(80);
        let content = format!("use {key} to call Claude");
        let result = scrub_secrets(&content, &default_policy());
        assert!(
            result.contains(SCRUB_MARKER),
            "expected SCRUB_MARKER in: {result}"
        );
        assert!(
            !result.contains(&key),
            "key must not appear in output: {result}"
        );
    }

    // ── 2. OpenAI sk-proj- key ─────────────────────────────────────────────

    #[test]
    fn redacts_openai_key_sk_proj() {
        let key = "sk-proj-".to_string() + &"b".repeat(20);
        let content = format!("Authorization: Bearer {key}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&key), "key must not appear: {result}");
    }

    // ── 3. OpenAI bare sk- key ─────────────────────────────────────────────

    #[test]
    fn redacts_openai_key_bare_sk() {
        let key = "sk-".to_string() + &"c".repeat(20);
        let content = format!("key={key}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&key), "key must not appear: {result}");
    }

    // ── 4. AWS AKIA access key ─────────────────────────────────────────────

    #[test]
    fn redacts_aws_access_key_akia() {
        let key = "AKIA".to_string() + &"A".repeat(16);
        let content = format!("aws_access_key_id={key}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&key), "key must not appear: {result}");
    }

    // ── 5. GitHub PAT ghp_ ────────────────────────────────────────────────

    #[test]
    fn redacts_github_pat_ghp() {
        let pat = "ghp_".to_string() + &"G".repeat(36);
        let content = format!("GITHUB_TOKEN={pat}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&pat), "pat must not appear: {result}");
    }

    // ── 6. GitLab PAT ─────────────────────────────────────────────────────

    #[test]
    fn redacts_gitlab_pat() {
        let pat = "glpat-".to_string() + &"x".repeat(20);
        let content = format!("CI_JOB_TOKEN={pat}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&pat), "pat must not appear: {result}");
    }

    // ── 7. Slack token ────────────────────────────────────────────────────

    #[test]
    fn redacts_slack_token() {
        let tok = "xoxb-".to_string() + &"1".repeat(10);
        let content = format!("slack token is {tok} use it");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(&tok), "token must not appear: {result}");
    }

    // ── 8. JWT ────────────────────────────────────────────────────────────

    #[test]
    fn redacts_jwt() {
        // Three base64url segments, first two starting with eyJ (base64 of `{`)
        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ1c2VyIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let content = format!("Bearer {jwt}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains(jwt), "jwt must not appear: {result}");
    }

    // ── 9. RSA private key block ───────────────────────────────────────────

    #[test]
    fn redacts_private_key_block() {
        let content = "key follows:\n-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA0Z3V\n-----END RSA PRIVATE KEY-----\ndone";
        let result = scrub_secrets(&content, &default_policy());
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(
            !result.contains("BEGIN RSA PRIVATE KEY"),
            "key block must not appear: {result}"
        );
    }

    // ── 10. Env-file PASSWORD line (value replaced, key kept) ──────────────

    #[test]
    fn redacts_env_file_password_line() {
        let content = "PASSWORD=hunter2";
        let result = scrub_secrets(&content, &default_policy());
        assert!(
            result.contains("PASSWORD="),
            "KEY= prefix must be preserved: {result}"
        );
        assert!(
            result.contains(SCRUB_MARKER),
            "value must be redacted: {result}"
        );
        assert!(
            !result.contains("hunter2"),
            "plaintext value must not appear: {result}"
        );
    }

    // ── 11. Env-file — case insensitive ────────────────────────────────────

    #[test]
    fn redacts_env_file_case_insensitive() {
        let content = "password=s3cr3t\nSECRET=abc123";
        let result = scrub_secrets(&content, &default_policy());
        assert!(!result.contains("s3cr3t"), "result: {result}");
        assert!(!result.contains("abc123"), "result: {result}");
        assert_eq!(
            result.matches(SCRUB_MARKER).count(),
            2,
            "expected 2 redactions: {result}"
        );
    }

    // ── 12. Multiple secrets in one string ────────────────────────────────

    #[test]
    fn multiple_secrets_in_one_string_all_redacted() {
        let gh_pat = "ghp_".to_string() + &"H".repeat(36);
        let aws = "AKIA".to_string() + &"B".repeat(16);
        let content = format!("github={gh_pat} aws={aws}");
        let result = scrub_secrets(&content, &default_policy());
        assert!(
            !result.contains(&gh_pat),
            "github pat must be gone: {result}"
        );
        assert!(!result.contains(&aws), "aws key must be gone: {result}");
        assert!(
            result.matches(SCRUB_MARKER).count() >= 2,
            "expected at least 2 redactions: {result}"
        );
    }

    // ── 13. Non-secret content passes through ─────────────────────────────

    #[test]
    fn non_secret_content_passes_through_unchanged() {
        let content = "the quick brown fox jumps over the lazy dog";
        let result = scrub_secrets(content, &default_policy());
        assert_eq!(result, content, "clean content must be unchanged");
    }

    // ── 14. disable_defaults skips all default patterns ───────────────────

    #[test]
    fn disable_defaults_flag_skips_all_patterns() {
        let key = "AKIA".to_string() + &"X".repeat(16);
        let content = format!("key={key}");
        let policy = ScrubPolicy {
            disable_defaults: true,
            extra_patterns: Vec::new(),
        };
        let result = scrub_secrets(&content, &policy);
        // With defaults disabled and no extras, key must survive.
        assert!(
            result.contains(&key),
            "key should survive when defaults disabled: {result}"
        );
    }

    // ── 15. Extra patterns are applied ────────────────────────────────────

    #[test]
    fn extra_patterns_are_applied() {
        let policy = ScrubPolicy {
            disable_defaults: true,
            extra_patterns: vec![r"CUSTOM-\d+".to_string()],
        };
        let content = "value is CUSTOM-12345 here";
        let result = scrub_secrets(content, &policy);
        assert!(result.contains(SCRUB_MARKER), "result: {result}");
        assert!(!result.contains("CUSTOM-12345"), "result: {result}");
    }

    // ── 16. Invalid extra pattern does not crash ──────────────────────────

    #[test]
    fn invalid_extra_pattern_does_not_crash() {
        let policy = ScrubPolicy {
            disable_defaults: false,
            extra_patterns: vec!["[".to_string()], // malformed regex
        };
        // Must not panic.
        let result = scrub_secrets("hello world", &policy);
        // The malformed pattern is skipped; clean content passes through.
        assert_eq!(result, "hello world");
    }

    // ── 17. Default policy has defaults enabled ───────────────────────────

    #[test]
    fn default_policy_has_defaults_enabled() {
        let p = ScrubPolicy::default();
        assert!(
            !p.disable_defaults,
            "disable_defaults must be false by default"
        );
        assert!(
            p.extra_patterns.is_empty(),
            "extra_patterns must be empty by default"
        );
    }
}
