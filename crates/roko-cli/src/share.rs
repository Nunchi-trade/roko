//! Share run transcripts to persistent external services.
//!
//! Supports:
//! - **GitHub Gist** (default) — uploads markdown via `gh gist create`, returns public URL
//! - **Local file** — saves to `.roko/shared/` for local serve
//!
//! The transcript is rendered as markdown so it displays natively on
//! GitHub, in Slack previews, and in any markdown viewer.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::inline::symbols;
use crate::run::RunReport;

/// A shared transcript result.
#[derive(Debug, Clone)]
pub struct ShareResult {
    /// The public URL where the transcript can be viewed.
    pub url: String,
    /// The backend used ("gist", "local", "file").
    pub backend: String,
    /// Local file path where the transcript was saved.
    pub local_path: String,
}

/// Share a run report. Tries GitHub Gist first, falls back to local file.
pub fn share_run(
    workdir: &Path,
    report: &RunReport,
    prompt: &str,
    agent: &str,
    role: &str,
    elapsed_s: f64,
) -> Result<ShareResult> {
    let run_id = &report.episode_id[..8.min(report.episode_id.len())];
    let markdown = render_markdown_transcript(report, prompt, agent, role, run_id, elapsed_s);

    // Save locally first (always)
    let shared_dir = workdir.join(".roko").join("shared");
    let _ = std::fs::create_dir_all(&shared_dir);
    let md_path = shared_dir.join(format!("{run_id}.md"));
    std::fs::write(&md_path, &markdown)
        .with_context(|| format!("write transcript to {}", md_path.display()))?;

    // Try GitHub Gist upload
    match upload_gist(run_id, &markdown) {
        Ok(url) => Ok(ShareResult {
            url,
            backend: "gist".into(),
            local_path: md_path.display().to_string(),
        }),
        Err(gist_err) => {
            // Gist failed — return local path
            tracing::debug!("gist upload failed: {gist_err}");
            Ok(ShareResult {
                url: format!("file://{}", md_path.display()),
                backend: "local".into(),
                local_path: md_path.display().to_string(),
            })
        }
    }
}

/// Upload markdown content as a public GitHub Gist via `gh` CLI.
fn upload_gist(run_id: &str, content: &str) -> Result<String> {
    // Check gh is available
    let gh_check = Command::new("gh").arg("--version").output();
    if gh_check.is_err() || !gh_check.unwrap().status.success() {
        bail!("gh CLI not found — install from https://cli.github.com/");
    }

    // Write to a temp file (gh gist create reads from file)
    let filename = format!("roko-run-{run_id}.md");
    let tmp_dir = std::env::temp_dir().join("roko-share");
    let _ = std::fs::create_dir_all(&tmp_dir);
    let tmp_path = tmp_dir.join(&filename);
    std::fs::write(&tmp_path, content).with_context(|| "write temp gist file")?;

    // Create public gist
    let output = Command::new("gh")
        .args([
            "gist",
            "create",
            "--public",
            "--desc",
            &format!("roko run transcript {run_id}"),
            tmp_path.to_str().unwrap_or(&filename),
        ])
        .output()
        .with_context(|| "run gh gist create")?;

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("gh gist create failed: {stderr}");
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        bail!("gh gist create returned empty URL");
    }
    Ok(url)
}

/// Render a run report as a markdown document.
fn render_markdown_transcript(
    report: &RunReport,
    prompt: &str,
    agent: &str,
    role: &str,
    run_id: &str,
    elapsed_s: f64,
) -> String {
    let mut md = String::with_capacity(2048);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Header
    let start = symbols::START;
    md.push_str(&format!("# {start} roko run · `{run_id}`\n\n"));
    md.push_str(&format!(
        "**agent** {agent} · **role** {role} · **{now}**\n\n"
    ));

    // Prompt
    md.push_str("## Prompt\n\n");
    md.push_str(&format!("```\n{prompt}\n```\n\n"));

    // Summary
    md.push_str("## Summary\n\n");
    md.push_str(&format!("| | |\n|---|---|\n"));
    md.push_str(&format!(
        "| Result | {} {} |\n",
        if report.overall_success() {
            symbols::PASS
        } else {
            symbols::FAIL
        },
        if report.overall_success() {
            "completed"
        } else {
            "failed"
        },
    ));
    md.push_str(&format!("| Duration | {elapsed_s:.1}s |\n"));
    md.push_str(&format!("| Episode | `{}` |\n", report.episode_id));
    md.push_str(&format!("| Signals | {} |\n\n", report.total_signals));

    // Gates
    if !report.gate_verdicts.is_empty() {
        md.push_str("## Gates\n\n");
        md.push_str("| Gate | Result |\n|---|---|\n");
        for (name, passed) in &report.gate_verdicts {
            let icon = if *passed {
                symbols::PASS
            } else {
                symbols::FAIL
            };
            md.push_str(&format!("| {name} | {icon} |\n"));
        }
        md.push_str("\n");
    }

    // Output
    if let Some(ref text) = report.output_text {
        md.push_str("## Agent Output\n\n");
        md.push_str(text);
        md.push_str("\n\n");
    }

    // Footer
    md.push_str("---\n\n");
    md.push_str("*Generated by [roko](https://github.com/nunchi/roko) agent runtime*\n");

    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> RunReport {
        RunReport {
            episode_id: "abc12345def67890".into(),
            prompt_id: "p1".into(),
            agent_output_id: "o1".into(),
            agent_success: true,
            gate_verdicts: vec![("compile".into(), true), ("test".into(), true)],
            total_signals: 7,
            output_text: Some("The analysis shows **strong results**.".into()),
            usage: None,
        }
    }

    #[test]
    fn markdown_transcript_renders() {
        let report = sample_report();
        let md = render_markdown_transcript(
            &report,
            "Summarize Q3",
            "researcher",
            "analyst",
            "abc12345",
            9.8,
        );
        assert!(md.contains("roko run"));
        assert!(md.contains("Summarize Q3"));
        assert!(md.contains("compile"));
        assert!(md.contains(symbols::PASS));
        assert!(md.contains("strong results"));
    }

    #[test]
    fn markdown_transcript_failed_run() {
        let report = RunReport {
            episode_id: "fail1234".into(),
            prompt_id: "p1".into(),
            agent_output_id: "o1".into(),
            agent_success: false,
            gate_verdicts: vec![("test".into(), false)],
            total_signals: 3,
            output_text: None,
            usage: None,
        };
        let md =
            render_markdown_transcript(&report, "Fix bug", "fixer", "implementer", "fail1234", 5.2);
        assert!(md.contains(symbols::FAIL));
        assert!(md.contains("failed"));
    }
}
