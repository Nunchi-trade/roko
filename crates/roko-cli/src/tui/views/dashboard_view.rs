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
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use roko_core::dashboard_snapshot::{ErrorEntry, GateVerdict, SnapshotStats};

use super::ViewState;
use crate::config::Config;
use crate::tui::ansi::parse_ansi_line;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::input::FocusZone;
use crate::tui::state::{RouteMetrics, TuiState};
use crate::tui::widgets::{self, braille};

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
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let outer = Layout::vertical([Constraint::Min(0), Constraint::Length(6)]).split(area);
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
    // Content-aware sizing: phase gets a fixed 4 lines, plan tree and task
    // progress split the rest proportionally based on content.
    let plan_count = tui_state.plans.len();
    let task_count = tui_state.current_task_checklist.len();
    let has_content = plan_count > 0 || task_count > 0;

    let (plan_pct, phase_h, task_pct) = if has_content {
        // Content-aware: plan tree gets more space when plans exist
        let plan_lines = (plan_count + 3).min(20) as u16; // header + plans + padding
        let task_lines = (task_count + 3).min(15) as u16;
        let total = plan_lines + task_lines;
        let plan_frac = if total > 0 {
            plan_lines * 100 / total
        } else {
            50
        };
        (
            plan_frac.max(30).min(70),
            4u16,
            (100 - plan_frac.max(30).min(70)),
        )
    } else {
        // Idle: compact layout — give more room to plan tree for column headers
        (45, 4, 51)
    };

    let sections = Layout::vertical([
        Constraint::Percentage(plan_pct),
        Constraint::Length(phase_h),
        Constraint::Percentage(task_pct),
    ])
    .split(area);

    let plan_focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let task_focused = matches!(tui_state.focus, FocusZone::TaskProgress);

    widgets::plan_tree::render_plan_tree(frame, sections[0], tui_state, plan_focused);
    widgets::phase_compact::render_phase_compact(frame, sections[1], tui_state, false);
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
    let sub = view_state
        .sub_tab
        .min(SUB_TAB_LABELS.len().saturating_sub(1));
    render_sub_tab_bar(frame, sections[0], sub, theme);

    let focused = matches!(tui_state.focus, FocusZone::RightPanel);

    match sub {
        0 => render_sub_agents(
            frame,
            sections[1],
            data,
            tui_state,
            view_state,
            focused,
            theme,
        ),
        1 => render_output_panel(
            frame,
            sections[1],
            data,
            tui_state,
            view_state,
            focused,
            theme,
        ),
        2 => render_sub_diff(frame, sections[1], data, tui_state, theme),
        3 => render_sub_errors(frame, sections[1], data, theme),
        4 => render_sub_git(frame, sections[1], tui_state, theme),
        5 => render_sub_mcp(frame, sections[1], data, tui_state, theme),
        6 => render_sub_processes(frame, sections[1], data, tui_state, focused, theme),
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
            spans.push(Span::styled(
                "\u{2502}",
                Style::default().fg(Color::DarkGray),
            ));
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
    tui_state: &TuiState,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let route_row_count = tui_state
        .agents
        .iter()
        .filter(|agent| tui_state.route_metrics.contains_key(&agent.id))
        .count();
    let route_height = if route_row_count == 0 {
        3
    } else {
        (route_row_count as u16 + 3).min(8)
    };
    let sections = Layout::vertical([
        Constraint::Percentage(52),
        Constraint::Length(route_height),
        Constraint::Min(0),
    ])
    .split(area);

    widgets::parallel_pool::render_parallel_pool(
        frame,
        sections[0],
        &tui_state.agents,
        tui_state
            .selected_agent
            .min(tui_state.agents.len().saturating_sub(1)),
        theme,
    );
    render_agent_routes_table(
        frame,
        sections[1],
        tui_state,
        &tui_state.route_metrics,
        theme,
    );
    render_output_panel(
        frame,
        sections[2],
        data,
        tui_state,
        view_state,
        focused,
        theme,
    );
}

