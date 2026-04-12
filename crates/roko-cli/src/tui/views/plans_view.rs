//! F2 Plans view -- wave browser + plan detail.
//!
//! Two-panel layout: left 40% wave browser with hierarchical
//! wave->plan list (expand/collapse), right 60% plan detail panel
//! with task list and phase timeline.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
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

    // Pipeline header with progress bar
    let total_plans = data.plans.len();
    let completed = data.plans.iter().filter(|p| p.completed).count();
    let pct = if total_plans > 0 {
        completed as f64 / total_plans as f64
    } else {
        0.0
    };
    let bar_w = 12;
    let filled = (pct * bar_w as f64).round() as usize;
    let bar = format!(
        "{}{}",
        "\u{2588}".repeat(filled.min(bar_w)),
        "\u{2500}".repeat(bar_w.saturating_sub(filled)),
    );
    let bar_color = if completed == total_plans && total_plans > 0 {
        Color::Green
    } else if completed > 0 {
        Color::Yellow
    } else {
        Color::DarkGray
    };

    let header_line = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("{completed}/{total_plans}"),
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(bar_color)),
        Span::styled(
            format!(" {:.0}%", pct * 100.0),
            Style::default().fg(bar_color),
        ),
    ]);
    let header = Paragraph::new(header_line).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Pipeline ")
            .border_style(theme.accent()),
    );
    frame.render_widget(header, sections[0]);

    // Plan list
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

    let items: Vec<ListItem<'_>> = data
        .plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let (icon, icon_style) = if plan.completed {
                ("\u{2713}", Style::default().fg(Color::Green))
            } else {
                ("\u{25cb}", Style::default().fg(Color::DarkGray))
            };

            let style = if i == view_state.selected {
                theme.selection()
            } else if plan.completed {
                theme.success()
            } else {
                theme.text()
            };

            let count_style = if plan.completed {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {icon} "), icon_style),
                Span::styled(&plan.title, style),
                Span::styled(
                    format!("  {}", plan.task_count),
                    count_style,
                ),
            ]))
            .style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Right panel: plan detail with task table, phase, model, and gate info.
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
        let sections = Layout::vertical([
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(6),
        ])
        .split(inner);

        // Plan header with progress bar
        let pct = if exec.tasks_total > 0 {
            exec.tasks_done as f64 / exec.tasks_total as f64
        } else {
            0.0
        };
        let bar_w = 16;
        let filled = (pct * bar_w as f64).round() as usize;
        let bar = format!(
            "{}{}",
            "\u{2588}".repeat(filled.min(bar_w)),
            "\u{2500}".repeat(bar_w.saturating_sub(filled)),
        );
        let bar_color = if exec.tasks_done == exec.tasks_total && exec.tasks_total > 0 {
            Color::Green
        } else {
            Color::Yellow
        };

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
                Span::raw(" "),
                Span::styled(bar, Style::default().fg(bar_color)),
                Span::styled(
                    format!(" {:.0}%", pct * 100.0),
                    Style::default().fg(bar_color),
                ),
            ]),
        ]);
        frame.render_widget(header, sections[0]);

        // Task table with enhanced columns
        render_execution_tasks(frame, sections[1], exec, view_state, theme);

        // Gate results summary at bottom
        render_gate_summary(frame, sections[2], data, theme);
        return;
    }

    // Fallback: show selected plan summary with enhanced metadata
    if let Some(plan) = data.plans.get(view_state.selected) {
        let sections = Layout::vertical([
            Constraint::Length(6),
            Constraint::Min(0),
        ])
        .split(inner);

        let status_style = if plan.completed {
            theme.success()
        } else {
            theme.warning()
        };

        let header_lines = vec![
            Line::from(vec![
                Span::styled("plan:   ", theme.muted()),
                Span::styled(&plan.title, theme.accent_bold()),
            ]),
            Line::from(vec![
                Span::styled("id:     ", theme.muted()),
                Span::raw(&plan.id),
            ]),
            Line::from(vec![
                Span::styled("tasks:  ", theme.muted()),
                Span::raw(plan.task_count.to_string()),
            ]),
            Line::from(vec![
                Span::styled("status: ", theme.muted()),
                if plan.completed {
                    Span::styled("completed", status_style)
                } else {
                    Span::styled("pending", status_style)
                },
            ]),
        ];

        // Add gate results for this plan if available
        let plan_gates: Vec<_> = data
            .gate_results
            .iter()
            .filter(|g| g.plan_id == plan.id)
            .collect();
        if !plan_gates.is_empty() {
            let passed = plan_gates.iter().filter(|g| g.passed).count();
            let total_gates = plan_gates.len();
            let gate_color = if passed == total_gates {
                Color::Green
            } else {
                Color::Red
            };
            let mut lines = header_lines;
            lines.push(Line::from(vec![
                Span::styled("gates:  ", theme.muted()),
                Span::styled(
                    format!("{passed}/{total_gates} passed"),
                    Style::default().fg(gate_color),
                ),
            ]));
            let header = Paragraph::new(lines);
            frame.render_widget(header, sections[0]);
        } else {
            let header = Paragraph::new(header_lines);
            frame.render_widget(header, sections[0]);
        }

        // Show matching active tasks with more detail
        let matching_tasks: Vec<_> = data
            .active_tasks
            .iter()
            .filter(|t| t.plan_id == plan.id)
            .collect();

        if !matching_tasks.is_empty() {
            let rows: Vec<Row<'_>> = matching_tasks
                .iter()
                .enumerate()
                .map(|(i, task)| {
                    let (icon, icon_style) = match task.status.as_str() {
                        "done" | "completed" => ("\u{2713}", Style::default().fg(Color::Green)),
                        "running" | "in_progress" => (
                            "\u{25ba}",
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        ),
                        "failed" => ("\u{2717}", Style::default().fg(Color::Red)),
                        _ => ("\u{00b7}", Style::default().fg(Color::DarkGray)),
                    };

                    let style = if i == view_state.secondary_selected {
                        theme.selection()
                    } else {
                        theme.text()
                    };

                    // Iteration info
                    let iter_str = if task.iteration > 0 {
                        format!("#{}", task.iteration)
                    } else {
                        "-".to_string()
                    };

                    // Agents assigned
                    let agents = if task.assigned_agents.is_empty() {
                        "-".to_string()
                    } else {
                        task.assigned_agents.join(", ")
                    };

                    Row::new(vec![
                        Cell::from(Span::styled(format!("{icon}"), icon_style)),
                        Cell::from(truncate(&task.task_id, 18)),
                        Cell::from(task.status.as_str()),
                        Cell::from(iter_str),
                        Cell::from(truncate(&agents, 14)),
                    ])
                    .style(style)
                })
                .collect();

            let widths = [
                Constraint::Length(2),
                Constraint::Min(12),
                Constraint::Length(10),
                Constraint::Length(5),
                Constraint::Min(10),
            ];
            let table = Table::new(rows, widths)
                .header(
                    Row::new([" ", "task", "status", "iter", "agents"])
                        .style(theme.accent().add_modifier(Modifier::BOLD)),
                )
                .column_spacing(1);
            frame.render_widget(table, sections[1]);
        } else {
            let empty = Paragraph::new("no active tasks for this plan")
                .style(theme.muted())
                .wrap(Wrap { trim: false });
            frame.render_widget(empty, sections[1]);
        }
    } else {
        let empty = Paragraph::new("select a plan from the left panel")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
    }
}

