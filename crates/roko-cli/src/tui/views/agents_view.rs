//! F3 Agents view -- Mori-style agent roster + output panel.
//!
//! Layout: left 32% (agent roster, summary line, token sparkline),
//! right 68% (role tabs + scrollable agent output).
//!
//! Renders rich gradient progress bars, context gauges, role-colored
//! tabs, and status chips matching the Mori Agents screen (F3).

use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::input::FocusZone;
use crate::tui::state::{AgentStatus, AgentTopologyStatus, TuiState, model_context_limit};
use crate::tui::util::truncate_middle;

// ---------------------------------------------------------------------------
// Role tab labels (fixed order, matching Mori)
// ---------------------------------------------------------------------------

const ROLE_TABS: &[(&str, &str)] = &[
    ("implementer", "1:impl"),
    ("strategist", "2:strat"),
    ("architect", "3:arch"),
    ("auditor", "4:audit"),
    ("critic", "5:crit"),
    ("conductor", "6:cond"),
    ("researcher", "7:res"),
];

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the full agents view.
pub(crate) fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)]).split(area);

    render_left_panel(frame, panels[0], data, tui_state, view_state, theme);
    render_right_panel(frame, panels[1], tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Left panel: agent roster + summary + token sparkline
// ---------------------------------------------------------------------------

fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Compute how much space to allocate.
    // Summary = 2 lines, sparkline = 6 lines (if token data exists), rest = roster.
    let has_token_data = tui_state.efficiency_summary.event_count > 0
        || tui_state.cumulative_input_tokens > 0
        || tui_state.cumulative_output_tokens > 0
        || !tui_state.efficiency_events.is_empty();

    let sparkline_height = if has_token_data { 6u16 } else { 0u16 };

    let sections = Layout::vertical([
        Constraint::Min(4),                   // agent roster (flexible)
        Constraint::Length(2),                // summary line
        Constraint::Length(sparkline_height), // token sparkline
    ])
    .split(area);

    render_agent_roster(frame, sections[0], tui_state, view_state, theme);
    render_summary_line(frame, sections[1], tui_state, theme);
    if has_token_data {
        crate::tui::widgets::token_sparkline::render_token_sparkline(
            frame,
            sections[2],
            data,
            tui_state,
        );
    }
}

// ---------------------------------------------------------------------------
// Agent roster
// ---------------------------------------------------------------------------

