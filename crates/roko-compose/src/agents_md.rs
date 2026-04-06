//! AGENTS.md conventions loader.
//!
//! Parses an `AGENTS.md` file (or similar conventions document) for
//! project-specific agent conventions. Extracts sections by heading and
//! returns structured data the [`SystemPromptBuilder`](super::system_prompt_builder)
//! can inject into layer 2 (conventions).
//!
//! Anti-pattern #8: **no `std::fs` in this module**. The raw markdown
//! content arrives via function parameters.

use serde::{Deserialize, Serialize};

/// A single heading-delimited section from an AGENTS.md file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsSection {
    /// The heading text (without `#` prefix).
    pub heading: String,
    /// The heading depth (1 = `#`, 2 = `##`, etc.).
    pub depth: usize,
    /// The body text under this heading (until the next heading of equal or
    /// lesser depth).
    pub body: String,
}

/// Structured representation of an AGENTS.md file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsMd {
    /// Parsed sections, in document order.
    pub sections: Vec<AgentsSection>,
    /// Raw content (kept for pass-through injection when the caller wants
    /// the full text rather than individual sections).
    pub raw: String,
}

impl AgentsMd {
    /// Parse an AGENTS.md markdown string into structured sections.
    ///
    /// Sections are delimited by ATX headings (`# Heading`). Content
    /// before the first heading is captured under a synthetic `"preamble"`
    /// heading at depth 0.
    #[must_use]
    pub fn parse(content: &str) -> Self {
        let mut sections = Vec::new();
        let mut current_heading = String::from("preamble");
        let mut current_depth: usize = 0;
        let mut current_body = String::new();

        for line in content.lines() {
            if let Some((depth, heading)) = parse_heading(line) {
                // Flush the previous section.
                let body = current_body.trim().to_string();
                if !body.is_empty() || !current_heading.is_empty() {
                    sections.push(AgentsSection {
                        heading: current_heading,
                        depth: current_depth,
                        body,
                    });
                }
                current_heading = heading;
                current_depth = depth;
                current_body = String::new();
            } else {
                if !current_body.is_empty() {
                    current_body.push('\n');
                }
                current_body.push_str(line);
            }
        }

        // Flush the last section.
        let body = current_body.trim().to_string();
        if !body.is_empty() || !current_heading.is_empty() {
            sections.push(AgentsSection {
                heading: current_heading,
                depth: current_depth,
                body,
            });
        }

        // Remove empty preamble if it has no body.
        if sections.first().is_some_and(|s| s.heading == "preamble" && s.body.is_empty()) {
            sections.remove(0);
        }

        Self {
            sections,
            raw: content.to_string(),
        }
    }

    /// Find the first section whose heading matches `name` (case-insensitive).
    #[must_use]
    pub fn section_by_name(&self, name: &str) -> Option<&AgentsSection> {
        let name_lower = name.to_lowercase();
        self.sections
            .iter()
            .find(|s| s.heading.to_lowercase() == name_lower)
    }

    /// Collect all sections at a given depth.
    #[must_use]
    pub fn sections_at_depth(&self, depth: usize) -> Vec<&AgentsSection> {
        self.sections.iter().filter(|s| s.depth == depth).collect()
    }

    /// Return a concatenated string of all section bodies whose heading
    /// contains `keyword` (case-insensitive). Useful for extracting all
    /// convention-related content.
    #[must_use]
    pub fn bodies_matching(&self, keyword: &str) -> String {
        let kw = keyword.to_lowercase();
        self.sections
            .iter()
            .filter(|s| s.heading.to_lowercase().contains(&kw))
            .map(|s| s.body.as_str())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// True when no meaningful content was parsed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }
}

/// Try to parse a line as an ATX heading. Returns `(depth, heading_text)`.
fn parse_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let depth = trimmed.chars().take_while(|&c| c == '#').count();
    if depth > 6 {
        return None;
    }
    let rest = trimmed[depth..].trim();
    // ATX headings require a space after the `#` characters (unless empty heading).
    if rest.is_empty() {
        return Some((depth, String::new()));
    }
    // Check there's whitespace right after the `#`s.
    if !trimmed.as_bytes().get(depth).is_some_and(|&b| b == b' ' || b == b'\t') {
        return None;
    }
    // Strip optional trailing `#` markers.
    let heading = rest.trim_end_matches('#').trim().to_string();
    Some((depth, heading))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_AGENTS_MD: &str = "\
# AGENTS.md

## Coding Conventions
- Use snake_case for variables
- Use thiserror for error types
- No unwrap() in library crates

## Module Organization
- One module per file
- Re-export public types from lib.rs

## Testing
- Every public function needs a test
- Use proptest for property-based tests

### Test Naming
- test_<function>_<scenario>
";

    #[test]
    fn parse_agents_md_extracts_sections() {
        let parsed = AgentsMd::parse(SAMPLE_AGENTS_MD);
        assert!(!parsed.is_empty());
        // Top-level heading + 3 h2 sections + 1 h3 section = 5
        assert_eq!(parsed.sections.len(), 5);
        assert_eq!(parsed.sections[0].heading, "AGENTS.md");
        assert_eq!(parsed.sections[0].depth, 1);
        assert_eq!(parsed.sections[1].heading, "Coding Conventions");
        assert_eq!(parsed.sections[1].depth, 2);
    }

    #[test]
    fn section_by_name_case_insensitive() {
        let parsed = AgentsMd::parse(SAMPLE_AGENTS_MD);
        let sec = parsed.section_by_name("coding conventions").unwrap();
        assert_eq!(sec.heading, "Coding Conventions");
        assert!(sec.body.contains("snake_case"));
    }

    #[test]
    fn bodies_matching_collects_relevant_content() {
        let parsed = AgentsMd::parse(SAMPLE_AGENTS_MD);
        let test_content = parsed.bodies_matching("test");
        assert!(test_content.contains("public function"));
        assert!(test_content.contains("test_<function>"));
    }

    #[test]
    fn empty_input_yields_empty_result() {
        let parsed = AgentsMd::parse("");
        assert!(parsed.is_empty());
    }

    #[test]
    fn preamble_text_before_first_heading() {
        let md = "Some preamble text.\n\n# Title\nBody here.";
        let parsed = AgentsMd::parse(md);
        assert_eq!(parsed.sections.len(), 2);
        assert_eq!(parsed.sections[0].heading, "preamble");
        assert!(parsed.sections[0].body.contains("preamble text"));
    }

    #[test]
    fn sections_at_depth_filters_correctly() {
        let parsed = AgentsMd::parse(SAMPLE_AGENTS_MD);
        let h2 = parsed.sections_at_depth(2);
        assert_eq!(h2.len(), 3);
        let h3 = parsed.sections_at_depth(3);
        assert_eq!(h3.len(), 1);
        assert_eq!(h3[0].heading, "Test Naming");
    }

    #[test]
    fn raw_content_preserved() {
        let parsed = AgentsMd::parse(SAMPLE_AGENTS_MD);
        assert_eq!(parsed.raw, SAMPLE_AGENTS_MD);
    }

    #[test]
    fn headings_with_trailing_hashes_stripped() {
        let md = "## Conventions ##\nSome content.";
        let parsed = AgentsMd::parse(md);
        assert_eq!(parsed.sections[0].heading, "Conventions");
    }
}
