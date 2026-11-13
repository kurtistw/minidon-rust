//! Wave 4 integration tests for `accounts::extract` (REQ-012, REQ-013, SC-ACC-018..021, review
//! m-04, plan review M-07). The extractors are mounted on test-only routes so the header rules
//! are checked on their own. `deps.py` is the source: the lower-cased header must start with
//! `bearer ` (one space), the rest is stripped with `str.strip()`, an unknown or expired token or
//! a suspended account is anonymous, and `CurrentAccount` turns anonymous into 401 "Sign in
//! required". Starlette decodes header bytes as latin-1.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use axum::routing::get;
use axum::{Json, Router};
use minidon_rust::accounts::extract::{CurrentAccount, OptionalAccount};
use minidon_rust::accounts::model::AccessToken;
use minidon_rust::accounts::{service, store};
use minidon_rust::platform::app::{build_app_with, AppState};
use minidon_rust::platform::clock::{Clock, FixedClock};
use reqwest::header::HeaderValue;
use serde_json::{json, Value};
use std::sync::Arc;
use time::macros::datetime;

struct Env {
    base: String,
    state: AppState,
    clock: Arc<FixedClock>,
}

async fn env() -> Env {
    let settings = support::schema_state().await;
    let clock = Arc::new(FixedClock::new(datetime!(2026-01-01 00:00:00)));
    let state = AppState {
        pool: settings.pool,
        clock: clock.clone(),
        settings: settings.settings,
    };
    let routes =
        Router::new()
            .route(
                "/opt",
                get(|OptionalAccount(a): OptionalAccount| async move {
                    Json(json!({ "who": a.map(|a| a.username) }))
                }),
            )
            .route(
                "/req",
                get(|CurrentAccount(a): CurrentAccount| async move {
                    Json(json!({ "who": a.username }))
                }),
            );
    let app = build_app_with(state.clone(), routes);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Env {
        base: format!("http://{addr}"),
        state,
        clock,
    }
}

impl Env {
    async fn account(&self, name: &str) -> i64 {
        service::register(
            &self.state.pool,
            self.clock.as_ref(),
            name,
            &format!("{name}@x.org"),
            "correct-horse",
        )
        .await
        .unwrap()
        .id
    }

    async fn token(&self, account_id: i64, text: &str, expires_in_days: i64) {
        let now = self.clock.now();
        store::insert_token(
            &self.state.pool,
            &AccessToken {
                token: text.into(),
                account_id,
                created_at: now,
                expires_at: now + time::Duration::days(expires_in_days),
            },
        )
        .await
        .unwrap();
    }

    async fn get(&self, path: &str, header: Option<&[u8]>) -> (u16, Value) {
        let mut request = support::client().get(format!("{}{path}", self.base));
        if let Some(bytes) = header {
            request = request.header("authorization", HeaderValue::from_bytes(bytes).unwrap());
        }
        let response = request.send().await.unwrap();
        (
            response.status().as_u16(),
            response.json().await.unwrap_or(Value::Null),
        )
    }
}

fn who(name: &str) -> Value {
    json!({ "who": name })
}

#[tokio::test]
async fn valid_token_authenticates_in_any_prefix_case() {
    let e = env().await;
    let id = e.account("alice").await;
    e.token(id, "tok-alice", 30).await;
    for header in [
        "Bearer tok-alice",
        "bearer tok-alice",
        "BEARER tok-alice",
        "bEaReR tok-alice",
    ] {
        assert_eq!(
            e.get("/req", Some(header.as_bytes())).await,
            (200, who("alice")),
            "{header}"
        );
        assert_eq!(
            e.get("/opt", Some(header.as_bytes())).await,
            (200, who("alice")),
            "{header}"
        );
    }
}

#[tokio::test]
async fn token_whitespace_is_stripped_as_python_strip_does() {
    let e = env().await;
    let id = e.account("alice").await;
    e.token(id, "tok-alice", 30).await;
    // U+001C to U+001F and U+000B cannot be sent by the HTTP client in use, and are covered by
    // `unit_token`. The latin-1 whitespace bytes can.
    let ok: [&[u8]; 4] = [
        b"Bearer   tok-alice  ",
        b"Bearer tok-alice\xa0", // latin-1 no-break space
        b"Bearer \x85tok-alice", // latin-1 next line
        b"Bearer \xa0\x85tok-alice\x85\xa0",
    ];
    for header in ok {
        assert_eq!(
            e.get("/req", Some(header)).await,
            (200, who("alice")),
            "{header:?}"
        );
    }
}

