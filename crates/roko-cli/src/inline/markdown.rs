//! Markdown rendering for terminal output.
//!
//! Uses `pulldown-cmark` to parse markdown and renders to styled ratatui
//! `Line`s with full ROSEDUST theming. Supports: headers, bold, italic,
//! code spans, fenced code blocks, tables, lists, links, block quotes.

use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::Theme;

use super::symbols;

/// Convert markdown to styled `Line`s.
pub fn render_markdown(source: &str, theme: &Theme) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(source, options);
    let mut renderer = MdRenderer::new(theme);
    for event in parser {
        renderer.process(event);
    }
    renderer.finish()
}

/// Convert markdown to styled `Line`s with bar prefix.
pub fn render_markdown_with_bar(source: &str, theme: &Theme) -> Vec<Line<'static>> {
    let lines = render_markdown(source, theme);
    let bar_style = theme.muted();

    lines
        .into_iter()
        .map(|line| {
            let mut spans = vec![
                Span::styled(symbols::BAR.to_string(), bar_style),
                Span::raw(" "),
            ];
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// Render a code block with a language header and border.
pub fn render_code_block(code: &str, language: Option<&str>, theme: &Theme) -> Vec<Line<'static>> {
    let lang_label = language.unwrap_or("text");
    let bar_style = theme.muted();
    let code_style = Style::default().fg(Theme::BONE);
    let border_style = Style::default().fg(Theme::TEXT_GHOST);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(symbols::BAR.to_string(), bar_style),
        Span::raw(" "),
        Span::styled(format!("┌─ {lang_label} "), border_style),
        Span::styled("─".repeat(30), border_style),
    ]));
    for code_line in code.lines() {
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), bar_style),
            Span::raw(" "),
            Span::styled("│ ".to_string(), border_style),
            Span::styled(code_line.to_string(), code_style),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled(symbols::BAR.to_string(), bar_style),
        Span::raw(" "),
        Span::styled(format!("└{}", "─".repeat(32 + lang_label.len())), border_style),
    ]));
    lines
}

/// Plain-text rendering with bar prefix.
pub fn render_plain_with_bar(text: &str, theme: &Theme) -> Vec<Line<'static>> {
    let bar_style = theme.muted();
    let text_style = theme.text();

    text.lines()
        .map(|line| {
            Line::from(vec![
                Span::styled(symbols::BAR.to_string(), bar_style),
                Span::raw(" "),
                Span::styled(line.to_string(), text_style),
            ])
        })
        .collect()
}

// ---------------------------------------------------------------------------
// pulldown-cmark → ratatui renderer
// ---------------------------------------------------------------------------

struct MdRenderer {
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    base_style: Style,
    // Code blocks
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_buf: String,
    // Tables
    in_table: bool,
    table_alignments: Vec<Alignment>,
    table_row: Vec<Vec<Span<'static>>>,
    table_rows: Vec<Vec<Vec<Span<'static>>>>,
    in_table_head: bool,
    // Lists
    list_depth: usize,
    // Block quotes
    blockquote_depth: usize,
    // Styles
    heading_style: Style,
    code_span_style: Style,
    link_style: Style,
    blockquote_prefix_style: Style,
    table_border_style: Style,
    table_header_style: Style,
}

impl MdRenderer {
    fn new(theme: &Theme) -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: Vec::new(),
            base_style: theme.text(),
            in_code_block: false,
            code_block_lang: None,
            code_block_buf: String::new(),
            in_table: false,
            table_alignments: Vec::new(),
            table_row: Vec::new(),
            table_rows: Vec::new(),
            in_table_head: false,
            list_depth: 0,
            blockquote_depth: 0,
            heading_style: Style::default()
                .fg(Theme::ROSE_BRIGHT)
                .add_modifier(Modifier::BOLD),
            code_span_style: Style::default()
                .fg(Theme::BONE)
                .add_modifier(Modifier::DIM),
            link_style: Style::default()
                .fg(Theme::DREAM)
                .add_modifier(Modifier::UNDERLINED),
            blockquote_prefix_style: Style::default().fg(Theme::TEXT_GHOST),
            table_border_style: Style::default().fg(Theme::TEXT_GHOST),
            table_header_style: Style::default()
                .fg(Theme::BONE)
                .add_modifier(Modifier::BOLD),
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or(self.base_style)
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(style);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn flush_line(&mut self) {
        if !self.current_spans.is_empty() {
            let mut spans = Vec::new();
            // Prepend blockquote prefix if inside a quote
            if self.blockquote_depth > 0 {
                let prefix = format!("{} ", "▎".repeat(self.blockquote_depth));
                spans.push(Span::styled(prefix, self.blockquote_prefix_style));
            }
            spans.append(&mut self.current_spans);
            self.lines.push(Line::from(spans));
        }
    }

