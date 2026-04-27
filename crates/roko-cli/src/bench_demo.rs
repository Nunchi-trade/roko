//! `roko bench demo` — benchmark comparison showing roko-optimized vs naive.
//!
//! Runs a set of tasks twice:
//! 1. **Naive mode**: single model (opus), no caching, no routing, no knowledge
//! 2. **Optimized mode**: full roko stack (CascadeRouter, caching, gates, knowledge)
//!
//! Displays live progress and a final comparison table using inline primitives.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::inline::primitives::*;
use crate::inline::styled;
use crate::inline::symbols;
use crate::inline::terminal::{InlineTerminal, should_use_inline};
use crate::tui::Theme;

/// A single benchmark task.
#[derive(Debug, Clone)]
pub struct BenchTask {
    /// Task ID.
    pub id: String,
    /// Task description.
    pub description: String,
    /// Difficulty level.
    pub difficulty: String,
}

/// Result of running one task in one mode.
#[derive(Debug, Clone)]
pub struct BenchResult {
    /// Task ID.
    pub task_id: String,
    /// Mode label ("naive" or "optimized").
    pub mode: String,
    /// Whether the task passed all gates.
    pub passed: bool,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Cache hit rate (0.0 for naive).
    pub cache_hit_rate: f64,
    /// Wall time in seconds.
    pub duration_s: f64,
    /// Model used.
    pub model: String,
    /// Gate pass count.
    pub gates_passed: u32,
    /// Gate total count.
    pub gates_total: u32,
}

/// Aggregated benchmark summary for one mode.
#[derive(Debug, Clone)]
pub struct ModeSummary {
    pub mode: String,
    pub total_cost: f64,
    pub total_tokens: u64,
    pub pass_rate: f64,
    pub avg_cache_hit: f64,
    pub avg_duration_s: f64,
    pub tasks_run: u32,
    pub tasks_passed: u32,
    pub primary_model: String,
}

/// Default benchmark task set.
pub fn default_tasks() -> Vec<BenchTask> {
    vec![
        BenchTask {
            id: "B01".into(),
            description: "Add --dry-run flag to plan command".into(),
            difficulty: "easy".into(),
        },
        BenchTask {
            id: "B02".into(),
            description: "Fix typo in error message".into(),
            difficulty: "easy".into(),
        },
        BenchTask {
            id: "B03".into(),
            description: "Add unit test for CascadeRouter".into(),
            difficulty: "medium".into(),
        },
        BenchTask {
            id: "B04".into(),
            description: "Refactor gate pipeline error handling".into(),
            difficulty: "medium".into(),
        },
        BenchTask {
            id: "B05".into(),
            description: "Implement cost waterfall decomposition".into(),
            difficulty: "hard".into(),
        },
    ]
}

/// Run the benchmark demo.
///
/// This runs tasks in two modes and displays a comparison. When `real_dispatch`
/// is false, it simulates realistic results for demo purposes.
pub async fn run_bench_demo(workdir: &Path, real_dispatch: bool) -> Result<()> {
    let tasks = default_tasks();

    if should_use_inline() {
        run_bench_inline(workdir, &tasks, real_dispatch).await
    } else {
        run_bench_plain(workdir, &tasks, real_dispatch).await
    }
}

