//! Access tokens and the bearer header (BR-010, BR-011, DERIVED from `accounts/service.py:72-88`
//! and `deps.py:17-22`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::rngs::OsRng;
use rand::RngCore;
use time::Duration;

use super::validate::python_strip;
use crate::platform::clock::NaiveUtc;

const TOKEN_BYTES: usize = 32;
const BEARER_PREFIX: &str = "bearer ";

/// 32 OS-random bytes as URL-safe base64 without padding: 43 characters, like Python's
/// `secrets.token_urlsafe(32)`. Stored in clear text as the primary key (copied, adr-001).
pub fn new_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Issue time plus the ttl in days (naive UTC, microseconds kept).
pub fn expires_at(now: NaiveUtc, ttl_days: i64) -> NaiveUtc {
    now.saturating_add(Duration::days(ttl_days))
}

/// Expired when `expires_at <= now`: an expiry exactly equal to now is already expired.
pub fn is_expired(expires_at: NaiveUtc, now: NaiveUtc) -> bool {
    expires_at <= now
}

/// Starlette decodes header bytes as latin-1. This does the same, so Python's `strip()` sees the
/// same characters (U+00A0 and U+0085 are whitespace there).
pub fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// The token in an `Authorization` header, or `None` for anonymous. Never an error: a header
/// that does not parse is a signed-out caller, not a 400 or 500.
///
/// Python: the lower-cased header must start with `bearer ` (one space), the rest is stripped
/// with `str.strip()`. An empty token matches no row, so it is the same as none. Only ASCII
/// can lower-case into the ASCII prefix, so an ASCII case-insensitive compare is equivalent.
pub fn parse_bearer(header: Option<&str>) -> Option<String> {
    let header = header?;
    let prefix = header.get(..BEARER_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(BEARER_PREFIX) {
        return None;
    }
    let token = python_strip(&header[BEARER_PREFIX.len()..]);
    (!token.is_empty()).then(|| token.to_owned())
}
