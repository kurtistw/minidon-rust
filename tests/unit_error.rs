//! Wave 1 unit tests for `platform::error` (REQ-019, threat rows 9 and 12).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::to_bytes;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use minidon_rust::platform::error::AppError;

async fn read(response: Response) -> (StatusCode, String, Vec<u8>) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, content_type, body.to_vec())
}

#[tokio::test]
async fn domain_errors_map_to_status_and_error_body() {
    let cases = [
        (
            AppError::Invalid("bad".into()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            AppError::Unauthorized("who".into()),
            StatusCode::UNAUTHORIZED,
        ),
        (AppError::Forbidden("no".into()), StatusCode::FORBIDDEN),
        (AppError::NotFound("gone".into()), StatusCode::NOT_FOUND),
        (AppError::Conflict("dup".into()), StatusCode::CONFLICT),
    ];
    for (error, expected) in cases {
        let message = error.to_string();
        let (status, content_type, body) = read(error.into_response()).await;
        assert_eq!(status, expected);
        assert_eq!(content_type, "application/json");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json, serde_json::json!({ "error": message }));
    }
}

#[tokio::test]
async fn message_is_passed_through_exactly() {
    let (_, _, body) = read(AppError::Conflict("Username is taken".into()).into_response()).await;
    assert_eq!(body, br#"{"error":"Username is taken"}"#);
}

#[tokio::test]
async fn internal_error_is_the_fixed_python_500_body() {
    let (status, content_type, body) = read(AppError::Internal.into_response()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(content_type, "text/plain; charset=utf-8");
    assert_eq!(body, b"Internal Server Error");
}

#[tokio::test]
async fn database_errors_become_internal_without_leaking_detail() {
    let error: AppError =
        sqlx::Error::Protocol("SELECT secret FROM accounts password=hunter2".into()).into();
    assert!(matches!(error, AppError::Internal));
    assert!(!error.to_string().contains("hunter2"));
    let (status, _, body) = read(error.into_response()).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, b"Internal Server Error");
}
