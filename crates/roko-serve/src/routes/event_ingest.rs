//! RuntimeEvent ingest endpoints for out-of-process roko commands.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};
use axum::routing::post;
use roko_core::RuntimeEvent;
use roko_core::foundation::EventConsumer;

use crate::error::ApiError;
use crate::extract::ApiJson;
use crate::state::AppState;

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/events/ingest", post(ingest_event))
        .route("/events/ingest/batch", post(ingest_event_batch))
}

async fn ingest_event(
    State(state): State<Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(event): ApiJson<RuntimeEvent>,
) -> Result<StatusCode, ApiError> {
    ensure_ingest_allowed(&state, Some(remote.ip()), &headers)?;
    consume_runtime_event(&state, &event);
    Ok(StatusCode::ACCEPTED)
}

async fn ingest_event_batch(
    State(state): State<Arc<AppState>>,
    ConnectInfo(remote): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(events): ApiJson<Vec<RuntimeEvent>>,
) -> Result<StatusCode, ApiError> {
    ensure_ingest_allowed(&state, Some(remote.ip()), &headers)?;
    if events.len() > 1_000 {
        return Err(ApiError::bad_request(
            "event ingest batch is limited to 1000 RuntimeEvent objects",
        ));
    }
    for event in events {
        consume_runtime_event(&state, &event);
    }
    Ok(StatusCode::ACCEPTED)
}

fn ensure_ingest_allowed(
    state: &AppState,
    remote_ip: Option<IpAddr>,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let config = state.load_roko_config();
    if config.serve.auth.enabled {
        return Ok(());
    }

    if request_has_ingest_token(headers, config.server.auth_token.as_deref()) {
        return Ok(());
    }

    if super::bind_is_loopback(&config.server.bind) {
        return Ok(());
    }

    if remote_ip.is_some_and(|ip| ip.is_loopback()) {
        return Ok(());
    }

    if let Some(remote_ip) = remote_ip
        && config
            .serve
            .event_ingest_allowlist
            .iter()
            .any(|allowed| ip_matches(allowed, remote_ip))
    {
        return Ok(());
    }

    Err(ApiError::forbidden(
        "event ingest requires a loopback caller, enabled serve auth, bearer token, or serve.event_ingest_allowlist",
    ))
}

fn consume_runtime_event(state: &AppState, event: &RuntimeEvent) {
    state.sse_adapter.consume(event);
    state.runtime_event_logger.consume(event);
}

fn ip_matches(allowed: &str, remote_ip: IpAddr) -> bool {
    let allowed = allowed.trim();
    if allowed.is_empty() {
        return false;
    }
    if let Ok(allowed_ip) = allowed.parse::<IpAddr>() {
        return allowed_ip == remote_ip;
    }
    let Some((base, prefix)) = allowed.split_once('/') else {
        return false;
    };
    let Ok(base_ip) = base.trim().parse::<IpAddr>() else {
        return false;
    };
    let Ok(prefix_bits) = prefix.trim().parse::<u8>() else {
        return false;
    };
    cidr_matches(base_ip, prefix_bits, remote_ip)
}

fn cidr_matches(base_ip: IpAddr, prefix_bits: u8, remote_ip: IpAddr) -> bool {
    match (base_ip, remote_ip) {
        (IpAddr::V4(base), IpAddr::V4(remote)) if prefix_bits <= 32 => {
            let mask = prefix_mask(prefix_bits, 32) as u32;
            (u32::from(base) & mask) == (u32::from(remote) & mask)
        }
        (IpAddr::V6(base), IpAddr::V6(remote)) if prefix_bits <= 128 => {
            let mask = prefix_mask(prefix_bits, 128);
            (u128::from(base) & mask) == (u128::from(remote) & mask)
        }
        _ => false,
    }
}

fn prefix_mask(prefix_bits: u8, total_bits: u8) -> u128 {
    if prefix_bits == 0 {
        0
    } else {
        u128::MAX << (total_bits - prefix_bits)
    }
}

fn request_has_ingest_token(headers: &HeaderMap, configured_token: Option<&str>) -> bool {
    let Some(expected) = std::env::var("ROKO_SERVER_AUTH_TOKEN")
        .ok()
        .filter(|token| !token.trim().is_empty())
        .or_else(|| {
            configured_token
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
    else {
        return false;
    };

    let Some(supplied) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
    else {
        return false;
    };

    supplied == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn ingest_token_accepts_configured_bearer() {
        let expected = std::env::var("ROKO_SERVER_AUTH_TOKEN")
            .ok()
            .filter(|token| !token.trim().is_empty())
            .unwrap_or_else(|| "test-ingest-token".to_owned());
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {expected}")).expect("valid header"),
        );

        assert!(request_has_ingest_token(&headers, Some(&expected)));
    }

    #[test]
    fn ingest_token_rejects_missing_or_wrong_bearer() {
        let mut headers = HeaderMap::new();
        assert!(!request_has_ingest_token(&headers, Some("expected")));

        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer wrong"));
        assert!(!request_has_ingest_token(&headers, Some("expected")));
    }

    #[test]
    fn ingest_allowlist_matches_exact_ip() {
        assert!(ip_matches("127.0.0.1", IpAddr::from([127, 0, 0, 1])));
        assert!(!ip_matches("127.0.0.2", IpAddr::from([127, 0, 0, 1])));
        assert!(!ip_matches("not-an-ip", IpAddr::from([127, 0, 0, 1])));
    }

    #[test]
    fn ingest_allowlist_matches_cidr_ranges() {
        assert!(ip_matches("10.2.0.0/16", IpAddr::from([10, 2, 3, 4])));
        assert!(!ip_matches("10.2.0.0/16", IpAddr::from([10, 3, 3, 4])));
        assert!(ip_matches("::1/128", "::1".parse().expect("valid ip")));
        assert!(!ip_matches("::1/128", "::2".parse().expect("valid ip")));
    }
}
