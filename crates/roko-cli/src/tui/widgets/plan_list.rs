//! Animated plan progress list widget.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::dashboard::Theme;

/// A plan entry for the progress list.
#[derive(Debug, Clone)]
pub struct PlanEntry {
    pub name: String,
    pub progress: f64,
    pub tasks_done: u32,
    pub tasks_total: u32,
    pub failed: bool,
}

/// Fractional block characters for smooth progress bars.
const BLOCKS: &[char] = &[' ', '░', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// Render an animated plan progress list with fractional block bars.
///
/// Each plan gets a colored progress bar. Failed plans are highlighted in red.
/// Supports scrolling and cursor selection.
pub fn render_plan_list(
    frame: &mut Frame<'_>,
    area: Rect,
    plans: &[PlanEntry],
    selected: usize,
    scroll: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("plans")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if plans.is_empty() {
        let empty = Paragraph::new("no plans")
            .style(theme.muted())
            .alignment(Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    let visible = inner.height as usize;
    let end = (scroll + visible).min(plans.len());
    let start = scroll.min(end);

    // Each plan takes 2 lines: name line + progress bar line.
    let mut lines: Vec<Line<'_>> = Vec::new();

    for (i, plan) in plans[start..end].iter().enumerate() {
        let idx = start + i;
        let is_selected = idx == selected;

        let name_style = if is_selected {
            theme.selection()
        } else if plan.failed {
            theme.danger()
        } else {
            Style::default().fg(theme.foreground)
        };

        let pct = (plan.progress * 100.0).round() as u64;
        let status = format!(
            " [{}/{}] {pct}%",
            plan.tasks_done, plan.tasks_total
        );

        lines.push(Line::from(vec![
            Span::styled(plan.name.clone(), name_style),
            Span::styled(status, Style::default().fg(theme.muted)),
        ]));

        // Progress bar.
        let bar_width = inner.width.saturating_sub(2) as usize;
        let bar = build_progress_bar(plan.progress, bar_width);
        let bar_color = if plan.failed {
            theme.danger
        } else {
            progress_color(plan.progress, theme)
        };

        lines.push(Line::from(Span::styled(
            bar,
            Style::default().fg(bar_color),
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Build a progress bar string using fractional block characters.
fn build_progress_bar(progress: f64, width: usize) -> String {
    let progress = progress.clamp(0.0, 1.0);
    let filled_exact = progress * width as f64;
    let full_blocks = filled_exact.floor() as usize;
    let fractional = filled_exact - full_blocks as f64;
    let fractional_idx = (fractional * (BLOCKS.len() - 1) as f64).round() as usize;

    let mut bar = String::with_capacity(width);
    for _ in 0..full_blocks.min(width) {
        bar.push('█');
    }
    if full_blocks < width && fractional_idx > 0 {
        bar.push(BLOCKS[fractional_idx.min(BLOCKS.len() - 1)]);
    }
    let remaining = width.saturating_sub(bar.chars().count());
    for _ in 0..remaining {
        bar.push('░');
    }
    bar
}

/// Gradient color based on completion percentage.
fn progress_color(progress: f64, theme: &Theme) -> Color {
    if progress >= 0.9 {
        theme.success
    } else if progress >= 0.5 {
        theme.accent
    } else if progress >= 0.2 {
        theme.warning
    } else {
        theme.muted
    }
}
