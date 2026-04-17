//! Incremental reader for append-only JSONL files.

use std::fs::File;
use std::io::{BufRead, BufReader, ErrorKind, Seek, SeekFrom};
use std::path::PathBuf;

/// Cursor for append-only JSONL files.
#[derive(Debug, Clone, Default)]
pub struct JsonlCursor {
    path: PathBuf,
    offset: u64,
    last_line: usize,
}

impl JsonlCursor {
    /// Create a cursor for `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            last_line: 0,
        }
    }

    /// Read and return lines appended since the last successful tick.
    ///
    /// Truncation and missing files reset the cursor to the beginning.
    pub fn read_new_lines(&mut self) -> std::io::Result<Vec<String>> {
        let len = match std::fs::metadata(&self.path) {
            Ok(meta) => meta.len(),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.reset();
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        };

        if len < self.offset {
            self.reset();
        }

        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.reset();
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        };

        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(self.offset))?;

        let mut out = Vec::new();
        let mut buf = String::new();

        loop {
            buf.clear();
            let n = reader.read_line(&mut buf)?;
            if n == 0 {
                break;
            }

            if !buf.ends_with('\n') {
                // Do not consume an incomplete trailing line.
                break;
            }

            self.offset += n as u64;
            self.last_line += 1;
            out.push(trim_line_ending(&buf));
        }

        Ok(out)
    }

    /// Byte offset of the next unread line.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Count of committed lines read since the last reset.
    #[allow(dead_code)]
    pub fn last_line(&self) -> usize {
        self.last_line
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.last_line = 0;
    }
}

fn trim_line_ending(line: &str) -> String {
    line.trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::JsonlCursor;
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
    fn reads_appended_lines_without_duplicates() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("signals.jsonl");
        fs::write(&path, "one\n").expect("seed file");

        let mut cursor = JsonlCursor::new(&path);

        assert_eq!(cursor.read_new_lines().expect("first tick"), vec!["one"]);
        let first_offset = cursor.offset();
        assert_eq!(cursor.last_line(), 1);
        assert_eq!(first_offset, fs::metadata(&path).expect("metadata").len());

        append(&path, "two\n");

        assert_eq!(cursor.read_new_lines().expect("second tick"), vec!["two"]);
        assert_eq!(cursor.last_line(), 2);
        assert_eq!(
            cursor.offset(),
            fs::metadata(&path).expect("metadata").len()
        );
        assert_eq!(
            cursor.read_new_lines().expect("idle tick"),
            Vec::<String>::new()
        );
        assert_eq!(
            cursor.offset(),
            fs::metadata(&path).expect("metadata").len()
        );
    }

    #[test]
    fn resets_on_truncation() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("episodes.jsonl");
        fs::write(&path, "one\ntwo\n").expect("seed file");

        let mut cursor = JsonlCursor::new(&path);
        assert_eq!(
            cursor.read_new_lines().expect("initial tick"),
            vec!["one", "two"]
        );
        assert_eq!(cursor.last_line(), 2);

        fs::write(&path, "reset\n").expect("truncate file");

        assert_eq!(
            cursor.read_new_lines().expect("after truncation"),
            vec!["reset"]
        );
        assert_eq!(cursor.last_line(), 1);
        assert_eq!(
            cursor.offset(),
            fs::metadata(&path).expect("metadata").len()
        );
    }

    #[test]
    fn resets_when_file_is_missing() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("events.jsonl");
        fs::write(&path, "one\ntwo\n").expect("seed file");

        let mut cursor = JsonlCursor::new(&path);
        assert_eq!(
            cursor.read_new_lines().expect("initial tick"),
            vec!["one", "two"]
        );
        assert_eq!(cursor.last_line(), 2);

        fs::remove_file(&path).expect("remove file");

        assert_eq!(
            cursor.read_new_lines().expect("missing file"),
            Vec::<String>::new()
        );
        assert_eq!(cursor.offset(), 0);
        assert_eq!(cursor.last_line(), 0);

        fs::write(&path, "fresh\n").expect("recreate file");

        assert_eq!(
            cursor.read_new_lines().expect("recreated file"),
            vec!["fresh"]
        );
        assert_eq!(cursor.last_line(), 1);
        assert_eq!(
            cursor.offset(),
            fs::metadata(&path).expect("metadata").len()
        );
    }

    #[test]
    fn waits_for_partial_trailing_line() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("signals.jsonl");
        fs::write(&path, "one\npart").expect("seed file");

        let mut cursor = JsonlCursor::new(&path);
        assert_eq!(cursor.read_new_lines().expect("initial tick"), vec!["one"]);
        let offset_after_complete_line = cursor.offset();
        assert_eq!(cursor.last_line(), 1);

        assert_eq!(
            cursor.read_new_lines().expect("partial line"),
            Vec::<String>::new()
        );
        assert_eq!(cursor.offset(), offset_after_complete_line);
        assert_eq!(cursor.last_line(), 1);

        append(&path, "ial\n");

        assert_eq!(
            cursor.read_new_lines().expect("completed line"),
            vec!["partial"]
        );
        assert_eq!(cursor.last_line(), 2);
        assert_eq!(
            cursor.offset(),
            fs::metadata(&path).expect("metadata").len()
        );
    }
}
