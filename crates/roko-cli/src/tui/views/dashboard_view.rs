//! F1 Dashboard view -- master-detail layout with 7 sub-tabs.
//!
//! Left panel (38%): plan tree + phase compact + task progress.
//! Right panel (62%): sub-tabbed detail view (Agents, Output, Diff,
//! Errors, Git, Context/MCP, Processes).
//! Wave progress ribbon at bottom.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};

/// Sub-tabs within the dashboard detail panel.
const SUB_TAB_LABELS: &[&str] = &[
    "a:Agents",
    "o:Output",
    "d:Diff",
    "e:Errors",
    "g:Git",
    "m:MCP",
    "P:Procs",
];

/// Render the full dashboard view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);

    let main = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(outer[0]);

    render_left_panel(frame, main[0], data, view_state, theme);
    render_right_panel(frame, main[1], data, view_state, theme);
    render_wave_ribbon(frame, outer[1], data, theme);
}

/// Left panel: plan tree (top 50%), phase compact (15%), task progress (35%).
fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(15),
        Constraint::Percentage(35),
    ])
    .split(area);

    render_plan_tree(frame, sections[0], data, view_state, theme);
    render_phase_compact(frame, sections[1], data, theme);
    render_task_progress(frame, sections[2], data, view_state, theme);
}

