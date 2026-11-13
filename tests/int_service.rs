//! Wave 3 integration tests for `accounts::service` (IT-ACC-004..IT-ACC-014, SC-ACC-001..027,
//! review M-03 pure part, m-03, m-12). In-memory SQLite with a copy of the Python-made schema,
//! a fixed clock at T0 = 2026-01-01 00:00:00.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use minidon_rust::accounts::model::Role;
use minidon_rust::accounts::password::{hash_password_with_salt, verify_password};
use minidon_rust::accounts::service;
use minidon_rust::platform::clock::FixedClock;
use minidon_rust::platform::error::AppError;
use sqlx::{Row, SqlitePool};
use time::macros::datetime;

const T0: &str = "2026-01-01 00:00:00.000000";
const GOOD_PW: &str = "correct-horse";

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

impl Env {
    async fn register(
        &self,
        username: &str,
        email: &str,
        password: &str,
    ) -> Result<minidon_rust::accounts::model::Account, AppError> {
        service::register(&self.pool, &self.clock, username, email, password).await
    }

    async fn ok(&self, username: &str) -> minidon_rust::accounts::model::Account {
        let email = format!("{}@x.org", username.replace('\n', "_nl"));
        self.register(username, &email, GOOD_PW).await.unwrap()
    }

    async fn sign_in(
        &self,
        username: &str,
        password: &str,
    ) -> Result<minidon_rust::accounts::model::AccessToken, AppError> {
        service::sign_in(&self.pool, &self.clock, 30, username, password).await
    }

