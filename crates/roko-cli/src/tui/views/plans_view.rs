//! F2 Plans view -- wave browser + plan detail.
//!
//! Two-panel layout: left 40% wave browser with hierarchical
//! wave->plan list (expand/collapse), right 60% plan detail panel
//! with task list and phase timeline.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// Render the full plans view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);

    render_wave_browser(frame, panels[0], data, view_state, theme);
    render_plan_detail(frame, panels[1], data, view_state, theme);
}

/// Left panel: wave browser with plan list.
fn render_wave_browser(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

    // Pipeline header
    let total_plans = data.plans.len();
    let completed = data.plans.iter().filter(|p| p.completed).count();
    let header_text = format!("Pipeline: {completed}/{total_plans} plans complete");
    let header = Paragraph::new(header_text)
        .style(theme.accent())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Overview ")
                .border_style(theme.accent()),
        );
    frame.render_widget(header, sections[0]);

    // Plan list grouped by completion status
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Waves / Plans ")
        .border_style(theme.muted());
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    if data.plans.is_empty() {
        let empty = Paragraph::new("no plans found")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // TODO: use plan_list widget here for proper wave grouping
    let items: Vec<ListItem<'_>> = data
        .plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let icon = if plan.completed { "[x]" } else { "[ ]" };
            let style = if i == view_state.selected {
                theme.selection()
            } else if plan.completed {
                theme.success()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(vec![
                Span::raw(format!("{icon} ")),
                Span::styled(&plan.title, style),
                Span::styled(
                    format!("  ({} tasks)", plan.task_count),
                    theme.muted(),
                ),
            ]))
            .style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Right panel: plan detail with task table and phase timeline.
fn render_plan_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Plan Detail ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Show execution detail if available
    if let Some(exec) = &data.current_plan_execution {
        let sections =
            Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(inner);

        // Plan header
        let header = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("plan: ", theme.muted()),
                Span::styled(&exec.plan_title, theme.accent_bold()),
            ]),
            Line::from(vec![
                Span::styled("progress: ", theme.muted()),
                Span::styled(
                    format!("{}/{}", exec.tasks_done, exec.tasks_total),
                    theme.info(),
                ),
            ]),
        ]);
        frame.render_widget(header, sections[0]);

        // Task table
        render_execution_tasks(frame, sections[1], exec, view_state, theme);
        return;
    }

    // Fallback: show selected plan summary
    if let Some(plan) = data.plans.get(view_state.selected) {
        let lines = vec![
            Line::from(vec![
                Span::styled("plan: ", theme.muted()),
                Span::styled(&plan.title, theme.accent_bold()),
            ]),
            Line::from(vec![
                Span::styled("id: ", theme.muted()),
                Span::raw(&plan.id),
            ]),
            Line::from(vec![
                Span::styled("tasks: ", theme.muted()),
                Span::raw(plan.task_count.to_string()),
            ]),
            Line::from(vec![
                Span::styled("status: ", theme.muted()),
                if plan.completed {
                    Span::styled("completed", theme.success())
                } else {
                    Span::styled("pending", theme.warning())
                },
            ]),
        ];
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    } else {
        let empty = Paragraph::new("select a plan from the left panel")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
    }
}

/// Render the task table for an active execution.
fn render_execution_tasks(
    frame: &mut Frame<'_>,
    area: Rect,
    exec: &crate::tui::dashboard::PlanExecutionSnapshot,
    view_state: &ViewState,
    theme: &Theme,
) {
    if exec.tasks.is_empty() {
        let empty = Paragraph::new("no tasks in execution")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, area);
        return;
    }

    // TODO: use phase_timeline widget here when available
    let rows: Vec<Row<'_>> = exec
        .tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let style = if task.is_current {
                theme.info()
            } else if i == view_state.secondary_selected {
                theme.selection()
            } else {
                theme.text()
            };
            Row::new(vec![
                Cell::from(truncate(&task.task_id, 16)),
                Cell::from(truncate(&task.title, 24)),
                Cell::from(task.phase.as_str()),
                Cell::from(truncate(&task.model, 14)),
                Cell::from(task.duration.as_str()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Min(12),
        Constraint::Min(16),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["task", "title", "phase", "model", "time"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
