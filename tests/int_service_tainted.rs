//! Wave 4 service tests for lone-surrogate input (adr-001 section 5 decisions 1 and 6, plan
//! review M-03). The handler scrubs the JSON body and tells the service which fields held a lone
//! surrogate. The service then follows Python's check order, so each field gets the outcome the
//! decisions name:
//!
//! - register username: Python's 422 username message (U+FFFD never matches the pattern).
//! - register email and password: Python answers 500 at the first database or hash step that
//!   sees the text. Rust answers 422 at the same step, so an earlier 422 or 409 still wins.
//! - sign-in password: unknown user stays 401, a known user gets the 422 before the hash and
//!   before the suspension check.
//! - profile display name and bio: the length rules first, then the 422.
//!
//! The sign-in USERNAME with a lone surrogate is not covered by any decision: Python answers a
//! 500, Rust 401 (no such account can exist). That is recorded as unresolved in the build log.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use minidon_rust::accounts::model::{Account, Tainted};
use minidon_rust::accounts::service::{self, UNSTORABLE_MESSAGE};
use minidon_rust::platform::clock::FixedClock;
use minidon_rust::platform::error::AppError;
use sqlx::SqlitePool;
use time::macros::datetime;

const GOOD_PW: &str = "correct-horse";
const FFFD: &str = "\u{fffd}";

struct Env {
    pool: SqlitePool,
    clock: FixedClock,
}

async fn env() -> Env {
    Env {
        pool: support::schema_state().await.pool,
        clock: FixedClock::new(datetime!(2026-01-01 00:00:00)),
    }
}

fn invalid(msg: &str) -> AppError {
    AppError::Invalid(msg.into())
}

impl Env {
    async fn register(&self, u: &str, e: &str, p: &str, t: Tainted) -> Result<Account, AppError> {
        service::register_with(&self.pool, &self.clock, u, e, p, t).await
    }

    async fn plain(&self, u: &str) -> Account {
        self.register(u, &format!("{u}@x.org"), GOOD_PW, Tainted::default())
            .await
            .unwrap()
    }

    async fn count(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn only(field: &str) -> Tainted {
    Tainted::from_fields(&[field.to_owned()])
}

#[test]
fn tainted_is_built_from_field_names_and_ignores_unknown_ones() {
    let t = Tainted::from_fields(&["password".into(), "zzz".into(), "bio".into()]);
    assert!(t.password && t.bio);
    assert!(!t.username && !t.email && !t.display_name);
    assert_eq!(Tainted::from_fields(&[]), Tainted::default());
}

// ---- register ----

#[tokio::test]
async fn register_username_with_a_lone_surrogate_is_the_python_username_422() {
    let e = env().await;
    let name = format!("sur{FFFD}");
    let result = e
        .register(&name, "s@x.org", GOOD_PW, only("username"))
        .await;
    assert_eq!(
        result.unwrap_err(),
        invalid("Username must be 1-30 letters, digits or underscores")
    );
    assert_eq!(e.count().await, 0);
}

#[tokio::test]
async fn register_email_with_a_lone_surrogate_is_refused_and_nothing_is_stored() {
    let e = env().await;
    let email = format!("s{FFFD}@x.org");
    let result = e.register("sur", &email, GOOD_PW, only("email")).await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));
    assert_eq!(e.count().await, 0);
}

#[tokio::test]
async fn register_email_check_order_matches_python() {
    let e = env().await;
    e.plain("alice").await;
    let email = format!("s{FFFD}@x.org");
    // Username taken: Python answers 409 before it ever binds the email.
    let taken = e.register("alice", &email, GOOD_PW, only("email")).await;
    assert_eq!(
        taken.unwrap_err(),
        AppError::Conflict("Username is taken".into())
    );
    // No "@": the email rule fires first, Python 422.
    let no_at = format!("s{FFFD}.x.org");
    let result = e.register("bob", &no_at, GOOD_PW, only("email")).await;
    assert_eq!(result.unwrap_err(), invalid("Email address is not valid"));
    // Short password: 422 first.
    let result = e.register("bob", &email, "short", only("email")).await;
    assert_eq!(
        result.unwrap_err(),
        invalid("Password must be at least 8 characters")
    );
    assert_eq!(e.count().await, 1);
}