// ---------------------------------------------------------------------------
// Sub-tab: Output -- shared agent output panel
// ---------------------------------------------------------------------------

fn render_output_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    focused: bool,
    theme: &Theme,
) {
    let border = if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Output ")
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Collect output lines from the best available source.
    //
    // Priority:
    //   1. current_plan_execution.agent_output_tail
    //   2. selected agent's live row data from tui_state.agents
    //   3. most recent task output from data.task_outputs
    let collected: Vec<String> = {
        // 1. Plan execution output tail.
        let exec_lines: Vec<String> = data
            .current_plan_execution
            .as_ref()
            .map(|exec| exec.agent_output_tail.clone())
            .unwrap_or_default();
        if !exec_lines.is_empty() {
            exec_lines
        } else if let Some(agent) = tui_state.agents.get(
            tui_state
                .selected_agent
                .min(tui_state.agents.len().saturating_sub(1)),
        ) {
            // 2. Selected agent output from live row data.
            if !agent.output_lines.is_empty() {
                agent.output_lines.clone()
            } else if !agent.last_output_line.is_empty() {
                vec![agent.last_output_line.clone()]
            } else if !agent.current_task.is_empty() {
                data.task_outputs
                    .get(&agent.current_task)
                    .cloned()
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    };

    // 3. Fallback: most recent task output from data.task_outputs.
    let collected = if collected.is_empty() {
        data.task_outputs
            .values()
            .max_by_key(|v| v.len())
            .cloned()
            .unwrap_or_default()
    } else {
        collected
    };
    let lines: Vec<&str> = collected.iter().map(String::as_str).collect();

    if lines.is_empty() {
        frame.render_widget(
            Paragraph::new("no agent output yet").style(theme.muted()),
            inner,
        );
        return;
    }

    let text: Vec<Line<'static>> = lines
        .iter()
        .map(|line| Line::from(parse_ansi_line(line)))
        .collect();
    let scroll = if view_state.auto_tail {
        text.len()
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16
    } else {
        let max_scroll = text.len().saturating_sub(inner.height as usize);
        max_scroll
            .saturating_sub((view_state.scroll as usize).min(max_scroll))
            .min(u16::MAX as usize) as u16
    };
    frame.render_widget(
        Paragraph::new(text)
            .style(theme.text())
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
}

fn render_agent_routes_table(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    route_metrics: &HashMap<String, RouteMetrics>,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Routes ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows: Vec<Row<'_>> = tui_state
        .agents
        .iter()
        .enumerate()
        .filter_map(|(idx, agent)| {
            route_metrics.get(&agent.id).map(|metric| {
                let tier_style = route_tier_style(&metric.tier, theme);
                let row_style = if idx == tui_state.selected_agent {
                    theme.selection()
                } else {
                    Style::default()
                };
                let model = if metric.model.is_empty() {
                    "-".to_string()
                } else {
                    truncate(&shorten_model(&metric.model), 18)
                };
                Row::new(vec![
                    Cell::from(truncate(&agent.id, 14)),
                    Cell::from(Span::styled(
                        model,
                        theme.text().add_modifier(Modifier::BOLD),
                    )),
                    Cell::from(Span::styled(truncate(&metric.tier, 10), tier_style)),
                ])
                .style(row_style)
            })
        })
        .collect();

    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("no agent route metrics").style(theme.muted()),
            inner,
        );
        return;
    }

    let rows = rows
        .into_iter()
        .take(inner.height.saturating_sub(2) as usize)
        .collect::<Vec<_>>();

    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Min(18),
                Constraint::Length(10),
            ],
        )
        .header(
            Row::new(["Agent", "Model", "Tier"]).style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1),
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
    let scroll = if tui_state.diff_scroll > 0 {
        Some(tui_state.diff_scroll)
    } else {
        None
    };
    widgets::diff_panel::render_diff_panel(frame, area, &data.git_diff, scroll, theme);
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
// Sub-tab: Git — inline summary from git commands
// ---------------------------------------------------------------------------