    async fn suspend(&self, username: &str) {
        sqlx::query("UPDATE accounts SET suspended_at = ? WHERE username = ?")
            .bind(T0)
            .bind(username)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn accounts(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn invalid(msg: &str) -> AppError {
    AppError::Invalid(msg.into())
}

fn conflict(msg: &str) -> AppError {
    AppError::Conflict(msg.into())
}

// ---- register: BR-001..BR-006 ----

#[tokio::test]
async fn sc_acc_001_valid_usernames_are_lower_cased() {
    let e = env().await;
    let account = e
        .register("Alice_01", "alice@x.org", GOOD_PW)
        .await
        .unwrap();
    assert_eq!(account.username, "alice_01");
    let stored: String = sqlx::query_scalar("SELECT username FROM accounts")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(stored, "alice_01");
    assert_eq!(
        e.register(&"a".repeat(30), "b@x.org", GOOD_PW)
            .await
            .unwrap()
            .username
            .len(),
        30
    );
}

#[tokio::test]
async fn sc_acc_002_invalid_usernames_are_rejected_with_the_python_message() {
    let e = env().await;
    let msg = "Username must be 1-30 letters, digits or underscores";
    let long = "a".repeat(31);
    for (i, name) in [
        "",
        &long,
        "bad-name",
        "has space",
        "１２３",
        "Ålice",
        "用户",
        "İstanbul",
    ]
    .iter()
    .enumerate()
    {
        let result = e.register(name, &format!("u{i}@x.org"), GOOD_PW).await;
        assert_eq!(result.unwrap_err(), invalid(msg), "{name:?}");
    }
    assert_eq!(e.accounts().await, 0);
}

#[tokio::test]
async fn sc_acc_003_kelvin_sign_folds_to_k() {
    let e = env().await;
    let account = e
        .register("\u{212A}elvin", "k@x.org", GOOD_PW)
        .await
        .unwrap();
    assert_eq!(account.username, "kelvin");
}

#[tokio::test]
async fn sc_acc_004_trailing_newline_username_is_accepted_and_kept() {
    // DELIBERATE parity copy, human decision 2026-11-13 (RC-001).
    let e = env().await;
    let with_newline = e.ok("alice\n").await;
    assert_eq!(with_newline.username, "alice\n");
    let plain = e.ok("alice").await;
    assert_ne!(
        plain.id, with_newline.id,
        "look-alike pair are two accounts"
    );
    let stored: Vec<String> = sqlx::query_scalar("SELECT username FROM accounts ORDER BY id")
        .fetch_all(&e.pool)
        .await
        .unwrap();
    assert_eq!(stored, ["alice\n", "alice"]);
}

#[tokio::test]
async fn sc_acc_004_reverse_order_and_case_variant() {
    let e = env().await;
    e.ok("alice").await;
    e.ok("alice\n").await;
    // ALICE\n lower-cases to alice\n, which exists.
    let result = e.register("ALICE\n", "other@x.org", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), conflict("Username is taken"));
}

#[tokio::test]
async fn sc_acc_004_edge_set() {
    let e = env().await;
    let msg = "Username must be 1-30 letters, digits or underscores";
    for (i, name) in ["alice\n\n", "\n", "ali\nce", "alice\r\n"]
        .iter()
        .enumerate()
    {
        let result = e.register(name, &format!("e{i}@x.org"), GOOD_PW).await;
        assert_eq!(result.unwrap_err(), invalid(msg), "{name:?}");
    }
    let thirty = format!("{}\n", "a".repeat(30));
    assert_eq!(
        e.register(&thirty, "t@x.org", GOOD_PW)
            .await
            .unwrap()
            .username,
        thirty
    );
}

#[tokio::test]
async fn lone_surrogate_username_stand_in_gets_the_username_422() {
    // adr-001 decision 6: wave 4 turns a lone surrogate escape in `username` into U+FFFD, and
    // the pure rules then give Python's 422 with an `{"error": ...}` body.
    let e = env().await;
    let result = e.register("ali\u{FFFD}ce", "a@x.org", GOOD_PW).await;
    assert_eq!(
        result.unwrap_err(),
        invalid("Username must be 1-30 letters, digits or underscores")
    );
}

#[tokio::test]
async fn sc_acc_006_email_is_trimmed_lower_cased_and_only_needs_an_at() {
    let e = env().await;
    assert_eq!(
        e.register("bob", "  Bob@Example.COM ", GOOD_PW)
            .await
            .unwrap()
            .email,
        "bob@example.com"
    );
    assert_eq!(e.register("at", "@", GOOD_PW).await.unwrap().email, "@");
    assert_eq!(
        e.register("umlaut", "ÄÖ@Ü.de", GOOD_PW)
            .await
            .unwrap()
            .email,
        "äö@ü.de"
    );
    assert_eq!(
        e.register("nbsp", "\u{A0}nb@x.org\u{A0}", GOOD_PW)
            .await
            .unwrap()
            .email,
        "nb@x.org"
    );
}

#[tokio::test]
async fn sc_acc_005_email_without_at_is_rejected() {
    let e = env().await;
    let result = e.register("bob", "not-an-email", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), invalid("Email address is not valid"));
    let result = e.register("bob", "   ", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), invalid("Email address is not valid"));
}

#[tokio::test]
async fn sc_acc_007_password_length_in_code_points() {
    let e = env().await;
    let msg = "Password must be at least 8 characters";
    assert_eq!(
        e.register("a1", "a1@x.org", "1234567").await.unwrap_err(),
        invalid(msg)
    );
    assert_eq!(
        e.register("a2", "a2@x.org", &"😀".repeat(7))
            .await
            .unwrap_err(),
        invalid(msg)
    );
    e.register("a3", "a3@x.org", "12345678").await.unwrap();
    e.register("a4", "a4@x.org", &"😀".repeat(8)).await.unwrap();
}

#[tokio::test]
async fn validation_order_is_username_then_email_then_password() {
    let e = env().await;
    let all_bad = e.register("bad name", "nope", "short").await;
    assert_eq!(
        all_bad.unwrap_err(),
        invalid("Username must be 1-30 letters, digits or underscores")
    );
    let email_and_pw = e.register("ok", "nope", "short").await;
    assert_eq!(
        email_and_pw.unwrap_err(),
        invalid("Email address is not valid")
    );
    let pw_only = e.register("ok", "ok@x.org", "short").await;
    assert_eq!(
        pw_only.unwrap_err(),
        invalid("Password must be at least 8 characters")
    );
}

#[tokio::test]
async fn validation_comes_before_the_uniqueness_checks() {
    let e = env().await;
    e.ok("carol").await;
    let result = e.register("carol", "bad", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), invalid("Email address is not valid"));
}

