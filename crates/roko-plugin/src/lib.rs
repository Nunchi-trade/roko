//! Plugin SDK for Roko event sources and feedback collectors.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cron::Schedule;
use globset::{Glob, GlobSet, GlobSetBuilder};
use notify::{EventKind, RecursiveMode, Watcher, recommended_watcher};
use roko_core::{
    Body, Kind, Result, RokoError, Signal, FS_CREATED, FS_DELETED, FS_MODIFIED,
};
use roko_core::config::{SchedulerConfig, SchedulerCronConfig, WatcherConfig, WatcherPathConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

/// Cloneable bounded sender used by event sources to publish signals into Roko.
pub type SignalSender = Sender<Signal>;

/// Outcome reported by a feedback collector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum FeedbackOutcome {
    /// The original work was accepted.
    Approved,
    /// The original work was rejected.
    Rejected,
    /// The original work received comments but no final verdict.
    Commented,
    /// The collector did not produce a meaningful signal.
    Ignored,
    /// The original work was merged.
    Merged,
}

/// Feedback emitted by collectors when they observe the outcome of past work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// Identifier of the original episode being reported on.
    pub original_episode_id: String,
    /// External service the feedback was collected from.
    pub service: String,
    /// Collector outcome for the original episode.
    pub outcome: FeedbackOutcome,
    /// Arbitrary structured metadata supplied by the service.
    pub metadata: Value,
    /// Time the feedback was observed.
    pub timestamp: DateTime<Utc>,
}

/// Kinds of event sources supported by the plugin SDK.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventSourceKind {
    /// HTTP webhook source.
    Webhook,
    /// Scheduled source.
    Cron,
    /// Filesystem watcher source.
    FileWatch,
    /// Custom source type provided by a plugin.
    Custom(String),
}

/// A filesystem event source backed by `notify`.
#[derive(Debug, Clone)]
pub struct FileWatchEventSource {
    paths: Vec<WatcherPathConfig>,
}

impl FileWatchEventSource {
    /// Create a watcher for the given directories.
    #[must_use]
    pub fn new<I, P>(directories: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Self {
            paths: directories
                .into_iter()
                .map(|directory| WatcherPathConfig {
                    directory: directory.into(),
                    ..WatcherPathConfig::default()
                })
                .collect(),
        }
    }

    /// Create a watcher from fully-specified path configs.
    #[must_use]
    pub fn from_paths<I>(paths: I) -> Self
    where
        I: IntoIterator<Item = WatcherPathConfig>,
    {
        Self {
            paths: paths.into_iter().collect(),
        }
    }

    /// Create a watcher from config.
    #[must_use]
    pub fn from_config(config: WatcherConfig) -> Self {
        Self::from_paths(config.paths)
    }

    /// Get the configured path entries.
    #[must_use]
    pub fn paths(&self) -> &[WatcherPathConfig] {
        &self.paths
    }
}

/// An asynchronous source of signals.
///
/// Implementors are expected to run until `cancel` fires, publishing
/// [`Signal`]s via `sender`. The trait is object-safe, so sources can be
/// stored and driven as `Box<dyn EventSource>`.
#[async_trait]
pub trait EventSource: Send + Sync + 'static {
    /// Human-readable source name.
    fn name(&self) -> &str;

    /// The source kind.
    fn kind(&self) -> EventSourceKind;

    /// Start the source and keep running until cancellation is requested.
    async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()>;
}

/// A scheduled event source backed by cron expressions.
#[derive(Debug, Clone)]
pub struct CronEventSource {
    schedules: Vec<CronSchedule>,
}

/// One cron schedule parsed from configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct CronSchedule {
    /// Human-readable schedule name.
    name: String,
    /// Standard cron expression.
    expression: String,
    /// Signal kind emitted when the schedule fires.
    signal_kind: String,
    /// Extra structured metadata included in the emitted signal body.
    #[serde(default)]
    metadata: Value,
}