/// Plan tree: hierarchical plan listing.
fn render_plan_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Plans ")
        .border_style(theme.accent());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.plans.is_empty() {
        let empty = Paragraph::new("no plans discovered")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // TODO: use plan_tree widget here when available
    let items: Vec<ListItem<'_>> = data
        .plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let marker = if plan.completed { "[x]" } else { "[ ]" };
            let style = if i == view_state.selected {
                theme.selection()
            } else if plan.completed {
                theme.success()
            } else {
                theme.text()
            };
            ListItem::new(Line::from(format!(
                "{} {} ({} tasks)",
                marker, plan.title, plan.task_count
            )))
            .style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Phase compact: current execution phase indicator.
fn render_phase_compact(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Phase ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use phase_timeline widget here when available
    let phase_text = if let Some(exec) = &data.current_plan_execution {
        format!(
            "{}: {}/{} tasks",
            exec.plan_title, exec.tasks_done, exec.tasks_total
        )
    } else {
        String::from("idle - no active execution")
    };

    let paragraph = Paragraph::new(phase_text)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Task progress: list of active tasks with status.
fn render_task_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Tasks ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use task_progress widget here when available
    if data.active_tasks.is_empty() {
        let empty = Paragraph::new("no active tasks")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = data
        .active_tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            let style = if i == view_state.secondary_selected {
                theme.selection()
            } else {
                match task.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "running" | "in_progress" => theme.info(),
                    "failed" => theme.danger(),
                    _ => theme.text(),
                }
            };
            Row::new(vec![
                Cell::from(truncate(&task.task_id, 20)),
                Cell::from(task.status.as_str()),
                Cell::from(format!("iter {}", task.iteration)),
            ])
            .style(style)
        })
        .collect();

    let widths = [Constraint::Min(14), Constraint::Length(12), Constraint::Length(8)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["task", "status", "iter"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Right panel: sub-tabbed detail view.
fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Sub-tab bar
    let sub_tab_idx = view_state.sub_tab.min(SUB_TAB_LABELS.len().saturating_sub(1));
    let spans: Vec<Span<'_>> = SUB_TAB_LABELS
        .iter()
        .enumerate()
        .flat_map(|(i, label)| {
            let style = if i == sub_tab_idx {
                theme.selection()
            } else {
                theme.muted()
            };
            let sep = if i + 1 < SUB_TAB_LABELS.len() {
                " | "
            } else {
                ""
            };
            vec![Span::styled(*label, style), Span::raw(sep)]
        })
        .collect();
    let tab_line = Paragraph::new(Line::from(spans));
    frame.render_widget(tab_line, sections[0]);

    // Sub-tab content
    match sub_tab_idx {
        0 => render_sub_agents(frame, sections[1], data, theme),
        1 => render_sub_output(frame, sections[1], data, view_state, theme),
        2 => render_sub_diff(frame, sections[1], data, theme),
        3 => render_sub_errors(frame, sections[1], data, theme),
        4 => render_sub_git(frame, sections[1], data, theme),
        5 => render_sub_mcp(frame, sections[1], data, theme),
        6 => render_sub_processes(frame, sections[1], data, theme),
        _ => {}
    }
}

/// Sub-tab: Agents roster summary.
fn render_sub_agents(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agents ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use agent_pool widget here when available
    if data.agents.is_empty() {
        let empty = Paragraph::new("no active agents")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = data
        .agents
        .iter()
        .map(|agent| {
            let status_style = match agent.status.as_str() {
                "running" => theme.info(),
                "idle" => theme.muted(),
                "error" | "failed" => theme.danger(),
                _ => theme.text(),
            };
            Row::new(vec![
                Cell::from(truncate(&agent.id, 16)),
                Cell::from(agent.label.as_str()),
                Cell::from(Span::styled(agent.status.as_str(), status_style)),
            ])
        })
        .collect();

    let widths = [Constraint::Min(12), Constraint::Min(14), Constraint::Length(10)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["id", "label", "status"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Sub-tab: Agent output tail.
fn render_sub_output(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Output ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<&str> = data
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if lines.is_empty() {
        let empty = Paragraph::new("no agent output yet")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let text: Vec<Line<'_>> = lines.iter().map(|l| Line::from(*l)).collect();
    let scroll = if view_state.auto_tail {
        text.len().saturating_sub(inner.height as usize) as u16
    } else {
        view_state.scroll
    };

    let paragraph = Paragraph::new(text)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

/// Sub-tab: Diff panel placeholder.
fn render_sub_diff(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Diff ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use diff_panel widget here when available
    let placeholder = Paragraph::new("diff panel: will show per-agent diffs")
        .style(theme.muted())
        .wrap(Wrap { trim: false });
    frame.render_widget(placeholder, inner);
}

/// Sub-tab: Error digest.
fn render_sub_errors(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Errors ")
        .border_style(theme.danger());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let failures = &data.gate_results_page.failure_rows;
    if failures.is_empty() {
        let empty = Paragraph::new("no errors or gate failures")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = failures
        .iter()
        .take(inner.height as usize)
        .map(|row| {
            ListItem::new(Line::from(vec![
                Span::styled(&row.gate_name, theme.danger()),
                Span::raw(" "),
                Span::styled(&row.task_id, theme.muted()),
                Span::raw(": "),
                Span::raw(truncate(&row.error_excerpt, 50)),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Sub-tab: Git info.
fn render_sub_git(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Git ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Compact git summary for the dashboard sub-tab
    let placeholder = Paragraph::new("git summary: use F4 for full git view")
        .style(theme.muted())
        .wrap(Wrap { trim: false });
    frame.render_widget(placeholder, inner);
}

/// Sub-tab: MCP / Context status.
fn render_sub_mcp(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" MCP / Context ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Token summary from efficiency data
    let eff = &data.efficiency;
    let lines = vec![
        Line::from(vec![
            Span::styled("input tokens:  ", theme.muted()),
            Span::styled(
                format_count(eff.total_input_tokens),
                theme.info(),
            ),
        ]),
        Line::from(vec![
            Span::styled("output tokens: ", theme.muted()),
            Span::styled(
                format_count(eff.total_output_tokens),
                theme.info(),
            ),
        ]),
        Line::from(vec![
            Span::styled("total cost:    ", theme.muted()),
            Span::styled(
                format!("${:.4}", eff.total_cost_usd),
                theme.warning(),
            ),
        ]),
    ];
    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Sub-tab: Process table.
fn render_sub_processes(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Processes ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.agents.is_empty() {
        let empty = Paragraph::new("no tracked processes")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = data
        .agents
        .iter()
        .map(|agent| {
            Row::new(vec![
                Cell::from(truncate(&agent.id, 14)),
                Cell::from(agent.label.as_str()),
                Cell::from(
                    agent
                        .plan_id
                        .as_deref()
                        .unwrap_or("-"),
                ),
                Cell::from(agent.status.as_str()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(10),
        Constraint::Min(12),
        Constraint::Min(10),
        Constraint::Length(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["pid", "label", "plan", "status"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Wave progress ribbon at bottom of dashboard.
fn render_wave_ribbon(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Wave Progress ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use wave_bar widget here when available
    let (done, total) = data
        .current_plan_execution
        .as_ref()
        .map(|exec| (exec.tasks_done, exec.tasks_total))
        .unwrap_or((0, 0));

    let ratio = if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    };

    let gauge = Gauge::default()
        .gauge_style(theme.info())
        .label(format!("{done}/{total} tasks"))
        .ratio(ratio.clamp(0.0, 1.0));
    frame.render_widget(gauge, inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn format_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