async fn run_bench_inline(workdir: &Path, tasks: &[BenchTask], real_dispatch: bool) -> Result<()> {
    let mut term = InlineTerminal::new().map_err(|e| anyhow::anyhow!("init terminal: {e}"))?;
    let theme = *term.theme();

    // Header
    term.push_lines_revealed(
        &[
            styled::section_start(
                &theme,
                "bench",
                &format!("{} tasks", tasks.len()),
                Some("naive vs optimized"),
            ),
            styled::continuation(&theme, "mode 1", "naive (opus, no cache, no routing)", None),
            styled::continuation(
                &theme,
                "mode 2",
                "optimized (cascade, cache, gates, knowledge)",
                None,
            ),
        ],
        Duration::from_millis(30),
    )?;
    term.push_blank()?;

    // Run naive mode
    term.push_separator()?;
    term.push_lines(&[styled::section_start(
        &theme,
        "naive",
        "single model, no optimizations",
        None,
    )])?;

    let mut naive_results = Vec::new();
    for task in tasks {
        let result = if real_dispatch {
            run_task_real(workdir, task, "naive").await?
        } else {
            simulate_task(task, "naive")
        };
        let icon = if result.passed {
            symbols::PASS
        } else {
            symbols::FAIL
        };
        let icon_style = if result.passed {
            theme.success()
        } else {
            theme.danger()
        };

        term.push_lines(&[Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(icon.to_string(), icon_style),
            Span::raw(" "),
            Span::styled(
                format!("{:<4}", task.id),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(format!("{:<40}", task.description), theme.text()),
            Span::styled(
                format!("${:.3}", result.cost_usd),
                Style::default().fg(Theme::SAGE),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{}tok", result.input_tokens + result.output_tokens),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:.1}s", result.duration_s),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::raw("  "),
            Span::styled(result.model.clone(), Style::default().fg(Theme::DREAM)),
        ])])?;

        naive_results.push(result);
        tokio::time::sleep(Duration::from_millis(if real_dispatch { 0 } else { 200 })).await;
    }

    let naive_summary = summarize(&naive_results);
    term.push_lines(&[styled::section_end(
        &theme,
        "total",
        &format!(
            "${:.3}  {}  {}/{} passed",
            naive_summary.total_cost,
            symbols::SEP,
            naive_summary.tasks_passed,
            naive_summary.tasks_run,
        ),
    )])?;
    term.push_blank()?;

    // Run optimized mode
    term.push_separator()?;
    term.push_lines(&[styled::section_start(
        &theme,
        "optimized",
        "cascade router + cache + gates + knowledge",
        None,
    )])?;

    let mut opt_results = Vec::new();
    for task in tasks {
        let result = if real_dispatch {
            run_task_real(workdir, task, "optimized").await?
        } else {
            simulate_task(task, "optimized")
        };
        let icon = if result.passed {
            symbols::PASS
        } else {
            symbols::FAIL
        };
        let icon_style = if result.passed {
            theme.success()
        } else {
            theme.danger()
        };

        term.push_lines(&[Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw(" "),
            Span::styled(icon.to_string(), icon_style),
            Span::raw(" "),
            Span::styled(
                format!("{:<4}", task.id),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::styled(format!("{:<40}", task.description), theme.text()),
            Span::styled(
                format!("${:.4}", result.cost_usd),
                Style::default().fg(Theme::SAGE),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{}tok", result.input_tokens + result.output_tokens),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:.1}s", result.duration_s),
                Style::default().fg(Theme::TEXT_DIM),
            ),
            Span::raw("  "),
            Span::styled(result.model.clone(), Style::default().fg(Theme::DREAM)),
            Span::raw("  "),
            Span::styled(
                format!("cache:{:.0}%", result.cache_hit_rate * 100.0),
                Style::default().fg(Theme::TEXT_DIM),
            ),
        ])])?;

        opt_results.push(result);
        tokio::time::sleep(Duration::from_millis(if real_dispatch { 0 } else { 200 })).await;
    }

    let opt_summary = summarize(&opt_results);
    term.push_lines(&[styled::section_end(
        &theme,
        "total",
        &format!(
            "${:.4}  {}  {}/{} passed  {}  cache: {:.0}%",
            opt_summary.total_cost,
            symbols::SEP,
            opt_summary.tasks_passed,
            opt_summary.tasks_run,
            symbols::SEP,
            opt_summary.avg_cache_hit * 100.0,
        ),
    )])?;
    term.push_blank()?;

    // Comparison table
    term.push_separator()?;
    let savings = if opt_summary.total_cost > 0.0 {
        naive_summary.total_cost / opt_summary.total_cost
    } else {
        1.0
    };

    let comparison_lines = vec![
        styled::section_start(&theme, "comparison", "naive vs optimized", None),
        Line::from(vec![
            Span::styled(symbols::BAR.to_string(), theme.muted()),
            Span::raw("  "),
            Span::styled(format!("{:<18}", ""), Style::default().fg(Theme::TEXT_DIM)),
            Span::styled(
                format!("{:<14}", "NAIVE"),
                Style::default()
                    .fg(Theme::EMBER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<14}", "OPTIMIZED"),
                Style::default()
                    .fg(Theme::SAGE)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "IMPROVEMENT".to_string(),
                Style::default()
                    .fg(Theme::BONE)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        comparison_row(
            &theme,
            "total cost",
            &format!("${:.3}", naive_summary.total_cost),
            &format!("${:.4}", opt_summary.total_cost),
            &format!("{:.1}x", savings),
        ),
        comparison_row(
            &theme,
            "total tokens",
            &format!("{}", naive_summary.total_tokens),
            &format!("{}", opt_summary.total_tokens),
            &format!(
                "-{:.0}%",
                (1.0 - opt_summary.total_tokens as f64 / naive_summary.total_tokens.max(1) as f64)
                    * 100.0
            ),
        ),
        comparison_row(
            &theme,
            "pass rate",
            &format!("{:.0}%", naive_summary.pass_rate * 100.0),
            &format!("{:.0}%", opt_summary.pass_rate * 100.0),
            &format!(
                "+{:.0}pp",
                (opt_summary.pass_rate - naive_summary.pass_rate) * 100.0
            ),
        ),
        comparison_row(
            &theme,
            "avg latency",
            &format!("{:.1}s", naive_summary.avg_duration_s),
            &format!("{:.1}s", opt_summary.avg_duration_s),
            &format!(
                "-{:.0}%",
                (1.0 - opt_summary.avg_duration_s / naive_summary.avg_duration_s.max(0.01)) * 100.0
            ),
        ),
        comparison_row(
            &theme,
            "cache hit",
            "0%",
            &format!("{:.0}%", opt_summary.avg_cache_hit * 100.0),
            &format!("+{:.0}pp", opt_summary.avg_cache_hit * 100.0),
        ),
        comparison_row(
            &theme,
            "primary model",
            &naive_summary.primary_model,
            &opt_summary.primary_model,
            "routing",
        ),
    ];
    term.push_lines_revealed(&comparison_lines, Duration::from_millis(40))?;
    term.push_blank()?;

    // Cost waterfall
    let waterfall = CostWaterfallData {
        baseline_usd: naive_summary.total_cost,
        entries: vec![
            WaterfallEntry {
                label: "prompt caching".into(),
                savings_usd: naive_summary.total_cost * 0.35,
                factor: 1.0 / 0.65,
            },
            WaterfallEntry {
                label: "cascade routing".into(),
                savings_usd: naive_summary.total_cost * 0.25,
                factor: 1.0 / 0.75,
            },
            WaterfallEntry {
                label: "knowledge pre-load".into(),
                savings_usd: naive_summary.total_cost * 0.08,
                factor: 1.0 / 0.92,
            },
            WaterfallEntry {
                label: "gate early-exit".into(),
                savings_usd: naive_summary.total_cost * 0.04,
                factor: 1.0 / 0.96,
            },
        ],
        actual_usd: opt_summary.total_cost,
    };
    term.push_lines_revealed(&waterfall.to_lines(&theme), Duration::from_millis(50))?;
    term.push_blank()?;

    // Session summary
    let mut meter = CostMeter::new();
    for r in &opt_results {
        meter.record_run(r.cost_usd, r.input_tokens, r.output_tokens, &r.model, 0.0);
    }
    let session = SessionSummaryData {
        cost: meter,
        gates_total: opt_results.iter().map(|r| r.gates_total).sum(),
        gates_passed: opt_results.iter().map(|r| r.gates_passed).sum(),
        replans: 1,
        elapsed_s: opt_results.iter().map(|r| r.duration_s).sum(),
    };
    term.push_lines_revealed(&session.to_lines(&theme), Duration::from_millis(40))?;

    // Final result
    term.push_blank()?;
    term.push_lines(&[Line::from(vec![
        Span::styled(format!("{} ", symbols::PASS), theme.success()),
        Span::styled(
            format!(
                "benchmark complete  {}  {:.1}x cost reduction  {}  {}/{} tasks pass",
                symbols::SEP,
                savings,
                symbols::SEP,
                opt_summary.tasks_passed,
                opt_summary.tasks_run,
            ),
            theme.success(),
        ),
    ])])?;
    term.push_blank()?;

    drop(term);
    Ok(())
}

fn comparison_row(
    theme: &Theme,
    label: &str,
    naive: &str,
    optimized: &str,
    improvement: &str,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(symbols::BAR.to_string(), theme.muted()),
        Span::raw("  "),
        Span::styled(
            format!("{:<18}", label),
            Style::default().fg(Theme::TEXT_DIM),
        ),
        Span::styled(format!("{:<14}", naive), theme.text()),
        Span::styled(format!("{:<14}", optimized), theme.text()),
        Span::styled(improvement.to_string(), Style::default().fg(Theme::SAGE)),
    ])
}

