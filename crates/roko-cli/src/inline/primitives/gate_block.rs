//! Primitive 4: `GateBlock` — gate pipeline progress display.
//!
//! Shows each gate rung with status:
//! ```text
//! ◆ gates      7 rungs  ·  policy: prod-sec
//! ├ compile    ✔  0 errors (142 crates, 2.1s)
//! ├ clippy     ✔  0 warnings (0.8s)
//! ├ test       ━━━━━━░░░░ 4/11 tests  (running...)
//! ├ secret_scan  ⏳ pending
//! └ verify     ⏳ pending
//! ```

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::symbols;

/// Status of a single gate rung.
#[derive(Debug, Clone)]
pub enum GateStatus {
    /// Not yet started.
    Pending,
    /// Currently executing.
    Running {
        /// Elapsed seconds.
        elapsed_s: f64,
        /// Optional progress (e.g. "4/11 tests").
        progress: Option<String>,
    },
    /// Completed successfully.
    Passed {
        /// Summary text (e.g. "0 errors (142 crates)").
        summary: String,
        /// Duration in seconds.
        duration_s: f64,
    },
    /// Failed.
    Failed {
        /// Error summary.
        reason: String,
        /// Duration in seconds.
        duration_s: f64,
    },
    /// Skipped (e.g. not applicable for this task).
    Skipped,
}

/// A single gate rung for display.
#[derive(Debug, Clone)]
pub struct GateRung {
    /// Gate name (e.g. "compile", "test", "clippy").
    pub name: String,
    /// Current status.
    pub status: GateStatus,
}

/// Full gate pipeline block.
#[derive(Debug, Clone)]
pub struct GateBlockData {
    /// Optional policy name.
    pub policy: Option<String>,
    /// Ordered list of gate rungs.
    pub rungs: Vec<GateRung>,
}

impl GateBlockData {
    /// Build from a simple verdicts list (post-execution).
    pub fn from_verdicts(verdicts: &[(String, bool)]) -> Self {
        Self {
            policy: None,
            rungs: verdicts
                .iter()
                .map(|(name, passed)| GateRung {
                    name: name.clone(),
                    status: if *passed {
                        GateStatus::Passed {
                            summary: String::new(),
                            duration_s: 0.0,
                        }
                    } else {
                        GateStatus::Failed {
                            reason: String::new(),
                            duration_s: 0.0,
                        }
                    },
                })
                .collect(),
        }
    }

    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::with_capacity(self.rungs.len() + 1);

        // Header
        let rung_count = self.rungs.len();
        let detail = self.policy.as_deref().map(|p| format!("policy: {p}"));
        lines.push(Line::from({
            let mut spans = vec![
                Span::styled(symbols::START.to_string(), theme.accent()),
                Span::raw(" "),
                Span::styled(
                    "gates".to_string(),
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("{rung_count} rung{}", if rung_count == 1 { "" } else { "s" }),
                    theme.text(),
                ),
            ];
            if let Some(ref d) = detail {
                spans.push(Span::styled(format!("  {}  ", symbols::SEP), theme.muted()));
                spans.push(Span::styled(d.clone(), theme.muted()));
            }
            spans
        }));

        // Rungs
        for (i, rung) in self.rungs.iter().enumerate() {
            let is_last = i == self.rungs.len() - 1;
            let connector = if is_last { symbols::END } else { symbols::BRANCH };
            lines.push(render_rung(theme, connector, rung));
        }

        lines
    }
}

fn render_rung(theme: &Theme, connector: &str, rung: &GateRung) -> Line<'static> {
    let name_width = 14;
    let padded_name = format!("{:<width$}", rung.name, width = name_width);

    match &rung.status {
        GateStatus::Pending => Line::from(vec![
            Span::styled(connector.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(padded_name, theme.muted()),
            Span::styled(symbols::PENDING.to_string(), theme.muted()),
            Span::styled(" pending".to_string(), theme.muted()),
        ]),
        GateStatus::Running { elapsed_s, progress } => {
            let mut spans = vec![
                Span::styled(connector.to_string(), theme.muted()),
                Span::raw(" "),
                Span::styled(padded_name, theme.accent()),
            ];
            if let Some(p) = progress {
                spans.push(Span::styled(p.clone(), theme.text()));
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("({elapsed_s:.1}s)"),
                Style::default().fg(Theme::TEXT_GHOST),
            ));
            Line::from(spans)
        }
        GateStatus::Passed { summary, duration_s } => {
            let mut spans = vec![
                Span::styled(connector.to_string(), theme.muted()),
                Span::raw(" "),
                Span::styled(padded_name, theme.text()),
                Span::styled(symbols::PASS.to_string(), theme.success()),
            ];
            if !summary.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(summary.clone(), theme.muted()));
            }
            if *duration_s > 0.0 {
                spans.push(Span::styled(
                    format!("  ({duration_s:.1}s)"),
                    Style::default().fg(Theme::TEXT_GHOST),
                ));
            }
            Line::from(spans)
        }
        GateStatus::Failed { reason, duration_s } => {
            let mut spans = vec![
                Span::styled(connector.to_string(), theme.muted()),
                Span::raw(" "),
                Span::styled(padded_name, theme.danger()),
                Span::styled(symbols::FAIL.to_string(), theme.danger()),
            ];
            if !reason.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(reason.clone(), theme.danger()));
            }
            if *duration_s > 0.0 {
                spans.push(Span::styled(
                    format!("  ({duration_s:.1}s)"),
                    Style::default().fg(Theme::TEXT_GHOST),
                ));
            }
            Line::from(spans)
        }
        GateStatus::Skipped => Line::from(vec![
            Span::styled(connector.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(padded_name, theme.muted()),
            Span::styled("skipped".to_string(), theme.muted()),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_verdicts() {
        let theme = Theme::dark();
        let data = GateBlockData::from_verdicts(&[
            ("compile".into(), true),
            ("test".into(), false),
            ("clippy".into(), true),
        ]);
        let lines = data.to_lines(&theme);
        // header + 3 rungs
        assert_eq!(lines.len(), 4);
    }

    #[test]
    fn all_statuses_render() {
        let theme = Theme::dark();
        let data = GateBlockData {
            policy: Some("prod-sec".into()),
            rungs: vec![
                GateRung { name: "compile".into(), status: GateStatus::Passed { summary: "0 errors".into(), duration_s: 2.1 } },
                GateRung { name: "test".into(), status: GateStatus::Running { elapsed_s: 3.2, progress: Some("4/11".into()) } },
                GateRung { name: "clippy".into(), status: GateStatus::Pending },
                GateRung { name: "diff".into(), status: GateStatus::Failed { reason: "3 files changed".into(), duration_s: 0.5 } },
                GateRung { name: "verify".into(), status: GateStatus::Skipped },
            ],
        };
        let lines = data.to_lines(&theme);
        assert_eq!(lines.len(), 6); // header + 5 rungs
    }
}
