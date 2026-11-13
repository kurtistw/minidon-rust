//! The five Accounts endpoints (REQ-001..REQ-016, REQ-019). Behaviour is DERIVED from
//! `accounts/routes.py` and `deps.py` and confirmed against the oracle by the parity harness.
//!
//! Python facts reproduced here:
//! - `POST /accounts` answers 201, everything else 200.
//! - `GET /accounts/{username}` has no authentication: the `Authorization` header is not read.
//! - `PATCH /accounts/me` order: JSON syntax (422), then who is signed in (401), then field
//!   types (422), then the rules.
//! - A wrong method on a known path is a 405 whose `Allow` header lists the methods of the first
//!   registered route for that path (`GET` for `/accounts/me`, `/accounts/{username}`, `POST`
//!   for the other two). `HEAD` is not served for `GET` routes.
//! - Lone surrogates in the body are handled as decided in adr-001 section 5 decision 6, see
//!   `platform::scrub` and `service::*_with`.
//!
//! Security-sensitive (routes feed the auth extractor): needs a reviewer other than its
//! author (I-14).

use axum::extract::{FromRequest, FromRequestParts, Path, Request, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};

use super::extract::CurrentAccount;
use super::model::{Account, ProfileIn, RegisterIn, Tainted, TokenIn, TokenOut};
use super::{identity, service};
use crate::platform::app::AppState;
use crate::platform::error::{method_not_allowed_allowing, not_found_response, AppError};
use crate::platform::json::RawBody;

/// Route patterns in registration order, for the trailing-slash redirect check.
pub const PATTERNS: &[&str] = &[
    "/api/v1/accounts",
    "/api/v1/auth/token",
    "/api/v1/accounts/me",
    "/api/v1/accounts/{username}",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/accounts", any(accounts_collection))
        .route("/api/v1/auth/token", any(auth_token))
        .route("/api/v1/accounts/me", any(accounts_me))
        .route("/api/v1/accounts/{username}", any(accounts_show))
}

type Reply = Result<Response, Response>;

fn fail(error: AppError) -> Response {
    error.into_response()
}

async fn profile_json(state: &AppState, account: &Account) -> Reply {
    let out = service::serialize_account(&state.pool, &state.settings.instance_domain, account)
        .await
        .map_err(fail)?;
    Ok(Json(out).into_response())
}

async fn accounts_collection(State(state): State<AppState>, req: Request) -> Response {
    if req.method() != Method::POST {
        return method_not_allowed_allowing("POST");
    }
    register(&state, req).await.unwrap_or_else(|r| r)
}

async fn register(state: &AppState, req: Request) -> Reply {
    let RawBody(body) = RawBody::from_request(req, state).await?;
    let (input, tainted): (RegisterIn, _) = body.into_model().map_err(|r| *r)?;
    let account = service::register_with(
        &state.pool,
        state.clock.as_ref(),
        &input.username,
        &input.email,
        &input.password,
        Tainted::from_fields(&tainted),
    )
    .await
    .map_err(fail)?;
    let mut response = profile_json(state, &account).await?;
    *response.status_mut() = StatusCode::CREATED;
    Ok(response)
}

async fn auth_token(State(state): State<AppState>, req: Request) -> Response {
    if req.method() != Method::POST {
        return method_not_allowed_allowing("POST");
    }
    sign_in(&state, req).await.unwrap_or_else(|r| r)
}

async fn sign_in(state: &AppState, req: Request) -> Reply {
    let RawBody(body) = RawBody::from_request(req, state).await?;
    let (input, tainted): (TokenIn, _) = body.into_model().map_err(|r| *r)?;
    let access = service::sign_in_with(
        &state.pool,
        state.clock.as_ref(),
        state.settings.token_ttl_days,
        &input.username,
        &input.password,
        Tainted::from_fields(&tainted),
    )
    .await
    .map_err(fail)?;
    Ok(Json(TokenOut::new(&access)).into_response())
}

async fn accounts_me(State(state): State<AppState>, req: Request) -> Response {
    let result = match *req.method() {
        Method::GET => show_me(&state, req).await,
        Method::PATCH => update_me(&state, req).await,
        _ => return method_not_allowed_allowing("GET"),
    };
    result.unwrap_or_else(|r| r)
}

async fn show_me(state: &AppState, req: Request) -> Reply {
    let (mut parts, _) = req.into_parts();
    let CurrentAccount(account) = CurrentAccount::from_request_parts(&mut parts, state)
        .await
        .map_err(fail)?;
    profile_json(state, &account).await
}

async fn update_me(state: &AppState, req: Request) -> Reply {
    let (mut parts, body) = req.into_parts();
    // Python reads who is signed in while it resolves dependencies, which happens after it
    // parsed the body and before it checks the field types. Same order here.
    let account = CurrentAccount::from_request_parts(&mut parts, state).await;
    let RawBody(body) = RawBody::from_request(Request::from_parts(parts, body), state).await?;
    let CurrentAccount(account) = account.map_err(fail)?;
    let (input, tainted): (ProfileIn, _) = body.into_model().map_err(|r| *r)?;
    let updated = service::update_profile_with(
        &state.pool,
        &account,
        input.display_name.as_deref(),
        input.bio.as_deref(),
        input.locked,
        Tainted::from_fields(&tainted),
    )
    .await
    .map_err(fail)?;
    profile_json(state, &updated).await
}

async fn accounts_show(
    State(state): State<AppState>,
    username: Result<Path<String>, axum::extract::rejection::PathRejection>,
    req: Request,
) -> Response {
    if req.method() != Method::GET {
        return method_not_allowed_allowing("GET");
    }
    let Ok(Path(username)) = username else {
        return not_found_response();
    };
    // No authentication here: the Authorization header is never read (review m-01).
    let result: Reply = async {
        let account = identity::get_by_username(&state.pool, &username)
            .await
            .map_err(fail)?;
        profile_json(&state, &account).await
    }
    .await;
    result.unwrap_or_else(|r| r)
}
