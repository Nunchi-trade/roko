//! Playbook — named sequences of actions proven to achieve a goal.
//!
//! A [`Playbook`] captures a goal and an ordered list of [`PlaybookStep`]s
//! that have historically led to success. [`PlaybookStore`] persists
//! playbooks as JSON files under a directory, one file per playbook id, and
//! tracks success/failure counters that feed back into the learning loop.
//!
//! # Example
//!
//! ```no_run
//! use roko_learn::playbook::{Playbook, PlaybookStep, PlaybookStore};
//!
//! # async fn run() -> std::io::Result<()> {
//! let store = PlaybookStore::new("/tmp/playbooks");
//! let mut pb = Playbook::new("fix-concurrency", "Resolve Send+Sync errors");
//! pb.steps.push(PlaybookStep::new(
//!     0,
//!     "Replace HashMap with DashMap",
//!     "edit_file",
//!     vec!["compile_ok".into()],
//! ));
//! store.save(&pb).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

const PLAYBOOK_MERGE_THRESHOLD: f64 = 0.80;

/// A single step within a [`Playbook`].
///
/// Steps are ordered by [`PlaybookStep::index`] and each carries the
/// descriptive text, an action kind discriminator, and the expected signals
/// that should appear when the step succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaybookStep {
    /// Zero-based position of the step within the playbook.
    pub index: u32,
    /// Human-readable description of what the step does.
    pub description: String,
    /// Action kind discriminator (e.g. `"edit_file"`, `"run_command"`).
    pub action_kind: String,
    /// Signals expected to appear when the step succeeds.
    pub expected_signals: Vec<String>,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod extraction_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn extract_from_tool_calls_produces_valid_playbook() {
        let tool_calls = vec![
            ("shell".to_string(), "cargo init".to_string()),
            (
                "write_file".to_string(),
                "Write src/lib.rs with function stub".to_string(),
            ),
            ("shell".to_string(), "cargo test".to_string()),
        ];

        let playbook =
            extract_playbook_from_episode("task-1", "Implement add function", &tool_calls)
                .expect("playbook should be extracted");

        assert_eq!(playbook.steps.len(), 3);
        assert_eq!(playbook.steps[0].index, 0);
        assert_eq!(playbook.steps[0].action_kind, "shell");
        assert_eq!(playbook.steps[1].index, 1);
        assert_eq!(playbook.steps[1].action_kind, "write_file");
        assert_eq!(playbook.steps[2].index, 2);
        assert_eq!(playbook.steps[2].action_kind, "shell");
        assert_eq!(playbook.success_count, 1);
        assert_eq!(playbook.failure_count, 0);
        assert!(playbook.id.starts_with("ep-task-1-"));
        assert_eq!(playbook.goal, "Implement add function");
        assert_eq!(playbook.name, "Learned: Implement add function");
    }

    #[test]
    fn extract_empty_tool_calls_returns_none() {
        assert!(extract_playbook_from_episode("task-2", "Empty task", &[]).is_none());
    }

    #[test]
    fn extract_truncates_long_prompt_in_goal() {
        let long_prompt = "x".repeat(500);
        let tool_calls = vec![("shell".to_string(), "echo hello".to_string())];

        let playbook = extract_playbook_from_episode("task-3", &long_prompt, &tool_calls)
            .expect("playbook should be extracted");

        assert_eq!(playbook.goal, format!("{}...", "x".repeat(200)));
        assert!(playbook.goal.len() <= 203);
    }

    #[tokio::test]
    async fn save_or_merge_creates_new_entry_for_extracted_playbook() {
        let dir = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(dir.path());
        let tool_calls = vec![("shell".to_string(), "cargo test".to_string())];

        let playbook = extract_playbook_from_episode("t1", "Run tests", &tool_calls)
            .expect("playbook should be extracted");
        store.save_or_merge(&playbook).await.expect("save_or_merge");

        let loaded = store
            .load(&playbook.id)
            .await
            .expect("load")
            .expect("playbook");
        assert_eq!(loaded.success_count, 1);
        assert_eq!(loaded.failure_count, 0);
        assert_eq!(loaded.steps.len(), 1);
        assert_eq!(loaded.steps[0].action_kind, "shell");

        let listed = store.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, playbook.id);
    }

    #[tokio::test]
    async fn save_or_merge_merges_similar_extracted_playbooks() {
        let dir = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(dir.path());
        let tool_calls = vec![
            ("shell".to_string(), "cargo test".to_string()),
            ("shell".to_string(), "cargo fmt --check".to_string()),
        ];

        let first =
            extract_playbook_from_episode("t1", "Run tests", &tool_calls).expect("first playbook");
        store.save_or_merge(&first).await.expect("save first");

        let mut second =
            extract_playbook_from_episode("t1", "Run tests", &tool_calls).expect("second playbook");
        second.id = format!("{}-retry", second.id);
        store.save_or_merge(&second).await.expect("merge second");

        let merged = store
            .load(&first.id)
            .await
            .expect("load merged")
            .expect("merged playbook");
        assert_eq!(merged.success_count, 2);
        assert_eq!(merged.failure_count, 0);
        assert_eq!(merged.steps.len(), 2);
        assert!(store.load(&second.id).await.expect("load second").is_none());

        let listed = store.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, first.id);
    }
}

impl PlaybookStep {
    /// Construct a new playbook step.
    pub fn new(
        index: u32,
        description: impl Into<String>,
        action_kind: impl Into<String>,
        expected_signals: Vec<String>,
    ) -> Self {
        Self {
            index,
            description: description.into(),
            action_kind: action_kind.into(),
            expected_signals,
        }
    }
}

