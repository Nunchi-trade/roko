//! Bounded gate-failure feedback for retry prompt composition.

/// Maximum diagnostic lines retained from a previous gate failure.
pub const MAX_GATE_FEEDBACK_LINES: usize = 24;
const MAX_GATE_FEEDBACK_LINE_CHARS: usize = 240;

/// Structured gate failure feedback rendered for retry prompts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateFeedback {
    /// Gate rung that failed.
    pub rung: u32,
    /// One-line retry summary.
    pub summary: String,
    /// Bounded diagnostic lines selected from raw gate output.
    pub diagnostics: Vec<String>,
}

impl GateFeedback {
    /// Parse raw gate output into bounded, actionable retry feedback.
    #[must_use]
    pub fn from_raw(raw_output: &str, rung: u32) -> Option<Self> {
        let trimmed = raw_output.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut diagnostics = Vec::new();
        for line in trimmed
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let lower = line.to_ascii_lowercase();
            if lower.contains("error")
                || lower.contains("failed")
                || lower.contains("warning")
                || line.contains("-->")
            {
                diagnostics.push(truncate_line(line));
            }
            if diagnostics.len() >= MAX_GATE_FEEDBACK_LINES {
                break;
            }
        }
        if diagnostics.is_empty() {
            diagnostics.extend(
                trimmed
                    .lines()
                    .take(MAX_GATE_FEEDBACK_LINES)
                    .map(str::trim)
                    .map(truncate_line),
            );
        }

        Some(Self {
            rung,
            summary: "The previous attempt failed verification. Fix the listed issues first and avoid broad rewrites.".to_string(),
            diagnostics,
        })
    }

    /// Render feedback as a prompt section.
    #[must_use]
    pub fn render_prompt_section(&self) -> String {
        let mut feedback = format!(
            "## Previous Verify Failure\n\nGate rung: {}\nSummary: {}\n\n### Actionable diagnostics\n",
            self.rung, self.summary
        );
        for line in &self.diagnostics {
            feedback.push_str("- ");
            feedback.push_str(line);
            feedback.push('\n');
        }
        feedback
    }
}

fn truncate_line(line: &str) -> String {
    line.chars().take(MAX_GATE_FEEDBACK_LINE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_feedback_selects_error_warning_and_location_lines() {
        let raw =
            "noise\nerror[E0308]: mismatched types\n --> src/lib.rs:9:1\nwarning: unused import\n";
        let feedback = GateFeedback::from_raw(raw, 2).unwrap();

        assert_eq!(feedback.rung, 2);
        assert_eq!(feedback.diagnostics.len(), 3);
        assert!(feedback.diagnostics[0].contains("error[E0308]"));
        assert!(feedback.diagnostics[1].contains("src/lib.rs:9:1"));
        assert!(feedback.diagnostics[2].contains("warning: unused import"));
        assert!(!feedback.diagnostics.iter().any(|line| line == "noise"));
    }

    #[test]
    fn gate_feedback_is_bounded() {
        let raw = (0..100)
            .map(|idx| format!("error: issue {idx} {}", "x".repeat(300)))
            .collect::<Vec<_>>()
            .join("\n");
        let feedback = GateFeedback::from_raw(&raw, 1).unwrap();

        assert_eq!(feedback.diagnostics.len(), MAX_GATE_FEEDBACK_LINES);
        assert!(
            feedback
                .diagnostics
                .iter()
                .all(|line| line.chars().count() <= MAX_GATE_FEEDBACK_LINE_CHARS)
        );
    }
}