#[tokio::test]
async fn anything_else_is_anonymous_never_a_400_or_500() {
    let e = env().await;
    let id = e.account("alice").await;
    e.token(id, "tok-alice", 30).await;
    let anonymous: [&[u8]; 11] = [
        b"Bearer\ttok-alice",    // the prefix needs a plain space
        b"Bearer",               // no token part
        b"Bearer ",              // empty token
        b"Basic tok-alice",      // other scheme
        b"tok-alice",            // no scheme
        b"Bearer tok-alice x",   // token with a space inside
        b"Bearer \xe9tok-alice", // a non-ASCII byte in the token
        b"Bearer tok-alicex",    // unknown token
        b"Bearer \xff\xfe",      // bytes that are not text
        b"Bearer  Bearer tok-alice",
        b"",
    ];
    for header in anonymous {
        let (status, body) = e.get("/req", Some(header)).await;
        assert_eq!(status, 401, "{header:?}");
        assert_eq!(body, json!({"error": "Sign in required"}), "{header:?}");
        assert_eq!(
            e.get("/opt", Some(header)).await,
            (200, json!({"who": null})),
            "{header:?}"
        );
    }
    assert_eq!(e.get("/req", None).await.0, 401);
    assert_eq!(e.get("/opt", None).await, (200, json!({"who": null})));
}

#[tokio::test]
async fn the_first_authorization_header_is_the_one_read() {
    let e = env().await;
    let id = e.account("alice").await;
    e.token(id, "tok-alice", 30).await;
    let send = |first: &'static str, second: &'static str| {
        let request = support::client()
            .get(format!("{}/req", e.base))
            .header("authorization", first)
            .header("authorization", second);
        async move { request.send().await.unwrap().status().as_u16() }
    };
    assert_eq!(send("Bearer tok-alice", "Bearer bad").await, 200);
    assert_eq!(send("Bearer bad", "Bearer tok-alice").await, 401);
}

#[tokio::test]
async fn expiry_boundary_is_exact() {
    let e = env().await;
    let id = e.account("ivy").await;
    e.token(id, "tok-ivy", 30).await; // expires T0 + 30 days
    let header = Some(&b"Bearer tok-ivy"[..]);
    e.clock.set(datetime!(2026-01-30 23:59:59));
    assert_eq!(e.get("/req", header).await.0, 200);
    e.clock.set(datetime!(2026-01-31 00:00:00));
    assert_eq!(e.get("/req", header).await.0, 401);
    e.clock.set(datetime!(2026-01-31 00:00:01));
    assert_eq!(e.get("/req", header).await.0, 401);
    assert_eq!(e.get("/opt", header).await, (200, json!({"who": null})));
}

#[tokio::test]
async fn suspension_is_seen_on_the_very_next_request() {
    let e = env().await;
    let id = e.account("jay").await;
    e.token(id, "tok-jay", 30).await;
    let header = Some(&b"Bearer tok-jay"[..]);
    assert_eq!(e.get("/req", header).await, (200, who("jay")));
    sqlx::query("UPDATE accounts SET suspended_at = '2026-01-01 00:00:00.000000' WHERE id = ?")
        .bind(id)
        .execute(&e.state.pool)
        .await
        .unwrap();
    assert_eq!(e.get("/req", header).await.0, 401);
    sqlx::query("UPDATE accounts SET suspended_at = NULL WHERE id = ?")
        .bind(id)
        .execute(&e.state.pool)
        .await
        .unwrap();
    assert_eq!(e.get("/req", header).await, (200, who("jay")));
}

#[tokio::test]
async fn a_token_of_a_deleted_account_row_is_anonymous() {
    let e = env().await;
    let id = e.account("kim").await;
    e.token(id, "tok-kim", 30).await;
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(&e.state.pool)
        .await
        .unwrap();
    assert_eq!(e.get("/req", Some(&b"Bearer tok-kim"[..])).await.0, 401);
}
