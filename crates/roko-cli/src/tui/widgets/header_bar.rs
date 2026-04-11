//! Top header bar — ported from Mori.
//!
//! 8 sections: heartbeat + name, wave indicator, progress bar with fire
//! gradient, plan count with semantic coloring, ETA/elapsed/cost/tokens,
//! system metrics (CPU/MEM), active agent spinner with role label, F-key strip.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::mori_theme::{gradient_fire, MoriTheme};
use super::super::tui_state::TuiState;

const HEARTBEAT_FRAMES: [&str; 4] = ["\u{00b7}", "\u{00b0}", "\u{2219}", "\u{25cf}"];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn sep() -> Span<'static> {
    Span::styled(
        "\u{2502}",
        Style::default()
            .fg(MoriTheme::TEXT_PHANTOM)
            .bg(MoriTheme::BG_SECONDARY),
    )
}

fn hdr_pct_color(pct: f64) -> Color {
    if pct >= 0.8 {
        MoriTheme::EMBER
    } else if pct >= 0.5 {
        MoriTheme::WARNING
    } else {
        MoriTheme::SAGE
    }
}

fn hdr_fmt_bytes(b: u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    if b >= GIB {
        format!("{:.0}G", b as f64 / GIB as f64)
    } else if b >= MIB {
        format!("{:.0}M", b as f64 / MIB as f64)
    } else {
        format!("{}K", b / 1024)
    }
}

