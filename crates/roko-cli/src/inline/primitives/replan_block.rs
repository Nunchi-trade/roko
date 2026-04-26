//! Primitive 9: `ReplanBlock` — gate failure + automatic replan visualization.
//!
//! ```text
//! │ gate       test ✖  assertion error in handler.rs:42
//! │ replan     escalating to sonnet (confidence: 0.67 → 0.91)
//! │ retry      attempt 2/3  ━━━━━━━━━━ done in 4.2s
//! │ gate       test ✔  all assertions pass
//! │ actual     $0.058 (replan cost: +$0.027)
//! ```

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::styled;
use super::super::symbols;

/// A replan event: a gate failed, the system replanned and retried.
#[derive(Debug, Clone)]
pub struct ReplanBlockData {
    /// Which gate failed.
    pub failed_gate: String,
    /// Failure reason.
    pub failure_reason: String,
    /// Strategy used for replan (e.g. "escalate to sonnet", "retry same model").
    pub strategy: String,
    /// Confidence before replan.
    pub confidence_before: Option<f64>,
    /// Confidence threshold required.
    pub confidence_required: Option<f64>,
    /// Attempt number of the retry.
    pub attempt: u32,
    /// Max attempts.
    pub max_attempts: u32,
    /// Whether the retry succeeded.
    pub retry_succeeded: bool,
    /// Duration of the retry in seconds.
    pub retry_duration_s: f64,
    /// Additional cost from the replan.
    pub replan_cost_usd: f64,
}

impl ReplanBlockData {
    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        // Failed gate line
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                format!("{:<10}", "gate"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(self.failed_gate.clone(), theme.text()),
            Span::raw(" "),
            Span::styled(symbols::FAIL.to_string(), theme.danger()),
            Span::raw("  "),
            Span::styled(self.failure_reason.clone(), theme.danger()),
        ]));

        // Replan strategy
        let conf_detail = match (self.confidence_before, self.confidence_required) {
            (Some(before), Some(required)) => {
                format!(
                    "confidence: {before:.2} {arrow} {required:.2}",
                    arrow = symbols::ARROW
                )
            }
            _ => String::new(),
        };

        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                format!("{:<10}", "replan"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(self.strategy.clone(), theme.warning()),
            if conf_detail.is_empty() {
                Span::raw("")
            } else {
                Span::styled(
                    format!("  ({})", conf_detail),
                    Style::default().fg(Theme::TEXT_DIM),
                )
            },
        ]));

        // Retry result
        let result_icon = if self.retry_succeeded {
            symbols::PASS
        } else {
            symbols::FAIL
        };
        let result_style = if self.retry_succeeded {
            theme.success()
        } else {
            theme.danger()
        };

        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                format!("{:<10}", "retry"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(
                format!("attempt {}/{}", self.attempt, self.max_attempts),
                theme.text(),
            ),
            Span::raw("  "),
            Span::styled(result_icon.to_string(), result_style),
            Span::styled(
                format!("  ({:.1}s)", self.retry_duration_s),
                Style::default().fg(Theme::TEXT_GHOST),
            ),
        ]));

        // Replan cost
        if self.replan_cost_usd > 0.0 {
            lines.push(styled::continuation(
                theme,
                "replan cost",
                &format!("+${:.4}", self.replan_cost_usd),
                None,
            ));
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replan_success() {
        let theme = Theme::dark();
        let data = ReplanBlockData {
            failed_gate: "test".into(),
            failure_reason: "assertion error in handler.rs:42".into(),
            strategy: "escalate to sonnet".into(),
            confidence_before: Some(0.67),
            confidence_required: Some(0.91),
            attempt: 2,
            max_attempts: 3,
            retry_succeeded: true,
            retry_duration_s: 4.2,
            replan_cost_usd: 0.027,
        };
        let lines = data.to_lines(&theme);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn replan_failure() {
        let theme = Theme::dark();
        let data = ReplanBlockData {
            failed_gate: "compile".into(),
            failure_reason: "syntax error".into(),
            strategy: "retry same model".into(),
            confidence_before: None,
            confidence_required: None,
            attempt: 3,
            max_attempts: 3,
            retry_succeeded: false,
            retry_duration_s: 8.1,
            replan_cost_usd: 0.0,
        };
        let lines = data.to_lines(&theme);
        assert!(lines.len() >= 3);
    }
}