#[tokio::test]
async fn sc_acc_008_duplicate_username_in_any_case() {
    let e = env().await;
    e.ok("carol").await;
    let result = e.register("CAROL", "new@x.org", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), conflict("Username is taken"));
    assert_eq!(e.accounts().await, 1);
}

#[tokio::test]
async fn sc_acc_009_a_suspended_username_stays_reserved() {
    let e = env().await;
    e.ok("dave").await;
    e.suspend("dave").await;
    let result = e.register("dave", "new@x.org", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), conflict("Username is taken"));
}

#[tokio::test]
async fn sc_acc_010_duplicate_email_including_suspended_and_case_variants() {
    let e = env().await;
    e.register("erin", "erin@x.org", GOOD_PW).await.unwrap();
    e.register("frank", "frank@x.org", GOOD_PW).await.unwrap();
    e.suspend("frank").await;
    let taken = conflict("Email is already registered");
    assert_eq!(
        e.register("new1", "ERIN@x.org", GOOD_PW).await.unwrap_err(),
        taken
    );
    assert_eq!(
        e.register("new2", " frank@x.org ", GOOD_PW)
            .await
            .unwrap_err(),
        taken
    );
}

#[tokio::test]
async fn username_conflict_is_reported_before_email_conflict() {
    let e = env().await;
    e.register("carol", "carol@x.org", GOOD_PW).await.unwrap();
    let result = e.register("carol", "carol@x.org", GOOD_PW).await;
    assert_eq!(result.unwrap_err(), conflict("Username is taken"));
}

#[tokio::test]
async fn sc_acc_011_first_account_is_admin_later_ones_are_users() {
    let e = env().await;
    assert_eq!(e.ok("first").await.role, Role::Admin);
    assert_eq!(e.ok("second").await.role, Role::User);
    assert_eq!(e.ok("third").await.role, Role::User);
    let roles: Vec<String> = sqlx::query_scalar("SELECT role FROM accounts ORDER BY id")
        .fetch_all(&e.pool)
        .await
        .unwrap();
    assert_eq!(roles, ["ADMIN", "USER", "USER"]);
}

#[tokio::test]
async fn sc_acc_012_admin_is_decided_by_count_even_when_all_are_suspended() {
    let e = env().await;
    e.ok("a").await;
    e.ok("b").await;
    e.suspend("a").await;
    e.suspend("b").await;
    assert_eq!(e.ok("c").await.role, Role::User);
}

#[tokio::test]
async fn registration_writes_python_defaults_and_the_clock_time() {
    let e = env().await;
    let account = e.ok("zed").await;
    assert_eq!(account.created_at, datetime!(2026-01-01 00:00:00));
    let row =
        sqlx::query("SELECT display_name, bio, locked, suspended_at, created_at FROM accounts")
            .fetch_one(&e.pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("display_name"), "");
    assert_eq!(row.get::<String, _>("bio"), "");
    assert_eq!(row.get::<i64, _>("locked"), 0);
    assert_eq!(row.get::<Option<String>, _>("suspended_at"), None);
    assert_eq!(row.get::<String, _>("created_at"), T0);
}

#[tokio::test]
async fn sc_acc_013_015_the_stored_hash_is_never_the_plaintext_and_verifies() {
    let e = env().await;
    let account = e.register("hash", "h@x.org", "my secret pw").await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM accounts")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(stored, account.password_hash);
    assert!(!stored.contains("my secret pw"));
    assert_eq!(stored.len(), 32 + 1 + 64);
    assert_eq!(verify_password("my secret pw", &stored), Ok(true));
}

// ---- sign in: BR-007..BR-010 ----

async fn seed_with_hash(e: &Env, username: &str, password: &str, suspended: bool) -> i64 {
    let stored = hash_password_with_salt(password, &[7u8; 16]);
    sqlx::query(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, \
         suspended_at, created_at) VALUES (?, ?, ?, '', '', 0, 'USER', ?, ?)",
    )
    .bind(username)
    .bind(format!("{}@x.org", username.replace('\n', "_nl")))
    .bind(stored)
    .bind(suspended.then_some(T0))
    .bind(T0)
    .execute(&e.pool)
    .await
    .unwrap()
    .last_insert_rowid()
}