/// A named sequence of actions proven to achieve a goal.
///
/// Success/failure counts accumulate across runs to form a track record the
/// router or composer can consult when choosing whether to replay this
/// playbook for a new task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playbook {
    /// Stable identifier — used as the file stem when persisted.
    pub id: String,
    /// Short human label.
    pub name: String,
    /// The goal this playbook tries to achieve.
    pub goal: String,
    /// Ordered list of steps.
    pub steps: Vec<PlaybookStep>,
    /// Number of successful completions recorded via
    /// [`PlaybookStore::record_outcome`].
    pub success_count: u64,
    /// Number of failed completions recorded via
    /// [`PlaybookStore::record_outcome`].
    pub failure_count: u64,
    /// Unix-millis timestamp when the playbook was first created.
    pub created_at_ms: i64,
    /// Unix-millis timestamp of the most recent
    /// [`PlaybookStore::record_outcome`] call, or `None` if never used.
    pub last_used_ms: Option<i64>,
}

impl Playbook {
    /// Construct a new playbook with no steps and zero counters.
    ///
    /// `created_at_ms` is set to `chrono::Utc::now().timestamp_millis()`.
    pub fn new(id: impl Into<String>, goal: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone();
        Self {
            id,
            name,
            goal: goal.into(),
            steps: Vec::new(),
            success_count: 0,
            failure_count: 0,
            created_at_ms: Utc::now().timestamp_millis(),
            last_used_ms: None,
        }
    }

    /// Total outcomes recorded so far.
    pub const fn total_outcomes(&self) -> u64 {
        self.success_count + self.failure_count
    }

    /// Success rate as a fraction in `[0.0, 1.0]`. Returns `None` when no
    /// outcomes have been recorded yet.
    #[allow(clippy::cast_precision_loss)]
    pub fn success_rate(&self) -> Option<f64> {
        let total = self.total_outcomes();
        if total == 0 {
            None
        } else {
            Some(self.success_count as f64 / total as f64)
        }
    }
}

/// Query metadata for selecting playbooks during prompt composition.
///
/// The query is intentionally small and stable: it captures the task
/// identity, the human-facing task title, optional task details, and the
/// calling role so the store can rank playbooks deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueryContext {
    /// Stable task identifier when available.
    pub task_id: String,
    /// Human-readable task title used as the primary search signal.
    pub task_title: String,
    /// Extended task details such as descriptions or acceptance criteria.
    pub task_body: String,
    /// Calling role that is composing the prompt.
    pub role: String,
    /// Reserved for future episode-aware ranking.
    pub recent_episodes: usize,
    /// Maximum number of ranked playbooks to return.
    pub max_results: usize,
}

impl QueryContext {
    /// Construct a new query context.
    #[must_use]
    pub fn new(
        task_id: impl Into<String>,
        task_title: impl Into<String>,
        task_body: impl Into<String>,
        role: impl Into<String>,
        recent_episodes: usize,
        max_results: usize,
    ) -> Self {
        Self {
            task_id: task_id.into(),
            task_title: task_title.into(),
            task_body: task_body.into(),
            role: role.into(),
            recent_episodes,
            max_results,
        }
    }

    fn to_query_text(&self) -> String {
        [
            self.task_title.as_str(),
            self.task_body.as_str(),
            self.role.as_str(),
            self.task_id.as_str(),
        ]
        .into_iter()
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// Extract a playbook from a successful episode's tool-call sequence.
///
/// Returns `None` when `tool_calls` is empty after trimming blank entries.
/// The generated playbook uses the prompt as the goal, a truncated learned
/// title as the display name, and one [`PlaybookStep`] per tool invocation.
#[must_use]
pub fn extract_playbook_from_episode(
    task_id: &str,
    task_prompt: &str,
    tool_calls: &[(String, String)],
) -> Option<Playbook> {
    let steps: Vec<PlaybookStep> = tool_calls
        .iter()
        .filter_map(|(tool_name, description)| {
            let tool_name = tool_name.trim();
            if tool_name.is_empty() {
                return None;
            }

            Some((tool_name.to_string(), description.trim().to_string()))
        })
        .enumerate()
        .map(|(index, (tool_name, description))| {
            PlaybookStep::new(index as u32, description, tool_name, Vec::new())
        })
        .collect();

    if steps.is_empty() {
        return None;
    }

    let prompt = task_prompt.trim();
    let prompt_source = {
        let source = if prompt.is_empty() {
            task_id.trim()
        } else {
            prompt
        };
        if source.is_empty() {
            "untitled task"
        } else {
            source
        }
    };
    let now_ms = Utc::now().timestamp_millis();
    let id_component = sanitize_playbook_id_component(task_id);
    let mut playbook = Playbook::new(
        format!("ep-{id_component}-{now_ms}"),
        truncate_chars(prompt_source, 200),
    );
    playbook.name = format!("Learned: {}", truncate_chars(prompt_source, 60));
    playbook.steps = steps;
    playbook.success_count = 1;
    playbook.failure_count = 0;
    playbook.created_at_ms = now_ms;
    playbook.last_used_ms = None;
    Some(playbook)
}

/// Extract a tool-call sequence from an episode payload.
///
/// The helper prefers explicit `extra["tool_calls"]`-style arrays when they
/// are present, then falls back to the episode's `external_actions` bag.
/// Each returned tuple is `(tool_name, description_or_args_summary)`.
#[must_use]
pub fn extract_tool_calls_from_episode(
    episode: &crate::episode_logger::Episode,
) -> Vec<(String, String)> {
    for key in ["tool_calls", "tool_sequence", "tools_used", "tools"] {
        if let Some(value) = episode.extra.get(key) {
            let calls = extract_tool_calls_from_value(value);
            if !calls.is_empty() {
                return calls;
            }
        }
    }

    extract_tool_calls_from_values(&episode.external_actions)
}

fn validate_playbook_id(id: &str) -> io::Result<()> {
    if id.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "playbook id must not be empty",
        ));
    }

    if !is_valid_playbook_id(id) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "playbook id must not contain path separators",
        ));
    }

    Ok(())
}

