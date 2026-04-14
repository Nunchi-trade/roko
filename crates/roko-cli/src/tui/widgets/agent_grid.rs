//! Agent status grid widget.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use roko_core::dashboard_snapshot::AgentState;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Pick an icon and style for an agent based on its active state.
fn agent_decoration(agent: &AgentState, theme: &Theme) -> (&'static str, ratatui::style::Style) {
    if agent.active {
        ("\u{25cf}", theme.success()) // filled circle
    } else {
        ("\u{25cb}", theme.muted()) // hollow circle
    }
}

/// Format byte count as a human-friendly string.
fn fmt_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the agent grid.
///
/// Each agent is shown as a row:
/// ```text
/// [*] coder    12.3 KB
/// [*] reviewer  4.1 KB
/// [ ] planner   0 B
/// ```
pub fn render_agent_grid(frame: &mut Frame<'_>, area: Rect, agents: &[AgentState], theme: &Theme) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted())
        .title(Span::styled("Agents", theme.accent()));

    if agents.is_empty() {
        let empty = Paragraph::new("No agents")
            .style(theme.muted())
            .block(outer);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem<'_>> = agents
        .iter()
        .map(|agent| {
            let (icon, style) = agent_decoration(agent, theme);
            let bytes = fmt_bytes(agent.output_bytes);
            let line = Line::from(vec![
                Span::styled(format!("{icon} "), style),
                Span::styled(&agent.role, theme.text()),
                Span::styled(format!("  {bytes}"), theme.muted()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).block(outer);
    frame.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn agent_grid_renders_without_panic() {
        let backend = TestBackend::new(40, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        let agents = vec![
            AgentState {
                agent_id: "a1".into(),
                role: "coder".into(),
                active: true,
                output_bytes: 12_500,
            },
            AgentState {
                agent_id: "a2".into(),
                role: "reviewer".into(),
                active: false,
                output_bytes: 400,
            },
        ];
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_agent_grid(frame, area, &agents, &theme);
            })
            .unwrap();
    }

    #[test]
    fn agent_grid_empty() {
        let backend = TestBackend::new(40, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_agent_grid(frame, area, &[], &theme);
            })
            .unwrap();
    }

    #[test]
    fn fmt_bytes_units() {
        assert_eq!(fmt_bytes(500), "500 B");
        assert_eq!(fmt_bytes(2048), "2.0 KB");
        assert_eq!(fmt_bytes(1_500_000), "1.4 MB");
    }
}
