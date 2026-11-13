//! Wave 1 smoke test: the app builds and answers an unknown route with Python's 404.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use minidon_rust::platform::app::build_app;

#[tokio::test]
async fn app_builds_and_unknown_route_is_404() {
    let _ = build_app(support::test_state().await);
    let base = support::spawn(axum::Router::new()).await;
    let response = reqwest::get(format!("{base}/nothing-here")).await.unwrap();
    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body, serde_json::json!({ "detail": "Not Found" }));
}
