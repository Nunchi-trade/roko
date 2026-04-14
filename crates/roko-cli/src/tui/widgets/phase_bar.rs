//! Phase indicator bar showing the current plan phase.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Known phases (in execution order)
// ---------------------------------------------------------------------------

const PHASES: &[&str] = &[
    "compose",
    "dispatch",
    "execute",
    "gate",
    "persist",
    "completed",
];

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render a horizontal phase indicator.
///
/// Each phase label is rendered; the current phase is highlighted.
///
/// ```text
///  compose > dispatch > execute > [gate] > persist > completed
/// ```
pub fn render_phase_bar(frame: &mut Frame<'_>, area: Rect, phase: &str, theme: &Theme) {
    let lower = phase.to_lowercase();

    let spans: Vec<Span<'_>> = PHASES
        .iter()
        .enumerate()
        .flat_map(|(i, &p)| {
            let mut v = Vec::with_capacity(2);
            if i > 0 {
                v.push(Span::styled(" \u{203a} ", theme.muted()));
            }
            if p == lower {
                v.push(Span::styled(
                    format!("[{p}]"),
                    theme.accent().add_modifier(Modifier::BOLD),
                ));
            } else {
                v.push(Span::styled(p, theme.muted()));
            }
            v
        })
        .collect();

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
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
    fn phase_bar_renders_without_panic() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_bar(frame, area, "gate", &theme);
            })
            .unwrap();
    }

    #[test]
    fn phase_bar_unknown_phase() {
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_bar(frame, area, "unknown", &theme);
            })
            .unwrap();
    }
}