fn render_sub_git(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let focused = matches!(tui_state.focus, FocusZone::RightPanel);
    let border = if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Git ")
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Use pre-populated git summary from TuiState (filled by background thread).
    // Zero I/O in the render path.
    let cached_lines = &tui_state.git_summary_lines;

    if cached_lines.is_empty() {
        frame.render_widget(
            Paragraph::new(" loading git data...").style(theme.muted()),
            inner,
        );
        return;
    }

    let lines: Vec<Line<'_>> = cached_lines
        .iter()
        .map(|s| Line::from(Span::styled(s.as_str(), theme.text())))
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Collect git summary data as plain strings from the cached git snapshot.
pub(crate) fn collect_git_summary(
    git_data: &crate::tui::views::git_view::GitViewData,
    age: &str,
) -> Vec<String> {
    let mut lines = Vec::new();

    let commit = git_data
        .commits
        .first()
        .map(|commit| commit.hash_short.as_str())
        .unwrap_or_default();

    if !git_data.current_branch.is_empty() {
        lines.push(format!(
            " branch: {}  {commit}  {age}",
            git_data.current_branch
        ));
    }

    let modified = git_data
        .status_lines
        .iter()
        .filter(|line| line.starts_with(" M") || line.starts_with("M "))
        .count();
    let added = git_data
        .status_lines
        .iter()
        .filter(|line| line.starts_with("A ") || line.starts_with("??"))
        .count();
    let deleted = git_data
        .status_lines
        .iter()
        .filter(|line| line.starts_with(" D") || line.starts_with("D "))
        .count();
    let total = git_data
        .status_lines
        .iter()
        .filter(|line| !line.is_empty())
        .count();
    if total > 0 {
        lines.push(format!(
            " status: {total} changed  M:{modified} A:{added} D:{deleted}"
        ));
    } else if !git_data.current_branch.is_empty() {
        lines.push(" status: clean".to_string());
    }

    if !git_data.commits.is_empty() {
        lines.push(String::new());
        lines.push(" recent commits:".to_string());
        for commit in git_data.commits.iter().take(8) {
            lines.push(format!(
                "  {}{} {}",
                commit.graph_prefix, commit.hash_short, commit.subject
            ));
        }
    }

    if git_data.worktrees.len() > 1 {
        lines.push(String::new());
        lines.push(format!(" worktrees: {}", git_data.worktrees.len()));
    }

    if lines.is_empty() {
        lines.push(" not a git repository".to_string());
    }

    lines
}

// ---------------------------------------------------------------------------
// Sub-tab: MCP / Context status
// ---------------------------------------------------------------------------

fn render_sub_mcp(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::RightPanel);
    let border = if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" MCP / Context ")
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let eff = &data.efficiency;
    let model_usage = aggregate_model_usage(&data.efficiency_events);
    let mcp_config = load_mcp_config_view(data.root());
    let total_trials: u64 = data
        .cascade_router
        .confidence_stats
        .values()
        .map(|stats| stats.trials)
        .sum();
    let total_successes: u64 = data
        .cascade_router
        .confidence_stats
        .values()
        .map(|stats| stats.successes)
        .sum();

    let mut lines = vec![
        section_header("MCP Config", theme),
        Line::from(vec![
            Span::styled("agent.mcp_config: ", theme.muted()),
            Span::styled(
                mcp_config.configured_path.as_ref().map_or_else(
                    || "(not set)".to_string(),
                    |path| path.display().to_string(),
                ),
                if mcp_config.configured_path.is_some() {
                    theme.text()
                } else {
                    theme.muted()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("resolved path:     ", theme.muted()),
            Span::styled(
                mcp_config
                    .resolved_path
                    .as_ref()
                    .map_or_else(|| "-".to_string(), |path| path.display().to_string()),
                if mcp_config.resolved_path.is_some() {
                    theme.text()
                } else {
                    theme.muted()
                },
            ),
        ]),
        Line::from(vec![
            Span::styled("server count:       ", theme.muted()),
            Span::styled(
                mcp_config.config.as_ref().map_or_else(
                    || "-".to_string(),
                    |config| config.servers.len().to_string(),
                ),
                theme.info(),
            ),
        ]),
        Line::from(Span::raw("")),
    ];

    if let Some(error) = &mcp_config.error {
        lines.push(Line::from(vec![
            Span::styled("status: ", theme.muted()),
            Span::styled(
                truncate(error, inner.width.saturating_sub(8) as usize),
                theme.danger(),
            ),
        ]));
    } else if let Some(config) = &mcp_config.config {
        if config.servers.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("status: ", theme.muted()),
                Span::styled("config loaded, no servers defined", theme.warning()),
            ]));
        } else {
            for server in &config.servers {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:>12}: ", truncate(&server.name, 12)),
                        theme.muted(),
                    ),
                    Span::styled(render_mcp_command(server), theme.text()),
                ]));
            }
        }
    } else if mcp_config.configured_path.is_some() {
        lines.push(Line::from(vec![
            Span::styled("status: ", theme.muted()),
            Span::styled("configured file not found", theme.warning()),
        ]));
    }

    lines.extend([
        Line::from(Span::raw("")),
        section_header("Efficiency", theme),
        Line::from(vec![
            Span::styled("input tokens:  ", theme.muted()),
            Span::styled(fmt_count(eff.total_input_tokens), theme.info()),
            Span::styled("  output: ", theme.muted()),
            Span::styled(fmt_count(eff.total_output_tokens), theme.info()),
            Span::styled("  cost: ", theme.muted()),
            Span::styled(format!("${:.4}", eff.total_cost_usd), theme.warning()),
        ]),
    ]);

    if model_usage.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("models: ", theme.muted()),
            Span::styled("no efficiency events", theme.muted()),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled("models: ", theme.muted()),
            Span::styled(format!("{} tracked", model_usage.len()), theme.text()),
        ]));
        for (model, usage) in model_usage {
            lines.push(Line::from(vec![
                Span::styled(format!("{:>12}: ", truncate(&model, 12)), theme.muted()),
                Span::styled(format!("{} turns", usage.turns), theme.text()),
                Span::styled("  in ", theme.muted()),
                Span::styled(fmt_count(usage.input_tokens), theme.info()),
                Span::styled("  out ", theme.muted()),
                Span::styled(fmt_count(usage.output_tokens), theme.info()),
                Span::styled("  ", theme.muted()),
                Span::styled(format!("${:.4}", usage.cost_usd), theme.warning()),
            ]));
        }
    }

    lines.extend([
        Line::from(Span::raw("")),
        section_header("Cascade Router", theme),
    ]);
    if data.cascade_router.model_slugs.is_empty() && total_trials == 0 {
        lines.push(Line::from(vec![
            Span::styled("status: ", theme.muted()),
            Span::styled("no router stats yet", theme.muted()),
        ]));
    } else {
        let success_rate = if total_trials > 0 {
            format!(
                "{:.0}%",
                total_successes as f64 / total_trials as f64 * 100.0
            )
        } else {
            "-".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled("models: ", theme.muted()),
            Span::styled(
                data.cascade_router.model_slugs.len().to_string(),
                theme.info(),
            ),
            Span::styled("  trials: ", theme.muted()),
            Span::styled(total_trials.to_string(), theme.text()),
            Span::styled("  success: ", theme.muted()),
            Span::styled(success_rate, theme.text()),
        ]));
        for slug in &data.cascade_router.model_slugs {
            let stats = data.cascade_router.confidence_stats.get(slug);
            let trials = stats.map_or(0, |entry| entry.trials);
            let successes = stats.map_or(0, |entry| entry.successes);
            let rate = if trials > 0 {
                format!("{:.0}%", successes as f64 / trials as f64 * 100.0)
            } else {
                "-".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{:>12}: ", truncate(slug, 12)), theme.muted()),
                Span::styled(format!("{successes}/{trials}"), theme.text()),
                Span::styled("  rate ", theme.muted()),
                Span::styled(rate, theme.info()),
            ]));
        }
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