#[tokio::test]
async fn sign_in_issues_a_43_char_token_for_30_days_and_stores_it() {
    let e = env().await;
    let account = e.ok("ivy").await;
    let access = e.sign_in("ivy", GOOD_PW).await.unwrap();
    assert_eq!(access.token.len(), 43);
    assert_eq!(access.account_id, account.id);
    assert_eq!(access.created_at, datetime!(2026-01-01 00:00:00));
    assert_eq!(access.expires_at, datetime!(2026-01-31 00:00:00));
    let row = sqlx::query("SELECT token, account_id, created_at, expires_at FROM access_tokens")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("token"), access.token);
    assert_eq!(
        row.get::<String, _>("expires_at"),
        "2026-01-31 00:00:00.000000"
    );
}

#[tokio::test]
async fn the_token_ttl_is_a_parameter() {
    let e = env().await;
    e.ok("ivy").await;
    let access = service::sign_in(&e.pool, &e.clock, 7, "ivy", GOOD_PW)
        .await
        .unwrap();
    assert_eq!(access.expires_at, datetime!(2026-01-08 00:00:00));
}

#[tokio::test]
async fn sign_in_finds_the_account_in_any_case() {
    let e = env().await;
    e.ok("ivy").await;
    assert!(e.sign_in("IVY", GOOD_PW).await.is_ok());
    assert!(e.sign_in("Ivy", GOOD_PW).await.is_ok());
}

