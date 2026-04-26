//! Primitive 3: `ToolCallBlock` — tool invocation display.
//!
//! Renders tool calls in two modes:
//! - **Collapsed** (default): single line with name, summary, duration
//! - **Expanded**: full input/output with syntax highlighting
//!
//! Used by chat, run, audit, and the dashboard.

use ratatui::text::Line;
use serde_json::Value;

use crate::tui::Theme;

use super::super::styled;
use super::super::symbols;

/// A completed tool call with its result.
#[derive(Debug, Clone)]
pub struct ToolCallBlock {
    /// Tool name (e.g. "ReadFile", "Edit", "Bash").
    pub name: String,
    /// Summarized input (e.g. "src/main.rs" for ReadFile).
    pub input_summary: String,
    /// Full input arguments (for expanded view).
    pub input: Value,
    /// Result text (for expanded view).
    pub result: Option<String>,
    /// Duration in seconds.
    pub duration_s: f64,
    /// Whether the tool call succeeded.
    pub success: bool,
    /// Whether to show expanded view.
    pub expanded: bool,
}

impl ToolCallBlock {
    /// Create a new tool call block from a start event.
    pub fn from_start(name: &str, input: &Value) -> Self {
        let input_summary = summarize_tool_input(name, input);
        Self {
            name: name.to_string(),
            input_summary,
            input: input.clone(),
            result: None,
            duration_s: 0.0,
            success: true,
            expanded: false,
        }
    }

    /// Set the result after the tool call completes.
    pub fn set_result(&mut self, result: &str, duration_s: f64, success: bool) {
        self.result = Some(result.to_string());
        self.duration_s = duration_s;
        self.success = success;
    }

    /// Render as styled lines for scrollback.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        if self.expanded {
            self.render_expanded(theme)
        } else {
            vec![self.render_collapsed(theme)]
        }
    }

    fn render_collapsed(&self, theme: &Theme) -> Line<'static> {
        styled::tool_call_collapsed(
            theme,
            &self.name,
            &self.input_summary,
            self.duration_s,
        )
    }

    fn render_expanded(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Header (expanded triangle)
        lines.push(styled::tool_call_expanded_header(
            theme,
            &self.name,
            &self.input_summary,
            self.duration_s,
        ));

        // Input
        if !self.input.is_null() {
            let input_str = serde_json::to_string_pretty(&self.input).unwrap_or_default();
            for line in input_str.lines().take(10) {
                lines.push(styled::indented_line(theme, line, 2));
            }
        }

        // Result (truncated)
        if let Some(ref result) = self.result {
            lines.push(styled::indented_line(theme, "─── result ───", 2));
            for line in result.lines().take(5) {
                lines.push(styled::indented_line(theme, line, 2));
            }
            let total_lines = result.lines().count();
            if total_lines > 5 {
                lines.push(styled::indented_line(
                    theme,
                    &format!("... +{} more lines", total_lines - 5),
                    2,
                ));
            }
        }

        lines
    }
}

/// Summarize tool input for the collapsed view.
///
/// Extracts the most relevant piece of information from the tool's input
/// arguments to show in a single line.
pub fn summarize_tool_input(name: &str, input: &Value) -> String {
    match name {
        "ReadFile" | "Read" => {
            let path = input
                .get("file_path")
                .or_else(|| input.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("?");
            let short = shorten_path(path);
            if let Some(limit) = input.get("limit").and_then(Value::as_u64) {
                format!("{short} ({limit} lines)")
            } else {
                short.to_string()
            }
        }
        "Edit" => {
            let path = input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?");
            let short = shorten_path(path);
            format!("{short}")
        }
        "Write" => {
            let path = input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?");
            shorten_path(path).to_string()
        }
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("?");
            if cmd.len() > 50 {
                format!("{}...", &cmd[..47])
            } else {
                cmd.to_string()
            }
        }
        "Grep" | "Glob" => {
            let pattern = input
                .get("pattern")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("\"{pattern}\"")
        }
        "WebSearch" => {
            let query = input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("?");
            format!("\"{query}\"")
        }
        _ => {
            // Generic: show first string value
            if let Some(obj) = input.as_object() {
                for (_, v) in obj.iter().take(1) {
                    if let Some(s) = v.as_str() {
                        let s = if s.len() > 40 {
                            format!("{}...", &s[..37])
                        } else {
                            s.to_string()
                        };
                        return s;
                    }
                }
            }
            String::new()
        }
    }
}

/// Shorten a file path by keeping only the last 2-3 components.
fn shorten_path(path: &str) -> &str {
    let parts: Vec<&str> = path.rsplitn(4, '/').collect();
    if parts.len() >= 3 {
        // Get the offset where the 3rd-from-end component starts
        let suffix_len: usize = parts[..3].iter().map(|p| p.len() + 1).sum::<usize>() - 1;
        &path[path.len().saturating_sub(suffix_len)..]
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_read_file() {
        let input = json!({"file_path": "/Users/will/dev/project/src/main.rs"});
        let summary = summarize_tool_input("ReadFile", &input);
        assert!(summary.contains("main.rs"));
    }

    #[test]
    fn summarize_bash() {
        let input = json!({"command": "cargo test --workspace"});
        let summary = summarize_tool_input("Bash", &input);
        assert_eq!(summary, "cargo test --workspace");
    }

    #[test]
    fn summarize_bash_long() {
        let input = json!({"command": "a".repeat(100)});
        let summary = summarize_tool_input("Bash", &input);
        assert!(summary.len() <= 53);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn summarize_grep() {
        let input = json!({"pattern": "fn main"});
        let summary = summarize_tool_input("Grep", &input);
        assert_eq!(summary, "\"fn main\"");
    }

    #[test]
    fn tool_call_collapsed_renders() {
        let theme = Theme::dark();
        let block = ToolCallBlock::from_start("ReadFile", &json!({"file_path": "src/lib.rs"}));
        let lines = block.to_lines(&theme);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn tool_call_expanded_renders() {
        let theme = Theme::dark();
        let mut block = ToolCallBlock::from_start("ReadFile", &json!({"file_path": "src/lib.rs"}));
        block.expanded = true;
        block.set_result("contents here", 0.3, true);
        let lines = block.to_lines(&theme);
        assert!(lines.len() > 1);
    }
}
