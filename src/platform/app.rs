//! Router wiring (adr-001 D1, D3).
//!
//! Layers, outermost first: catch-panic (any panic becomes the Python 500), the 1 MiB body
//! limit, then the path decoder. The decoder runs BEFORE routing (the router is mounted as the
//! fallback service of an outer router, so the layer sits in front of it): uvicorn decodes the
//! whole path before Starlette routes on it, see [`super::path`]. Unmatched requests get Python's
//! 404 body, or a 307 when the path matches a known route with a trailing slash added or removed
//! (Starlette's redirect_slashes). No documentation routes (`/docs`, `/redoc`, `/openapi.json`)
//! exist (OP-06).

use std::any::Any;
use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use sqlx::SqlitePool;
use tower_http::catch_panic::CatchPanicLayer;

use super::clock::Clock;
use super::config::{Settings, MAX_BODY_BYTES};
use super::error::{internal_server_error, method_not_allowed_response, not_found_response};
use super::path::{canonical_path, decode_path, location_for, pattern_matches, toggle_slash};
use crate::accounts;

/// Shared handler state. Nothing in it sits behind a `Mutex`, so a panic cannot poison it.
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<Settings>,
}

/// The real app: every capability router merged in (story 1: Accounts).
pub fn build_app(state: AppState) -> Router {
    build_app_with_patterns(
        state,
        accounts::routes::router(),
        accounts::routes::PATTERNS,
    )
}

/// The app with the given routes and no trailing-slash redirects.
pub fn build_app_with(state: AppState, routes: Router<AppState>) -> Router {
    build_app_with_patterns(state, routes, &[])
}

/// The app with the given routes. `patterns` lists their paths (`{name}` for a parameter) so
/// the fallback can tell Starlette's trailing-slash redirect from a plain 404.
pub fn build_app_with_patterns(
    state: AppState,
    routes: Router<AppState>,
    patterns: &'static [&'static str],
) -> Router {
    let inner = routes
        .fallback(move |req: Request| async move { not_found_or_redirect(&req, patterns) })
        .method_not_allowed_fallback(|| async { method_not_allowed_response() })
        .with_state(state);
    Router::new()
        .fallback_service(inner)
        .layer(middleware::from_fn(decode_target))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(CatchPanicLayer::custom(panic_response))
}

/// What the decoder remembers for the fallback: the decoded path and the raw query.
#[derive(Clone)]
struct Target {
    decoded: String,
    query: Option<String>,
}

/// Replace the path with its canonical re-encoding of the decoded text, and remember the
/// decoded path for the redirect.
async fn decode_target(mut req: Request, next: Next) -> Response {
    let decoded = decode_path(req.uri().path());
    let query = req.uri().query().map(str::to_owned);
    let rewritten = match &query {
        Some(query) => format!("{}?{query}", canonical_path(&decoded)),
        None => canonical_path(&decoded),
    };
    let mut parts = req.uri().clone().into_parts();
    if let Ok(path_and_query) = rewritten.parse() {
        parts.path_and_query = Some(path_and_query);
        if let Ok(uri) = Uri::from_parts(parts) {
            *req.uri_mut() = uri;
        }
    }
    req.extensions_mut().insert(Target { decoded, query });
    next.run(req).await
}

fn not_found_or_redirect(req: &Request, patterns: &[&str]) -> Response {
    let Some(target) = req.extensions().get::<Target>() else {
        return not_found_response();
    };
    let candidate = toggle_slash(&target.decoded);
    let known = candidate
        .as_deref()
        .is_some_and(|path| patterns.iter().any(|p| pattern_matches(p, path)));
    let (true, Some(path)) = (known, candidate) else {
        return not_found_response();
    };
    let host = req
        .headers()
        .get(header::HOST)
        .map(|value| accounts::token::latin1_to_string(value.as_bytes()))
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    let location = location_for(&host, &path, target.query.as_deref());
    match HeaderValue::from_str(&location) {
        Ok(value) => (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, value)]).into_response(),
        Err(_) => not_found_response(),
    }
}

/// The panic payload is dropped on purpose: it may hold request data.
fn panic_response(_: Box<dyn Any + Send + 'static>) -> Response {
    internal_server_error()
}
