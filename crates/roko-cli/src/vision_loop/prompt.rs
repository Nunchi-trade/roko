//! Per-file-type prompt templates for vision evaluation.

use super::IterationRecord;

/// Build the system prompt for the vision evaluator.
pub fn system_prompt(goal: &str, ext: &str, history: &[IterationRecord]) -> String {
    let framework_hint = framework_hint(ext);
    let history_section = if history.is_empty() {
        String::new()
    } else {
        let mut lines = Vec::with_capacity(history.len() + 1);
        lines.push("Previous iterations:".to_string());
        for record in history {
            lines.push(format!(
                "  Iteration {}: {:.1}/10 — {}",
                record.iteration, record.score, record.notes
            ));
        }
        lines.join("\n")
    };

    format!(
        "You are a senior UI engineer iterating on a component. \
         Evaluate the screenshot against the goal and produce improved code.\n\
         \n\
         Goal: {goal}\n\
         File type: .{ext} ({framework_hint})\n\
         Constraints: Only modify THIS file. No new dependencies. Return the COMPLETE file.\n\
         \n\
         {history_section}\n\
         \n\
         Respond with ONLY valid JSON (no markdown fences, no commentary):\n\
         {{\"score\": <1-10>, \"notes\": \"...\", \"improved_code\": \"...full file...\"}}"
    )
}

/// Build the user message text (code portion — image is sent as a separate content block).
pub fn user_code_block(current_code: &str) -> String {
    format!("Current source code:\n```\n{current_code}\n```")
}

/// Build a retry hint after regression detection.
pub fn regression_hint(regressed_iteration: u32) -> String {
    format!(
        "\nIMPORTANT: The approach in iteration {regressed_iteration} caused a regression. \
         Avoid that approach. Instead, build on the best-scoring iteration."
    )
}

/// Map file extension to a framework hint for the prompt.
fn framework_hint(ext: &str) -> &'static str {
    match ext {
        "tsx" | "jsx" => "React / JSX component",
        "vue" => "Vue single-file component",
        "svelte" => "Svelte component",
        "html" => "plain HTML page",
        "css" | "scss" | "less" => "stylesheet",
        "astro" => "Astro component",
        _ => "web component",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn system_prompt_includes_goal_and_ext() {
        let prompt = system_prompt("clean landing page", "tsx", &[]);
        assert!(prompt.contains("clean landing page"));
        assert!(prompt.contains(".tsx"));
        assert!(prompt.contains("React / JSX component"));
        assert!(prompt.contains("\"score\""));
    }

    #[test]
    fn system_prompt_includes_history() {
        let history = vec![
            IterationRecord {
                iteration: 1,
                score: 4.0,
                notes: "layout misaligned".into(),
                timestamp: Utc::now(),
            },
            IterationRecord {
                iteration: 2,
                score: 6.0,
                notes: "layout fixed, colors off".into(),
                timestamp: Utc::now(),
            },
        ];
        let prompt = system_prompt("make it pretty", "vue", &history);
        assert!(prompt.contains("Iteration 1: 4.0/10"));
        assert!(prompt.contains("Iteration 2: 6.0/10"));
        assert!(prompt.contains("Vue single-file component"));
    }

    #[test]
    fn framework_hints_cover_common_extensions() {
        assert_eq!(framework_hint("tsx"), "React / JSX component");
        assert_eq!(framework_hint("jsx"), "React / JSX component");
        assert_eq!(framework_hint("vue"), "Vue single-file component");
        assert_eq!(framework_hint("svelte"), "Svelte component");
        assert_eq!(framework_hint("html"), "plain HTML page");
        assert_eq!(framework_hint("rs"), "web component"); // fallback
    }

    #[test]
    fn regression_hint_references_iteration() {
        let hint = regression_hint(3);
        assert!(hint.contains("iteration 3"));
        assert!(hint.contains("regression"));
    }
}
