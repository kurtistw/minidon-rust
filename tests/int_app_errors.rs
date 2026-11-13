//! Wave 1 integration tests for `platform::app` (REQ-019, SC-ACC-028, threat rows 12 and 13,
//! review m-06, m-07).
//!
//! The platform has no JSON route yet (accounts arrive in wave 4), so these tests mount their
//! own test-only routes next to the real fallbacks and layers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use axum::routing::{get, post};
use axum::{Json, Router};
use minidon_rust::platform::app::AppState;
use minidon_rust::platform::config::MAX_BODY_BYTES;
use minidon_rust::platform::error::ApiJson;
use reqwest::{Client, Response};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Echo {
    name: String,
    #[allow(dead_code)]
    age: u32,
}

async fn echo(ApiJson(body): ApiJson<Echo>) -> Json<Value> {
    Json(json!({ "name": body.name }))
}

#[allow(clippy::panic)]
async fn boom() -> &'static str {
    panic!("secret internal detail hunter2");
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/echo", post(echo))
        .route("/boom", get(boom))
        .route("/only-get", get(|| async { "ok" }))
}

async fn json_body(response: Response) -> Value {
    response.json().await.unwrap()
}

fn detail_is_a_list(body: &Value) -> bool {
    body.get("detail").is_some_and(Value::is_array)
        && body.as_object().is_some_and(|o| o.len() == 1)
}

#[tokio::test]
async fn unknown_route_is_python_404() {
    let base = support::spawn(routes()).await;
    let response = reqwest::get(format!("{base}/no/such/route")).await.unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(json_body(response).await, json!({ "detail": "Not Found" }));
}

#[tokio::test]
async fn wrong_method_is_python_405() {
    let base = support::spawn(routes()).await;
    let response = Client::new()
        .post(format!("{base}/only-get"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 405);
    assert!(response.headers().contains_key("allow"));
    assert_eq!(
        json_body(response).await,
        json!({ "detail": "Method Not Allowed" })
    );
}

#[tokio::test]
async fn documentation_routes_are_not_served() {
    let base = support::spawn(routes()).await;
    for path in ["/docs", "/redoc", "/openapi.json"] {
        let response = reqwest::get(format!("{base}{path}")).await.unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
}

#[tokio::test]
async fn valid_json_body_reaches_the_handler() {
    let base = support::spawn(routes()).await;
    let response = Client::new()
        .post(format!("{base}/echo"))
        .json(&json!({ "name": "alice", "age": 3 }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(json_body(response).await, json!({ "name": "alice" }));
}

#[tokio::test]
async fn malformed_json_is_422_with_a_detail_list() {
    let base = support::spawn(routes()).await;
    let response = Client::new()
        .post(format!("{base}/echo"))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert!(detail_is_a_list(&json_body(response).await));
}

#[tokio::test]
async fn missing_field_and_wrong_type_are_422_without_echoing_input() {
    let base = support::spawn(routes()).await;
    let client = Client::new();
    for body in [
        json!({ "name": "alice" }),
        json!({ "name": 5, "age": 3 }),
        json!({ "name": "hunter2", "age": "hunter2" }),
        json!([1, 2]),
        Value::Null,
    ] {
        let response = client
            .post(format!("{base}/echo"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 422, "{body}");
        let text = response.text().await.unwrap();
        assert!(!text.contains("hunter2"), "input echoed: {text}");
        let parsed: Value = serde_json::from_str(&text).unwrap();
        assert!(detail_is_a_list(&parsed), "{text}");
    }
}

#[tokio::test]
async fn missing_or_wrong_content_type_is_422() {
    let base = support::spawn(routes()).await;
    let client = Client::new();
    let missing = client
        .post(format!("{base}/echo"))
        .body(r#"{"name":"a","age":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 422);
    assert!(detail_is_a_list(&json_body(missing).await));

    let wrong = client
        .post(format!("{base}/echo"))
        .header("content-type", "text/plain")
        .body(r#"{"name":"a","age":1}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 422);
    assert!(detail_is_a_list(&json_body(wrong).await));
}

#[tokio::test]
async fn panic_gives_the_fixed_500_and_the_server_keeps_serving() {
    let base = support::spawn(routes()).await;
    let response = reqwest::get(format!("{base}/boom")).await.unwrap();
    assert_eq!(response.status(), 500);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(response.text().await.unwrap(), "Internal Server Error");

    let after = reqwest::get(format!("{base}/only-get")).await.unwrap();
    assert_eq!(after.status(), 200);
    assert_eq!(after.text().await.unwrap(), "ok");
}

fn padded_body(total_len: usize) -> String {
    let prefix = r#"{"age":1,"name":""#;
    let suffix = r#""}"#;
    let fill = total_len - prefix.len() - suffix.len();
    format!("{prefix}{}{suffix}", "a".repeat(fill))
}

#[tokio::test]
async fn body_of_exactly_one_mebibyte_is_accepted() {
    let base = support::spawn(routes()).await;
    let body = padded_body(MAX_BODY_BYTES);
    assert_eq!(body.len(), MAX_BODY_BYTES);
    let response = Client::new()
        .post(format!("{base}/echo"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn body_above_one_mebibyte_is_413_and_the_server_survives() {
    let base = support::spawn(routes()).await;
    let response = Client::new()
        .post(format!("{base}/echo"))
        .header("content-type", "application/json")
        .body(padded_body(MAX_BODY_BYTES + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    assert_eq!(
        json_body(response).await,
        json!({ "detail": "Request Entity Too Large" })
    );
    assert_eq!(
        reqwest::get(format!("{base}/only-get"))
            .await
            .unwrap()
            .status(),
        200
    );
}
