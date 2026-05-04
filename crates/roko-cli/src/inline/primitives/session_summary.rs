//! Primitive 10: `SessionSummary` — end-of-session roll-up.
//!
//! ```text
//! ◆ session summary
//! │ runs          3
//! │ total cost    $0.084  ·  baseline: $2.61  ·  savings: 31.1x
//! │ cache hit     87%  (↑ from 0% on first run)
//! │ tokens        12,121 in / 3,193 out
//! │ gates         24/24 passed  (2 replans, both succeeded)
//! └ model         haiku (97% of tokens)
//! ```

use ratatui::text::Line;

use crate::tui::Theme;

use super::super::styled;
use super::super::symbols;
use super::cost_meter::CostMeter;

/// Data for a session summary block.
#[derive(Debug, Clone)]
pub struct SessionSummaryData {
    /// Cost meter with cumulative session data.
    pub cost: CostMeter,
    /// Total gate checks performed.
    pub gates_total: u32,
    /// Total gate checks passed.
    pub gates_passed: u32,
    /// Number of replans triggered.
    pub replans: u32,
    /// Elapsed session time in seconds.
    pub elapsed_s: f64,
}

impl SessionSummaryData {
    /// Create from a cost meter with minimal gate info.
    pub fn from_meter(cost: &CostMeter) -> Self {
        Self {
            cost: cost.clone(),
            gates_total: 0,
            gates_passed: 0,
            replans: 0,
            elapsed_s: 0.0,
        }
    }

    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(8);
        let ratio = self.cost.savings_ratio();

        lines.push(styled::section_start(theme, "session", "summary", None));

        let elapsed_detail = if self.elapsed_s > 0.0 {
            Some(format!("{:.1}s elapsed", self.elapsed_s))
        } else {
            None
        };
        lines.push(styled::continuation(
            theme,
            "runs",
            &self.cost.run_count.to_string(),
            elapsed_detail.as_deref(),
        ));

        // Cost line with baseline and savings
        let cost_value = format!("${:.4}", self.cost.total_cost.max(0.0));
        let cost_detail = if ratio > 1.5 {
            format!(
                "baseline: ${:.4}  {}  savings: {ratio:.1}x",
                self.cost.naive_baseline,
                symbols::SEP,
            )
        } else {
            String::new()
        };
        lines.push(styled::continuation(
            theme,
            "total cost",
            &cost_value,
            if cost_detail.is_empty() {
                None
            } else {
                Some(&cost_detail)
            },
        ));

        // Cache hit rate
        let cache_rate = self.cost.cache_hit_rate();
        if self.cost.cache_hits + self.cost.cache_misses > 0 {
            lines.push(styled::continuation(
                theme,
                "cache hit",
                &format!("{cache_rate:.0}%"),
                None,
            ));
        }

        // Tokens
        lines.push(styled::continuation(
            theme,
            "tokens",
            &format!(
                "{} in / {} out",
                self.cost.input_tokens, self.cost.output_tokens
            ),
            None,
        ));

        // Gates
        if self.gates_total > 0 {
            let gates_text = format!("{}/{} passed", self.gates_passed, self.gates_total);
            let gates_detail = if self.replans > 0 {
                Some(format!(
                    "{} replan{}",
                    self.replans,
                    if self.replans == 1 { "" } else { "s" }
                ))
            } else {
                None
            };
            lines.push(styled::continuation(
                theme,
                "gates",
                &gates_text,
                gates_detail.as_deref(),
            ));
        }

        // Primary model
        let model = self.cost.primary_model().unwrap_or("—");
        lines.push(styled::section_end(theme, "model", model));

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_meter_renders() {
        let theme = Theme::dark();
        let mut meter = CostMeter::new();
        meter.record_run(0.031, 4821, 1203, "haiku", 0.93);
        meter.record_run(0.022, 3100, 890, "haiku", 0.71);

        let summary = SessionSummaryData::from_meter(&meter);
        let lines = summary.to_lines(&theme);
        // header + runs + cost + tokens + model = 5 minimum
        assert!(lines.len() >= 5, "got {} lines", lines.len());
    }

    #[test]
    fn with_gates_renders() {
        let theme = Theme::dark();
        let mut meter = CostMeter::new();
        meter.record_run(0.05, 5000, 1500, "sonnet", 1.5);

        let summary = SessionSummaryData {
            cost: meter,
            gates_total: 12,
            gates_passed: 11,
            replans: 1,
            elapsed_s: 45.3,
        };
        let lines = summary.to_lines(&theme);
        assert!(lines.len() >= 6); // includes gates line
    }
}
