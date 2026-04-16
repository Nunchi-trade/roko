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

    let codes = params
        .split(';')
        .filter_map(|code| code.parse::<u8>().ok())
        .collect::<Vec<_>>();

    let mut i = 0usize;
    while i < codes.len() {
        match codes[i] {
            38 if codes.get(i + 1) == Some(&5) => {
                if let Some(&index) = codes.get(i + 2) {
                    *style = style.fg(Color::Indexed(index));
                    i += 3;
                    continue;
                }
            }
            48 if codes.get(i + 1) == Some(&5) => {
                if let Some(&index) = codes.get(i + 2) {
                    *style = style.bg(Color::Indexed(index));
                    i += 3;
                    continue;
                }
            }
            38 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    *style = style.fg(Color::Rgb(r, g, b));
                    i += 5;
                    continue;
                }
            }
            48 if codes.get(i + 1) == Some(&2) => {
                if let (Some(&r), Some(&g), Some(&b)) =
                    (codes.get(i + 2), codes.get(i + 3), codes.get(i + 4))
                {
                    *style = style.bg(Color::Rgb(r, g, b));
                    i += 5;
                    continue;
                }
            }
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            22 => *style = style.remove_modifier(Modifier::BOLD),
            30..=37 => style.fg = Some(map_ansi_color(codes[i] - 30, false)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(map_ansi_color(codes[i] - 40, false)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(map_ansi_color(codes[i] - 90, true)),
            100..=107 => style.bg = Some(map_ansi_color(codes[i] - 100, true)),
            _ => {}
        }

        i += 1;
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

    #[test]
    fn parses_indexed_foreground_and_background_colors() {
        let spans = parse_ansi_line("\x1b[38;5;208;48;5;235morange\x1b[0m");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "orange");
        assert_eq!(spans[0].style.fg, Some(Color::Indexed(208)));
        assert_eq!(spans[0].style.bg, Some(Color::Indexed(235)));
    }

    #[test]
    fn parses_rgb_foreground_and_background_colors() {
        let spans = parse_ansi_line("\x1b[38;2;255;136;0;48;2;12;34;56mcolor");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "color");
        assert_eq!(spans[0].style.fg, Some(Color::Rgb(255, 136, 0)));
        assert_eq!(spans[0].style.bg, Some(Color::Rgb(12, 34, 56)));
    }
}
