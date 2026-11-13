//! Compatibility harness (wave 6, REQ-016, REQ-017, OP-05, OP-09): the Python oracle and the Rust
//! build serve ONE database file at the same time. Whatever one build writes the other must read,
//! with no migration. The `Pair` of waves 4 and 5 uses two copies and cannot show that.
//!
//! The file is made by Python (`create_tables`), Rust never runs DDL. Requests are sent one at a
//! time, so the two builds never write at once (SQLite locks the file between requests anyway).
//! The clock is moved on both builds together.
//!
//! `COMPAT_PYTHON_ONLY=1` is the test-first check: every request meant for Rust goes to the oracle
//! instead, so each expectation is proven on Python alone before Rust takes part.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use minidon_rust::platform::clock::{format_wire, NaiveUtc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode};
use sqlx::{Connection, Row};

use super::{
    compare_with, oracle, send_raw, tmp_dir, Observed, PythonServer, Req, RustServer, Side, Tokens,
};

pub struct SharedPair {
    pub py: PythonServer,
    pub rs: RustServer,
    pub db: PathBuf,
    binary: bool,
    python_only: bool,
    _dir: tempfile::TempDir,
}

impl SharedPair {
    /// Both builds on one Python-made empty database. Rust is the real router, in process, on a
    /// fixed clock.
    pub async fn new() -> Self {
        Self::build(false).await
    }

    /// Rust is the real binary on the system clock (Python is frozen at the real start time, so
    /// dates are only checked for format). Used for the smoke test.
    pub async fn new_with_binary() -> Self {
        Self::build(true).await
    }

    async fn build(binary: bool) -> Self {
        let oracle = oracle();
        let dir = tempfile::tempdir_in(tmp_dir()).expect("temp dir");
        let db = dir.path().join("shared.db");
        std::fs::copy(&oracle.template_db, &db).expect("copy the Python-made database");
        let py = PythonServer::start(&db, &dir.path().join("python.log"));
        let rs = if binary {
            RustServer::start_binary(&db)
        } else {
            RustServer::start(&db).await
        };
        let pair = Self {
            py,
            rs,
            db,
            binary,
            python_only: std::env::var_os("COMPAT_PYTHON_ONLY").is_some(),
            _dir: dir,
        };
        // The binary reads the system clock, so Python is frozen at the real time of the start:
        // a token from either build must be valid for the other.
        let start = if binary {
            let now = time::OffsetDateTime::now_utc();
            time::PrimitiveDateTime::new(now.date(), now.time())
        } else {
            super::T0
        };
        pair.set_clock(start).await;
        pair
    }

    pub fn python_only(&self) -> bool {
        self.python_only
    }

    pub fn db_dir(&self) -> &Path {
        self.db.parent().expect("db dir")
    }

