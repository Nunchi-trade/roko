//! Task breakdown modal.
//!
//! Displays task details, phase, outcome, and related gate verdicts in a
//! bordered scrollable overlay.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use roko_core::dashboard_snapshot::{GateVerdict, TaskState};

use crate::tui::dashboard::Theme;

/// Render the task detail modal overlay.
///
/// Caller should pass an area produced by `centered_rect(86, 84, frame.area())`.
pub fn render_task_detail_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    task: &TaskState,
    gates: &[GateVerdict],
    scroll: u16,
    theme: &Theme,
) {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Task: {} ", task.task_id))
        .border_style(theme.warning());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(8), Constraint::Min(0)])
        .split(inner);

    // Task metadata header.
    let phase_style = match task.phase.as_str() {
        "completed" => theme.success(),
        _ => theme.warning(),
    };

    let outcome_str = task.outcome.as_deref().unwrap_or("in progress");
    let outcome_style = match task.outcome.as_deref() {
        Some(o) if o.contains("fail") || o.contains("error") => theme.danger(),
        Some(_) => theme.success(),
        None => theme.muted(),
    };

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Task ID:  ", theme.muted()),
            Span::styled(&task.task_id, theme.text().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Plan:     ", theme.muted()),
            Span::styled(&task.plan_id, theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Phase:    ", theme.muted()),
            Span::styled(&task.phase, phase_style),
        ]),
        Line::from(vec![
            Span::styled("Outcome:  ", theme.muted()),
            Span::styled(outcome_str, outcome_style),
        ]),
        Line::from(""),
    ];

    let header = Paragraph::new(header_lines)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(header, chunks[0]);

    // Gate verdicts for this task.
    let mut lines: Vec<Line> = Vec::new();

    let task_gates: Vec<_> = gates
        .iter()
        .filter(|g| g.task_id == task.task_id && g.plan_id == task.plan_id)
        .collect();

    if task_gates.is_empty() {
        lines.push(Line::from(Span::styled(
            "No gate verdicts for this task.",
            theme.muted(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "Gate Verdicts",
            theme.accent().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        let passed_count = task_gates.iter().filter(|g| g.passed).count();
        let failed_count = task_gates.len() - passed_count;

        lines.push(Line::from(vec![
            Span::styled(format!("  {passed_count} passed"), theme.success()),
            Span::styled("  /  ", theme.muted()),
            Span::styled(
                format!("{failed_count} failed"),
                if failed_count > 0 {
                    theme.danger()
                } else {
                    theme.muted()
                },
            ),
        ]));
        lines.push(Line::from(""));

        for g in &task_gates {
            let icon = if g.passed { "+" } else { "x" };
            let style = if g.passed {
                theme.success()
            } else {
                theme.danger()
            };

            let ts = format_ts(g.ts_millis);

            lines.push(Line::from(vec![
                Span::styled(format!("  [{icon}] "), style),
                Span::styled(
                    format!("{:<20}", g.gate),
                    theme.text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(ts, theme.muted()),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, chunks[1]);
}

/// Format a Unix-millis timestamp as a short time string.
fn format_ts(ts_millis: u64) -> String {
    use std::time::{Duration, UNIX_EPOCH};

    let dt = UNIX_EPOCH + Duration::from_millis(ts_millis);
    let secs = dt.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();

    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
