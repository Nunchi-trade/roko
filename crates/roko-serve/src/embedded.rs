//! Serve the React SPA — disk-first, embedded-fallback.
//!
//! **Dev workflow**: rebuild the frontend (`npm run build` in `demo/demo-app`),
//! then refresh the browser. No Rust recompile needed.
//!
//! **Production**: the binary carries a baked-in copy of `dist/` via
//! `rust-embed`. When the on-disk `dist/` doesn't exist (deployed container,
//! different machine), the embedded copy is served automatically.
//!
//! Override the disk path with `ROKO_SPA_DIR=/path/to/dist`.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(rust_embed::Embed)]
#[folder = "../../demo/demo-app/dist/"]
struct EmbeddedAssets;

/// Resolved once at startup: the on-disk dist/ directory, if it exists.
fn disk_dist_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        // 1. Explicit env override
        if let Ok(dir) = std::env::var("ROKO_SPA_DIR") {
            let p = PathBuf::from(&dir);
            if p.join("index.html").is_file() {
                tracing::info!(path = %p.display(), "serving SPA from ROKO_SPA_DIR");
                return Some(p);
            }
            tracing::warn!(path = %dir, "ROKO_SPA_DIR set but index.html not found, falling back to embedded");
        }

        // 2. Relative to the compile-time crate directory (works during local dev)
        let compile_time: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../demo/demo-app/dist");
        let p = Path::new(compile_time);
        if p.join("index.html").is_file() {
            let canon = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            tracing::info!(path = %canon.display(), "serving SPA from disk (dev mode)");
            return Some(canon);
        }

        tracing::debug!("no on-disk dist/ found, serving embedded SPA assets");
        None
    })
    .as_ref()
}

/// Try to read a file from disk, returning `(bytes, actual_path_served)`.
fn read_from_disk(path: &str) -> Option<(Vec<u8>, String)> {
    let dir = disk_dist_dir()?;

    // Try exact path first
    if !path.is_empty() {
        let full = dir.join(path);
        if full.is_file() {
            return std::fs::read(&full).ok().map(|b| (b, path.to_string()));
        }
    }

    // SPA fallback: serve index.html for client-side routes
    let index = dir.join("index.html");
    std::fs::read(&index)
        .ok()
        .map(|b| (b, "index.html".to_string()))
}

/// Try to read from the embedded (compile-time) assets.
fn read_from_embedded(path: &str) -> Option<(Vec<u8>, String)> {
    if !path.is_empty() {
        if let Some(file) = EmbeddedAssets::get(path) {
            return Some((file.data.into_owned(), path.to_string()));
        }
    }

    // SPA fallback
    EmbeddedAssets::get("index.html").map(|f| (f.data.into_owned(), "index.html".to_string()))
}

/// Axum fallback handler: disk → embedded → 404.
pub async fn serve_embedded(req: axum::extract::Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');

    let Some((body, served_path)) = read_from_disk(path).or_else(|| read_from_embedded(path))
    else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };

    let mime = mime_guess::from_path(&served_path)
        .first_or_octet_stream()
        .to_string();

    // Cache hashed assets aggressively; HTML/other never
    let cache = if served_path.contains("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        body,
    )
        .into_response()
}
