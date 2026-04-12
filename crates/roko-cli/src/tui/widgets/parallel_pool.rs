//! Parallel agent roster table widget.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::super::dashboard::Theme;

/// State of a single parallel agent instance.
#[derive(Debug, Clone)]
pub struct ParallelAgentState {
    pub role: String,
    pub model: String,
    pub task: String,
    pub tokens_used: u64,
    pub tokens_total: u64,
    pub state: AgentRunState,
    pub context_pct: f64,
}

/// Running state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRunState {
    Active,
    Idle,
    Done,
    Failed,
}

impl AgentRunState {
    fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Idle => "idle",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    fn color(self, theme: &Theme) -> Color {
        match self {
            Self::Active => theme.accent,
            Self::Idle => theme.muted,
            Self::Done => theme.success,
            Self::Failed => theme.danger,
        }
    }
}

/// Render a table of parallel agent instances.
///
/// Active agents are sorted first. The selected row is highlighted.
pub fn render_parallel_pool(
    frame: &mut Frame<'_>,
    area: Rect,
    agents: &[ParallelAgentState],
    selected: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("parallel agents")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if agents.is_empty() {
        let empty = Paragraph::new("no parallel agents")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Sort: active first, then by role.
    let mut sorted: Vec<(usize, &ParallelAgentState)> = agents.iter().enumerate().collect();
    sorted.sort_by(|(_, a), (_, b)| {
        let a_active = a.state == AgentRunState::Active;
        let b_active = b.state == AgentRunState::Active;
        b_active
            .cmp(&a_active)
            .then_with(|| a.role.cmp(&b.role))
    });

    let rows: Vec<Row<'_>> = sorted
        .iter()
        .map(|(orig_idx, agent)| {
            let is_selected = *orig_idx == selected;
            let row_style = if is_selected {
                theme.selection()
            } else {
                Style::default()
            };

            let state_color = agent.state.color(theme);
            let ctx_pct = format!("{:.0}%", agent.context_pct * 100.0);
            let tokens = format!("{}/{}", agent.tokens_used, agent.tokens_total);

            Row::new(vec![
                Cell::from(truncate(&agent.role, 12)),
                Cell::from(truncate(&agent.model, 14)),
                Cell::from(truncate(&agent.task, 20)),
                Cell::from(tokens),
                Cell::from(Span::styled(
                    agent.state.label().to_string(),
                    Style::default()
                        .fg(state_color)
                        .add_modifier(Modifier::BOLD),
                )),
                Cell::from(ctx_pct),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(14),
            Constraint::Min(16),
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(6),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("role"),
            Cell::from("model"),
            Cell::from("task"),
            Cell::from("tokens"),
            Cell::from("state"),
            Cell::from("ctx"),
        ])
        .style(
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(1);

    frame.render_widget(table, inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
