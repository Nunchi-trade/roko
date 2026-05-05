//! dashboard command handlers.
#![allow(unused_imports)]

use crate::*;
use roko_fs::RokoLayout;

pub(crate) async fn cmd_dashboard(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
    text: bool,
    state_hub: Option<roko_cli::state_hub::SharedStateHub>,
) -> Result<i32> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);

    let initial_page = page.as_deref().map(|page| {
        parse_dashboard_page(page).ok_or_else(|| {
            anyhow!(
                "unknown dashboard page `{page}`; available pages: {}",
                dashboard_page_slugs().join(", ")
            )
        })
    });
    let initial_page = initial_page.transpose()?;

    if !text && !list_pages && std::io::stdout().is_terminal() {
        // Use the Mori-style interactive TUI with 60fps event loop.
        let app = if let Some(state_hub) = state_hub.as_ref() {
            App::new_connected_with_page(&workdir, initial_page, state_hub)
        } else {
            App::new_with_page(&workdir, initial_page)
        };
        let tui_result = tokio::task::spawn_blocking(move || app.run())
            .await
            .context("dashboard TUI worker failed")?;
        if tui_result.is_ok() {
            return Ok(EXIT_SUCCESS);
        }
    }

    let output = render_dashboard_text(cli, Some(workdir), page, list_pages).await?;
    print!("{output}");
    Ok(EXIT_SUCCESS)
}

pub(crate) async fn render_dashboard_text(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
) -> Result<String> {
    let workdir = workdir.unwrap_or_else(|| resolve_workdir(cli));
    prepare_runtime_hooks(&workdir, cli.quiet);

    let dashboard = DashboardScaffold::new_in(&workdir);
    if list_pages {
        let mut out = String::new();
        for page in dashboard.pages() {
            let _ = writeln!(out, "{:<16} {}", page.id.slug(), page.title);
        }
        return Ok(out);
    }

    let snapshot = DashboardSnapshot::load(&workdir).await?;

    if let Some(page) = page {
        let Some(page_id) = parse_dashboard_page(&page) else {
            anyhow::bail!(
                "unknown dashboard page `{page}`; available pages: {}",
                dashboard_page_slugs().join(", ")
            );
        };
        Ok(match page_id {
            PageId::Health | PageId::Trends => snapshot
                .render_page_text(page_id)
                .unwrap_or_else(|| dashboard.render_active_page_text()),
            _ => {
                let mut dashboard = dashboard;
                let _ = dashboard.set_active_page(page_id);
                dashboard.render_active_page_text()
            }
        })
    } else {
        Ok(dashboard.render_overview_text())
    }
}

pub(crate) fn render_data_page(title: &str, slug: &str, intent: &str, lines: &[String]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{title} ({slug})");
    let _ = writeln!(out, "group: efficiency");
    let _ = writeln!(out, "intent: {intent}");
    let _ = writeln!(
        out,
        "focus: {}",
        lines.first().map_or("n/a", String::as_str)
    );
    let _ = writeln!(out, "widgets ({}):", lines.len());
    for (idx, line) in lines.iter().enumerate() {
        let _ = writeln!(out, "- {}: {}", idx + 1, line);
    }
    out
}

pub(crate) fn format_percent(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

pub(crate) fn format_currency(value: f64) -> String {
    format!("${value:.4}")
}

pub(crate) fn format_duration(ms: f64) -> String {
    if ms >= 1000.0 {
        let seconds = ms / 1000.0;
        format!("{seconds:.2}s")
    } else {
        format!("{ms:.0}ms")
    }
}

pub(crate) async fn load_task_metrics(path: PathBuf) -> Vec<TaskMetric> {
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Vec::new();
    };

    let mut records = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(metric) = TaskMetric::from_jsonl(line) {
            records.push(metric);
        }
    }
    records
}

pub(crate) async fn load_cfactor_history(path: PathBuf) -> Vec<CFactor> {
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return Vec::new();
    };

    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<CFactor>(line).ok())
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DashboardSnapshot {
    episodes: Vec<Episode>,
    task_metrics: Vec<TaskMetric>,
    headlines: Headlines,
    cfactor_history: Vec<CFactor>,
    cfactor: Option<CFactor>,
}

impl DashboardSnapshot {
    async fn load(workdir: &Path) -> Result<Self> {
        let layout = RokoLayout::for_project(workdir);
        let episodes = EpisodeLogger::read_all_lossy(layout.episodes_path()).await?;
        let task_metrics = load_task_metrics(layout.memory_dir().join("task-metrics.jsonl")).await;
        let cfactor_history =
            load_cfactor_history(workdir.join(".roko").join("learn").join("c-factor.jsonl")).await;
        let cfactor = cfactor_history.last().cloned();
        let headlines = compute_headlines(&task_metrics);
        Ok(Self {
            episodes,
            task_metrics,
            headlines,
            cfactor_history,
            cfactor,
        })
    }

    fn render_page_text(&self, page: PageId) -> Option<String> {
        match page {
            PageId::Health => Some(self.render_health_page_text()),
            PageId::Trends => Some(self.render_trends_page_text()),
            _ => None,
        }
    }

