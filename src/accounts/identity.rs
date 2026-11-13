//! The identity API other capabilities call (STATED ownership: CF-004, baseline gate record;
//! the behaviour is DERIVED from BR-011 and BR-012, `accounts/service.py:81-110`).
//!
//! Later stories call these functions and never read the accounts or token tables themselves.
//! Every call reads the database: there is no cache, so a suspension or an expiry is seen on the
//! very next request (threat row 7).

use sqlx::SqlitePool;

use super::model::Account;
use super::{store, token};
use crate::platform::clock::Clock;
use crate::platform::error::AppError;

/// An active account by username, in any case. A suspended account looks as if it does not exist
/// (404 "Account not found"), like Python's `get_by_username`.
pub async fn get_by_username(pool: &SqlitePool, username: &str) -> Result<Account, AppError> {
    match store::find_by_username(pool, username).await? {
        Some(account) if account.suspended_at.is_none() => Ok(account),
        _ => Err(AppError::NotFound("Account not found".into())),
    }
}

/// The variant that also finds suspended accounts: registration, sign-in and Moderation (BR-012).
pub async fn get_by_username_including_suspended(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<Account>, AppError> {
    store::find_by_username(pool, username).await
}

/// Any account by id, suspended or not.
pub async fn get_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Account>, AppError> {
    store::find_by_id(pool, id).await
}

/// The account behind a token, or `None` for signed-out: unknown token, expired token (the
/// instant of expiry already counts as expired), missing account row, or suspended account.
pub async fn authenticate(
    pool: &SqlitePool,
    clock: &dyn Clock,
    token_text: &str,
) -> Result<Option<Account>, AppError> {
    let Some(access) = store::find_token(pool, token_text).await? else {
        return Ok(None);
    };
    if token::is_expired(access.expires_at, clock.now()) {
        return Ok(None);
    }
    Ok(store::find_by_id(pool, access.account_id)
        .await?
        .filter(|account| account.suspended_at.is_none()))
}
