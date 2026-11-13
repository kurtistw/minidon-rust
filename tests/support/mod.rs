//! Shared test helpers. Each test file includes this with `mod support;`.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use minidon_rust::platform::app::{build_app_with, AppState};
use minidon_rust::platform::clock::{Clock, FixedClock};
use minidon_rust::platform::config::Settings;
use minidon_rust::platform::db::connect;
use time::macros::datetime;

/// State on an in-memory database and a fixed clock at T0 = 2026-01-01 00:00:00 UTC.
pub async fn test_state() -> AppState {
    let settings = Settings {
        database_url: "sqlite://".into(),
        instance_domain: "minidon.local".into(),
        token_ttl_days: 30,
    };
    let pool = connect(&settings).await.unwrap();
    let clock: Arc<dyn Clock> = Arc::new(FixedClock::new(datetime!(2026-01-01 00:00:00)));
    AppState {
        pool,
        clock,
        settings: Arc::new(settings),
    }
}

/// Serve the app on a free local port and return its base URL.
pub async fn spawn(extra: Router<AppState>) -> String {
    let app = build_app_with(test_state().await, extra);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

const PYTHON_SCHEMA: &str = include_str!("../fixtures/python_schema.sql");

/// Apply the Python-made schema fixture. Tests only: production code never runs DDL.
pub async fn apply_python_schema(pool: &sqlx::SqlitePool) {
    for statement in PYTHON_SCHEMA
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sqlx::query(statement).execute(pool).await.unwrap();
    }
}

/// `test_state` with a copy of the Python-made schema applied (accounts, access_tokens,
/// follows, statuses).
pub async fn schema_state() -> AppState {
    let state = test_state().await;
    apply_python_schema(&state.pool).await;
    state
}

/// A full app (the real router) on a free port, with the Python schema, a fixed clock at T0 and
/// handles to the pool and the clock.
pub struct App {
    pub base: String,
    pub pool: sqlx::SqlitePool,
    pub clock: Arc<FixedClock>,
}

pub async fn spawn_app() -> App {
    let settings = Settings {
        database_url: "sqlite://".into(),
        instance_domain: "minidon.local".into(),
        token_ttl_days: 30,
    };
    let pool = connect(&settings).await.unwrap();
    apply_python_schema(&pool).await;
    let clock = Arc::new(FixedClock::new(datetime!(2026-01-01 00:00:00)));
    let state = AppState {
        pool: pool.clone(),
        clock: clock.clone(),
        settings: Arc::new(settings),
    };
    let app = minidon_rust::platform::app::build_app(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    App {
        base: format!("http://{addr}"),
        pool,
        clock,
    }
}

/// A client that never follows redirects and ignores proxy settings.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .unwrap()
}