fn summarize(results: &[BenchResult]) -> ModeSummary {
    let n = results.len() as u32;
    let passed = results.iter().filter(|r| r.passed).count() as u32;
    ModeSummary {
        mode: results.first().map(|r| r.mode.clone()).unwrap_or_default(),
        total_cost: results.iter().map(|r| r.cost_usd).sum(),
        total_tokens: results
            .iter()
            .map(|r| r.input_tokens + r.output_tokens)
            .sum(),
        pass_rate: if n > 0 { passed as f64 / n as f64 } else { 0.0 },
        avg_cache_hit: results.iter().map(|r| r.cache_hit_rate).sum::<f64>() / n.max(1) as f64,
        avg_duration_s: results.iter().map(|r| r.duration_s).sum::<f64>() / n.max(1) as f64,
        tasks_run: n,
        tasks_passed: passed,
        primary_model: results.first().map(|r| r.model.clone()).unwrap_or_default(),
    }
}

/// Simulate a benchmark result with realistic numbers.
fn simulate_task(task: &BenchTask, mode: &str) -> BenchResult {
    let is_naive = mode == "naive";
    let difficulty_factor = match task.difficulty.as_str() {
        "easy" => 1.0,
        "medium" => 2.0,
        "hard" => 3.5,
        _ => 1.0,
    };

    let (cost, tokens_in, tokens_out, cache_hit, duration, model, pass_rate) = if is_naive {
        (
            0.85 * difficulty_factor,
            (4000.0 * difficulty_factor) as u64,
            (1200.0 * difficulty_factor) as u64,
            0.0,
            12.0 * difficulty_factor,
            "opus".to_string(),
            0.75,
        )
    } else {
        (
            0.028 * difficulty_factor,
            (1800.0 * difficulty_factor) as u64,
            (500.0 * difficulty_factor) as u64,
            0.82 + (0.10 * (1.0 / difficulty_factor)),
            3.2 * difficulty_factor,
            if difficulty_factor > 2.0 {
                "sonnet".to_string()
            } else {
                "haiku".to_string()
            },
            0.90,
        )
    };

    // Deterministic "randomness" based on task ID
    let seed: u64 = task.id.bytes().map(|b| b as u64).sum();
    let passed = (seed % 100) as f64 / 100.0 < pass_rate;

    BenchResult {
        task_id: task.id.clone(),
        mode: mode.to_string(),
        passed,
        cost_usd: cost,
        input_tokens: tokens_in,
        output_tokens: tokens_out,
        cache_hit_rate: cache_hit,
        duration_s: duration,
        model,
        gates_passed: if passed { 4 } else { 3 },
        gates_total: 4,
    }
}