fn is_valid_playbook_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains(std::path::MAIN_SEPARATOR)
        && !id.contains('/')
        && !id.contains('\\')
        && id != ".."
        && id != "."
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut iter = text.trim().chars();
    let mut out = String::with_capacity(max_chars.saturating_add(3));
    for _ in 0..max_chars {
        let Some(ch) = iter.next() else {
            return out;
        };
        out.push(ch);
    }

    if iter.next().is_some() {
        out.push_str("...");
    }

    out
}

fn sanitize_playbook_id_component(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }

    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "task".to_string()
    } else {
        trimmed.to_string()
    }
}

fn playbook_merge_query(playbook: &Playbook) -> String {
    let mut parts = Vec::new();
    if !playbook.goal.trim().is_empty() {
        parts.push(playbook.goal.trim());
    }
    if !playbook.name.trim().is_empty() {
        parts.push(playbook.name.trim());
    }
    for step in &playbook.steps {
        if !step.description.trim().is_empty() {
            parts.push(step.description.trim());
        }
        if !step.action_kind.trim().is_empty() {
            parts.push(step.action_kind.trim());
        }
        for signal in &step.expected_signals {
            let signal = signal.trim();
            if !signal.is_empty() {
                parts.push(signal);
            }
        }
    }
    parts.join(" ")
}

fn merge_playbooks(mut existing: Playbook, incoming: &Playbook) -> Playbook {
    if existing.name.trim().is_empty() || existing.name == existing.id {
        if !incoming.name.trim().is_empty() {
            existing.name = incoming.name.clone();
        }
    }
    if existing.goal.trim().is_empty() && !incoming.goal.trim().is_empty() {
        existing.goal = incoming.goal.clone();
    }
    if incoming.steps.len() > existing.steps.len() {
        existing.steps = incoming.steps.clone();
    }
    existing.success_count = existing
        .success_count
        .saturating_add(incoming.success_count.max(1));
    existing.failure_count = existing
        .failure_count
        .saturating_add(incoming.failure_count);
    existing.created_at_ms = existing.created_at_ms.min(incoming.created_at_ms);
    existing.last_used_ms = Some(Utc::now().timestamp_millis());
    existing
}

fn extract_tool_calls_from_values(values: &[Value]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for value in values {
        out.extend(extract_tool_calls_from_value(value));
    }
    out
}

fn extract_tool_calls_from_value(value: &Value) -> Vec<(String, String)> {
    match value {
        Value::Array(items) => extract_tool_calls_from_values(items),
        Value::Object(object) => {
            for key in ["tool_calls", "tool_sequence", "tools_used", "tools"] {
                if let Some(items) = object.get(key).and_then(Value::as_array) {
                    let calls = extract_tool_calls_from_values(items);
                    if !calls.is_empty() {
                        return calls;
                    }
                }
            }

            extract_tool_call_from_object(object).into_iter().collect()
        }
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Vec::new()
            } else {
                vec![(text.to_string(), text.to_string())]
            }
        }
        _ => Vec::new(),
    }
}

fn extract_tool_call_from_object(
    object: &serde_json::Map<String, Value>,
) -> Option<(String, String)> {
    let name = extract_named_string(
        object,
        &["tool_name", "tool", "name", "action_type", "kind"],
    )
    .or_else(|| {
        object
            .get("function")
            .and_then(Value::as_object)
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string)
    })?;

    let description = object
        .get("description")
        .or_else(|| object.get("summary"))
        .or_else(|| object.get("args_summary"))
        .and_then(|value| summarize_value(value, 160))
        .or_else(|| {
            let mut parts = Vec::new();

            if let Some(value) = object.get("arguments").or_else(|| object.get("args")) {
                if let Some(summary) = summarize_value(value, 160) {
                    parts.push(summary);
                }
            }

            if let Some(value) = object.get("resource_id").and_then(Value::as_str) {
                let value = value.trim();
                if !value.is_empty() {
                    parts.push(value.to_string());
                }
            }

            if let Some(value) = object.get("metadata") {
                if let Some(summary) = summarize_value(value, 160) {
                    parts.push(summary);
                }
            }

            if let Some(function_args) = object
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("arguments"))
            {
                if let Some(summary) = summarize_value(function_args, 160) {
                    parts.push(summary);
                }
            }

            if parts.is_empty() {
                summarize_inline_call_object(object)
            } else {
                Some(parts.join(" "))
            }
        })
        .unwrap_or_else(|| name.clone());

    Some((name, description))
}