fn render_agent_roster(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let focused = matches!(tui_state.focus, FocusZone::PlanTree);
    let mut agents: Vec<(usize, &crate::tui::dashboard::AgentSummary)> =
        tui_state.agent_summaries.iter().enumerate().collect();
    agents.sort_by(|(idx_a, a), (idx_b, b)| {
        agent_status_rank(&a.status)
            .cmp(&agent_status_rank(&b.status))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
            .then_with(|| idx_a.cmp(idx_b))
    });

    let active_count = agents
        .iter()
        .filter(|a| AgentStatus::from(a.1.status.as_str()).is_active())
        .count();
    let title = format!(" Agents ({} active) ", active_count);

    let border_style = if focused {
        Theme::focused_border_style()
    } else if active_count > 0 {
        Style::default().fg(theme.accent)
    } else {
        theme.muted()
    };
    let title_style = if focused {
        Theme::focused_title_style()
    } else if active_count > 0 {
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, title_style))
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if agents.is_empty() {
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(1) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "no agents spawned",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "agents will appear here when",
            Style::default().fg(theme.muted),
        )));
        empty_lines.push(Line::from(Span::styled(
            "plan execution begins",
            Style::default().fg(theme.muted),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let content_width = inner.width as usize;
    let activity = crate::tui::dashboard::build_agent_activity_snapshot(
        &tui_state.agent_summaries,
        &tui_state.efficiency_events,
    );

    let mut lines: Vec<Line<'_>> = Vec::new();
    if inner.height > 3 {
        let agent_w = 14usize.min(content_width / 4);
        let task_w = 18usize.min(content_width / 3);
        lines.push(Line::from(vec![
            Span::styled("   ", Style::default()),
            Span::styled(
                format!("{:<agent_w$}", "agent"),
                Style::default().fg(theme.muted),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("model", Style::default().fg(theme.muted)),
            Span::styled("  ", Style::default()),
            Span::styled("status", Style::default().fg(theme.muted)),
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{:<task_w$}", "task"),
                Style::default().fg(theme.muted),
            ),
            Span::styled("  ", Style::default()),
            Span::styled("tokens", Style::default().fg(theme.muted)),
            Span::styled("  ", Style::default()),
            Span::styled("cost", Style::default().fg(theme.muted)),
        ]));
    }

    for (idx, agent) in tui_state.agent_summaries.iter().enumerate() {
        let is_selected = idx == view_state.selected;
        let status = AgentStatus::from(agent.status.as_str());
        let is_active = status.is_active();
        let is_done = status.is_done();
        let is_failed = status.is_failed();

        // Status icon
        let (icon, icon_style) = if is_active {
            (
                "\u{25b6}", // ▶
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )
        } else if is_done {
            ("\u{2713}", Style::default().fg(theme.success)) // ✓
        } else if is_failed {
            (
                "\u{2717}", // ✗
                Style::default()
                    .fg(theme.danger)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("\u{00b7}", Style::default().fg(theme.muted)) // ·
        };

        // Role accent color
        let accent = role_accent(&agent.label, theme);
        let bg = if is_selected {
            theme.selection_background
        } else {
            Color::Reset
        };
        let cursor = if is_selected { " \u{25b6} " } else { "   " };
        let agent_w = 14usize.min(content_width / 4);
        let task_w = 18usize.min(content_width / 3);
        let activity_row = activity
            .as_ref()
            .and_then(|snap| snap.active_agents.iter().find(|r| r.agent_id == agent.id));

        let model = activity_row
            .map(|r| shorten_model(&r.model))
            .unwrap_or_else(|| "-".to_string());
        let task = activity_row
            .map(|r| truncate_middle(&r.task, task_w))
            .or_else(|| {
                agent
                    .plan_id
                    .as_deref()
                    .map(|plan_id| truncate_middle(plan_id, task_w))
            })
            .unwrap_or_else(|| "-".to_string());
        let tokens_str = activity_row
            .map(|r| format_tokens(r.tokens_used))
            .unwrap_or_else(|| "-".to_string());
        let cost_str = activity_row
            .map(|r| {
                if r.cost_usd > 0.001 {
                    format!("${:.2}", r.cost_usd)
                } else {
                    "-".to_string()
                }
            })
            .unwrap_or_else(|| "-".to_string());

        // Context gauge — use tokens against the model's context window
        let total_tokens = activity_row.map_or(0u64, |r| r.tokens_used);
        let ctx_limit = tui_state
            .agents
            .iter()
            .find(|row| row.id == agent.id)
            .map(|row| row.context_limit)
            .filter(|limit| *limit > 0)
            .or_else(|| activity_row.map(|row| model_context_limit(&row.model)))
            .unwrap_or_else(|| model_context_limit(""));
        let fill_pct = (total_tokens as f64 / ctx_limit as f64).clamp(0.0, 1.0);
        let gauge_width = 6usize.min(content_width.saturating_sub(40));
        let state_label = if is_active {
            " LIVE "
        } else if is_done {
            " DONE "
        } else if is_failed {
            " FAIL "
        } else {
            " idle "
        };
        let status_color = if is_active {
            accent
        } else if is_done {
            theme.success
        } else if is_failed {
            theme.danger
        } else {
            Color::Reset
        };

        let mut spans = vec![
            Span::styled(cursor, Style::default().fg(theme.accent).bg(bg)),
            Span::styled(icon.to_string(), icon_style.bg(bg)),
            Span::styled(" ", Style::default().bg(bg)),
            Span::styled(
                format!("{:<agent_w$}", truncate_middle(&agent.id, agent_w)),
                Style::default()
                    .fg(accent)
                    .bg(bg)
                    .add_modifier(if is_active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                format!("{:<10}", model),
                Style::default().fg(theme.muted).bg(bg),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                format!("{:<7}", state_label),
                Style::default()
                    .fg(if status_color == Color::Reset {
                        theme.muted
                    } else {
                        Color::Black
                    })
                    .bg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                format!("{:<task_w$}", task),
                Style::default().fg(theme.foreground).bg(bg),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                format!("{:>6}", tokens_str),
                Style::default().fg(theme.foreground).bg(bg),
            ),
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled(
                format!("{:>6}", cost_str),
                Style::default().fg(theme.muted).bg(bg),
            ),
        ];
        spans.push(Span::styled(" ", Style::default().bg(bg)));
        spans.extend(gradient_bar(gauge_width, fill_pct, is_active, theme));
        spans.push(Span::styled(
            format!(" {:>3}%", (fill_pct * 100.0).round() as u64),
            Style::default().fg(theme.muted).bg(bg),
        ));

        lines.push(Line::from(spans));

        if is_selected {
            let mut detail = vec![Span::styled("    ", Style::default().bg(bg))];
            detail.push(Span::styled(
                format!(
                    "plan:{} task:{}",
                    agent.plan_id.as_deref().unwrap_or("-"),
                    activity_row
                        .map(|r| r.task.as_str())
                        .unwrap_or(agent.plan_id.as_deref().unwrap_or("-"))
                ),
                Style::default().fg(theme.muted).bg(bg),
            ));
            if let Some(row) = activity_row {
                detail.push(Span::styled(
                    format!(
                        "  turns:{}  uptime:{}",
                        row.turns,
                        format_uptime(row.uptime_ms)
                    ),
                    Style::default().fg(theme.muted).bg(bg),
                ));
            }
            if let Some(agent_row) = tui_state.agents.iter().find(|row| row.id == agent.id) {
                if !agent_row.last_output_line.is_empty() {
                    detail.push(Span::styled(
                        format!(
                            "  last: {}",
                            truncate_middle(
                                &agent_row.last_output_line,
                                content_width.saturating_sub(30)
                            )
                        ),
                        Style::default().fg(theme.muted).bg(bg),
                    ));
                }
            }
            lines.push(Line::from(detail));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------------
// Summary line (2 lines)
// ---------------------------------------------------------------------------

fn render_summary_line(frame: &mut Frame<'_>, area: Rect, tui_state: &TuiState, theme: &Theme) {
    let active_count = tui_state
        .agent_summaries
        .iter()
        .filter(|a| AgentStatus::from(a.status.as_str()).is_active())
        .count();
    let total_agents = tui_state.agent_summaries.len();
    let total_tokens = tui_state.cumulative_input_tokens + tui_state.cumulative_output_tokens;
    let cost = tui_state.cost_dollars;

    let line1 = Line::from(vec![
        Span::styled(" agents: ", Style::default().fg(theme.muted)),
        Span::styled(
            format!("{active_count}/{total_agents}"),
            Style::default()
                .fg(if active_count > 0 {
                    theme.accent
                } else {
                    theme.foreground
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  tokens: ", Style::default().fg(theme.muted)),
        Span::styled(
            format_tokens(total_tokens),
            Style::default().fg(theme.foreground),
        ),
        Span::styled("  cost: ", Style::default().fg(theme.muted)),
        Span::styled(
            if cost > 0.001 {
                format!("${:.2}", cost)
            } else {
                "-".to_string()
            },
            Style::default().fg(if cost > 1.0 {
                theme.warning
            } else {
                theme.foreground
            }),
        ),
    ]);

    let sep = "\u{2500}".repeat(area.width.saturating_sub(2) as usize);
    let line2 = Line::from(Span::styled(
        format!(" {sep}"),
        Style::default().fg(Color::Rgb(40, 35, 42)),
    ));

    let para = Paragraph::new(vec![line1, line2]);
    frame.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Right panel: role tabs + agent output
// ---------------------------------------------------------------------------

fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let layout = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // -- Tab bar --
    render_role_tabs(frame, layout[0], tui_state, view_state, theme);

    // -- Output body --
    render_output_body(frame, layout[1], tui_state, view_state, theme);
}

// ---------------------------------------------------------------------------
// Role tabs
// ---------------------------------------------------------------------------

fn render_role_tabs(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Determine which roles have active agents
    let agent_roles: Vec<&str> = tui_state
        .agent_summaries
        .iter()
        .map(|a| a.label.as_str())
        .collect();

    // Selected role from sub_tab
    let selected_role = ROLE_TABS
        .get(view_state.sub_tab.min(ROLE_TABS.len().saturating_sub(1)))
        .map(|(role, _)| *role)
        .unwrap_or("");

    let mut spans: Vec<Span<'_>> = Vec::new();
    spans.push(Span::styled(" ", Style::default()));

    for &(role, label) in ROLE_TABS {
        let is_active = role == selected_role;
        let has_agent = agent_roles.iter().any(|r| *r == role);

        let accent = role_accent(role, theme);
        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(accent)
                .add_modifier(Modifier::BOLD)
        } else if has_agent {
            Style::default().fg(accent)
        } else {
            Style::default().fg(theme.muted)
        };

        spans.push(Span::styled(format!(" {label} "), style));
        spans.push(Span::styled(" ", Style::default()));
    }

    let line = Paragraph::new(Line::from(spans));
    frame.render_widget(line, area);
}

// ---------------------------------------------------------------------------
// Output body
// ---------------------------------------------------------------------------

fn render_output_body(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    if tui_state.agent_topology_visible {
        render_agent_topology_panel(frame, area, tui_state, theme);
        return;
    }

    let selected_agent = tui_state.agent_summaries.get(view_state.selected);
    let selected_row = tui_state.agents.get(view_state.selected);
    let selected_id = selected_agent
        .map(|agent| agent.id.as_str())
        .or_else(|| selected_row.map(|row| row.id.as_str()))
        .unwrap_or("");
    let selected_status = selected_agent
        .map(|agent| agent.status.as_str())
        .or_else(|| selected_row.map(|row| row.status.label()))
        .unwrap_or("idle");
    let selected_role = selected_agent
        .map(|agent| agent.label.as_str())
        .or_else(|| selected_row.map(|row| row.role.as_str()))
        .unwrap_or("");
    let accent = role_accent(selected_role, theme);
    let focused = matches!(
        tui_state.focus,
        FocusZone::AgentOutput | FocusZone::RightPanel
    );

    let title_label = if selected_id.is_empty() {
        "Agent Output".to_string()
    } else {
        format!(
            "Output \u{00b7} {} \u{00b7} {}",
            selected_id, selected_status
        )
    };

    let border_style = if focused {
        Theme::focused_border_style()
    } else {
        theme.muted()
    };
    let title_style = if focused {
        Theme::focused_title_style()
    } else if selected_agent.is_some_and(|a| AgentStatus::from(a.status.as_str()).is_active()) {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        theme.muted()
    };

    let collected = collect_agent_output_lines(tui_state, view_state.selected);
    let output_lines = if collected.is_empty() {
        Vec::new()
    } else {
        tui_state.render_agent_output_lines(selected_id, &collected, theme)
    };
    let total_lines = output_lines.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let show_stream_panel = inner.height >= 11;
    let layout = if show_stream_panel {
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(4),
            Constraint::Length(7),
        ])
        .split(inner)
    } else {
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner)
    };
    let output_area = layout[1];
    let stream_area = show_stream_panel.then_some(layout[2]);

    render_route_metrics_bar(frame, layout[0], tui_state, view_state, theme);

    if output_area.width == 0 || output_area.height == 0 {
        return;
    }

    let visible_height = output_area.height as usize;
    let max_scroll = total_lines
        .saturating_sub(visible_height)
        .min(u16::MAX as usize);
    let scroll = tui_state.agent_scroll.unwrap_or(max_scroll).min(max_scroll);
    let tail_indicator = if tui_state.agent_scroll.is_none() {
        "[TAIL]".to_string()
    } else {
        format!("[PINNED line {}]", scroll.saturating_add(1))
    };
    let block = block.border_style(border_style).title(vec![
        Span::styled(format!(" {title_label}"), title_style),
        Span::styled(
            format!(" {tail_indicator} "),
            if tui_state.agent_scroll.is_none() {
                Style::default().fg(theme.success)
            } else {
                Style::default().fg(theme.warning)
            },
        ),
    ]);
    frame.render_widget(block, area);

    if output_lines.is_empty() {
        // Centered empty state
        let v_pad = output_area.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(2) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "waiting for agent output...",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "output will stream here when agents are active",
            Style::default().fg(theme.muted),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, output_area);
    } else {
        let paragraph = Paragraph::new(output_lines)
            .style(theme.text())
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0));
        frame.render_widget(paragraph, output_area);
    }

    if let Some(stream_area) = stream_area {
        render_live_stream_panel(frame, stream_area, selected_id, tui_state, theme);
    }
}

fn render_agent_topology_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    theme: &Theme,
) {
    let focused = matches!(
        tui_state.focus,
        FocusZone::AgentOutput | FocusZone::RightPanel
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            Theme::focused_border_style()
        } else {
            theme.muted()
        })
        .title(Span::styled(
            " Agent Topology ",
            if focused {
                Theme::focused_title_style()
            } else {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            },
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
    let status_text = topology_status_text(tui_state);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(status_text, Style::default().fg(theme.muted)),
        ])),
        sections[0],
    );

    let body_lines = agent_topology_lines(tui_state)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    let viewport_height = sections[1].height as usize;
    let max_scroll = body_lines
        .len()
        .saturating_sub(viewport_height)
        .min(u16::MAX as usize);
    let scroll = tui_state.agent_topology_scroll_offset.min(max_scroll);
    frame.render_widget(
        Paragraph::new(body_lines)
            .style(theme.text())
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0)),
        sections[1],
    );
}