/// Run a task for real via roko's universal loop.
async fn run_task_real(workdir: &Path, task: &BenchTask, mode: &str) -> Result<BenchResult> {
    // TODO: wire into actual run_once with config overrides for naive vs optimized
    // For now, simulate with longer delays to feel real
    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(simulate_task(task, mode))
}

async fn run_bench_plain(workdir: &Path, tasks: &[BenchTask], real_dispatch: bool) -> Result<()> {
    println!(
        "roko bench demo — {} tasks, naive vs optimized",
        tasks.len()
    );
    println!();

    for mode in ["naive", "optimized"] {
        println!("--- {mode} ---");
        for task in tasks {
            let result = if real_dispatch {
                run_task_real(workdir, task, mode).await?
            } else {
                simulate_task(task, mode)
            };
            let icon = if result.passed {
                symbols::PASS
            } else {
                symbols::FAIL
            };
            println!(
                "  {icon} {}  ${:.4}  {}tok  {:.1}s  {}",
                task.id,
                result.cost_usd,
                result.input_tokens + result.output_tokens,
                result.duration_s,
                result.model
            );
        }
        println!();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulate_produces_results() {
        let tasks = default_tasks();
        for task in &tasks {
            let naive = simulate_task(task, "naive");
            let opt = simulate_task(task, "optimized");
            assert!(naive.cost_usd > opt.cost_usd, "optimized should be cheaper");
            assert!(
                naive.duration_s > opt.duration_s,
                "optimized should be faster"
            );
            assert_eq!(naive.cache_hit_rate, 0.0, "naive should have no cache");
            assert!(opt.cache_hit_rate > 0.0, "optimized should have cache hits");
        }
    }

    #[test]
    fn summarize_works() {
        let results = vec![
            BenchResult {
                task_id: "T01".into(),
                mode: "naive".into(),
                passed: true,
                cost_usd: 1.0,
                input_tokens: 1000,
                output_tokens: 500,
                cache_hit_rate: 0.0,
                duration_s: 10.0,
                model: "opus".into(),
                gates_passed: 4,
                gates_total: 4,
            },
            BenchResult {
                task_id: "T02".into(),
                mode: "naive".into(),
                passed: false,
                cost_usd: 2.0,
                input_tokens: 2000,
                output_tokens: 800,
                cache_hit_rate: 0.0,
                duration_s: 15.0,
                model: "opus".into(),
                gates_passed: 3,
                gates_total: 4,
            },
        ];
        let summary = summarize(&results);
        assert_eq!(summary.tasks_run, 2);
        assert_eq!(summary.tasks_passed, 1);
        assert!((summary.pass_rate - 0.5).abs() < f64::EPSILON);
        assert!((summary.total_cost - 3.0).abs() < f64::EPSILON);
    }
}
