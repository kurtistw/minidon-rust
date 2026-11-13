//! Settings. Same env vars and defaults as `minidon/config.py` (Python, e1fbd3a).

use std::fmt;
use std::path::PathBuf;

/// Largest request body accepted (adr-001 section 5 decision 2). Python has no limit.
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Same dev address as Python (`python -m minidon` binds 127.0.0.1:8000, hard coded there).
pub const DEFAULT_BIND: &str = "127.0.0.1:8000";

const DEFAULT_DATABASE_URL: &str = "sqlite:///./minidon.db";
const DEFAULT_DOMAIN: &str = "minidon.local";
const DEFAULT_TOKEN_TTL_DAYS: i64 = 30;

/// Only the settings Accounts needs. Later stories add theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// SQLAlchemy style URL, as Python reads it (`MINIDON_DATABASE_URL`).
    pub database_url: String,
    /// `MINIDON_DOMAIN`.
    pub instance_domain: String,
    /// Access token lifetime. Python has no env var for it, so neither has Rust (review m-15):
    /// tests override it in code.
    pub token_ttl_days: i64,
}

impl Settings {
    /// Read settings through a lookup function, so tests need no process environment.
    pub fn from_lookup<F>(lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        Self {
            database_url: lookup("MINIDON_DATABASE_URL")
                .unwrap_or_else(|| DEFAULT_DATABASE_URL.to_owned()),
            instance_domain: lookup("MINIDON_DOMAIN").unwrap_or_else(|| DEFAULT_DOMAIN.to_owned()),
            token_ttl_days: DEFAULT_TOKEN_TTL_DAYS,
        }
    }

    /// Read settings from the process environment.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }
}

/// Where the SQLite database lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseLocation {
    Memory,
    File(PathBuf),
}

/// A `MINIDON_DATABASE_URL` that Rust cannot open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

fn unsupported(url: &str, why: &str) -> ConfigError {
    ConfigError(format!(
        "MINIDON_DATABASE_URL {url:?} is not supported: {why}. \
         Use sqlite:///relative/path.db or sqlite:////absolute/path.db"
    ))
}

/// Translate the SQLAlchemy SQLite URL Python uses into a file location (review M-09).
///
/// `sqlite:///./minidon.db` is the relative path `./minidon.db` and `sqlite:////tmp/m.db` is the
/// absolute path `/tmp/m.db`, so both builds open the same file. Handing the URL to sqlx as it is
/// would read the path differently.
pub fn parse_database_url(url: &str) -> Result<DatabaseLocation, ConfigError> {
    let Some(rest) = url.strip_prefix("sqlite://") else {
        return Err(unsupported(
            url,
            "only the plain `sqlite` scheme is handled",
        ));
    };
    if rest.is_empty() {
        return Ok(DatabaseLocation::Memory);
    }
    let Some(path) = rest.strip_prefix('/') else {
        return Err(unsupported(url, "a host part is not allowed"));
    };
    if path.contains('?') {
        return Err(unsupported(url, "query options are not handled"));
    }
    if path.is_empty() {
        return Err(unsupported(url, "the file path is empty"));
    }
    if path == ":memory:" {
        return Ok(DatabaseLocation::Memory);
    }
    Ok(DatabaseLocation::File(PathBuf::from(path)))
}

/// The address to listen on: `MINIDON_BIND`, default [`DEFAULT_BIND`]. Python has no such
/// variable (its address is fixed), so this only adds a way to run two builds side by side, as
/// the parity harness does. The value must be `ip:port`.
pub fn bind_address_from<F>(lookup: F) -> Result<String, ConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = lookup("MINIDON_BIND").unwrap_or_else(|| DEFAULT_BIND.to_owned());
    match value.parse::<std::net::SocketAddr>() {
        Ok(_) => Ok(value),
        Err(_) => Err(ConfigError(format!(
            "MINIDON_BIND {value:?} is not an address: use ip:port, for example 127.0.0.1:8000"
        ))),
    }
}
