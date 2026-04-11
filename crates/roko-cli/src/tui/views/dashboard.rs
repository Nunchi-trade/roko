//! Master-detail dashboard view.
//!
//! Left panel shows the plan list with phase and progress; the right panel
//! shows task detail for the selected plan.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Table, Wrap};

use roko_core::dashboard_snapshot::DashboardSnapshot;

use crate::tui::dashboard::Theme;

/// Render the master-detail dashboard view.
///
/// The left 40% shows the plan list; the right 60% shows tasks for the
/// selected plan.
pub fn render_dashboard_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    selected_plan: usize,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    render_plan_list(frame, chunks[0], snapshot, selected_plan, theme);
    render_plan_detail(frame, chunks[1], snapshot, selected_plan, theme);
}

// ---- left panel: plan list ----

fn render_plan_list(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    selected: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Plans ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut plans: Vec<_> = snapshot.plans.values().collect();
    plans.sort_by(|a, b| a.plan_id.cmp(&b.plan_id));

    if plans.is_empty() {
        let empty = Paragraph::new("No plans active.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem> = plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let marker = if plan.active { ">" } else { " " };
            let progress = if plan.tasks_total > 0 {
                format!("{}/{} done", plan.tasks_done, plan.tasks_total)
            } else {
                String::from("no tasks")
            };

            let style = if i == selected {
                theme.selection()
            } else if plan.active {
                theme.text()
            } else {
                theme.muted()
            };

            let line = Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(&plan.plan_id, style.add_modifier(Modifier::BOLD)),
                Span::styled(format!("  [{}]  {}", plan.phase, progress), style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ---- right panel: selected plan detail ----

fn render_plan_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DashboardSnapshot,
    selected: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tasks ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut plans: Vec<_> = snapshot.plans.values().collect();
    plans.sort_by(|a, b| a.plan_id.cmp(&b.plan_id));

    let Some(plan) = plans.get(selected) else {
        let empty = Paragraph::new("Select a plan.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    };

    // Split detail area: progress gauge + task table.
    let detail_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    // Progress gauge.
    let ratio = if plan.tasks_total > 0 {
        plan.tasks_done as f64 / plan.tasks_total as f64
    } else {
        0.0
    };
    let gauge = Gauge::default()
        .block(Block::default().title(format!("{} progress", plan.plan_id)))
        .gauge_style(theme.success())
        .percent((ratio * 100.0) as u16);
    frame.render_widget(gauge, detail_chunks[0]);

    // Task table.
    let tasks: Vec<_> = snapshot
        .tasks
        .values()
        .filter(|t| t.plan_id == plan.plan_id)
        .collect();

    if tasks.is_empty() {
        let empty = Paragraph::new("No tasks recorded.")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, detail_chunks[1]);
        return;
    }

    let header = Row::new(vec![
        Cell::from("Task").style(theme.accent()),
        Cell::from("Phase").style(theme.accent()),
        Cell::from("Outcome").style(theme.accent()),
    ]);

    let rows: Vec<Row> = tasks
        .iter()
        .map(|task| {
            let outcome_style = match task.outcome.as_deref() {
                Some(o) if o.contains("fail") || o.contains("error") => theme.danger(),
                Some(_) => theme.success(),
                None => theme.muted(),
            };
            Row::new(vec![
                Cell::from(task.task_id.as_str()).style(theme.text()),
                Cell::from(task.phase.as_str()).style(theme.warning()),
                Cell::from(task.outcome.as_deref().unwrap_or("-")).style(outcome_style),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(30),
            Constraint::Percentage(25),
            Constraint::Percentage(45),
        ],
    )
    .header(header)
    .column_spacing(1);

    frame.render_widget(table, detail_chunks[1]);
}

// ---- gate verdict sidebar lines ----

fn gate_lines(snapshot: &DashboardSnapshot, plan_id: &str, theme: &Theme) -> Vec<Line<'static>> {
    snapshot
        .gates
        .iter()
        .filter(|g| g.plan_id == plan_id)
        .map(|g| {
            let icon = if g.passed { "+" } else { "x" };
            let style = if g.passed {
                theme.success()
            } else {
                theme.danger()
            };
            Line::from(Span::styled(
                format!(
                    "[{icon}] {} / {} : {}",
                    g.task_id,
                    g.gate,
                    if g.passed { "pass" } else { "fail" }
                ),
                style,
            ))
        })
        .collect()
}
