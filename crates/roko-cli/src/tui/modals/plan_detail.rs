//! Scrollable plan metadata modal.
//!
//! Displays plan details, task list, progress, and gate verdicts in a
//! bordered overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use roko_core::dashboard_snapshot::{GateVerdict, PlanState, TaskState};

use crate::tui::dashboard::Theme;

/// Render the plan detail modal overlay.
///
/// Caller should pass an area produced by `centered_rect(86, 84, frame.area())`.
pub fn render_plan_detail_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanState,
    tasks: &[TaskState],
    scroll: u16,
    theme: &Theme,
) {
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Plan: {} ", plan.plan_id))
        .border_style(theme.warning());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(0)])
        .split(inner);

    // Plan metadata header.
    let ratio = if plan.tasks_total > 0 {
        plan.tasks_done as f64 / plan.tasks_total as f64
    } else {
        0.0
    };

    let status_str = if plan.active { "ACTIVE" } else { "COMPLETE" };
    let status_style = if plan.active {
        theme.success()
    } else {
        theme.muted()
    };

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Status: ", theme.muted()),
            Span::styled(status_str, status_style),
            Span::styled(format!("  Phase: {}", plan.phase), theme.text()),
        ]),
        Line::from(vec![
            Span::styled("Tasks: ", theme.muted()),
            Span::styled(
                format!(
                    "{} total / {} done / {} failed",
                    plan.tasks_total, plan.tasks_done, plan.tasks_failed
                ),
                theme.text(),
            ),
        ]),
        Line::from(""),
    ];

    let header = Paragraph::new(header_lines)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(header, chunks[0]);

    // Task list (scrollable).
    if tasks.is_empty() {
        let empty = Paragraph::new("No tasks recorded for this plan.")
            .style(theme.muted())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        "Tasks",
        theme.accent().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for task in tasks {
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

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<24}", task.task_id),
                theme.text().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<14}", task.phase), phase_style),
            Span::styled(outcome_str, outcome_style),
        ]));
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, chunks[1]);
}