    fn render_health_page_text(&self) -> String {
        let summary = self.health_summary();
        let cfactor = self.cfactor.as_ref().cloned().unwrap_or_default();
        let trend =
            cfactor_trend_arrow(&self.cfactor_history, Duration::from_secs(7 * 24 * 60 * 60));
        render_data_page(
            "Health",
            PageId::Health.slug(),
            "Top-line health gauges derived from the latest snapshot.",
            &[
                format!(
                    "focus: current C-Factor {:.2} {}, {} episodes",
                    cfactor.overall, trend, summary.episode_count
                ),
                format!("episodes: {}", summary.episode_count),
                format!("success rate: {}", format_percent(summary.success_rate)),
                format!(
                    "avg cost / episode: {}",
                    format_currency(summary.avg_cost_per_episode)
                ),
                format!(
                    "avg wall time: {}",
                    format_duration(summary.avg_wall_time_ms)
                ),
                format!("cache hit rate: {}", format_percent(summary.cache_hit_rate)),
                format!("haiku share: {}", format_percent(summary.haiku_share)),
                format!("current c-factor: {:.2} {}", cfactor.overall, trend),
                format!(
                    "gate pass rate: {}",
                    format_percent(cfactor.components.gate_pass_rate)
                ),
                format!(
                    "cost efficiency: {}",
                    format_percent(cfactor.components.cost_efficiency)
                ),
                format!("speed: {}", format_percent(cfactor.components.speed)),
                format!(
                    "information flow rate: {}",
                    format_percent(cfactor.components.information_flow_rate)
                ),
                format!(
                    "first-try rate: {}",
                    format_percent(cfactor.components.first_try_rate)
                ),
                format!(
                    "knowledge growth: {}",
                    format_percent(cfactor.components.knowledge_growth)
                ),
                format!(
                    "knowledge integration rate: {}",
                    format_percent(cfactor.components.knowledge_integration_rate)
                ),
                format!(
                    "convergence velocity: {}",
                    format_percent(cfactor.components.convergence_velocity)
                ),
                format!(
                    "turn-taking equality: {}",
                    format_percent(cfactor.components.turn_taking_equality)
                ),
                format!(
                    "social sensitivity: {}",
                    format_percent(cfactor.components.social_perceptiveness)
                ),
            ],
        )
    }

    fn render_trends_page_text(&self) -> String {
        let summary = self.health_summary();
        let headlines = self.headlines;
        render_data_page(
            "Trends",
            PageId::Trends.slug(),
            "Time-series learning signals from the current snapshot.",
            &[
                format!(
                    "focus: {} records across {} plans, {} pass rate",
                    headlines.n_records,
                    headlines.n_plans,
                    format_percent(headlines.first_attempt_pass_rate)
                ),
                format!(
                    "first-attempt pass rate: {}",
                    format_percent(headlines.first_attempt_pass_rate)
                ),
                format!(
                    "avg iterations per plan: {:.2}",
                    headlines.avg_iterations_per_plan
                ),
                format!(
                    "avg cost per plan: {}",
                    format_currency(headlines.avg_cost_per_plan)
                ),
                format!(
                    "avg input tokens per spawn: {:.0}",
                    headlines.avg_input_tokens_per_spawn
                ),
                format!("haiku share: {}", format_percent(summary.haiku_share)),
                format!("cache hit rate: {}", format_percent(summary.cache_hit_rate)),
            ],
        )
    }

    #[allow(clippy::cast_precision_loss)]
    fn health_summary(&self) -> DashboardHealthSummary {
        let episode_count = self.episodes.len();
        let success_count = self
            .episodes
            .iter()
            .filter(|episode| episode.success)
            .count();
        let total_cost = self
            .episodes
            .iter()
            .map(|episode| episode.usage.cost_usd)
            .sum::<f64>();
        let total_wall_ms = self
            .episodes
            .iter()
            .map(|episode| episode.usage.wall_ms)
            .sum::<u64>();
        let total_input_tokens = self
            .episodes
            .iter()
            .map(|episode| episode.usage.input_tokens)
            .sum::<u64>();
        let total_cache_read_tokens = self
            .episodes
            .iter()
            .map(|episode| episode.usage.cache_read_tokens)
            .sum::<u64>();
        let avg_cost_per_episode = if episode_count == 0 {
            0.0
        } else {
            total_cost / episode_count as f64
        };
        let avg_wall_time_ms = if episode_count == 0 {
            0.0
        } else {
            total_wall_ms as f64 / episode_count as f64
        };
        let cache_hit_rate = if total_input_tokens == 0 {
            0.0
        } else {
            total_cache_read_tokens as f64 / total_input_tokens as f64
        };
        let haiku_share = if self.task_metrics.is_empty() {
            0.0
        } else {
            self.task_metrics
                .iter()
                .filter(|metric| metric.model.to_ascii_lowercase().contains("haiku"))
                .count() as f64
                / self.task_metrics.len() as f64
        };

        DashboardHealthSummary {
            episode_count,
            success_rate: if episode_count == 0 {
                0.0
            } else {
                success_count as f64 / episode_count as f64
            },
            avg_cost_per_episode,
            avg_wall_time_ms,
            cache_hit_rate,
            haiku_share,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DashboardHealthSummary {
    episode_count: usize,
    success_rate: f64,
    avg_cost_per_episode: f64,
    avg_wall_time_ms: f64,
    cache_hit_rate: f64,
    haiku_share: f64,
}

#[cfg(test)]
pub(crate) async fn dashboard_output(
    cli: &Cli,
    workdir: Option<PathBuf>,
    page: Option<String>,
    list_pages: bool,
) -> Result<String> {
    render_dashboard_text(cli, workdir, page, list_pages).await
}
