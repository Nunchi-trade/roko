//! Dream visualization widget for the TUI dashboard.
//!
//! Renders dream state information: current phase (NREM/REM/Integration),
//! replay candidates with utility scores, counterfactual hypotheses,
//! and MAP-Elites archive coverage.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::tui::Theme;

// ---------------------------------------------------------------------------
// Dream state snapshot (populated from dashboard data or sidecar)
// ---------------------------------------------------------------------------

/// Snapshot of dream state for TUI rendering.
#[derive(Debug, Clone, Default)]
pub struct DreamSnapshot {
    /// Current dream phase label.
    pub phase: DreamPhaseLabel,
    /// Replay candidates with their utility scores.
    pub replay_candidates: Vec<ReplayCandidate>,
    /// Counterfactual hypotheses being explored.
    pub hypotheses: Vec<HypothesisEntry>,
    /// MAP-Elites archive coverage (occupied / total cells).
    pub archive_coverage: Option<(usize, usize)>,
    /// Best quality in the MAP-Elites archive.
    pub archive_best_quality: Option<f64>,
    /// Total dream cycles completed.
    pub cycles_completed: usize,
    /// Mean waking improvement from dreams.
    pub mean_waking_improvement: Option<f64>,
}

/// Simplified dream phase label for TUI display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DreamPhaseLabel {
    /// No dream is active.
    #[default]
    Idle,
    /// NREM replay phase.
    NremReplay {
        /// Progress numerator.
        done: usize,
        /// Progress denominator.
        total: usize,
    },
    /// REM imagination / counterfactual phase.
    RemImagination {
        /// Progress numerator.
        done: usize,
        /// Progress denominator.
        total: usize,
    },
    /// Integration / evaluation phase.
    Integration {
        /// Progress numerator.
        done: usize,
        /// Progress denominator.
        total: usize,
    },
    /// Threat rehearsal phase.
    ThreatRehearsal,
}

impl DreamPhaseLabel {
    /// Human-readable name for the phase.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::NremReplay { .. } => "NREM Replay",
            Self::RemImagination { .. } => "REM Imagination",
            Self::Integration { .. } => "Integration",
            Self::ThreatRehearsal => "Threat Rehearsal",
        }
    }

    /// Phase color for the TUI.
    #[must_use]
    pub const fn color(self) -> Color {
        match self {
            Self::Idle => Color::Rgb(120, 120, 140),
            Self::NremReplay { .. } => Color::Rgb(100, 140, 200),
            Self::RemImagination { .. } => Color::Rgb(180, 100, 200),
            Self::Integration { .. } => Color::Rgb(100, 200, 140),
            Self::ThreatRehearsal => Color::Rgb(200, 100, 100),
        }
    }
}

/// A replay candidate with its utility score.
#[derive(Debug, Clone)]
pub struct ReplayCandidate {
    /// Episode identifier.
    pub episode_id: String,
    /// Replay utility score.
    pub utility: f64,
    /// Why this episode was selected.
    pub reason: String,
}

/// A counterfactual hypothesis being explored during REM.
#[derive(Debug, Clone)]
pub struct HypothesisEntry {
    /// Hypothesis identifier.
    pub id: String,
    /// Short description.
    pub summary: String,
    /// Current confidence level.
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the dream visualization widget.
pub fn render_dream_view(
    frame: &mut Frame<'_>,
    area: Rect,
    snapshot: &DreamSnapshot,
    focused: bool,
) {
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
        .title("Dreams")
        .style(Theme::block_style())
        .border_style(border_style)
        .title_style(ttl_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 3 || inner.width < 20 {
        return;
    }

    // Split into: phase header (3 lines), replay candidates, hypotheses
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(40),
        Constraint::Percentage(40),
    ])
    .split(inner);

    render_phase_header(frame, sections[0], snapshot);
    render_replay_candidates(frame, sections[1], snapshot);
    render_hypotheses(frame, sections[2], snapshot);
}

// ---------------------------------------------------------------------------
// Phase header: phase name + progress + archive coverage
// ---------------------------------------------------------------------------

