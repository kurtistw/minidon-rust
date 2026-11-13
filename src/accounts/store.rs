//! The only code that touches the `accounts` and `access_tokens` tables, plus the read-only
//! profile counts (DERIVED, adr-002 2.1).
//!
//! Bound parameters only, never SQL built from input text (threat row 14). Nothing is cached:
//! every request reads the database (S-05). Texts and enum names follow `platform::clock` and
//! `model::Role`. No DDL.
//!
//! Python sets `display_name`, `bio`, `locked`, `role`, `suspended_at` and `created_at` in code
//! (`accounts/models.py`), so the Python-made schema has no column defaults. [`insert_account`]
//! writes every column itself (review m-03).

use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use super::model::{AccessToken, Account, ProfileCounts, Role};
use super::validate::normalise_username;
use crate::platform::clock::{format_stored, parse_stored, NaiveUtc};
use crate::platform::error::AppError;

/// A literal SELECT of every account column (sqlx only takes literal SQL, so no `format!`).
macro_rules! select_account {
    ($tail:literal) => {
        concat!(
            "SELECT id, username, email, password_hash, display_name, bio, locked, role, ",
            "suspended_at, created_at FROM accounts ",
            $tail
        )
    };
}

/// What registration supplies. Everything else gets Python's defaults.
#[derive(Debug, Clone)]
pub struct NewAccount<'a> {
    pub username: &'a str,
    pub email: &'a str,
    pub password_hash: &'a str,
    pub role: Role,
    pub created_at: NaiveUtc,
}

fn read_time(row: &SqliteRow, column: &str) -> Result<NaiveUtc, AppError> {
    let text: String = row.try_get(column)?;
    parse_stored(&text).map_err(|_| AppError::Internal)
}

fn read_account(row: &SqliteRow) -> Result<Account, AppError> {
    let role: String = row.try_get("role")?;
    let suspended_at: Option<String> = row.try_get("suspended_at")?;
    Ok(Account {
        id: row.try_get("id")?,
        username: row.try_get("username")?,
        email: row.try_get("email")?,
        password_hash: row.try_get("password_hash")?,
        display_name: row.try_get("display_name")?,
        bio: row.try_get("bio")?,
        locked: row.try_get("locked")?,
        role: Role::from_stored(&role).ok_or(AppError::Internal)?,
        suspended_at: suspended_at
            .map(|text| parse_stored(&text).map_err(|_| AppError::Internal))
            .transpose()?,
        created_at: read_time(row, "created_at")?,
    })
}

/// Number of accounts, suspended ones included (BR-006 decides the first admin by this).
pub async fn count_accounts(pool: &SqlitePool) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(pool)
        .await?)
}

/// Find by username. The input is lower-cased first, exactly as Python's `find_by_username`
/// does (`service.py:99`, review M-01), then compared to the stored text as is. Suspended
/// accounts are returned: the caller decides.
pub async fn find_by_username(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<Account>, AppError> {
    let row = sqlx::query(select_account!("WHERE username = ?"))
    .bind(normalise_username(username))
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(read_account).transpose()
}

/// Find by email, exact match. The service normalises the email first.
pub async fn find_by_email(pool: &SqlitePool, email: &str) -> Result<Option<Account>, AppError> {
    let row = sqlx::query(select_account!("WHERE email = ?"))
    .bind(email)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(read_account).transpose()
}

pub async fn find_by_id(pool: &SqlitePool, id: i64) -> Result<Option<Account>, AppError> {
    let row = sqlx::query(select_account!("WHERE id = ?"))
    .bind(id)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(read_account).transpose()
}

/// Insert one account with Python's column values: empty display name and bio, not locked,
/// not suspended. A unique violation (the Python count-then-insert race, or a look-alike) is an
/// internal error, as in Python.
pub async fn insert_account(
    pool: &SqlitePool,
    new: &NewAccount<'_>,
) -> Result<Account, AppError> {
    let result = sqlx::query(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, \
         suspended_at, created_at) VALUES (?, ?, ?, '', '', 0, ?, NULL, ?)",
    )
    .bind(new.username)
    .bind(new.email)
    .bind(new.password_hash)
    .bind(new.role.stored_name())
    .bind(format_stored(new.created_at))
    .execute(pool)
    .await?;
    Ok(Account {
        id: result.last_insert_rowid(),
        username: new.username.to_owned(),
        email: new.email.to_owned(),
        password_hash: new.password_hash.to_owned(),
        display_name: String::new(),
        bio: String::new(),
        locked: false,
        role: new.role,
        suspended_at: None,
        created_at: new.created_at,
    })
}

/// Update only the given fields. `None` leaves a column alone, `Some("")` sets it empty.
pub async fn update_profile(
    pool: &SqlitePool,
    id: i64,
    display_name: Option<&str>,
    bio: Option<&str>,
    locked: Option<bool>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE accounts SET display_name = COALESCE(?, display_name), bio = COALESCE(?, bio), \
         locked = COALESCE(?, locked) WHERE id = ?",
    )
    .bind(display_name)
    .bind(bio)
    .bind(locked)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn insert_token(pool: &SqlitePool, token: &AccessToken) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO access_tokens (token, account_id, created_at, expires_at) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&token.token)
    .bind(token.account_id)
    .bind(format_stored(token.created_at))
    .bind(format_stored(token.expires_at))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn find_token(pool: &SqlitePool, token: &str) -> Result<Option<AccessToken>, AppError> {
    let row = sqlx::query(
        "SELECT token, account_id, created_at, expires_at FROM access_tokens WHERE token = ?",
    )
    .bind(token)
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        Ok(AccessToken {
            token: row.try_get("token")?,
            account_id: row.try_get("account_id")?,
            created_at: read_time(&row, "created_at")?,
            expires_at: read_time(&row, "expires_at")?,
        })
    })
    .transpose()
}

/// The three profile counts, read from the `follows` and `statuses` tables as Python does
/// (`social/queries.py:66-85`, `accounts/schemas.py:30-37`). Read only: Accounts owns neither
/// table. SEAM: story 2 (Following) and story 3 (Posting) take these queries over through their
/// own modules, and this function then calls them.
pub async fn profile_counts(pool: &SqlitePool, account_id: i64) -> Result<ProfileCounts, AppError> {
    let followers = sqlx::query_scalar(
        "SELECT COUNT(*) FROM follows WHERE followee_id = ? AND accepted",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    let following = sqlx::query_scalar(
        "SELECT COUNT(*) FROM follows WHERE follower_id = ? AND accepted",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    let statuses = sqlx::query_scalar(
        "SELECT COUNT(*) FROM statuses WHERE account_id = ? AND deleted_at IS NULL \
         AND reblog_of_id IS NULL",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await?;
    Ok(ProfileCounts {
        followers,
        following,
        statuses,
    })
}
