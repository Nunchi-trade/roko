//! Shared atomic-write helpers.
//!
//! Many subsystems persist JSON (or raw bytes) by writing to a temporary file
//! in the same directory and then renaming over the target path. This module
//! consolidates that pattern into two reusable functions so that each call-site
//! doesn't reinvent the same write-tmp-rename dance.
//!
//! # Naming
//!
//! The temporary file is placed next to the target using
//! [`Path::with_extension`] (e.g. `foo.json` -> `foo.json.tmp`).  This keeps
//! the temp file on the same filesystem as the target, which is required for
//! [`std::fs::rename`] to be atomic on POSIX.

use std::io;
use std::path::Path;

/// Atomically persist a `serde::Serialize` value as pretty-printed JSON.
///
/// 1. Creates the parent directory if it does not exist.
/// 2. Serializes `value` to pretty-printed JSON.
/// 3. Writes the JSON to a sibling `.json.tmp` file.
/// 4. Renames (atomic on POSIX) the temp file over `path`.
///
/// # Errors
///
/// Returns [`std::io::Error`] if directory creation, serialization, write, or
/// rename fails.
pub fn atomic_write_json<T: serde::Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    atomic_write_bytes(path, json.as_bytes())
}

/// Atomically persist raw bytes to `path`.
///
/// 1. Creates the parent directory if it does not exist.
/// 2. Writes `data` to a sibling `.tmp` file (derived from the target
///    extension, e.g. `foo.json` -> `foo.json.tmp`).
/// 3. Renames the temp file over `path`.
///
/// # Errors
///
/// Returns [`std::io::Error`] if directory creation, write, or rename fails.
pub fn atomic_write_bytes(path: &Path, data: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Build a temp path by appending `.tmp` to the existing extension
    // (e.g. `data.json` -> `data.json.tmp`).
    let tmp = tmp_path_for(path);
    std::fs::write(&tmp, data)?;

    // Rename is atomic on the same filesystem (POSIX guarantee).
    // If rename fails, clean up the temp file best-effort.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    Ok(())
}

/// Derive a sibling temp-file path from `target`.
///
/// Appends `.tmp` to whatever extension `target` already has, so
/// `learn/router.json` becomes `learn/router.json.tmp` and
/// `rules.toml` becomes `rules.toml.tmp`.
fn tmp_path_for(target: &Path) -> std::path::PathBuf {
    let ext = target.extension().and_then(|e| e.to_str()).unwrap_or("dat");
    target.with_extension(format!("{ext}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_json_round_trip() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("data.json");

        let value = serde_json::json!({"key": "value", "num": 42});
        atomic_write_json(&path, &value).expect("write");

        let read_back: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read")).expect("parse");
        assert_eq!(read_back, value);

        // Temp file should not linger.
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn atomic_write_bytes_creates_parent_dirs() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("deep/nested/dir/output.bin");

        atomic_write_bytes(&path, b"hello world").expect("write");
        assert_eq!(std::fs::read(&path).expect("read"), b"hello world");
    }

    #[test]
    fn atomic_write_bytes_overwrites_existing() {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("data.bin");

        atomic_write_bytes(&path, b"first").expect("write 1");
        atomic_write_bytes(&path, b"second").expect("write 2");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
    }

    #[test]
    fn tmp_path_for_appends_tmp_extension() {
        let p = Path::new("/a/b/router.json");
        assert_eq!(tmp_path_for(p), Path::new("/a/b/router.json.tmp"));

        let p2 = Path::new("rules.toml");
        assert_eq!(tmp_path_for(p2), Path::new("rules.toml.tmp"));

        let p3 = Path::new("no_extension");
        assert_eq!(tmp_path_for(p3), Path::new("no_extension.dat.tmp"));
    }
}
