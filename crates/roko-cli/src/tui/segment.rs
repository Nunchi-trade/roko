//! Semantic parsing and cached styling for agent output.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::ansi::parse_ansi_line;
use super::dashboard::Theme;

/// Semantic segment types for agent output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// Natural language reasoning.
    Thinking,
    /// Markdown headers.
    Heading,
    /// Tool invocations and shell commands.
    ToolUse,
    /// Code blocks and indented code snippets.
    Code,
    /// Positive outcomes.
    Success,
    /// Failures and compiler errors.
    Error,
    /// Empty lines.
    Blank,
    /// Turn boundary markers.
    TurnMarker,
}

impl SegmentKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Thinking => "thinking",
            Self::Heading => "heading",
            Self::ToolUse => "tool use",
            Self::Code => "code",
            Self::Success => "success",
            Self::Error => "error",
            Self::Blank => "blank",
            Self::TurnMarker => "turn",
        }
    }
}

/// Consecutive lines that share the same semantic kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentGroup {
    /// Shared kind for all lines in the group.
    pub kind: SegmentKind,
    /// Raw lines in display order.
    pub lines: Vec<String>,
}

/// Cached parse and render result for an agent output buffer.
#[derive(Debug, Clone, Default)]
pub struct CachedRender {
    /// Last rendered byte length of the raw output.
    pub last_len: usize,
    /// Parsed semantic groups.
    pub groups: Vec<SegmentGroup>,
    /// Styled lines ready for ratatui rendering.
    pub styled_lines: Vec<Line<'static>>,
}

pub(crate) fn output_byte_len(lines: &[String]) -> usize {
    lines.iter().map(String::len).sum::<usize>() + lines.len().saturating_sub(1)
}

pub(crate) fn render_cached_output(lines: &[String], theme: &Theme) -> CachedRender {
    let groups = parse_segment_groups(lines);
    let styled_lines = render_groups(&groups, theme);
    CachedRender {
        last_len: output_byte_len(lines),
        groups,
        styled_lines,
    }
}

fn parse_segment_groups(lines: &[String]) -> Vec<SegmentGroup> {
    let mut groups = Vec::new();
    let mut state = ParseState::default();

    for raw_line in preprocess_lines(lines) {
        let plain = strip_ansi(&raw_line);
        let trimmed = plain.trim();
        let is_fence = plain.trim_start().starts_with("```");
        let was_in_indented_code = state.in_indented_code;
        let kind = classify_line(&plain, trimmed, is_fence, &state);

        push_group(&mut groups, kind, raw_line);

        if is_fence {
            state.in_fenced_code = !state.in_fenced_code;
            state.in_indented_code = false;
            state.prev_was_blank = false;
        } else if state.in_fenced_code {
            state.prev_was_blank = false;
        } else if kind == SegmentKind::Blank {
            state.prev_was_blank = true;
            state.in_indented_code = false;
        } else {
            state.prev_was_blank = false;
            if kind == SegmentKind::Code {
                state.in_indented_code = was_in_indented_code || leading_spaces(&plain) >= 4;
            } else {
                state.in_indented_code = false;
            }
        }
    }

    groups
}

fn preprocess_lines(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();

    for line in lines {
        let expanded = insert_prompt_breaks(line);
        if expanded.is_empty() {
            out.push(String::new());
            continue;
        }

        let mut saw_chunk = false;
        for chunk in expanded.lines() {
            saw_chunk = true;
            if chunk.len() > 120 && !chunk.trim_start().starts_with("```") {
                out.extend(split_on_sentences(chunk));
            } else {
                out.push(chunk.to_string());
            }
        }
        if !saw_chunk {
            out.push(String::new());
        }
    }

    out
}

fn insert_prompt_breaks(line: &str) -> String {
    let mut expanded = line.to_string();
    for marker in ["Now ", "Let ", "I'll ", "I need ", "Next ", "First "] {
        expanded = expanded.replace(&format!(" {marker}"), &format!("\n{marker}"));
        expanded = expanded.replace(&format!(":{marker}"), &format!(":\n{marker}"));
    }
    expanded
}

fn split_on_sentences(line: &str) -> Vec<String> {
    if line.len() <= 120 {
        return vec![line.to_string()];
    }

    let split_at = line
        .match_indices(". ")
        .map(|(idx, _)| idx + 1)
        .filter(|idx| *idx <= 120)
        .last()
        .or_else(|| line[..120].rmatch_indices(' ').map(|(idx, _)| idx).next())
        .unwrap_or(120);

    let (head, tail) = line.split_at(split_at);
    let mut lines = vec![head.trim_end().to_string()];
    let tail = tail.trim_start();
    if !tail.is_empty() {
        lines.extend(split_on_sentences(tail));
    }
    lines
}

