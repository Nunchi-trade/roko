//! Inline-rendered wrapper for `roko run`.
//!
//! Wraps [`run_once`] with the inline terminal engine to produce structured
//! clack-style output. Falls back to plain text when stdout is not a TTY.
//!
//! This is the same set of primitives used by `roko chat`, ensuring visual
//! consistency across all commands.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::config::Config;
use crate::inline::markdown;
use crate::inline::primitives::{RunBlockData, ToolCallInfo};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::{InlineTerminal, should_use_inline};
use crate::run::{RunReport, run_once};
use crate::tui::Theme;
use roko_core::StateHub;

/// Run the universal loop with inline terminal output.
///
/// Displays:
/// - Agent header with role
/// - Gate verdicts with ✔/✖
/// - Agent output rendered as markdown
/// - Cost summary
///
/// Falls back to the existing plain `run_once` output when not a TTY.
pub async fn run_once_inline(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
    external_hub: Option<&StateHub>,
) -> Result<RunReport> {
    if !should_use_inline() {
        // Fallback: run normally, print text summary
        let report = run_once(workdir, config, prompt_text, external_hub).await?;
        print_plain_report(&report);
        return Ok(report);
    }

    let mut term = InlineTerminal::new()
        .map_err(|e| anyhow::anyhow!("init inline terminal: {e}"))?;
    let theme = *term.theme();

    // Header
    term.push_lines(&[styled::section_start(
        &theme,
        "run",
        &config.prompt.role,
        Some(&config.agent.command),
    )])?;

    // Spinner while running
    let start = Instant::now();
    term.push_lines(&[styled::spinner_line(
        &theme,
        0,
        "executing...",
        0.0,
    )])?;

    // Execute the universal loop
    let report = run_once(workdir, config, prompt_text, external_hub).await?;

    let elapsed = start.elapsed().as_secs_f64();

    // Build the RunBlock from the report
    let block = RunBlockData {
        agent_name: config.agent.command.clone(),
        identity: None,
        attested: false,
        predicted_cost: None,
        predicted_time: None,
        predicted_route: None,
        gate_verdicts: report.gate_verdicts.clone(),
        knowledge_loaded: None,
        actual_cost: None, // TODO: wire from report when available
        actual_route: None,
        actual_time: Some(elapsed),
        tool_calls: Vec::new(),
        deposited_count: 0,
        deposited_path: None,
        chain_block: None,
    };

    // Push the structured summary
    term.push_lines(&block.to_lines(&theme))?;

    // Push agent output as markdown
    if let Some(ref text) = report.output_text {
        term.push_blank()?;
        let md_lines = markdown::render_markdown_with_bar(text, &theme);
        term.push_lines(&md_lines)?;
    }

    // Result line
    term.push_blank()?;
    let result_line = if report.overall_success() {
        Line::from(vec![
            Span::styled(symbols::PASS.to_string(), theme.success()),
            Span::raw(" "),
            Span::styled(
                format!("completed in {elapsed:.1}s"),
                Style::default().fg(Theme::SAGE),
            ),
        ])
    } else {
        let failed_gates: Vec<&str> = report
            .gate_verdicts
            .iter()
            .filter(|(_, p)| !p)
            .map(|(n, _)| n.as_str())
            .collect();
        Line::from(vec![
            Span::styled(symbols::FAIL.to_string(), theme.danger()),
            Span::raw(" "),
            Span::styled(
                format!(
                    "failed: {}",
                    if failed_gates.is_empty() {
                        "agent error".to_string()
                    } else {
                        failed_gates.join(", ")
                    }
                ),
                Style::default().fg(Theme::EMBER).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    term.push_lines(&[result_line])?;
    term.push_blank()?;

    drop(term); // restore terminal
    Ok(report)
}

/// Plain text fallback for non-TTY environments.
fn print_plain_report(report: &RunReport) {
    if report.overall_success() {
        println!("{} completed successfully", symbols::PASS);
    } else {
        println!("{} failed", symbols::FAIL);
    }
    for (gate, passed) in &report.gate_verdicts {
        let sym = if *passed { symbols::PASS } else { symbols::FAIL };
        println!("  {sym} {gate}");
    }
    if let Some(ref text) = report.output_text {
        println!();
        // Truncate for plain output
        for line in text.lines().take(20) {
            println!("  {line}");
        }
        let total = text.lines().count();
        if total > 20 {
            println!("  ... +{} more lines", total - 20);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_report_success() {
        let report = RunReport {
            episode_id: "abc".into(),
            prompt_id: "def".into(),
            agent_output_id: "ghi".into(),
            agent_success: true,
            gate_verdicts: vec![("compile".into(), true), ("test".into(), true)],
            total_signals: 5,
            output_text: Some("Hello world".into()),
        };
        // Just verify it doesn't panic
        print_plain_report(&report);
    }

    #[test]
    fn plain_report_failure() {
        let report = RunReport {
            episode_id: "abc".into(),
            prompt_id: "def".into(),
            agent_output_id: "ghi".into(),
            agent_success: false,
            gate_verdicts: vec![("compile".into(), true), ("test".into(), false)],
            total_signals: 3,
            output_text: None,
        };
        print_plain_report(&report);
    }
}
