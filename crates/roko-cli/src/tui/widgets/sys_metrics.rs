//! System metrics widget — CPU/MEM/NET/DSK gauges with braille sparklines.
//!
//! Ported from Mori's sys_metrics.rs.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::mori_theme::MoriTheme;
use super::super::tui_state::TuiState;
use super::braille;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fmt_bytes(b: u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    if b >= GIB {
        format!("{:.1}G", b as f64 / GIB as f64)
    } else if b >= MIB {
        format!("{:.0}M", b as f64 / MIB as f64)
    } else {
        format!("{}K", b / 1024)
    }
}

fn fmt_rate(bps: f64) -> String {
    const GIB: f64 = (1u64 << 30) as f64;
    const MIB: f64 = (1u64 << 20) as f64;
    const KIB: f64 = 1024.0;
    if bps >= GIB {
        format!("{:.1}G", bps / GIB)
    } else if bps >= MIB {
        format!("{:.1}M", bps / MIB)
    } else if bps >= KIB {
        format!("{:.1}K", bps / KIB)
    } else if bps > 0.5 {
        format!("{:.0}B", bps)
    } else {
        "0B".to_string()
    }
}

fn pct_color(pct: f64) -> Color {
    if pct >= 0.8 {
        MoriTheme::EMBER
    } else if pct >= 0.5 {
        MoriTheme::WARNING
    } else {
        MoriTheme::SAGE
    }
}

/// Render a mini inline gauge with solid blocks and breathing shimmer.
fn render_mini_gauge(
    width: usize,
    fill_pct: f64,
    color: Color,
    breathing: f64,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let pct = fill_pct.clamp(0.0, 1.0);
    let filled = (pct * width as f64) as usize;
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::new();

    // Filled: solid blocks with per-cell shimmer
    if filled > 0 {
        if let Color::Rgb(r, g, b) = color {
            for i in 0..filled {
                let cell_t = if filled > 1 {
                    i as f64 / (filled - 1) as f64
                } else {
                    1.0
                };
                let shimmer = 1.0 + (cell_t * 6.28 + breathing * 3.0).sin() * 0.08;
                let br = (r as f64 * breathing * shimmer).min(255.0) as u8;
                let bg = (g as f64 * breathing * shimmer).min(255.0) as u8;
                let bb = (b as f64 * breathing * shimmer).min(255.0) as u8;
                spans.push(Span::styled(
                    "\u{2588}",
                    Style::default().fg(Color::Rgb(br, bg, bb)),
                ));
            }
        } else {
            spans.push(Span::styled(
                "\u{2588}".repeat(filled),
                Style::default().fg(color),
            ));
        }
    }

    // Empty: dim track
    if empty > 0 {
        spans.push(Span::styled(
            "\u{2500}".repeat(empty),
            Style::default().fg(MoriTheme::TEXT_GHOST),
        ));
    }

    spans
}

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

/// Render the system metrics widget.
pub fn render_sys_metrics(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("System")
        .style(MoriTheme::block_style())
        .border_style(Style::default().fg(MoriTheme::TEXT_GHOST))
        .title_style(MoriTheme::title_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 12 || inner.height < 2 {
        return;
    }

    let w = inner.width as usize;
    let gauge_w = 10usize.min(w.saturating_sub(18));
    let spark_w = w.saturating_sub(12 + gauge_w + 1) / 2;
    let breathing = state.atmosphere.breathing_brightness();

    let mut lines = Vec::new();

    // CPU
    {
        let cpu_pct = state.sys.cpu_pct as f64;
        let val = format!("{:>5.1}%", cpu_pct);
        let pct = cpu_pct / 100.0;
        let color = pct_color(pct);
        let data: Vec<f32> = state.sys.cpu_history.iter().copied().collect();
        let mut spans = vec![
            Span::styled("CPU ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(val, Style::default().fg(color)),
            Span::styled(" ", Style::default()),
        ];
        spans.extend(render_mini_gauge(gauge_w, pct, color, breathing));
        spans.push(Span::styled(" ", Style::default()));
        spans.extend(braille::braille_spans_f32(&data, 100.0, spark_w, color));
        lines.push(Line::from(spans));
    }

    // MEM
    if inner.height >= 2 {
        let mem_frac = if state.sys.mem_total_bytes > 0 {
            state.sys.mem_used_bytes as f64 / state.sys.mem_total_bytes as f64
        } else {
            0.0
        };
        let val = format!("{:>6}", fmt_bytes(state.sys.mem_used_bytes));
        let color = pct_color(mem_frac);
        let data: Vec<f32> = state.sys.mem_history.iter().copied().collect();
        let mut spans = vec![
            Span::styled("MEM ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(val, Style::default().fg(color)),
            Span::styled(" ", Style::default()),
        ];
        spans.extend(render_mini_gauge(gauge_w, mem_frac, color, breathing));
        spans.push(Span::styled(" ", Style::default()));
        spans.extend(braille::braille_spans_f32(&data, 1.0, spark_w, color));
        lines.push(Line::from(spans));
    }

    // NET
    if inner.height >= 3 {
        let down = state.sys.net_down_bytes_sec;
        let val = format!("\u{2193}{:>4}", fmt_rate(down as f64));
        let spans = vec![
            Span::styled("NET ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(val, Style::default().fg(MoriTheme::DREAM)),
        ];
        lines.push(Line::from(spans));
    }

    // DSK
    if inner.height >= 4 {
        let read = state.sys.disk_read_bytes_sec;
        let val = format!("R{:>4}", fmt_rate(read as f64));
        let spans = vec![
            Span::styled("DSK ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(val, Style::default().fg(MoriTheme::BONE_DIM)),
        ];
        lines.push(Line::from(spans));
    }

    // FPS
    if inner.height >= 5 {
        let fps = state.atmosphere.fps();
        let fps_color = if fps >= 50.0 {
            MoriTheme::SAGE
        } else if fps >= 25.0 {
            MoriTheme::WARNING
        } else {
            MoriTheme::EMBER
        };
        lines.push(Line::from(vec![
            Span::styled("FPS ", Style::default().fg(MoriTheme::TEXT_DIM)),
            Span::styled(format!("{:>5.1}", fps), Style::default().fg(fps_color)),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}