fn strip_ansi(line: &str) -> String {
    parse_ansi_line(line)
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect()
}

fn is_heading(trimmed: &str) -> bool {
    trimmed.starts_with("# ") || trimmed.starts_with("## ") || trimmed.starts_with("### ")
}

fn is_tool_use(trimmed: &str) -> bool {
    [
        "▸ ", "> ", "$ ", "Running ", "Reading ", "Writing ", "Editing ", "Created ",
    ]
    .iter()
    .any(|prefix| trimmed.starts_with(prefix))
}

fn is_success(trimmed: &str) -> bool {
    trimmed.contains('✓')
        || trimmed.contains("PASS")
        || trimmed.contains("APPROVE")
        || trimmed.starts_with("ok ")
}

fn is_error(trimmed: &str) -> bool {
    trimmed.contains("ERROR")
        || trimmed.contains("FAILED")
        || trimmed.contains("REVISE")
        || trimmed.contains("error[")
        || trimmed.starts_with("error")
}

fn leading_spaces(line: &str) -> usize {
    line.chars().take_while(|ch| *ch == ' ').count()
}

#[derive(Debug, Default, Clone, Copy)]
struct ParseState {
    in_fenced_code: bool,
    in_indented_code: bool,
    prev_was_blank: bool,
}

fn classify_line(line: &str, trimmed: &str, is_fence: bool, state: &ParseState) -> SegmentKind {
    let leading_spaces = leading_spaces(line);

    if state.in_fenced_code {
        return SegmentKind::Code;
    }

    if state.in_indented_code && (trimmed.is_empty() || leading_spaces >= 4) {
        return SegmentKind::Code;
    }

    if trimmed.starts_with("────") {
        SegmentKind::TurnMarker
    } else if trimmed.is_empty() {
        SegmentKind::Blank
    } else if is_fence {
        SegmentKind::Code
    } else if state.prev_was_blank && leading_spaces >= 4 {
        SegmentKind::Code
    } else if is_heading(trimmed) {
        SegmentKind::Heading
    } else if is_tool_use(trimmed) {
        SegmentKind::ToolUse
    } else if is_success(trimmed) {
        SegmentKind::Success
    } else if is_error(trimmed) {
        SegmentKind::Error
    } else {
        SegmentKind::Thinking
    }
}

fn push_group(groups: &mut Vec<SegmentGroup>, kind: SegmentKind, line: String) {
    let should_merge = groups.last().is_some_and(|group| group.kind == kind);
    if should_merge {
        if let Some(group) = groups.last_mut() {
            group.lines.push(line);
        }
        return;
    }

    groups.push(SegmentGroup {
        kind,
        lines: vec![line],
    });
}

fn render_groups(groups: &[SegmentGroup], theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for group in groups {
        lines.push(render_group_header(group.kind, theme));
        if matches!(group.kind, SegmentKind::Blank) {
            lines.extend(std::iter::repeat_with(Line::default).take(group.lines.len()));
            continue;
        }
        for line in &group.lines {
            lines.push(render_content_line(line, content_style(group.kind, theme)));
        }
    }

    lines
}

fn render_group_header(kind: SegmentKind, theme: &Theme) -> Line<'static> {
    let accent = match kind {
        SegmentKind::Thinking => Theme::FG_DIM,
        SegmentKind::Heading => theme.accent,
        SegmentKind::ToolUse => theme.info,
        SegmentKind::Code => theme.success,
        SegmentKind::Success => theme.success,
        SegmentKind::Error => theme.danger,
        SegmentKind::Blank => theme.muted,
        SegmentKind::TurnMarker => theme.muted,
    };
    let header_style = Style::default()
        .fg(accent)
        .add_modifier(Modifier::DIM | Modifier::BOLD);

    Line::from(vec![
        Span::styled(" ", Style::default()),
        Span::styled(kind.label().to_ascii_uppercase(), header_style),
        Span::styled(" ", Style::default()),
        Span::styled("────────", Style::default().fg(Theme::TEXT_PHANTOM)),
    ])
}

fn render_content_line(line: &str, base_style: Style) -> Line<'static> {
    let mut spans = Vec::new();
    spans.push(Span::styled(" ", base_style));

    for span in parse_ansi_line(line) {
        spans.push(Span::styled(
            span.content.into_owned(),
            base_style.patch(span.style),
        ));
    }

    Line::from(spans)
}