#[tokio::test]
async fn register_password_with_a_lone_surrogate_is_refused_after_the_uniqueness_checks() {
    let e = env().await;
    let pw = format!("pass{FFFD}word");
    let result = e.register("sur", "s@x.org", &pw, only("password")).await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));
    assert_eq!(e.count().await, 0);

    e.plain("alice").await;
    // Username taken and email taken still win (Python reaches the hash last).
    let taken = e
        .register("alice", "new@x.org", &pw, only("password"))
        .await;
    assert_eq!(
        taken.unwrap_err(),
        AppError::Conflict("Username is taken".into())
    );
    let email_taken = e
        .register("carol", "alice@x.org", &pw, only("password"))
        .await;
    assert_eq!(
        email_taken.unwrap_err(),
        AppError::Conflict("Email is already registered".into())
    );
    // Too short with the replacement char counted as one code point: 422 first.
    let short = e
        .register("carol", "c@x.org", &format!("a{FFFD}"), only("password"))
        .await;
    assert_eq!(
        short.unwrap_err(),
        invalid("Password must be at least 8 characters")
    );
    assert_eq!(e.count().await, 1);
}

#[tokio::test]
async fn a_tainted_field_the_service_does_not_use_changes_nothing() {
    let e = env().await;
    let account = e
        .register("dave", "d@x.org", GOOD_PW, only("zzz"))
        .await
        .unwrap();
    assert_eq!(account.username, "dave");
}

// ---- sign-in ----

#[tokio::test]
async fn sign_in_unknown_user_with_a_lone_surrogate_password_is_401() {
    let e = env().await;
    let pw = format!("pass{FFFD}word");
    let result =
        service::sign_in_with(&e.pool, &e.clock, 30, "nobody", &pw, only("password")).await;
    assert_eq!(
        result.unwrap_err(),
        AppError::Unauthorized("Invalid username or password".into())
    );
}

#[tokio::test]
async fn sign_in_known_user_with_a_lone_surrogate_password_is_refused_before_hash_and_suspension() {
    let e = env().await;
    e.plain("alice").await;
    let pw = format!("pass{FFFD}word");
    let result =
        service::sign_in_with(&e.pool, &e.clock, 30, "alice", &pw, only("password")).await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));

    sqlx::query("UPDATE accounts SET suspended_at = '2026-01-01 00:00:00.000000'")
        .execute(&e.pool)
        .await
        .unwrap();
    let result =
        service::sign_in_with(&e.pool, &e.clock, 30, "alice", &pw, only("password")).await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM access_tokens")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(tokens, 0);
}

#[tokio::test]
async fn sign_in_username_with_a_lone_surrogate_finds_no_account_unresolved_difference() {
    let e = env().await;
    e.plain("alice").await;
    let name = format!("al{FFFD}ice");
    let result =
        service::sign_in_with(&e.pool, &e.clock, 30, &name, GOOD_PW, only("username")).await;
    assert_eq!(
        result.unwrap_err(),
        AppError::Unauthorized("Invalid username or password".into())
    );
}

// ---- profile ----

#[tokio::test]
async fn profile_text_with_a_lone_surrogate_is_refused_after_the_length_rules() {
    let e = env().await;
    let account = e.plain("mia").await;
    let name = format!("a{FFFD}b");
    let result = service::update_profile_with(
        &e.pool,
        &account,
        Some(&name),
        None,
        None,
        only("display_name"),
    )
    .await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));

    let bio = format!("a{FFFD}");
    let result =
        service::update_profile_with(&e.pool, &account, None, Some(&bio), None, only("bio")).await;
    assert_eq!(result.unwrap_err(), invalid(UNSTORABLE_MESSAGE));

    // Too long: the Python length message, not the unstorable one.
    let long = format!("{FFFD}{}", "a".repeat(30));
    let result = service::update_profile_with(
        &e.pool,
        &account,
        Some(&long),
        None,
        None,
        only("display_name"),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        invalid("Display name must be at most 30 characters")
    );

    // A tainted display name next to a too long bio: Python reaches the bio rule first.
    let long_bio = "b".repeat(501);
    let result = service::update_profile_with(
        &e.pool,
        &account,
        Some(&name),
        Some(&long_bio),
        None,
        only("display_name"),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        invalid("Bio must be at most 500 characters")
    );

    let stored: (String, String) = sqlx::query_as("SELECT display_name, bio FROM accounts")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(stored, (String::new(), String::new()));
}

#[tokio::test]
async fn profile_update_without_a_tainted_field_still_works() {
    let e = env().await;
    let account = e.plain("mia").await;
    let updated = service::update_profile_with(
        &e.pool,
        &account,
        Some("Mia"),
        None,
        Some(true),
        Tainted::default(),
    )
    .await
    .unwrap();
    assert_eq!(updated.display_name, "Mia");
    assert!(updated.locked);
}
