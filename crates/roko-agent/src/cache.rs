//! Short-lived content-addressed response cache for identical backend requests.

use crate::tool_loop::LlmError;
use crate::translate::BackendResponse;
use roko_core::ContentHash;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Notify};

/// Default retention window for cached backend responses.
pub const DEFAULT_RESPONSE_CACHE_TTL_MS: u64 = 30_000;

/// In-memory cache keyed by the content hash of a full backend request.
pub struct ResponseCache {
    entries: Mutex<HashMap<ContentHash, CacheState>>,
    ttl_ms: u64,
}

struct CacheEntry {
    response: BackendResponse,
    created_at: Instant,
}

enum CacheState {
    Ready(CacheEntry),
    InFlight(Arc<Notify>),
}

impl ResponseCache {
    /// Create a response cache with the given TTL in milliseconds.
    #[must_use]
    pub fn new(ttl_ms: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl_ms,
        }
    }

    /// Return a cached response when present, otherwise compute and store it.
    pub async fn get_or_compute<F, Fut>(
        &self,
        prompt_hash: ContentHash,
        compute: F,
    ) -> Result<BackendResponse, LlmError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<BackendResponse, LlmError>>,
    {
        let mut compute = Some(compute);

        loop {
            enum Lookup {
                Wait(Arc<Notify>),
                Compute(Arc<Notify>),
            }

            let lookup = {
                let mut cache = self.entries.lock().await;

                if let Some(CacheState::Ready(entry)) = cache.get(&prompt_hash) {
                    if entry.created_at.elapsed() < self.ttl() {
                        return Ok(entry.response.clone());
                    }
                }

                match cache.get(&prompt_hash) {
                    Some(CacheState::InFlight(notify)) => Lookup::Wait(Arc::clone(notify)),
                    _ => {
                        cache.remove(&prompt_hash);
                        let notify = Arc::new(Notify::new());
                        cache.insert(prompt_hash, CacheState::InFlight(Arc::clone(&notify)));
                        Lookup::Compute(notify)
                    }
                }
            };

            let notify = match lookup {
                Lookup::Wait(waiter) => {
                    waiter.notified().await;
                    continue;
                }
                Lookup::Compute(notify) => notify,
            };

            let response = compute
                .take()
                .expect("response cache compute closure called once")()
            .await;

            let mut cache = self.entries.lock().await;
            match &response {
                Ok(response) => {
                    cache.insert(
                        prompt_hash,
                        CacheState::Ready(CacheEntry {
                            response: response.clone(),
                            created_at: Instant::now(),
                        }),
                    );
                }
                Err(_) => {
                    cache.remove(&prompt_hash);
                }
            }
            drop(cache);
            notify.notify_waiters();

            return response;
        }
    }

    fn ttl(&self) -> Duration {
        Duration::from_millis(self.ttl_ms)
    }
}

/// Return the process-wide shared response cache used by default backend instances.
#[must_use]
pub fn shared_response_cache() -> Arc<ResponseCache> {
    static CACHE: OnceLock<Arc<ResponseCache>> = OnceLock::new();
    Arc::clone(CACHE.get_or_init(|| Arc::new(ResponseCache::new(DEFAULT_RESPONSE_CACHE_TTL_MS))))
}

/// Hash a backend request scope, endpoint, and serialized body into a stable cache key.
#[must_use]
pub fn request_hash(scope: &str, endpoint: &str, body: &[u8]) -> ContentHash {
    let mut bytes = Vec::with_capacity(scope.len() + endpoint.len() + body.len() + 2);
    bytes.extend_from_slice(scope.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(endpoint.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(body);
    ContentHash::of(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Barrier;
    use tokio::time::sleep;

    #[tokio::test]
    async fn response_cache_hits_within_ttl() {
        let cache = ResponseCache::new(30_000);
        let calls = AtomicUsize::new(0);
        let prompt_hash = ContentHash::of(b"prompt");

        let first = cache
            .get_or_compute(prompt_hash, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(BackendResponse::Text("cached".into()))
            })
            .await
            .unwrap();
        let second = cache
            .get_or_compute(prompt_hash, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(BackendResponse::Text("fresh".into()))
            })
            .await
            .unwrap();

        assert!(matches!(first, BackendResponse::Text(ref text) if text == "cached"));
        assert!(matches!(second, BackendResponse::Text(ref text) if text == "cached"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn response_cache_expires_after_ttl() {
        let cache = ResponseCache::new(10);
        let calls = AtomicUsize::new(0);
        let prompt_hash = ContentHash::of(b"prompt");

        cache
            .get_or_compute(prompt_hash, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(BackendResponse::Text("cached".into()))
            })
            .await
            .unwrap();

        sleep(Duration::from_millis(20)).await;

        let response = cache
            .get_or_compute(prompt_hash, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(BackendResponse::Text("fresh".into()))
            })
            .await
            .unwrap();

        assert!(matches!(response, BackendResponse::Text(ref text) if text == "fresh"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn response_cache_coalesces_inflight_requests() {
        let cache = Arc::new(ResponseCache::new(30_000));
        let barrier = Arc::new(Barrier::new(2));
        let calls = Arc::new(AtomicUsize::new(0));
        let prompt_hash = ContentHash::of(b"prompt");

        let first_cache = Arc::clone(&cache);
        let first_barrier = Arc::clone(&barrier);
        let first_calls = Arc::clone(&calls);
        let first = tokio::spawn(async move {
            first_cache
                .get_or_compute(prompt_hash, || async move {
                    first_calls.fetch_add(1, Ordering::SeqCst);
                    first_barrier.wait().await;
                    Ok(BackendResponse::Text("shared".into()))
                })
                .await
        });

        barrier.wait().await;

        let second_cache = Arc::clone(&cache);
        let second_calls = Arc::clone(&calls);
        let second = tokio::spawn(async move {
            second_cache
                .get_or_compute(prompt_hash, || async move {
                    second_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(BackendResponse::Text("unexpected".into()))
                })
                .await
        });

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();

        assert!(matches!(first, BackendResponse::Text(ref text) if text == "shared"));
        assert!(matches!(second, BackendResponse::Text(ref text) if text == "shared"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
