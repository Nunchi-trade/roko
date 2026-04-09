//! Manual test harness for [`roko_orchestrator::discover_plans`].
//!
//! Run against a real plans directory:
//!
//! ```bash
//! cargo run -p roko-orchestrator --example discover -- /Users/will/dev/uniswap/bardo/.mori/plans
//! ```
//!
//! Falls back to `./.mori/plans` (or `./plans`) relative to the current
//! working directory when no argument is supplied.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use roko_orchestrator::{PlanInfo, discover_plans};

fn main() -> ExitCode {
    let arg = env::args().nth(1);
    let path = arg.map_or_else(
        || {
            let candidates = [".mori/plans", "plans", ".roko/plans"];
            candidates
                .into_iter()
                .map(PathBuf::from)
                .find(|p| p.exists())
                .unwrap_or_else(|| PathBuf::from(".mori/plans"))
        },
        PathBuf::from,
    );

    println!("Scanning {}", path.display());

    let plans = match discover_plans(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };

    if plans.is_empty() {
        println!("  (no plans found)");
        return ExitCode::SUCCESS;
    }

    println!("Discovered {} plan(s):\n", plans.len());
    for PlanInfo {
        num,
        base,
        frontmatter,
        ..
    } in &plans
    {
        let (priority, depends_on, crates) = frontmatter.as_ref().map_or_else(
            || ("-".to_string(), 0_usize, 0_usize),
            |fm| {
                (
                    fm.priority.map_or_else(|| "-".into(), |p| p.to_string()),
                    fm.depends_on.len(),
                    fm.crates_touched.len(),
                )
            },
        );
        println!(
            "  [{num:>4}] {base:<42} priority={priority:>3} deps={depends_on:>2} crates={crates:>2}"
        );
    }

    ExitCode::SUCCESS
}
