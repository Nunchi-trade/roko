//! Primitive 18: `ErrorBlock` — structured error display.
//!
//! ```text
//! │ ✖  gate failed: compile (rung 1/7)
//! │    error[E0308]: expected `i32`, found `String`
//! │      --> src/handler.rs:42:18
//! │    42 │     let cost: i32 = calculate_cost();
//! │       │                     ^^^^^^^^^^^^^^^^ expected i32, found String
//! │
//! │    retry in 10s (attempt 1/3, exponential backoff)
//! ```

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::symbols;

/// Severity level of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Fatal — execution cannot continue.
    Fatal,
    /// Error — task failed, may retry.
    Error,
    /// Warning — task succeeded with issues.
    Warning,
    /// Info — informational, no action needed.
    Info,
}

/// A structured error for display.
#[derive(Debug, Clone)]
pub struct ErrorBlockData {
    /// Error severity.
    pub severity: ErrorSeverity,
    /// Source (e.g. "compile", "test", "agent").
    pub source: String,
    /// One-line summary.
    pub summary: String,
    /// Optional file location (file:line:col).
    pub location: Option<String>,
    /// Detailed error lines (compiler output, stack trace, etc.).
    pub details: Vec<String>,
    /// Retry info if applicable.
    pub retry: Option<RetryInfo>,
}

/// Retry context for an error.
#[derive(Debug, Clone)]
pub struct RetryInfo {
    /// Current attempt number.
    pub attempt: u32,
    /// Maximum attempts.
    pub max_attempts: u32,
    /// Seconds until next retry.
    pub retry_in_s: f64,
    /// Strategy description (e.g. "exponential backoff", "escalate to sonnet").
    pub strategy: String,
}

impl ErrorBlockData {
    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        let (icon, style) = match self.severity {
            ErrorSeverity::Fatal => (symbols::FAIL, theme.danger()),
            ErrorSeverity::Error => (symbols::FAIL, theme.danger()),
            ErrorSeverity::Warning => (symbols::WARN, theme.warning()),
            ErrorSeverity::Info => (symbols::INFO, theme.info()),
        };

        // Header: │ ✖  source: summary
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(icon.to_string(), style),
            Span::raw("  "),
            Span::styled(
                format!("{}: ", self.source),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(self.summary.clone(), style),
        ]));

        // Location
        if let Some(ref loc) = self.location {
            lines.push(Line::from(vec![
                Span::styled(symbols::BAR.to_string(), theme.muted()),
                Span::raw("    "),
                Span::styled(
                    format!("{} {loc}", symbols::ARROW),
                    Style::default().fg(Theme::TEXT_DIM),
                ),
            ]));
        }

        // Detail lines (indented, dimmed)
        for detail in &self.details {
            lines.push(Line::from(vec![
                Span::styled(symbols::BAR.to_string(), theme.muted()),
                Span::raw("    "),
                Span::styled(detail.clone(), Style::default().fg(Theme::TEXT_DIM)),
            ]));
        }

        // Retry info
        if let Some(ref retry) = self.retry {
            lines.push(Line::from(vec![
                Span::styled(symbols::BAR.to_string(), theme.muted()),
            ]));
            lines.push(Line::from(vec![
                Span::styled(symbols::BAR.to_string(), theme.muted()),
                Span::raw("    "),
                Span::styled(
                    format!(
                        "retry in {:.0}s (attempt {}/{}, {})",
                        retry.retry_in_s, retry.attempt, retry.max_attempts, retry.strategy,
                    ),
                    theme.warning(),
                ),
            ]));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_block_minimal() {
        let theme = Theme::dark();
        let data = ErrorBlockData {
            severity: ErrorSeverity::Error,
            source: "compile".into(),
            summary: "expected i32, found String".into(),
            location: None,
            details: vec![],
            retry: None,
        };
        let lines = data.to_lines(&theme);
        assert!(!lines.is_empty());
    }

    #[test]
    fn error_block_full() {
        let theme = Theme::dark();
        let data = ErrorBlockData {
            severity: ErrorSeverity::Error,
            source: "compile".into(),
            summary: "error[E0308]: mismatched types".into(),
            location: Some("src/handler.rs:42:18".into()),
            details: vec![
                "42 │     let cost: i32 = calculate_cost();".into(),
                "   │                     ^^^^^^^^^^^^^^^^ expected i32".into(),
            ],
            retry: Some(RetryInfo {
                attempt: 1,
                max_attempts: 3,
                retry_in_s: 10.0,
                strategy: "exponential backoff".into(),
            }),
        };
        let lines = data.to_lines(&theme);
        // header + location + 2 details + blank + retry = 6
        assert!(lines.len() >= 5);
    }

    #[test]
    fn warning_uses_warn_icon() {
        let theme = Theme::dark();
        let data = ErrorBlockData {
            severity: ErrorSeverity::Warning,
            source: "clippy".into(),
            summary: "unused variable".into(),
            location: None,
            details: vec![],
            retry: None,
        };
        let lines = data.to_lines(&theme);
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains(symbols::WARN));
    }
}
