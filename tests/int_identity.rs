//! Wave 3 integration tests for `accounts::identity` (IT-ACC-011, IT-ACC-012, SC-ACC-018,
//! SC-ACC-020..022, review M-01). The stable API other capabilities call (CF-004).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use minidon_rust::accounts::identity;
use minidon_rust::accounts::model::Role;
use minidon_rust::platform::clock::{Clock, FixedClock};
use minidon_rust::platform::error::AppError;
use sqlx::SqlitePool;
use time::macros::datetime;
use time::Duration;

const T0: &str = "2026-01-01 00:00:00.000000";

async fn pool() -> SqlitePool {
    support::schema_state().await.pool
}

async fn seed(pool: &SqlitePool, username: &str, suspended: bool) -> i64 {
    sqlx::query(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, \
         suspended_at, created_at) VALUES (?, ?, 'h', '', '', 0, 'USER', ?, ?)",
    )
    .bind(username)
    .bind(format!("{}@x.org", username.replace('\n', "_nl")))
    .bind(suspended.then_some(T0))
    .bind(T0)
    .execute(pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

async fn seed_token(pool: &SqlitePool, token: &str, account_id: i64, expires_at: &str) {
    sqlx::query(
        "INSERT INTO access_tokens (token, account_id, created_at, expires_at) VALUES (?, ?, ?, ?)",
    )
    .bind(token)
    .bind(account_id)
    .bind(T0)
    .bind(expires_at)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn get_by_username_finds_an_active_account_in_any_case() {
    let pool = pool().await;
    seed(&pool, "kim", false).await;
    for name in ["kim", "KIM", "Kim"] {
        assert_eq!(
            identity::get_by_username(&pool, name)
                .await
                .unwrap()
                .username,
            "kim"
        );
    }
}

#[tokio::test]
async fn get_by_username_hides_suspended_accounts_in_any_case() {
    let pool = pool().await;
    seed(&pool, "lou", true).await;
    for name in ["lou", "LOU", "Lou"] {
        let err = identity::get_by_username(&pool, name).await.unwrap_err();
        assert_eq!(
            err,
            AppError::NotFound("Account not found".into()),
            "{name}"
        );
    }
}

#[tokio::test]
async fn get_by_username_unknown_is_not_found() {
    let pool = pool().await;
    let err = identity::get_by_username(&pool, "nobody")
        .await
        .unwrap_err();
    assert_eq!(err, AppError::NotFound("Account not found".into()));
}

#[tokio::test]
async fn get_by_username_keeps_the_trailing_newline_look_alike_apart() {
    let pool = pool().await;
    let plain = seed(&pool, "alice", false).await;
    let newline = seed(&pool, "alice\n", false).await;
    assert_eq!(
        identity::get_by_username(&pool, "alice").await.unwrap().id,
        plain
    );
    // What `GET /accounts/alice%0A` reaches after URL decoding.
    assert_eq!(
        identity::get_by_username(&pool, "alice\n")
            .await
            .unwrap()
            .id,
        newline
    );
    assert_eq!(
        identity::get_by_username(&pool, "ALICE\n")
            .await
            .unwrap()
            .id,
        newline
    );
}

#[tokio::test]
async fn the_including_suspended_variant_finds_a_suspended_account() {
    let pool = pool().await;
    seed(&pool, "lou", true).await;
    seed(&pool, "kim", false).await;
    let lou = identity::get_by_username_including_suspended(&pool, "LOU")
        .await
        .unwrap();
    assert!(lou.unwrap().suspended_at.is_some());
    let kim = identity::get_by_username_including_suspended(&pool, "kim")
        .await
        .unwrap();
    assert!(kim.unwrap().suspended_at.is_none());
    assert!(identity::get_by_username_including_suspended(&pool, "zed")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn get_by_id_returns_any_account_including_suspended() {
    let pool = pool().await;
    let id = seed(&pool, "lou", true).await;
    assert_eq!(
        identity::get_by_id(&pool, id)
            .await
            .unwrap()
            .unwrap()
            .username,
        "lou"
    );
    assert!(identity::get_by_id(&pool, id + 1).await.unwrap().is_none());
}

#[tokio::test]
async fn a_valid_token_authenticates() {
    let pool = pool().await;
    let id = seed(&pool, "ivy", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    let account = identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(account.id, id);
    assert_eq!(account.role, Role::User);
}

#[tokio::test]
async fn it_acc_expiry_boundary_with_a_fixed_clock() {
    // Token issued at T0, valid for 30 days: valid 1 s before, signed out at the instant and after.
    let pool = pool().await;
    let id = seed(&pool, "ivy", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock = FixedClock::new(datetime!(2026-01-31 00:00:00));
    clock.advance(Duration::seconds(-1));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_some());
    clock.advance(Duration::seconds(1));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_none());
    clock.advance(Duration::seconds(1));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn unknown_empty_and_odd_tokens_are_signed_out() {
    let pool = pool().await;
    let id = seed(&pool, "ivy", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    for token in ["", "TOK", "tok ", "nope", "' OR 1=1 --", "tok\u{0}"] {
        assert!(
            identity::authenticate(&pool, &clock, token)
                .await
                .unwrap()
                .is_none(),
            "{token:?}"
        );
    }
}

#[tokio::test]
async fn suspending_signs_out_on_the_very_next_request() {
    let pool = pool().await;
    let id = seed(&pool, "jay", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_some());
    sqlx::query("UPDATE accounts SET suspended_at = ? WHERE id = ?")
        .bind(T0)
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_none());
    // And back: nothing is cached, so unsuspending is seen at once too.
    sqlx::query("UPDATE accounts SET suspended_at = NULL WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn a_token_of_a_deleted_account_row_is_signed_out() {
    let pool = pool().await;
    let id = seed(&pool, "gone", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    sqlx::query("DELETE FROM accounts WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn deleting_the_token_row_signs_out_at_once() {
    let pool = pool().await;
    let id = seed(&pool, "ivy", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock = FixedClock::new(datetime!(2026-01-01 00:00:00));
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_some());
    sqlx::query("DELETE FROM access_tokens")
        .execute(&pool)
        .await
        .unwrap();
    assert!(identity::authenticate(&pool, &clock, "tok")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn authenticate_uses_the_clock_it_is_given() {
    let pool = pool().await;
    let id = seed(&pool, "ivy", false).await;
    seed_token(&pool, "tok", id, "2026-01-31 00:00:00.000000").await;
    let clock: Box<dyn Clock> = Box::new(FixedClock::new(datetime!(2030-01-01 00:00:00)));
    assert!(identity::authenticate(&pool, clock.as_ref(), "tok")
        .await
        .unwrap()
        .is_none());
}