// ---------------------------------------------------------------------------
// Sub-tab: Processes -- process table + sys metrics
// ---------------------------------------------------------------------------

fn render_sub_processes(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    focused: bool,
    theme: &Theme,
) {
    let border = if focused {
        theme.accent()
    } else {
        theme.muted()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Processes ")
        .border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut process_rows: Vec<_> = tui_state.process_metrics.iter().collect();
    process_rows.sort_by(|a, b| a.role.cmp(&b.role).then_with(|| a.pid.cmp(&b.pid)));

    if process_rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No process data").style(theme.muted()),
            inner,
        );
        return;
    }

    let inner_width = inner.width as usize;
    let role_width = if inner_width < 72 {
        10
    } else if inner_width < 96 {
        14
    } else {
        18
    };
    let uptime_width = if inner_width < 72 { 9 } else { 12 };
    let trend_width: usize = if inner_width < 72 {
        12
    } else if inner_width < 96 {
        18
    } else {
        22
    };
    let spark_width = trend_width.saturating_sub(10).max(4);

    let rows: Vec<Row<'_>> = process_rows
        .into_iter()
        .map(|proc| {
            let cpu_style = if proc.cpu_pct >= 50.0 {
                theme.warning()
            } else {
                theme.info()
            };
            let state_style = match proc.state.as_str() {
                "running" => theme.info(),
                "sleeping" => theme.muted(),
                "stopped" => theme.danger(),
                _ => theme.muted(),
            };

            let cpu_history: Vec<f32> = proc.cpu_history.iter().copied().collect();
            let mem_history: Vec<u64> = proc.mem_history.iter().copied().collect();

            let mut trend_spans = vec![Span::styled("c ", theme.muted())];
            trend_spans.extend(braille::braille_spans_f32(
                &cpu_history,
                100.0,
                spark_width,
                theme.info,
            ));
            trend_spans.push(Span::styled(" m ", theme.muted()));
            trend_spans.extend(braille::braille_spans_u64(
                &mem_history,
                spark_width,
                theme.warning,
            ));

            Row::new(vec![
                Cell::from(Span::styled(proc.pid.to_string(), theme.text())),
                Cell::from(Span::styled(truncate(&proc.role, role_width), theme.text())),
                Cell::from(Span::styled(format!("{:>5.1}%", proc.cpu_pct), cpu_style)),
                Cell::from(Span::styled(
                    format_mem_bytes(proc.mem_bytes),
                    theme.warning(),
                )),
                Cell::from(Span::styled(truncate(&proc.state, 9), state_style)),
                Cell::from(Span::styled(format_uptime(proc.uptime_secs), theme.text())),
                Cell::from(Line::from(trend_spans)),
            ])
        })
        .collect();
    let visible_rows = inner.height.saturating_sub(2) as usize;
    let max_scroll = rows.len().saturating_sub(visible_rows.max(1));
    let scroll = tui_state.diff_scroll.min(max_scroll);
    let rows = rows
        .into_iter()
        .skip(scroll)
        .take(visible_rows.max(1))
        .collect::<Vec<_>>();

    let widths = [
        Constraint::Length(7),
        Constraint::Min(role_width as u16),
        Constraint::Length(7),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(uptime_width as u16),
        Constraint::Min(trend_width as u16),
    ];

    frame.render_widget(
        Table::new(rows, widths)
            .header(
                Row::new(["PID", "Role", "CPU%", "MEM", "State", "Uptime", "Trend"])
                    .style(theme.accent().add_modifier(Modifier::BOLD)),
            )
            .column_spacing(1),
        inner,
    );
}

