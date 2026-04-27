//! Serve the React SPA from embedded assets (via `rust-embed`).
//!
//! In release builds the `dist/` output of `demo/demo-app` is baked into the
//! binary. In debug builds with the `debug-embed` feature, `rust-embed` reads
//! from disk so you can iterate with `vite build` + page refresh.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "../../demo/demo-app/dist/"]
struct DemoAssets;

/// Axum fallback handler that serves embedded SPA assets.
///
/// Tries the request path first, then falls back to `index.html` for
/// client-side routing.
pub async fn serve_embedded(req: axum::extract::Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');

    // Try exact file first, then SPA fallback to index.html
    let file = if path.is_empty() {
        DemoAssets::get("index.html")
    } else {
        DemoAssets::get(path).or_else(|| DemoAssets::get("index.html"))
    };

    match file {
        Some(content) => {
            // Determine MIME type from the *actual* file served
            let served_path = if DemoAssets::get(path).is_some() && !path.is_empty() {
                path
            } else {
                "index.html"
            };
            let mime = mime_guess::from_path(served_path)
                .first_or_octet_stream()
                .to_string();

            // Cache immutable hashed assets aggressively; HTML never
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
                content.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
