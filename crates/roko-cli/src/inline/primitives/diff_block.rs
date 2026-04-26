//! Primitive 14: `DiffBlock` — inline diff display.
//!
//! ```text
//! │ ▸ diff  3 files changed, +42 -17
//! │   deploy/env.yaml:14  removed AWS_SECRET_ACCESS_KEY
//! │   deploy/env.yaml:15  added Secrets Manager ARN
//! │   src/handler.rs:42   updated credential loading
//! ```

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::tui::Theme;

use super::super::symbols;

/// A single file change in a diff.
#[derive(Debug, Clone)]
pub struct DiffEntry {
    /// File path (relative).
    pub path: String,
    /// Lines added.
    pub additions: u32,
    /// Lines removed.
    pub deletions: u32,
    /// One-line description of the change.
    pub summary: Option<String>,
}

/// A diff block showing file changes.
#[derive(Debug, Clone)]
pub struct DiffBlockData {
    /// Individual file changes.
    pub entries: Vec<DiffEntry>,
    /// Whether to show expanded (per-file) or collapsed (summary only).
    pub expanded: bool,
}

impl DiffBlockData {
    /// Total additions across all files.
    pub fn total_additions(&self) -> u32 {
        self.entries.iter().map(|e| e.additions).sum()
    }

    /// Total deletions across all files.
    pub fn total_deletions(&self) -> u32 {
        self.entries.iter().map(|e| e.deletions).sum()
    }

    /// Render as styled lines.
    pub fn to_lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let add = self.total_additions();
        let del = self.total_deletions();
        let file_count = self.entries.len();

        let disclosure = if self.expanded {
            symbols::EXPANDED
        } else {
            symbols::COLLAPSED
        };

        // Summary line
        lines.push(Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(disclosure.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled("diff".to_string(), theme.info()),
            Span::raw("  "),
            Span::styled(
                format!(
                    "{file_count} file{} changed",
                    if file_count == 1 { "" } else { "s" },
                ),
                theme.text(),
            ),
            Span::raw(", "),
            Span::styled(format!("+{add}"), theme.success()),
            Span::raw(" "),
            Span::styled(format!("-{del}"), theme.danger()),
        ]));

        if self.expanded {
            for entry in &self.entries {
                let mut spans = vec![
                    Span::styled(symbols::BAR.to_string(), theme.muted()),
                    Span::raw("   "),
                    Span::styled(entry.path.clone(), theme.text()),
                ];
                if entry.additions > 0 || entry.deletions > 0 {
                    spans.push(Span::raw("  "));
                    if entry.additions > 0 {
                        spans.push(Span::styled(
                            format!("+{}", entry.additions),
                            theme.success(),
                        ));
                        spans.push(Span::raw(" "));
                    }
                    if entry.deletions > 0 {
                        spans.push(Span::styled(
                            format!("-{}", entry.deletions),
                            theme.danger(),
                        ));
                    }
                }
                if let Some(ref summary) = entry.summary {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        summary.clone(),
                        Style::default().fg(Theme::TEXT_DIM),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_collapsed() {
        let theme = Theme::dark();
        let data = DiffBlockData {
            entries: vec![
                DiffEntry {
                    path: "src/main.rs".into(),
                    additions: 10,
                    deletions: 3,
                    summary: None,
                },
                DiffEntry {
                    path: "src/lib.rs".into(),
                    additions: 5,
                    deletions: 2,
                    summary: Some("added function".into()),
                },
            ],
            expanded: false,
        };
        let lines = data.to_lines(&theme);
        assert_eq!(lines.len(), 1); // collapsed = summary only
        assert_eq!(data.total_additions(), 15);
        assert_eq!(data.total_deletions(), 5);
    }

    #[test]
    fn diff_expanded() {
        let theme = Theme::dark();
        let data = DiffBlockData {
            entries: vec![
                DiffEntry {
                    path: "a.rs".into(),
                    additions: 1,
                    deletions: 0,
                    summary: None,
                },
                DiffEntry {
                    path: "b.rs".into(),
                    additions: 0,
                    deletions: 1,
                    summary: None,
                },
            ],
            expanded: true,
        };
        let lines = data.to_lines(&theme);
        assert_eq!(lines.len(), 3); // summary + 2 entries
    }
}