fn extract_named_string(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = object.get(*key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

fn summarize_value(value: &Value, max_chars: usize) -> Option<String> {
    let text = match value {
        Value::String(text) => text.trim().to_string(),
        Value::Null => return None,
        _ => serde_json::to_string(value).ok()?,
    };

    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(truncate_chars(text, max_chars))
    }
}

fn summarize_inline_call_object(object: &serde_json::Map<String, Value>) -> Option<String> {
    let mut parts = Vec::new();
    for key in ["command", "path", "target", "query", "text", "body", "url"] {
        if let Some(value) = object.get(key).and_then(|value| summarize_value(value, 80)) {
            parts.push(format!("{key}={value}"));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// JSON-file backed store for playbooks.
///
/// Each playbook is persisted to `<root>/<id>.json`. The store uses a
/// [`parking_lot::Mutex`] to hand out unique temp-file suffixes so that
/// concurrent saves of the same playbook id do not stomp each other's
/// `.tmp` scratch files before the atomic rename, plus a per-id
/// [`tokio::sync::Mutex`] to serialize load+save cycles in
/// [`PlaybookStore::record_outcome`] so concurrent outcomes don't lose
/// updates.
#[derive(Debug, Clone)]
pub struct PlaybookStore {
    root: PathBuf,
    tmp_counter: Arc<Mutex<u64>>,
    id_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl PlaybookStore {
    /// Create a store rooted at `path`. The directory is not created
    /// eagerly — it is created on the first write.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            root: path.into(),
            tmp_counter: Arc::new(Mutex::new(0)),
            id_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn id_lock(&self, id: &str) -> Arc<AsyncMutex<()>> {
        Arc::clone(
            self.id_locks
                .lock()
                .entry(id.to_string())
                .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
        )
    }

    fn next_tmp_suffix(&self) -> u64 {
        let mut counter = self.tmp_counter.lock();
        *counter = counter.wrapping_add(1);
        *counter
    }

    /// Root directory used for persistence.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    /// Persist `playbook` under `<root>/<playbook.id>.json`, replacing any
    /// existing file with the same id.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`io::Error`] if the root directory cannot be
    /// created, the playbook cannot be serialized, or the file cannot be
    /// written.
    pub async fn save(&self, playbook: &Playbook) -> io::Result<()> {
        validate_playbook_id(&playbook.id)?;
        let bytes = serde_json::to_vec_pretty(playbook)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        tokio::fs::create_dir_all(&self.root).await?;
        let path = self.path_for(&playbook.id);
        // Use a unique tmp suffix per call so that concurrent saves of the
        // same id do not race on a single shared scratch file. The atomic
        // rename then publishes the final file.
        let suffix = self.next_tmp_suffix();
        let tmp = self.root.join(format!("{}.{suffix}.tmp", playbook.id));
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }

    /// Save `playbook`, or merge it into the most similar existing playbook.
    ///
    /// The merge path increments the chosen playbook's success count instead
    /// of creating a duplicate entry for the same task pattern.
    ///
    /// # Errors
    ///
    /// Returns an error for any I/O or serialization failure.
    pub async fn save_or_merge(&self, playbook: &Playbook) -> io::Result<()> {
        validate_playbook_id(&playbook.id)?;
        let merge_lock = self.id_lock("__playbook_merge__/global");
        let _merge_guard = merge_lock.lock().await;

        let exact_lock = self.id_lock(&playbook.id);
        let _exact_guard = exact_lock.lock().await;
        if let Some(existing) = self.load(&playbook.id).await? {
            let merged = merge_playbooks(existing, playbook);
            self.save(&merged).await?;
            return Ok(());
        }

        if let Some(candidate) = self.best_similar_playbook(playbook).await? {
            let candidate_lock = self.id_lock(&candidate.id);
            let _candidate_guard = candidate_lock.lock().await;
            if let Some(existing) = self.load(&candidate.id).await? {
                let merged = merge_playbooks(existing, playbook);
                self.save(&merged).await?;
                return Ok(());
            }
        }

        self.save(playbook).await
    }

    /// Load the playbook stored under `id`, or `None` if the file does not
    /// exist.
    ///
    /// # Errors
    ///
    /// Returns an error for any I/O failure other than `NotFound`, or if
    /// the file contents cannot be deserialized.
    pub async fn load(&self, id: &str) -> io::Result<Option<Playbook>> {
        let path = self.path_for(id);
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let pb: Playbook = serde_json::from_slice(&bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok(Some(pb))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Look up a playbook by task type.
    ///
    /// This is a thin alias for [`PlaybookStore::load`] used by the
    /// orchestrator's pre-dispatch learning hook.
    ///
    /// # Errors
    ///
    /// Returns any I/O error raised by [`PlaybookStore::load`].
    pub async fn lookup(&self, task_type: &str) -> io::Result<Option<Playbook>> {
        self.load(task_type).await
    }

    /// Return playbooks ranked by textual relevance to the supplied query
    /// context.
    ///
    /// Ranking prefers textual matches against the task title, role, and task
    /// id, then recency, then stable evidence and id ordering. Results are
    /// capped by [`QueryContext::max_results`].
    ///
    /// # Errors
    ///
    /// Returns an error if any persisted playbook file cannot be read or
    /// deserialized.
    pub async fn query(&self, ctx: &QueryContext) -> io::Result<Vec<Playbook>> {
        let mut ranked = self.rank_playbooks(&ctx.to_query_text()).await?;
        ranked.truncate(ctx.max_results);
        Ok(ranked)
    }

    /// Return up to `limit` playbooks ranked by textual relevance to `query`
    /// and recency.
    ///
    /// Relevance considers the playbook id, name, goal, step descriptions,
    /// action kinds, and expected signals. Recency uses `last_used_ms` when
    /// available and falls back to `created_at_ms`.
    ///
    /// If the query yields no textual matches, the store falls back to the
    /// most recently used playbooks so callers still get a best-effort prompt
    /// injection candidate list.
    ///
    /// # Errors
    ///
    /// Returns an error if any persisted playbook file cannot be read or
    /// deserialized.
    pub async fn relevant(
        &self,
        query: impl AsRef<str>,
        limit: usize,
    ) -> io::Result<Vec<Playbook>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut ranked = self.rank_playbooks(query.as_ref()).await?;
        ranked.truncate(limit);
        Ok(ranked)
    }

    async fn rank_playbooks(&self, query: &str) -> io::Result<Vec<Playbook>> {
        let query = normalize_query(query);
        let query_terms = tokenize(&query);
        let now_ms = Utc::now().timestamp_millis();
        let mut scored: Vec<(f64, f64, Playbook)> = self
            .list()
            .await?
            .into_iter()
            .map(|playbook| {
                let recency = recency_score(&playbook, now_ms);
                let relevance = if query.is_empty() {
                    recency
                } else {
                    relevance_score(&playbook, &query, &query_terms)
                };
                (relevance, recency, playbook)
            })
            .collect();

        let has_match = scored.iter().any(|(relevance, _, _)| *relevance > 0.0);
        if !has_match && !query.is_empty() {
            scored = scored
                .into_iter()
                .map(|(_, recency, playbook)| (recency, recency, playbook))
                .collect();
        } else {
            scored.retain(|(relevance, _, _)| *relevance > 0.0);
        }

        scored.sort_by(|(relevance_a, recency_a, a), (relevance_b, recency_b, b)| {
            relevance_b
                .total_cmp(relevance_a)
                .then_with(|| recency_b.total_cmp(recency_a))
                .then_with(|| playbook_anchor_ms(b).cmp(&playbook_anchor_ms(a)))
                .then_with(|| b.total_outcomes().cmp(&a.total_outcomes()))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(scored
            .into_iter()
            .map(|(_, _, playbook)| playbook)
            .collect())
    }

    /// List all playbooks in the store. Returns an empty vector if the
    /// directory does not yet exist.
    ///
    /// The order is filesystem-dependent. Files that fail to deserialize
    /// are skipped silently (they may be partial writes from a crash).
    ///
    /// # Errors
    ///
    /// Returns an error for any I/O failure other than `NotFound` on the
    /// root directory.
    pub async fn list(&self) -> io::Result<Vec<Playbook>> {
        let mut out = Vec::new();
        let mut entries = match tokio::fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e),
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match tokio::fs::read(&path).await {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if let Ok(pb) = serde_json::from_slice::<Playbook>(&bytes) {
                out.push(pb);
            }
        }
        Ok(out)
    }

    /// Load all persisted playbooks.
    ///
    /// This is a compatibility alias for callers expecting the documented
    /// `load_all()` surface.
    pub async fn load_all(&self) -> io::Result<Vec<Playbook>> {
        self.list().await
    }

    /// Record the outcome of executing the playbook identified by `id`.
    ///
    /// On success, increments [`Playbook::success_count`]; on failure,
    /// increments [`Playbook::failure_count`]. Updates
    /// [`Playbook::last_used_ms`] to the current wall-clock time.
    ///
    /// Returns `Ok(true)` if a playbook with that id existed and was
    /// updated, `Ok(false)` if no such playbook is stored.
    ///
    /// Concurrent calls for the same `id` are serialized through a
    /// per-id async mutex, so the load+save cycle is atomic and two
    /// simultaneous outcomes cannot lose an update.
    ///
    /// # Errors
    ///
    /// Returns an error for any I/O or serialization failure.
    pub async fn record_outcome(&self, id: &str, success: bool) -> io::Result<bool> {
        let lock = self.id_lock(id);
        let _guard = lock.lock().await;
        let Some(mut pb) = self.load(id).await? else {
            return Ok(false);
        };
        if success {
            pb.success_count = pb.success_count.saturating_add(1);
        } else {
            pb.failure_count = pb.failure_count.saturating_add(1);
        }
        pb.last_used_ms = Some(Utc::now().timestamp_millis());
        self.save(&pb).await?;
        Ok(true)
    }

    /// Record an outcome for the playbook identified by `id`.
    ///
    /// This is a convenience wrapper around [`PlaybookStore::record_outcome`]
    /// for call sites that already have the originating task definition and
    /// only need to persist the success/failure result.
    ///
    /// # Errors
    ///
    /// Returns any I/O error raised while loading or updating the playbook.
    pub async fn record(&self, id: &str, success: bool) -> io::Result<bool> {
        self.record_outcome(id, success).await
    }

    /// Delete the playbook stored under `id`. Returns `Ok(true)` if a file
    /// was removed, `Ok(false)` if no file existed.
    ///
    /// # Errors
    ///
    /// Returns an error for any I/O failure other than `NotFound`.
    pub async fn delete(&self, id: &str) -> io::Result<bool> {
        let path = self.path_for(id);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn best_similar_playbook(&self, playbook: &Playbook) -> io::Result<Option<Playbook>> {
        let query = playbook_merge_query(playbook);
        if query.is_empty() {
            return Ok(None);
        }

        let query = normalize_query(&query);
        if query.is_empty() {
            return Ok(None);
        }
        let query_terms = tokenize(&query);
        let mut best: Option<(f64, Playbook)> = None;

        for candidate in self.list().await? {
            let score = relevance_score(&candidate, &query, &query_terms);
            if best
                .as_ref()
                .map_or(true, |(best_score, _)| score > *best_score)
            {
                best = Some((score, candidate));
            }
        }

        Ok(best.and_then(|(score, candidate)| {
            (score >= PLAYBOOK_MERGE_THRESHOLD).then_some(candidate)
        }))
    }
}

fn normalize_query(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| token.to_string())
        .collect()
}

fn relevance_score(playbook: &Playbook, query: &str, query_terms: &[String]) -> f64 {
    let mut best = score_field(&playbook.id, query, query_terms) * 1.0;
    best = best.max(score_field(&playbook.name, query, query_terms) * 0.95);
    best = best.max(score_field(&playbook.goal, query, query_terms) * 0.9);
    for step in &playbook.steps {
        best = best.max(score_field(&step.description, query, query_terms) * 0.8);
        best = best.max(score_field(&step.action_kind, query, query_terms) * 0.75);
        for signal in &step.expected_signals {
            best = best.max(score_field(signal, query, query_terms) * 0.7);
        }
    }
    best
}

fn score_field(field: &str, query: &str, query_terms: &[String]) -> f64 {
    let normalized = normalize_query(field);
    if normalized.is_empty() || query.is_empty() {
        return 0.0;
    }

    if normalized == query {
        return 1.0;
    }
    if normalized.starts_with(query) || query.starts_with(&normalized) {
        return 0.9;
    }
    if normalized.contains(query) || query.contains(&normalized) {
        return 0.8;
    }

    let field_terms = tokenize(&normalized);
    if field_terms.is_empty() || query_terms.is_empty() {
        return 0.0;
    }

    let overlap = query_terms
        .iter()
        .filter(|term| field_terms.iter().any(|candidate| candidate == *term))
        .count();
    if overlap == 0 {
        0.0
    } else {
        overlap as f64 / query_terms.len() as f64
    }
}

fn playbook_anchor_ms(playbook: &Playbook) -> i64 {
    playbook.last_used_ms.unwrap_or(playbook.created_at_ms)
}

fn recency_score(playbook: &Playbook, now_ms: i64) -> f64 {
    let age_ms = now_ms.saturating_sub(playbook_anchor_ms(playbook)).max(0);
    let age_days = age_ms as f64 / 86_400_000.0;
    1.0 / (1.0 + age_days)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_steps() -> Vec<PlaybookStep> {
        vec![
            PlaybookStep::new(
                0,
                "Read failing test output",
                "read_file",
                vec!["test_output".into()],
            ),
            PlaybookStep::new(
                1,
                "Patch the faulty module",
                "edit_file",
                vec!["file_patched".into(), "compile_ok".into()],
            ),
            PlaybookStep::new(
                2,
                "Re-run the test",
                "run_command",
                vec!["tests_green".into()],
            ),
        ]
    }

    fn sample_playbook(id: &str) -> Playbook {
        let mut pb = Playbook::new(id, "Make the failing test pass");
        pb.name = format!("playbook-{id}");
        pb.steps = sample_steps();
        pb
    }

    fn learned_playbook(id: &str, goal: &str, steps: Vec<(&str, &str)>) -> Playbook {
        let mut pb = Playbook::new(id, goal);
        pb.name = format!("Learned: {goal}");
        pb.steps = steps
            .into_iter()
            .enumerate()
            .map(|(index, (action_kind, description))| {
                PlaybookStep::new(index as u32, description, action_kind, Vec::new())
            })
            .collect();
        pb.success_count = 1;
        pb
    }

    #[test]
    fn extract_playbook_from_episode_builds_steps_and_metadata() {
        let tool_calls = vec![
            (
                "read_file".to_string(),
                r#"{"path":"src/lib.rs"}"#.to_string(),
            ),
            ("bash".to_string(), "cargo test".to_string()),
        ];

        let playbook =
            extract_playbook_from_episode("task-1", "Fix the failing tests", &tool_calls)
                .expect("playbook");

        assert!(playbook.id.starts_with("ep-task-1-"));
        assert_eq!(playbook.name, "Learned: Fix the failing tests");
        assert_eq!(playbook.goal, "Fix the failing tests");
        assert_eq!(playbook.steps.len(), 2);
        assert_eq!(playbook.steps[0].index, 0);
        assert_eq!(playbook.steps[0].action_kind, "read_file");
        assert_eq!(playbook.steps[0].description, r#"{"path":"src/lib.rs"}"#);
        assert_eq!(playbook.steps[1].index, 1);
        assert_eq!(playbook.steps[1].action_kind, "bash");
        assert_eq!(playbook.steps[1].description, "cargo test");
        assert_eq!(playbook.success_count, 1);
        assert_eq!(playbook.failure_count, 0);
        assert!(playbook.last_used_ms.is_none());
    }

    #[test]
    fn extract_playbook_from_episode_returns_none_for_empty_tool_calls() {
        assert!(extract_playbook_from_episode("task-1", "prompt", &[]).is_none());
        assert!(
            extract_playbook_from_episode(
                "task-1",
                "prompt",
                &[("   ".to_string(), "   ".to_string())],
            )
            .is_none()
        );
    }

    #[test]
    fn extract_tool_calls_from_episode_prefers_extra_tool_calls() {
        let mut episode = crate::episode_logger::Episode::new("agent-1", "task-1");
        episode.extra.insert(
            "tool_calls".to_string(),
            serde_json::json!([
                {
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "read_file",
                        "arguments": {"path": "src/lib.rs"}
                    }
                },
                {
                    "id": "call-2",
                    "name": "bash",
                    "arguments": "cargo test"
                }
            ]),
        );
        episode.external_actions = vec![serde_json::json!({
            "service": "github",
            "action_type": "review_pr",
            "resource_id": "pr-12",
            "metadata": {"state": "approved"}
        })];

        let calls = extract_tool_calls_from_episode(&episode);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "read_file");
        assert!(calls[0].1.contains("src/lib.rs"));
        assert_eq!(calls[1].0, "bash");
        assert_eq!(calls[1].1, "cargo test");
    }

    #[test]
    fn extract_tool_calls_from_episode_falls_back_to_external_actions() {
        let mut episode = crate::episode_logger::Episode::new("agent-1", "task-1");
        episode.external_actions = vec![serde_json::json!({
            "service": "github",
            "action_type": "review_pr",
            "resource_id": "pr-12",
            "metadata": {"state": "approved"}
        })];

        let calls = extract_tool_calls_from_episode(&episode);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "review_pr");
        assert!(calls[0].1.contains("pr-12"));
        assert!(calls[0].1.contains("approved"));
    }

    #[tokio::test]
    async fn save_load_roundtrip_preserves_all_fields() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let mut original = sample_playbook("fix-test-001");
        original.success_count = 3;
        original.failure_count = 1;
        original.last_used_ms = Some(1_700_000_000_000);

        store.save(&original).await.expect("save");
        let loaded = store
            .load("fix-test-001")
            .await
            .expect("load ok")
            .expect("some");
        assert_eq!(loaded, original);
        assert_eq!(loaded.steps.len(), 3);
        assert_eq!(loaded.steps[1].expected_signals.len(), 2);
    }

    #[tokio::test]
    async fn load_missing_returns_none() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let result = store.load("does-not-exist").await.expect("load ok");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_saved_playbooks() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        for id in ["alpha", "beta", "gamma"] {
            store.save(&sample_playbook(id)).await.expect("save");
        }
        // Drop a non-json file to confirm it's ignored.
        tokio::fs::write(tmp.path().join("ignore.txt"), b"noise")
            .await
            .expect("write noise");
        let mut listed = store.list().await.expect("list");
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].id, "alpha");
        assert_eq!(listed[1].id, "beta");
        assert_eq!(listed[2].id, "gamma");
    }

    #[tokio::test]
    async fn list_on_missing_directory_is_empty() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path().join("never-created"));
        let listed = store.list().await.expect("list ok");
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn relevant_ranks_by_match_strength_then_recency() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let now = Utc::now().timestamp_millis();
        let mut strong_old = sample_playbook("strong-old");
        strong_old.id = "implementation".to_string();
        strong_old.name = "Implementation heavy playbook".to_string();
        strong_old.goal = "Implementation implementation workflow".to_string();
        strong_old.last_used_ms = Some(now - 86_400_000 * 3);

        let mut weak_new = sample_playbook("weak-new");
        weak_new.name = "Implementation quick pass".to_string();
        weak_new.goal = "Implementation helper".to_string();
        weak_new.last_used_ms = Some(now);

        let mut unrelated_recent = sample_playbook("unrelated");
        unrelated_recent.name = "Docs cleanup".to_string();
        unrelated_recent.goal = "Refresh documentation and examples".to_string();
        unrelated_recent.last_used_ms = Some(now - 86_400_000);

        store.save(&strong_old).await.expect("save strong");
        store.save(&weak_new).await.expect("save weak");
        store.save(&unrelated_recent).await.expect("save unrelated");

        let ranked = store.relevant("implementation", 3).await.expect("relevant");
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].id, "implementation");
        assert_eq!(ranked[1].id, "weak-new");
    }

    #[tokio::test]
    async fn relevant_falls_back_to_recent_when_query_has_no_match() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let now = Utc::now().timestamp_millis();
        let mut older = sample_playbook("older");
        older.goal = "Older playbook".to_string();
        older.last_used_ms = Some(now - 86_400_000 * 2);

        let mut newer = sample_playbook("newer");
        newer.goal = "Newer playbook".to_string();
        newer.last_used_ms = Some(now);

        store.save(&older).await.expect("save older");
        store.save(&newer).await.expect("save newer");

        let ranked = store.relevant("no-match-here", 2).await.expect("relevant");
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].id, "newer");
        assert_eq!(ranked[1].id, "older");
    }

    #[tokio::test]
    async fn query_ranks_matching_playbooks_and_respects_max_results() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let now = Utc::now().timestamp_millis();
        let mut exact_new = sample_playbook("exact-new");
        exact_new.goal = "Implement REST API".to_string();
        exact_new.last_used_ms = Some(now);

        let mut exact_old = sample_playbook("exact-old");
        exact_old.goal = "Implement REST API".to_string();
        exact_old.last_used_ms = Some(now - 86_400_000 * 2);

        let mut partial = sample_playbook("partial");
        partial.goal = "Implement REST service".to_string();
        partial.last_used_ms = Some(now - 86_400_000);

        let mut unrelated = sample_playbook("unrelated");
        unrelated.goal = "Write release notes".to_string();
        unrelated.last_used_ms = Some(now - 86_400_000 * 4);

        store.save(&exact_new).await.expect("save exact new");
        store.save(&exact_old).await.expect("save exact old");
        store.save(&partial).await.expect("save partial");
        store.save(&unrelated).await.expect("save unrelated");

        let ranked = store
            .query(&QueryContext::new(
                "task-42",
                "Implement REST API",
                "Implement REST API endpoints and routing",
                "implementer",
                10,
                2,
            ))
            .await
            .expect("query");

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].id, "exact-new");
        assert_eq!(ranked[1].id, "exact-old");
    }

    #[tokio::test]
    async fn query_falls_back_to_recency_and_id_when_no_match_exists() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let anchor = Utc::now().timestamp_millis();
        let mut newer = sample_playbook("newer");
        newer.goal = "Refresh docs".to_string();
        newer.created_at_ms = anchor;
        newer.last_used_ms = Some(anchor);

        let mut beta = sample_playbook("beta");
        beta.goal = "Audit dependencies".to_string();
        beta.created_at_ms = anchor - 10_000;
        beta.last_used_ms = Some(anchor - 86_400_000);

        let mut alpha = sample_playbook("alpha");
        alpha.goal = "Audit dependencies".to_string();
        alpha.created_at_ms = anchor - 10_000;
        alpha.last_used_ms = Some(anchor - 86_400_000);

        store.save(&newer).await.expect("save newer");
        store.save(&beta).await.expect("save beta");
        store.save(&alpha).await.expect("save alpha");

        let ranked = store
            .query(&QueryContext::new(
                "task-99",
                "Completely unrelated request",
                "Unrelated task details",
                "reviewer",
                10,
                3,
            ))
            .await
            .expect("query");

        assert_eq!(ranked.len(), 3);
        assert_eq!(ranked[0].id, "newer");
        assert_eq!(ranked[1].id, "alpha");
        assert_eq!(ranked[2].id, "beta");
    }

    #[tokio::test]
    async fn record_success_increments_success_count() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        store.save(&sample_playbook("p1")).await.expect("save");

        assert!(store.record_outcome("p1", true).await.expect("record"));
        assert!(store.record_outcome("p1", true).await.expect("record"));
        let pb = store.load("p1").await.expect("load").expect("some");
        assert_eq!(pb.success_count, 2);
        assert_eq!(pb.failure_count, 0);
        assert!(pb.last_used_ms.is_some());
    }

    #[tokio::test]
    async fn record_failure_increments_failure_count() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        store.save(&sample_playbook("p2")).await.expect("save");

        assert!(store.record_outcome("p2", false).await.expect("record"));
        let pb = store.load("p2").await.expect("load").expect("some");
        assert_eq!(pb.success_count, 0);
        assert_eq!(pb.failure_count, 1);
    }

    #[tokio::test]
    async fn record_outcome_missing_returns_false() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let ok = store
            .record_outcome("nonexistent", true)
            .await
            .expect("record");
        assert!(!ok);
    }

    #[tokio::test]
    async fn save_replaces_existing_entry() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let mut first = sample_playbook("same-id");
        first.goal = "first goal".to_string();
        store.save(&first).await.expect("save1");

        let mut second = sample_playbook("same-id");
        second.goal = "second goal".to_string();
        second.steps.pop();
        store.save(&second).await.expect("save2");

        let loaded = store.load("same-id").await.expect("load").expect("some");
        assert_eq!(loaded.goal, "second goal");
        assert_eq!(loaded.steps.len(), 2);

        // Only one file should exist for this id.
        let listed = store.list().await.expect("list");
        assert_eq!(listed.iter().filter(|p| p.id == "same-id").count(), 1);
    }

    #[tokio::test]
    async fn save_or_merge_merges_similar_playbooks() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let first = learned_playbook(
            "ep-task-1-111",
            "Audit dependencies",
            vec![("read_file", "Read Cargo.toml")],
        );
        store.save(&first).await.expect("save first");

        let second = learned_playbook(
            "ep-task-1-222",
            "Audit dependencies",
            vec![
                ("read_file", "Read Cargo.toml"),
                ("run_command", "Run cargo tree"),
            ],
        );
        store.save_or_merge(&second).await.expect("merge second");

        let loaded = store
            .load(&first.id)
            .await
            .expect("load first")
            .expect("some");
        assert_eq!(loaded.success_count, 2);
        assert_eq!(loaded.steps.len(), 2);
        assert!(store.load(&second.id).await.expect("load second").is_none());
    }

    #[tokio::test]
    async fn save_or_merge_keeps_dissimilar_playbooks_separate() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());

        let first = learned_playbook(
            "ep-task-1-111",
            "Audit dependencies",
            vec![("read_file", "Read Cargo.toml")],
        );
        store.save(&first).await.expect("save first");

        let other = learned_playbook(
            "ep-task-2-222",
            "Rewrite the dashboard theme",
            vec![("edit_file", "Update CSS variables")],
        );
        store.save_or_merge(&other).await.expect("save other");

        let mut listed = store.list().await.expect("list");
        listed.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, first.id);
        assert_eq!(listed[1].id, other.id);
    }

    #[tokio::test]
    async fn steps_ordering_is_preserved() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let mut pb = Playbook::new("ordered", "Preserve order");
        for i in 0..8u32 {
            pb.steps.push(PlaybookStep::new(
                i,
                format!("step {i}"),
                "noop",
                vec![format!("signal-{i}")],
            ));
        }
        store.save(&pb).await.expect("save");
        let loaded = store.load("ordered").await.expect("load").expect("some");
        assert_eq!(loaded.steps.len(), 8);
        for (i, step) in loaded.steps.iter().enumerate() {
            assert_eq!(step.index as usize, i);
            assert_eq!(step.description, format!("step {i}"));
            assert_eq!(step.expected_signals, vec![format!("signal-{i}")]);
        }
    }

    #[tokio::test]
    async fn last_used_ms_updates_on_record_outcome() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let mut pb = sample_playbook("ts");
        pb.last_used_ms = None;
        store.save(&pb).await.expect("save");

        let before = Utc::now().timestamp_millis();
        store.record_outcome("ts", true).await.expect("record");
        let after = Utc::now().timestamp_millis();

        let loaded = store.load("ts").await.expect("load").expect("some");
        let ts = loaded.last_used_ms.expect("set");
        assert!(
            ts >= before && ts <= after,
            "timestamp {ts} outside [{before}, {after}]"
        );
    }

    #[tokio::test]
    async fn empty_id_save_rejected() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        let pb = Playbook::new("", "empty");
        let err = store.save(&pb).await.expect_err("must reject");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn path_traversal_id_rejected() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        for bad in ["..", ".", "a/b", "a\\b"] {
            let pb = Playbook::new(bad, "bad id");
            let err = store
                .save(&pb)
                .await
                .expect_err(&format!("must reject {bad}"));
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[tokio::test]
    async fn success_rate_and_totals() {
        let mut pb = Playbook::new("rate", "measure");
        assert_eq!(pb.total_outcomes(), 0);
        assert!(pb.success_rate().is_none());
        pb.success_count = 3;
        pb.failure_count = 1;
        assert_eq!(pb.total_outcomes(), 4);
        assert!((pb.success_rate().expect("set") - 0.75).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn concurrent_record_outcome_does_not_lose_updates() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        store.save(&sample_playbook("race")).await.expect("save");

        let mut handles = Vec::new();
        for i in 0..32 {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                store.record_outcome("race", i % 2 == 0).await
            }));
        }
        for h in handles {
            h.await.expect("join").expect("record");
        }
        let pb = store.load("race").await.expect("load").expect("some");
        // Every call must be accounted for — no lost updates.
        assert_eq!(pb.success_count + pb.failure_count, 32);
        assert_eq!(pb.success_count, 16);
        assert_eq!(pb.failure_count, 16);
    }

    #[tokio::test]
    async fn delete_removes_playbook() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PlaybookStore::new(tmp.path());
        store.save(&sample_playbook("gone")).await.expect("save");
        assert!(store.delete("gone").await.expect("delete"));
        assert!(store.load("gone").await.expect("load").is_none());
        assert!(!store.delete("gone").await.expect("delete again"));
    }
}
