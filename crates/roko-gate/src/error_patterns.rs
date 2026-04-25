//! Gate failure pattern extraction and normalized keys.
//!
//! This module keeps gate failure identity separate from raw logs. Compile
//! diagnostics are keyed by error code plus file path, while fallback text
//! failures use a compact digest so prompt context stays bounded.

use std::collections::HashSet;

use crate::compile_errors::{CompileError, GateFailureClassification};
use crate::review_verdict::ParsedReviewVerdict;

const MAX_DIGEST_ERRORS: usize = 10;
const MAX_DIGEST_CHARS: usize = 200;

/// A normalized failure pattern candidate emitted by a gate parser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailurePatternRecord {
    /// Stable key used for de-duplication.
    pub key: String,
    /// Gate that emitted the failure.
    pub gate: String,
    /// Coarse failure class or category.
    pub classification: String,
    /// Compact human-readable signature.
    pub digest: String,
    /// Optional machine or reviewer suggestion.
    pub suggestion: Option<String>,
}

/// Return the compile-diagnostic failure key from error code plus file path.
///
/// Line and column numbers are intentionally ignored: two `E0425` errors in
/// the same file usually share a root cause, while the same code in a
/// different file may require a different fix.
#[must_use]
pub fn error_key(error: &CompileError) -> String {
    let code = error
        .code
        .as_deref()
        .filter(|code| !code.trim().is_empty())
        .unwrap_or_else(|| error_category_label(error));
    let file = error
        .file
        .as_deref()
        .map(normalize_file_path)
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    format!("{code}::{file}")
}

/// Build failure records from a structured gate classification.
#[must_use]
pub fn records_from_classification(
    classification: &GateFailureClassification,
) -> Vec<FailurePatternRecord> {
    if !classification.compile_errors.is_empty() {
        return classification
            .compile_errors
            .iter()
            .map(|error| FailurePatternRecord {
                key: error_key(error),
                gate: classification.gate.clone(),
                classification: format!("{:?}", error.category).to_ascii_lowercase(),
                digest: compile_error_digest(error),
                suggestion: error.suggestion.clone(),
            })
            .collect();
    }

    let digest = extract_error_digest(&classification.raw_excerpt);
    let class = format!("{:?}", classification.primary).to_ascii_lowercase();
    vec![FailurePatternRecord {
        key: format!("{}::{class}::{digest}", classification.gate),
        gate: classification.gate.clone(),
        classification: class,
        digest,
        suggestion: None,
    }]
}

/// Build failure records from a parsed reviewer verdict.
#[must_use]
pub fn records_from_parsed_review_verdict(
    parsed: &ParsedReviewVerdict,
) -> Vec<FailurePatternRecord> {
    parsed
        .evidence
        .blocking_findings
        .iter()
        .map(|finding| {
            let digest = truncate_chars(&collapse_whitespace(finding), MAX_DIGEST_CHARS);
            let status = format!("{:?}", parsed.evidence.status).to_ascii_lowercase();
            FailurePatternRecord {
                key: format!(
                    "review::{}::{}::{digest}",
                    parsed.evidence.reviewer_role_id, status
                ),
                gate: format!("review:{}", parsed.evidence.reviewer_role_id),
                classification: status,
                digest,
                suggestion: Some(format!("{:?}", parsed.evidence.required_next_action)),
            }
        })
        .collect()
}

/// Extract a compact, de-duplicated digest from cargo/test output.
///
/// The result contains at most ten unique error signatures and caps each
/// signature at 200 characters. Large logs are never copied wholesale.
#[must_use]
pub fn extract_error_digest(output: &str) -> String {
    let mut seen = HashSet::new();
    let mut digests = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if !is_error_signature(trimmed) {
            continue;
        }
        let digest = truncate_chars(&collapse_whitespace(trimmed), MAX_DIGEST_CHARS);
        if seen.insert(digest.clone()) {
            digests.push(digest);
        }
        if digests.len() >= MAX_DIGEST_ERRORS {
            break;
        }
    }

    if digests.is_empty() {
        output
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| truncate_chars(&collapse_whitespace(line.trim()), MAX_DIGEST_CHARS))
            .unwrap_or_default()
    } else {
        digests.join("\n")
    }
}

