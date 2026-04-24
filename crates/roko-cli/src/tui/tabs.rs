//! Tab identifiers for the Mori-style TUI navigation.
//!
//! These map directly to `F1`-`F10` function keys and represent the top-level
//! tab bar in the interactive dashboard.

use crossterm::event::KeyCode;

/// Top-level TUI tabs, mapped to F1-F10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// F1 - Overview dashboard with health gauges, plan progress, cost.
    Dashboard,
    /// F2 - Plan tree, task progress, wave overview.
    Plans,
    /// F3 - Agent output, diffs, token burn, parallel pool.
    Agents,
    /// F4 - Git branch tree, commit graph, worktree list.
    Git,
    /// F5 - Scrollable log viewer with filtering.
    Logs,
    /// F6 - Config editor / effective config view.
    Config,
    /// F7 - Engram DAG inspector, episode replay.
    Inspect,
    /// F8 - Marketplace: job browser, creation, assignment.
    Marketplace,
    /// F9 - Atelier: PRD workshop, plan progress.
    Atelier,
    /// F10 - Learning: cascade router, model routing, efficiency.
    Learning,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 10] = [
        Tab::Dashboard,
        Tab::Plans,
        Tab::Agents,
        Tab::Git,
        Tab::Logs,
        Tab::Config,
        Tab::Inspect,
        Tab::Marketplace,
        Tab::Atelier,
        Tab::Learning,
    ];

    /// The function key that activates this tab.
    #[must_use]
    pub const fn fkey(self) -> KeyCode {
        match self {
            Self::Dashboard => KeyCode::F(1),
            Self::Plans => KeyCode::F(2),
            Self::Agents => KeyCode::F(3),
            Self::Git => KeyCode::F(4),
            Self::Logs => KeyCode::F(5),
            Self::Config => KeyCode::F(6),
            Self::Inspect => KeyCode::F(7),
            Self::Marketplace => KeyCode::F(8),
            Self::Atelier => KeyCode::F(9),
            Self::Learning => KeyCode::F(10),
        }
    }

    /// Try to match a key code to a tab.
    #[must_use]
    pub const fn from_key(key: KeyCode) -> Option<Self> {
        match key {
            KeyCode::F(1) => Some(Self::Dashboard),
            KeyCode::F(2) => Some(Self::Plans),
            KeyCode::F(3) => Some(Self::Agents),
            KeyCode::F(4) => Some(Self::Git),
            KeyCode::F(5) => Some(Self::Logs),
            KeyCode::F(6) => Some(Self::Config),
            KeyCode::F(7) => Some(Self::Inspect),
            KeyCode::F(8) => Some(Self::Marketplace),
            KeyCode::F(9) => Some(Self::Atelier),
            KeyCode::F(10) => Some(Self::Learning),
            _ => None,
        }
    }

    /// Human-readable label for the tab bar.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Plans => "Plans",
            Self::Agents => "Agents",
            Self::Git => "Git",
            Self::Logs => "Logs",
            Self::Config => "Config",
            Self::Inspect => "Inspect",
            Self::Marketplace => "Marketplace",
            Self::Atelier => "Atelier",
            Self::Learning => "Learning",
        }
    }

    /// Short label with function key hint, e.g. "F1 Dashboard".
    #[must_use]
    pub const fn label_with_key(self) -> &'static str {
        match self {
            Self::Dashboard => "F1 Dashboard",
            Self::Plans => "F2 Plans",
            Self::Agents => "F3 Agents",
            Self::Git => "F4 Git",
            Self::Logs => "F5 Logs",
            Self::Config => "F6 Config",
            Self::Inspect => "F7 Inspect",
            Self::Marketplace => "F8 Marketplace",
            Self::Atelier => "F9 Atelier",
            Self::Learning => "F10 Learning",
        }
    }

    /// Zero-based index in the tab bar.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Dashboard => 0,
            Self::Plans => 1,
            Self::Agents => 2,
            Self::Git => 3,
            Self::Logs => 4,
            Self::Config => 5,
            Self::Inspect => 6,
            Self::Marketplace => 7,
            Self::Atelier => 8,
            Self::Learning => 9,
        }
    }

    /// Next tab (wraps around).
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Dashboard => Self::Plans,
            Self::Plans => Self::Agents,
            Self::Agents => Self::Git,
            Self::Git => Self::Logs,
            Self::Logs => Self::Config,
            Self::Config => Self::Inspect,
            Self::Inspect => Self::Marketplace,
            Self::Marketplace => Self::Atelier,
            Self::Atelier => Self::Learning,
            Self::Learning => Self::Dashboard,
        }
    }

    /// Previous tab (wraps around).
    #[must_use]
    pub const fn prev(self) -> Self {
        match self {
            Self::Dashboard => Self::Learning,
            Self::Plans => Self::Dashboard,
            Self::Agents => Self::Plans,
            Self::Git => Self::Agents,
            Self::Logs => Self::Git,
            Self::Config => Self::Logs,
            Self::Inspect => Self::Config,
            Self::Marketplace => Self::Inspect,
            Self::Atelier => Self::Marketplace,
            Self::Learning => Self::Atelier,
        }
    }
}

impl Default for Tab {
    fn default() -> Self {
        Self::Dashboard
    }
}

impl std::fmt::Display for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fkey_roundtrip() {
        for tab in Tab::ALL {
            assert_eq!(Tab::from_key(tab.fkey()), Some(tab));
        }
    }

    #[test]
    fn next_prev_cycle() {
        let mut t = Tab::Dashboard;
        for _ in 0..10 {
            t = t.next();
        }
        assert_eq!(t, Tab::Dashboard);

        for _ in 0..10 {
            t = t.prev();
        }
        assert_eq!(t, Tab::Dashboard);
    }

    #[test]
    fn index_is_sequential() {
        for (i, tab) in Tab::ALL.iter().enumerate() {
            assert_eq!(tab.index(), i);
        }
    }
}
