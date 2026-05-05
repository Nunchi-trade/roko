//! Incremental task-output tail manager for `.roko/task-outputs/`.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;

use super::jsonl_cursor::JsonlCursor;

const TASK_OUTPUT_TAIL_CAP: usize = roko_core::defaults::DEFAULT_TASK_OUTPUT_TAIL_CAP;

/// Bounded incremental tail manager for per-task output files.
#[derive(Debug, Clone, Default)]
pub struct TaskOutputCursors {
    base_dir: PathBuf,
    cursors: HashMap<String, TaskOutputCursor>,
    revision: u64,
}

#[derive(Debug, Clone, Default)]
struct TaskOutputCursor {
    path: PathBuf,
    tail: RingBuffer<String>,
    cursor: JsonlCursor,
}

#[derive(Debug, Clone)]
struct RingBuffer<T> {
    items: Vec<T>,
    cap: usize,
}

impl<T> RingBuffer<T> {
    fn new(cap: usize) -> Self {
        Self {
            items: Vec::new(),
            cap,
        }
    }

    fn push(&mut self, item: T) {
        if self.cap == 0 {
            return;
        }
        if self.items.len() == self.cap {
            self.items.remove(0);
        }
        self.items.push(item);
    }

    fn clear(&mut self) {
        self.items.clear();
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn as_slice(&self) -> &[T] {
        &self.items
    }
}

impl<T> Default for RingBuffer<T> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TaskOutputCursors {
    /// Create a task-output cursor manager rooted at `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            cursors: HashMap::new(),
            revision: 0,
        }
    }

    /// Walk the directory once, adding new task files and dropping stale ones.
    pub fn reconcile(&mut self) -> io::Result<bool> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut changed = false;

        if self.base_dir.exists() {
            for entry in std::fs::read_dir(&self.base_dir)? {
                let entry = entry?;
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "txt") {
                    continue;
                }

                let Some(task_id) = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(ToOwned::to_owned)
                else {
                    continue;
                };

                seen.insert(task_id.clone());
                match self.cursors.get_mut(&task_id) {
                    Some(cursor) if cursor.path == path => {}
                    Some(cursor) => {
                        cursor.path = path.clone();
                        cursor.cursor = JsonlCursor::new(&cursor.path);
                        cursor.tail.clear();
                        changed = true;
                    }
                    None => {
                        self.cursors.insert(
                            task_id,
                            TaskOutputCursor {
                                path: path.clone(),
                                tail: RingBuffer::new(TASK_OUTPUT_TAIL_CAP),
                                cursor: JsonlCursor::new(&path),
                            },
                        );
                        changed = true;
                    }
                }
            }
        }

        let before = self.cursors.len();
        self.cursors.retain(|task_id, _| seen.contains(task_id));
        if self.cursors.len() != before {
            changed = true;
        }

        if changed {
            self.bump_revision();
        }

        Ok(changed)
    }

    /// Incrementally tail each tracked task-output file.
    pub fn tick(&mut self) -> io::Result<bool> {
        let mut changed = false;

        for cursor in self.cursors.values_mut() {
            let before_offset = cursor.cursor.offset();
            let had_tail = !cursor.tail.is_empty();
            let lines = cursor.cursor.read_new_lines()?;

            if cursor.cursor.offset() < before_offset {
                if had_tail {
                    cursor.tail.clear();
                    changed = true;
                }
            }

            if !lines.is_empty() {
                for line in lines {
                    cursor.tail.push(line);
                }
                changed = true;
            }
        }

        if changed {
            self.bump_revision();
        }

        Ok(changed)
    }

    /// Return the bounded tail for `task_id`, if the task is tracked.
    pub fn tail_for(&self, task_id: &str) -> Option<&[String]> {
        self.cursors
            .get(task_id)
            .map(|cursor| cursor.tail.as_slice())
    }

    /// Clone the tracked tails into a plain map for downstream consumers.
    pub fn snapshot(&self) -> HashMap<String, Vec<String>> {
        self.cursors
            .iter()
            .map(|(task_id, cursor)| (task_id.clone(), cursor.tail.as_slice().to_vec()))
            .collect()
    }

    /// Monotonic revision that advances whenever reconcile or tick changes state.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::TaskOutputCursors;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use tempfile::tempdir;

    fn append(path: &std::path::Path, text: &str) {
        let mut file = OpenOptions::new()
            .append(true)
            .open(path)
            .expect("open for append");
        file.write_all(text.as_bytes()).expect("append bytes");
    }

    #[test]
    fn tails_appended_lines_without_duplicates() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("task-123.txt");
        fs::write(&path, "").expect("seed file");

        let mut cursors = TaskOutputCursors::new(dir.path());
        assert!(cursors.reconcile().expect("reconcile"));
        assert!(!cursors.tick().expect("initial tick"));

        for index in 1..=5 {
            append(&path, &format!("line-{index}\n"));
            assert!(cursors.tick().expect("append tick"));

            let expected: Vec<String> = (1..=index).map(|n| format!("line-{n}")).collect();
            assert_eq!(cursors.tail_for("task-123"), Some(expected.as_slice()));
        }
    }

    #[test]
    fn caps_each_tail_at_four_hundred_lines() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("task-456.txt");
        fs::write(&path, "").expect("seed file");

        let mut cursors = TaskOutputCursors::new(dir.path());
        assert!(cursors.reconcile().expect("reconcile"));

        for index in 1..=405 {
            append(&path, &format!("line-{index}\n"));
            let _ = cursors.tick().expect("tick");
        }

        let tail = cursors.tail_for("task-456").expect("tail");
        assert_eq!(tail.len(), 400);
        assert_eq!(tail.first().map(String::as_str), Some("line-6"));
        assert_eq!(tail.last().map(String::as_str), Some("line-405"));
    }

    #[test]
    fn drops_stale_tasks_on_reconcile() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("task-789.txt");
        fs::write(&path, "one\n").expect("seed file");

        let mut cursors = TaskOutputCursors::new(dir.path());
        assert!(cursors.reconcile().expect("reconcile"));
        assert!(cursors.tick().expect("tick"));
        assert!(cursors.tail_for("task-789").is_some());

        fs::remove_file(&path).expect("remove file");

        assert!(cursors.reconcile().expect("reconcile after delete"));
        assert!(cursors.tail_for("task-789").is_none());
    }
}
