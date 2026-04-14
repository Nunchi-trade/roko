//! Error digest widget showing gate failures and recent errors.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use roko_core::dashboard_snapshot::{ErrorEntry, GateVerdict, SnapshotStats};

use super::super::dashboard::Theme;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Format a millisecond timestamp as a short time string.
fn fmt_ts(ts_millis: u64) -> String {
    let secs = ts_millis / 1000;
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

// ---------------------------------------------------------------------------
// Public render entry-point
// ---------------------------------------------------------------------------

/// Render the error digest panel.
///
/// Top half: gate pass/fail summary.
/// Bottom half: recent errors list.
pub fn render_error_digest(
    frame: &mut Frame<'_>,
    area: Rect,
    gates: &[GateVerdict],
    errors: &[ErrorEntry],
    stats: &SnapshotStats,
    theme: &Theme,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.muted())
        .title(Span::styled("Errors & Gates", theme.accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.height < 3 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    // --- Gate summary ---
    render_gate_summary(frame, sections[0], gates, stats, theme);

    // --- Error list ---
    render_error_list(frame, sections[1], errors, theme);
}

/// Render the gate pass/fail ratio header.
fn render_gate_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    gates: &[GateVerdict],
    stats: &SnapshotStats,
    theme: &Theme,
) {
    let total = stats.gates_passed + stats.gates_failed;
    let ratio_text = if total == 0 {
        "No gates evaluated".to_string()
    } else {
        let pct = (stats.gates_passed as f64 / total as f64 * 100.0).round();
        format!("Gates: {}/{} passed ({pct}%)", stats.gates_passed, total)
    };

    let ratio_style = if stats.gates_failed > 0 {
        theme.danger()
    } else if total > 0 {
        theme.success()
    } else {
        theme.muted()
    };

    // Show the last few failed gates inline.
    let recent_failures: Vec<&GateVerdict> =
        gates.iter().rev().filter(|g| !g.passed).take(3).collect();

    let mut lines = vec![Line::from(Span::styled(ratio_text, ratio_style))];

    for gv in &recent_failures {
        lines.push(Line::from(vec![
            Span::styled("\u{2718} ", theme.danger()),
            Span::styled(format!("{}/{} ", gv.plan_id, gv.task_id), theme.text()),
            Span::styled(&gv.gate, theme.muted()),
        ]));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

/// Render the scrollable error list.
fn render_error_list(frame: &mut Frame<'_>, area: Rect, errors: &[ErrorEntry], theme: &Theme) {
    if errors.is_empty() {
        let empty = Paragraph::new("No errors").style(theme.muted());
        frame.render_widget(empty, area);
        return;
    }

    // Show most recent errors first.
    let items: Vec<ListItem<'_>> = errors
        .iter()
        .rev()
        .take(area.height as usize)
        .map(|entry| {
            let ts = fmt_ts(entry.ts_millis);
            let line = Line::from(vec![
                Span::styled(format!("[{ts}] "), theme.muted()),
                Span::styled(&entry.message, theme.danger()),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, area);
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
    fn error_digest_renders_without_panic() {
        let backend = TestBackend::new(80, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();

        let gates = vec![
            GateVerdict {
                plan_id: "p1".into(),
                task_id: "t1".into(),
                gate: "compile".into(),
                passed: true,
                ts_millis: 1_000_000,
            },
            GateVerdict {
                plan_id: "p1".into(),
                task_id: "t2".into(),
                gate: "test".into(),
                passed: false,
                ts_millis: 1_001_000,
            },
        ];

        let errors = vec![
            ErrorEntry {
                message: "compilation failed".into(),
                ts_millis: 1_001_000,
            },
            ErrorEntry {
                message: "test timeout".into(),
                ts_millis: 1_002_000,
            },
        ];

        let stats = SnapshotStats {
            gates_passed: 1,
            gates_failed: 1,
            errors_total: 2,
            ..Default::default()
        };

        terminal
            .draw(|frame| {
                let area = frame.area();
                render_error_digest(frame, area, &gates, &errors, &stats, &theme);
            })
            .unwrap();
    }

    #[test]
    fn error_digest_empty() {
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::dark();
        terminal
            .draw(|frame| {
                let area = frame.area();
                render_error_digest(frame, area, &[], &[], &SnapshotStats::default(), &theme);
            })
            .unwrap();
    }
}
