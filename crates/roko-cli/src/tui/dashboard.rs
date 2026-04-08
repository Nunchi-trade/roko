//! Dashboard scaffold container for future TUI wiring.
//!
//! This module keeps the existing page scaffold intact, but layers a
//! best-effort learning snapshot on top so the health and trends pages
//! can render real stats when the memory JSONL files are present.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};

use roko_core::metric::{Headlines, TaskMetric, compute_headlines};
use roko_learn::episode_logger::{Episode, EpisodeLogger};

use super::pages::{PageId, PageScaffold, efficiency, operations};

const MEMORY_DIR: &str = ".roko/memory";
const EPISODES_FILE: &str = "episodes.jsonl";
const TASK_METRICS_FILE: &str = "task-metrics.jsonl";

/// In-memory scaffold of all placeholder dashboard pages.
#[derive(Debug, Clone)]
pub struct DashboardScaffold {
    pages: BTreeMap<PageId, PageScaffold>,
    active_page: PageId,
    snapshot: DashboardSnapshot,
}

impl DashboardScaffold {
    /// Build the full scaffold with all placeholder pages.
    #[must_use]
    pub fn new() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new_in(root)
    }

    /// Build the scaffold and load snapshot data relative to `root`.
    #[must_use]
    pub fn new_in(root: impl AsRef<Path>) -> Self {
        let mut pages = BTreeMap::new();
        for page in efficiency::scaffold_pages()
            .into_iter()
            .chain(operations::scaffold_pages())
        {
            pages.insert(page.id, page);
        }

        let root = resolve_snapshot_root(root.as_ref());
        let snapshot = load_snapshot_best_effort(&root);

        Self {
            pages,
            active_page: PageId::Health,
            snapshot,
        }
    }

    /// List all pages in stable order.
    #[must_use]
    pub fn pages(&self) -> Vec<&PageScaffold> {
        self.pages.values().collect()
    }

    /// Current active page.
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.active_page
    }

    /// Set active page if it exists in the scaffold.
    pub fn set_active_page(&mut self, page: PageId) -> bool {
        if self.pages.contains_key(&page) {
            self.active_page = page;
            true
        } else {
            false
        }
    }

    /// Return a specific page by ID.
    #[must_use]
    pub fn page(&self, page: PageId) -> Option<&PageScaffold> {
        self.pages.get(&page)
    }

    /// Build a high-level summary used by future command wiring.
    #[must_use]
    pub fn summary(&self) -> DashboardSummary {
        let widget_count = self.pages.values().map(|p| p.widgets.len()).sum();
        DashboardSummary {
            active_page: self.active_page,
            page_count: self.pages.len(),
            widget_count,
        }
    }

    /// Render a plain-text dashboard summary suitable for CLI output.
    #[must_use]
    pub fn render_overview_text(&self) -> String {
        let mut out = self.summary().to_string();
        out.push_str("\nactive page:\n");
        if let Some(page) = self.page(self.active_page) {
            let _ = writeln!(out, "{}", page.render_summary_line(true));
        }
        out.push_str("pages:\n");
        out.push_str(&self.render_page_index_text());
        out
    }

    /// Render the compact page index only.
    #[must_use]
    pub fn render_page_index_text(&self) -> String {
        let mut out = String::new();
        for page in self.pages.values() {
            let _ = writeln!(
                out,
                "{}",
                page.render_summary_line(page.id == self.active_page)
            );
        }
        out
    }

    /// Render one page as plain text. Returns `None` if the page does not exist.
    #[must_use]
    pub fn render_page_text(&self, page: PageId) -> Option<String> {
        let scaffold = self.page(page)?;
        match page {
            PageId::Health => self
                .snapshot
                .render_health_page(scaffold)
                .or_else(|| Some(scaffold.render_text())),
            PageId::Trends => self
                .snapshot
                .render_trends_page(scaffold)
                .or_else(|| Some(scaffold.render_text())),
            _ => Some(scaffold.render_text()),
        }
    }

    /// Render one page's widget list only. Returns `None` if the page does not exist.
    #[must_use]
    pub fn render_page_list_text(&self, page: PageId) -> Option<String> {
        self.page(page).map(PageScaffold::render_widget_list)
    }

    /// Render the current active page as plain text.
    #[must_use]
    pub fn render_active_page_text(&self) -> String {
        self.render_page_text(self.active_page)
            .unwrap_or_else(|| String::from("<missing active page>"))
    }

    /// Render the health page as plain text.
    #[must_use]
    pub fn render_health_page_text(&self) -> String {
        self.render_page_text(PageId::Health)
            .unwrap_or_else(|| String::from("<missing health page>"))
    }

    /// Render the trends page as plain text.
    #[must_use]
    pub fn render_trends_page_text(&self) -> String {
        self.render_page_text(PageId::Trends)
            .unwrap_or_else(|| String::from("<missing trends page>"))
    }
}

