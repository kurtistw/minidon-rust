//! The parity normaliser (DERIVED from test-plan section 2, review M-08).
//!
//! It replaces generated ids, token values and timestamps with stable placeholders and nothing
//! else, so error bodies, key order, text and 404 and 405 bodies are compared as they are
//! (OP-06). A timestamp is first checked against Python's exact wire format, so a format change
//! is reported and not hidden: naive ISO with `T`, no zone, and the fraction left out when the
//! microseconds are zero. The placeholder keeps the shape (`<TIMESTAMP>` or
//! `<TIMESTAMP+FRACTION>`), so a body that gains or loses a fraction does not normalise equal.
#![allow(dead_code)]

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormaliseError {
    /// A `created_at` or `expires_at` value that is not Python's wire format.
    BadDate { field: String, value: String },
    /// An `access_token` that is not 43 URL-safe base64 characters.
    BadToken(String),
}

impl fmt::Display for NormaliseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDate { field, value } => {
                write!(f, "{field} {value:?} is not Python's wire date format")
            }
            Self::BadToken(value) => write!(f, "access_token {value:?} is not 43 url-safe chars"),
        }
    }
}

impl std::error::Error for NormaliseError {}

static WIRE_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{6})?$").expect("static pattern")
});
static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{43}$").expect("static pattern"));
static DATE_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(created_at|expires_at)":(?:"((?:[^"\\]|\\.)*)"|([^,}\]\s]*))"#)
        .expect("static pattern")
});
static TOKEN_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""access_token":"((?:[^"\\]|\\.)*)""#).expect("static pattern"));
static ID_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""id":-?\d+"#).expect("static pattern"));

/// Is this exactly Python's wire date? Returns whether it has a fraction.
pub fn check_wire_date(value: &str) -> Result<bool, ()> {
    let Some(caps) = WIRE_DATE.captures(value) else {
        return Err(());
    };
    match caps.get(1).map(|m| m.as_str()) {
        Some(".000000") => Err(()), // Python leaves a zero fraction out
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

/// Every `(field, value)` date in a body, each checked against the wire format.
pub fn wire_dates(body: &str) -> Result<Vec<(String, String)>, NormaliseError> {
    let mut found = Vec::new();
    for caps in DATE_FIELD.captures_iter(body) {
        let field = caps[1].to_owned();
        let value = caps
            .get(2)
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().to_owned())
            .unwrap_or_default();
        if caps.get(2).is_none() || check_wire_date(&value).is_err() {
            return Err(NormaliseError::BadDate { field, value });
        }
        found.push((field, value));
    }
    Ok(found)
}

/// Normalise a response body text.
pub fn normalise(body: &str) -> Result<String, NormaliseError> {
    // Validate first, so a bad value is an error and never a placeholder.
    let dates = wire_dates(body)?;
    for caps in TOKEN_FIELD.captures_iter(body) {
        if !TOKEN.is_match(&caps[1]) {
            return Err(NormaliseError::BadToken(caps[1].to_owned()));
        }
    }
    let mut next_date = dates.into_iter();
    let with_dates = DATE_FIELD.replace_all(body, |caps: &regex::Captures<'_>| {
        let has_fraction = next_date
            .next()
            .is_some_and(|(_, value)| matches!(check_wire_date(&value), Ok(true)));
        let placeholder = if has_fraction {
            "<TIMESTAMP+FRACTION>"
        } else {
            "<TIMESTAMP>"
        };
        format!(r#""{}":"{placeholder}""#, &caps[1])
    });
    let with_tokens = TOKEN_FIELD.replace_all(&with_dates, r#""access_token":"<TOKEN>""#);
    Ok(ID_FIELD
        .replace_all(&with_tokens, r#""id":"<ID>""#)
        .into_owned())
}
