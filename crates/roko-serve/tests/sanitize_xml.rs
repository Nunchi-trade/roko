//! Integration tests for sanitize_agent_content with XML-like content.

use roko_serve::sanitize_agent_content;

#[test]
fn strips_function_calls_block() {
    let raw =
        "Before\n<function_calls>\n<invoke name=\"foo\">\n</invoke>\n</function_calls>\nAfter";
    assert_eq!(sanitize_agent_content(raw), "Before\n\nAfter");
}

#[test]
fn combined_artifacts() {
    let raw = "Here is my analysis:\n\n<function_calls>\n<invoke name=\"read\">\n</invoke>\n</function_calls>\n\n\n\n{\"type\":\"assistant\",\"content\":\"internal\"}\nThe result is 42.";
    let cleaned = sanitize_agent_content(raw);
    assert!(!cleaned.contains("function_calls"));
    assert!(!cleaned.contains(r#""type":"assistant""#));
    assert!(cleaned.contains("Here is my analysis"));
    assert!(cleaned.contains("The result is 42"));
}
