//! Minimal ANSI SGR parsing helpers for TUI text rendering.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use regex::Regex;

static ANSI_SGR_RE: OnceLock<Regex> = OnceLock::new();

/// Parse a single line containing ANSI SGR escape sequences into styled spans.
pub fn parse_ansi_line(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut last = 0usize;

    for captures in ansi_sgr_re().captures_iter(line) {
        let Some(full_match) = captures.get(0) else {
            continue;
        };

        if full_match.start() > last {
            spans.push(Span::styled(
                line[last..full_match.start()].to_string(),
                style,
            ));
        }

        apply_sgr_codes(
            captures.get(1).map_or("", |params| params.as_str()),
            &mut style,
        );
        last = full_match.end();
    }

    if last < line.len() {
        spans.push(Span::styled(line[last..].to_string(), style));
    }

    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }

    spans
}

fn ansi_sgr_re() -> &'static Regex {
    ANSI_SGR_RE.get_or_init(|| Regex::new(r"\x1b\[([0-9;]*)m").expect("valid ANSI SGR regex"))
}

fn apply_sgr_codes(params: &str, style: &mut Style) {
    if params.is_empty() {
        *style = Style::default();
        return;
    }

    for code in params.split(';') {
        let Ok(code) = code.parse::<u8>() else {
            continue;
        };

        match code {
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            22 => *style = style.remove_modifier(Modifier::BOLD),
            30..=37 => style.fg = Some(map_ansi_color(code - 30, false)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(map_ansi_color(code - 40, false)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(map_ansi_color(code - 90, true)),
            100..=107 => style.bg = Some(map_ansi_color(code - 100, true)),
            _ => {}
        }
    }
}

fn map_ansi_color(index: u8, bright: bool) -> Color {
    match (index, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::White,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        (7, true) => Color::Gray,
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier};

    use super::parse_ansi_line;

    #[test]
    fn parses_plain_text_without_codes() {
        let spans = parse_ansi_line("plain text");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "plain text");
        assert_eq!(spans[0].style.fg, None);
    }

    #[test]
    fn parses_basic_foreground_color_and_reset() {
        let spans = parse_ansi_line("before \x1b[31mred\x1b[0m after");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "before ");
        assert_eq!(spans[1].content.as_ref(), "red");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[2].content.as_ref(), " after");
        assert_eq!(spans[2].style.fg, None);
    }

    #[test]
    fn parses_combined_bold_foreground_and_background() {
        let spans = parse_ansi_line("\x1b[1;32;44mhi");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "hi");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[0].style.bg, Some(Color::Blue));
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
    }
}
