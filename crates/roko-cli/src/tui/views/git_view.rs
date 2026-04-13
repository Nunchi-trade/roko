//! F4 Git view -- branch tree, worktree list, commit graph, branch info.
//!
//! Two-panel layout: left 35% (branch tree + worktree list),
//! right 65% (commit graph + branch info).
//!
//! Populates data by running git commands when the TuiState fields are
//! empty, so the view always shows real repository state.

use std::process::Command;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, List, ListItem, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use super::ViewState;
use crate::tui::dashboard::{DashboardData, Theme};
use crate::tui::state::TuiState;

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

/// Git view data container.
#[derive(Debug, Clone, Default)]
pub struct GitViewData {
    pub branches: Vec<GitBranchNode>,
    pub worktrees: Vec<WorktreeEntry>,
    pub commits: Vec<CommitEntry>,
    pub current_branch: String,
    pub remote_url: String,
    pub status_lines: Vec<String>,
}

/// Render the full git view.
///
/// Uses pre-populated git data from `TuiState::git_view_data` (filled by
/// the background refresh thread) so the render path does zero I/O.
/// Falls back to an empty `GitViewData` if the background thread hasn't
/// delivered data yet.
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    _data: &DashboardData,
    tui_state: &TuiState,
    view_state: &ViewState,
    theme: &Theme,
) {
    let empty = GitViewData::default();
    let git_data = tui_state.git_view_data.as_ref().unwrap_or(&empty);
    render_with_git_data(frame, area, git_data, view_state, theme);
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

/// Left panel: branch tree (top 50%) + worktree list (mid 25%) + status (bottom 25%).
fn render_left_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(25),
        Constraint::Percentage(25),
    ])
    .split(area);

    render_branch_tree(frame, sections[0], git_data, view_state, theme);
    render_worktree_list(frame, sections[1], git_data, theme);
    render_status(frame, sections[2], git_data, theme);
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
        .title(format!(" Branches ({}) ", git_data.branches.len()))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if git_data.branches.is_empty() {
        let empty = Paragraph::new("no branch data")
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
        .title(format!(" Worktrees ({}) ", git_data.worktrees.len()))
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

/// Status panel: git status summary.
fn render_status(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    theme: &Theme,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Status ")
        .border_style(theme.muted());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if git_data.status_lines.is_empty() {
        let empty = Paragraph::new("clean working tree")
            .style(theme.success())
            .wrap(Wrap { trim: false });
        frame.render_widget(empty, inner);
        return;
    }

    let lines: Vec<Line<'_>> = git_data
        .status_lines
        .iter()
        .take(inner.height as usize)
        .map(|line| {
            let style = if line.starts_with('M') || line.starts_with(" M") {
                theme.warning()
            } else if line.starts_with('A') || line.starts_with("??") {
                theme.success()
            } else if line.starts_with('D') {
                theme.danger()
            } else {
                theme.text()
            };
            Line::from(Span::styled(truncate(line, 40), style))
        })
        .collect();

    let remaining = git_data.status_lines.len().saturating_sub(inner.height as usize);
    let mut all_lines = lines;
    if remaining > 0 {
        all_lines.push(Line::from(Span::styled(
            format!("  ... +{remaining} more"),
            theme.muted(),
        )));
    }

    let paragraph = Paragraph::new(all_lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

/// Right panel: commit graph (top 60%) + branch info (bottom 40%).
fn render_right_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    git_data: &GitViewData,
    view_state: &ViewState,
    theme: &Theme,
) {
    let sections =
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).split(area);

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
        .title(format!(" Commit Graph ({}) ", git_data.commits.len()))
        .border_style(theme.accent());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if git_data.commits.is_empty() {
        let empty = Paragraph::new("no commit history")
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

    let current_node = git_data.branches.iter().find(|b| b.is_current);

    let tracking_display = current_node
        .and_then(|n| n.tracking.as_deref())
        .unwrap_or("(none)");

    let lines = vec![
        Line::from(vec![
            Span::styled("branch:   ", theme.muted()),
            Span::styled(current, theme.accent_bold()),
        ]),
        Line::from(vec![
            Span::styled("remote:   ", theme.muted()),
            Span::raw(if git_data.remote_url.is_empty() {
                "(none)"
            } else {
                git_data.remote_url.as_str()
            }),
        ]),
        Line::from(vec![
            Span::styled("tracking: ", theme.muted()),
            Span::raw(tracking_display),
        ]),
        Line::from(vec![
            Span::styled("ahead:    ", theme.muted()),
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
        Line::from(vec![
            Span::styled("branches: ", theme.muted()),
            Span::raw(git_data.branches.len().to_string()),
            Span::raw("  "),
            Span::styled("worktrees: ", theme.muted()),
            Span::raw(git_data.worktrees.len().to_string()),
        ]),
        Line::from(vec![
            Span::styled("modified: ", theme.muted()),
            Span::raw(git_data.status_lines.len().to_string()),
            Span::styled(" files", theme.muted()),
        ]),
    ];

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

// ---------------------------------------------------------------------------
// Git data collection
// ---------------------------------------------------------------------------

/// Collect live git data by running git commands.
///
/// This is intentionally expensive (multiple git subprocess calls) and
/// should only be called from a background thread, never from the render path.
pub fn collect_git_data() -> GitViewData {
    let current_branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"])
        .unwrap_or_default()
        .trim()
        .to_string();

    let remote_url = run_git(&["remote", "get-url", "origin"])
        .unwrap_or_default()
        .trim()
        .to_string();

    let branches = collect_branches(&current_branch);
    let worktrees = collect_worktrees();
    let commits = collect_commits();
    let status_lines = collect_status();

    GitViewData {
        branches,
        worktrees,
        commits,
        current_branch,
        remote_url,
        status_lines,
    }
}

/// Run a git command and return stdout as a string.
fn run_git(args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}

/// Collect branch list with ahead/behind info.
fn collect_branches(current_branch: &str) -> Vec<GitBranchNode> {
    // git branch --format with ahead/behind
    let output = run_git(&[
        "for-each-ref",
        "--sort=-committerdate",
        "--format=%(refname:short)\t%(upstream:short)\t%(upstream:track)",
        "refs/heads/",
    ]);

    let Some(output) = output else {
        return Vec::new();
    };

    let mut branches = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        let name = parts.first().map_or("", |s| s.trim()).to_string();
        let tracking = parts.get(1).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

        let track_info = parts.get(2).map_or("", |s| s.trim());
        let (ahead, behind) = parse_ahead_behind(track_info);

        let is_current = name == current_branch;
        // Compute depth from path segments (e.g. feature/foo -> depth 1)
        let depth = name.matches('/').count().min(3) as u16;

        branches.push(GitBranchNode {
            name,
            is_current,
            tracking,
            ahead,
            behind,
            depth,
        });
    }

    // Ensure current branch is first
    branches.sort_by(|a, b| b.is_current.cmp(&a.is_current).then(a.name.cmp(&b.name)));
    branches
}

/// Parse "[ahead N, behind M]" from git tracking info.
fn parse_ahead_behind(s: &str) -> (u32, u32) {
    let mut ahead = 0u32;
    let mut behind = 0u32;
    if s.contains("ahead") {
        if let Some(n) = s
            .split("ahead ")
            .nth(1)
            .and_then(|s| s.split([',', ']']).next())
            .and_then(|n| n.trim().parse().ok())
        {
            ahead = n;
        }
    }
    if s.contains("behind") {
        if let Some(n) = s
            .split("behind ")
            .nth(1)
            .and_then(|s| s.split(']').next())
            .and_then(|n| n.trim().parse().ok())
        {
            behind = n;
        }
    }
    (ahead, behind)
}

/// Collect worktree list.
fn collect_worktrees() -> Vec<WorktreeEntry> {
    let output = run_git(&["worktree", "list", "--porcelain"]);
    let Some(output) = output else {
        return Vec::new();
    };

    let mut worktrees = Vec::new();
    let mut current_path = String::new();
    let mut current_branch = String::new();

    for line in output.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            if !current_path.is_empty() {
                worktrees.push(WorktreeEntry {
                    path: current_path.clone(),
                    branch: current_branch.clone(),
                    status: String::from("active"),
                });
            }
            current_path = path.trim().to_string();
            current_branch = String::new();
        } else if let Some(branch) = line.strip_prefix("branch ") {
            current_branch = branch
                .trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(branch.trim())
                .to_string();
        } else if line.trim() == "bare" {
            current_branch = String::from("(bare)");
        } else if line.trim() == "detached" {
            current_branch = String::from("(detached)");
        }
    }

    // Push last entry
    if !current_path.is_empty() {
        worktrees.push(WorktreeEntry {
            path: current_path,
            branch: current_branch,
            status: String::from("active"),
        });
    }

    worktrees
}

/// Collect recent commits with graph.
fn collect_commits() -> Vec<CommitEntry> {
    let output = run_git(&[
        "log",
        "--oneline",
        "--graph",
        "--decorate=short",
        "-30",
        "--format=%h\t%s\t%an",
    ]);
    let Some(output) = output else {
        return Vec::new();
    };

    let mut commits = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // The graph characters come before the hash. Split on the first
        // non-graph character sequence that looks like a short hash.
        let (graph_prefix, rest) = split_graph_line(line);
        let parts: Vec<&str> = rest.splitn(3, '\t').collect();

        let hash_short = parts.first().map_or("", |s| s.trim()).to_string();
        let subject = parts.get(1).map_or("", |s| s.trim()).to_string();
        let author = parts.get(2).map_or("", |s| s.trim()).to_string();

        if !hash_short.is_empty() {
            commits.push(CommitEntry {
                hash_short,
                subject,
                author,
                graph_prefix,
            });
        }
    }

    commits
}

/// Split a git log --graph line into graph prefix and rest.
fn split_graph_line(line: &str) -> (String, &str) {
    // Graph chars: *, |, /, \, space
    let graph_end = line
        .char_indices()
        .find(|(_, ch)| !matches!(ch, '*' | '|' | '/' | '\\' | ' ' | '_'))
        .map_or(line.len(), |(idx, _)| idx);
    let prefix = &line[..graph_end];
    let rest = &line[graph_end..];
    (prefix.to_string(), rest)
}

/// Collect git status --short.
fn collect_status() -> Vec<String> {
    let output = run_git(&["status", "--short"]);
    let Some(output) = output else {
        return Vec::new();
    };
    output
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(50)
        .map(|l| l.to_string())
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
