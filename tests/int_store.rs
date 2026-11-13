//! Wave 3 integration tests for `accounts::store` on a copy of the Python-made schema
//! (IT-ACC-004..006, IT-ACC-011, IT-ACC-014, review M-01, M-02, m-03). In-memory SQLite, the
//! schema from `tests/fixtures/python_schema.sql`. Rows are seeded the way Python writes them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use minidon_rust::accounts::model::{AccessToken, ProfileCounts, Role};
use minidon_rust::accounts::store::{self, NewAccount};
use minidon_rust::platform::error::AppError;
use sqlx::{Row, SqlitePool};
use time::macros::datetime;

const T0: &str = "2026-01-01 00:00:00.000000";

async fn pool() -> SqlitePool {
    support::schema_state().await.pool
}

async fn seed(pool: &SqlitePool, username: &str, role: &str, suspended: bool) -> i64 {
    let suspended_at = suspended.then_some(T0);
    sqlx::query(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, \
         suspended_at, created_at) VALUES (?, ?, 'h', '', '', 0, ?, ?, ?)",
    )
    .bind(username)
    .bind(format!("{}@x.org", username.replace(['\n', '\''], "_")))
    .bind(role)
    .bind(suspended_at)
    .bind(T0)
    .execute(pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

fn new_account<'a>(username: &'a str, email: &'a str, role: Role) -> NewAccount<'a> {
    NewAccount {
        username,
        email,
        password_hash: "salt$digest",
        role,
        created_at: datetime!(2026-01-01 00:00:00),
    }
}

#[tokio::test]
async fn count_accounts_counts_every_row_including_suspended() {
    let pool = pool().await;
    assert_eq!(store::count_accounts(&pool).await.unwrap(), 0);
    seed(&pool, "a", "USER", true).await;
    seed(&pool, "b", "USER", true).await;
    assert_eq!(store::count_accounts(&pool).await.unwrap(), 2);
}

#[tokio::test]
async fn insert_supplies_every_column_default_python_sets_in_code() {
    // m-03: the Python-made schema has NOT NULL columns with no database defaults.
    let pool = pool().await;
    let account = store::insert_account(&pool, &new_account("alice", "a@x.org", Role::Admin))
        .await
        .unwrap();
    assert_eq!(account.username, "alice");
    assert_eq!(account.display_name, "");
    assert_eq!(account.bio, "");
    assert!(!account.locked);
    assert_eq!(account.role, Role::Admin);
    assert_eq!(account.suspended_at, None);
    assert_eq!(account.created_at, datetime!(2026-01-01 00:00:00));
    assert!(account.id > 0);

    let row = sqlx::query("SELECT * FROM accounts WHERE id = ?")
        .bind(account.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("display_name"), "");
    assert_eq!(row.get::<String, _>("bio"), "");
    assert_eq!(row.get::<i64, _>("locked"), 0);
    assert_eq!(row.get::<String, _>("role"), "ADMIN");
    assert_eq!(row.get::<Option<String>, _>("suspended_at"), None);
    assert_eq!(row.get::<String, _>("created_at"), T0);
    assert_eq!(row.get::<String, _>("password_hash"), "salt$digest");
}

#[tokio::test]
async fn insert_stores_the_role_name_upper_case() {
    let pool = pool().await;
    store::insert_account(&pool, &new_account("u", "u@x.org", Role::User))
        .await
        .unwrap();
    let role: String = sqlx::query_scalar("SELECT role FROM accounts WHERE username = 'u'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(role, "USER");
}

#[tokio::test]
async fn duplicate_username_or_email_is_an_internal_error_like_the_python_race() {
    let pool = pool().await;
    store::insert_account(&pool, &new_account("dup", "d@x.org", Role::User))
        .await
        .unwrap();
    let same_name = store::insert_account(&pool, &new_account("dup", "other@x.org", Role::User)).await;
    assert_eq!(same_name.unwrap_err(), AppError::Internal);
    let same_mail = store::insert_account(&pool, &new_account("other", "d@x.org", Role::User)).await;
    assert_eq!(same_mail.unwrap_err(), AppError::Internal);
}

#[tokio::test]
async fn find_by_username_lower_cases_its_input() {
    // M-01: Python compares `Account.username == username.lower()`.
    let pool = pool().await;
    seed(&pool, "alice", "USER", false).await;
    seed(&pool, "kelvin", "USER", false).await;
    for name in ["alice", "ALICE", "Alice"] {
        let found = store::find_by_username(&pool, name).await.unwrap();
        assert_eq!(found.unwrap().username, "alice", "{name}");
    }
    let found = store::find_by_username(&pool, "\u{212A}ELVIN").await.unwrap();
    assert_eq!(found.unwrap().username, "kelvin", "Kelvin sign folds to k");
}

#[tokio::test]
async fn find_by_username_compares_the_stored_form_exactly_after_lower_casing() {
    let pool = pool().await;
    seed(&pool, "alice", "USER", false).await;
    seed(&pool, "alice\n", "USER", false).await;
    let plain = store::find_by_username(&pool, "alice").await.unwrap().unwrap();
    let newline = store::find_by_username(&pool, "ALICE\n").await.unwrap().unwrap();
    assert_eq!(plain.username, "alice");
    assert_eq!(newline.username, "alice\n");
    assert_ne!(plain.id, newline.id);
    assert!(store::find_by_username(&pool, "alice\n\n").await.unwrap().is_none());
    assert!(store::find_by_username(&pool, " alice").await.unwrap().is_none());
}

#[tokio::test]
async fn find_by_username_never_reads_a_stored_upper_case_name() {
    // Python lower-cases the input only. A row stored as `Mixed` is not reachable.
    let pool = pool().await;
    seed(&pool, "Mixed", "USER", false).await;
    assert!(store::find_by_username(&pool, "mixed").await.unwrap().is_none());
    assert!(store::find_by_username(&pool, "Mixed").await.unwrap().is_none());
}

#[tokio::test]
async fn quotes_percent_and_sql_text_in_usernames_stay_inert() {
    let pool = pool().await;
    seed(&pool, "alice", "USER", false).await;
    for name in ["' OR '1'='1", "%", "al%", "_lice", "a'; DROP TABLE accounts; --", "alice' --"] {
        assert!(store::find_by_username(&pool, name).await.unwrap().is_none(), "{name}");
    }
    assert_eq!(store::count_accounts(&pool).await.unwrap(), 1);
    // The odd name round-trips through insert and find.
    let odd = "it's%";
    store::insert_account(&pool, &new_account(odd, "odd@x.org", Role::User))
        .await
        .unwrap();
    assert_eq!(
        store::find_by_username(&pool, odd).await.unwrap().unwrap().username,
        odd
    );
}

#[tokio::test]
async fn find_by_username_returns_suspended_rows_the_caller_filters() {
    let pool = pool().await;
    seed(&pool, "lou", "USER", true).await;
    let found = store::find_by_username(&pool, "lou").await.unwrap().unwrap();
    assert_eq!(found.suspended_at, Some(datetime!(2026-01-01 00:00:00)));
}

#[tokio::test]
async fn find_by_email_is_an_exact_match() {
    let pool = pool().await;
    seed(&pool, "erin", "USER", false).await;
    assert!(store::find_by_email(&pool, "erin@x.org").await.unwrap().is_some());
    // The service normalises first. The store does not.
    assert!(store::find_by_email(&pool, "ERIN@x.org").await.unwrap().is_none());
    assert!(store::find_by_email(&pool, "nobody@x.org").await.unwrap().is_none());
}

#[tokio::test]
async fn find_by_id_finds_suspended_and_missing() {
    let pool = pool().await;
    let id = seed(&pool, "kim", "MODERATOR", true).await;
    let account = store::find_by_id(&pool, id).await.unwrap().unwrap();
    assert_eq!(account.role, Role::Moderator);
    assert!(store::find_by_id(&pool, id + 100).await.unwrap().is_none());
}

#[tokio::test]
async fn all_three_stored_roles_read_back() {
    let pool = pool().await;
    for (name, role) in [("u", "USER"), ("m", "MODERATOR"), ("a", "ADMIN")] {
        seed(&pool, name, role, false).await;
    }
    let roles: Vec<Role> = {
        let mut out = Vec::new();
        for name in ["u", "m", "a"] {
            out.push(store::find_by_username(&pool, name).await.unwrap().unwrap().role);
        }
        out
    };
    assert_eq!(roles, [Role::User, Role::Moderator, Role::Admin]);
}

#[tokio::test]
async fn an_unreadable_role_or_date_is_an_internal_error_not_a_panic() {
    let pool = pool().await;
    seed(&pool, "badrole", "OWNER", false).await;
    let result = store::find_by_username(&pool, "badrole").await;
    assert_eq!(result.unwrap_err(), AppError::Internal);

    seed(&pool, "baddate", "USER", false).await;
    sqlx::query("UPDATE accounts SET created_at = 'yesterday' WHERE username = 'baddate'")
        .execute(&pool)
        .await
        .unwrap();
    let result = store::find_by_username(&pool, "baddate").await;
    assert_eq!(result.unwrap_err(), AppError::Internal);
}

#[tokio::test]
async fn update_profile_changes_only_the_given_fields() {
    let pool = pool().await;
    let id = seed(&pool, "mia", "USER", false).await;
    store::update_profile(&pool, id, Some("Mia"), Some("bio"), Some(false))
        .await
        .unwrap();
    store::update_profile(&pool, id, None, Some("new bio"), None).await.unwrap();
    let account = store::find_by_id(&pool, id).await.unwrap().unwrap();
    assert_eq!(account.display_name, "Mia");
    assert_eq!(account.bio, "new bio");
    assert!(!account.locked);

    store::update_profile(&pool, id, None, None, Some(true)).await.unwrap();
    let account = store::find_by_id(&pool, id).await.unwrap().unwrap();
    assert!(account.locked);
    assert_eq!(account.bio, "new bio");

    // An empty string is a value, not "leave alone".
    store::update_profile(&pool, id, Some(""), None, None).await.unwrap();
    let account = store::find_by_id(&pool, id).await.unwrap().unwrap();
    assert_eq!(account.display_name, "");
    let locked: i64 = sqlx::query_scalar("SELECT locked FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(locked, 1, "stored as an integer like Python");
}

#[tokio::test]
async fn update_profile_touches_no_other_account() {
    let pool = pool().await;
    let a = seed(&pool, "a", "USER", false).await;
    let b = seed(&pool, "b", "USER", false).await;
    store::update_profile(&pool, a, Some("A"), Some("x"), Some(true)).await.unwrap();
    let other = store::find_by_id(&pool, b).await.unwrap().unwrap();
    assert_eq!(other.display_name, "");
    assert!(!other.locked);
}

fn token_row(token: &str, account_id: i64) -> AccessToken {
    AccessToken {
        token: token.into(),
        account_id,
        created_at: datetime!(2026-01-01 00:00:00),
        expires_at: datetime!(2026-01-31 00:00:00),
    }
}

#[tokio::test]
async fn tokens_are_stored_in_clear_text_and_found_by_exact_text() {
    let pool = pool().await;
    let id = seed(&pool, "ivy", "USER", false).await;
    store::insert_token(&pool, &token_row("tok-AbC_123", id)).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT token FROM access_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, "tok-AbC_123");
    let found = store::find_token(&pool, "tok-AbC_123").await.unwrap().unwrap();
    assert_eq!(found, token_row("tok-AbC_123", id));
    assert!(store::find_token(&pool, "tok-abc_123").await.unwrap().is_none());
    assert!(store::find_token(&pool, "").await.unwrap().is_none());
    assert!(store::find_token(&pool, "' OR 1=1 --").await.unwrap().is_none());
    let row = sqlx::query("SELECT created_at, expires_at FROM access_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("created_at"), T0);
    assert_eq!(row.get::<String, _>("expires_at"), "2026-01-31 00:00:00.000000");
}

async fn follow(pool: &SqlitePool, follower: i64, followee: i64, accepted: bool) {
    sqlx::query(
        "INSERT INTO follows (follower_id, followee_id, accepted, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(follower)
    .bind(followee)
    .bind(accepted)
    .bind(T0)
    .execute(pool)
    .await
    .unwrap();
}

async fn status(pool: &SqlitePool, account: i64, deleted: bool, reblog_of: Option<i64>) {
    sqlx::query(
        "INSERT INTO statuses (account_id, content, spoiler_text, visibility, reblog_of_id, \
         created_at, deleted_at) VALUES (?, 'c', '', 'PUBLIC', ?, ?, ?)",
    )
    .bind(account)
    .bind(reblog_of)
    .bind(T0)
    .bind(deleted.then_some(T0))
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn profile_counts_are_zero_on_an_empty_instance() {
    let pool = pool().await;
    let id = seed(&pool, "a", "USER", false).await;
    assert_eq!(store::profile_counts(&pool, id).await.unwrap(), ProfileCounts::default());
}

#[tokio::test]
async fn profile_counts_read_follows_and_statuses_like_python() {
    // M-02. Followers and following count accepted follows only. Statuses skip deleted rows
    // and boosts, and other accounts' rows.
    let pool = pool().await;
    let a = seed(&pool, "a", "USER", false).await;
    let b = seed(&pool, "b", "USER", false).await;
    let c = seed(&pool, "c", "USER", false).await;
    follow(&pool, b, a, true).await;
    follow(&pool, c, a, true).await;
    follow(&pool, a, b, true).await;
    follow(&pool, a, c, false).await;
    status(&pool, a, false, None).await;
    status(&pool, a, false, None).await;
    status(&pool, a, true, None).await;
    status(&pool, a, false, Some(1)).await;
    status(&pool, b, false, None).await;
    assert_eq!(
        store::profile_counts(&pool, a).await.unwrap(),
        ProfileCounts { followers: 2, following: 1, statuses: 2 }
    );
    assert_eq!(
        store::profile_counts(&pool, b).await.unwrap(),
        ProfileCounts { followers: 1, following: 1, statuses: 1 }
    );
    assert_eq!(
        store::profile_counts(&pool, c).await.unwrap(),
        ProfileCounts { followers: 0, following: 1, statuses: 0 }
    );
}

#[tokio::test]
async fn store_leaves_the_schema_untouched() {
    let pool = pool().await;
    let before: Vec<String> = sqlx::query_scalar("SELECT sql FROM sqlite_master ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    let id = seed(&pool, "a", "USER", false).await;
    store::insert_account(&pool, &new_account("z", "z@x.org", Role::User)).await.unwrap();
    store::update_profile(&pool, id, Some("x"), None, None).await.unwrap();
    store::insert_token(&pool, &token_row("t", id)).await.unwrap();
    store::profile_counts(&pool, id).await.unwrap();
    let after: Vec<String> = sqlx::query_scalar("SELECT sql FROM sqlite_master ORDER BY name")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}
