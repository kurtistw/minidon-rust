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

// ---- wave 5: start for real on an address chosen with MINIDON_BIND ----

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

const PYTHON_SCHEMA: &str = include_str!("fixtures/python_schema.sql");

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn database_with_schema(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("app.db");
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

fn http(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn the_binary_serves_on_the_address_given_by_minidon_bind() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_with_schema(dir.path()).await;
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_minidon-rust"))
        .env(
            "MINIDON_DATABASE_URL",
            format!("sqlite:///{}", path.display()),
        )
        .env("MINIDON_BIND", format!("127.0.0.1:{port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let reply = http(
        port,
        "GET /api/v1/accounts/nobody HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n",
    );
    assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
    assert!(
        reply.ends_with(r#"{"error":"Account not found"}"#),
        "{reply}"
    );
    let body = r#"{"username":"alice","email":"a@x.org","password":"password1"}"#;
    let reply = http(
        port,
        &format!(
            "POST /api/v1/accounts HTTP/1.1\r\nhost: x\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        ),
    );
    assert!(reply.starts_with("HTTP/1.1 201"), "{reply}");
    assert!(reply.contains(r#""role":"admin""#), "{reply}");
}

#[tokio::test]
async fn a_bad_minidon_bind_stops_startup_naming_the_variable() {
    let dir = tempfile::tempdir().unwrap();
    let path = database_with_schema(dir.path()).await;
    let output = Command::new(env!("CARGO_BIN_EXE_minidon-rust"))
        .env(
            "MINIDON_DATABASE_URL",
            format!("sqlite:///{}", path.display()),
        )
        .env("MINIDON_BIND", "not-an-address")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("MINIDON_BIND"));
}
