//! Per-language prompt hints for coding agents.
//!
//! [`prompt_hints_for`] returns language-specific best practices that can be
//! injected into agent system prompts to improve code quality.

/// Returns language-specific coding hints for the given language name.
///
/// The `language` parameter should match the value returned by
/// [`LanguageProvider::language_name()`](roko_core::language::LanguageProvider::language_name).
///
/// Unknown languages return a generic hint.
pub fn prompt_hints_for(language: &str) -> &'static str {
    match language {
        "rust" => concat!(
            "Use Result for error handling, derive common traits, ",
            "prefer &str over String in function signatures.",
        ),
        "typescript" => concat!(
            "Use strict TypeScript, prefer interfaces over type aliases for objects, ",
            "use const assertions.",
        ),
        "go" => concat!(
            "Use error wrapping with fmt.Errorf, prefer table-driven tests, ",
            "follow Go naming conventions.",
        ),
        _ => "Follow language-idiomatic conventions and standard formatting.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_hints() {
        let hints = prompt_hints_for("rust");
        assert!(hints.contains("Result"));
        assert!(hints.contains("&str"));
    }

    #[test]
    fn typescript_hints() {
        let hints = prompt_hints_for("typescript");
        assert!(hints.contains("strict TypeScript"));
        assert!(hints.contains("const assertions"));
    }

    #[test]
    fn go_hints() {
        let hints = prompt_hints_for("go");
        assert!(hints.contains("fmt.Errorf"));
        assert!(hints.contains("table-driven"));
    }

    #[test]
    fn unknown_language_returns_generic() {
        let hints = prompt_hints_for("haskell");
        assert!(hints.contains("idiomatic"));
    }
}
