//! Full plan browser view.
//!
//! Shows all plans with expanded task lists, phase indicators, and progress.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Table, Wrap};

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the full plan browser view with expanded task lists.
pub fn render_plans_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    selected: usize,
    scroll: u16,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Plan Browser ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut plans: Vec<_> = snapshot.plans.values().collect();
    plans.sort_by(|a, b| a.plan_id.cmp(&b.plan_id));

    if plans.is_empty() {
        let empty = Paragraph::new("No plans discovered.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Build a scrollable list of plan sections, each with their tasks inlined.
    let mut lines: Vec<Line> = Vec::new();

    for (i, plan) in plans.iter().enumerate() {
        let is_selected = i == selected;

        // Plan header line.
        let active_marker = if plan.active { " ACTIVE" } else { "" };
        let header_style = if is_selected {
            theme.selection()
        } else {
            theme.accent()
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", plan.plan_id),
                header_style.add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("[{}]{active_marker}", plan.phase), header_style),
        ]));

        // Progress summary.
        let total = plan.tasks_total;
        let done = plan.tasks_done;
        let failed = plan.tasks_failed;
        let pending = total.saturating_sub(done).saturating_sub(failed);

        lines.push(Line::from(Span::styled(
            format!("    {done} done / {failed} failed / {pending} pending (of {total})"),
            theme.muted(),
        )));

        // Tasks for this plan.
        let mut tasks: Vec<_> = snapshot
            .tasks
            .values()
            .filter(|t| t.plan_id == plan.plan_id)
            .collect();
        tasks.sort_by(|a, b| a.task_id.cmp(&b.task_id));

        for task in &tasks {
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
                Span::styled("      ", theme.text()),
                Span::styled(&task.task_id, theme.text()),
                Span::styled("  ", theme.text()),
                Span::styled(&task.phase, phase_style),
                Span::styled("  ", theme.text()),
                Span::styled(outcome_str, outcome_style),
            ]));
        }

        // Gate verdicts for this plan.
        let gates: Vec<_> = snapshot
            .gates
            .iter()
            .filter(|g| g.plan_id == plan.plan_id)
            .collect();

        if !gates.is_empty() {
            lines.push(Line::from(Span::styled("    gates:", theme.muted())));
            for g in &gates {
                let icon = if g.passed { "+" } else { "x" };
                let style = if g.passed {
                    theme.success()
                } else {
                    theme.danger()
                };
                lines.push(Line::from(Span::styled(
                    format!("      [{icon}] {}: {}", g.task_id, g.gate),
                    style,
                )));
            }
        }

        // Separator between plans.
        lines.push(Line::from(""));
    }

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, inner);
}
