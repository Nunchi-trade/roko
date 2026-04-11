//! Config / stats view — F6 tab.
//!
//! Two-column stats dashboard: plan/agent counts on the left,
//! gate results, C-Factor, tokens, and cost on the right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::TuiState;

/// Render the F6 config / stats view.
pub fn render_config_view(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Stats & Config ")
        .border_style(Style::default().fg(MoriTheme::TEXT_PHANTOM));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    frame.render_widget(left_panel(state), columns[0]);
    frame.render_widget(right_panel(state), columns[1]);
}

// ---------------------------------------------------------------------------
// Left column: plan + agent stats
// ---------------------------------------------------------------------------

fn left_panel(state: &TuiState) -> Paragraph<'static> {
    let total_plans = state.plans.len();
    let active_plans = state.plans.iter().filter(|p| p.active).count();
    let done_plans = state.plans.iter().filter(|p| !p.active).count();

    let (tasks_done, tasks_total) = state.task_counts();
    let tasks_failed: usize = state.plans.iter().map(|p| p.tasks_failed).sum();

    let total_agents = state.agents.len();
    let active_agents = state.active_agent_count();

    let lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "  Plans",
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        stat("  Total", total_plans),
        stat("  Active", active_plans),
        stat("  Done", done_plans),
        Line::from(""),
        Line::from(Span::styled(
            "  Tasks",
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        stat("  Done", tasks_done),
        stat("  Total", tasks_total),
        stat("  Failed", tasks_failed),
        Line::from(""),
        Line::from(Span::styled(
            "  Agents",
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        stat("  Active", active_agents),
        stat("  Total", total_agents),
    ];

    Paragraph::new(lines)
        .style(Style::default().fg(MoriTheme::TEXT))
        .wrap(Wrap { trim: false })
}

// ---------------------------------------------------------------------------
// Right column: gate results, C-Factor, tokens, cost
// ---------------------------------------------------------------------------

fn right_panel(state: &TuiState) -> Paragraph<'static> {
    let gates_passed = state.gate_results.iter().filter(|g| g.passed).count();
    let gates_failed = state.gate_results.iter().filter(|g| !g.passed).count();

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "  Gates",
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        stat("  Passed", gates_passed),
        stat("  Failed", gates_failed),
        Line::from(""),
    ];

    // C-Factor
    if let Some(cf) = &state.cfactor {
        let score_str = format!("{:.3}", cf.overall);
        lines.push(Line::from(Span::styled(
            "  C-Factor",
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("  Score:  ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(
                score_str,
                Style::default()
                    .fg(MoriTheme::BONE)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(stat("  Episodes", cf.episode_count));
        lines.push(Line::from(""));
    }

    // Tokens + cost
    lines.push(Line::from(Span::styled(
        "  Efficiency",
        Style::default()
            .fg(MoriTheme::BONE)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let token_str = format_tokens(state.token_total);
    lines.push(Line::from(vec![
        Span::styled("  Tokens: ", Style::default().fg(MoriTheme::TEXT_DIM)),
        Span::styled(
            token_str,
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let cost_str = format!("${:.4}", state.cost_dollars);
    lines.push(Line::from(vec![
        Span::styled("  Cost:   ", Style::default().fg(MoriTheme::TEXT_DIM)),
        Span::styled(
            cost_str,
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    Paragraph::new(lines)
        .style(Style::default().fg(MoriTheme::TEXT))
        .wrap(Wrap { trim: false })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stat(label: &'static str, value: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(MoriTheme::TEXT_DIM)),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(MoriTheme::BONE)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn format_tokens(total: u64) -> String {
    if total >= 1_000_000 {
        format!("{:.1}M", total as f64 / 1_000_000.0)
    } else if total >= 1_000 {
        format!("{:.1}K", total as f64 / 1_000.0)
    } else {
        total.to_string()
    }
}
