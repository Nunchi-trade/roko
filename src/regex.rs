pub fn matches(pattern: &str, text: &str) -> bool {
    fn helper(pattern: &[char], text: &[char]) -> bool {
        if pattern.is_empty() {
            return text.is_empty();
        }

        let first_matches = !text.is_empty() && (pattern[0] == '.' || pattern[0] == text[0]);

        if pattern.len() >= 2 && pattern[1] == '*' {
            helper(&pattern[2..], text) || (first_matches && helper(pattern, &text[1..]))
        } else {
            first_matches && helper(&pattern[1..], &text[1..])
        }
    }

    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    helper(&pattern_chars, &text_chars)
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn literal_match() {
        assert!(matches("abc", "abc"));
    }

    #[test]
    fn literal_mismatch() {
        assert!(!matches("abc", "abd"));
    }

    #[test]
    fn dot_matches_any_single_character() {
        assert!(matches("a.c", "abc"));
        assert!(matches("...", "xyz"));
    }

    #[test]
    fn star_matches_zero_or_more() {
        assert!(matches("ab*c", "ac"));
        assert!(matches("ab*c", "abc"));
        assert!(matches("ab*c", "abbbc"));
    }

    #[test]
    fn combined_dot_star() {
        assert!(matches(".*", "anything"));
    }

    #[test]
    fn empty_pattern_only_matches_empty_text() {
        assert!(matches("", ""));
        assert!(!matches("", "x"));
    }

    #[test]
    fn star_can_consume_empty_string() {
        assert!(matches("a*", ""));
        assert!(matches("a*b*", ""));
    }

    #[test]
    fn complex_case() {
        assert!(matches("c*a*b", "aab"));
        assert!(!matches("mis*is*p*.", "mississippi"));
    }
}
