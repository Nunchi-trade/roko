//! Shared display helpers for TUI views and widgets.
//!
//! Consolidates `shorten_model`, `display_model`, `event_model_slug`, and
//! `truncate` which were previously copy-pasted across 10+ TUI source files.

/// Shorten a model slug for compact display.
///
/// Strips common prefixes/infixes (`claude-`, `gpt-`, `-codex`, etc.) so the
/// model fits into narrow table columns.
pub fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

/// Human-friendly model name for display, handling `None`/empty/sentinel values.
pub fn display_model(model: Option<&str>) -> String {
    match model {
        None | Some("") | Some("-") | Some("unknown-model") => "unknown".to_string(),
        Some(m) => shorten_model(m),
    }
}

/// Extract the effective model identifier from an efficiency event.
///
/// Prefers `model_used` (the model the backend actually routed to) over
/// `model` (the requested model), falling back to `"unknown"`.
pub fn event_model_slug(event: &roko_learn::efficiency::AgentEfficiencyEvent) -> String {
    let model = event.model.trim();
    let used = event.model_used.trim();
    if !model.is_empty() {
        model.to_string()
    } else if !used.is_empty() {
        used.to_string()
    } else {
        "unknown".to_string()
    }
}

/// Truncate a string to at most `max` characters, appending `...` when
/// truncated. UTF-8 safe — counts by `char`, not by byte.
pub fn truncate(s: &str, max: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max {
        return s.to_string();
    }
    if max <= 3 {
        return ".".repeat(max);
    }
    let keep = max - 3;
    let truncated: String = s.chars().take(keep).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_model_strips_prefixes() {
        assert_eq!(shorten_model("claude-sonnet-4-20250514"), "s4-20250514");
        assert_eq!(shorten_model("gpt-4o-mini"), "4om");
        assert_eq!(shorten_model("claude-haiku-4-5"), "h4-5");
    }

    #[test]
    fn display_model_handles_sentinels() {
        assert_eq!(display_model(None), "unknown");
        assert_eq!(display_model(Some("")), "unknown");
        assert_eq!(display_model(Some("-")), "unknown");
        assert_eq!(display_model(Some("unknown-model")), "unknown");
        assert_eq!(display_model(Some("claude-opus-4-6")), "o4-6");
    }

    #[test]
    fn truncate_ascii() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 8), "hello...");
        assert_eq!(truncate("ab", 2), "ab");
        assert_eq!(truncate("abcdef", 3), "...");
    }

    #[test]
    fn truncate_utf8_safe() {
        // Multi-byte characters must not panic.
        let s = "café résumé";
        let t = truncate(s, 6);
        assert_eq!(t, "caf...");
    }
}
