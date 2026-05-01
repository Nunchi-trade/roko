//! Size-based JSONL rotation helper.
//!
//! Episode and efficiency logs are append-only and grow without bound
//! over a long-running plan. This helper rotates a JSONL file once it
//! crosses a fixed size threshold, keeping the live `<name>.jsonl`
//! young and pushing the older content into numbered siblings
//! (`<name>.jsonl.1`, `<name>.jsonl.2`, …) up to a small cap. Anything
//! beyond the cap is dropped on the next rotation.
//!
//! The helper is deliberately tiny: callers invoke
//! [`rotate_if_needed`] right before appending, and the rotation is a
//! sequence of plain `rename`s — no copying, no compaction. A crash
//! mid-rotation can leave the file system in an intermediate but
//! recoverable state (a missing `<name>.jsonl` that the next append
//! re-creates from scratch); JSONL readers already tolerate truncated
//! tails, so no data integrity is lost.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Default rotation threshold: rotate once the live file is at or
/// beyond 10 MiB.
pub const DEFAULT_ROTATION_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;

/// Maximum number of rotated siblings retained. The newest is `.1`,
/// the oldest is `.MAX_ROTATED_FILES`.
pub const MAX_ROTATED_FILES: usize = 5;

/// Build the rotated sibling path for `base` at index `n` (1-based).
///
/// `episodes.jsonl` → `episodes.jsonl.1`, `episodes.jsonl.2`, …
#[must_use]
pub fn rotation_path(base: &Path, n: usize) -> PathBuf {
    let mut suffixed: OsString = base.as_os_str().to_os_string();
    suffixed.push(format!(".{n}"));
    PathBuf::from(suffixed)
}

/// If `path` exists and is at or beyond `threshold_bytes`, rotate the
/// chain (`<base>.jsonl.N` → `<base>.jsonl.(N+1)`, etc.) keeping the
/// most recent `MAX_ROTATED_FILES` rotated siblings, then move the
/// live `path` to `<base>.jsonl.1`. After this call the live `path`
/// no longer exists; the next append re-creates it.
///
/// No-op if `path` is missing or smaller than the threshold.
///
/// # Errors
///
/// Returns the underlying `std::io::Error` from any `metadata`,
/// `rename`, or `remove_file` call. A failure leaves the file system
/// in a partial but recoverable state — the next call retries from
/// whatever state remains.
pub async fn rotate_if_needed(path: &Path, threshold_bytes: u64) -> std::io::Result<bool> {
    let size = match tokio::fs::metadata(path).await {
        Ok(meta) => meta.len(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if size < threshold_bytes {
        return Ok(false);
    }

    // Drop the oldest rotation if present so that shifting `.N` →
    // `.(N+1)` cannot exceed the cap.
    let drop_target = rotation_path(path, MAX_ROTATED_FILES);
    if exists(&drop_target).await {
        tokio::fs::remove_file(&drop_target).await?;
    }

    // Shift `.N` → `.(N+1)` for N from MAX_ROTATED_FILES-1 down to 1.
    for n in (1..MAX_ROTATED_FILES).rev() {
        let src = rotation_path(path, n);
        let dst = rotation_path(path, n + 1);
        if exists(&src).await {
            tokio::fs::rename(&src, &dst).await?;
        }
    }

    // Move the live file aside.
    let first = rotation_path(path, 1);
    tokio::fs::rename(path, &first).await?;
    Ok(true)
}

async fn exists(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::io::AsyncWriteExt;

    async fn write(path: &Path, bytes: usize) {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.expect("mkdir");
        }
        let mut file = tokio::fs::File::create(path).await.expect("create");
        let chunk = vec![b'x'; 64 * 1024];
        let mut remaining = bytes;
        while remaining > 0 {
            let n = remaining.min(chunk.len());
            file.write_all(&chunk[..n]).await.expect("write");
            remaining -= n;
        }
        file.flush().await.expect("flush");
    }

    #[tokio::test]
    async fn rotation_path_appends_numeric_suffix() {
        let p = PathBuf::from("/tmp/episodes.jsonl");
        assert_eq!(
            rotation_path(&p, 1).to_str().unwrap(),
            "/tmp/episodes.jsonl.1"
        );
        assert_eq!(
            rotation_path(&p, 5).to_str().unwrap(),
            "/tmp/episodes.jsonl.5"
        );
    }

    #[tokio::test]
    async fn small_file_is_not_rotated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("episodes.jsonl");
        write(&path, 1024).await;
        let rotated = rotate_if_needed(&path, 10 * 1024 * 1024).await.unwrap();
        assert!(!rotated);
        assert!(path.exists(), "live file kept");
        assert!(!rotation_path(&path, 1).exists());
    }

    #[tokio::test]
    async fn missing_file_is_not_rotated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.jsonl");
        let rotated = rotate_if_needed(&path, 10 * 1024 * 1024).await.unwrap();
        assert!(!rotated);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn file_past_threshold_rotates_to_first_slot() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("episodes.jsonl");
        // Write just past the threshold (use a small threshold to keep the test fast).
        let threshold = 4 * 1024;
        write(&path, threshold as usize + 1).await;
        let rotated = rotate_if_needed(&path, threshold).await.unwrap();
        assert!(rotated);
        assert!(!path.exists(), "live file moved aside");
        assert!(
            rotation_path(&path, 1).exists(),
            "first rotation slot occupied"
        );
    }

    #[tokio::test]
    async fn rotation_chain_shifts_existing_siblings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("episodes.jsonl");
        let threshold = 4 * 1024;

        // Pre-populate `.1` and `.2` to verify the shift.
        write(&rotation_path(&path, 1), 8).await;
        write(&rotation_path(&path, 2), 9).await;
        // Live file now exceeds threshold.
        write(&path, threshold as usize + 1).await;

        let rotated = rotate_if_needed(&path, threshold).await.unwrap();
        assert!(rotated);

        // The previously-`.2` content is now at `.3`; previously-`.1`
        // is now at `.2`; the live file is at `.1`.
        let one = tokio::fs::metadata(&rotation_path(&path, 1)).await.unwrap();
        let two = tokio::fs::metadata(&rotation_path(&path, 2)).await.unwrap();
        let three = tokio::fs::metadata(&rotation_path(&path, 3)).await.unwrap();
        assert!(
            one.len() > threshold,
            "`.1` is the freshly-rotated live file"
        );
        assert_eq!(two.len(), 8, "previous `.1` is now at `.2`");
        assert_eq!(three.len(), 9, "previous `.2` is now at `.3`");
    }

    #[tokio::test]
    async fn rotation_drops_files_beyond_cap() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("episodes.jsonl");
        let threshold = 4 * 1024;

        // Pre-populate every rotated slot. The slot at MAX_ROTATED_FILES
        // must be dropped on the next rotation.
        for n in 1..=MAX_ROTATED_FILES {
            write(&rotation_path(&path, n), n).await;
        }
        // Live file now exceeds threshold.
        write(&path, threshold as usize + 1).await;

        let rotated = rotate_if_needed(&path, threshold).await.unwrap();
        assert!(rotated);

        // Slot MAX is now what was previously at MAX-1 (length MAX-1),
        // and the original MAX-length file at slot MAX is gone.
        let last = tokio::fs::metadata(&rotation_path(&path, MAX_ROTATED_FILES))
            .await
            .unwrap();
        assert_eq!(last.len() as usize, MAX_ROTATED_FILES - 1);

        // Slot MAX+1 must NOT exist — it's beyond the cap.
        let beyond = rotation_path(&path, MAX_ROTATED_FILES + 1);
        assert!(!beyond.exists());
    }
}
