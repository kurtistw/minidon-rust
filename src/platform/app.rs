//! Router wiring (adr-001 D1, D3).
//!
//! Layers, outermost first: catch-panic (any panic becomes the Python 500), then the 1 MiB body
//! limit. The fallbacks answer 404 and 405 with Python's bodies. No documentation routes
//! (`/docs`, `/redoc`, `/openapi.json`) exist (OP-06).
//!
//! Wave 4 merges the accounts router through [`build_app_with`]'s `routes` argument.

use std::any::Any;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::response::Response;
use axum::Router;
use sqlx::SqlitePool;
use tower_http::catch_panic::CatchPanicLayer;

use super::clock::Clock;
use super::config::{Settings, MAX_BODY_BYTES};
use super::error::{internal_server_error, method_not_allowed_response, not_found_response};

/// Shared handler state. Nothing in it sits behind a `Mutex`, so a panic cannot poison it.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<Settings>,
}

/// The app with no capability routes yet.
pub fn build_app(state: AppState) -> Router {
    build_app_with(state, Router::new())
}

/// The app with the given capability routes merged in.
pub fn build_app_with(state: AppState, routes: Router<AppState>) -> Router {
    routes
        .fallback(|| async { not_found_response() })
        .method_not_allowed_fallback(|| async { method_not_allowed_response() })
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::custom(panic_response))
        .with_state(state)
}

/// The panic payload is dropped on purpose: it may hold request data.
fn panic_response(_: Box<dyn Any + Send + 'static>) -> Response {
    internal_server_error()
}
