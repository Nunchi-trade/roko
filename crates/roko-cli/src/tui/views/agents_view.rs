//! F3 Agents view -- roster + output panel.
//!
//! Two-panel layout: left 35% agent roster (pool table),
//! right 65% agent output with scroll pinning (auto-tail + manual pin).

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

/// Render the full agents view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    _tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    render_agent_roster(frame, panels[0], data, view_state, theme);
    render_agent_output(frame, panels[1], data, view_state, theme);
}

/// Left panel: agent roster table with token/cost columns.
fn render_agent_roster(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let active_count = data
        .agents
        .iter()
        .filter(|a| a.status == "running" || a.status == "active")
        .count();
    let title = format!(" Agent Roster ({}/{}) ", active_count, data.agents.len());

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if data.agents.is_empty() {
        // Centered empty state message
        let empty_lines = vec![
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled(
                "no agents registered",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "agents will appear here when",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                "plan execution begins",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    // Build activity snapshot for token/cost info
    let activity =
        crate::tui::dashboard::build_agent_activity_snapshot(&data.agents, &data.efficiency_events);

    let rows: Vec<Row<'_>> = data
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let is_selected = i == view_state.selected;

            let (status_icon, status_style) = match agent.status.as_str() {
                "running" | "active" => (
                    "\u{25ba}",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                "idle" => ("\u{00b7}", Style::default().fg(Color::DarkGray)),
                "done" | "completed" => ("\u{2713}", Style::default().fg(Color::Green)),
                "error" | "failed" => ("\u{2717}", Style::default().fg(Color::Red)),
                _ => ("?", Style::default().fg(Color::DarkGray)),
            };

            let row_style = if is_selected {
                theme.selection()
            } else {
                match agent.status.as_str() {
                    "running" | "active" => theme.info(),
                    "idle" => theme.muted(),
                    "error" | "failed" => theme.danger(),
                    _ => theme.text(),
                }
            };

            // Find activity data for this agent
            let activity_row = activity.as_ref().and_then(|snap| {
                snap.active_agents
                    .iter()
                    .find(|r| r.agent_id == agent.id)
            });

            let tokens_str = activity_row
                .map(|r| format_tokens(r.tokens_used))
                .unwrap_or_else(|| "-".to_string());

            let cost_str = activity_row
                .map(|r| {
                    if r.cost_usd > 0.0 {
                        format!("${:.2}", r.cost_usd)
                    } else {
                        "-".to_string()
                    }
                })
                .unwrap_or_else(|| "-".to_string());

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{status_icon}"),
                    status_style,
                )),
                Cell::from(truncate(&agent.id, 12)),
                Cell::from(agent.label.as_str()),
                Cell::from(tokens_str),
                Cell::from(cost_str),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Min(8),
        Constraint::Min(10),
        Constraint::Length(7),
        Constraint::Length(7),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new([" ", "id", "role", "tokens", "cost"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Right panel: agent output with TAIL/PINNED indicator and scroll.
fn render_agent_output(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

    // Per-role tabs (built from agent list)
    let mut roles: Vec<&str> = Vec::new();
    for agent in &data.agents {
        if !roles.contains(&agent.label.as_str()) {
            roles.push(agent.label.as_str());
        }
    }
    if roles.is_empty() {
        roles.push("all");
    }
    let role_idx = view_state.sub_tab.min(roles.len().saturating_sub(1));
    let role_spans: Vec<Span<'_>> = roles
        .iter()
        .enumerate()
        .flat_map(|(i, role)| {
            let style = if i == role_idx {
                theme.selection()
            } else {
                theme.muted()
            };
            let sep = if i + 1 < roles.len() { " | " } else { "" };
            vec![Span::styled(*role, style), Span::raw(sep)]
        })
        .collect();
    let role_bar = Paragraph::new(Line::from(role_spans));
    frame.render_widget(role_bar, sections[0]);

    // Output panel with TAIL/PINNED indicator in title
    let tail_indicator = if view_state.auto_tail {
        "TAIL"
    } else {
        &format!("PINNED @{}", view_state.scroll)
    };
    let title = format!(" Output [{}] ", tail_indicator);

    let title_style = if view_state.auto_tail {
        theme.accent()
    } else {
        theme.warning()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, title_style))
        .border_style(theme.accent());
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    let lines: Vec<&str> = data
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if lines.is_empty() {
        // Centered empty state
        let v_pad = inner.height / 2;
        let mut empty_lines: Vec<Line<'_>> = Vec::new();
        for _ in 0..v_pad.saturating_sub(1) {
            empty_lines.push(Line::from(""));
        }
        empty_lines.push(Line::from(Span::styled(
            "waiting for agent output...",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
        empty_lines.push(Line::from(""));
        empty_lines.push(Line::from(Span::styled(
            "output will stream here when agents are active",
            Style::default().fg(Color::DarkGray),
        )));
        let empty = Paragraph::new(empty_lines)
            .alignment(Alignment::Center)
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
