//! F1 Dashboard view -- master-detail layout with 7 sub-tabs.
//!
//! Left panel (38%): plan tree + phase compact + task progress.
//! Right panel (62%): sub-tabbed detail view (Agents, Output, Diff,
//! Errors, Git, Context/MCP, Processes).
//! Bottom ribbon: wave progress + token sparkline + sys metrics.
//!
//! Calls real compiled widgets where available; falls back to inline
//! rendering when widget modules depend on uncompiled `mori_theme` /
//! `tui_state` types.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use roko_core::dashboard_snapshot::{ErrorEntry, GateVerdict, SnapshotStats};

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::input::FocusZone;
use crate::tui::state::TuiState;
use crate::tui::widgets;

// ---------------------------------------------------------------------------
// Sub-tab labels
// ---------------------------------------------------------------------------

const SUB_TAB_LABELS: &[(&str, &str)] = &[
    ("a", "Agents"),
    ("o", "Output"),
    ("d", "Diff"),
    ("e", "Errors"),
    ("g", "Git"),
    ("m", "MCP"),
    ("P", "Procs"),
];

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the full dashboard view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);
    let main = Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)])
        .split(outer[0]);

    render_left_panel(frame, main[0], data, tui_state, view_state, theme);
    render_right_panel(frame, main[1], data, tui_state, view_state, theme);
    render_bottom_ribbon(frame, outer[1], data, theme);
}

// ===========================================================================
// Left panel: plan tree (50%) + phase compact (15%) + task progress (35%)
// ===========================================================================

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(15),
        Constraint::Percentage(35),
    ])
    .split(area);

    let plan_focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let task_focused = matches!(tui_state.focus, FocusZone::TaskProgress);

    render_plan_tree(frame, sections[0], data, view_state, plan_focused, theme);
    render_phase_compact(frame, sections[1], data, theme);
    render_task_progress(frame, sections[2], data, view_state, task_focused, theme);
}

// ---------------------------------------------------------------------------
// Plan tree -- delegates to widgets::plan_list
// ---------------------------------------------------------------------------

fn render_plan_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let entries: Vec<widgets::plan_list::PlanEntry> = data
        .plans
        .iter()
        .map(|p| {
            let done = if p.completed { p.task_count as u32 } else { 0 };
            widgets::plan_list::PlanEntry {
                name: p.title.clone(),
                progress: if p.task_count > 0 { done as f64 / p.task_count as f64 } else { 0.0 },
                tasks_done: done,
                tasks_total: p.task_count as u32,
                failed: false,
            }
        })
        .collect();

    if entries.is_empty() {
        let border = if focused { theme.accent() } else { theme.muted() };
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Plans (0) ")
            .border_style(border);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("no plans discovered").style(theme.muted()),
            inner,
        );
        return;
    }

    widgets::plan_list::render_plan_list(
        frame,
        area,
        &entries,
        view_state.selected,
        view_state.scroll as usize,
        theme,
    );
}

// ---------------------------------------------------------------------------
// Phase compact -- delegates to widgets::phase_timeline
// ---------------------------------------------------------------------------

fn render_phase_compact(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let phase_stages = ["preflight", "implement", "verify", "gate", "merge"];

    let (phases, current_idx) = if let Some(exec) = &data.current_plan_execution {
        let pct = if exec.tasks_total > 0 {
            exec.tasks_done as f64 / exec.tasks_total as f64
        } else {
            0.0
        };
        let entries: Vec<_> = phase_stages
            .iter()
            .map(|&name| widgets::phase_timeline::PhaseEntry {
                name: name.to_string(),
                elapsed_secs: 0.0,
            })
            .collect();
        let idx = ((pct * phase_stages.len() as f64).floor() as usize)
            .min(phase_stages.len().saturating_sub(1));
        (entries, idx)
    } else {
        let entries = vec![widgets::phase_timeline::PhaseEntry {
            name: "idle".into(),
            elapsed_secs: 0.0,
        }];
        (entries, 0)
    };

    widgets::phase_timeline::render_phase_timeline(frame, area, &phases, current_idx, theme);
}

