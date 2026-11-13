//! JSON request bodies the way FastAPI reads them (DERIVED from the oracle, confirmed by the
//! parity harness), with the lone-surrogate pre-parse in front (adr-001 section 5 decision 6).
//!
//! FastAPI parses the body in the route handler, before it resolves dependencies such as the
//! signed-in account, and validates the fields after them. So a syntax error is a 422 even for a
//! caller without a token, while a wrong field type without a token is a 401. [`RawBody`] does
//! the first step and [`Body::into_model`] the second. Rules checked on Python:
//! - only `application/json` and `application/*+json` bodies are parsed (any charset
//!   parameter, any letter case). Other or missing content types leave the body unparsed, which
//!   fails field validation (422), not parsing.
//! - an empty body is "missing", also a field validation error.
//! - duplicate keys: the last one wins. Extra fields are ignored.
//!
//! Error bodies are fixed texts: serde messages can echo the input value (passwords).

use axum::body::Bytes;
use axum::extract::{FromRequest, Request};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::{payload_too_large_response, validation_response};
use super::scrub::scrub_lone_surrogates;

/// A request body after the parse step.
#[derive(Debug, Clone)]
pub enum Body {
    /// Parsed JSON, with the top level fields that held a lone surrogate (now U+FFFD).
    Json { value: Value, tainted: Vec<String> },
    /// JSON content type with an empty body.
    Missing,
    /// A content type that is not JSON: the body is left alone and fails validation.
    NotJson,
}

fn is_json_content_type(headers: &HeaderMap) -> bool {
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let media = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || (media.starts_with("application/") && media.ends_with("+json"))
}

impl Body {
    /// Field validation: build the model, or answer 422 `{"detail": [...]}`. The tainted field
    /// names go back with the model.
    pub fn into_model<T: DeserializeOwned>(self) -> Result<(T, Vec<String>), Box<Response>> {
        match self {
            Self::Json { value, tainted } => serde_json::from_value::<T>(value)
                .map(|model| (model, tainted))
                .map_err(|_| Box::new(validation_response("value_error", "Invalid request body"))),
            Self::Missing => Err(Box::new(validation_response("missing", "Field required"))),
            Self::NotJson => Err(Box::new(validation_response(
                "model_attributes_type",
                "Input should be a valid dictionary or object to extract fields from",
            ))),
        }
    }
}

/// The parse step. Rejects a body that is not valid JSON (422) or is above the limit (413).
#[derive(Debug, Clone)]
pub struct RawBody(pub Body);

impl<S: Send + Sync> FromRequest<S> for RawBody {
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let json = is_json_content_type(req.headers());
        let bytes = Bytes::from_request(req, state).await.map_err(|rejection| {
            if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
                payload_too_large_response()
            } else {
                validation_response("json_invalid", "JSON decode error")
            }
        })?;
        if !json {
            return Ok(Self(Body::NotJson));
        }
        if bytes.is_empty() {
            return Ok(Self(Body::Missing));
        }
        let invalid = || validation_response("json_invalid", "JSON decode error");
        let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        let scrubbed = scrub_lone_surrogates(text);
        let value = serde_json::from_str::<Value>(&scrubbed.text).map_err(|_| invalid())?;
        Ok(Self(Body::Json {
            value,
            tainted: scrubbed.tainted,
        }))
    }
}

/// Both steps at once, for routes with no dependency in between.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let RawBody(body) = RawBody::from_request(req, state).await?;
        body.into_model::<T>()
            .map(|(model, _)| Self(model))
            .map_err(|response| *response)
    }
}
