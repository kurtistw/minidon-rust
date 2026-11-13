//! Who is signed in (BR-011, DERIVED from `deps.py`; ownership of the shared extractor is
//! STATED in adr-002 2.1 as design, not as a human decision).
//!
//! `OptionalAccount` is anonymous for anything that does not authenticate: no header, another
//! scheme, an unknown or expired token, a missing or suspended account. `CurrentAccount` turns
//! anonymous into 401 "Sign in required". The header bytes are decoded as latin-1, as Starlette
//! does, so Python's `strip()` and this code see the same characters. Every call reads the
//! database (no cache), so a suspension is seen on the very next request.
//!
//! Security-sensitive: needs a reviewer other than its author (I-14).

use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

use super::model::Account;
use super::{identity, token};
use crate::platform::app::AppState;
use crate::platform::error::AppError;

/// The signed-in account, if any.
#[derive(Debug, Clone)]
pub struct OptionalAccount(pub Option<Account>);

/// The signed-in account, or 401.
#[derive(Debug, Clone)]
pub struct CurrentAccount(pub Account);

async fn account_from(parts: &Parts, state: &AppState) -> Result<Option<Account>, AppError> {
    // The first Authorization header is the one Python reads.
    let header = parts
        .headers
        .get(AUTHORIZATION)
        .map(|value| token::latin1_to_string(value.as_bytes()));
    let Some(bearer) = token::parse_bearer(header.as_deref()) else {
        return Ok(None);
    };
    identity::authenticate(&state.pool, state.clock.as_ref(), &bearer).await
}

impl FromRequestParts<AppState> for OptionalAccount {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(account_from(parts, state).await?))
    }
}

impl FromRequestParts<AppState> for CurrentAccount {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        account_from(parts, state)
            .await?
            .map(Self)
            .ok_or_else(|| AppError::Unauthorized("Sign in required".into()))
    }
}
