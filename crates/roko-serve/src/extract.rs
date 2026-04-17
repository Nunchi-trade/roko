//! Shared request extractors and validation helpers for the HTTP API.

use std::borrow::Cow;

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use serde::de::DeserializeOwned;
use validator::{Validate, ValidationError};

use crate::error::ApiError;

/// JSON request-body extractor with shared parse failures.
pub struct ApiJson<T>(pub T);

/// JSON request-body extractor with shared parse failures and request validation.
pub struct ValidJson<T>(pub T);

/// Trait for request payloads that support post-deserialization validation.
pub(crate) trait RequestPayload {
    fn validate_payload(&self) -> Result<(), ApiError>;
}

pub(crate) fn validate_with_validator<T>(value: &T) -> Result<(), ApiError>
where
    T: Validate,
{
    value.validate().map_err(ApiError::validation)
}

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        parse_json_request(req, state).await.map(Self)
    }
}

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + RequestPayload,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let value: T = parse_json_request(req, state).await?;
        value.validate_payload()?;
        Ok(Self(value))
    }
}

async fn parse_json_request<S, T>(req: Request, state: &S) -> Result<T, ApiError>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    let bytes = Bytes::from_request(req, state)
        .await
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    serde_json::from_slice(&bytes).map_err(ApiError::parse)
}

/// Reject blank strings after trimming whitespace.
pub(crate) fn validate_non_blank(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        let mut error = ValidationError::new("non_blank");
        error.message = Some(Cow::Borrowed("must not be blank"));
        return Err(error);
    }

    Ok(())
}

/// Reject vector entries that are blank after trimming whitespace.
pub(crate) fn validate_string_items_non_blank(values: &[String]) -> Result<(), ValidationError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        let mut error = ValidationError::new("non_blank_items");
        error.message = Some(Cow::Borrowed("must not contain blank entries"));
        return Err(error);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_non_blank_rejects_whitespace() {
        assert!(validate_non_blank("   ").is_err());
        assert!(validate_non_blank("demo").is_ok());
    }

    #[test]
    fn validate_string_items_non_blank_rejects_blank_entries() {
        assert!(validate_string_items_non_blank(&["good".into(), " ".into()]).is_err());
        assert!(validate_string_items_non_blank(&["good".into(), "ok".into()]).is_ok());
    }
}
