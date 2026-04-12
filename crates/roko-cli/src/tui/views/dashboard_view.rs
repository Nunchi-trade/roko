//! F1 Dashboard view -- master-detail layout with 7 sub-tabs.
//!
//! Left panel (38%): plan tree + phase compact + task progress.
//! Right panel (62%): sub-tabbed detail view (Agents, Output, Diff,
//! Errors, Git, Context/MCP, Processes).
//! Wave progress ribbon at bottom.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

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
    _tui_state: &TuiState,
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

// ---------------------------------------------------------------------------
// Plan tree — rich rendering with colored counts, progress bars, status icons
// ---------------------------------------------------------------------------

/// Plan tree: hierarchical plan listing with colored task counts and progress.
fn render_plan_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let total = data.plans.len();
    let completed = data.plans.iter().filter(|p| p.completed).count();
    let pending = total.saturating_sub(completed);

    // Build title with colored counts
    let title = format!(
        " Plans ({completed}/{total}) ",
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
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

    let bar_width = 10usize;

    let items: Vec<ListItem<'_>> = data
        .plans
        .iter()
        .enumerate()
        .map(|(i, plan)| {
            let is_selected = i == view_state.selected;

            // Status icon with semantic color
            let (icon, icon_style) = if plan.completed {
                ("\u{2713}", Style::default().fg(Color::Green))      // checkmark green
            } else {
                ("\u{25cb}", Style::default().fg(Color::DarkGray))   // circle gray
            };

            // Task count coloring: green for all done, yellow for in-progress, gray for none
            let count_style = if plan.completed {
                Style::default().fg(Color::Green)
            } else if plan.task_count > 0 {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Progress bar: filled blocks for done ratio
            let done_count = if plan.completed { plan.task_count } else { 0 };
            let fill_pct = if plan.task_count > 0 {
                done_count as f64 / plan.task_count as f64
            } else {
                0.0
            };
            let filled = (fill_pct * bar_width as f64).round() as usize;
            let empty = bar_width.saturating_sub(filled);

            let bar_color = if plan.completed {
                Color::Green
            } else if done_count > 0 {
                Color::Yellow
            } else {
                Color::DarkGray
            };

            let bar_str = format!(
                "{}{}",
                "\u{2588}".repeat(filled.min(bar_width)),
                "\u{2500}".repeat(empty),
            );

            // Selection style
            let text_style = if is_selected {
                theme.selection()
            } else if plan.completed {
                theme.success()
            } else {
                theme.text()
            };

            // Truncate name
            let max_name = (inner.width as usize).saturating_sub(bar_width + 18);
            let name = truncate(&plan.title, max_name);

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {icon} "), icon_style),
                Span::styled(name, text_style),
                Span::raw(" "),
                Span::styled(
                    format!("{:>2}/{:<2}", done_count, plan.task_count),
                    count_style,
                ),
                Span::raw(" "),
                Span::styled(bar_str, Style::default().fg(bar_color)),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

// ---------------------------------------------------------------------------
// Phase compact — segmented phase bar with stage names
// ---------------------------------------------------------------------------

/// Phase compact: segmented bar showing execution stages.
fn render_phase_compact(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    // Derive phase info from execution state
    let (phase_label, phase_pct) = if let Some(exec) = &data.current_plan_execution {
        let pct = if exec.tasks_total > 0 {
            exec.tasks_done as f64 / exec.tasks_total as f64
        } else {
            0.0
        };
        // Derive stage name from task phases
        let current_phase = exec
            .tasks
            .iter()
            .find(|t| t.is_current)
            .map(|t| t.phase.as_str())
            .unwrap_or("idle");
        (current_phase.to_string(), pct)
    } else {
        ("idle".to_string(), 0.0)
    };

    let title = format!(" Phase \u{00b7} {} ", phase_label);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 1 || inner.width < 10 {
        return;
    }

    let bar_width = inner.width as usize;
    let mut lines: Vec<Line<'_>> = Vec::new();

    // Line 1: Segmented phase bar
    // Define canonical phases and determine which are done/active/pending
    let phase_stages = ["preflight", "implement", "verify", "gate", "merge"];
    let seg_width = bar_width / phase_stages.len().max(1);
    let leftover = bar_width.saturating_sub(seg_width * phase_stages.len());

    let mut bar_spans: Vec<Span<'_>> = Vec::new();

    for (i, &stage) in phase_stages.iter().enumerate() {
        let w = if i == phase_stages.len() - 1 {
            seg_width + leftover
        } else {
            seg_width
        };

        // Determine if this stage is done, active, or pending based on overall progress
        let stage_threshold = (i as f64 + 1.0) / phase_stages.len() as f64;
        let prev_threshold = i as f64 / phase_stages.len() as f64;

        let (fill_char, fill_color) = if phase_pct >= stage_threshold {
            // Stage complete
            ("\u{2588}", Color::Green)
        } else if phase_pct > prev_threshold {
            // Stage active - partial fill
            ("\u{2593}", Color::Yellow)
        } else {
            // Stage pending
            ("\u{2500}", Color::DarkGray)
        };

        bar_spans.push(Span::styled(
            fill_char.repeat(w),
            Style::default().fg(fill_color),
        ));
    }
    lines.push(Line::from(bar_spans));

    // Line 2: Stage labels below the bar
    if inner.height >= 2 {
        let mut label_spans: Vec<Span<'_>> = Vec::new();
        for (i, &stage) in phase_stages.iter().enumerate() {
            let w = if i == phase_stages.len() - 1 {
                seg_width + leftover
            } else {
                seg_width
            };
            let stage_threshold = (i as f64 + 1.0) / phase_stages.len() as f64;
            let prev_threshold = i as f64 / phase_stages.len() as f64;

            let label_color = if phase_pct >= stage_threshold {
                Color::Green
            } else if phase_pct > prev_threshold {
                Color::Yellow
            } else {
                Color::DarkGray
            };

            let padded = format!("{:^width$}", stage, width = w);
            label_spans.push(Span::styled(padded, Style::default().fg(label_color)));
        }
        lines.push(Line::from(label_spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Task progress — table with elapsed time and iteration counts
// ---------------------------------------------------------------------------

/// Task progress: list of active tasks with elapsed time and iteration.
fn render_task_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let total = data.active_tasks.len();
    let done = data
        .active_tasks
        .iter()
        .filter(|t| t.status == "done" || t.status == "completed")
        .count();

    let title = format!(" Tasks ({done}/{total}) ");
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.active_tasks.is_empty() {
        let empty = Paragraph::new("no active tasks")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Build enhanced rows with status icon, elapsed time, and iteration
    let rows: Vec<Row<'_>> = data
        .active_tasks
        .iter()
        .enumerate()
        .map(|(i, task)| {
            // Status icon
            let (icon, status_style) = match task.status.as_str() {
                "done" | "completed" => ("\u{2713}", Style::default().fg(Color::Green)),
                "running" | "in_progress" => ("\u{25ba}", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                "failed" => ("\u{2717}", Style::default().fg(Color::Red)),
                "blocked" => ("\u{2717}", Style::default().fg(Color::Red)),
                _ => ("\u{00b7}", Style::default().fg(Color::DarkGray)),
            };

            let row_style = if i == view_state.secondary_selected {
                theme.selection()
            } else {
                match task.status.as_str() {
                    "done" | "completed" => theme.success(),
                    "running" | "in_progress" => theme.info(),
                    "failed" => theme.danger(),
                    _ => theme.text(),
                }
            };

            // Elapsed time from efficiency events (find matching task)
            let elapsed_ms: u64 = data
                .efficiency_events
                .iter()
                .filter(|e| e.task_id == task.task_id)
                .map(|e| e.wall_time_ms)
                .sum();
            let elapsed_str = if elapsed_ms > 0 {
                format_duration_ms(elapsed_ms as f64)
            } else {
                "-".to_string()
            };

            // Iteration count
            let iter_str = if task.iteration > 0 {
                format!("#{}", task.iteration)
            } else {
                "-".to_string()
            };

            Row::new(vec![
                Cell::from(Span::styled(format!(" {icon}"), status_style)),
                Cell::from(truncate(&task.task_id, 18)),
                Cell::from(task.status.as_str()),
                Cell::from(elapsed_str),
                Cell::from(iter_str),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Min(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(6),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "task", "status", "elapsed", "iter"])
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

// ---------------------------------------------------------------------------
// Sub-tab: Agents — roster with model name and token counts
// ---------------------------------------------------------------------------

/// Sub-tab: Agents roster with model and token info.
fn render_sub_agents(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let active_count = data
        .agents
        .iter()
        .filter(|a| a.status == "running" || a.status == "active")
        .count();
    let title = format!(" Agents ({} active) ", active_count);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.agents.is_empty() {
        let empty = Paragraph::new("no active agents")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Build agent activity snapshot for model/token info
    let activity =
        crate::tui::dashboard::build_agent_activity_snapshot(&data.agents, &data.efficiency_events);

    let rows: Vec<Row<'_>> = data
        .agents
        .iter()
        .map(|agent| {
            let status_style = match agent.status.as_str() {
                "running" | "active" => theme.info(),
                "idle" => theme.muted(),
                "error" | "failed" => theme.danger(),
                _ => theme.text(),
            };

            // Find matching activity row for model/token info
            let activity_row = activity.as_ref().and_then(|snap| {
                snap.active_agents
                    .iter()
                    .find(|row| row.agent_id == agent.id)
            });

            let model_str = activity_row
                .map(|r| shorten_model(&r.model))
                .unwrap_or_else(|| "-".to_string());

            let tokens_str = activity_row
                .map(|r| format_tokens(r.tokens_used))
                .unwrap_or_else(|| "-".to_string());

            let cost_str = activity_row
                .map(|r| {
                    if r.cost_usd > 0.0 {
                        format!("${:.3}", r.cost_usd)
                    } else {
                        "-".to_string()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            Row::new(vec![
                Cell::from(truncate(&agent.id, 14)),
                Cell::from(agent.label.as_str()),
                Cell::from(Span::styled(agent.status.as_str(), status_style)),
                Cell::from(model_str),
                Cell::from(tokens_str),
                Cell::from(cost_str),
            ])
        })
        .collect();

    let widths = [
        Constraint::Min(10),
        Constraint::Min(12),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["id", "label", "status", "model", "tokens", "cost"])
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

// ---------------------------------------------------------------------------
// Wave progress ribbon — fire-gradient gauge at bottom
// ---------------------------------------------------------------------------

/// Wave progress ribbon with fire-gradient coloring.
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

    if inner.width < 4 || inner.height < 1 {
        return;
    }

    let bar_width = inner.width as usize;
    let filled = (ratio.clamp(0.0, 1.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    // Fire gradient: dark red -> orange -> yellow -> white at full
    let mut bar_spans: Vec<Span<'_>> = Vec::new();
    for i in 0..filled.min(bar_width) {
        let t = if filled > 1 {
            i as f64 / (filled - 1) as f64
        } else {
            ratio
        };
        let color = fire_gradient(t);
        bar_spans.push(Span::styled(
            "\u{2588}",
            Style::default().fg(color),
        ));
    }
    if empty > 0 {
        bar_spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Append label
    let label = format!("  {done}/{total} tasks");
    bar_spans.push(Span::styled(label, Style::default().fg(Color::White)));

    let line = Line::from(bar_spans);
    let paragraph = Paragraph::new(line);
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fire gradient: interpolate from deep red through orange to bright yellow.
fn fire_gradient(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    // Red channel: always high
    let r = (180.0 + t * 75.0).min(255.0) as u8;
    // Green channel: ramps up from dark to bright
    let g = (t * 200.0).min(255.0) as u8;
    // Blue channel: stays very low, slight rise at end
    let b = (t * t * 60.0).min(255.0) as u8;
    Color::Rgb(r, g, b)
}

/// Shorten a model slug for compact display.
fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

/// Format token count compactly.
fn format_tokens(n: u64) -> String {
    if n == 0 {
        "-".to_string()
    } else if n < 1_000 {
        format!("{n}")
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Format duration from milliseconds to compact string.
fn format_duration_ms(ms: f64) -> String {
    let secs = (ms / 1000.0) as u64;
    if secs >= 3600 {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else if secs > 0 {
        format!("{}s", secs)
    } else {
        format!("{:.0}ms", ms)
    }
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
