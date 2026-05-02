//! Atomic file I/O utilities.
//!
//! All state-critical writes should use [`atomic_write`] or [`atomic_write_async`]
//! to prevent partial / corrupted files on crash.

use std::io;
use std::path::Path;

/// Atomically write `data` to `path` by writing to a `.tmp` sibling, then
/// renaming.  Safe when `path` and the temp file are on the same filesystem
/// (rename is atomic on POSIX / NTFS).
///
/// Parent directories are created if they don't exist.
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        // Clean up the temp file on rename failure.
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Async variant of [`atomic_write`].
pub async fn atomic_write_async(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = tmp_path(path);
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await.inspect_err(|_| {
        // Schedule cleanup but don't block on it.
        let tmp_clone = tmp.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_file(&tmp_clone).await;
        });
    })
}

/// Atomically write string data to `path`.
pub fn atomic_write_str(path: &Path, data: &str) -> io::Result<()> {
    atomic_write(path, data.as_bytes())
}

/// Async variant of [`atomic_write_str`].
pub async fn atomic_write_str_async(path: &Path, data: &str) -> io::Result<()> {
    atomic_write_async(path, data.as_bytes()).await
}

/// Read a file, returning `Ok(None)` for `NotFound` instead of `Err`.
///
/// This eliminates the TOCTOU race in the common pattern:
/// ```ignore
/// if path.exists() {
///     let s = fs::read_to_string(&path)?;
///     // ...
/// }
/// ```
pub fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Async variant of [`read_optional`].
pub async fn read_optional_async(path: &Path) -> io::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read bytes from a file, returning `Ok(None)` for `NotFound`.
pub fn read_optional_bytes(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Compute the temporary file path for atomic writes.
fn tmp_path(path: &Path) -> std::path::PathBuf {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::path::PathBuf::from(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn atomic_write_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn atomic_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b/c/test.txt");
        atomic_write(&path, b"nested").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    fn atomic_write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn atomic_write_no_tmp_file_remains() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write(&path, b"data").unwrap();
        let tmp = tmp_path(&path);
        assert!(!tmp.exists(), "temp file should be cleaned up");
    }

    #[test]
    fn read_optional_returns_none_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.txt");
        assert_eq!(read_optional(&path).unwrap(), None);
    }

    #[test]
    fn read_optional_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "content").unwrap();
        assert_eq!(read_optional(&path).unwrap(), Some("content".to_owned()));
    }

    #[test]
    fn read_optional_bytes_returns_none_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.bin");
        assert_eq!(read_optional_bytes(&path).unwrap(), None);
    }

    #[test]
    fn atomic_write_str_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        atomic_write_str(&path, "string data").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "string data");
    }

    #[tokio::test]
    async fn atomic_write_async_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("async_test.txt");
        atomic_write_async(&path, b"async hello").await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(&path).await.unwrap(),
            "async hello"
        );
    }

    #[tokio::test]
    async fn read_optional_async_returns_none_for_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.txt");
        assert_eq!(read_optional_async(&path).await.unwrap(), None);
    }

    #[tokio::test]
    async fn read_optional_async_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, "async content").await.unwrap();
        assert_eq!(
            read_optional_async(&path).await.unwrap(),
            Some("async content".to_owned())
        );
    }
}
