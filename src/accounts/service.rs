//! Registration, sign-in and profile rules (BR-001 STATED, BR-009 STATED, the rest DERIVED from
//! `accounts/service.py`). Messages are Python's, copied. Check order is Python's too.
//!
//! No dummy hash and no throttle, by parity (threat rows 4 and 5): an unknown user is a 401
//! before any hashing, and wrong passwords never lock an account.

use sqlx::SqlitePool;

use super::model::{AccessToken, Account, AccountOut, Role};
use super::password::{hash_password, verify_password};
use super::store::{self, NewAccount};
use super::token::{expires_at, new_token};
use super::validate::{
    bio_valid, display_name_valid, email_valid, normalise_email, normalise_username,
    password_valid, username_valid, BIO_MESSAGE, DISPLAY_NAME_MESSAGE, EMAIL_MESSAGE,
    PASSWORD_MESSAGE, USERNAME_MESSAGE,
};
use crate::platform::clock::Clock;
use crate::platform::error::AppError;

const BAD_CREDENTIALS: &str = "Invalid username or password";
const SUSPENDED: &str = "This account is suspended";

/// PBKDF2 at 100,000 rounds is CPU work, so it runs off the async threads.
async fn blocking<T, F>(work: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| AppError::Internal)
}

/// Register. Order as Python: validate username, email, password (422), then username taken
/// (409, suspended accounts included), then email taken (409), then count-then-insert with the
/// first account becoming Admin. The same count-then-insert race is kept on purpose (BR-006).
pub async fn register(
    pool: &SqlitePool,
    clock: &dyn Clock,
    username: &str,
    email: &str,
    password: &str,
) -> Result<Account, AppError> {
    let username = normalise_username(username);
    let email = normalise_email(email);
    if !username_valid(&username) {
        return Err(AppError::Invalid(USERNAME_MESSAGE.into()));
    }
    if !email_valid(&email) {
        return Err(AppError::Invalid(EMAIL_MESSAGE.into()));
    }
    if !password_valid(password) {
        return Err(AppError::Invalid(PASSWORD_MESSAGE.into()));
    }
    if store::find_by_username(pool, &username).await?.is_some() {
        return Err(AppError::Conflict("Username is taken".into()));
    }
    if store::find_by_email(pool, &email).await?.is_some() {
        return Err(AppError::Conflict("Email is already registered".into()));
    }
    // The first account on a fresh instance is the one running it.
    let role = if store::count_accounts(pool).await? == 0 {
        Role::Admin
    } else {
        Role::User
    };
    let secret = password.to_owned();
    let password_hash = blocking(move || hash_password(&secret)).await?;
    store::insert_account(
        pool,
        &NewAccount {
            username: &username,
            email: &email,
            password_hash: &password_hash,
            role,
            created_at: clock.now(),
        },
    )
    .await
}

/// Sign in and issue a token. Unknown user: 401 before hashing. Wrong password: the same 401.
/// A correct password on a suspended account: 403, checked after the password (BR-009). The
/// lookup includes suspended accounts and is case-insensitive.
pub async fn sign_in(
    pool: &SqlitePool,
    clock: &dyn Clock,
    token_ttl_days: i64,
    username: &str,
    password: &str,
) -> Result<AccessToken, AppError> {
    let Some(account) = store::find_by_username(pool, username).await? else {
        return Err(AppError::Unauthorized(BAD_CREDENTIALS.into()));
    };
    let secret = password.to_owned();
    let stored = account.password_hash.clone();
    // A malformed stored hash is an internal error (Python's 500), never a panic.
    if !blocking(move || verify_password(&secret, &stored)).await?? {
        return Err(AppError::Unauthorized(BAD_CREDENTIALS.into()));
    }
    if account.suspended_at.is_some() {
        return Err(AppError::Forbidden(SUSPENDED.into()));
    }
    let now = clock.now();
    let access = AccessToken {
        token: new_token(),
        account_id: account.id,
        created_at: now,
        expires_at: expires_at(now, token_ttl_days),
    };
    store::insert_token(pool, &access).await?;
    Ok(access)
}

/// Update the given fields (BR-013, BR-014). Every given field is validated before anything is
/// written, display name first, so an invalid bio next to a valid display name changes nothing
/// (review m-12). `None` leaves a field alone.
pub async fn update_profile(
    pool: &SqlitePool,
    account: &Account,
    display_name: Option<&str>,
    bio: Option<&str>,
    locked: Option<bool>,
) -> Result<Account, AppError> {
    if display_name.is_some_and(|value| !display_name_valid(value)) {
        return Err(AppError::Invalid(DISPLAY_NAME_MESSAGE.into()));
    }
    if bio.is_some_and(|value| !bio_valid(value)) {
        return Err(AppError::Invalid(BIO_MESSAGE.into()));
    }
    let mut updated = account.clone();
    if let Some(value) = display_name {
        value.clone_into(&mut updated.display_name);
    }
    if let Some(value) = bio {
        value.clone_into(&mut updated.bio);
    }
    if let Some(value) = locked {
        updated.locked = value;
    }
    if display_name.is_some() || bio.is_some() || locked.is_some() {
        store::update_profile(pool, account.id, display_name, bio, locked).await?;
    }
    Ok(updated)
}

/// The public profile with its counts read from the database (review M-02).
pub async fn serialize_account(
    pool: &SqlitePool,
    instance_domain: &str,
    account: &Account,
) -> Result<AccountOut, AppError> {
    let counts = store::profile_counts(pool, account.id).await?;
    Ok(AccountOut::new(account, instance_domain, counts))
}
