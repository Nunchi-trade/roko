//! Output validation and TOML repair for enrichment steps.
//!
//! Ported from `apps/mori/src/support_enrich/mod.rs` lines 1624-1663.
//!
//! Validation logic is pure: it takes a string and returns a result. No I/O.
//! The repair prompt is built here but the actual LLM call happens in the
//! pipeline (I/O at boundary only).

use super::step::EnrichStep;

/// Validate the output of a step.
///
/// - Empty output is always an error.
/// - TOML steps additionally parse the content as TOML.
///
/// Ported from Mori `validate_step_output` (lines 1653-1663).
pub fn validate_step_output(step: EnrichStep, content: &str) -> Result<(), String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(format!("generated output for {step} was empty"));
    }
    if step.is_toml() {
        toml::from_str::<toml::Value>(trimmed)
            .map_err(|e| format!("generated invalid TOML for {step}: {e}"))?;
    }
    Ok(())
}

/// Normalize raw LLM output for a step.
///
/// Strips markdown fences and leading prose. For TOML steps, trims to start
/// at `[meta]` if present and drops trailing fences.
///
/// Ported from Mori `normalize_step_output` (lines 1636-1651).
pub fn normalize_step_output(step: EnrichStep, content: &str) -> String {
    let cleaned = strip_fences(content);
    if !step.is_toml() {
        return cleaned.trim().to_string();
    }

    let mut trimmed = cleaned.trim();
    if let Some(meta_idx) = trimmed.find("[meta]") {
        trimmed = &trimmed[meta_idx..];
    }
    trimmed.find("\n```").map_or_else(
        || trimmed.to_string(),
        |fence_idx| trimmed[..fence_idx].trim().to_string(),
    )
}

/// Attempt to repair invalid TOML output.
///
/// Returns `Ok(repaired)` if the repaired content passes validation, or
/// `Err(message)` if repair also failed.
///
/// The caller must provide the repair text from the LLM (this function does
/// not call the LLM itself — I/O at boundary only).
pub fn repair_toml_output(step: EnrichStep, repaired_raw: &str) -> Result<String, String> {
    let normalized = normalize_step_output(step, repaired_raw);
    validate_step_output(step, &normalized)?;
    Ok(normalized)
}

/// Strip markdown code fences from LLM output.
fn strip_fences(content: &str) -> String {
    let trimmed = content.trim();

    // Strip opening ```toml or ```markdown fence.
    let without_open = trimmed.strip_prefix("```").map_or(trimmed, |rest| {
        // Skip the language tag line.
        rest.find('\n').map_or(rest, |nl| &rest[nl + 1..])
    });

    // Strip closing ``` fence.
    let without_close = without_open.strip_suffix("```").unwrap_or(without_open);

    without_close.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty() {
        let err = validate_step_output(EnrichStep::Tasks, "").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_rejects_whitespace_only() {
        let err = validate_step_output(EnrichStep::Tasks, "   \n  ").unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_accepts_valid_toml() {
        let toml = "[meta]\nplan = \"test\"\n\n[[task]]\nid = \"T1\"\n";
        assert!(validate_step_output(EnrichStep::Tasks, toml).is_ok());
    }

    #[test]
    fn validate_rejects_invalid_toml() {
        let bad = "not valid toml {{{";
        let err = validate_step_output(EnrichStep::Tasks, bad).unwrap_err();
        assert!(err.contains("invalid TOML"));
    }

    #[test]
    fn validate_accepts_any_non_empty_markdown() {
        assert!(validate_step_output(EnrichStep::Decompose, "# Title\nSome content.").is_ok());
    }

    #[test]
    fn normalize_strips_fences() {
        let raw = "```toml\n[meta]\nplan = \"x\"\n```";
        let result = normalize_step_output(EnrichStep::Tasks, raw);
        assert!(result.starts_with("[meta]"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn normalize_trims_to_meta() {
        let raw = "Here is the TOML:\n\n[meta]\nplan = \"x\"\n\nsome trailing text";
        let result = normalize_step_output(EnrichStep::Tasks, raw);
        assert!(result.starts_with("[meta]"));
    }

    #[test]
    fn normalize_markdown_just_trims() {
        let raw = "  # Title\n\nContent.  ";
        let result = normalize_step_output(EnrichStep::Decompose, raw);
        assert_eq!(result, "# Title\n\nContent.");
    }

    #[test]
    fn repair_toml_output_accepts_valid() {
        let repaired = "[meta]\nplan = \"test\"\n";
        assert!(repair_toml_output(EnrichStep::Tasks, repaired).is_ok());
    }

    #[test]
    fn repair_toml_output_rejects_still_invalid() {
        let still_bad = "not valid toml <<<";
        assert!(repair_toml_output(EnrichStep::Tasks, still_bad).is_err());
    }

    #[test]
    fn strip_fences_handles_no_fences() {
        assert_eq!(strip_fences("plain text"), "plain text");
    }

    #[test]
    fn strip_fences_handles_toml_fence() {
        let raw = "```toml\ncontent here\n```";
        assert_eq!(strip_fences(raw), "content here\n");
    }
}
