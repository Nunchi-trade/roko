//! Tab definitions for the TUI dashboard.

/// The available tabs in the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Dashboard,
    Plans,
    Agents,
    Logs,
    Signals,
    Config,
}

static ALL_TABS: [Tab; 6] = [
    Tab::Dashboard,
    Tab::Plans,
    Tab::Agents,
    Tab::Logs,
    Tab::Signals,
    Tab::Config,
];

impl Tab {
    /// Human-readable label for the tab.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Plans => "Plans",
            Self::Agents => "Agents",
            Self::Logs => "Logs",
            Self::Signals => "Signals",
            Self::Config => "Config",
        }
    }

    /// All tabs in display order.
    #[must_use]
    pub fn all() -> &'static [Tab] {
        &ALL_TABS
    }

    /// Map a zero-based index to a tab.
    #[must_use]
    pub fn from_index(i: usize) -> Option<Tab> {
        ALL_TABS.get(i).copied()
    }

    /// The function-key shortcut for this tab.
    #[must_use]
    pub const fn fkey(&self) -> &'static str {
        match self {
            Self::Dashboard => "F1",
            Self::Plans => "F2",
            Self::Agents => "F3",
            Self::Logs => "F4",
            Self::Signals => "F5",
            Self::Config => "F6",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tabs_count() {
        assert_eq!(Tab::all().len(), 6);
    }

    #[test]
    fn from_index_round_trip() {
        for (i, tab) in Tab::all().iter().enumerate() {
            assert_eq!(Tab::from_index(i), Some(*tab));
        }
        assert_eq!(Tab::from_index(6), None);
    }

    #[test]
    fn labels_not_empty() {
        for tab in Tab::all() {
            assert!(!tab.label().is_empty());
        }
    }

    #[test]
    fn fkeys_sequential() {
        let expected = ["F1", "F2", "F3", "F4", "F5", "F6"];
        for (tab, fk) in Tab::all().iter().zip(expected.iter()) {
            assert_eq!(tab.fkey(), *fk);
        }
    }
}
