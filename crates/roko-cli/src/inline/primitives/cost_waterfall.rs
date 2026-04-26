//! Primitive 11: `CostWaterfall` — decomposed savings breakdown.
//!
//! ```text
//! ◆ cost waterfall
//! │ baseline (opus, no cache, no routing)     $2.61
//! │ ├── prompt caching                       -$1.31  (5.0x)
//! │ ├── cascade routing (haiku)              -$0.78  (3.1x)
//! │ ├── knowledge pre-load                   -$0.29  (1.4x)
//! │ └── gate early-exit                      -$0.14  (1.2x)
//! │ actual                                    $0.084
//! └ savings ratio                             31.1x
//! ```

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::styled;
use super::super::symbols;

/// A single line in the cost waterfall.
#[derive(Debug, Clone)]
pub struct WaterfallEntry {
    /// Label (e.g. "prompt caching").
    pub label: String,
    /// Cost reduction in USD (positive = savings).
    pub savings_usd: f64,
    /// Reduction factor (e.g. 5.0 for "5x").
    pub factor: f64,
}

/// Full cost waterfall data.
#[derive(Debug, Clone)]
pub struct CostWaterfallData {
    /// Naive baseline cost (all Opus, no optimization).
    pub baseline_usd: f64,
    /// Breakdown of where savings came from.
    pub entries: Vec<WaterfallEntry>,
    /// Actual cost after all optimizations.
    pub actual_usd: f64,
}

impl CostWaterfallData {
    /// Compute savings ratio.
    #[must_use]
    pub fn savings_ratio(&self) -> f64 {
        if self.actual_usd <= 0.0 {
            return 1.0;
        }
        self.baseline_usd / self.actual_usd
    }

    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(self.entries.len() + 4);

        lines.push(styled::section_start(theme, "cost waterfall", "this session", None));

        // Baseline
        lines.push(styled::continuation(
            theme,
            "baseline",
            &format!("${:.4}", self.baseline_usd),
            Some("opus, no cache, no routing"),
        ));

        // Entries
        let entry_count = self.entries.len();
        for (i, entry) in self.entries.iter().enumerate() {
            let is_last = i == entry_count - 1;
            let connector = if is_last {
                format!("{} {}── ", symbols::BAR, symbols::END)
            } else {
                format!("{} {}── ", symbols::BAR, symbols::BRANCH)
            };

            let savings_str = format!("-${:.4}", entry.savings_usd);
            let factor_str = format!("({:.1}x)", entry.factor);

            lines.push(Line::from(vec![
                Span::styled(connector, theme.muted()),
                Span::styled(
                    format!("{:<28}", entry.label),
                    theme.text(),
                ),
                Span::styled(
                    format!("{:<12}", savings_str),
                    theme.success(),
                ),
                Span::styled(
                    factor_str,
                    Style::default().fg(Theme::TEXT_DIM),
                ),
            ]));
        }

        // Actual
        lines.push(styled::continuation(
            theme,
            "actual",
            &format!("${:.4}", self.actual_usd),
            None,
        ));

        // Savings ratio
        let ratio = self.savings_ratio();
        let ratio_style = if ratio >= 10.0 {
            Style::default()
                .fg(Theme::SAGE)
                .add_modifier(Modifier::BOLD)
        } else {
            theme.text()
        };

        lines.push(Line::from(vec![
            Span::styled(symbols::END.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(
                format!("{:<10}", "savings"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(format!("{ratio:.1}x"), ratio_style),
            Span::styled(
                "  methodology: real tokens, real prices, this session".to_string(),
                Style::default().fg(Theme::TEXT_GHOST),
            ),
        ]));

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waterfall_renders() {
        let theme = Theme::dark();
        let data = CostWaterfallData {
            baseline_usd: 2.61,
            entries: vec![
                WaterfallEntry { label: "prompt caching".into(), savings_usd: 1.31, factor: 5.0 },
                WaterfallEntry { label: "cascade routing (haiku)".into(), savings_usd: 0.78, factor: 3.1 },
                WaterfallEntry { label: "knowledge pre-load".into(), savings_usd: 0.29, factor: 1.4 },
                WaterfallEntry { label: "gate early-exit".into(), savings_usd: 0.14, factor: 1.2 },
            ],
            actual_usd: 0.084,
        };
        let lines = data.to_lines(&theme);
        // header + baseline + 4 entries + actual + savings = 8
        assert_eq!(lines.len(), 8);
        assert!(data.savings_ratio() > 30.0);
    }

    #[test]
    fn waterfall_zero_cost() {
        let data = CostWaterfallData {
            baseline_usd: 1.0,
            entries: vec![],
            actual_usd: 0.0,
        };
        assert_eq!(data.savings_ratio(), 1.0);
    }
}
