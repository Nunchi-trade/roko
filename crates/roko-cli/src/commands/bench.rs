//! bench command handlers.
#![allow(unused_imports)]

use crate::*;

pub(crate) async fn cmd_bench(cli: &Cli, cmd: BenchCmd) -> Result<i32> {
    match cmd {
        BenchCmd::Demo { real, workdir } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            roko_cli::bench_demo::run_bench_demo(&workdir, real).await?;
            Ok(EXIT_SUCCESS)
        }
        BenchCmd::Swe {
            dataset,
            batch_size,
            offset,
            agent_mode,
            predictions,
            agent_command,
            report,
            export_predictions,
            no_learning,
            keep_workdirs,
            workdir,
        } => {
            let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
            let report = roko_cli::bench::run_swe_bench(roko_cli::bench::SweBenchOptions {
                workdir,
                dataset,
                batch_size,
                offset,
                agent_mode,
                predictions,
                agent_command,
                report,
                export_predictions,
                record_learning: !no_learning,
                keep_workdirs,
            })
            .await?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                println!("{}", report.render_text());
                println!();
                println!(
                    "note: this is fast proxy scoring, not official SWE-bench Docker scoring."
                );
            }
            Ok(EXIT_SUCCESS)
        }
    }
}
