//! Inline-rendered wrapper for `roko run`.
//!
//! Wraps [`run_once`] with the inline terminal engine to produce structured
//! clack-style output. Falls back to plain text when stdout is not a TTY.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::text::{Line, Span};

use crate::config::Config;
use crate::inline::markdown;
use crate::inline::plaintext;
use crate::inline::primitives::{GateBlockData, RunBlockData};
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::{InlineTerminal, should_use_inline};
use crate::run::{RunReport, run_once};
use crate::state_hub::StateHub;
use crate::tui::Theme;

/// Run the universal loop with inline terminal output.
///
/// Displays: agent header, gate pipeline with per-rung results,
/// markdown-rendered output, and session summary.
///
/// Falls back to plain text when not a TTY.
pub async fn run_once_inline(
    workdir: &Path,
    config: &Config,
    prompt_text: &str,
    external_hub: Option<&StateHub>,
) -> Result<RunReport> {
    if !should_use_inline() {
        let report = run_once(workdir, config, prompt_text, external_hub).await?;
        print_plain_report(&report, config);
        return Ok(report);
    }

    let mut term =
        InlineTerminal::new().map_err(|e| anyhow::anyhow!("init inline terminal: {e}"))?;
    let theme = *term.theme();

    // Header
    let gate_count = config.gates.len();
    let gate_detail = if gate_count > 0 {
        Some(format!(
            "{gate_count} gate{}",
            if gate_count == 1 { "" } else { "s" }
        ))
    } else {
        None
    };
    term.push_lines_revealed(
        &[styled::section_start(
            &theme,
            "run",
            &config.prompt.role,
            gate_detail.as_deref(),
        )],
        Duration::from_millis(30),
    )?;

    // Show prompt (truncated)
    let prompt_preview: String = prompt_text.chars().take(80).collect();
    term.push_lines(&[styled::continuation(
        &theme,
        "prompt",
        &prompt_preview,
        if prompt_text.len() > 80 {
            Some("...")
        } else {
            None
        },
    )])?;

    // Spinner while executing
    let start = Instant::now();
    term.push_lines(&[styled::spinner_line(
        &theme,
        0,
        &format!("dispatching to {}...", config.agent.command),
        0.0,
    )])?;

    // Execute
    let report = run_once(workdir, config, prompt_text, external_hub).await?;
    let elapsed = start.elapsed().as_secs_f64();

    // Gate results as a proper GateBlock
    if !report.gate_verdicts.is_empty() {
        let gate_block = GateBlockData::from_verdicts(&report.gate_verdicts);
        term.push_blank()?;
        term.push_lines_revealed(&gate_block.to_lines(&theme), Duration::from_millis(40))?;
    }

    // Agent output as markdown
    if let Some(ref text) = report.output_text {
        term.push_blank()?;
        let md_lines = markdown::render_markdown_with_bar(text, &theme);
        if md_lines.len() > 30 {
            // Truncate long output, show first 25 + count
            term.push_lines_revealed(&md_lines[..25], Duration::from_millis(20))?;
            term.push_lines(&[styled::continuation(
                &theme,
                "",
                &format!("... +{} more lines", md_lines.len() - 25),
                None,
            )])?;
        } else {
            term.push_lines_revealed(&md_lines, Duration::from_millis(20))?;
        }
    }

    // Result line
    term.push_blank()?;
    let result_line = if report.overall_success() {
        Line::from(vec![
            Span::styled(symbols::PASS.to_string(), theme.success()),
            Span::raw(" "),
            Span::styled(
                format!(
                    "completed in {elapsed:.1}s  {}  episode {}",
                    symbols::SEP,
                    &report.episode_id[..8.min(report.episode_id.len())],
                ),
                theme.success(),
            ),
        ])
    } else {
        let failed: Vec<&str> = report
            .gate_verdicts
            .iter()
            .filter(|(_, p)| !p)
            .map(|(n, _)| n.as_str())
            .collect();
        let reason = if failed.is_empty() {
            "agent error".to_string()
        } else {
            format!(
                "gate{} failed: {}",
                if failed.len() == 1 { "" } else { "s" },
                failed.join(", ")
            )
        };
        Line::from(vec![
            Span::styled(symbols::FAIL.to_string(), theme.danger()),
            Span::raw(" "),
            Span::styled(reason, theme.danger()),
        ])
    };
    term.push_lines(&[result_line])?;
    term.push_blank()?;

    drop(term);
    Ok(report)
}

/// Plain text fallback for non-TTY environments.
fn print_plain_report(report: &RunReport, config: &Config) {
    // Build the same primitives, render as plain text
    let theme = Theme::dark();

    let block = RunBlockData {
        agent_name: config.agent.command.clone(),
        gate_verdicts: report.gate_verdicts.clone(),
        actual_time: None,
        ..Default::default()
    };
    plaintext::print_plain(&block.to_lines(&theme));

    if let Some(ref text) = report.output_text {
        println!();
        for line in text.lines().take(20) {
            println!("  {line}");
        }
        let total = text.lines().count();
        if total > 20 {
            println!("  ... +{} more lines", total - 20);
        }
    }

    println!();
    if report.overall_success() {
        println!("{} completed  episode={}", symbols::PASS, report.episode_id);
    } else {
        println!("{} failed  episode={}", symbols::FAIL, report.episode_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report(success: bool) -> RunReport {
        RunReport {
            episode_id: "abc12345".into(),
            prompt_id: "def".into(),
            agent_output_id: "ghi".into(),
            agent_success: success,
            gate_verdicts: vec![("compile".into(), true), ("test".into(), success)],
            total_signals: 5,
            output_text: Some("Hello **world**".into()),
        }
    }

    #[test]
    fn plain_report_success() {
        let config = Config::default();
        print_plain_report(&make_report(true), &config);
    }

    #[test]
    fn plain_report_failure() {
        let config = Config::default();
        print_plain_report(&make_report(false), &config);
    }

    #[test]
    fn plain_report_uses_primitives() {
        // Verify that the plain fallback actually renders RunBlock lines
        let theme = Theme::dark();
        let block = RunBlockData {
            agent_name: "test-agent".into(),
            gate_verdicts: vec![("compile".into(), true)],
            ..Default::default()
        };
        let lines = block.to_lines(&theme);
        let text = crate::inline::plaintext::lines_to_plain(&lines);
        assert!(text.contains("test-agent"));
        assert!(text.contains(symbols::PASS));
    }
}