    fn process(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => {
                if self.in_code_block {
                    self.code_block_buf.push_str(&text);
                } else if self.in_table {
                    let style = if self.in_table_head {
                        self.table_header_style
                    } else {
                        self.current_style()
                    };
                    if let Some(cell) = self.table_row.last_mut() {
                        cell.push(Span::styled(text.to_string(), style));
                    }
                } else {
                    let style = self.current_style();
                    self.current_spans
                        .push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(code) => {
                if self.in_table {
                    if let Some(cell) = self.table_row.last_mut() {
                        cell.push(Span::styled(
                            format!("`{code}`"),
                            self.code_span_style,
                        ));
                    }
                } else {
                    self.current_spans
                        .push(Span::styled(format!("`{code}`"), self.code_span_style));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if !self.in_table {
                    self.flush_line();
                }
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_line();
                // Render heading with a decorative prefix
                let prefix = match level as usize {
                    1 => "━━ ".to_string(),
                    2 => "── ".to_string(),
                    _ => format!("{} ", "#".repeat(level as usize)),
                };
                self.current_spans
                    .push(Span::styled(prefix, self.heading_style));
                self.push_style(self.heading_style);
            }
            Tag::Emphasis => {
                let style = self.current_style().add_modifier(Modifier::ITALIC);
                self.push_style(style);
            }
            Tag::Strong => {
                let style = self.current_style().add_modifier(Modifier::BOLD);
                self.push_style(style);
            }
            Tag::Strikethrough => {
                let style = self.current_style().add_modifier(Modifier::CROSSED_OUT);
                self.push_style(style);
            }
            Tag::Link { .. } => {
                self.push_style(self.link_style);
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_block_buf.clear();
                self.code_block_lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                        let lang = lang.to_string();
                        if lang.is_empty() { None } else { Some(lang) }
                    }
                    _ => None,
                };
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                self.blockquote_depth += 1;
                let style = Style::default().fg(Theme::TEXT_DIM);
                self.push_style(style);
            }
            Tag::List(_) => {
                if self.list_depth == 0 {
                    self.flush_line();
                }
                self.list_depth += 1;
            }
            Tag::Item => {
                self.flush_line();
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let bullet = match self.list_depth {
                    1 => "•",
                    2 => "◦",
                    _ => "‣",
                };
                self.current_spans.push(Span::styled(
                    format!("{indent}{bullet} "),
                    Style::default().fg(Theme::TEXT_DIM),
                ));
            }
            Tag::Paragraph => {
                self.flush_line();
            }
            Tag::Table(alignments) => {
                self.flush_line();
                self.in_table = true;
                self.table_alignments = alignments;
                self.table_rows.clear();
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_row.clear();
            }
            Tag::TableRow => {
                self.table_row.clear();
            }
            Tag::TableCell => {
                self.table_row.push(Vec::new());
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.pop_style();
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                let code = std::mem::take(&mut self.code_block_buf);
                let lang = self.code_block_lang.take();
                let lang_label = lang.as_deref().unwrap_or("");
                let border = self.table_border_style;
                let code_style = Style::default().fg(Theme::BONE);

                // Top border
                if lang_label.is_empty() {
                    self.lines.push(Line::from(vec![
                        Span::styled("  ┌".to_string(), border),
                        Span::styled("─".repeat(40), border),
                    ]));
                } else {
                    self.lines.push(Line::from(vec![
                        Span::styled(format!("  ┌─ {lang_label} "), border),
                        Span::styled("─".repeat(34usize.saturating_sub(lang_label.len())), border),
                    ]));
                }

                // Code lines
                for code_line in code.lines() {
                    self.lines.push(Line::from(vec![
                        Span::styled("  │ ".to_string(), border),
                        Span::styled(code_line.to_string(), code_style),
                    ]));
                }

                // Bottom border
                self.lines.push(Line::from(vec![
                    Span::styled("  └".to_string(), border),
                    Span::styled("─".repeat(40), border),
                ]));
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                self.pop_style();
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                if self.list_depth == 0 {
                    self.flush_line();
                }
            }
            TagEnd::Paragraph => {
                self.flush_line();
                // Blank line after paragraphs
                self.lines.push(Line::raw(""));
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                // Store header row
                let row = std::mem::take(&mut self.table_row);
                self.table_rows.push(row);
            }
            TagEnd::TableRow => {
                let row = std::mem::take(&mut self.table_row);
                self.table_rows.push(row);
            }
            TagEnd::Table => {
                self.in_table = false;
                self.render_table();
            }
            TagEnd::TableCell | TagEnd::Item => {}
            _ => {}
        }
    }