fn compile_error_digest(error: &CompileError) -> String {
    let code = error.code.as_deref().unwrap_or(error_category_label(error));
    let mut digest = format!("{code}: {}", collapse_whitespace(&error.message));
    if let Some(file) = error.file.as_deref() {
        digest.push_str(" [");
        digest.push_str(&normalize_file_path(file));
        digest.push(']');
    }
    truncate_chars(&digest, MAX_DIGEST_CHARS)
}

fn error_category_label(error: &CompileError) -> &'static str {
    match error.category {
        crate::compile_errors::ErrorCategory::Syntax => "syntax",
        crate::compile_errors::ErrorCategory::UnresolvedImport => "unresolved_import",
        crate::compile_errors::ErrorCategory::TypeMismatch => "type_mismatch",
        crate::compile_errors::ErrorCategory::Lifetime => "lifetime",
        crate::compile_errors::ErrorCategory::MissingMember => "missing_member",
        crate::compile_errors::ErrorCategory::Unused => "unused",
        crate::compile_errors::ErrorCategory::Visibility => "visibility",
        crate::compile_errors::ErrorCategory::Macro => "macro",
        crate::compile_errors::ErrorCategory::TraitBound => "trait_bound",
        crate::compile_errors::ErrorCategory::Ownership => "ownership",
        crate::compile_errors::ErrorCategory::Other => "other",
    }
}

fn is_error_signature(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("error[")
        || lower.starts_with("error:")
        || lower.contains("test result: failed")
        || lower.contains("panicked at")
        || lower.contains("assertion failed")
}

fn normalize_file_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .split(':')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile_errors::{classify_gate_failure, parse_cargo_json};

    #[test]
    fn error_key_ignores_line_but_keeps_file() {
        let stderr_a = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/a.rs","line_start":10,"column_start":3,"is_primary":true}],"children":[]}}"#;
        let stderr_b = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/a.rs","line_start":99,"column_start":7,"is_primary":true}],"children":[]}}"#;
        let stderr_c = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/b.rs","line_start":10,"column_start":3,"is_primary":true}],"children":[]}}"#;

        let mut summary_a = parse_cargo_json(stderr_a);
        let mut summary_b = parse_cargo_json(stderr_b);
        let mut summary_c = parse_cargo_json(stderr_c);
        let a = summary_a.errors.remove(0);
        let b = summary_b.errors.remove(0);
        let c = summary_c.errors.remove(0);

        assert_eq!(error_key(&a), error_key(&b));
        assert_ne!(error_key(&a), error_key(&c));
        assert_eq!(error_key(&a), "E0425::src/a.rs");
    }

    #[test]
    fn extract_error_digest_dedupes_and_bounds() {
        let mut output = String::new();
        for i in 0..20 {
            output.push_str(&format!("error[E04{i:02}]: {}\n", "x".repeat(260)));
        }
        output.push_str("error[E0400]: duplicate\n");

        let digest = extract_error_digest(&output);
        let lines: Vec<_> = digest.lines().collect();
        assert_eq!(lines.len(), 10);
        assert!(lines.iter().all(|line| line.chars().count() <= 200));
    }

    #[test]
    fn records_from_classification_use_compile_keys() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/main.rs","line_start":1,"column_start":1,"is_primary":true}],"children":[{"message":"import foo","level":"help"}]}}"#;
        let classification = classify_gate_failure("compile:cargo", json_line);
        let records = records_from_classification(&classification);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "E0425::src/main.rs");
        assert_eq!(records[0].gate, "compile:cargo");
        assert_eq!(records[0].suggestion.as_deref(), Some("import foo"));
    }
}
