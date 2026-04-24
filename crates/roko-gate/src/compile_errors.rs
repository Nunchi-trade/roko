//! Structured compile error classification.
//!
//! Parses `cargo check --message-format=json` output and classifies errors
//! into categories that agents can act on programmatically.

use serde::{Deserialize, Serialize};

/// Category of compile error.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// Syntax error — malformed Rust code.
    Syntax,
    /// Unresolved import or symbol.
    UnresolvedImport,
    /// Type mismatch (expected X, found Y).
    TypeMismatch,
    /// Lifetime or borrow checker violation.
    Lifetime,
    /// Missing field, method, or trait implementation.
    MissingMember,
    /// Unused variable, import, or function.
    Unused,
    /// Visibility or access violation.
    Visibility,
    /// Macro expansion error.
    Macro,
    /// Trait bound not satisfied.
    TraitBound,
    /// Move/ownership error.
    Ownership,
    /// Other / unclassified error.
    Other,
}

/// A single structured compile error.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompileError {
    /// Error category.
    pub category: ErrorCategory,
    /// Rustc error code (e.g. "E0433", "E0308").
    pub code: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// File path (relative to workspace root).
    pub file: Option<String>,
    /// Line number.
    pub line: Option<u32>,
    /// Column number.
    pub column: Option<u32>,
    /// Rustc-suggested fix, if available.
    pub suggestion: Option<String>,
}

/// Summary of all compile errors from a build.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompileErrorSummary {
    /// Total error count.
    pub error_count: usize,
    /// Total warning count.
    pub warning_count: usize,
    /// Classified errors.
    pub errors: Vec<CompileError>,
    /// Category distribution.
    pub categories: std::collections::HashMap<ErrorCategory, usize>,
}

/// Classify a rustc error code into a category.
pub fn classify_error_code(code: &str) -> ErrorCategory {
    match code {
        // Syntax / parsing
        "E0060" | "E0061" | "E0064" | "E0065" => ErrorCategory::Syntax,

        // Unresolved imports / paths
        "E0432" | "E0433" | "E0412" | "E0425" | "E0531" => ErrorCategory::UnresolvedImport,

        // Type mismatches
        "E0308" | "E0271" | "E0369" => ErrorCategory::TypeMismatch,

        // Trait bounds (E0277 = "trait bound not satisfied")
        "E0277" => ErrorCategory::TraitBound,

        // Lifetimes / borrowing
        "E0106" | "E0495" | "E0597" | "E0502" | "E0499" | "E0596" => ErrorCategory::Lifetime,

        // Missing fields / methods / impls
        "E0046" | "E0063" | "E0599" | "E0609" => ErrorCategory::MissingMember,

        // Unused
        "E0170" => ErrorCategory::Unused,

        // Visibility
        "E0603" | "E0624" | "E0616" => ErrorCategory::Visibility,

        // Macro
        "E0659" | "E0658" => ErrorCategory::Macro,

        // Move / ownership (E0505 = borrow while moved, E0507 = move out of borrow)
        "E0382" | "E0505" | "E0507" | "E0515" | "E0716" => ErrorCategory::Ownership,

        _ => ErrorCategory::Other,
    }
}

/// Classify a raw error message line (without a code) by pattern matching.
fn classify_message(msg: &str) -> ErrorCategory {
    let lower = msg.to_lowercase();
    if lower.contains("cannot find") || lower.contains("not found") || lower.contains("unresolved")
    {
        ErrorCategory::UnresolvedImport
    } else if lower.contains("expected") && lower.contains("found") {
        ErrorCategory::TypeMismatch
    } else if lower.contains("lifetime")
        || lower.contains("borrow")
        || lower.contains("does not live long enough")
    {
        ErrorCategory::Lifetime
    } else if lower.contains("missing field") || lower.contains("no method named") {
        ErrorCategory::MissingMember
    } else if lower.contains("unused") {
        ErrorCategory::Unused
    } else if lower.contains("private") || lower.contains("visibility") {
        ErrorCategory::Visibility
    } else if lower.contains("moved") || lower.contains("use of moved") {
        ErrorCategory::Ownership
    } else if lower.contains("trait bound") || lower.contains("is not satisfied") {
        ErrorCategory::TraitBound
    } else if lower.contains("macro") {
        ErrorCategory::Macro
    } else if lower.contains("syntax") || lower.contains("unexpected token") {
        ErrorCategory::Syntax
    } else {
        ErrorCategory::Other
    }
}

