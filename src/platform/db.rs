//! Database access (adr-001 D2, OP-05).
//!
//! The schema is owned by the Python app. This module opens the file Python made, checks that
//! the tables Accounts needs exist, and never runs DDL. It does not create the file either.
//! Connection settings follow a default SQLAlchemy and pysqlite connection: rollback journal
//! (not WAL, which would change the file for Python) and foreign keys not enforced.

use std::fmt;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::Row;

use super::config::{parse_database_url, DatabaseLocation, Settings};

/// Tables story 1 needs. Later stories add the tables they use.
const REQUIRED_TABLES: [&str; 2] = ["accounts", "access_tokens"];

const MAX_CONNECTIONS: u32 = 5;

/// A startup problem with the database. The message is meant for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbError(String);

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DbError {}

fn options_for(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .journal_mode(SqliteJournalMode::Delete)
        .foreign_keys(false)
}

/// Open the database named by `MINIDON_DATABASE_URL`. Does not create it.
pub async fn connect(settings: &Settings) -> Result<SqlitePool, DbError> {
    let location =
        parse_database_url(&settings.database_url).map_err(|e| DbError(e.to_string()))?;
    let (options, max_connections) = match &location {
        DatabaseLocation::File(path) => (options_for(path), MAX_CONNECTIONS),
        // Every in-memory connection is its own database, so one connection only.
        DatabaseLocation::Memory => (
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(false),
            1,
        ),
    };
    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .map_err(|e| {
            let target = match &location {
                DatabaseLocation::File(path) => path.display().to_string(),
                DatabaseLocation::Memory => ":memory:".to_owned(),
            };
            // Only the sqlx error kind text is kept, it carries no request data.
            DbError(format!("cannot open database {target}: {e}"))
        })
}

/// Fail with a clear message when a required table is missing. Never creates anything.
pub async fn verify_schema(pool: &SqlitePool) -> Result<(), DbError> {
    let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(pool)
        .await
        .map_err(|e| DbError(format!("cannot read the database schema: {e}")))?;
    let present: Vec<String> = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let missing: Vec<&str> = REQUIRED_TABLES
        .iter()
        .copied()
        .filter(|table| !present.iter().any(|name| name == table))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(DbError(format!(
        "database schema is missing tables: {}. minidon-rust never creates the schema: \
         start the Python app once (python -m minidon) to create it",
        missing.join(", ")
    )))
}

/// `connect` then `verify_schema`.
pub async fn connect_and_verify(settings: &Settings) -> Result<SqlitePool, DbError> {
    let pool = connect(settings).await?;
    verify_schema(&pool).await?;
    Ok(pool)
}