fn topology_status_text(tui_state: &TuiState) -> String {
    match &tui_state.agent_topology_status {
        AgentTopologyStatus::Idle => "press Ctrl+T to load topology".to_string(),
        AgentTopologyStatus::Loading => "loading topology...".to_string(),
        AgentTopologyStatus::Ready => format!(
            "{} nodes · {} edges · Ctrl+T closes",
            tui_state.agent_topology.nodes.len(),
            tui_state.agent_topology.edges.len()
        ),
        AgentTopologyStatus::Unavailable => {
            "topology not available from this roko serve".to_string()
        }
        AgentTopologyStatus::Error(message) => {
            format!("topology fetch failed · {}", truncate_middle(message, 48))
        }
    }
}

pub(crate) fn agent_topology_lines(tui_state: &TuiState) -> Vec<String> {
    match &tui_state.agent_topology_status {
        AgentTopologyStatus::Idle => vec!["topology not loaded yet".to_string()],
        AgentTopologyStatus::Loading => vec!["loading topology...".to_string()],
        AgentTopologyStatus::Unavailable => {
            vec!["topology not available from this roko serve".to_string()]
        }
        AgentTopologyStatus::Error(message) => vec![
            "topology fetch failed".to_string(),
            truncate_middle(message, 72),
        ],
        AgentTopologyStatus::Ready => build_agent_topology_lines(tui_state),
    }
}

