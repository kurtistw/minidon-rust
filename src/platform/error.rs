//! One error type for every rejection path (REQ-019, adr-001 D3).
//!
//! Domain errors answer `{"error": msg}` with the status Python gives them. Framework
//! validation answers 422 `{"detail": [...]}`. Anything unexpected answers the fixed 500 body.
//! Nothing here logs or echoes request bodies, `Authorization` headers, tokens or passwords.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::json;

/// Mirrors the domain errors of `minidon/errors.py` that story 1 uses, plus Internal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// 422
    Invalid(String),
    /// 401
    Unauthorized(String),
    /// 403
    Forbidden(String),
    /// 404
    NotFound(String),
    /// 409
    Conflict(String),
    /// 500, fixed body, no detail.
    Internal,
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(msg)
            | Self::Unauthorized(msg)
            | Self::Forbidden(msg)
            | Self::NotFound(msg)
            | Self::Conflict(msg) => f.write_str(msg),
            Self::Internal => f.write_str("Internal Server Error"),
        }
    }
}

impl std::error::Error for AppError {}

impl From<sqlx::Error> for AppError {
    /// The database error is dropped on purpose: it may carry SQL or values.
    fn from(_: sqlx::Error) -> Self {
        Self::Internal
    }
}

/// Python's 500: `text/plain` with a fixed body.
pub fn internal_server_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "Internal Server Error",
    )
        .into_response()
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal => return internal_server_error(),
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

/// FastAPI style 422 body. Only the status and the top-level shape are contract (adr-001
/// section 5 decision 3). The messages are fixed texts: serde messages can echo the input value.
pub fn validation_response(kind: &str, message: &str) -> Response {
    let body = json!({ "detail": [{ "type": kind, "loc": ["body"], "msg": message }] });
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}

/// 404 as Python: `{"detail": "Not Found"}`.
pub fn not_found_response() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "detail": "Not Found" })),
    )
        .into_response()
}

/// 405 as Python: `{"detail": "Method Not Allowed"}`.
pub fn method_not_allowed_response() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        Json(json!({ "detail": "Method Not Allowed" })),
    )
        .into_response()
}

/// 413 above the body limit. Python has no limit, so this is a recorded deviation (decision 2).
/// The body uses the framework `detail` style. DERIVED choice, see the build log.
pub fn payload_too_large_response() -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        Json(json!({ "detail": "Request Entity Too Large" })),
    )
        .into_response()
}

/// JSON body extractor whose rejections follow the Python contract: 422 `{"detail": [...]}`
/// for a bad body, 413 above the limit.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(map_json_rejection(&rejection)),
        }
    }
}

fn map_json_rejection(rejection: &JsonRejection) -> Response {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return payload_too_large_response();
    }
    match rejection {
        JsonRejection::JsonSyntaxError(_) => {
            validation_response("json_invalid", "JSON decode error")
        }
        JsonRejection::MissingJsonContentType(_) => validation_response(
            "model_attributes_type",
            "Input should be a valid dictionary or object",
        ),
        _ => validation_response("value_error", "Invalid request body"),
    }
}
