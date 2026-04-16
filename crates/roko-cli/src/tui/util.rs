//! Shared TUI utility helpers.

/// Truncate a string in the middle with an ellipsis if it exceeds `max`.
pub(crate) fn truncate_middle(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return "\u{2026}".repeat(max);
    }
    let keep_left = (max - 1) / 2;
    let keep_right = max - keep_left - 1;
    let left: String = s.chars().take(keep_left).collect();
    let right: String = s
        .chars()
        .rev()
        .take(keep_right)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{left}\u{2026}{right}")
}

#[cfg(test)]
mod tests {
    use super::truncate_middle;

    #[test]
    fn middle_truncation_edge_cases() {
        assert_eq!(truncate_middle("hello", 10), "hello");
        assert_eq!(truncate_middle("hello world", 5), "he\u{2026}ld");
        assert_eq!(truncate_middle("abc", 0), "");
        assert_eq!(truncate_middle("abcdef", 3), "\u{2026}\u{2026}\u{2026}");
    }
}