impl Default for DashboardScaffold {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary metadata for the dashboard scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DashboardSummary {
    /// Currently selected page.
    pub active_page: PageId,
    /// Number of pages scaffolded.
    pub page_count: usize,
    /// Number of widgets scaffolded across all pages.
    pub widget_count: usize,
}

impl fmt::Display for DashboardSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dashboard scaffold: {} pages, {} widgets, active={}",
            self.page_count,
            self.widget_count,
            self.active_page.slug()
        )
    }
}

/// Best-effort learning snapshot for dashboard rendering.
#[derive(Debug, Clone)]
pub struct DashboardSnapshot {
    root: PathBuf,
    episode_count: usize,
    success_rate: Option<f64>,
    average_cost_usd: Option<f64>,
    average_wall_time_ms: Option<f64>,
    task_metric_count: usize,
    haiku_share: Option<f64>,
    cache_hit_rate: Option<f64>,
    headlines: Headlines,
}

impl DashboardSnapshot {
    /// Load the learning snapshot from a workspace root.
    pub async fn load(root: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let root = resolve_snapshot_root(root.as_ref());
        let memory_dir = root.join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let task_metrics_path = memory_dir.join(TASK_METRICS_FILE);

        let episodes_logger = EpisodeLogger::new(&episodes_path);
        let episodes = EpisodeLogger::read_all_lossy(episodes_logger.path())
            .await
            .map_err(std::io::Error::other)?;
        let task_metrics = read_task_metrics(&task_metrics_path).await?;

        Ok(Self::from_records(root, &episodes, &task_metrics))
    }

    fn empty(root: PathBuf) -> Self {
        Self::from_records(root, &[], &[])
    }

    fn from_records(root: PathBuf, episodes: &[Episode], task_metrics: &[TaskMetric]) -> Self {
        let episode_count = episodes.len();
        let success_rate = if episode_count == 0 {
            None
        } else {
            let successes = episodes.iter().filter(|episode| episode.success).count();
            Some(count_to_f64(successes) / count_to_f64(episode_count))
        };
        let average_cost_usd = if episode_count == 0 {
            None
        } else {
            Some(
                episodes
                    .iter()
                    .map(|episode| episode.usage.cost_usd)
                    .sum::<f64>()
                    / count_to_f64(episode_count),
            )
        };
        let average_wall_time_ms = if episode_count == 0 {
            None
        } else {
            Some(
                episodes
                    .iter()
                    .map(|episode| wall_ms_to_f64(episode.usage.wall_ms))
                    .sum::<f64>()
                    / count_to_f64(episode_count),
            )
        };

        let task_metric_count = task_metrics.len();
        let haiku_share = if task_metric_count == 0 {
            None
        } else {
            let haiku = task_metrics
                .iter()
                .filter(|metric| metric.model.to_ascii_lowercase().contains("haiku"))
                .count();
            Some(count_to_f64(haiku) / count_to_f64(task_metric_count))
        };
        let cache_hit_rate = if task_metric_count == 0 {
            None
        } else {
            Some(
                task_metrics
                    .iter()
                    .map(|metric| metric.cache_hit_rate)
                    .sum::<f64>()
                    / count_to_f64(task_metric_count),
            )
        };
        let headlines = compute_headlines(task_metrics);

        Self {
            root,
            episode_count,
            success_rate,
            average_cost_usd,
            average_wall_time_ms,
            task_metric_count,
            haiku_share,
            cache_hit_rate,
            headlines,
        }
    }

