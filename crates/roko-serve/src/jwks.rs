//! JWKS cache and JWT verification for Privy authentication.
//!
//! Fetches the Privy JWKS endpoint, caches the keys with a 1-hour TTL,
//! and verifies ES256-signed JWTs against the cached keys.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{Algorithm, DecodingKey, TokenData, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::RwLock;

/// The Nunchi Privy application ID. Project-level constant, not a secret.
pub const NUNCHI_PRIVY_APP_ID: &str = "cmhw01vut003tjx0d5lmqc8zs";

/// JWKS endpoint for Privy.
const JWKS_URL: &str = "https://auth.privy.io/.well-known/jwks.json";

/// Cache TTL: keys are refreshed after this duration.
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// Maximum staleness before we warn about using stale keys.
const MAX_STALE: Duration = Duration::from_secs(86400);

/// Decoded claims from a Privy JWT.
#[derive(Debug, Clone, Deserialize)]
pub struct PrivyClaims {
    /// Privy user identifier (e.g. `did:privy:...`).
    pub sub: String,
    /// Token issuer.
    #[serde(default)]
    pub iss: String,
    /// Audience claim.
    #[serde(default)]
    pub aud: Option<serde_json::Value>,
}

/// A single JWK from the JWKS response.
#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

/// The JWKS response envelope.
#[derive(Debug, Clone, Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

struct CacheInner {
    keys: Vec<Jwk>,
    fetched_at: Instant,
}

/// Thread-safe JWKS cache with automatic refresh.
pub struct JwksCache {
    http: reqwest::Client,
    cache: RwLock<Option<CacheInner>>,
}

impl JwksCache {
    /// Create a new cache backed by the given HTTP client.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            cache: RwLock::new(None),
        }
    }

    /// Eagerly fetch JWKS on startup. Logs a warning on failure.
    pub async fn prime(&self) {
        match self.fetch_jwks().await {
            Ok(keys) => {
                tracing::info!(key_count = keys.len(), "JWKS cache primed");
                let mut cache = self.cache.write().await;
                *cache = Some(CacheInner {
                    keys,
                    fetched_at: Instant::now(),
                });
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to prime JWKS cache; JWT auth will retry on first request");
            }
        }
    }

    /// Validate a JWT against the cached JWKS keys.
    ///
    /// Returns `Some(claims)` on success, `None` if verification fails.
    pub async fn validate(&self, token: &str, privy_app_id: &str) -> Option<PrivyClaims> {
        // Parse the JWT header to get the kid.
        let header = decode_header(token).ok()?;
        let kid = header.kid.as_deref()?;

        // First attempt with cached keys.
        if let Some(claims) = self.try_validate_with_cache(token, kid, privy_app_id).await {
            return Some(claims);
        }

        // Key rotation handling: refetch JWKS once and retry.
        if self.refresh_jwks().await.is_ok() {
            if let Some(claims) = self.try_validate_with_cache(token, kid, privy_app_id).await {
                return Some(claims);
            }
        }

        None
    }

    async fn try_validate_with_cache(
        &self,
        token: &str,
        kid: &str,
        privy_app_id: &str,
    ) -> Option<PrivyClaims> {
        self.ensure_fresh().await;

        let cache = self.cache.read().await;
        let inner = cache.as_ref()?;
        let jwk = inner.keys.iter().find(|k| k.kid == kid)?;
        let decoding_key = ec_decoding_key(jwk)?;

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[privy_app_id]);
        validation.set_issuer(&["privy.io"]);

        let token_data: TokenData<PrivyClaims> = decode(token, &decoding_key, &validation).ok()?;
        Some(token_data.claims)
    }

    async fn ensure_fresh(&self) {
        let needs_refresh = {
            let cache = self.cache.read().await;
            match cache.as_ref() {
                None => true,
                Some(inner) => inner.fetched_at.elapsed() > CACHE_TTL,
            }
        };

        if needs_refresh {
            let _ = self.refresh_jwks().await;
        }
    }

    async fn refresh_jwks(&self) -> Result<(), String> {
        match self.fetch_jwks().await {
            Ok(keys) => {
                let mut cache = self.cache.write().await;
                *cache = Some(CacheInner {
                    keys,
                    fetched_at: Instant::now(),
                });
                Ok(())
            }
            Err(err) => {
                // If we have stale keys, keep using them but warn.
                let cache = self.cache.read().await;
                if let Some(inner) = cache.as_ref() {
                    let stale_for = inner.fetched_at.elapsed();
                    if stale_for > MAX_STALE {
                        tracing::warn!(
                            stale_secs = stale_for.as_secs(),
                            error = %err,
                            "JWKS cache is very stale (>24h), verification may fail"
                        );
                    } else {
                        tracing::debug!(
                            stale_secs = stale_for.as_secs(),
                            error = %err,
                            "JWKS refresh failed, using stale cache"
                        );
                    }
                }
                Err(err)
            }
        }
    }

    async fn fetch_jwks(&self) -> Result<Vec<Jwk>, String> {
        let response = self
            .http
            .get(JWKS_URL)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("JWKS fetch failed: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("JWKS endpoint returned {}", response.status()));
        }

        let jwks: JwksResponse = response
            .json()
            .await
            .map_err(|e| format!("JWKS parse failed: {e}"))?;

        Ok(jwks.keys)
    }
}

/// Build a `DecodingKey` from an EC JWK (P-256 / ES256).
fn ec_decoding_key(jwk: &Jwk) -> Option<DecodingKey> {
    if jwk.kty != "EC" {
        return None;
    }
    if jwk.crv.as_deref() != Some("P-256") {
        return None;
    }
    let x = jwk.x.as_deref()?;
    let y = jwk.y.as_deref()?;
    DecodingKey::from_ec_components(x, y).ok()
}

/// Create a new `JwksCache` wrapped in `Arc` for shared ownership.
pub fn new_jwks_cache(http: reqwest::Client) -> Arc<JwksCache> {
    Arc::new(JwksCache::new(http))
}
