//! Cached prompt context data — loads knowledge, episodes, playbooks, and
//! section effectiveness once, then serves them from memory for subsequent
//! prompt assemblies within the same run.
//!
//! ## Lifecycle
//!
//! The cache is built once before the runner event loop starts, and
//! refreshed every `max_age` (default 5 min) or after gate failures that
//! may have updated the knowledge store. When stale, the caller rebuilds
//! with [`PromptCache::load`] and swaps the `Arc`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use roko_learn::episode_logger::Episode;
use roko_learn::playbook::Playbook;
use roko_learn::section_effect::SectionEffectivenessRegistry;
use roko_neuro::KnowledgeEntry;
use tracing::debug;

/// Default maximum age before the cache is considered stale.
const DEFAULT_MAX_AGE: Duration = Duration::from_secs(300);

/// Pre-loaded prompt context data.
///
/// All fields are read-only snapshots taken at `built_at`. The cache is
/// intentionally cheap to clone (inner vecs are behind `Arc` when shared
/// across tasks, but the cache itself is typically wrapped in `Arc` by the
/// caller).
#[derive(Debug, Clone)]
pub struct PromptCache {
    /// Knowledge entries loaded from the neuro store.
    pub neuro_entries: Vec<KnowledgeEntry>,
    /// Episodes loaded from JSONL logs.
    pub episodes: Vec<Episode>,
    /// Playbooks loaded from the playbook directory.
    pub playbooks: Vec<Playbook>,
    /// Section effectiveness registry loaded from disk.
    pub effectiveness: SectionEffectivenessRegistry,
    /// Instant when this cache was built.
    built_at: Instant,
    /// Configured staleness threshold.
    max_age: Duration,
}

impl PromptCache {
    /// Load all prompt context data from disk.
    ///
    /// This performs all the I/O that would otherwise happen per-task:
    /// - Neuro knowledge store query (all entries, not topic-filtered)
    /// - Episode JSONL scan (all episode files)
    /// - Playbook directory iteration
    /// - Section effectiveness registry load
    ///
    /// Any individual load failure is silently ignored (the corresponding
    /// vec/registry will be empty), matching the existing fallback behavior
    /// in the per-task sources.
    pub fn load(workdir: &Path) -> Self {
        let now = Instant::now();

        let neuro_entries = load_neuro_entries(workdir);
        let episodes = load_episodes(workdir);
        let playbooks = load_playbooks(workdir);
        let effectiveness = load_effectiveness(workdir);

        debug!(
            neuro = neuro_entries.len(),
            episodes = episodes.len(),
            playbooks = playbooks.len(),
            elapsed_ms = now.elapsed().as_millis(),
            "prompt cache loaded"
        );

        Self {
            neuro_entries,
            episodes,
            playbooks,
            effectiveness,
            built_at: now,
            max_age: DEFAULT_MAX_AGE,
        }
    }

    /// Returns `true` if the cache has exceeded its maximum age.
    pub fn is_stale(&self) -> bool {
        self.built_at.elapsed() > self.max_age
    }

    /// Override the maximum age for staleness checks.
    #[must_use]
    pub fn with_max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Age of this cache snapshot.
    pub fn age(&self) -> Duration {
        self.built_at.elapsed()
    }
}

// ─── Loaders ──────────────────────────────────────────────────────────────

fn load_neuro_entries(workdir: &Path) -> Vec<KnowledgeEntry> {
    let store = roko_neuro::KnowledgeStore::for_workdir(workdir);
    if !store.path().exists() {
        return Vec::new();
    }
    // Load all entries so we can do in-memory filtering per-task.
    store.query("", 500).unwrap_or_default()
}

fn load_episodes(workdir: &Path) -> Vec<Episode> {
    let mut episodes = Vec::new();
    for path in episode_paths(workdir) {
        if let Ok(file) = std::fs::File::open(&path) {
            let reader = std::io::BufReader::new(file);
            for line in std::io::BufRead::lines(reader).map_while(Result::ok) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Ok(episode) = serde_json::from_str::<Episode>(trimmed) {
                    episodes.push(episode);
                }
            }
        }
    }
    episodes
}

fn episode_paths(workdir: &Path) -> Vec<PathBuf> {
    [
        workdir.join(".roko").join("episodes.jsonl"),
        workdir.join(".roko").join("learn").join("episodes.jsonl"),
        workdir.join(".roko").join("memory").join("episodes.jsonl"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect()
}

fn load_playbooks(workdir: &Path) -> Vec<Playbook> {
    let root = workdir.join(".roko").join("learn").join("playbooks");
    if !root.is_dir() {
        return Vec::new();
    }
    let mut playbooks = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(playbook) = serde_json::from_str::<Playbook>(&text) {
                    playbooks.push(playbook);
                }
            }
        }
    }
    playbooks
}

fn load_effectiveness(workdir: &Path) -> SectionEffectivenessRegistry {
    let path = workdir.join(roko_learn::section_effect::DEFAULT_SECTION_EFFECTS_PATH);
    if !path.exists() {
        return SectionEffectivenessRegistry::new();
    }
    SectionEffectivenessRegistry::load_or_new(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_from_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PromptCache::load(tmp.path());
        assert!(cache.neuro_entries.is_empty());
        assert!(cache.episodes.is_empty());
        assert!(cache.playbooks.is_empty());
        assert!(!cache.is_stale());
    }

    #[test]
    fn staleness_check() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = PromptCache::load(tmp.path()).with_max_age(Duration::from_millis(0));
        // After construction with 0ms max_age, the cache is immediately stale.
        assert!(cache.is_stale());
    }
}
