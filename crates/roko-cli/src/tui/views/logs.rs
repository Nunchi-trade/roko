//! Error and event log view.
//!
//! Displays recent errors and gate verdicts in chronological order.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the error and event log view.
pub fn render_logs_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    scroll: u16,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Logs ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();

    // Errors section.
    if !snapshot.errors.is_empty() {
        lines.push(Line::from(Span::styled("Errors", theme.danger())));
        lines.push(Line::from(""));

        for err in snapshot.errors.iter().rev() {
            lines.push(Line::from(vec![
                Span::styled(format_ts(err.ts_millis), theme.muted()),
                Span::styled("  ", theme.text()),
                Span::styled(&err.message, theme.danger()),
            ]));
        }

        lines.push(Line::from(""));
    }

    // Recent gate verdicts.
    if !snapshot.gates.is_empty() {
        lines.push(Line::from(Span::styled(
            "Recent Gate Verdicts",
            theme.accent(),
        )));
        lines.push(Line::from(""));

        for g in snapshot.gates.iter().rev().take(50) {
            let icon = if g.passed { "+" } else { "x" };
            let style = if g.passed {
                theme.success()
            } else {
                theme.danger()
            };
            lines.push(Line::from(vec![
                Span::styled(format_ts(g.ts_millis), theme.muted()),
                Span::styled(format!("  [{icon}] "), style),
                Span::styled(
                    format!("{}/{}: {}", g.plan_id, g.task_id, g.gate),
                    theme.text(),
                ),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled("No log entries.", theme.muted())));
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);
}

/// Format a Unix-millis timestamp as a short time string.
fn format_ts(ts_millis: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let dt = UNIX_EPOCH + Duration::from_millis(ts_millis);
    let secs = dt
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // HH:MM:SS UTC.
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
