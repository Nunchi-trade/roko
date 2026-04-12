//! F1 Dashboard view -- master-detail layout with 7 sub-tabs.
//!
//! Left panel (38%): plan tree + phase compact + task progress.
//! Right panel (62%): sub-tabbed detail view (Agents, Output, Diff,
//! Errors, Git, Context/MCP, Processes).
//! Bottom ribbon: wave progress + token sparkline + sys metrics.
//!
//! Delegates to compiled widgets for all panels.

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
    render_bottom_ribbon(frame, outer[1], data, tui_state, theme);
}

// ===========================================================================
// Left panel: plan tree (50%) + phase compact (15%) + task progress (35%)
// ===========================================================================

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    _view_state: &ViewState,
    _theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(15),
        Constraint::Percentage(35),
    ])
    .split(area);

    let plan_focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let task_focused = matches!(tui_state.focus, FocusZone::TaskProgress);

    // Use the full Mori plan_tree widget (wave groups, sparklines, data-rain)
    widgets::plan_tree::render_plan_tree(frame, sections[0], tui_state, plan_focused);
    // Use the Mori phase_compact widget (segmented bar with spinner)
    widgets::phase_compact::render_phase_compact(frame, sections[1], tui_state, false);
    // Use the Mori task_progress widget (semantic checklist)
    widgets::task_progress::render_task_progress(frame, sections[2], tui_state, task_focused);
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
    _data: &DashboardData,
    tui_state: &TuiState,
    _theme: &Theme,
) {
    // Use the Mori wave_progress + token_sparkline + sys_metrics widgets
    let sections = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(40),
        Constraint::Percentage(20),
    ])
    .split(area);

    // Wave progress: use the real widget
    widgets::wave_progress::render_wave_progress(frame, sections[0], tui_state);
    // Token sparkline: use the real widget
    widgets::token_sparkline::render_token_sparkline(frame, sections[1], tui_state);
    // Sys metrics: use the real widget
    widgets::sys_metrics::render_sys_metrics(frame, sections[2], tui_state);
}

// ===========================================================================
// Helpers
// ===========================================================================

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