#[derive(Debug, Clone)]
struct ActiveCronSchedule {
    schedule: CronSchedule,
    parsed: Schedule,
    next_fire: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct PendingFileWatchSignal {
    signal_kind: &'static str,
    event_kind: &'static str,
}

#[derive(Debug, Clone)]
struct CompiledFileWatchPath {
    directory: PathBuf,
    include: Option<GlobSet>,
    exclude: GlobSet,
}

impl From<SchedulerCronConfig> for CronSchedule {
    fn from(value: SchedulerCronConfig) -> Self {
        Self {
            name: value.name,
            expression: value.expression,
            signal_kind: value.signal_kind,
            metadata: value.metadata,
        }
    }
}

const FILE_WATCH_DEBOUNCE_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

impl CronEventSource {
    /// Create a cron event source from config.
    #[must_use]
    pub fn from_config(config: SchedulerConfig) -> Self {
        Self {
            schedules: config.cron.into_iter().map(CronSchedule::from).collect(),
        }
    }

    fn compile_schedules(&self) -> Result<Vec<ActiveCronSchedule>> {
        self.schedules
            .iter()
            .map(|schedule| {
                let parsed = Schedule::from_str(&schedule.expression).map_err(|err| {
                    RokoError::config(format!(
                        "invalid cron expression for schedule '{}': {err}",
                        schedule.name.as_str()
                    ))
                })?;
                let next_fire = parsed.upcoming(Utc).next();
                Ok(ActiveCronSchedule {
                    schedule: schedule.clone(),
                    parsed,
                    next_fire,
                })
            })
            .collect()
    }
}

#[async_trait]
impl EventSource for CronEventSource {
    fn name(&self) -> &str {
        "cron"
    }

    fn kind(&self) -> EventSourceKind {
        EventSourceKind::Cron
    }

    async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()> {
        let mut schedules = self.compile_schedules()?;
        if schedules.is_empty() {
            cancel.cancelled().await;
            return Ok(());
        }

        loop {
            if cancel.is_cancelled() {
                break;
            }

            let now = Utc::now();
            let mut due = Vec::new();
            let mut next_fire: Option<DateTime<Utc>> = None;

            for (idx, active) in schedules.iter().enumerate() {
                match active.next_fire {
                    Some(fire_at) if fire_at <= now => due.push(idx),
                    Some(fire_at) => {
                        next_fire = Some(match next_fire {
                            Some(current) => current.min(fire_at),
                            None => fire_at,
                        });
                    }
                    None => {}
                }
            }

            if due.is_empty() {
                let Some(fire_at) = next_fire else {
                    cancel.cancelled().await;
                    break;
                };

                let wait = fire_at.signed_duration_since(now).to_std().unwrap_or_default();
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(wait) => {}
                }
                continue;
            }

            for idx in due {
                let fired_at = Utc::now();
                let signal = cron_signal(&schedules[idx].schedule, fired_at);
                sender.send(signal).await.map_err(|_| {
                    RokoError::cancelled(format!(
                        "cron signal receiver dropped for schedule '{}'",
                        schedules[idx].schedule.name.as_str()
                    ))
                })?;
                schedules[idx].next_fire = schedules[idx].parsed.upcoming(Utc).next();
            }
        }

        Ok(())
    }
}

#[async_trait]
impl EventSource for FileWatchEventSource {
    fn name(&self) -> &str {
        "fswatcher"
    }

    fn kind(&self) -> EventSourceKind {
        EventSourceKind::FileWatch
    }

    async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()> {
        let watched_paths = compile_file_watch_paths(&self.paths)?;
        if watched_paths.is_empty() {
            cancel.cancelled().await;
            return Ok(());
        }

        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = recommended_watcher(move |result| {
            let _ = event_tx.send(result);
        })
        .map_err(|err| {
            RokoError::transport(format!("failed to create filesystem watcher: {err}"))
        })?;

        for watched_path in &watched_paths {
            watcher
                .watch(&watched_path.directory, RecursiveMode::Recursive)
                .map_err(|err| {
                RokoError::config(format!(
                    "failed to watch directory '{}': {err}",
                    watched_path.directory.display()
                ))
            })?;
        }

        drain_file_watch_events(event_rx, sender, cancel, watched_paths).await
    }
}

