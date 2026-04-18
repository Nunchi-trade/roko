//! Compact 2-line phase widget with segmented phase bar.
//!
//! Line 1: Segmented 8-phase bar — each phase gets a fixed-width cell,
//!         colored by status (Done=SAGE, Active=WARNING+spinner, Pending=dashes).
//! Line 2: Active phase detail (icon + name + pct + elapsed + ETA) or error state.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::super::atmosphere::Atmosphere;
use super::super::state::{PhaseStatus, TuiState};
use crate::tui::Theme;

// ---------------------------------------------------------------------------
// Phase labels
// ---------------------------------------------------------------------------

// Phase labels reserved for future abbreviated display.
// The segmented bar currently uses phase_pipeline names directly.

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render a compact 2-line phase widget.
///
/// ```text
/// ┌ Phase ──────────────────────────────────┐
/// │ ████████████████████░░░░░░────────────── │
/// │ ✧ implementer  42%  2m31s  ETA ~4m      │
/// └─────────────────────────────────────────┘
/// ```
pub fn render_phase_compact(frame: &mut Frame<'_>, area: Rect, state: &TuiState, focused: bool) {
    let atm = &state.atmosphere;

    // Find active phase index
    let active_idx = state
        .phase_pipeline
        .iter()
        .position(|s| s.status == PhaseStatus::Active);

    let title = if let Some(idx) = active_idx {
        format!("Phase \u{00b7} {}", state.phase_pipeline[idx].name)
    } else {
        "Phase".to_string()
    };

    let (border_style, ttl_style) = if focused {
        (Theme::focused_border_style(), Theme::focused_title_style())
    } else {
        (
            Theme::unfocused_border_style(),
            Theme::unfocused_title_style(),
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(Theme::block_style())
        .border_style(border_style)
        .title_style(ttl_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 1 || inner.width < 10 {
        return;
    }

    let inner_w = inner.width as usize;

    // ── Line 1: Segmented phase bar ──────────────────────────────────────
    let total_phases = state.phase_pipeline.len().max(1);
    let seg_width = inner_w / total_phases;
    let leftover = inner_w.saturating_sub(seg_width * total_phases);

    let spinner_ch = atm.spinner_ethereal();
    let mut bar_spans: Vec<Span> = Vec::new();

    for (i, step) in state.phase_pipeline.iter().enumerate() {
        let w = if i == total_phases - 1 {
            seg_width + leftover
        } else {
            seg_width
        };

        match step.status {
            PhaseStatus::Done => {
                let fill: String = "\u{2588}".repeat(w);
                bar_spans.push(Span::styled(fill, Style::default().fg(Theme::SAGE)));
            }
            PhaseStatus::Active => {
                if w > 0 {
                    let fill_count = w.saturating_sub(1);
                    let fill: String = "\u{2588}".repeat(fill_count);
                    bar_spans.push(Span::styled(fill, Style::default().fg(Theme::WARNING)));
                    bar_spans.push(Span::styled(
                        spinner_ch.to_string(),
                        Style::default()
                            .fg(Theme::WARNING)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }
            PhaseStatus::Failed => {
                let fill: String = "\u{2588}".repeat(w);
                bar_spans.push(Span::styled(fill, Style::default().fg(Theme::EMBER)));
            }
            PhaseStatus::Pending => {
                let fill: String = "\u{2500}".repeat(w);
                bar_spans.push(Span::styled(fill, Style::default().fg(Theme::TEXT_GHOST)));
            }
        }
    }

    let bar_line = Line::from(bar_spans);
    frame.render_widget(
        Paragraph::new(bar_line),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );

    if inner.height < 2 {
        return;
    }

    // ── Line 2: Active phase detail ──────────────────────────────────────
    let detail_line = if let Some(idx) = state
        .phase_pipeline
        .iter()
        .position(|s| s.status == PhaseStatus::Failed)
    {
        let name = &state.phase_pipeline[idx].name;
        Line::from(vec![
            Span::styled(
                "HALTED ",
                Style::default()
                    .fg(Theme::EMBER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("at {name}"), Style::default().fg(Theme::EMBER)),
        ])
    } else if let Some(idx) = active_idx {
        build_active_detail(&state.phase_pipeline[idx], atm)
    } else {
        // All done or all pending
        let all_done = state
            .phase_pipeline
            .iter()
            .all(|s| s.status == PhaseStatus::Done);
        if all_done && !state.phase_pipeline.is_empty() {
            Line::from(Span::styled(
                "all phases complete",
                Style::default().fg(Theme::SAGE),
            ))
        } else {
            Line::from(Span::styled(
                "waiting...",
                Style::default().fg(Theme::TEXT_DIM),
            ))
        }
    };

    frame.render_widget(
        Paragraph::new(detail_line),
        Rect::new(inner.x, inner.y + 1, inner.width, 1),
    );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the active-phase detail line: icon + name + pct + elapsed + ETA.
fn build_active_detail(step: &super::super::state::PhaseStep, atm: &Atmosphere) -> Line<'static> {
    let pulse_color = pulse_active(atm.heartbeat());
    let icon = atm.spinner_ethereal().to_string();

    let mut spans = vec![
        Span::styled(
            icon,
            Style::default()
                .fg(pulse_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", step.name),
            Style::default()
                .fg(Theme::ROSE)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    // Percentage
    if step.pct > 0.0 {
        spans.push(Span::styled(
            format!(" {:.0}%", step.pct.min(99.0)),
            Style::default().fg(Theme::DREAM),
        ));
    }

    // Elapsed time
    if step.elapsed_secs > 0.0 {
        let secs = step.elapsed_secs as u64;
        let time_str = format!(" {}m{:02}s", secs / 60, secs % 60);
        let time_style = {
            let pulse = atm.heartbeat();
            let base_r = 170.0_f64;
            let r = (base_r * pulse).clamp(0.0, 255.0) as u8;
            Style::default().fg(Color::Rgb(r, 112, 136))
        };
        spans.push(Span::styled(time_str, time_style));
    }

    Line::from(spans)
}

/// Modulate a base rose color with heartbeat pulse.
fn pulse_active(heartbeat: f64) -> Color {
    let base_r = 170.0;
    let base_g = 112.0;
    let base_b = 136.0;
    let scale = heartbeat.clamp(0.9, 1.1);
    Color::Rgb(
        (base_r * scale).min(255.0) as u8,
        (base_g * scale).min(255.0) as u8,
        (base_b * scale).min(255.0) as u8,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::super::super::state::{PhaseStatus, PhaseStep, TuiState};
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_state(steps: Vec<PhaseStep>) -> TuiState {
        use super::super::super::dashboard::DashboardData;
        let data = DashboardData::default();
        let mut state = TuiState::from_dashboard_data(&data);
        state.phase_pipeline = steps;
        state
    }

    #[test]
    fn phase_compact_renders_without_panic() {
        let steps = vec![
            PhaseStep {
                name: "preflight".into(),
                status: PhaseStatus::Done,
                elapsed_secs: 3.0,
                pct: 100.0,
            },
            PhaseStep {
                name: "implementer".into(),
                status: PhaseStatus::Active,
                elapsed_secs: 45.0,
                pct: 42.0,
            },
            PhaseStep {
                name: "reviewing".into(),
                status: PhaseStatus::Pending,
                elapsed_secs: 0.0,
                pct: 0.0,
            },
        ];
        let state = make_state(steps);
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_compact(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn phase_compact_all_done() {
        let steps = vec![
            PhaseStep {
                name: "preflight".into(),
                status: PhaseStatus::Done,
                elapsed_secs: 3.0,
                pct: 100.0,
            },
            PhaseStep {
                name: "implementer".into(),
                status: PhaseStatus::Done,
                elapsed_secs: 60.0,
                pct: 100.0,
            },
        ];
        let state = make_state(steps);
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_compact(frame, area, &state, true);
            })
            .unwrap();
    }

    #[test]
    fn phase_compact_empty_pipeline() {
        let state = make_state(Vec::new());
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_compact(frame, area, &state, false);
            })
            .unwrap();
    }

    #[test]
    fn phase_compact_failed_state() {
        let steps = vec![
            PhaseStep {
                name: "preflight".into(),
                status: PhaseStatus::Done,
                elapsed_secs: 3.0,
                pct: 100.0,
            },
            PhaseStep {
                name: "compile-gate".into(),
                status: PhaseStatus::Failed,
                elapsed_secs: 12.0,
                pct: 80.0,
            },
            PhaseStep {
                name: "reviewing".into(),
                status: PhaseStatus::Pending,
                elapsed_secs: 0.0,
                pct: 0.0,
            },
        ];
        let state = make_state(steps);
        let backend = TestBackend::new(60, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_phase_compact(frame, area, &state, false);
            })
            .unwrap();
    }
}