fn format_elapsed(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

fn shorten_model(slug: &str) -> String {
    slug.replace("claude-", "")
        .replace("sonnet-", "s")
        .replace("opus-", "o")
        .replace("haiku-", "h")
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the header bar with all 8 sections.
pub fn render_header_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &TuiState,
) {
    let bg = Style::default().bg(MoriTheme::BG_SECONDARY);
    let compact = area.width < 120;

    let (done, total) = state.task_counts();
    let elapsed_secs = state.elapsed_secs() as u64;
    let elapsed_str = format_elapsed(elapsed_secs);

    let mut spans: Vec<Span<'static>> = vec![Span::styled(" ", bg)];

    // ── 1. Heartbeat dot ──────────────────────────────────────────────
    let hb_idx = (state.atmosphere.frame() / 8) as usize % HEARTBEAT_FRAMES.len();
    let hb_brightness = state.atmosphere.heartbeat();
    let hb_r = (170.0 * hb_brightness).min(255.0) as u8;
    let hb_g = (112.0 * hb_brightness * 0.8).min(255.0) as u8;
    let hb_b = (136.0 * hb_brightness).min(255.0) as u8;
    spans.push(Span::styled(
        HEARTBEAT_FRAMES[hb_idx],
        Style::default()
            .fg(Color::Rgb(hb_r, hb_g, hb_b))
            .bg(MoriTheme::BG_SECONDARY),
    ));

    // App name
    spans.push(Span::styled(
        " roko",
        Style::default()
            .fg(MoriTheme::ROSE)
            .bg(MoriTheme::BG_SECONDARY)
            .add_modifier(Modifier::BOLD),
    ));

    // ── 2. Wave indicator ─────────────────────────────────────────────
    if !state.execution_waves.is_empty() {
        let total_waves = state.wave_count();
        let wave_idx = state.current_wave() + 1;
        spans.push(Span::styled(
            format!("  Wave {wave_idx}/{total_waves}"),
            Style::default()
                .fg(MoriTheme::BONE)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    spans.push(sep());

    // ── 3. Progress bar with fire gradient ────────────────────────────
    let bar_width = 15usize;
    if total > 0 {
        let fraction = done as f64 / total.max(1) as f64;
        let filled = (fraction * bar_width as f64) as usize;
        let empty = bar_width.saturating_sub(filled);
        let bar_color = gradient_fire().sample(fraction);

        spans.push(Span::styled("  ", bg));
        if filled > 0 {
            spans.push(Span::styled(
                "\u{2588}".repeat(filled),
                Style::default()
                    .fg(bar_color)
                    .bg(MoriTheme::BG_SECONDARY),
            ));
        }
        if empty > 0 {
            spans.push(Span::styled(
                "\u{2500}".repeat(empty),
                Style::default()
                    .fg(MoriTheme::TEXT_PHANTOM)
                    .bg(MoriTheme::BG_SECONDARY),
            ));
        }
    }

    // ── 4. Plan count with semantic coloring ──────────────────────────
    let fill_pct = if total > 0 {
        done as f64 / total as f64
    } else {
        0.0
    };
    let all_done = state.plans.iter().all(|p| !p.active);
    let has_failures = state.plans.iter().any(|p| p.tasks_failed > 0);

    let progress_text = if all_done && total > 0 && !has_failures {
        " COMPLETE".to_string()
    } else if has_failures {
        format!(" ERR:{done}/{total}")
    } else {
        format!("  {done}/{total}")
    };
    let progress_style = if has_failures {
        Style::default()
            .fg(MoriTheme::EMBER)
            .add_modifier(Modifier::BOLD)
    } else if all_done && total > 0 {
        Style::default()
            .fg(MoriTheme::SAGE)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MoriTheme::semantic_color(fill_pct))
    };
    spans.push(Span::styled(
        progress_text,
        progress_style.bg(MoriTheme::BG_SECONDARY),
    ));

    // Percentage (hidden when compact)
    if !compact && total > 0 && !(all_done && !has_failures) {
        let pct = (fill_pct * 100.0) as u32;
        spans.push(Span::styled(
            format!("  {pct}%"),
            Style::default()
                .fg(MoriTheme::semantic_color(fill_pct))
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    // In-flight agent count
    let in_flight = state.active_agent_count();
    if in_flight > 0 && !(all_done && !has_failures) {
        spans.push(Span::styled(
            format!("  {in_flight}\u{25b8}"),
            Style::default()
                .fg(MoriTheme::ROSE_DIM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    spans.push(sep());

    // ── 5. ETA / elapsed / cost / tokens ──────────────────────────────
    // ETA estimate (simple: remaining proportional to elapsed)
    if total > 0 && done < total && done > 0 {
        let rate = elapsed_secs as f64 / done as f64;
        let remaining = ((total - done) as f64 * rate) as u64;
        let eta_str = format_elapsed(remaining.max(1));
        spans.push(Span::styled(
            format!("  ETA:{eta_str}"),
            Style::default()
                .fg(MoriTheme::DREAM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    // Elapsed
    spans.push(Span::styled(
        format!("  {elapsed_str}"),
        Style::default()
            .fg(MoriTheme::FG_DIM)
            .bg(MoriTheme::BG_SECONDARY),
    ));

    // Cost
    if state.cost_dollars > 0.001 {
        let cost_str = if state.cost_dollars >= 1.0 {
            format!("${:.2}", state.cost_dollars)
        } else {
            format!("${:.3}", state.cost_dollars)
        };
        spans.push(Span::styled(
            format!("  {cost_str}"),
            Style::default()
                .fg(MoriTheme::BONE_DIM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    // Tokens
    if state.token_total > 0 {
        let tok_display = if state.token_total >= 1_000_000 {
            format!("  {}M tok", state.token_total / 1_000_000)
        } else if state.token_total >= 1_000 {
            format!("  {}K tok", state.token_total / 1_000)
        } else {
            format!("  {} tok", state.token_total)
        };
        spans.push(Span::styled(
            tok_display,
            Style::default()
                .fg(MoriTheme::FG_DIM)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    spans.push(sep());

    // ── 6. System metrics (CPU/MEM) ───────────────────────────────────
    {
        let colon = if compact { "" } else { ":" };

        let cpu_frac = state.sys.cpu_pct as f64 / 100.0;
        spans.push(Span::styled(
            format!(" C{}{:.0}%", colon, state.sys.cpu_pct),
            Style::default()
                .fg(hdr_pct_color(cpu_frac))
                .bg(MoriTheme::BG_SECONDARY),
        ));

        let mem_frac = if state.sys.mem_total_bytes > 0 {
            state.sys.mem_used_bytes as f64 / state.sys.mem_total_bytes as f64
        } else {
            0.0
        };
        spans.push(Span::styled(
            format!(" M{}{}", colon, hdr_fmt_bytes(state.sys.mem_used_bytes)),
            Style::default()
                .fg(hdr_pct_color(mem_frac))
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    spans.push(sep());

    // ── 7. Active agent spinner with role label ───────────────────────
    if let Some(agent) = state.agents.iter().find(|a| a.active) {
        let short = shorten_model(&agent.model);
        let role_color = MoriTheme::role_accent(&agent.role);
        spans.push(Span::styled(
            format!(
                "  {} {}({})",
                state.atmosphere.spinner(),
                agent.role,
                short
            ),
            Style::default()
                .fg(role_color)
                .bg(MoriTheme::BG_SECONDARY),
        ));
    }

    // ── 8. F-key strip (right-aligned) ────────────────────────────────
    use super::super::tui_state::Tab;

    let fkey_items: Vec<(&str, Color, &str, Tab)> = vec![
        (" F1", MoriTheme::ROSE, "dash", Tab::Dashboard),
        (" F2", MoriTheme::BONE_DIM, "plans", Tab::Plans),
        (" F3", MoriTheme::SAGE, "agents", Tab::Agents),
        (" F4", MoriTheme::DREAM, "logs", Tab::Logs),
        (" F5", MoriTheme::DREAM, "sigs", Tab::Signals),
        (" F6", MoriTheme::BONE_DIM, "cfg", Tab::Config),
    ];

    let fkey_width: u16 = fkey_items
        .iter()
        .map(|(k, _, l, _)| k.len() + 1 + l.len())
        .sum::<usize>() as u16
        + 1; // trailing space

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(fkey_width)])
        .split(area);

    // Render left content
    let left_line = Line::from(spans);
    frame.render_widget(Paragraph::new(left_line).style(bg), chunks[0]);

    // Render F-key indicators with active tab highlighting
    let current_tab = state.active_tab;
    let mut fkey_spans: Vec<Span<'static>> = Vec::new();
    for (key, color, label, tab) in &fkey_items {
        let is_active = *tab == current_tab;
        if is_active {
            fkey_spans.push(Span::styled(
                format!("{key}:{label}"),
                Style::default()
                    .fg(MoriTheme::VOID)
                    .bg(*color)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            fkey_spans.push(Span::styled(
                key.to_string(),
                Style::default()
                    .fg(*color)
                    .bg(MoriTheme::BG_SECONDARY)
                    .add_modifier(Modifier::BOLD),
            ));
            fkey_spans.push(Span::styled(
                format!(":{label}"),
                Style::default()
                    .fg(MoriTheme::FG_DIM)
                    .bg(MoriTheme::BG_SECONDARY),
            ));
        }
    }
    fkey_spans.push(Span::styled(" ", bg));

    let fkey_line = Line::from(fkey_spans);
    frame.render_widget(Paragraph::new(fkey_line).style(bg), chunks[1]);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::super::super::tui_state::TuiState;
    use super::super::super::dashboard::DashboardData;

    #[test]
    fn header_bar_renders_without_panic() {
        let data = DashboardData::default();
        let state = TuiState::from_dashboard_data(&data);
        let backend = TestBackend::new(140, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_header_bar(frame, area, &state);
            })
            .unwrap();
    }

    #[test]
    fn header_bar_compact() {
        let data = DashboardData::default();
        let state = TuiState::from_dashboard_data(&data);
        let backend = TestBackend::new(80, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_header_bar(frame, area, &state);
            })
            .unwrap();
    }
}
