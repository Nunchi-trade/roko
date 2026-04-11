//! Agent roster and activity view (F3).
//!
//! Two-panel layout: agent list (left 40%) + agent output (right 60%).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::TuiState;
use super::super::widgets;

/// Render the agent roster and activity view.
pub fn render_agents_view(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    if state.agents.is_empty() {
        let empty = Paragraph::new("no agents spawned")
            .style(Style::default().fg(MoriTheme::TEXT_DIM))
            .alignment(Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    widgets::agent_pool::render_agent_pool(frame, chunks[0], state, false);
    widgets::agent_output::render_agent_output(frame, chunks[1], state, false);
}
