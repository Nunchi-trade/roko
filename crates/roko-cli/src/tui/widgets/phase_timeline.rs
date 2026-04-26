//! Horizontal phase timeline widget.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::dashboard::Theme;

/// A single phase in the timeline.
#[derive(Debug, Clone)]
pub struct PhaseEntry {
    pub name: String,
    pub elapsed_secs: f64,
}

/// Render a horizontal phase timeline.
///
/// Standard phases: preflight, strategist, implementer, verifier, reviewer, committing.
/// Current phase is highlighted with accent color, completed phases in success,
/// pending phases in muted.
pub fn render_phase_timeline(
    frame: &mut Frame<'_>,
    area: Rect,
    phases: &[PhaseEntry],
    current_idx: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("phase timeline")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 || phases.is_empty() {
        return;
    }

    // Compute proportional widths based on elapsed time.
    let total_elapsed: f64 = phases.iter().map(|p| p.elapsed_secs).sum();
    let min_width: u16 = 8; // Minimum column width for readability.

    let constraints: Vec<Constraint> = if total_elapsed <= 0.0 {
        // Equal widths if no time data.
        vec![Constraint::Ratio(1, phases.len() as u32); phases.len()]
    } else {
        phases
            .iter()
            .map(|phase| {
                let ratio = phase.elapsed_secs / total_elapsed;
                let width = ((inner.width as f64) * ratio).round() as u16;
                Constraint::Min(width.max(min_width))
            })
            .collect()
    };

    let columns = Layout::horizontal(constraints).split(inner);

    for (i, (phase, col)) in phases.iter().zip(columns.iter()).enumerate() {
        let (style, border_style) = if i == current_idx {
            (
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
                Style::default().fg(theme.accent),
            )
        } else if i < current_idx {
            (theme.success(), Style::default().fg(theme.success))
        } else {
            (theme.muted(), Style::default().fg(theme.muted))
        };

        let elapsed_label = format_elapsed(phase.elapsed_secs);
        let content = if col.height >= 3 {
            vec![
                Line::from(Span::styled(phase.name.clone(), style)),
                Line::from(Span::styled(elapsed_label, Style::default().fg(theme.muted))),
            ]
        } else {
            vec![Line::from(Span::styled(phase.name.clone(), style))]
        };

        let paragraph = Paragraph::new(content)
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style),
            );

        frame.render_widget(paragraph, *col);
    }
}

fn format_elapsed(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.1}s", secs)
    } else {
        let mins = (secs / 60.0).floor() as u64;
        let remaining = secs - (mins as f64 * 60.0);
        format!("{mins}m{:.0}s", remaining)
    }
}
