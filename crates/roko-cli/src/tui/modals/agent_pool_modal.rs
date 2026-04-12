//! Full agent roster modal showing all agent details in a scrollable table.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::super::dashboard::Theme;

/// A row in the agent pool table.
#[derive(Debug, Clone)]
pub struct AgentPoolRow {
    pub role: String,
    pub model: String,
    pub task: String,
    pub tokens: u64,
    pub cost_usd: f64,
    pub state: String,
    pub context_pct: u8,
}

/// Render the agent pool modal.
///
/// Shows all agents in a scrollable table with columns:
/// role, model, task, tokens, cost, state, context%.
/// Centered ~90x70 rectangle.
pub fn render_agent_pool(
    frame: &mut Frame<'_>,
    area: Rect,
    agents: &[AgentPoolRow],
    scroll_offset: u16,
    theme: &Theme,
) {
    let popup = centered_rect(90, 70, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Agent Pool ")
        .title_alignment(Alignment::Center)
        .border_style(theme.accent());

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut lines: Vec<Line<'_>> = Vec::new();

    // Header row
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                " {:<12} {:<14} {:<16} {:>8} {:>8} {:<10} {:>4}",
                "ROLE", "MODEL", "TASK", "TOKENS", "COST", "STATE", "CTX%"
            ),
            theme.accent_bold(),
        ),
    ]));
    // Separator
    lines.push(Line::from(Span::styled(
        " ".to_string() + &"-".repeat(inner.width.saturating_sub(2) as usize),
        theme.muted(),
    )));

    if agents.is_empty() {
        lines.push(Line::from(Span::styled(
            " No agents running.",
            theme.muted(),
        )));
    } else {
        for agent in agents {
            let state_style = match agent.state.as_str() {
                "running" | "active" => theme.success(),
                "idle" | "waiting" => theme.info(),
                "error" | "crashed" => theme.danger(),
                "stopping" => theme.warning(),
                _ => theme.muted(),
            };

            let ctx_style = if agent.context_pct >= 90 {
                theme.danger()
            } else if agent.context_pct >= 70 {
                theme.warning()
            } else {
                theme.muted()
            };

            let task_short = if agent.task.len() > 15 {
                format!("{}...", &agent.task[..12])
            } else {
                agent.task.clone()
            };

            lines.push(Line::from(vec![
                Span::styled(format!(" {:<12}", agent.role), theme.text()),
                Span::styled(format!(" {:<14}", agent.model), theme.muted()),
                Span::styled(format!(" {:<16}", task_short), theme.text()),
                Span::styled(format!(" {:>8}", format_tokens(agent.tokens)), theme.muted()),
                Span::styled(
                    format!(" ${:>7.2}", agent.cost_usd),
                    theme.muted(),
                ),
                Span::styled(format!(" {:<10}", agent.state), state_style),
                Span::styled(format!(" {:>3}%", agent.context_pct), ctx_style),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(" [Esc]", theme.accent_bold()),
        Span::styled(" close   ", theme.muted()),
        Span::styled("[j/k]", theme.accent_bold()),
        Span::styled(" scroll", theme.muted()),
    ]));

    let paragraph = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .scroll((scroll_offset, 0));
    frame.render_widget(paragraph, inner);
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}