fn render_phase_header(frame: &mut Frame<'_>, area: Rect, snap: &DreamSnapshot) {
    let phase = snap.phase;
    let mut lines: Vec<Line> = Vec::new();

    // Line 1: phase indicator with progress
    let mut spans = vec![
        Span::styled(
            "\u{25cf} ",
            Style::default()
                .fg(phase.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            phase.name(),
            Style::default()
                .fg(phase.color())
                .add_modifier(Modifier::BOLD),
        ),
    ];

    match phase {
        DreamPhaseLabel::NremReplay { done, total }
        | DreamPhaseLabel::RemImagination { done, total }
        | DreamPhaseLabel::Integration { done, total } => {
            spans.push(Span::styled(
                format!("  {done}/{total}"),
                Style::default().fg(Color::Rgb(180, 180, 190)),
            ));
        }
        _ => {}
    }

    lines.push(Line::from(spans));

    // Line 2: cycles + waking improvement
    let mut stats_spans = vec![Span::styled(
        format!("Cycles: {}", snap.cycles_completed),
        Style::default().fg(Color::Rgb(160, 160, 175)),
    )];

    if let Some(imp) = snap.mean_waking_improvement {
        stats_spans.push(Span::styled(
            format!("  Waking +{imp:.1}%"),
            Style::default().fg(Color::Rgb(100, 200, 140)),
        ));
    }

    lines.push(Line::from(stats_spans));

    // Line 3: MAP-Elites archive
    if let Some((occupied, total)) = snap.archive_coverage {
        let mut arch_spans = vec![Span::styled(
            format!("Archive: {occupied}/{total} cells"),
            Style::default().fg(Color::Rgb(160, 160, 175)),
        )];
        if let Some(best) = snap.archive_best_quality {
            arch_spans.push(Span::styled(
                format!("  Best: {best:.3}"),
                Style::default().fg(Color::Rgb(180, 100, 200)),
            ));
        }
        lines.push(Line::from(arch_spans));
    } else {
        lines.push(Line::from(Span::styled(
            "Archive: --",
            Style::default().fg(Color::Rgb(120, 120, 140)),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

// ---------------------------------------------------------------------------
// Replay candidates panel
// ---------------------------------------------------------------------------

fn render_replay_candidates(frame: &mut Frame<'_>, area: Rect, snap: &DreamSnapshot) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title("Replay Candidates")
        .title_style(Style::default().fg(Color::Rgb(100, 140, 200)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snap.replay_candidates.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "No replay candidates",
            Style::default().fg(Color::Rgb(120, 120, 140)),
        )));
        frame.render_widget(msg, inner);
        return;
    }

    let max_lines = inner.height as usize;
    let lines: Vec<Line> = snap
        .replay_candidates
        .iter()
        .take(max_lines)
        .map(|c| {
            let bar_len = ((c.utility * 10.0).round() as usize).min(10);
            let bar: String = "\u{2588}".repeat(bar_len);
            let empty: String = "\u{2591}".repeat(10 - bar_len);
            Line::from(vec![
                Span::styled(
                    format!("{bar}{empty}"),
                    Style::default().fg(Color::Rgb(100, 140, 200)),
                ),
                Span::styled(
                    format!(" {:.2} ", c.utility),
                    Style::default()
                        .fg(Color::Rgb(180, 180, 190))
                        .add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    truncate(&c.episode_id, 20),
                    Style::default().fg(Color::Rgb(200, 200, 210)),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

// ---------------------------------------------------------------------------
// Hypotheses panel
// ---------------------------------------------------------------------------

fn render_hypotheses(frame: &mut Frame<'_>, area: Rect, snap: &DreamSnapshot) {
    let block = Block::default()
        .borders(Borders::TOP)
        .title("Hypotheses")
        .title_style(Style::default().fg(Color::Rgb(180, 100, 200)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snap.hypotheses.is_empty() {
        let msg = Paragraph::new(Line::from(Span::styled(
            "No active hypotheses",
            Style::default().fg(Color::Rgb(120, 120, 140)),
        )));
        frame.render_widget(msg, inner);
        return;
    }

    let max_lines = inner.height as usize;
    let lines: Vec<Line> = snap
        .hypotheses
        .iter()
        .take(max_lines)
        .map(|h| {
            let conf_color = if h.confidence > 0.7 {
                Color::Rgb(100, 200, 140)
            } else if h.confidence > 0.4 {
                Color::Rgb(200, 180, 100)
            } else {
                Color::Rgb(200, 100, 100)
            };
            Line::from(vec![
                Span::styled(
                    format!("[{:.0}%] ", h.confidence * 100.0),
                    Style::default().fg(conf_color),
                ),
                Span::styled(
                    truncate(&h.summary, (inner.width as usize).saturating_sub(8)),
                    Style::default().fg(Color::Rgb(200, 200, 210)),
                ),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
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

    fn test_snapshot() -> DreamSnapshot {
        DreamSnapshot {
            phase: DreamPhaseLabel::NremReplay { done: 5, total: 12 },
            replay_candidates: vec![
                ReplayCandidate {
                    episode_id: "ep-001".into(),
                    utility: 0.85,
                    reason: "high surprise".into(),
                },
                ReplayCandidate {
                    episode_id: "ep-002".into(),
                    utility: 0.42,
                    reason: "recent failure".into(),
                },
            ],
            hypotheses: vec![
                HypothesisEntry {
                    id: "h-1".into(),
                    summary: "Caching reduces latency by 30%".into(),
                    confidence: 0.78,
                },
                HypothesisEntry {
                    id: "h-2".into(),
                    summary: "Retry storms cause cascade".into(),
                    confidence: 0.35,
                },
            ],
            archive_coverage: Some((42, 100)),
            archive_best_quality: Some(0.95),
            cycles_completed: 7,
            mean_waking_improvement: Some(12.5),
        }
    }

    #[test]
    fn dream_view_renders_without_panic() {
        let snap = test_snapshot();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_dream_view(frame, area, &snap, false);
            })
            .unwrap();
    }

    #[test]
    fn dream_view_idle_renders() {
        let snap = DreamSnapshot::default();
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_dream_view(frame, area, &snap, true);
            })
            .unwrap();
    }

    #[test]
    fn dream_view_small_area() {
        let snap = test_snapshot();
        let backend = TestBackend::new(15, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_dream_view(frame, area, &snap, false);
            })
            .unwrap();
    }
}
