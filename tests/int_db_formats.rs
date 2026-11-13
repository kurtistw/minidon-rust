//! Wave 1 integration tests for `platform::db` (REQ-017, NFR-011, D2, review M-04, M-09).
//!
//! The schema comes from `tests/fixtures/python_schema.sql`, generated from the Python app at
//! e1fbd3a (`create_tables()`, accounts, access_tokens, follows and statuses only). Only the tests apply it. The
//! production code never runs DDL.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::platform::clock::{format_stored, parse_stored};
use minidon_rust::platform::config::Settings;
use minidon_rust::platform::db::{connect, connect_and_verify, verify_schema};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{Connection, Row};
use std::path::Path;
use time::macros::datetime;

const PYTHON_SCHEMA: &str = include_str!("fixtures/python_schema.sql");

fn url_for(path: &Path) -> String {
    format!("sqlite:///{}", path.display())
}

fn settings_for(path: &Path) -> Settings {
    Settings {
        database_url: url_for(path),
        instance_domain: "minidon.local".into(),
        token_ttl_days: 30,
    }
}

/// A database file the way Python leaves it: schema applied, connection closed.
async fn python_made_db(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("minidon.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
    for statement in PYTHON_SCHEMA
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();
    path
}

#[tokio::test]
async fn opens_a_python_made_database_and_accepts_its_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = python_made_db(&dir).await;
    let pool = connect_and_verify(&settings_for(&path)).await.unwrap();
    let row = sqlx::query("SELECT COUNT(*) AS n FROM accounts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i64, _>("n"), 0);
}

#[tokio::test]
async fn empty_database_fails_clearly_and_no_ddl_is_run() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.db");
    // An existing, empty database file.
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    SqliteConnection::connect_with(&options)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();

    let pool = connect(&settings_for(&path)).await.unwrap();
    let err = verify_schema(&pool).await.unwrap_err().to_string();
    assert!(
        err.contains("accounts") && err.contains("access_tokens"),
        "{err}"
    );
    assert!(err.contains("never creates"), "{err}");

    let tables: i64 = sqlx::query("SELECT COUNT(*) AS n FROM sqlite_master")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
    assert_eq!(tables, 0, "Rust must not create any table");
}

#[tokio::test]
async fn one_missing_table_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = python_made_db(&dir).await;
    let options = SqliteConnectOptions::new().filename(&path);
    let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("DROP TABLE access_tokens")
        .execute(&mut conn)
        .await
        .unwrap();
    conn.close().await.unwrap();

    let err = connect_and_verify(&settings_for(&path))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("access_tokens"), "{err}");
    assert!(!err.contains("tables: accounts"), "accounts exists: {err}");
}

#[tokio::test]
async fn missing_file_is_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.db");
    let err = connect_and_verify(&settings_for(&path))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("absent.db"), "{err}");
    assert!(!path.exists(), "Rust must not create the database file");
}

#[tokio::test]
async fn connecting_and_verifying_leaves_the_file_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = python_made_db(&dir).await;
    let before = std::fs::read(&path).unwrap();
    let pool = connect_and_verify(&settings_for(&path)).await.unwrap();
    pool.close().await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let siblings: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        siblings,
        vec!["minidon.db".to_owned()],
        "no -wal or -journal left behind"
    );
}

#[tokio::test]
async fn connection_pragmas_match_a_default_python_connection() {
    // SQLAlchemy with pysqlite: rollback journal, foreign keys not enforced.
    let dir = tempfile::tempdir().unwrap();
    let path = python_made_db(&dir).await;
    let pool = connect_and_verify(&settings_for(&path)).await.unwrap();
    let journal: String = sqlx::query("PRAGMA journal_mode")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(journal.to_lowercase(), "delete");
    let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(foreign_keys, 0);
}

#[tokio::test]
async fn unsupported_url_is_a_clear_startup_error() {
    let settings = Settings {
        database_url: "postgresql://u@h/db".into(),
        ..Settings::from_lookup(|_| None)
    };
    let err = connect(&settings).await.unwrap_err().to_string();
    assert!(err.contains("MINIDON_DATABASE_URL"), "{err}");
}

#[tokio::test]
async fn stored_datetime_text_is_written_and_read_in_the_python_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = python_made_db(&dir).await;
    let pool = connect_and_verify(&settings_for(&path)).await.unwrap();

    let created = datetime!(2026-01-01 00:00:00);
    sqlx::query(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, \
         suspended_at, created_at) VALUES (?, ?, ?, '', '', 0, 'USER', NULL, ?)",
    )
    .bind("alice")
    .bind("a@example.test")
    .bind("x")
    .bind(format_stored(created))
    .execute(&pool)
    .await
    .unwrap();

    let raw: String = sqlx::query("SELECT created_at FROM accounts WHERE username = ?")
        .bind("alice")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get(0);
    assert_eq!(raw, "2026-01-01 00:00:00.000000");
    assert_eq!(parse_stored(&raw).unwrap(), created);

    // Text exactly as SQLAlchemy writes it for a datetime with microseconds.
    assert_eq!(
        parse_stored("2026-03-04 05:06:07.080910").unwrap(),
        datetime!(2026-03-04 05:06:07.080_910)
    );
}

#[tokio::test]
async fn profile_count_tables_are_checked_too() {
    // Profile counts read `follows` and `statuses` (review M-02), so startup checks them.
    for (table, drop) in [
        ("follows", "DROP TABLE follows"),
        ("statuses", "DROP TABLE statuses"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = python_made_db(&dir).await;
        let options = SqliteConnectOptions::new().filename(&path);
        let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
        sqlx::query(drop).execute(&mut conn).await.unwrap();
        conn.close().await.unwrap();

        let err = connect_and_verify(&settings_for(&path))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains(table), "{err}");
    }
}