fn cron_signal(schedule: &CronSchedule, fired_at: DateTime<Utc>) -> Signal {
    Signal::builder(Kind::Custom(schedule.signal_kind.clone()))
        .body(Body::Json(serde_json::json!({
            "name": schedule.name.clone(),
            "expression": schedule.expression.clone(),
            "fired_at": fired_at.to_rfc3339(),
        })))
        .build()
}

fn compile_file_watch_paths(
    paths: &[WatcherPathConfig],
) -> Result<Vec<CompiledFileWatchPath>> {
    paths
        .iter()
        .map(|path| {
            let include = compile_globset(&path.include, &path.directory, "include")?;
            let exclude = compile_globset_with_defaults(
                &path.exclude,
                &path.directory,
                "exclude",
                default_file_watch_excludes(),
            )?;
            Ok(CompiledFileWatchPath {
                directory: path.directory.clone(),
                include,
                exclude,
            })
        })
        .collect()
}

fn compile_globset(
    patterns: &[String],
    directory: &Path,
    kind: &str,
) -> Result<Option<GlobSet>> {
    if patterns.is_empty() {
        return Ok(None);
    }

    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|err| {
            RokoError::config(format!(
                "invalid {kind} glob for watcher '{}': {err}",
                directory.display()
            ))
        })?);
    }

    builder.build().map(Some).map_err(|err| {
        RokoError::config(format!(
            "failed to build {kind} globset for watcher '{}': {err}",
            directory.display()
        ))
    })
}

fn compile_globset_with_defaults(
    patterns: &[String],
    directory: &Path,
    kind: &str,
    defaults: &[&str],
) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in defaults {
        builder.add(Glob::new(pattern).map_err(|err| {
            RokoError::config(format!(
                "invalid default {kind} glob for watcher '{}': {err}",
                directory.display()
            ))
        })?);
    }
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|err| {
            RokoError::config(format!(
                "invalid {kind} glob for watcher '{}': {err}",
                directory.display()
            ))
        })?);
    }

    builder.build().map_err(|err| {
        RokoError::config(format!(
            "failed to build {kind} globset for watcher '{}': {err}",
            directory.display()
        ))
    })
}

fn default_file_watch_excludes() -> &'static [&'static str] {
    &[
        ".git",
        ".git/**",
        "**/.git",
        "**/.git/**",
        "**/*.swp",
        "**/*.swx",
        "**/*~",
        "**/.#*",
        "**/#*#",
        "**/.DS_Store",
        "**/Thumbs.db",
    ]
}

fn normalize_glob_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn watch_path_is_enabled(path: &Path, watch_root: &Path, filters: &CompiledFileWatchPath) -> bool {
    let mut candidates = Vec::with_capacity(2);
    candidates.push(normalize_glob_path(path));

    if let Ok(relative) = path.strip_prefix(watch_root) {
        let relative = normalize_glob_path(relative);
        if !relative.is_empty() {
            candidates.push(relative);
        }
    }

    let included = match &filters.include {
        Some(include) => candidates.iter().any(|candidate| include.is_match(candidate)),
        None => true,
    };
    if !included {
        return false;
    }

    !candidates
        .iter()
        .any(|candidate| filters.exclude.is_match(candidate))
}

fn file_watch_signal(path: &Path, signal_kind: &str, event_kind: &str) -> Signal {
    Signal::builder(Kind::Custom(signal_kind.to_string()))
        .body(Body::Json(serde_json::json!({
            "path": path.to_string_lossy().into_owned(),
            "event_kind": event_kind,
        })))
        .build()
}

fn classify_file_watch_event(kind: &EventKind) -> Option<(&'static str, &'static str)> {
    match kind {
        EventKind::Create(_) => Some((FS_CREATED, "created")),
        EventKind::Modify(_) => Some((FS_MODIFIED, "modified")),
        EventKind::Remove(_) => Some((FS_DELETED, "deleted")),
        _ => None,
    }
}