/// Parse cargo JSON diagnostic output into structured errors.
///
/// Input: the stderr from `cargo check --message-format=json`.
/// Each line is a JSON message; we extract `compiler-message` entries.
pub fn parse_cargo_json(stderr: &str) -> CompileErrorSummary {
    let mut summary = CompileErrorSummary::default();

    for line in stderr.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        // cargo emits {"reason": "compiler-message", "message": {...}}
        if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }

        let Some(message) = msg.get("message") else {
            continue;
        };

        let level = message.get("level").and_then(|l| l.as_str()).unwrap_or("");
        let text = message
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let code_str = message
            .get("code")
            .and_then(|c| c.get("code"))
            .and_then(|c| c.as_str());

        if level == "warning" {
            summary.warning_count += 1;
            continue;
        }

        if level != "error" {
            continue;
        }

        summary.error_count += 1;

        // Extract location from primary span.
        let (file, line_num, col) = message
            .get("spans")
            .and_then(|s| s.as_array())
            .and_then(|spans| {
                spans
                    .iter()
                    .find(|s| s.get("is_primary") == Some(&serde_json::Value::Bool(true)))
            })
            .map(|span| {
                (
                    span.get("file_name")
                        .and_then(|f| f.as_str())
                        .map(String::from),
                    span.get("line_start")
                        .and_then(|l| l.as_u64())
                        .and_then(|l| u32::try_from(l).ok()),
                    span.get("column_start")
                        .and_then(|c| c.as_u64())
                        .and_then(|c| u32::try_from(c).ok()),
                )
            })
            .unwrap_or((None, None, None));

        // Extract suggestion from children.
        let suggestion = message
            .get("children")
            .and_then(|c| c.as_array())
            .and_then(|children| {
                children.iter().find_map(|child| {
                    let level = child.get("level").and_then(|l| l.as_str())?;
                    if level == "help" || level == "suggestion" {
                        child
                            .get("message")
                            .and_then(|m| m.as_str())
                            .map(String::from)
                    } else {
                        None
                    }
                })
            });

        let category = code_str.map_or_else(|| classify_message(text), classify_error_code);

        *summary.categories.entry(category.clone()).or_insert(0) += 1;

        summary.errors.push(CompileError {
            category,
            code: code_str.map(String::from),
            message: text.to_string(),
            file,
            line: line_num,
            column: col,
            suggestion,
        });
    }

    summary
}