    fn render_health_page(&self, page: &PageScaffold) -> Option<String> {
        if self.episode_count == 0 {
            return None;
        }

        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", page.title, page.id.slug());
        let _ = writeln!(out, "group: {}", page.id.group());
        let _ = writeln!(out, "intent: {}", page.intent);
        let _ = writeln!(
            out,
            "source: {}/{}",
            self.root.join(MEMORY_DIR).display(),
            EPISODES_FILE
        );
        let _ = writeln!(out, "episodes: {}", self.episode_count);
        let _ = writeln!(
            out,
            "success rate: {}",
            format_pct(self.success_rate.unwrap_or(0.0))
        );
        let _ = writeln!(
            out,
            "average cost: {}",
            format_usd(self.average_cost_usd.unwrap_or(0.0))
        );
        let _ = writeln!(
            out,
            "average wall time: {}",
            format_ms(self.average_wall_time_ms.unwrap_or(0.0))
        );
        if let Some(hit_rate) = self.cache_hit_rate {
            let _ = writeln!(out, "cache hit rate: {}", format_pct(hit_rate));
        }
        if let Some(haiku_share) = self.haiku_share {
            let _ = writeln!(out, "haiku share: {}", format_pct(haiku_share));
        }
        if self.task_metric_count > 0 {
            let _ = writeln!(out, "task metrics: {}", self.task_metric_count);
        }
        out.push_str("widgets (scaffold):\n");
        for widget in &page.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        Some(out)
    }

    fn render_trends_page(&self, page: &PageScaffold) -> Option<String> {
        if self.task_metric_count == 0 {
            return None;
        }

        let mut out = String::new();
        let _ = writeln!(out, "{} ({})", page.title, page.id.slug());
        let _ = writeln!(out, "group: {}", page.id.group());
        let _ = writeln!(out, "intent: {}", page.intent);
        let _ = writeln!(
            out,
            "source: {}/{}",
            self.root.join(MEMORY_DIR).display(),
            TASK_METRICS_FILE
        );
        let _ = writeln!(out, "task metrics: {}", self.task_metric_count);
        let _ = writeln!(
            out,
            "first-attempt pass rate: {}",
            format_pct(self.headlines.first_attempt_pass_rate)
        );
        let _ = writeln!(
            out,
            "avg iterations per plan: {}",
            format_float(self.headlines.avg_iterations_per_plan)
        );
        let _ = writeln!(
            out,
            "avg cost per plan: {}",
            format_usd(self.headlines.avg_cost_per_plan)
        );
        let _ = writeln!(
            out,
            "avg input tokens per spawn: {}",
            format_float(self.headlines.avg_input_tokens_per_spawn)
        );
        let _ = writeln!(out, "plans: {}", self.headlines.n_plans);
        let _ = writeln!(out, "records: {}", self.headlines.n_records);
        if let Some(hit_rate) = self.cache_hit_rate {
            let _ = writeln!(out, "cache hit rate: {}", format_pct(hit_rate));
        }
        if let Some(haiku_share) = self.haiku_share {
            let _ = writeln!(out, "haiku share: {}", format_pct(haiku_share));
        }
        out.push_str("headlines:\n");
        let _ = writeln!(
            out,
            "- first_attempt_pass_rate: {}",
            format_pct(self.headlines.first_attempt_pass_rate)
        );
        let _ = writeln!(
            out,
            "- avg_iterations_per_plan: {}",
            format_float(self.headlines.avg_iterations_per_plan)
        );
        let _ = writeln!(
            out,
            "- avg_cost_per_plan: {}",
            format_usd(self.headlines.avg_cost_per_plan)
        );
        let _ = writeln!(
            out,
            "- avg_input_tokens_per_spawn: {}",
            format_float(self.headlines.avg_input_tokens_per_spawn)
        );
        out.push_str("widgets (scaffold):\n");
        for widget in &page.widgets {
            let _ = writeln!(out, "{}", widget.render_line());
        }
        Some(out)
    }
}

fn load_snapshot_best_effort(root: &Path) -> DashboardSnapshot {
    load_snapshot_blocking(root).unwrap_or_else(|_| DashboardSnapshot::empty(root.to_path_buf()))
}

fn load_snapshot_blocking(root: &Path) -> Result<DashboardSnapshot, std::io::Error> {
    let root = root.to_path_buf();
    let load = move || -> Result<DashboardSnapshot, std::io::Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(std::io::Error::other)?;
        runtime.block_on(DashboardSnapshot::load(&root))
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(load)
            .join()
            .map_err(|_| std::io::Error::other("dashboard snapshot loader panicked"))?
    } else {
        load()
    }
}