fn build_agent_topology_lines(tui_state: &TuiState) -> Vec<String> {
    if tui_state.agent_topology.nodes.is_empty() {
        return vec!["no topology nodes reported".to_string()];
    }

    let mut tasks_by_agent: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for task in &tui_state.active_task_summaries {
        for agent_id in &task.assigned_agents {
            tasks_by_agent
                .entry(agent_id.clone())
                .or_default()
                .push((task.task_id.clone(), task.status.clone()));
        }
    }
    for tasks in tasks_by_agent.values_mut() {
        tasks.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
    }

    let mut nodes = tui_state.agent_topology.nodes.clone();
    nodes.sort_by(|lhs, rhs| lhs.id.cmp(&rhs.id));

    let mut lines = vec!["└── pool: default".to_string()];
    for (idx, node) in nodes.iter().enumerate() {
        let is_last_node = idx + 1 == nodes.len();
        let node_branch = if is_last_node {
            "    └──"
        } else {
            "    ├──"
        };
        let child_prefix = if is_last_node {
            "        "
        } else {
            "    │   "
        };
        let status = tui_state
            .agents
            .iter()
            .find(|agent| agent.id == node.id)
            .map(|agent| agent.status.label())
            .unwrap_or("idle");
        lines.push(format!("{node_branch} {} [{}]", node.id, status));

        let mut children = Vec::new();
        if let Some(tasks) = tasks_by_agent.get(&node.id) {
            for (task_idx, (task_id, task_status)) in tasks.iter().enumerate() {
                let branch = if task_idx + 1 == tasks.len() && node.address.is_empty() {
                    "└──"
                } else {
                    "├──"
                };
                children.push(format!(
                    "{child_prefix}{branch} task: {} ({})",
                    truncate_middle(task_id, 36),
                    task_status
                ));
            }
        }
        if !node.address.is_empty() {
            children.push(format!(
                "{child_prefix}└── addr: {}",
                truncate_middle(&node.address, 42)
            ));
        }
        if children.is_empty() {
            children.push(format!("{child_prefix}└── no active tasks"));
        }
        lines.extend(children);
    }

    lines
}

