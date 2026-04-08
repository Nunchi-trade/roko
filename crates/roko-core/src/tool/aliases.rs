//! Canonical `snake_case` ↔ backend-native tool-name aliases (§36.13).
//!
//! Roko's canonical names are always `snake_case` (e.g. `read_file`).
//! Hosted backends use their own conventions:
//!
//! | Backend | Convention | Examples |
//! |---|---|---|
//! | Claude CLI | `PascalCase` | `Read`, `Edit`, `Bash`, `Agent` |
//! | Codex (MCP)| `snake_case` | `mori_read_file` (prefixed) |
//! | Cursor ACP | `snake_case` | `read_file` |
//! | Ollama     | canonical  | whatever `ToolDef.name` is |
//! | OpenAI     | canonical  | whatever `ToolDef.name` is |
//!
//! This module provides the alias table the Claude translator (§36.30)
//! uses to emit `--tools <csv>` flags and to normalize incoming
//! `tool_use` blocks back into canonical names.

// ─── Alias table ──────────────────────────────────────────────────────────

/// One alias entry: canonical `snake_case` ↔ hosted-backend names.
///
/// A `None` claude name means the tool is Roko-specific and has no
/// hosted-backend equivalent (e.g. `ls`, `apply_patch`, `run_tests`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolAlias {
    /// Canonical `snake_case` name — the single source of truth.
    pub canonical: &'static str,
    /// Claude CLI tool name, if the CLI exposes one (`--tools` flag).
    pub claude: Option<&'static str>,
}

/// Every built-in tool's alias entry. Mirrors the 16 tools listed in §36.b.
///
/// Order follows §36.b: `read_file` → `write_file` → `edit_file` →
/// `multi_edit` → `glob` → `grep` → `bash` → `ls` → `web_fetch` →
/// `web_search` → `notebook_edit` → `todo_write` → `task` →
/// `exit_plan_mode` → `apply_patch` → `run_tests`.
pub const ALIASES: &[ToolAlias] = &[
    ToolAlias {
        canonical: "read_file",
        claude: Some("Read"),
    },
    ToolAlias {
        canonical: "write_file",
        claude: Some("Write"),
    },
    ToolAlias {
        canonical: "edit_file",
        claude: Some("Edit"),
    },
    ToolAlias {
        canonical: "multi_edit",
        claude: Some("MultiEdit"),
    },
    ToolAlias {
        canonical: "glob",
        claude: Some("Glob"),
    },
    ToolAlias {
        canonical: "grep",
        claude: Some("Grep"),
    },
    ToolAlias {
        canonical: "bash",
        claude: Some("Bash"),
    },
    ToolAlias {
        canonical: "ls",
        claude: None,
    },
    ToolAlias {
        canonical: "web_fetch",
        claude: Some("WebFetch"),
    },
    ToolAlias {
        canonical: "web_search",
        claude: Some("WebSearch"),
    },
    ToolAlias {
        canonical: "notebook_edit",
        claude: Some("NotebookEdit"),
    },
    ToolAlias {
        canonical: "todo_write",
        claude: Some("TodoWrite"),
    },
    ToolAlias {
        canonical: "task",
        claude: Some("Agent"),
    },
    ToolAlias {
        canonical: "exit_plan_mode",
        claude: Some("ExitPlanMode"),
    },
    ToolAlias {
        canonical: "apply_patch",
        claude: None,
    },
    ToolAlias {
        canonical: "run_tests",
        claude: None,
    },
];

// ─── Lookup helpers ───────────────────────────────────────────────────────

/// Resolve a Claude CLI `PascalCase` name back to its canonical `snake_case`.
///
/// Returns `None` if the name isn't in the built-in alias table — the
/// caller is then free to treat it as an MCP or plugin tool whose name
/// flows through untranslated.
#[must_use]
pub fn canonical_of_claude(claude: &str) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|a| a.claude == Some(claude))
        .map(|a| a.canonical)
}

/// Resolve a canonical `snake_case` name to its Claude CLI `PascalCase` name.
#[must_use]
pub fn claude_of_canonical(canonical: &str) -> Option<&'static str> {
    ALIASES
        .iter()
        .find(|a| a.canonical == canonical)
        .and_then(|a| a.claude)
}

/// The full list of canonical names in §36.b order.
pub fn canonical_names() -> impl Iterator<Item = &'static str> {
    ALIASES.iter().map(|a| a.canonical)
}

/// All Claude CLI names that Roko canonically owns.
pub fn claude_names() -> impl Iterator<Item = &'static str> {
    ALIASES.iter().filter_map(|a| a.claude)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn aliases_match_parity_checklist_sixteen_entries() {
        assert_eq!(ALIASES.len(), 16);
    }

    #[test]
    fn every_canonical_name_is_snake_case() {
        for a in ALIASES {
            assert_eq!(a.canonical, a.canonical.to_lowercase());
            assert!(!a.canonical.contains(' '));
            assert!(!a.canonical.contains('-'));
        }
    }

    #[test]
    fn every_claude_name_starts_with_uppercase() {
        for a in ALIASES {
            if let Some(name) = a.claude {
                assert!(
                    name.chars().next().is_some_and(char::is_uppercase),
                    "Claude alias should be PascalCase: {name}"
                );
            }
        }
    }

    #[test]
    fn canonical_names_are_unique() {
        let mut seen = HashSet::new();
        for a in ALIASES {
            assert!(
                seen.insert(a.canonical),
                "duplicate canonical: {}",
                a.canonical
            );
        }
    }

    #[test]
    fn claude_names_are_unique_when_present() {
        let mut seen = HashSet::new();
        for a in ALIASES {
            if let Some(name) = a.claude {
                assert!(seen.insert(name), "duplicate Claude name: {name}");
            }
        }
    }

    #[test]
    fn canonical_of_claude_round_trips() {
        assert_eq!(canonical_of_claude("Read"), Some("read_file"));
        assert_eq!(canonical_of_claude("Bash"), Some("bash"));
        assert_eq!(canonical_of_claude("MultiEdit"), Some("multi_edit"));
        assert_eq!(canonical_of_claude("Agent"), Some("task"));
        assert_eq!(canonical_of_claude("ExitPlanMode"), Some("exit_plan_mode"));
        assert_eq!(canonical_of_claude("Nonesuch"), None);
    }

    #[test]
    fn claude_of_canonical_round_trips() {
        assert_eq!(claude_of_canonical("read_file"), Some("Read"));
        assert_eq!(claude_of_canonical("bash"), Some("Bash"));
        assert_eq!(claude_of_canonical("task"), Some("Agent"));
        // Roko-only tools return None.
        assert_eq!(claude_of_canonical("ls"), None);
        assert_eq!(claude_of_canonical("apply_patch"), None);
        assert_eq!(claude_of_canonical("run_tests"), None);
        // Unknown names return None.
        assert_eq!(claude_of_canonical("unknown"), None);
    }

    #[test]
    fn canonical_names_iterator_is_full() {
        let names: Vec<&str> = canonical_names().collect();
        assert_eq!(names.len(), 16);
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"apply_patch"));
    }

    #[test]
    fn claude_names_iterator_skips_none() {
        let names: Vec<&str> = claude_names().collect();
        // 16 entries, 3 without claude names (ls, apply_patch, run_tests) = 13.
        assert_eq!(names.len(), 13);
        assert!(!names.contains(&"")); // no empties
    }
}
