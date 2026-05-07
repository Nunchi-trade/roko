//! `roko develop` — plan-first development workflow.
//!
//! Thin wrapper over `do_cmd` that forces `--plan`, shows a plan approval
//! screen before executing, and auto-launches the TUI dashboard if running
//! in a TTY.

use crate::*;
use roko_cli::runner::plan_loader;
use roko_cli::task_parser::TaskDef;
use std::io::IsTerminal;
use std::path::PathBuf;

/// Main entry point for `roko develop`.
pub(crate) async fn cmd_develop(
    cli: &Cli,
    workdir: Option<PathBuf>,
    prompt_args: Vec<String>,
    dry_run: bool,
    yes: bool,
    continue_work: bool,
    provider: Option<String>,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));

    // --continue: delegate to `roko do --continue`
    if continue_work {
        return commands::do_cmd::cmd_do(
            cli,
            Some(workdir),
            vec![],
            true, // --plan
            None,
            false,
            yes,
            false,
            false,
            Some(None), // --continue with no specific ID
            false,
            provider,
            Vec::new(),
        )
        .await;
    }

    let prompt = prompt_args.join(" ").trim().to_string();
    if prompt.is_empty() {
        eprintln!("usage: roko develop \"description of what to build\"");
        return Ok(EXIT_FAILURE);
    }

    // --dry-run: show plan preview without executing.
    if dry_run {
        return commands::do_cmd::cmd_do(
            cli,
            Some(workdir),
            prompt_args,
            true, // --plan (forced)
            None,
            true, // --dry-run
            yes,
            false,
            false,
            None,
            false,
            provider,
            Vec::new(),
        )
        .await;
    }

    // --yes: skip approval, go straight to plan + execute.
    if yes {
        let code = commands::do_cmd::cmd_do(
            cli,
            Some(workdir),
            prompt_args,
            true, // --plan (forced)
            None,
            false,
            true, // --yes
            false,
            false,
            None,
            false,
            provider,
            Vec::new(),
        )
        .await?;
        if code == EXIT_SUCCESS && std::io::stderr().is_terminal() {
            hint_tui_dashboard();
        }
        return Ok(code);
    }

    // Interactive mode: first dry-run to show the plan, then ask for approval.
    eprintln!("\u{25b8} roko develop: classifying and previewing plan...");
    eprintln!();

    // Show the plan preview via do_cmd dry-run.
    let _preview = commands::do_cmd::cmd_do(
        cli,
        Some(workdir.clone()),
        prompt_args.clone(),
        true, // --plan
        None,
        true, // dry-run (preview only)
        false,
        false,
        false,
        None,
        false,
        provider.clone(),
        Vec::new(),
    )
    .await?;

    // Check if existing plans are on disk to show task table.
    let plans_dir = roko_cli::plan::plans_dir(&workdir);
    if plans_dir.is_dir() {
        if let Ok(plans) = plan_loader::load_plans(&plans_dir) {
            if !plans.is_empty() && !show_plan_approval(&plans) {
                eprintln!("\u{25b8} Aborted.");
                return Ok(EXIT_SUCCESS);
            }
        }
    }

    // User approved (or no plans to show) — execute.
    eprintln!("\u{25b8} Executing plan...");
    let code = commands::do_cmd::cmd_do(
        cli,
        Some(workdir),
        prompt_args,
        true, // --plan
        None,
        false, // NOT dry-run
        true,  // yes (already approved above)
        false,
        false,
        None,
        false,
        provider,
        Vec::new(),
    )
    .await?;

    if code == EXIT_SUCCESS && std::io::stderr().is_terminal() {
        hint_tui_dashboard();
    }
    Ok(code)
}

/// Show a table of tasks from loaded plans for user approval.
/// Returns true if approved, false if user quit.
fn show_plan_approval(plans: &[plan_loader::Plan]) -> bool {
    let is_tty = std::io::stdin().is_terminal();
    if !is_tty {
        return true;
    }

    let total_tasks: usize = plans.iter().map(|p| p.tasks.tasks.len()).sum();
    eprintln!();
    eprintln!(
        "\u{2500}\u{2500}\u{2500} Plan: {total_tasks} task(s) across {} plan(s) \u{2500}\u{2500}\u{2500}",
        plans.len()
    );
    eprintln!();
    eprintln!("  {:<8} {:<14} {:<12} {}", "ID", "TIER", "STATUS", "TITLE");
    eprintln!(
        "  {:\u{2500}<8} {:\u{2500}<14} {:\u{2500}<12} {:\u{2500}<40}",
        "", "", "", ""
    );

    for plan in plans {
        for task in &plan.tasks.tasks {
            print_task_row(task);
        }
    }
    eprintln!();

    loop {
        eprint!("  [Enter] Execute  |  [q] Quit  > ");
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return false;
        }
        let input = input.trim().to_lowercase();
        match input.as_str() {
            "" | "y" | "yes" => return true,
            "q" | "quit" | "n" | "no" => return false,
            _ => eprintln!("  Enter to execute, q to quit."),
        }
    }
}

fn print_task_row(task: &TaskDef) {
    let title = if task.title.len() > 50 {
        format!("{}...", &task.title[..47])
    } else {
        task.title.clone()
    };
    eprintln!(
        "  {:<8} {:<14} {:<12} {title}",
        task.id, task.tier, task.status
    );
}

fn hint_tui_dashboard() {
    eprintln!();
    eprintln!("\u{25b8} Tip: run `roko dashboard` to watch progress in the TUI.");
}