/// Render the task table for an active execution with enhanced columns.
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

    let rows: Vec<Row<'_>> = exec
        .tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            // Status icon
            let (icon, icon_color) = if task.is_current {
                ("\u{25ba}", Color::Yellow)
            } else if task.phase == "done" || task.phase == "completed" {
                ("\u{2713}", Color::Green)
            } else if task.phase == "failed" {
                ("\u{2717}", Color::Red)
            } else {
                ("\u{00b7}", Color::DarkGray)
            };

            let style = if task.is_current {
                theme.info()
            } else if i == view_state.secondary_selected {
                theme.selection()
            } else {
                theme.text()
            };

            // Shorten model name for display
            let short_model = if task.model.is_empty() {
                "-".to_string()
            } else {
                shorten_model(&task.model)
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{icon}"),
                    Style::default().fg(icon_color),
                )),
                Cell::from(truncate(&task.task_id, 14)),
                Cell::from(truncate(&task.title, 22)),
                Cell::from(task.phase.as_str()),
                Cell::from(short_model),
                Cell::from(task.duration.as_str()),
            ])
            .style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Min(10),
        Constraint::Min(14),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "task", "title", "phase", "model", "time"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, area);
}

/// Render gate results summary at bottom of plan detail.
fn render_gate_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title(" Gate Results ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.gate_results.is_empty() && data.gate_results_page.gate_rows.is_empty() {
        let empty = Paragraph::new("no gate results yet")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Show gate summary rows if available
    if !data.gate_results_page.gate_rows.is_empty() {
        let rows: Vec<Row<'_>> = data
            .gate_results_page
            .gate_rows
            .iter()
            .take(inner.height as usize)
            .map(|row| {
                let rate_color = if row.pass_rate >= 0.9 {
                    Color::Green
                } else if row.pass_rate >= 0.5 {
                    Color::Yellow
                } else {
                    Color::Red
                };
                Row::new(vec![
                    Cell::from(truncate(&row.gate_name, 14)),
                    Cell::from(format!("{}", row.total_runs)),
                    Cell::from(Span::styled(
                        format!("{:.0}%", row.pass_rate * 100.0),
                        Style::default().fg(rate_color),
                    )),
                    Cell::from(format!("{:.0}ms", row.avg_duration_ms)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(8),
        ];
        let table = Table::new(rows, widths)
            .header(
                Row::new(["gate", "runs", "pass%", "avg ms"])
                    .style(theme.accent().add_modifier(Modifier::BOLD)),
            )
            .column_spacing(1);
        frame.render_widget(table, inner);
    } else {
        // Fall back to raw gate results
        let passed = data.gate_results.iter().filter(|g| g.passed).count();
        let total = data.gate_results.len();
        let color = if passed == total && total > 0 {
            Color::Green
        } else if passed > 0 {
            Color::Yellow
        } else {
            Color::Red
        };
        let summary = Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{passed}/{total} gates passed"),
                Style::default().fg(color),
            ),
        ]));
        frame.render_widget(summary, inner);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shorten model slug for compact display.
fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
