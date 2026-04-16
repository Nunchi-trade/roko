//! Token burn sparkline widget.
//!
//! Shows an efficiency summary, a token-usage sparkline, and a compact model
//! tier distribution using the live dashboard snapshot.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::braille;
use super::rosedust::brighten;
use crate::tui::dashboard::DashboardData;
use crate::tui::pages::efficiency::build_efficiency_snapshot;
use crate::tui::state::TuiState;
use crate::tui::Theme;

fn fmt_tokens(n: u64) -> String {
    if n == 0 {
        "0".to_string()
    } else if n < 1_000 {
        format!("{n}")
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

fn fmt_rate(rate: f64) -> String {
    if rate <= 0.5 {
        "idle".to_string()
    } else if rate >= 1_000_000.0 {
        format!("{:.1}M/min", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1}k/min", rate / 1_000.0)
    } else {
        format!("{rate:.0}/min")
    }
}

fn tier_color(tier: &str) -> Color {
    match tier {
        "T0" => Theme::SAGE,
        "T1" => Theme::ROSE,
        "T2" => Theme::WARNING,
        _ => Theme::TEXT_DIM,
    }
}

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "T0" => "haiku",
        "T1" => "sonnet",
        "T2" => "opus",
        _ => "other",
    }
}

fn sparkline_window(width: usize, total_samples: usize) -> usize {
    let preferred = if width >= 120 {
        100
    } else if width >= 80 {
        50
    } else {
        10
    };
    preferred.min(total_samples.max(2))
}

/// Render the token burn sparkline widget.
pub fn render_token_sparkline(
    frame: &mut Frame<'_>,
    area: Rect,
    data: &DashboardData,
    state: &TuiState,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    if inner_width < 10 || inner_height < 2 {
        return;
    }

    let snapshot = build_efficiency_snapshot(data);
    let window = sparkline_window(inner_width, snapshot.token_series.len());
    let display: Vec<u64> = snapshot
        .token_series
        .iter()
        .rev()
        .take(window)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let pulsed_color = brighten(Theme::ROSE, state.atmosphere.breathing_brightness());
    let border_color = if snapshot.total_cost_usd > 0.0 {
        Theme::ROSE_DIM
    } else {
        Theme::TEXT_GHOST
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Efficiency")
        .style(Theme::block_style())
        .border_style(Style::default().fg(border_color))
        .title_style(Theme::title_style());
    let inner = block.inner(area);

    if inner.width < 8 || inner.height < 1 {
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    let summary1 = Line::from(vec![
        Span::styled(" tokens ", Style::default().fg(Theme::BONE_DIM)),
        Span::styled(
            fmt_tokens(snapshot.total_tokens),
            Style::default().fg(Theme::BONE),
        ),
        Span::styled(" cost ", Style::default().fg(Theme::BONE_DIM)),
        Span::styled(
            format!("${:.2}", snapshot.total_cost_usd),
            Style::default().fg(Theme::WARNING),
        ),
        Span::styled(" avg/task ", Style::default().fg(Theme::BONE_DIM)),
        Span::styled(
            fmt_tokens(snapshot.average_tokens_per_task.round() as u64),
            Style::default().fg(Theme::FG),
        ),
    ]);
    lines.push(summary1);

    if inner.height > 3 {
        let summary2 = Line::from(vec![
            Span::styled(" succ ", Style::default().fg(Theme::BONE_DIM)),
            Span::styled(
                format!("{:.0}%", snapshot.success_rate * 100.0),
                Style::default().fg(if snapshot.success_rate >= 0.9 {
                    Theme::SAGE
                } else if snapshot.success_rate >= 0.6 {
                    Theme::WARNING
                } else {
                    Theme::EMBER
                }),
            ),
            Span::styled(" events ", Style::default().fg(Theme::BONE_DIM)),
            Span::styled(
                format!("{}", snapshot.event_count),
                Style::default().fg(Theme::TEXT),
            ),
            Span::styled(" window ", Style::default().fg(Theme::BONE_DIM)),
            Span::styled(
                format!("{window}"),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ]);
        lines.push(summary2);
    }

    if display.len() >= 2 {
        let rate = if snapshot.total_tokens > 0 {
            display.iter().copied().sum::<u64>() as f64 / display.len() as f64
        } else {
            0.0
        };
        let spark_w = inner_width
            .saturating_sub(fmt_tokens(snapshot.total_tokens).len() + fmt_rate(rate).len() + 4)
            .max(8);
        let mut spans = vec![Span::styled(
            format!(" {} ", fmt_tokens(snapshot.total_tokens)),
            Style::default().fg(Theme::BONE_DIM),
        )];
        spans.extend(braille::braille_spans_u64(&display, spark_w, pulsed_color));
        spans.push(Span::styled(
            format!(" {} ", fmt_rate(rate)),
            Style::default().fg(Theme::ROSE),
        ));
        lines.push(Line::from(spans));
    } else {
        lines.push(Line::from(Span::styled(
            format!(" {} waiting for data...", state.atmosphere.spinner()),
            Style::default().fg(Theme::TEXT_DIM),
        )));
    }

    let remaining_rows = inner_height.saturating_sub(lines.len());
    let event_count = snapshot.event_count.max(1) as f64;
    for tier in ["T0", "T1", "T2"].into_iter().take(remaining_rows) {
        let count = snapshot.tier_counts.get(tier).copied().unwrap_or_default();
        let pct = count as f64 / event_count;
        let label = format!(" {:>2} {:<6} ", tier, tier_label(tier));
        let suffix = format!(" {} ({:.0}%)", count, pct * 100.0);
        let bar_w = inner_width
            .saturating_sub(label.len() + suffix.len())
            .max(6);
        let filled = (pct.clamp(0.0, 1.0) * bar_w as f64).round() as usize;
        let empty = bar_w.saturating_sub(filled);
        lines.push(Line::from(vec![
            Span::styled(label, Style::default().fg(tier_color(tier))),
            Span::styled(
                "\u{2588}".repeat(filled.min(bar_w)),
                Style::default().fg(tier_color(tier)),
            ),
            Span::styled(
                "\u{2500}".repeat(empty),
                Style::default().fg(Theme::TEXT_PHANTOM),
            ),
            Span::styled(suffix, Style::default().fg(Theme::BONE_DIM)),
        ]));
    }

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