fn format_mem_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_uptime(uptime_secs: f64) -> String {
    let total = uptime_secs.max(0.0).round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

// ===========================================================================
// Bottom ribbon: wave progress (40%) | token sparkline (40%) | sys (20%)
// ===========================================================================

fn render_bottom_ribbon(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
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
    widgets::token_sparkline::render_token_sparkline(frame, sections[1], data, tui_state);
    // Sys metrics: use the real widget
    widgets::sys_metrics::render_sys_metrics(frame, sections[2], tui_state);
}

// ===========================================================================
// Helpers
// ===========================================================================

#[derive(Debug, Clone, Default)]
struct McpConfigView {
    configured_path: Option<PathBuf>,
    resolved_path: Option<PathBuf>,
    config: Option<roko_agent::mcp::McpConfig>,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ModelUsageAggregate {
    turns: usize,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: f64,
}

fn section_header(title: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        theme.accent().add_modifier(Modifier::BOLD),
    ))
}

fn load_mcp_config_view(root: &Path) -> McpConfigView {
    let config_path = root.join("roko.toml");
    let config = match Config::from_file(&config_path) {
        Ok(config) => config,
        Err(error) => {
            return McpConfigView {
                error: Some(format!("failed to load roko.toml: {error}")),
                ..McpConfigView::default()
            };
        }
    };

    let Some(configured_path) = config.agent.mcp_config else {
        return McpConfigView::default();
    };
    let resolved_path = resolve_mcp_config_path(root, &configured_path);
    if !resolved_path.is_file() {
        return McpConfigView {
            configured_path: Some(configured_path),
            resolved_path: Some(resolved_path),
            ..McpConfigView::default()
        };
    }

    match roko_agent::mcp::McpConfig::load(&resolved_path) {
        Ok(config) => McpConfigView {
            configured_path: Some(configured_path),
            resolved_path: Some(resolved_path),
            config: Some(config),
            error: None,
        },
        Err(error) => McpConfigView {
            configured_path: Some(configured_path),
            resolved_path: Some(resolved_path),
            config: None,
            error: Some(error.to_string()),
        },
    }
}

