//! F4 Git view -- branch tree, worktree list, commit graph, branch info.
//!
//! Two-panel layout: left 35% (branch tree + worktree list),
//! right 65% (commit graph + branch info).

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};

/// A node in the branch tree display.
#[derive(Debug, Clone)]
pub struct GitBranchNode {
    /// Branch name (e.g. "main", "feature/foo").
    pub name: String,
    /// Whether this is the currently checked-out branch.
    pub is_current: bool,
    /// Remote tracking branch, if any.
    pub tracking: Option<String>,
    /// Commits ahead of tracking branch.
    pub ahead: u32,
    /// Commits behind tracking branch.
    pub behind: u32,
    /// Indent depth for hierarchical display (e.g. feature/ prefix).
    pub depth: u16,
}

/// A worktree entry.
#[derive(Debug, Clone)]
pub struct WorktreeEntry {
    pub path: String,
    pub branch: String,
    pub status: String,
}

/// A commit log entry.
#[derive(Debug, Clone)]
pub struct CommitEntry {
    pub hash_short: String,
    pub subject: String,
    pub author: String,
    pub graph_prefix: String,
}

/// Git view data container. Populated externally and passed in.
/// When the full TuiState is wired, this will be sourced from there.
#[derive(Debug, Clone, Default)]
pub struct GitViewData {
    pub branches: Vec<GitBranchNode>,
    pub worktrees: Vec<WorktreeEntry>,
    pub commits: Vec<CommitEntry>,
    pub current_branch: String,
    pub remote_url: String,
}

/// Render the full git view.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    view_state: &ViewState,
    theme: &Theme,
) {
    // Git data is not yet available from DashboardData; render with
    // placeholder data. The integration layer will supply GitViewData
    // via an extended render function once TuiState is wired.
    let git_data = GitViewData::default();
    render_with_git_data(frame, area, &git_data, view_state, theme);
}

/// Render the git view with explicit git data (for integration layer).
pub fn render_with_git_data(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let panels =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    render_left_panel(frame, panels[0], git_data, view_state, theme);
    render_right_panel(frame, panels[1], git_data, view_state, theme);
}

/// Left panel: branch tree (top 60%) + worktree list (bottom 40%).
fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

    render_branch_tree(frame, sections[0], git_data, view_state, theme);
    render_worktree_list(frame, sections[1], git_data, theme);
}

/// Branch tree: hierarchical branch listing.
fn render_branch_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branches ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // TODO: use branch_tree widget here when available
    if git_data.branches.is_empty() {
        let empty = Paragraph::new("no branch data (run git fetch)")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let items: Vec<ListItem<'_>> = git_data
        .branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let indent = "  ".repeat(branch.depth as usize);
            let marker = if branch.is_current { "* " } else { "  " };
            let ahead_behind = if branch.ahead > 0 || branch.behind > 0 {
                format!(" [+{}/-{}]", branch.ahead, branch.behind)
            } else {
                String::new()
            };

            let style = if i == view_state.selected {
                theme.selection()
            } else if branch.is_current {
                theme.accent_bold()
            } else {
                theme.text()
            };

            ListItem::new(Line::from(vec![
                Span::raw(format!("{indent}{marker}")),
                Span::styled(&branch.name, style),
                Span::styled(ahead_behind, theme.muted()),
            ]))
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Worktree list: simple table with path, branch, status.
fn render_worktree_list(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Worktrees ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if git_data.worktrees.is_empty() {
        let empty = Paragraph::new("no worktrees")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let rows: Vec<Row<'_>> = git_data
        .worktrees
        .iter()
        .map(|wt| {
            Row::new(vec![
                Cell::from(truncate(&wt.path, 24)),
                Cell::from(wt.branch.as_str()),
                Cell::from(wt.status.as_str()),
            ])
        })
        .collect();

    let widths = [Constraint::Min(16), Constraint::Min(12), Constraint::Length(10)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["path", "branch", "status"])
                .style(theme.accent().add_modifier(Modifier::BOLD)),
        )
        .column_spacing(1);
    frame.render_widget(table, inner);
}

/// Right panel: commit graph (top 50%) + branch info (bottom 50%).
fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    render_commit_graph(frame, sections[0], git_data, view_state, theme);
    render_branch_info(frame, sections[1], git_data, theme);
}

/// Commit graph: rendered git log with graph characters.
fn render_commit_graph(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Commit Graph ")
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if git_data.commits.is_empty() {
        let empty = Paragraph::new("no commit history loaded")
            .style(theme.muted())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = git_data
        .commits
        .iter()
        .map(|commit| {
            Line::from(vec![
                Span::styled(&commit.graph_prefix, theme.muted()),
                Span::styled(
                    format!(" {} ", commit.hash_short),
                    theme.warning(),
                ),
                Span::styled(&commit.subject, theme.text()),
                Span::styled(
                    format!("  ({})", commit.author),
                    theme.muted(),
                ),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((view_state.scroll, 0));
    frame.render_widget(paragraph, inner);
}

/// Branch info: current branch, remote tracking, ahead/behind.
fn render_branch_info(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branch Info ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let current = if git_data.current_branch.is_empty() {
        "(detached HEAD)"
    } else {
        git_data.current_branch.as_str()
    };

    let current_node = git_data
        .branches
        .iter()
        .find(|b| b.is_current);

    let lines = vec![
        Line::from(vec![
            Span::styled("branch:  ", theme.muted()),
            Span::styled(current, theme.accent_bold()),
        ]),
        Line::from(vec![
            Span::styled("remote:  ", theme.muted()),
            Span::raw(if git_data.remote_url.is_empty() {
                "(none)"
            } else {
                git_data.remote_url.as_str()
            }),
        ]),
        Line::from(vec![
            Span::styled("tracking:", theme.muted()),
            Span::raw(
                current_node
                    .and_then(|n| n.tracking.as_deref())
                    .unwrap_or("(none)"),
            ),
        ]),
        Line::from(vec![
            Span::styled("ahead:   ", theme.muted()),
            Span::styled(
                current_node.map_or("0".to_string(), |n| n.ahead.to_string()),
                theme.success(),
            ),
            Span::raw("  "),
            Span::styled("behind: ", theme.muted()),
            Span::styled(
                current_node.map_or("0".to_string(), |n| n.behind.to_string()),
                theme.warning(),
            ),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
