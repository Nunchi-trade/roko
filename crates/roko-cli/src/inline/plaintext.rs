//! Non-TTY plain-text fallback renderer.
//!
//! When stdout is not a TTY (piped, CI, redirected), all primitives render
//! as plain text with Unicode symbols but no ANSI color codes. This ensures
//! every `roko` command produces readable output in any environment.

use ratatui::text::Line;

/// Convert styled ratatui `Line`s to plain text by stripping styles.
///
/// Preserves the text content and Unicode symbols (◆│└✔✖) but removes
/// all ANSI color/modifier information.
pub fn lines_to_plain(lines: &[Line<'_>]) -> String {
    let mut output = String::new();
    for line in lines {
        for span in &line.spans {
            output.push_str(&span.content);
        }
        output.push('\n');
    }
    output
}

/// Print styled lines to stdout as plain text.
///
/// Use this when `should_use_inline()` returns `false`.
pub fn print_plain(lines: &[Line<'_>]) {
    for line in lines {
        for span in &line.spans {
            print!("{}", span.content);
        }
        println!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;
    use ratatui::text::Span;

    #[test]
    fn strips_styles() {
        let lines = vec![
            Line::from(vec![
                Span::styled("hello ", Style::default()),
                Span::styled("world", Style::default()),
            ]),
            Line::from(vec![Span::raw("plain")]),
        ];
        let text = lines_to_plain(&lines);
        assert_eq!(text, "hello world\nplain\n");
    }

    #[test]
    fn empty_lines() {
        let lines: Vec<Line<'_>> = vec![];
        let text = lines_to_plain(&lines);
        assert!(text.is_empty());
    }

    #[test]
    fn unicode_preserved() {
        let lines = vec![Line::from(vec![
            Span::raw("◆ agent  auditor@v1  ·  ✔ pass"),
        ])];
        let text = lines_to_plain(&lines);
        assert!(text.contains('◆'));
        assert!(text.contains('✔'));
    }
}