async fn drain_file_watch_events(
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<
        std::result::Result<notify::Event, notify::Error>,
    >,
    sender: SignalSender,
    cancel: CancellationToken,
    watched_paths: Vec<CompiledFileWatchPath>,
) -> Result<()> {
    let mut pending: HashMap<PathBuf, PendingFileWatchSignal> = HashMap::new();
    let debounce_sleep = tokio::time::sleep(FILE_WATCH_DEBOUNCE_WINDOW);
    tokio::pin!(debounce_sleep);
    let mut debounce_active = false;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            maybe_result = event_rx.recv() => {
                let Some(result) = maybe_result else {
                    break;
                };

                let event = result.map_err(|err| {
                    RokoError::transport(format!("filesystem watcher error: {err}"))
                })?;

                let Some((signal_kind, event_kind)) = classify_file_watch_event(&event.kind) else {
                    continue;
                };

                for path in event.paths.into_iter() {
                    if !watched_paths
                        .iter()
                        .any(|watched| watch_path_is_enabled(&path, &watched.directory, watched))
                    {
                        continue;
                    }
                    pending.insert(path, PendingFileWatchSignal { signal_kind, event_kind });
                }

                debounce_sleep
                    .as_mut()
                    .reset(tokio::time::Instant::now() + FILE_WATCH_DEBOUNCE_WINDOW);
                debounce_active = true;
            }
            _ = &mut debounce_sleep, if debounce_active => {
                debounce_active = false;
                flush_pending_file_watch_signals(&mut pending, &sender).await?;
            }
        }
    }

    if !pending.is_empty() {
        flush_pending_file_watch_signals(&mut pending, &sender).await?;
    }

    Ok(())
}

async fn flush_pending_file_watch_signals(
    pending: &mut HashMap<PathBuf, PendingFileWatchSignal>,
    sender: &SignalSender,
) -> Result<()> {
    let mut batched: Vec<_> = pending.drain().collect();
    batched.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (path, signal) in batched {
        let signal = file_watch_signal(&path, signal.signal_kind, signal.event_kind);
        sender.send(signal).await.map_err(|_| {
            RokoError::cancelled("filesystem watcher receiver dropped")
        })?;
    }

    Ok(())
}

/// Periodically collects outcomes for previously emitted work.
///
/// Collectors poll external systems like GitHub, Slack, or CI at a fixed
/// cadence and return typed feedback for any results found since the last run.
#[async_trait]
pub trait FeedbackCollector: Send + Sync + 'static {
    /// Human-readable collector name.
    fn name(&self) -> &str;

    /// Services this collector talks to, such as `["github", "slack"]`.
    fn services(&self) -> Vec<String>;

    /// Poll interval for this collector.
    fn interval(&self) -> std::time::Duration;

    /// Collect feedback observed since the given timestamp.
    async fn collect(&self, since: DateTime<Utc>) -> Result<Vec<FeedbackSignal>>;
}

/// Top-level plugin metadata plus the event sources and feedback collectors it exposes.
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub event_sources: Vec<Box<dyn EventSource>>,
    pub feedback_collectors: Vec<Box<dyn FeedbackCollector>>,
}

/// Fluent builder for [`PluginManifest`].
pub struct PluginBuilder {
    name: String,
    version: String,
    event_sources: Vec<Box<dyn EventSource>>,
    feedback_collectors: Vec<Box<dyn FeedbackCollector>>,
}