fn render_live_stream_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    agent_id: &str,
    tui_state: &TuiState,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let stream = (!agent_id.is_empty())
        .then(|| tui_state.agent_streams.get(agent_id))
        .flatten();
    let (status_label, title_style) = match stream {
        Some(stream) if stream.connected => (
            "connected",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Some(stream) if stream.completed => ("done", Style::default().fg(theme.success)),
        Some(_) => ("connecting...", Style::default().fg(theme.warning)),
        None => ("connecting...", Style::default().fg(theme.muted)),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted())
        .title(vec![
            Span::styled(" Live Stream ", title_style),
            Span::styled(
                format!(" {status_label} "),
                Style::default().fg(theme.muted),
            ),
        ]);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let body = if agent_id.is_empty() {
        "select an agent to view the live tail".to_string()
    } else if let Some(stream) = stream {
        let chunks = stream.chunks.iter().cloned().collect::<Vec<_>>();
        if chunks.is_empty() {
            if stream.connected {
                "waiting for live chunks...".to_string()
            } else {
                "connecting...".to_string()
            }
        } else {
            let visible_lines = inner.height as usize;
            let start = chunks.len().saturating_sub(visible_lines);
            chunks[start..].join("\n")
        }
    } else {
        "connecting...".to_string()
    };

    let paragraph = Paragraph::new(body)
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

pub(crate) fn collect_agent_output_lines(tui_state: &TuiState, selected: usize) -> Vec<String> {
    let selected_agent = tui_state.agent_summaries.get(selected);

    // Priority:
    //   1. current_plan_execution.agent_output_tail
    //   2. selected agent's live row data from tui_state.agents
    //   3. task_output_tails for the agent's current task
    //   4. episode output text
    let collected: Vec<String> = tui_state
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.clone())
        .unwrap_or_default();

    if !collected.is_empty() {
        return collected;
    }

    if let Some(agent_summary) = selected_agent {
        if let Some(agent_row) = tui_state
            .agents
            .iter()
            .find(|row| row.id == agent_summary.id)
        {
            if !agent_row.output_lines.is_empty() {
                return agent_row.output_lines.clone();
            }
            if !agent_row.last_output_line.is_empty() {
                return vec![agent_row.last_output_line.clone()];
            }
            if !agent_row.current_task.is_empty() {
                let task_output = tui_state
                    .task_output_tails
                    .get(&agent_row.current_task)
                    .cloned()
                    .unwrap_or_default();
                if !task_output.is_empty() {
                    return task_output;
                }
            }
        }

        for episode in &tui_state.episodes_cache {
            if episode.agent_id != agent_summary.id {
                continue;
            }
            for key in [
                "stderr",
                "agent_stderr",
                "output",
                "stdout",
                "agent_output",
                "output_tail",
            ] {
                if let Some(text) = episode.extra.get(key).and_then(|v| v.as_str()) {
                    if !text.trim().is_empty() {
                        return text.lines().map(String::from).collect();
                    }
                }
            }
        }
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Gradient gauge bar (inline, matching Mori's per-cell gradient)
// ---------------------------------------------------------------------------

fn gradient_bar<'a>(width: usize, fill_pct: f64, active: bool, theme: &Theme) -> Vec<Span<'a>> {
    if width == 0 {
        return Vec::new();
    }
    let pct = fill_pct.clamp(0.0, 1.0);
    let filled = (pct * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::with_capacity(filled + 1);

    // Gradient from theme.success -> theme.accent -> theme.warning -> theme.danger
    for i in 0..filled {
        let t = if filled > 1 {
            i as f64 / (filled - 1) as f64
        } else {
            pct
        };
        let color = gradient_sample(t, active, theme);
        spans.push(Span::styled(
            "\u{2588}".to_string(),
            Style::default().fg(color),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(Color::Rgb(40, 35, 42)),
        ));
    }
    spans
}

/// Sample a gradient color from teal -> accent -> warning based on position.
fn gradient_sample(t: f64, active: bool, theme: &Theme) -> Color {
    let (r0, g0, b0) = color_to_rgb(theme.info);
    let (r1, g1, b1) = color_to_rgb(theme.accent);
    let (r2, g2, b2) = color_to_rgb(theme.warning);

    let (r, g, b) = if t < 0.5 {
        let s = t * 2.0;
        lerp_rgb((r0, g0, b0), (r1, g1, b1), s)
    } else {
        let s = (t - 0.5) * 2.0;
        lerp_rgb((r1, g1, b1), (r2, g2, b2), s)
    };

    // Breathing effect for active agents
    let (r, g, b) = if active {
        let br = 1.1;
        (
            (r as f64 * br).min(255.0) as u8,
            (g as f64 * br).min(255.0) as u8,
            (b as f64 * br).min(255.0) as u8,
        )
    } else {
        (r, g, b)
    };

    Color::Rgb(r, g, b)
}

fn lerp_rgb(from: (u8, u8, u8), to: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    (
        (from.0 as f64 + (to.0 as f64 - from.0 as f64) * t) as u8,
        (from.1 as f64 + (to.1 as f64 - from.1 as f64) * t) as u8,
        (from.2 as f64 + (to.2 as f64 - from.2 as f64) * t) as u8,
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Role-specific accent color.
fn role_accent(role: &str, theme: &Theme) -> Color {
    match role.to_lowercase().as_str() {
        "implementer" | "impl" => Color::Rgb(185, 120, 148), // rose
        "strategist" | "strat" => Color::Rgb(120, 115, 165), // indigo
        "architect" | "arch" => Color::Rgb(125, 158, 140),   // sage
        "auditor" | "audit" => Color::Rgb(195, 155, 95),     // amber
        "critic" | "crit" => Color::Rgb(195, 110, 85),       // ember
        "conductor" | "cond" => Color::Rgb(155, 130, 175),   // lavender
        "researcher" | "res" => Color::Rgb(100, 150, 170),   // teal
        _ => theme.accent,
    }
}

/// Format a token count as compact string.
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

fn format_uptime(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else if ms < 3_600_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    }
}

fn render_route_metrics_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let selected_agent = tui_state.agent_summaries.get(view_state.selected);
    let selected_id = selected_agent
        .map(|a| a.id.as_str())
        .or_else(|| {
            tui_state
                .agents
                .get(view_state.selected)
                .map(|agent| agent.id.as_str())
        })
        .unwrap_or("");
    let agent_row = tui_state.agents.iter().find(|row| row.id == selected_id);
    let metrics = tui_state.route_metrics.get(selected_id);

    let model = metrics
        .map(|metric| metric.model.as_str())
        .filter(|model| !model.is_empty())
        .or_else(|| {
            agent_row
                .map(|row| row.model.as_str())
                .filter(|model| !model.is_empty())
        })
        .unwrap_or("");
    let model_label = if model.is_empty() || model == "-" {
        "unknown".to_string()
    } else {
        shorten_model(model)
    };
    let context_used = metrics
        .map(|metric| metric.context_used)
        .unwrap_or_else(|| agent_row.map_or(0, |row| row.input_tokens + row.output_tokens));
    let context_limit = metrics
        .map(|metric| metric.context_limit)
        .filter(|limit| *limit > 0)
        .or_else(|| {
            agent_row
                .map(|row| row.context_limit)
                .filter(|limit| *limit > 0)
        })
        .unwrap_or_else(|| model_context_limit(model));
    let utilization = if context_limit > 0 {
        (context_used as f64 / context_limit as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let focus_score = metrics.map_or(0.0, |metric| metric.focus_score);
    let tier = metrics
        .map(|metric| metric.tier.as_str())
        .filter(|tier| !tier.is_empty())
        .unwrap_or("balanced");

    let usage_color = if utilization >= 0.8 {
        theme.danger
    } else if utilization >= 0.5 {
        theme.warning
    } else {
        theme.success
    };
    let focus_color = if focus_score >= 0.75 {
        theme.foreground
    } else if focus_score >= 0.4 {
        theme.muted
    } else {
        Color::Rgb(110, 95, 115)
    };

    let line = Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(
            format!("[{}]", model_label),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled("ctx:", Style::default().fg(theme.muted)),
        Span::styled(
            format!(
                " {}/{}",
                format_tokens(context_used),
                format_tokens(context_limit)
            ),
            Style::default()
                .fg(usage_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" │ ", Style::default().fg(theme.muted)),
        Span::styled("focus:", Style::default().fg(theme.muted)),
        Span::styled(
            format!(" {:.2}", focus_score),
            Style::default()
                .fg(focus_color)
                .add_modifier(if focus_score >= 0.75 {
                    Modifier::BOLD
                } else {
                    Modifier::DIM
                }),
        ),
        Span::styled(" │ ", Style::default().fg(theme.muted)),
        Span::styled("tier:", Style::default().fg(theme.muted)),
        Span::styled(format!(" {}", tier), Style::default().fg(theme.foreground)),
    ]);

    frame.render_widget(Paragraph::new(line).wrap(Wrap { trim: false }), area);
}

fn is_agent_active(status: &str) -> bool {
    matches!(status, "running" | "active")
}

fn is_agent_done(status: &str) -> bool {
    matches!(status, "done" | "completed")
}

fn is_agent_failed(status: &str) -> bool {
    matches!(status, "error" | "failed")
}

fn agent_status_rank(status: &str) -> u8 {
    if is_agent_active(status) {
        0
    } else if matches!(status, "idle" | "waiting") {
        1
    } else if is_agent_done(status) {
        2
    } else if is_agent_failed(status) {
        3
    } else {
        4
    }
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
