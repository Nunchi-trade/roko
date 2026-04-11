//! Token burn sparkline widget — multi-row braille sparklines showing
//! cumulative and per-role token usage with rate suffix.
//!
//! Ported from Mori's token_sparkline.rs.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::super::mori_theme::{brighten, MoriTheme};
use super::super::tui_state::TuiState;
use super::braille;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn rate_color(rate: f64) -> ratatui::style::Color {
    if rate > 100_000.0 {
        MoriTheme::EMBER
    } else if rate > 10_000.0 {
        MoriTheme::WARNING
    } else if rate > 0.5 {
        MoriTheme::ROSE
    } else {
        MoriTheme::TEXT_DIM
    }
}

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the token burn sparkline widget.
pub fn render_token_sparkline(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let inner_height = area.height.saturating_sub(2) as usize;
    if inner_width < 10 || inner_height < 2 {
        return;
    }

    // Aggregate cumulative tokens across all roles
    let mut combined: Vec<u64> = Vec::new();
    for history in state.token_history.values() {
        if combined.is_empty() {
            combined = history.iter().copied().collect();
        } else {
            for (i, &val) in history.iter().enumerate() {
                if i < combined.len() {
                    combined[i] = combined[i].saturating_add(val);
                }
            }
        }
    }

    if combined.len() < 2 {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Token Burn")
            .style(MoriTheme::block_style())
            .border_style(Style::default().fg(MoriTheme::TEXT_GHOST))
            .title_style(MoriTheme::title_style());
        let p = Paragraph::new(Line::from(Span::styled(
            format!(" {} waiting for data...", state.atmosphere.spinner()),
            Style::default().fg(MoriTheme::TEXT_DIM),
        )))
        .block(block);
        frame.render_widget(p, area);
        return;
    }

    let total_str = fmt_tokens(state.token_total);
    let rate_str = if state.token_rate > 1_000_000.0 {
        format!("{:.1}M/min", state.token_rate / 1_000_000.0)
    } else if state.token_rate > 1_000.0 {
        format!("{:.1}k/min", state.token_rate / 1_000.0)
    } else if state.token_rate > 0.5 {
        format!("{:.0}/min", state.token_rate)
    } else {
        "idle".to_string()
    };

    let rc = rate_color(state.token_rate);
    let breathing_mod = state.atmosphere.breathing_brightness();
    let pulsed_color = brighten(rc, breathing_mod);

    let mut lines: Vec<Line> = Vec::new();

    // -- Aggregate braille row --
    let label_len = total_str.len() + 2;
    let rate_len = rate_str.len() + 2;
    let spark_w = inner_width.saturating_sub(label_len + rate_len);

    let display: Vec<u64> = if combined.len() > spark_w * 2 {
        combined[combined.len() - spark_w * 2..].to_vec()
    } else {
        combined.clone()
    };

    let mut spans: Vec<Span> = vec![Span::styled(
        format!(" {} ", total_str),
        Style::default().fg(MoriTheme::BONE_DIM),
    )];
    spans.extend(braille::braille_spans_u64(&display, spark_w, pulsed_color));
    spans.push(Span::styled(
        format!(" {} ", rate_str),
        Style::default().fg(rc),
    ));
    lines.push(Line::from(spans));

    // -- Per-role braille rows --
    let mut roles: Vec<&String> = state.token_history.keys().collect();
    roles.sort();
    let remaining_rows = inner_height.saturating_sub(lines.len());

    // Total from last sample for percentage calculation
    let total_last: u64 = state
        .token_history
        .values()
        .filter_map(|h| h.back().copied())
        .sum();

    for role in roles.into_iter().take(remaining_rows) {
        if let Some(history) = state.token_history.get(role) {
            if history.len() < 2 {
                continue;
            }
            let accent = MoriTheme::role_accent(role);
            let label = format!(" {:5} ", &role[..role.len().min(5)]);
            let role_last = history.back().copied().unwrap_or(0);
            let pct = if total_last > 0 {
                (role_last as f64 / total_last as f64 * 100.0).round() as u64
            } else {
                0
            };
            let suffix = format!(" {} ({}%)", fmt_tokens(role_last), pct);
            let agent_spark_w = inner_width
                .saturating_sub(label.len())
                .saturating_sub(suffix.len());

            let values: Vec<u64> = history.iter().copied().collect();
            let mut agent_spans = vec![Span::styled(label, Style::default().fg(accent))];
            agent_spans.extend(braille::braille_spans_u64(&values, agent_spark_w, accent));
            agent_spans.push(Span::styled(
                suffix,
                Style::default().fg(MoriTheme::BONE_DIM),
            ));
            lines.push(Line::from(agent_spans));
        }
    }

    let border_color = if state.token_rate > 0.5 {
        MoriTheme::ROSE_DIM
    } else {
        MoriTheme::TEXT_GHOST
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Token Burn")
        .style(MoriTheme::block_style())
        .border_style(Style::default().fg(border_color))
        .title_style(MoriTheme::title_style());

    let p = Paragraph::new(lines).block(block);
    frame.render_widget(p, area);
}
