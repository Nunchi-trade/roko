//! Git branch tree widget with hierarchical connectors.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use super::super::dashboard::Theme;

/// The type of a git ref node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchType {
    Local,
    Remote,
    Tag,
}

/// A single node in the git branch tree.
#[derive(Debug, Clone)]
pub struct GitTreeNode {
    pub name: String,
    pub branch_type: BranchType,
    pub is_current: bool,
    pub children: Vec<GitTreeNode>,
}

/// Render a hierarchical git branch tree with connectors.
pub fn render_branch_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    nodes: &[GitTreeNode],
    cursor: usize,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("branches")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    let mut flat_index: usize = 0;
    flatten_tree(nodes, &mut lines, &mut flat_index, cursor, 0, theme);

    if lines.is_empty() {
        let empty = Paragraph::new("no branches")
            .style(theme.muted())
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(empty, inner);
        return;
    }

    // Scroll to keep cursor visible.
    let visible = inner.height as usize;
    let scroll = if cursor >= visible {
        (cursor - visible + 1) as u16
    } else {
        0
    };

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, inner);
}

fn flatten_tree<'a>(
    nodes: &[GitTreeNode],
    lines: &mut Vec<Line<'a>>,
    flat_index: &mut usize,
    cursor: usize,
    depth: usize,
    theme: &Theme,
) {
    let count = nodes.len();
    for (i, node) in nodes.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if depth == 0 {
            String::new()
        } else {
            let prefix = "  ".repeat(depth.saturating_sub(1));
            let branch = if is_last { "└── " } else { "├── " };
            format!("{prefix}{branch}")
        };

        let is_selected = *flat_index == cursor;
        let type_label = match node.branch_type {
            BranchType::Local => "",
            BranchType::Remote => " (remote)",
            BranchType::Tag => " (tag)",
        };

        let name_style = if is_selected {
            theme.selection()
        } else if node.is_current {
            Style::default()
                .fg(theme.success)
                .add_modifier(Modifier::BOLD)
        } else {
            match node.branch_type {
                BranchType::Local => Style::default().fg(theme.foreground),
                BranchType::Remote => Style::default().fg(theme.muted),
                BranchType::Tag => Style::default().fg(theme.warning),
            }
        };

        let current_marker = if node.is_current { "* " } else { "  " };

        lines.push(Line::from(vec![
            Span::styled(connector, Style::default().fg(theme.muted)),
            Span::raw(current_marker),
            Span::styled(node.name.clone(), name_style),
            Span::styled(type_label.to_string(), Style::default().fg(theme.muted)),
        ]));

        *flat_index += 1;

        if !node.children.is_empty() {
            flatten_tree(&node.children, lines, flat_index, cursor, depth + 1, theme);
        }
    }
}
