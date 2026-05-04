//! Styled line builders that compose symbols + theme into ratatui `Line`s.
//!
//! All functions return `Line<'static>` with owned `String` content, so they
//! can be stored, pushed into scrollback, or rendered at any later time
//! without lifetime concerns.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::symbols;

// ---------------------------------------------------------------------------
// Line builders — all return Line<'static> with owned data
// ---------------------------------------------------------------------------

/// Section header: `◆ label  value · detail`
pub fn section_start(
    theme: &Theme,
    label: &str,
    value: &str,
    detail: Option<&str>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(symbols::START.to_string(), theme.accent()),
        Span::raw(" "),
        Span::styled(
            label.to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(value.to_string(), theme.text()),
    ];
    if let Some(d) = detail {
        spans.push(Span::styled(format!("  {}  ", symbols::SEP), theme.muted()));
        spans.push(Span::styled(d.to_string(), theme.muted()));
    }
    Line::from(spans)
}

/// Continuation line: `│ label  value · detail`
pub fn continuation(
    theme: &Theme,
    label: &str,
    value: &str,
    detail: Option<&str>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(format!("{label:<10}"), Style::default().fg(Theme::TEXT_DIM)),
        Span::styled(value.to_string(), theme.text()),
    ];
    if let Some(d) = detail {
        spans.push(Span::styled(format!("  {}  ", symbols::SEP), theme.muted()));
        spans.push(Span::styled(d.to_string(), theme.muted()));
    }
    Line::from(spans)
}

/// Last item: `└ label  value`
pub fn section_end(theme: &Theme, label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::END.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(format!("{label:<10}"), Style::default().fg(Theme::TEXT_DIM)),
        Span::styled(value.to_string(), theme.text()),
    ])
}

/// A gate verdict item: `gate_name ✔` or `gate_name ✖`
pub fn gate_verdict(theme: &Theme, name: &str, passed: bool) -> Vec<Span<'static>> {
    let symbol = if passed { symbols::PASS } else { symbols::FAIL };
    let style = if passed {
        theme.success()
    } else {
        theme.danger()
    };
    vec![
        Span::styled(name.to_string(), theme.text()),
        Span::raw(" "),
        Span::styled(symbol.to_string(), style),
    ]
}

/// Gates summary line: `│ gates    compile ✔  test ✔  clippy ✖`
pub fn gates_line(theme: &Theme, verdicts: &[(String, bool)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(
            format!("{:<10}", "gates"),
            Style::default().fg(Theme::TEXT_DIM),
        ),
    ];
    for (i, (name, passed)) in verdicts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("   "));
        }
        spans.extend(gate_verdict(theme, name, *passed));
    }
    Line::from(spans)
}

/// Cost comparison: `$0.031 (-28% vs predicted)`
pub fn cost_delta(actual: f64, predicted: f64) -> String {
    if predicted <= 0.0 {
        return format!("${:.3}", actual.max(0.0));
    }
    let pct = ((actual - predicted) / predicted * 100.0).round() as i64;
    let sign = if pct <= 0 { "" } else { "+" };
    format!("${:.3}  ({sign}{pct}% vs predicted)", actual.max(0.0))
}

/// Collapsed tool call: `│ ▸ ToolName  summary  (duration)`
pub fn tool_call_collapsed(
    theme: &Theme,
    tool_name: &str,
    summary: &str,
    duration_s: f64,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(symbols::COLLAPSED.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(tool_name.to_string(), theme.info()),
        Span::raw("  "),
        Span::styled(summary.to_string(), theme.muted()),
        Span::styled(
            format!("  ({duration_s:.1}s)"),
            Style::default().fg(Theme::TEXT_GHOST),
        ),
    ])
}

/// Spinner line: `│ ⠋ message... (elapsed)`
pub fn spinner_line(theme: &Theme, tick: u64, message: &str, elapsed_s: f64) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(symbols::spinner_frame(tick).to_string(), theme.accent()),
        Span::raw(" "),
        Span::styled(message.to_string(), theme.text()),
        Span::styled(
            format!(" ({elapsed_s:.1}s)"),
            Style::default().fg(Theme::TEXT_GHOST),
        ),
    ])
}

/// Status bar: `$0.0310  ·  4821 in / 1203 out  ·  haiku  ·  ━━━━━━░░ 62%`
pub fn status_bar(
    theme: &Theme,
    cost_usd: f64,
    input_tokens: u64,
    output_tokens: u64,
    model: &str,
    progress: Option<f64>,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("${:.4}", cost_usd.max(0.0)),
            Style::default().fg(Theme::SAGE),
        ),
        Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
        Span::styled(
            format!("{input_tokens} in / {output_tokens} out"),
            Style::default().fg(Theme::TEXT_DIM),
        ),
        Span::styled(format!("  {}  ", symbols::SEP), theme.muted()),
        Span::styled(model.to_string(), theme.info()),
    ];
    if let Some(p) = progress {
        let bar = symbols::progress_bar(p, 10);
        let pct = (p * 100.0).round() as u32;
        spans.push(Span::styled(format!("  {}  ", symbols::SEP), theme.muted()));
        spans.push(Span::styled(bar, theme.accent()));
        spans.push(Span::styled(
            format!(" {pct}%"),
            Style::default().fg(Theme::TEXT_DIM),
        ));
    }
    Line::from(spans)
}

/// Expanded tool call header: `│ ▾ ToolName  summary  (duration)`
pub fn tool_call_expanded_header(
    theme: &Theme,
    tool_name: &str,
    summary: &str,
    duration_s: f64,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(symbols::EXPANDED.to_string(), theme.muted()),
        Span::raw(" "),
        Span::styled(tool_name.to_string(), theme.info()),
        Span::raw("  "),
        Span::styled(summary.to_string(), theme.muted()),
        Span::styled(
            format!("  ({duration_s:.1}s)"),
            Style::default().fg(Theme::TEXT_GHOST),
        ),
    ])
}

/// Indented line inside a tool call or block: `│   content`
pub fn indented_line(theme: &Theme, text: &str, indent: usize) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw(" ".repeat(indent + 1)),
        Span::styled(text.to_string(), Style::default().fg(Theme::TEXT_DIM)),
    ])
}

/// Empty bar line (just the vertical connector).
pub fn bar_empty(theme: &Theme) -> Line<'static> {
    Line::from(vec![Span::styled(symbols::BAR.to_string(), theme.muted())])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_start_renders() {
        let theme = Theme::dark();
        let line = section_start(&theme, "agent", "auditor@v1", Some("attested"));
        assert!(line.spans.len() >= 5);
    }

    #[test]
    fn cost_delta_negative() {
        let s = cost_delta(0.031, 0.043);
        assert!(s.contains('-'));
        assert!(s.contains("predicted"));
    }

    #[test]
    fn cost_delta_zero_predicted() {
        let s = cost_delta(0.031, 0.0);
        assert!(s.starts_with('$'));
        assert!(!s.contains("predicted"));
    }

    #[test]
    fn gates_line_renders() {
        let theme = Theme::dark();
        let verdicts = vec![("compile".to_string(), true), ("test".to_string(), false)];
        let line = gates_line(&theme, &verdicts);
        assert!(line.spans.len() > 3);
    }
}
