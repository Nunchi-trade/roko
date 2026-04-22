//! Parallel agent roster table widget.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::super::dashboard::Theme;
use super::super::state::{AgentRow, AgentStatus};

/// Render a table of parallel agent instances.
///
/// Active agents are sorted first. The selected row is highlighted.
pub(crate) fn render_parallel_pool(
    frame: &mut Frame<'_>,
    area: Rect,
    agents: &[AgentRow],
    selected: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("parallel agents")
        .border_style(theme.muted())
        .title_style(theme.accent_bold());
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
    let mut sorted: Vec<(usize, &AgentRow)> = agents.iter().enumerate().collect();
    sorted.sort_by(|(_, a), (_, b)| {
        b.active
            .cmp(&a.active)
            .then_with(|| a.role.cmp(&b.role))
            .then_with(|| a.id.cmp(&b.id))
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

            let current_task = if !agent.current_task.is_empty() {
                agent.current_task.clone()
            } else if !agent.current_plan.is_empty() {
                agent.current_plan.clone()
            } else {
                "-".to_string()
            };
            let status = agent.status;
            let ctx_limit = agent.context_limit.max(1);
            let ctx_ratio = (agent.input_tokens as f64 / ctx_limit as f64).clamp(0.0, 1.0);

            Row::new(vec![
                Cell::from(truncate(&agent.id, 12)),
                Cell::from(truncate(&agent.role, 10)),
                Cell::from(truncate(&agent.model, 12)),
                Cell::from(truncate(&current_task, 18)),
                Cell::from(render_status_label(status, theme)),
                Cell::from(render_context_gauge(
                    agent.input_tokens,
                    ctx_limit,
                    ctx_ratio,
                    theme,
                )),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Min(10),
            Constraint::Length(8),
            Constraint::Min(14),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("agent id"),
            Cell::from("role"),
            Cell::from("model"),
            Cell::from("task"),
            Cell::from("progress"),
            Cell::from("context"),
        ])
        .style(
            Style::default()
                .fg(theme.accent)
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

fn render_status_label(status: AgentStatus, theme: &Theme) -> Line<'static> {
    let (label, color) = match status {
        AgentStatus::Active => ("active", theme.accent),
        AgentStatus::Idle => ("idle", theme.muted),
        AgentStatus::Done => ("done", theme.success),
        AgentStatus::Failed => ("failed", theme.danger),
    };

    Line::from(vec![Span::styled(
        format!("{:^8}", label),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )])
}

fn render_context_gauge(
    input_tokens: u64,
    context_limit: u64,
    ctx_ratio: f64,
    theme: &Theme,
) -> Line<'static> {
    let gauge_width = 6usize;
    let filled = (ctx_ratio * gauge_width as f64).round() as usize;
    let empty = gauge_width.saturating_sub(filled);
    let fill_color = if ctx_ratio >= 0.8 {
        theme.danger
    } else if ctx_ratio >= 0.5 {
        theme.warning
    } else {
        theme.accent
    };

    let label = format!("{}k/{}k", input_tokens / 1000, context_limit.max(1) / 1000);

    Line::from(vec![
        Span::styled(
            "\u{2588}".repeat(filled),
            Style::default().fg(fill_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2500}".repeat(empty), Style::default().fg(theme.muted)),
        Span::styled(" ", Style::default()),
        Span::styled(label, Style::default().fg(theme.foreground)),
    ])
}
