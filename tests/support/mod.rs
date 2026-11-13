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
