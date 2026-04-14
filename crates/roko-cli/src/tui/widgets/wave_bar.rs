//! Wave progress bar widget.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::dashboard::Theme;

/// Progress state for the current wave.
#[derive(Debug, Clone)]
pub struct WaveProgress {
    pub wave_number: u32,
    pub plan_count: u32,
    pub task_count: u32,
    pub tasks_done: u32,
    pub eta_secs: Option<f64>,
}

impl WaveProgress {
    /// Completion ratio for this wave.
    pub fn ratio(&self) -> f64 {
        if self.task_count == 0 {
            0.0
        } else {
            (self.tasks_done as f64 / self.task_count as f64).clamp(0.0, 1.0)
        }
    }
}

/// Render a compact 1-line wave progress bar.
///
/// Shows: wave number, progress bar, plan count, task count, and ETA.
pub fn render_wave_bar(frame: &mut Frame<'_>, area: Rect, wave: &WaveProgress, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let ratio = wave.ratio();
    let pct = (ratio * 100.0).round() as u64;

    // Build the semantic bar.
    let meta_width = 40_usize; // Space for labels on either side.
    let bar_width = (area.width as usize).saturating_sub(meta_width).max(4);
    let filled = (ratio * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar_filled: String = "█".repeat(filled);
    let bar_empty: String = "░".repeat(empty);

    let bar_color = if ratio >= 0.9 {
        theme.success
    } else if ratio >= 0.5 {
        theme.accent
    } else {
        theme.warning
    };

    let eta_label = match wave.eta_secs {
        Some(secs) if secs < 60.0 => format!("ETA {:.0}s", secs),
        Some(secs) => format!("ETA {:.0}m", secs / 60.0),
        None => String::from("ETA --"),
    };

    let spans = vec![
        Span::styled(
            format!("W{} ", wave.wave_number),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(bar_filled, Style::default().fg(bar_color)),
        Span::styled(bar_empty, Style::default().fg(theme.muted)),
        Span::raw(" "),
        Span::styled(
            format!("{pct}%"),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{}P {}T ", wave.plan_count, wave.task_count),
            Style::default().fg(theme.muted),
        ),
        Span::styled(eta_label, Style::default().fg(theme.warning)),
    ];

    let line = Paragraph::new(Line::from(spans));
    frame.render_widget(line, area);
}