/// Parse plain-text stderr (non-JSON) into structured errors.
///
/// Fallback for when `--message-format=json` isn't used.
pub fn parse_plain_stderr(stderr: &str) -> CompileErrorSummary {
    let mut summary = CompileErrorSummary::default();

    for line in stderr.lines() {
        let trimmed = line.trim_start();

        if trimmed.starts_with("error[") {
            summary.error_count += 1;

            // Parse "error[E0308]: mismatched types"
            let code = trimmed
                .get(6..)
                .and_then(|s| s.find(']').map(|i| &s[..i]))
                .map(String::from);

            let message = trimmed
                .find("]: ")
                .map(|i| trimmed[i + 3..].to_string())
                .unwrap_or_else(|| trimmed.to_string());

            let category = code
                .as_ref()
                .map_or_else(|| classify_message(&message), |c| classify_error_code(c));

            *summary.categories.entry(category.clone()).or_insert(0) += 1;

            summary.errors.push(CompileError {
                category,
                code,
                message,
                file: None,
                line: None,
                column: None,
                suggestion: None,
            });
        } else if let Some(rest) = trimmed.strip_prefix("error:") {
            summary.error_count += 1;
            let message = rest.trim().to_string();
            let category = classify_message(&message);

            *summary.categories.entry(category.clone()).or_insert(0) += 1;

            summary.errors.push(CompileError {
                category,
                code: None,
                message,
                file: None,
                line: None,
                column: None,
                suggestion: None,
            });
        } else if trimmed.starts_with("warning:") || trimmed.starts_with("warning[") {
            summary.warning_count += 1;
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_common_error_codes() {
        assert_eq!(
            classify_error_code("E0433"),
            ErrorCategory::UnresolvedImport
        );
        assert_eq!(classify_error_code("E0308"), ErrorCategory::TypeMismatch);
        assert_eq!(classify_error_code("E0597"), ErrorCategory::Lifetime);
        assert_eq!(classify_error_code("E0063"), ErrorCategory::MissingMember);
        assert_eq!(classify_error_code("E0382"), ErrorCategory::Ownership);
        assert_eq!(classify_error_code("E9999"), ErrorCategory::Other);
    }

    #[test]
    fn classify_message_patterns() {
        assert_eq!(
            classify_message("cannot find value `foo` in this scope"),
            ErrorCategory::UnresolvedImport
        );
        assert_eq!(
            classify_message("expected `u32`, found `String`"),
            ErrorCategory::TypeMismatch
        );
        assert_eq!(
            classify_message("`x` does not live long enough"),
            ErrorCategory::Lifetime
        );
        assert_eq!(
            classify_message("missing field `name` in initializer"),
            ErrorCategory::MissingMember
        );
        assert_eq!(
            classify_message("value used here after move"),
            ErrorCategory::Other // "move" not "moved"
        );
    }

    #[test]
    fn parse_plain_stderr_basic() {
        let stderr = "\
warning: unused variable
error[E0433]: failed to resolve: use of undeclared crate
error: aborting due to 1 previous error
";
        let summary = parse_plain_stderr(stderr);
        assert_eq!(summary.error_count, 2);
        assert_eq!(summary.warning_count, 1);
        assert_eq!(summary.errors.len(), 2);
        assert_eq!(summary.errors[0].category, ErrorCategory::UnresolvedImport);
        assert_eq!(summary.errors[0].code.as_deref(), Some("E0433"));
    }

    #[test]
    fn parse_cargo_json_basic() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"cannot find value `foo`","code":{"code":"E0425","explanation":null},"level":"error","spans":[{"file_name":"src/main.rs","byte_start":0,"byte_end":3,"line_start":1,"line_end":1,"column_start":1,"column_end":4,"is_primary":true}],"children":[{"message":"consider importing this","level":"help"}]}}"#;
        let summary = parse_cargo_json(json_line);
        assert_eq!(summary.error_count, 1);
        assert_eq!(summary.errors[0].category, ErrorCategory::UnresolvedImport);
        assert_eq!(summary.errors[0].code.as_deref(), Some("E0425"));
        assert_eq!(summary.errors[0].file.as_deref(), Some("src/main.rs"));
        assert_eq!(summary.errors[0].line, Some(1));
        assert_eq!(
            summary.errors[0].suggestion.as_deref(),
            Some("consider importing this")
        );
    }

    #[test]
    fn parse_cargo_json_warnings_only() {
        let json_line = r#"{"reason":"compiler-message","message":{"message":"unused variable","code":null,"level":"warning","spans":[],"children":[]}}"#;
        let summary = parse_cargo_json(json_line);
        assert_eq!(summary.error_count, 0);
        assert_eq!(summary.warning_count, 1);
        assert!(summary.errors.is_empty());
    }

    #[test]
    fn summary_tracks_category_distribution() {
        let stderr = "\
error[E0433]: failed to resolve
error[E0433]: unresolved import
error[E0308]: mismatched types
";
        let summary = parse_plain_stderr(stderr);
        assert_eq!(
            summary.categories.get(&ErrorCategory::UnresolvedImport),
            Some(&2)
        );
        assert_eq!(
            summary.categories.get(&ErrorCategory::TypeMismatch),
            Some(&1)
        );
    }
}
