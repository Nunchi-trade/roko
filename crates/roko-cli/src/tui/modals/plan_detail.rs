//! Scrollable plan detail modal.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::plan::PlanSummary;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// Render the plan detail modal for the selected plan.
pub fn render_plan_detail_modal(
    frame: &mut Frame<'_>,
    area: Rect,
    plan_idx: usize,
    scroll_offset: usize,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let popup = centered_rect(86, 84, area);
    frame.render_widget(Clear, popup);

    let Some(plan) = data.plans.get(plan_idx) else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Plan Detail ")
            .title_alignment(Alignment::Center)
            .border_style(theme.warning());
        let inner = block.inner(popup);
        frame.render_widget(block, popup);
        frame.render_widget(
            Paragraph::new("Selected plan is no longer available.")
                .style(theme.muted())
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Plan Detail: {} ", plan.id))
        .title_alignment(Alignment::Center)
        .border_style(theme.warning());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    render_header(frame, sections[0], plan, plan_idx, data, tui_state, theme);
    render_tasks(
        frame,
        sections[1],
        plan,
        plan_idx,
        scroll_offset,
        data,
        tui_state,
        theme,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Esc]", theme.accent_bold()),
            Span::styled(" close  ", theme.muted()),
            Span::styled("[Up/Down]", theme.accent_bold()),
            Span::styled(" scroll", theme.muted()),
        ])),
        sections[2],
    );
}

fn render_header(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanSummary,
    plan_idx: usize,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let state_plan = tui_state.plans.get(plan_idx);
    let active_exec = data
        .current_plan_execution
        .as_ref()
        .filter(|exec| exec.plan_id == plan.id);

    let tasks_total = active_exec
        .map(|exec| exec.tasks_total)
        .or_else(|| state_plan.map(|entry| entry.tasks_total))
        .unwrap_or(plan.task_count);
    let tasks_done = active_exec
        .map(|exec| exec.tasks_done)
        .or_else(|| state_plan.map(|entry| entry.tasks_done))
        .unwrap_or_else(|| if plan.completed { plan.task_count } else { 0 });
    let tasks_failed = state_plan.map_or(0, |entry| entry.tasks_failed);
    let pct = if tasks_total > 0 {
        tasks_done as f64 / tasks_total as f64
    } else {
        0.0
    };
    let filled = (pct * 20.0).round() as usize;
    let progress_bar = format!(
        "{}{}",
        "#".repeat(filled.min(20)),
        "-".repeat(20usize.saturating_sub(filled.min(20)))
    );
    let status = if let Some(entry) = state_plan {
        if entry.active {
            ("ACTIVE", theme.warning())
        } else if entry.tasks_failed > 0 {
            ("FAILED", theme.danger())
        } else {
            ("COMPLETE", theme.success())
        }
    } else if plan.completed {
        ("COMPLETE", theme.success())
    } else {
        ("PENDING", theme.muted())
    };

    let header_lines = vec![
        Line::from(vec![
            Span::styled("Name: ", theme.muted()),
            Span::styled(&plan.title, theme.accent_bold()),
        ]),
        Line::from(vec![
            Span::styled("Status: ", theme.muted()),
            Span::styled(status.0, status.1),
            Span::styled("  Progress: ", theme.muted()),
            Span::styled(
                format!("{tasks_done}/{tasks_total}"),
                Style::default().fg(theme.foreground),
            ),
        ]),
        Line::from(vec![
            Span::styled("Bar: ", theme.muted()),
            Span::styled(progress_bar, status.1),
            Span::styled(
                format!(" {:.0}%", pct * 100.0),
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if tasks_failed > 0 {
                    format!("  failed: {tasks_failed}")
                } else {
                    String::new()
                },
                theme.danger(),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(header_lines)
            .style(theme.text())
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_tasks(
    frame: &mut Frame<'_>,
    area: Rect,
    plan: &PlanSummary,
    plan_idx: usize,
    scroll_offset: usize,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let mut lines: Vec<Line<'_>> = vec![
        Line::from(Span::styled(
            "Tasks",
            theme.accent().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(exec) = data
        .current_plan_execution
        .as_ref()
        .filter(|exec| exec.plan_id == plan.id)
    {
        for task in &exec.tasks {
            let phase_style = match task.phase.as_str() {
                "done" | "completed" => theme.success(),
                "failed" | "error" => theme.danger(),
                "running" | "in_progress" => theme.warning(),
                _ => theme.muted(),
            };
            let prefix = if task.is_current { "> " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(prefix, theme.text()),
                Span::styled(
                    format!("{:<16}", task.task_id),
                    theme.text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{:<12}", task.phase), phase_style),
                Span::styled(task.title.as_str(), theme.text()),
            ]));
        }
    } else {
        let active_tasks: Vec<_> = data
            .active_tasks
            .iter()
            .filter(|task| task.plan_id == plan.id)
            .collect();

        if !active_tasks.is_empty() {
            for task in active_tasks {
                let status_style = match task.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "failed" | "error" => theme.danger(),
                    "running" | "in_progress" => theme.warning(),
                    _ => theme.muted(),
                };
                lines.push(Line::from(vec![
                    Span::styled("  ", theme.text()),
                    Span::styled(
                        format!("{:<16}", task.task_id),
                        theme.text().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:<12}", task.status), status_style),
                    Span::styled(
                        if task.assigned_agents.is_empty() {
                            String::from("active")
                        } else {
                            format!("agents: {}", task.assigned_agents.join(", "))
                        },
                        theme.text(),
                    ),
                ]));
            }
        } else if let Some(state_plan) = tui_state.plans.get(plan_idx) {
            for task in &state_plan.tasks {
                let status_style = match task.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "failed" | "error" => theme.danger(),
                    "running" | "in_progress" => theme.warning(),
                    _ => theme.muted(),
                };
                let title = if task.name.is_empty() {
                    task.id.as_str()
                } else {
                    task.name.as_str()
                };
                lines.push(Line::from(vec![
                    Span::styled("  ", theme.text()),
                    Span::styled(
                        format!("{:<16}", task.id),
                        theme.text().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:<12}", task.status), status_style),
                    Span::styled(title, theme.text()),
                ]));
            }
        }
    }

    if lines.len() == 2 {
        lines.push(Line::from(Span::styled(
            "No task detail is currently available for this plan.",
            theme.muted(),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.text())
            .wrap(Wrap { trim: false })
            .scroll((scroll_offset.min(u16::MAX as usize) as u16, 0)),
        area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
