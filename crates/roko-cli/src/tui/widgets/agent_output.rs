//! Agent output widget — shows per-agent role tabs and scrollable output.
//!
//! Ported from Mori's agent_output.rs (core render logic only).

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::rosedust::MoriTheme;
use super::super::state::TuiState;

// ---------------------------------------------------------------------------
// Role tab labels
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

/// Render the agent output pane with role tabs and scrollable text.
pub fn render_agent_output(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // -- Tab bar --
    render_tabs(frame, layout[0], state);

    // -- Output body --
    let selected_role = state
        .agents
        .get(state.selected_agent)
        .map(|a| a.role.as_str())
        .unwrap_or("");
    let accent = MoriTheme::role_accent(selected_role);

    // Gather output from the selected agent
    let output_text = state
        .agents
        .get(state.selected_agent)
        .map(|a| a.last_output_line.as_str())
        .unwrap_or("");

    let visible_height = layout[1].height.saturating_sub(2) as usize;
    let all_lines: Vec<&str> = if output_text.is_empty() {
        Vec::new()
    } else {
        output_text.lines().collect()
    };
    let total = all_lines.len();

    // Auto-scroll: show tail unless user has scrolled up
    let start = if state.output_scroll == 0 {
        total.saturating_sub(visible_height)
    } else {
        state
            .output_scroll
            .min(total.saturating_sub(visible_height.min(total)))
    };
    let end = (start + visible_height).min(total);

    let lines: Vec<Line> = if total == 0 {
        let agent_active = state
            .agents
            .get(state.selected_agent)
            .map_or(false, |a| a.active);
        if agent_active {
            vec![Line::from(Span::styled(
                format!(" {} waiting for output...", state.atmosphere.spinner()),
                Style::default().fg(MoriTheme::TEXT_DIM),
            ))]
        } else {
            vec![Line::from(Span::styled(
                " No output",
                Style::default().fg(MoriTheme::TEXT_DIM),
            ))]
        }
    } else {
        all_lines[start..end]
            .iter()
            .map(|&line| {
                let style = Style::default().fg(MoriTheme::FG_DIM);
                Line::from(vec![Span::raw(" "), Span::styled(line, style)])
            })
            .collect()
    };

    let title_label = if selected_role.is_empty() {
        "Agent Output".to_string()
    } else {
        format!("Output \u{00b7} {}", selected_role)
    };

    let (border_s, title_s) = if focused {
        (
            MoriTheme::focused_border_style(),
            MoriTheme::focused_title_style(),
        )
    } else {
        (
            MoriTheme::unfocused_border_style(),
            MoriTheme::unfocused_title_style(),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title_label)
        .style(MoriTheme::block_style())
        .border_style(border_s)
        .title_style(title_s);

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, layout[1]);
}

// ---------------------------------------------------------------------------
// Tab bar
// ---------------------------------------------------------------------------

fn render_tabs(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let selected_role = state
        .agents
        .get(state.selected_agent)
        .map(|a| a.role.as_str())
        .unwrap_or("");

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(" ", Style::default().bg(MoriTheme::BG)));

    for &(role, label) in ROLE_TABS {
        let is_active = role == selected_role;
        let has_agent = state.agents.iter().any(|a| a.role == role);

        let style = if is_active {
            MoriTheme::tab_active_style()
        } else if has_agent {
            Style::default()
                .fg(MoriTheme::role_accent(role))
                .bg(MoriTheme::BG_SECONDARY)
        } else {
            MoriTheme::tab_inactive_style()
        };

        spans.push(Span::styled(format!(" {} ", label), style));
        spans.push(Span::styled(" ", Style::default().bg(MoriTheme::BG)));
    }

    let line = Line::from(spans);
    let para = Paragraph::new(line).style(Style::default().bg(MoriTheme::BG));
    frame.render_widget(para, area);
}