fn resolve_mcp_config_path(root: &Path, configured_path: &Path) -> PathBuf {
    if configured_path.is_absolute() {
        configured_path.to_path_buf()
    } else {
        root.join(configured_path)
    }
}

fn render_mcp_command(server: &roko_agent::mcp::McpServerConfig) -> String {
    if server.args.is_empty() {
        server.command.clone()
    } else {
        format!("{} {}", server.command, server.args.join(" "))
    }
}

fn aggregate_model_usage(
    events: &[roko_learn::efficiency::AgentEfficiencyEvent],
) -> Vec<(String, ModelUsageAggregate)> {
    let mut usage: BTreeMap<String, ModelUsageAggregate> = BTreeMap::new();
    for event in events {
        let model = event_model_slug(event);
        let entry = usage.entry(model).or_default();
        entry.turns += 1;
        entry.input_tokens += event.input_tokens;
        entry.output_tokens += event.output_tokens;
        entry.cost_usd += event.cost_usd;
    }

    let mut usage: Vec<(String, ModelUsageAggregate)> = usage.into_iter().collect();
    usage.sort_by(|a, b| {
        b.1.cost_usd
            .partial_cmp(&a.1.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.input_tokens.cmp(&a.1.input_tokens))
            .then_with(|| a.0.cmp(&b.0))
    });
    usage
}

fn event_model_slug(event: &roko_learn::efficiency::AgentEfficiencyEvent) -> String {
    let model = if event.model.is_empty() {
        event.model_used.as_str()
    } else {
        event.model.as_str()
    };
    if model.is_empty() {
        "unknown".to_string()
    } else {
        model.to_string()
    }
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

fn route_tier_style(tier: &str, theme: &Theme) -> Style {
    match tier {
        "fast" => theme.success(),
        "balanced" => theme.accent(),
        "deep" => theme.warning(),
        _ => theme.muted(),
    }
}