fn resolve_snapshot_root(start: &Path) -> PathBuf {
    let mut cursor = Some(start);
    while let Some(dir) = cursor {
        let memory_dir = dir.join(MEMORY_DIR);
        if memory_dir.join(EPISODES_FILE).exists() || memory_dir.join(TASK_METRICS_FILE).exists() {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    start.to_path_buf()
}

async fn read_task_metrics(path: &Path) -> Result<Vec<TaskMetric>, std::io::Error> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };

    let mut metrics = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(metric) = TaskMetric::from_jsonl(line) {
            metrics.push(metric);
        }
    }
    Ok(metrics)
}

fn format_pct(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

fn format_float(value: f64) -> String {
    format!("{value:.2}")
}

fn format_usd(value: f64) -> String {
    format!("${value:.4}")
}

fn format_ms(value: f64) -> String {
    format!("{value:.0} ms")
}

fn count_to_f64(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

fn wall_ms_to_f64(wall_ms: u64) -> f64 {
    f64::from(u32::try_from(wall_ms).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use tempfile::tempdir;

    fn write_jsonl(path: &Path, lines: &[String]) {
        fs::create_dir_all(path.parent().expect("file has parent"))
            .expect("should create parent dir");
        fs::write(path, lines.join("\n") + "\n").expect("should write jsonl");
    }

    fn sample_episode(
        agent: &str,
        task: &str,
        success: bool,
        cost_usd: f64,
        wall_ms: u64,
    ) -> Episode {
        let mut episode = Episode::new(agent, task);
        episode.success = success;
        episode.usage.cost_usd = cost_usd;
        episode.usage.wall_ms = wall_ms;
        episode
    }

    fn sample_metric(
        plan: &str,
        task: &str,
        iteration: u32,
        passed: bool,
        model: &str,
        input_tokens: u64,
        cache_hit_rate: f64,
        cost_usd: f64,
    ) -> TaskMetric {
        let mut metric = TaskMetric::new(
            roko_core::metric::ConfigHash::from("hash".to_string()),
            plan,
            task,
        );
        metric.iteration = iteration;
        metric.gate_passed = passed;
        metric.model = model.to_string();
        metric.input_tokens = input_tokens;
        metric.cached_tokens = (input_tokens as f64 * cache_hit_rate).round() as u64;
        metric.cache_hit_rate = cache_hit_rate;
        metric.cost_usd = cost_usd;
        metric
    }

    #[test]
    fn scaffold_has_expected_page_count() {
        let dashboard = DashboardScaffold::new();
        let summary = dashboard.summary();
        assert_eq!(summary.page_count, 10);
        assert!(summary.widget_count >= 20);
        assert_eq!(summary.active_page, PageId::Health);
    }

    #[test]
    fn can_switch_active_page() {
        let mut dashboard = DashboardScaffold::new();
        assert!(dashboard.set_active_page(PageId::PlanView));
        assert_eq!(dashboard.active_page(), PageId::PlanView);
    }

    #[test]
    fn overview_render_contains_active_page_and_counts() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard.render_overview_text();
        assert!(rendered.contains("dashboard scaffold: 10 pages"));
        assert!(rendered.contains("active=health"));
        assert!(rendered.contains("active page:"));
        assert!(rendered.contains("* Health [health] efficiency"));
    }

    #[test]
    fn page_render_includes_widgets() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard
            .render_page_text(PageId::PlanView)
            .expect("plan page should exist");
        assert!(rendered.contains("Plan View (plan-view)"));
        assert!(rendered.contains("widgets (2):"));
        assert!(rendered.contains("DAG [dag]"));
    }

    #[test]
    fn page_index_render_contains_compact_summaries() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard.render_page_index_text();
        assert!(rendered.contains("* Health [health] efficiency | 3 widgets"));
        assert!(rendered.contains("Plan View [plan-view] operations | 2 widgets"));
    }

    #[test]
    fn page_list_render_focuses_on_one_page_widget_list() {
        let dashboard = DashboardScaffold::new();
        let rendered = dashboard
            .render_page_list_text(PageId::ConfigView)
            .expect("config page should exist");
        assert!(rendered.contains("Config View [config-view]"));
        assert!(rendered.contains("widgets (2):"));
        assert!(rendered.contains("Effective Config [effective_config]"));
    }

    #[test]
    fn snapshot_loader_aggregates_episode_and_metric_stats() {
        let tempdir = tempdir().expect("tempdir");
        let memory_dir = tempdir.path().join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let metrics_path = memory_dir.join(TASK_METRICS_FILE);

        let episodes = vec![
            serde_json::to_string(&sample_episode("agent-a", "task-a", true, 1.50, 1_000))
                .expect("episode json"),
            serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.50, 3_000))
                .expect("episode json"),
        ];
        write_jsonl(&episodes_path, &episodes);

        let metrics = vec![
            sample_metric("plan-a", "t1", 1, true, "claude-haiku-4-5", 100, 0.20, 0.10),
            sample_metric(
                "plan-a",
                "t1",
                2,
                false,
                "claude-sonnet-4-5",
                300,
                0.50,
                0.20,
            ),
            sample_metric("plan-b", "t2", 1, true, "claude-haiku-4-5", 200, 0.25, 0.30),
        ];
        write_jsonl(
            &metrics_path,
            &metrics
                .iter()
                .map(|metric| metric.to_jsonl().expect("metric json"))
                .collect::<Vec<_>>(),
        );

        let snapshot = load_snapshot_blocking(tempdir.path()).expect("snapshot should load");

        assert_eq!(snapshot.episode_count, 2);
        assert_eq!(snapshot.task_metric_count, 3);
        assert_eq!(snapshot.success_rate, Some(0.5));
        assert!((snapshot.average_cost_usd.expect("avg cost") - 1.0).abs() < 1e-9);
        assert!((snapshot.average_wall_time_ms.expect("avg wall") - 2_000.0).abs() < 1e-9);
        assert!((snapshot.haiku_share.expect("haiku share") - (2.0 / 3.0)).abs() < 1e-9);
        assert!((snapshot.cache_hit_rate.expect("cache hit") - (0.95 / 3.0)).abs() < 1e-9);
        assert_eq!(snapshot.headlines.n_plans, 2);
        assert_eq!(snapshot.headlines.n_records, 3);
        assert!((snapshot.headlines.first_attempt_pass_rate - 1.0).abs() < 1e-9);
        assert!((snapshot.headlines.avg_iterations_per_plan - 1.5).abs() < 1e-9);
    }

    #[test]
    fn health_and_trends_render_real_stats_when_snapshot_exists() {
        let tempdir = tempdir().expect("tempdir");
        let memory_dir = tempdir.path().join(MEMORY_DIR);
        let episodes_path = memory_dir.join(EPISODES_FILE);
        let metrics_path = memory_dir.join(TASK_METRICS_FILE);

        write_jsonl(
            &episodes_path,
            &[
                serde_json::to_string(&sample_episode("agent-a", "task-a", true, 1.50, 1_000))
                    .expect("episode json"),
                serde_json::to_string(&sample_episode("agent-b", "task-b", false, 0.50, 3_000))
                    .expect("episode json"),
            ],
        );
        write_jsonl(
            &metrics_path,
            &[
                sample_metric("plan-a", "t1", 1, true, "claude-haiku-4-5", 100, 0.20, 0.10)
                    .to_jsonl()
                    .expect("metric json"),
                sample_metric(
                    "plan-a",
                    "t1",
                    2,
                    false,
                    "claude-sonnet-4-5",
                    300,
                    0.50,
                    0.20,
                )
                .to_jsonl()
                .expect("metric json"),
                sample_metric("plan-b", "t2", 1, true, "claude-haiku-4-5", 200, 0.25, 0.30)
                    .to_jsonl()
                    .expect("metric json"),
            ],
        );

        let dashboard = DashboardScaffold::new_in(tempdir.path());
        let health = dashboard.render_health_page_text();
        let trends = dashboard.render_trends_page_text();

        assert!(health.contains("episodes: 2"));
        assert!(health.contains("success rate: 50.0%"));
        assert!(health.contains("average cost: $1.0000"));
        assert!(health.contains("average wall time: 2000 ms"));
        assert!(health.contains("haiku share: 66.7%"));
        assert!(health.contains("cache hit rate: 31.7%"));

        assert!(trends.contains("task metrics: 3"));
        assert!(trends.contains("first-attempt pass rate: 100.0%"));
        assert!(trends.contains("avg iterations per plan: 1.50"));
        assert!(trends.contains("avg cost per plan: $0.3000"));
        assert!(trends.contains("avg input tokens per spawn: 200.00"));
        assert!(trends.contains("haiku share: 66.7%"));
        assert!(trends.contains("cache hit rate: 31.7%"));
        assert!(trends.contains("- avg_cost_per_plan: $0.3000"));
    }
}