// ---------------------------------------------------------------------------
// Task progress -- inline (MoriTheme widget not compiled)
// ---------------------------------------------------------------------------

fn render_task_progress(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let total = data.active_tasks.len();
    let done = data
        .active_tasks
        .iter()
        .filter(|t| t.status == "done" || t.status == "completed")
        .count();

    let border = if focused { theme.accent() } else { theme.muted() };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Tasks ({done}/{total}) "))
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.active_tasks.is_empty() {
        frame.render_widget(
            Paragraph::new("no active tasks").style(theme.muted()),
            inner,
        );
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Progress bar.
    if total > 0 && inner.width > 12 {
        let bar_w = (inner.width as usize).saturating_sub(12);
        let pct = done as f64 / total.max(1) as f64;
        let filled = (pct * bar_w as f64).round() as usize;
        let bar_color = match () {
            _ if done == total => Color::Green,
            _ if done > 0 => Color::Yellow,
            _ => Color::DarkGray,
        };
        lines.push(Line::from(vec![
            Span::raw(" "),
            Span::styled("\u{2588}".repeat(filled.min(bar_w)), Style::default().fg(bar_color)),
            Span::styled("\u{2591}".repeat(bar_w.saturating_sub(filled)), Style::default().fg(Color::DarkGray)),
            Span::styled(format!("  {done}/{total}"), Style::default().fg(Color::Gray)),
        ]));
    }

    // Task rows.
    let visible = (inner.height as usize).saturating_sub(lines.len() + 1);
    let start = view_state.secondary_selected.min(total.saturating_sub(1));
    let end = (start + visible).min(total);

    for (i, task) in data.active_tasks[start..end].iter().enumerate() {
        let idx = start + i;
        let (icon, icon_s) = task_icon(task.status.as_str());
        let text_s = if idx == view_state.secondary_selected && focused {
            theme.selection()
        } else {
            task_text_style(task.status.as_str(), theme)
        };

        let elapsed_ms: u64 = data
            .efficiency_events
            .iter()
            .filter(|e| e.task_id == task.task_id)
            .map(|e| e.wall_time_ms)
            .sum();

        let mut spans = vec![
            Span::styled(format!(" {icon} "), icon_s),
            Span::styled(truncate(&task.task_id, 18), text_s),
        ];
        if elapsed_ms > 0 {
            spans.push(Span::styled(
                format!(" {}", fmt_duration_ms(elapsed_ms as f64)),
                Style::default().fg(Color::DarkGray),
            ));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

// ===========================================================================
// Right panel: sub-tabbed detail view
// ===========================================================================

fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);
    let sub = view_state.sub_tab.min(SUB_TAB_LABELS.len().saturating_sub(1));
    render_sub_tab_bar(frame, sections[0], sub, theme);

    let focused = matches!(tui_state.focus, FocusZone::RightPanel);

    match sub {
        0 => render_sub_agents(frame, sections[1], data, view_state, focused, theme),
        1 => render_output_panel(frame, sections[1], data, view_state, focused, theme),
        2 => render_sub_diff(frame, sections[1], data, tui_state, theme),
        3 => render_sub_errors(frame, sections[1], data, theme),
        4 => render_sub_git(frame, sections[1], theme),
        5 => render_sub_mcp(frame, sections[1], data, theme),
        6 => render_sub_processes(frame, sections[1], data, focused, theme),
        _ => {}
    }
}

/// Sub-tab bar with key labels and active highlighting.
fn render_sub_tab_bar(frame: &mut Frame<'_>, area: Rect, active: usize, theme: &Theme) {
    let mut spans: Vec<Span<'_>> = vec![Span::raw(" ")];
    for (i, (key, label)) in SUB_TAB_LABELS.iter().enumerate() {
        let style = if i == active {
            Style::default()
                .fg(theme.background)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.muted)
        };
        spans.push(Span::styled(format!(" {key}:{label} "), style));
        if i + 1 < SUB_TAB_LABELS.len() {
            spans.push(Span::styled("\u{2502}", Style::default().fg(Color::DarkGray)));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Sub-tab: Agents -- pool (top) + output (bottom)
// ---------------------------------------------------------------------------

fn render_sub_agents(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let sections = Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // Build pool entries once.
    let activity =
        crate::tui::dashboard::build_agent_activity_snapshot(&data.agents, &data.efficiency_events);

    let pool: Vec<widgets::parallel_pool::ParallelAgentState> = data
        .agents
        .iter()
        .map(|agent| {
            let row = activity.as_ref().and_then(|s| {
                s.active_agents.iter().find(|r| r.agent_id == agent.id)
            });
            let state = match agent.status.as_str() {
                "running" | "active" => widgets::parallel_pool::AgentRunState::Active,
                "done" | "completed" => widgets::parallel_pool::AgentRunState::Done,
                "error" | "failed" => widgets::parallel_pool::AgentRunState::Failed,
                _ => widgets::parallel_pool::AgentRunState::Idle,
            };
            let used = row.map_or(0, |r| r.tokens_used);
            let total = row.map_or(200_000, |r| if r.tokens_used > 0 { r.tokens_used * 2 } else { 200_000 });
            widgets::parallel_pool::ParallelAgentState {
                role: agent.label.clone(),
                model: row.map_or_else(|| "-".to_string(), |r| r.model.clone()),
                task: agent.plan_id.as_deref().unwrap_or("-").to_string(),
                tokens_used: used,
                tokens_total: total,
                state,
                context_pct: if total > 0 { used as f64 / total as f64 } else { 0.0 },
            }
        })
        .collect();

    widgets::parallel_pool::render_parallel_pool(frame, sections[0], &pool, view_state.selected, theme);
    render_output_panel(frame, sections[1], data, view_state, focused, theme);
}

// ---------------------------------------------------------------------------
// Sub-tab: Output -- shared agent output panel
// ---------------------------------------------------------------------------

fn render_output_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let border = if focused { theme.accent() } else { theme.muted() };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Output ")
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<&str> = data
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("no agent output yet").style(theme.muted()),
            inner,
        );
        return;
    }

    let text: Vec<Line<'_>> = lines.iter().map(|l| Line::from(*l)).collect();
    let scroll = if view_state.auto_tail {
        text.len().saturating_sub(inner.height as usize) as u16
    } else {
        view_state.scroll
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(theme.text())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Sub-tab: Diff -- delegates to widgets::diff_panel
// ---------------------------------------------------------------------------

fn render_sub_diff(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let diff_text = gather_diff_text(data, tui_state);
    let scroll = if tui_state.diff_scroll > 0 {
        Some(tui_state.diff_scroll)
    } else {
        None
    };
    widgets::diff_panel::render_diff_panel(frame, area, &diff_text, scroll, theme);
}

fn gather_diff_text(data: &DashboardData, tui_state: &TuiState) -> String {
    // Try selected agent's diff content.
    if let Some(agent) = tui_state
        .agents_by_id
        .values()
        .nth(tui_state.selected_agent_tab)
    {
        if !agent.diff_content.is_empty() {
            return agent.diff_content.clone();
        }
    }
    // Fallback: extract diff-like lines from execution output.
    if let Some(exec) = &data.current_plan_execution {
        let diff_lines: Vec<&str> = exec
            .agent_output_tail
            .iter()
            .map(String::as_str)
            .filter(|l| {
                l.starts_with('+') || l.starts_with('-') || l.starts_with("@@") || l.starts_with("diff ")
            })
            .collect();
        if !diff_lines.is_empty() {
            return diff_lines.join("\n");
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Sub-tab: Errors -- delegates to widgets::error_digest
// ---------------------------------------------------------------------------

fn render_sub_errors(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let verdicts: Vec<GateVerdict> = data
        .gate_results
        .iter()
        .map(|g| GateVerdict {
            plan_id: g.plan_id.clone(),
            task_id: String::new(),
            gate: g.gate_name.clone(),
            passed: g.passed,
            ts_millis: 0,
        })
        .collect();

    let errors: Vec<ErrorEntry> = data
        .gate_results_page
        .failure_rows
        .iter()
        .map(|row| ErrorEntry {
            message: format!("{}: {} - {}", row.gate_name, row.task_id, row.error_excerpt),
            ts_millis: row.created_at_ms.max(0) as u64,
        })
        .collect();

    let stats = SnapshotStats {
        gates_passed: data.gate_results.iter().filter(|g| g.passed).count(),
        gates_failed: data.gate_results.iter().filter(|g| !g.passed).count(),
        errors_total: errors.len(),
        ..Default::default()
    };

    widgets::error_digest::render_error_digest(frame, area, &verdicts, &errors, &stats, theme);
}

// ---------------------------------------------------------------------------
// Sub-tab: Git (placeholder)
// ---------------------------------------------------------------------------

fn render_sub_git(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Git ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new("git summary: use F4 for full git view").style(theme.muted()),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Sub-tab: MCP / Context status
// ---------------------------------------------------------------------------

fn render_sub_mcp(frame: &mut Frame<'_>, area: Rect, data: &DashboardData, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" MCP / Context ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let eff = &data.efficiency;
    let lines = vec![
        Line::from(vec![
            Span::styled("input tokens:  ", theme.muted()),
            Span::styled(fmt_count(eff.total_input_tokens), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("output tokens: ", theme.muted()),
            Span::styled(fmt_count(eff.total_output_tokens), theme.info()),
        ]),
        Line::from(vec![
            Span::styled("total cost:    ", theme.muted()),
            Span::styled(format!("${:.4}", eff.total_cost_usd), theme.warning()),
        ]),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("cascade router: ", theme.muted()),
            Span::styled(
                format!("{} models", data.cascade_router.model_slugs.len()),
                theme.text(),
            ),
        ]),
        Line::from(vec![
            Span::styled("experiments:    ", theme.muted()),
            Span::styled(format!("{} total", data.experiments.len()), theme.text()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ---------------------------------------------------------------------------
// Sub-tab: Processes -- process table + sys metrics
// ---------------------------------------------------------------------------

fn render_sub_processes(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    focused: bool,
    theme: &Theme,
) {
    let sections = Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // Process table.
    let border = if focused { theme.accent() } else { theme.muted() };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Processes ")
        .border_style(border);
    let inner = block.inner(sections[0]);
    frame.render_widget(block, sections[0]);

    if data.agents.is_empty() {
        frame.render_widget(
            Paragraph::new("no tracked processes").style(theme.muted()),
            inner,
        );
    } else {
        let activity = crate::tui::dashboard::build_agent_activity_snapshot(
            &data.agents,
            &data.efficiency_events,
        );
        let rows: Vec<Row<'_>> = data
            .agents
            .iter()
            .map(|agent| {
                let ss = match agent.status.as_str() {
                    "running" | "active" => theme.info(),
                    "error" | "failed" => theme.danger(),
                    _ => theme.muted(),
                };
                let row = activity
                    .as_ref()
                    .and_then(|s| s.active_agents.iter().find(|r| r.agent_id == agent.id));
                Row::new(vec![
                    Cell::from(truncate(&agent.id, 14)),
                    Cell::from(agent.label.as_str()),
                    Cell::from(Span::styled(agent.status.as_str(), ss)),
                    Cell::from(row.map_or("-".into(), |r| shorten_model(&r.model))),
                    Cell::from(row.map_or("-".into(), |r| fmt_tokens(r.tokens_used))),
                    Cell::from(agent.plan_id.as_deref().unwrap_or("-")),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(10),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(12),
            Constraint::Length(8),
            Constraint::Min(8),
        ];
        frame.render_widget(
            Table::new(rows, widths)
                .header(
                    Row::new(["pid", "label", "status", "model", "tokens", "plan"])
                        .style(theme.accent().add_modifier(Modifier::BOLD)),
                )
                .column_spacing(1),
            inner,
        );
    }

    // System metrics summary.
    let sys_block = Block::default()
        .borders(Borders::ALL)
        .title(" System ")
        .border_style(theme.muted());
    let sys_inner = sys_block.inner(sections[1]);
    frame.render_widget(sys_block, sections[1]);

    let eff = &data.efficiency;
    let active = data
        .agents
        .iter()
        .filter(|a| a.status == "running" || a.status == "active")
        .count();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("agents:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{active} active / {} total", data.agents.len()), theme.text()),
            ]),
            Line::from(vec![
                Span::styled("tokens:  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{}in + {}out", fmt_count(eff.total_input_tokens), fmt_count(eff.total_output_tokens)),
                    theme.text(),
                ),
            ]),
            Line::from(vec![
                Span::styled("cost:    ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("${:.4}", eff.total_cost_usd), theme.warning()),
            ]),
        ]),
        sys_inner,
    );
}

// ===========================================================================
// Bottom ribbon: wave progress (40%) | token sparkline (40%) | sys (20%)
// ===========================================================================

fn render_bottom_ribbon(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    theme: &Theme,
) {
    let sections = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(40),
        Constraint::Percentage(20),
    ])
    .split(area);

    // Wave progress.
    {
        let (done, total) = data
            .current_plan_execution
            .as_ref()
            .map(|e| (e.tasks_done, e.tasks_total))
            .unwrap_or((0, 0));
        let ratio = if total > 0 { done as f64 / total as f64 } else { 0.0 };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Wave ")
            .border_style(theme.muted());
        let inner = block.inner(sections[0]);
        frame.render_widget(block, sections[0]);

        if inner.width >= 4 && inner.height >= 1 {
            let w = inner.width as usize;
            let filled = (ratio.clamp(0.0, 1.0) * w as f64).round() as usize;
            let mut spans: Vec<Span<'_>> = Vec::new();
            for i in 0..filled.min(w) {
                let t = if filled > 1 { i as f64 / (filled - 1) as f64 } else { ratio };
                spans.push(Span::styled("\u{2588}", Style::default().fg(ocean_gradient(t))));
            }
            if filled < w {
                spans.push(Span::styled(
                    "\u{2500}".repeat(w.saturating_sub(filled)),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(spans)), inner);
        }
    }

    // Token sparkline.
    {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Tokens ")
            .border_style(theme.muted());
        let inner = block.inner(sections[1]);
        frame.render_widget(block, sections[1]);
        let eff = &data.efficiency;
        let total = eff.total_input_tokens + eff.total_output_tokens;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {} total  ${:.3}", fmt_count(total), eff.total_cost_usd),
                Style::default().fg(theme.foreground),
            ))),
            inner,
        );
    }

    // Sys badge.
    {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Sys ")
            .border_style(theme.muted());
        let inner = block.inner(sections[2]);
        frame.render_widget(block, sections[2]);
        let active = data
            .agents
            .iter()
            .filter(|a| a.status == "running" || a.status == "active")
            .count();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" {active}agt"),
                Style::default().fg(theme.foreground),
            ))),
            inner,
        );
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

fn task_icon(status: &str) -> (&'static str, Style) {
    match status {
        "done" | "completed" => ("\u{2713}", Style::default().fg(Color::Green)),
        "running" | "in_progress" => (
            "\u{25ba}",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ),
        "failed" => ("\u{2717}", Style::default().fg(Color::Red)),
        "blocked" => ("\u{2717}", Style::default().fg(Color::Red)),
        _ => ("\u{00b7}", Style::default().fg(Color::DarkGray)),
    }
}

fn task_text_style<'a>(status: &str, theme: &'a Theme) -> Style {
    match status {
        "done" | "completed" => theme.success(),
        "running" | "in_progress" => theme.info(),
        "failed" => theme.danger(),
        _ => theme.text(),
    }
}

fn ocean_gradient(t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(
        (40.0 + t * 60.0).min(255.0) as u8,
        (180.0 - t * 80.0).min(255.0) as u8,
        (200.0 + t * 55.0).min(255.0) as u8,
    )
}

fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("gpt-", "")
        .replace("-codex", "c")
        .replace("-mini", "m")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

fn fmt_tokens(n: u64) -> String {
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

fn fmt_duration_ms(ms: f64) -> String {
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
    } else if max <= 3 {
        s[..max].to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
