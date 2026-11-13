//! Wave 1 startup test (REQ-017, NFR-011): the binary refuses to start on a database without the
//! Python schema, says why, and creates nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Command;

use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::{Connection, Row};

fn run_with_url(url: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_minidon-rust"))
        .env("MINIDON_DATABASE_URL", url)
        .output()
        .unwrap()
}

#[tokio::test]
async fn empty_database_stops_startup_with_a_clear_message_and_no_ddl() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    SqliteConnection::connect_with(&options)
        .await
        .unwrap()
        .close()
        .await
        .unwrap();

    let output = run_with_url(&format!("sqlite:///{}", path.display()));
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing tables"), "{stderr}");
    assert!(stderr.contains("accounts"), "{stderr}");
    assert!(stderr.contains("never creates"), "{stderr}");

    let mut conn = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path))
        .await
        .unwrap();
    let tables: i64 = sqlx::query("SELECT COUNT(*) FROM sqlite_master")
        .fetch_one(&mut conn)
        .await
        .unwrap()
        .get(0);
    assert_eq!(tables, 0, "startup must not run DDL");
}

#[test]
fn missing_database_file_stops_startup_and_is_not_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("absent.db");
    let output = run_with_url(&format!("sqlite:///{}", path.display()));
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("absent.db"));
    assert!(!path.exists());
}

#[test]
fn unsupported_database_url_stops_startup() {
    let output = run_with_url("mysql://nope");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("MINIDON_DATABASE_URL"));
}
