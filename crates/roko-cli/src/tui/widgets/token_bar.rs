//! Token/output usage bar widget.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Gauge};

use roko_core::dashboard_snapshot::AgentState;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Maximum expected bytes for gauge scaling (10 MB).
const MAX_BYTES: usize = 10 * 1024 * 1024;

/// Render a gauge showing total bytes processed across all agents.
///
/// ```text
/// ┌ Output ────────────────────────────────────┐
/// │ [=================        ] 1.7 MB / 10 MB │
/// └────────────────────────────────────────────┘
/// ```
pub fn render_token_bar(frame: &mut Frame<'_>, area: Rect, agents: &[AgentState], theme: &Theme) {
    let total_bytes: usize = agents.iter().map(|a| a.output_bytes).sum();
    let ratio = (total_bytes as f64 / MAX_BYTES as f64).min(1.0);
    let label = format!(
        "{} / {} total output",
        fmt_bytes(total_bytes),
        fmt_bytes(MAX_BYTES)
    );

    let gauge = Gauge::default()
        .ratio(ratio)
        .label(label)
        .gauge_style(theme.info())
        .use_unicode(true)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.muted())
                .title(Span::styled("Output", theme.accent())),
        );

    frame.render_widget(gauge, area);
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn token_bar_renders_without_panic() {
        let backend = TestBackend::new(60, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        let agents = vec![
            AgentState {
                agent_id: "a1".into(),
                role: "coder".into(),
                active: true,
                output_bytes: 500_000,
            },
            AgentState {
                agent_id: "a2".into(),
                role: "tester".into(),
                active: true,
                output_bytes: 1_200_000,
            },
        ];
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_token_bar(frame, area, &agents, &theme);
            })
            .unwrap();
    }

    #[test]
    fn token_bar_empty() {
        let backend = TestBackend::new(60, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_token_bar(frame, area, &[], &theme);
            })
            .unwrap();
    }
}