    fn render_table(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }

        let border = self.table_border_style;
        let col_count = self.table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if col_count == 0 {
            return;
        }

        // Compute column widths from content
        let mut widths = vec![0usize; col_count];
        for row in &self.table_rows {
            for (i, cell) in row.iter().enumerate() {
                let text_len: usize = cell.iter().map(|s| s.content.len()).sum();
                if i < widths.len() {
                    widths[i] = widths[i].max(text_len);
                }
            }
        }
        // Minimum width and padding
        for w in &mut widths {
            *w = (*w).max(3) + 2; // 1 char padding each side
        }

        // Top border
        let top: String = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("┬");
        self.lines.push(Line::from(vec![
            Span::styled(format!("  ┌{top}┐"), border),
        ]));

        for (row_idx, row) in self.table_rows.iter().enumerate() {
            // Data row
            let mut spans = vec![Span::styled("  │".to_string(), border)];
            for (col_idx, cell) in row.iter().enumerate() {
                let text: String = cell.iter().map(|s| s.content.as_ref()).collect();
                let width = widths.get(col_idx).copied().unwrap_or(10);
                let padded = format!(" {:<w$}", text, w = width - 1);

                let style = if row_idx == 0 {
                    self.table_header_style
                } else {
                    self.base_style
                };
                spans.push(Span::styled(padded, style));
                spans.push(Span::styled("│".to_string(), border));
            }
            // Fill missing columns
            for col_idx in row.len()..col_count {
                let width = widths.get(col_idx).copied().unwrap_or(10);
                spans.push(Span::styled(" ".repeat(width), self.base_style));
                spans.push(Span::styled("│".to_string(), border));
            }
            self.lines.push(Line::from(spans));

            // Separator after header
            if row_idx == 0 {
                let sep: String = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("┼");
                self.lines.push(Line::from(vec![
                    Span::styled(format!("  ├{sep}┤"), border),
                ]));
            }
        }

        // Bottom border
        let bottom: String = widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("┴");
        self.lines.push(Line::from(vec![
            Span::styled(format!("  └{bottom}┘"), border),
        ]));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        // Trim trailing blank lines
        while self.lines.last().is_some_and(|l| l.spans.is_empty() || l.to_string().trim().is_empty()) {
            self.lines.pop();
        }
        self.lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_plain_text() {
        let theme = Theme::dark();
        let lines = render_markdown("Hello world", &theme);
        assert!(!lines.is_empty());
    }

    #[test]
    fn render_heading() {
        let theme = Theme::dark();
        let lines = render_markdown("# Title\n\nBody", &theme);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn render_code_block_fenced() {
        let theme = Theme::dark();
        let lines = render_markdown("```rust\nlet x = 1;\n```", &theme);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn render_table() {
        let theme = Theme::dark();
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |";
        let lines = render_markdown(md, &theme);
        // Should have: top border, header, separator, 2 data rows, bottom border
        assert!(lines.len() >= 6, "got {} lines", lines.len());
        // Check that table borders are present
        let all_text: String = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        assert!(all_text.contains('┌'), "missing top border");
        assert!(all_text.contains('┘'), "missing bottom border");
        assert!(all_text.contains('│'), "missing column separator");
    }

    #[test]
    fn render_bold_italic() {
        let theme = Theme::dark();
        let lines = render_markdown("**bold** and *italic*", &theme);
        assert!(!lines.is_empty());
        assert!(lines[0].spans.len() >= 2);
    }

    #[test]
    fn render_list() {
        let theme = Theme::dark();
        let lines = render_markdown("- one\n- two\n- three", &theme);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn render_blockquote() {
        let theme = Theme::dark();
        let lines = render_markdown("> This is a quote", &theme);
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains('▎'), "missing blockquote prefix");
    }

    #[test]
    fn render_with_bar_has_prefix() {
        let theme = Theme::dark();
        let lines = render_markdown_with_bar("Hello", &theme);
        assert!(!lines.is_empty());
        let first_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first_text.starts_with(symbols::BAR));
    }

    #[test]
    fn render_plain_with_bar_basic() {
        let theme = Theme::dark();
        let lines = render_plain_with_bar("hello\nworld", &theme);
        assert_eq!(lines.len(), 2);
    }
}