impl PluginBuilder {
    /// Create a builder for a plugin with the package version as the default version.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            event_sources: Vec::new(),
            feedback_collectors: Vec::new(),
        }
    }

    /// Add an event source to the plugin.
    pub fn event_source<T>(mut self, source: T) -> Self
    where
        T: EventSource,
    {
        self.event_sources.push(Box::new(source));
        self
    }

    /// Add a feedback collector to the plugin.
    pub fn feedback_collector<T>(mut self, collector: T) -> Self
    where
        T: FeedbackCollector,
    {
        self.feedback_collectors.push(Box::new(collector));
        self
    }

    /// Build the final plugin manifest.
    pub fn build(self) -> PluginManifest {
        PluginManifest {
            name: self.name,
            version: self.version,
            event_sources: self.event_sources,
            feedback_collectors: self.feedback_collectors,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use notify::{CreateKind, ModifyKind, RemoveKind};
    use roko_core::{Body, Kind, Signal};
    use serde_json::json;
    use tokio::time::{sleep, timeout};

    struct DummyEventSource;
    struct DummyFeedbackCollector;

    #[async_trait]
    impl EventSource for DummyEventSource {
        fn name(&self) -> &str {
            "dummy"
        }

        fn kind(&self) -> EventSourceKind {
            EventSourceKind::Custom("dummy".to_string())
        }

        async fn start(&self, sender: SignalSender, cancel: CancellationToken) -> Result<()> {
            let signal = Signal::builder(Kind::Task)
                .body(Body::text("hello"))
                .build();
            sender.send(signal).await.expect("signal should be sent");
            cancel.cancelled().await;
            Ok(())
        }
    }

    #[async_trait]
    impl FeedbackCollector for DummyFeedbackCollector {
        fn name(&self) -> &str {
            "dummy-feedback"
        }

        fn services(&self) -> Vec<String> {
            vec!["github".to_string(), "slack".to_string()]
        }

        fn interval(&self) -> Duration {
            Duration::from_secs(60)
        }

        async fn collect(&self, _since: DateTime<Utc>) -> Result<Vec<FeedbackSignal>> {
            Ok(vec![FeedbackSignal {
                original_episode_id: "episode-123".to_string(),
                service: "github".to_string(),
                outcome: FeedbackOutcome::Approved,
                metadata: json!({ "reviewer": "alice" }),
                timestamp: DateTime::<Utc>::UNIX_EPOCH,
            }])
        }
    }

    #[tokio::test]
    async fn event_source_is_object_safe() {
        let source: Box<dyn EventSource> = Box::new(DummyEventSource);
        assert_eq!(source.name(), "dummy");
        assert_eq!(source.kind(), EventSourceKind::Custom("dummy".to_string()));

        let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let runner = tokio::spawn(async move { source.start(sender, cancel_for_task).await });

        let signal = receiver.recv().await.expect("signal should be sent");
        assert_eq!(signal.body, Body::text("hello"));

        cancel.cancel();
        runner
            .await
            .expect("task should complete")
            .expect("source should exit cleanly");
    }

    #[test]
    fn cron_signal_payload_includes_cron_fields() {
        let schedule = CronSchedule {
            name: "weekly-digest".to_string(),
            expression: "0 9 * * MON".to_string(),
            signal_kind: "scheduler:cron:weekly-digest".to_string(),
            metadata: json!({ "team": "platform" }),
        };

        let signal = cron_signal(&schedule, DateTime::<Utc>::UNIX_EPOCH);

        assert_eq!(
            signal.kind,
            Kind::Custom("scheduler:cron:weekly-digest".to_string())
        );
        assert_eq!(
            signal.body,
            Body::Json(json!({
                "name": "weekly-digest",
                "expression": "0 9 * * MON",
                "fired_at": DateTime::<Utc>::UNIX_EPOCH.to_rfc3339(),
            }))
        );
    }

    #[tokio::test]
    async fn cron_event_source_rejects_invalid_expression() {
        let source = CronEventSource {
            schedules: vec![CronSchedule {
                name: "broken".to_string(),
                expression: "definitely not cron".to_string(),
                signal_kind: "scheduler:cron:broken".to_string(),
                metadata: json!({ "source": "test" }),
            }],
        };

        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let cancel = CancellationToken::new();

        let err = source.start(sender, cancel).await.expect_err("invalid cron should fail");
        assert!(
            err.to_string().contains("broken"),
            "error should include schedule name"
        );
    }

    #[tokio::test]
    async fn file_watch_event_source_emits_create_modify_delete_signals() {
        let tempdir = make_tempdir();
        let watched_dir = tempdir.join("nested");
        fs::create_dir(&watched_dir).expect("nested directory should be created");

        let source = FileWatchEventSource::new([tempdir.as_path()]);
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let cancel = CancellationToken::new();
        let runner = tokio::spawn({
            let cancel = cancel.clone();
            async move { source.start(sender, cancel).await }
        });

        sleep(Duration::from_millis(100)).await;

        let file_path = watched_dir.join(format!(
            "watch-{}.txt",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos()
        ));
        let file_path_string = file_path.to_string_lossy().into_owned();

        fs::File::create(&file_path).expect("file should be created");
        assert_eq!(
            wait_for_watch_event(&mut receiver, &file_path_string, FS_CREATED, "created").await,
            Some("created")
        );

        sleep(Duration::from_millis(600)).await;

        fs::write(&file_path, b"beta").expect("file should be modified");
        assert_eq!(
            wait_for_watch_event(&mut receiver, &file_path_string, FS_MODIFIED, "modified")
                .await,
            Some("modified")
        );

        sleep(Duration::from_millis(600)).await;

        fs::remove_file(&file_path).expect("file should be deleted");
        assert_eq!(
            wait_for_watch_event(&mut receiver, &file_path_string, FS_DELETED, "deleted").await,
            Some("deleted")
        );

        cancel.cancel();
        runner
            .await
            .expect("task should complete")
            .expect("watcher should exit cleanly");

        fs::remove_dir_all(&tempdir).expect("tempdir should be removed");
    }

    #[tokio::test]
    async fn file_watch_event_source_debounces_same_file_events_and_keeps_latest_kind() {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let cancel = CancellationToken::new();
        let watched_paths = compile_file_watch_paths(&[WatcherPathConfig {
            directory: PathBuf::from("/tmp"),
            include: Vec::new(),
            exclude: Vec::new(),
        }])
        .expect("watch path should compile");
        let runner = tokio::spawn({
            let cancel = cancel.clone();
            async move { drain_file_watch_events(event_rx, sender, cancel, watched_paths).await }
        });

        let path = PathBuf::from("/tmp/roko-plugin-debounce.txt");

        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Create(CreateKind::Any)).add_path(path.clone()),
            ))
            .expect("create event should be accepted");
        sleep(Duration::from_millis(100)).await;
        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Modify(ModifyKind::Any)).add_path(path.clone()),
            ))
            .expect("modify event should be accepted");
        sleep(Duration::from_millis(100)).await;
        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Remove(RemoveKind::Any)).add_path(path.clone()),
            ))
            .expect("remove event should be accepted");
        drop(event_tx);

        let signal = timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("signal should be emitted")
            .expect("one signal should be emitted");
        assert_eq!(signal.kind, Kind::Custom(FS_DELETED.to_string()));
        let body: serde_json::Value = signal.body.as_json().expect("signal body should be json");
        let path_string = path.to_string_lossy().into_owned();
        assert_eq!(body.get("path").and_then(|value| value.as_str()), Some(path_string.as_str()));
        assert_eq!(body.get("event_kind").and_then(|value| value.as_str()), Some("deleted"));
        assert!(
            timeout(Duration::from_millis(700), receiver.recv())
                .await
                .expect("receiver should close after flush")
                .is_none(),
            "debounce should collapse to a single signal"
        );

        runner
            .await
            .expect("task should complete")
            .expect("debounce loop should exit cleanly");
    }

    #[tokio::test]
    async fn file_watch_event_source_applies_include_and_exclude_filters() {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let cancel = CancellationToken::new();
        let watch_root = PathBuf::from("/tmp/roko-plugin-filtered");
        let watched_paths = compile_file_watch_paths(&[WatcherPathConfig {
            directory: watch_root.clone(),
            include: vec!["**/*".to_string()],
            exclude: vec![
                ".git".to_string(),
                ".git/**".to_string(),
                "**/*.swp".to_string(),
                "**/*~".to_string(),
            ],
        }])
        .expect("watch path should compile");
        let runner = tokio::spawn({
            let cancel = cancel.clone();
            async move { drain_file_watch_events(event_rx, sender, cancel, watched_paths).await }
        });

        let allowed = watch_root.join("notes.md");
        let excluded_git = watch_root.join(".git").join("HEAD");
        let excluded_swap = watch_root.join("draft.swp");

        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Create(CreateKind::Any))
                    .add_path(excluded_git.clone()),
            ))
            .expect("git event should be accepted");
        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Create(CreateKind::Any))
                    .add_path(excluded_swap.clone()),
            ))
            .expect("swap event should be accepted");
        event_tx
            .send(Ok(
                notify::Event::new(notify::EventKind::Create(CreateKind::Any))
                    .add_path(allowed.clone()),
            ))
            .expect("allowed event should be accepted");
        drop(event_tx);

        let signal = timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("signal should be emitted")
            .expect("sender should remain open");
        assert_eq!(signal.kind, Kind::Custom(FS_CREATED.to_string()));
        let body: serde_json::Value = signal.body.as_json().expect("signal body should be json");
        let allowed_string = allowed.to_string_lossy().into_owned();
        assert_eq!(
            body.get("path").and_then(|value| value.as_str()),
            Some(allowed_string.as_str())
        );

        assert!(
            timeout(Duration::from_millis(700), receiver.recv())
                .await
                .expect("receiver should close after flush")
                .is_none(),
            "excluded paths should not emit signals"
        );

        runner
            .await
            .expect("task should complete")
            .expect("filtering loop should exit cleanly");
    }

    #[tokio::test]
    async fn feedback_collector_is_object_safe() {
        let collector: Box<dyn FeedbackCollector> = Box::new(DummyFeedbackCollector);
        assert_eq!(collector.name(), "dummy-feedback");
        assert_eq!(collector.services(), vec!["github", "slack"]);
        assert_eq!(collector.interval(), Duration::from_secs(60));

        let feedback = collector
            .collect(DateTime::<Utc>::UNIX_EPOCH)
            .await
            .expect("collector should succeed");
        assert_eq!(feedback.len(), 1);
        assert_eq!(feedback[0].original_episode_id, "episode-123");
        assert_eq!(feedback[0].service, "github");
        assert_eq!(feedback[0].outcome, FeedbackOutcome::Approved);
        assert_eq!(feedback[0].metadata, json!({ "reviewer": "alice" }));
        assert_eq!(feedback[0].timestamp, DateTime::<Utc>::UNIX_EPOCH);
    }

    async fn wait_for_watch_event(
        receiver: &mut tokio::sync::mpsc::Receiver<Signal>,
        expected_path: &str,
        expected_signal_kind: &str,
        expected_event_kind: &str,
    ) -> Option<&'static str> {
        timeout(Duration::from_secs(5), async {
            loop {
                let signal = receiver.recv().await?;
                if signal.kind == Kind::Custom(expected_signal_kind.to_string()) {
                    let body: serde_json::Value = signal.body.as_json().ok()?;
                    if body.get("path").and_then(|value| value.as_str()) == Some(expected_path)
                        && body.get("event_kind").and_then(|value| value.as_str())
                            == Some(expected_event_kind)
                    {
                        return Some(expected_event_kind);
                    }
                }
            }
        })
        .await
        .ok()
        .flatten()
    }

    fn make_tempdir() -> std::path::PathBuf {
        let base = std::env::temp_dir();
        let unique = format!(
            "roko-plugin-fswatcher-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time should be valid")
                .as_nanos()
        );
        let path = base.join(unique);
        fs::create_dir(&path).expect("tempdir should be created");
        path
    }

    #[test]
    fn plugin_builder_supports_fluent_api() {
        let manifest = PluginBuilder::new("my-plugin")
            .event_source(DummyEventSource)
            .feedback_collector(DummyFeedbackCollector)
            .build();

        assert_eq!(manifest.name, "my-plugin");
        assert_eq!(manifest.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(manifest.event_sources.len(), 1);
        assert_eq!(manifest.feedback_collectors.len(), 1);
        assert_eq!(manifest.event_sources[0].name(), "dummy");
        assert_eq!(manifest.feedback_collectors[0].name(), "dummy-feedback");
    }
}