#[tokio::test]
async fn every_sign_in_issues_a_new_token_and_keeps_the_old_ones() {
    let e = env().await;
    e.ok("ivy").await;
    let first = e.sign_in("ivy", GOOD_PW).await.unwrap();
    let second = e.sign_in("ivy", GOOD_PW).await.unwrap();
    assert_ne!(first.token, second.token);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM access_tokens")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[tokio::test]
async fn sc_acc_016_wrong_password_and_unknown_user_look_identical() {
    let e = env().await;
    e.ok("gina").await;
    let wrong = e.sign_in("gina", "wrong-password").await.unwrap_err();
    let unknown = e.sign_in("nobody", GOOD_PW).await.unwrap_err();
    assert_eq!(
        wrong,
        AppError::Unauthorized("Invalid username or password".into())
    );
    assert_eq!(unknown, wrong);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM access_tokens")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn unknown_user_is_401_even_for_odd_passwords() {
    // Python never hashes for an unknown user (service.py:66-68). The service must not either,
    // so a password that cannot be hashed would still be a 401 here (adr-001 decision 6).
    let e = env().await;
    for pw in ["", "\u{FFFD}", &"x".repeat(100_000)] {
        let err = e.sign_in("nobody", pw).await.unwrap_err();
        assert_eq!(
            err,
            AppError::Unauthorized("Invalid username or password".into())
        );
    }
}

#[tokio::test]
async fn fifty_wrong_passwords_are_all_401_and_nothing_locks_out() {
    let e = env().await;
    seed_with_hash(&e, "gina", GOOD_PW, false).await;
    for i in 0..50 {
        let err = e.sign_in("gina", &format!("wrong-{i}")).await.unwrap_err();
        assert_eq!(
            err,
            AppError::Unauthorized("Invalid username or password".into())
        );
    }
    assert!(e.sign_in("gina", GOOD_PW).await.is_ok());
}

#[tokio::test]
async fn sc_acc_017_suspended_account_with_the_right_password_is_403_and_gets_no_token() {
    let e = env().await;
    seed_with_hash(&e, "hank", GOOD_PW, true).await;
    let err = e.sign_in("hank", GOOD_PW).await.unwrap_err();
    assert_eq!(err, AppError::Forbidden("This account is suspended".into()));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM access_tokens")
        .fetch_one(&e.pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn sc_acc_017_wrong_password_on_a_suspended_account_is_401_not_403() {
    let e = env().await;
    seed_with_hash(&e, "hank", GOOD_PW, true).await;
    let err = e.sign_in("hank", "wrong-password").await.unwrap_err();
    assert_eq!(
        err,
        AppError::Unauthorized("Invalid username or password".into())
    );
}

#[tokio::test]
async fn sign_in_reaches_the_look_alike_account_it_names() {
    let e = env().await;
    let newline = seed_with_hash(&e, "alice\n", "pw-for-newline", false).await;
    let plain = seed_with_hash(&e, "alice", "pw-for-plain", false).await;
    assert_eq!(
        e.sign_in("alice\n", "pw-for-newline")
            .await
            .unwrap()
            .account_id,
        newline
    );
    assert_eq!(
        e.sign_in("alice", "pw-for-plain").await.unwrap().account_id,
        plain
    );
    assert!(e.sign_in("alice", "pw-for-newline").await.is_err());
    assert!(e.sign_in("alice\n", "pw-for-plain").await.is_err());
}

#[tokio::test]
async fn a_tampered_hash_row_is_an_internal_error_and_the_service_keeps_working() {
    let e = env().await;
    e.ok("ok").await;
    e.ok("bad").await;
    sqlx::query("UPDATE accounts SET password_hash = 'not-a-hash' WHERE username = 'bad'")
        .execute(&e.pool)
        .await
        .unwrap();
    assert_eq!(
        e.sign_in("bad", GOOD_PW).await.unwrap_err(),
        AppError::Internal
    );
    assert!(e.sign_in("ok", GOOD_PW).await.is_ok());
}

// ---- profile: BR-013, BR-014 ----

async fn profile_row(e: &Env, id: i64) -> (String, String, i64) {
    let row = sqlx::query("SELECT display_name, bio, locked FROM accounts WHERE id = ?")
        .bind(id)
        .fetch_one(&e.pool)
        .await
        .unwrap();
    (row.get("display_name"), row.get("bio"), row.get("locked"))
}

#[tokio::test]
async fn sc_acc_023_display_name_boundary() {
    let e = env().await;
    let account = e.ok("mia").await;
    let updated = service::update_profile(&e.pool, &account, Some(&"a".repeat(30)), None, None)
        .await
        .unwrap();
    assert_eq!(updated.display_name.len(), 30);
    let err = service::update_profile(&e.pool, &account, Some(&"a".repeat(31)), None, None)
        .await
        .unwrap_err();
    assert_eq!(err, invalid("Display name must be at most 30 characters"));
}

#[tokio::test]
async fn sc_acc_024_bio_boundary() {
    let e = env().await;
    let account = e.ok("mia").await;
    assert!(
        service::update_profile(&e.pool, &account, None, Some(&"a".repeat(500)), None)
            .await
            .is_ok()
    );
    let err = service::update_profile(&e.pool, &account, None, Some(&"a".repeat(501)), None)
        .await
        .unwrap_err();
    assert_eq!(err, invalid("Bio must be at most 500 characters"));
}

#[tokio::test]
async fn sc_acc_025_limits_count_code_points() {
    let e = env().await;
    let account = e.ok("mia").await;
    let marks = "e\u{301}".repeat(15);
    for ok in ["😀".repeat(30), "用".repeat(30), marks.clone()] {
        assert!(
            service::update_profile(&e.pool, &account, Some(&ok), None, None)
                .await
                .is_ok()
        );
    }
    for bad in ["😀".repeat(31), "用".repeat(31), format!("{marks}x")] {
        assert!(
            service::update_profile(&e.pool, &account, Some(&bad), None, None)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn sc_acc_026_partial_update_leaves_other_fields_alone() {
    let e = env().await;
    let account = e.ok("mia").await;
    service::update_profile(&e.pool, &account, Some("Mia"), Some("old bio"), Some(true))
        .await
        .unwrap();
    let account = minidon_rust::accounts::identity::get_by_id(&e.pool, account.id)
        .await
        .unwrap()
        .unwrap();
    let updated = service::update_profile(&e.pool, &account, None, Some("new bio"), None)
        .await
        .unwrap();
    assert_eq!(updated.display_name, "Mia");
    assert_eq!(updated.bio, "new bio");
    assert!(updated.locked);
    assert_eq!(
        profile_row(&e, account.id).await,
        ("Mia".into(), "new bio".into(), 1)
    );
}

#[tokio::test]
async fn nothing_given_changes_nothing() {
    let e = env().await;
    let account = e.ok("mia").await;
    let updated = service::update_profile(&e.pool, &account, None, None, None)
        .await
        .unwrap();
    assert_eq!(updated, account);
}

#[tokio::test]
async fn empty_strings_are_values() {
    let e = env().await;
    let account = e.ok("mia").await;
    service::update_profile(&e.pool, &account, Some("Mia"), Some("bio"), None)
        .await
        .unwrap();
    let updated = service::update_profile(&e.pool, &account, Some(""), Some(""), None)
        .await
        .unwrap();
    assert_eq!(
        (updated.display_name.as_str(), updated.bio.as_str()),
        ("", "")
    );
}

#[tokio::test]
async fn m_12_an_invalid_field_changes_nothing_even_when_another_field_is_valid() {
    // REQ-014-AC1 "profile unchanged": all fields are validated before anything is written.
    let e = env().await;
    let account = e.ok("mia").await;
    service::update_profile(&e.pool, &account, Some("Mia"), Some("bio"), Some(false))
        .await
        .unwrap();
    let err = service::update_profile(
        &e.pool,
        &account,
        Some("New"),
        Some(&"b".repeat(501)),
        Some(true),
    )
    .await
    .unwrap_err();
    assert_eq!(err, invalid("Bio must be at most 500 characters"));
    assert_eq!(
        profile_row(&e, account.id).await,
        ("Mia".into(), "bio".into(), 0)
    );
}

#[tokio::test]
async fn display_name_error_wins_when_both_are_invalid() {
    let e = env().await;
    let account = e.ok("mia").await;
    let err = service::update_profile(
        &e.pool,
        &account,
        Some(&"a".repeat(31)),
        Some(&"b".repeat(501)),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err, invalid("Display name must be at most 30 characters"));
}

#[tokio::test]
async fn sc_acc_027_locked_flag_round_trip() {
    let e = env().await;
    let account = e.ok("nia").await;
    assert!(!account.locked);
    let updated = service::update_profile(&e.pool, &account, None, None, Some(true))
        .await
        .unwrap();
    assert!(updated.locked);
    let seen = minidon_rust::accounts::identity::get_by_username(&e.pool, "nia")
        .await
        .unwrap();
    assert!(seen.locked);
    let updated = service::update_profile(&e.pool, &seen, None, None, Some(false))
        .await
        .unwrap();
    assert!(!updated.locked);
    assert_eq!(profile_row(&e, account.id).await.2, 0);
}

// ---- serialisation ----

#[tokio::test]
async fn sc_acc_029_profile_shape_with_zero_counts() {
    let e = env().await;
    let account = e.ok("kim").await;
    let out = service::serialize_account(&e.pool, "minidon.local", &account)
        .await
        .unwrap();
    assert_eq!(out.acct, "kim@minidon.local");
    assert_eq!(out.role, Role::Admin);
    assert_eq!(out.created_at, "2026-01-01T00:00:00");
    assert_eq!(
        (out.followers_count, out.following_count, out.statuses_count),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn m_02_profile_counts_come_from_the_follows_and_statuses_tables() {
    let e = env().await;
    let a = e.ok("a").await;
    let b = e.ok("b").await;
    sqlx::query("INSERT INTO follows (follower_id, followee_id, accepted, created_at) VALUES (?, ?, 1, ?), (?, ?, 0, ?)")
        .bind(b.id).bind(a.id).bind(T0)
        .bind(a.id).bind(b.id).bind(T0)
        .execute(&e.pool).await.unwrap();
    sqlx::query("INSERT INTO statuses (account_id, content, spoiler_text, visibility, created_at) VALUES (?, 'x', '', 'PUBLIC', ?)")
        .bind(a.id).bind(T0).execute(&e.pool).await.unwrap();
    let out = service::serialize_account(&e.pool, "d", &a).await.unwrap();
    assert_eq!(
        (out.followers_count, out.following_count, out.statuses_count),
        (1, 0, 1)
    );
    let out = service::serialize_account(&e.pool, "d", &b).await.unwrap();
    assert_eq!(
        (out.followers_count, out.following_count, out.statuses_count),
        (0, 1, 0)
    );
}