    /// Move both clocks (Python through the wrapper's control route).
    pub async fn set_clock(&self, at: NaiveUtc) {
        if !self.binary {
            self.rs.clock.set(at);
        }
        let body = format!(r#"{{"at":"{}"}}"#, format_wire(at));
        let request = Req::post_json("/__clock", &body).bytes(Side::Python, self.py.port);
        let answer = send_raw(self.py.port, request, false).await;
        assert_eq!(answer.status, 200, "set clock: {}", answer.body());
    }

    fn port(&self, side: Side) -> u16 {
        match side {
            Side::Rust if !self.python_only => self.rs.port,
            _ => self.py.port,
        }
    }

    /// Send to one build. A token is the value that build issued, so it is used as is.
    pub async fn send(&self, side: Side, req: &Req) -> Observed {
        let port = self.port(side);
        send_raw(port, req.bytes(side, port), false).await
    }

    pub async fn register(&self, side: Side, user: &str, email: &str, password: &str) -> Observed {
        let answer = self.send(side, &Req::register(user, email, password)).await;
        assert_eq!(
            answer.status,
            201,
            "{side:?} register {user}: {}",
            answer.describe_for_test()
        );
        answer
    }

    pub async fn sign_in(&self, side: Side, user: &str, password: &str) -> Observed {
        self.send(side, &Req::sign_in(user, password)).await
    }

    /// Sign in (must be 200) and return the token.
    pub async fn token(&self, side: Side, user: &str, password: &str) -> String {
        let answer = self.sign_in(side, user, password).await;
        assert_eq!(
            answer.status,
            200,
            "{side:?} sign in {user}: {}",
            answer.describe_for_test()
        );
        answer.json()["access_token"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    fn tokens(token: &str) -> Tokens {
        Tokens {
            py: token.to_owned(),
            rs: token.to_owned(),
        }
    }

    /// `GET /accounts/me` on one build with a token issued by either build.
    pub async fn me(&self, side: Side, token: &str) -> Observed {
        self.send(side, &Req::me(&Self::tokens(token))).await
    }

    pub async fn patch_me(&self, side: Side, token: &str, body: &str) -> Observed {
        self.send(side, &Req::patch_me(&Self::tokens(token), body))
            .await
    }

    pub async fn lookup(&self, side: Side, user: &str) -> Observed {
        self.send(side, &Req::lookup(user)).await
    }

    /// The same request to both builds on the shared file: the answers must be the same after
    /// the normaliser (ids, tokens and timestamps replaced, dates also compared by value when
    /// both clocks are fixed). Only for requests that do not write.
    pub async fn same(&self, req: &Req) -> (Observed, Observed) {
        let py = self.send(Side::Python, req).await;
        let rs = self.send(Side::Rust, req).await;
        if !self.python_only {
            if let Err(why) = compare_with(&py, &rs, self.py.port, self.rs.port, !self.binary) {
                panic!(
                    "COMPAT FAILURE\n  request: {}\n  python: {}\n  rust:   {}\n  why: {why}",
                    req.label,
                    py.describe_for_test(),
                    rs.describe_for_test()
                );
            }
        }
        (py, rs)
    }

    // ---- the file itself ----

    async fn connect(&self, read_only: bool) -> SqliteConnection {
        // sqlx defaults to WAL, which would change the file for Python: keep the rollback journal.
        let options = SqliteConnectOptions::new()
            .filename(&self.db)
            .foreign_keys(false)
            .journal_mode(SqliteJournalMode::Delete)
            .read_only(read_only);
        SqliteConnection::connect_with(&options)
            .await
            .expect("open the shared database")
    }

    /// Seed rows with DML (never DDL) while both servers are idle.
    pub async fn exec(&self, statement: &str) {
        let mut conn = self.connect(false).await;
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("seed {statement}: {e}"));
        conn.close().await.expect("close");
    }

    /// Rows as text (`CAST` every column in the statement). NULL shows as `None`.
    pub async fn rows(&self, statement: &str) -> Vec<Vec<Option<String>>> {
        let mut conn = self.connect(true).await;
        let rows = sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|e| panic!("read {statement}: {e}"));
        let out = rows
            .iter()
            .map(|row| {
                (0..row.len())
                    .map(|i| row.try_get::<Option<String>, _>(i).expect("text column"))
                    .collect()
            })
            .collect();
        conn.close().await.expect("close");
        out
    }

    pub async fn scalar(&self, statement: &str) -> Option<String> {
        self.rows(statement)
            .await
            .into_iter()
            .next()
            .and_then(|row| row.into_iter().next())
            .flatten()
    }

    /// Everything DDL can change: every object of `sqlite_master` with its SQL text, the pragmas
    /// `schema_version` (bumped by any DDL), `user_version` and `journal_mode`.
    pub async fn schema_fingerprint(&self) -> String {
        let objects = self
            .rows(
                "SELECT type || '|' || name || '|' || tbl_name || '|' || COALESCE(sql, '') \
                 FROM sqlite_master ORDER BY type, name",
            )
            .await;
        let mut lines: Vec<String> = objects
            .into_iter()
            .map(|r| r[0].clone().unwrap_or_default())
            .collect();
        for pragma in ["schema_version", "user_version", "journal_mode"] {
            let value = self
                .scalar(&format!("SELECT CAST(({}) AS TEXT)", pragma_select(pragma)))
                .await;
            lines.push(format!("{pragma}={}", value.unwrap_or_default()));
        }
        lines.join("\n")
    }

    /// Names of every file next to the database (a `-wal`, `-shm` or `-journal` left behind
    /// would mean a changed journal mode).
    pub fn files_next_to_db(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.db_dir())
            .expect("read db dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("shared.db"))
            .collect();
        names.sort();
        names
    }
}

fn pragma_select(pragma: &str) -> String {
    format!("SELECT {pragma} FROM pragma_{pragma}")
}
