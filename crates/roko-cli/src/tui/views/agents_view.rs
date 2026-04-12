//! F3 Agents view -- roster + output panel.
//!
//! Two-panel layout: left 35% agent roster (pool table),
//! right 65% agent output with scroll pinning (auto-tail + manual pin).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
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

/// Left panel: agent roster table.
fn render_agent_roster(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agent Roster ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use agent_pool / parallel_pool widget here when available
    if data.agents.is_empty() {
        let empty = Paragraph::new("no agents registered")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = data
        .agents
        .iter()
        .enumerate()
        .map(|(i, agent)| {
            let style = if i == view_state.selected {
                theme.selection()
            } else {
                match agent.status.as_str() {
                    "running" => theme.info(),
                    "idle" => theme.muted(),
                    "error" | "failed" => theme.danger(),
                    _ => theme.text(),
                }
            };
            Row::new(vec![
                Cell::from(truncate(&agent.id, 14)),
                Cell::from(agent.label.as_str()),
                Cell::from(status_icon(agent.status.as_str())),
            ])
            .style(style)
        })
        .collect();

    let widths = [Constraint::Min(10), Constraint::Min(12), Constraint::Length(8)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["id", "role", "status"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Right panel: agent output with scroll pinning.
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

    // Output panel
    let tail_indicator = if view_state.auto_tail {
        " [TAIL] "
    } else {
        " [PINNED] "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Output{tail_indicator}"))
        .border_style(theme.accent());
    let inner = block.inner(sections[1]);
    frame.render_widget(block, sections[1]);

    let lines: Vec<&str> = data
        .current_plan_execution
        .as_ref()
        .map(|exec| exec.agent_output_tail.iter().map(String::as_str).collect())
        .unwrap_or_default();

    if lines.is_empty() {
        let empty = Paragraph::new("waiting for agent output...")
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

fn status_icon(status: &str) -> &'static str {
    match status {
        "running" => ">>",
        "idle" => "--",
        "done" | "completed" => "ok",
        "error" | "failed" => "!!",
        _ => "??",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