fn content_style(kind: SegmentKind, theme: &Theme) -> Style {
    match kind {
        SegmentKind::Thinking => Style::default().fg(Theme::FG_DIM),
        SegmentKind::Heading => Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
        SegmentKind::ToolUse => Style::default().fg(theme.info),
        SegmentKind::Code => Style::default()
            .fg(theme.success)
            .bg(Theme::BG_SECONDARY),
        SegmentKind::Success => Style::default()
            .fg(theme.success)
            .add_modifier(Modifier::BOLD),
        SegmentKind::Error => Style::default()
            .fg(theme.danger)
            .add_modifier(Modifier::BOLD),
        SegmentKind::Blank => Style::default(),
        SegmentKind::TurnMarker => Style::default().fg(theme.muted).add_modifier(Modifier::DIM),
    }
}

#[cfg(test)]
mod tests {
    use super::{SegmentKind, output_byte_len, parse_segment_groups, render_cached_output};
    use crate::tui::dashboard::Theme;

    #[test]
    fn parses_and_groups_semantic_segments() {
        let groups = parse_segment_groups(&[
            "# Heading".to_string(),
            "Running cargo check".to_string(),
            "$ cargo test".to_string(),
            "Thinking out loud".to_string(),
            "Next thought".to_string(),
            "✓ PASS".to_string(),
            "ERROR compile failed".to_string(),
            String::new(),
            "──── turn 2".to_string(),
        ]);

        assert_eq!(
            groups.iter().map(|group| group.kind).collect::<Vec<_>>(),
            vec![
                SegmentKind::Heading,
                SegmentKind::ToolUse,
                SegmentKind::Thinking,
                SegmentKind::Success,
                SegmentKind::Error,
                SegmentKind::Blank,
                SegmentKind::TurnMarker,
            ]
        );
        assert_eq!(groups[1].lines.len(), 2);
        assert_eq!(groups[2].lines.len(), 2);
    }

    #[test]
    fn preprocesses_prompt_breaks_and_long_sentences() {
        let groups = parse_segment_groups(&[format!(
            "Reasoning starts here and keeps going. {} Now break this thought. Let this one split too.",
            "Another sentence. ".repeat(8)
        )]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, SegmentKind::Thinking);
        assert!(groups[0].lines.len() >= 3);
    }

    #[test]
    fn hard_splits_long_single_sentence_without_boundaries() {
        let groups = parse_segment_groups(&["x".repeat(260)]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, SegmentKind::Thinking);
        assert!(groups[0].lines.len() >= 3);
        assert!(groups[0].lines.iter().all(|line| line.len() <= 120));
    }

    #[test]
    fn recognizes_fenced_and_indented_code() {
        let groups = parse_segment_groups(&[
            String::new(),
            "    let answer = 42;".to_string(),
            "    let more = answer + 1;".to_string(),
            "    ".to_string(),
            "    println!(\"done\");".to_string(),
            "```rust".to_string(),
            "fn main() {}".to_string(),
            "```".to_string(),
        ]);

        assert_eq!(groups[0].kind, SegmentKind::Blank);
        assert_eq!(groups[1].kind, SegmentKind::Code);
        assert_eq!(groups[1].lines.len(), 7);
    }

    #[test]
    fn cached_render_tracks_byte_len_and_headers() {
        let render = render_cached_output(
            &["Running tests".to_string(), "PASS".to_string()],
            &Theme::dark(),
        );

        assert_eq!(
            render.last_len,
            output_byte_len(&["Running tests".to_string(), "PASS".to_string()])
        );
        assert_eq!(render.groups.len(), 2);
        assert!(render.styled_lines.len() >= 4);
        assert!(
            render.styled_lines[0]
                .spans
                .iter()
                .any(|span| span.content.contains("TOOL USE"))
        );
    }

    #[test]
    fn recognizes_mori_success_and_error_markers() {
        let groups =
            parse_segment_groups(&["ok wrote patch".to_string(), "REVISE plan".to_string()]);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, SegmentKind::Success);
        assert_eq!(groups[1].kind, SegmentKind::Error);
    }

    #[test]
    fn blank_groups_render_a_header_line() {
        let render = render_cached_output(&[String::new(), String::new()], &Theme::dark());

        assert_eq!(render.groups.len(), 1);
        assert_eq!(render.groups[0].kind, SegmentKind::Blank);
        assert!(
            render.styled_lines[0]
                .spans
                .iter()
                .any(|span| span.content.contains("BLANK"))
        );
    }
}
