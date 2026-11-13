//! Wave 2 unit tests for `accounts::token` (UT-ACC-010, SC-ACC-018, SC-ACC-019, REQ-011,
//! REQ-012, review m-04). Header expectations are Python `deps.py:17-22`:
//! `authorization.lower().startswith("bearer ")` then `authorization[7:].strip()`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::accounts::token::*;
use time::macros::datetime;
use time::Duration;

#[test]
fn token_shape_is_43_url_safe_chars_from_32_bytes() {
    let token = new_token();
    assert_eq!(token.len(), 43);
    assert!(token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert!(token.len() <= 64, "fits the VARCHAR(64) column");
}

#[test]
fn tokens_are_not_repeated() {
    let tokens: std::collections::HashSet<String> = (0..200).map(|_| new_token()).collect();
    assert_eq!(tokens.len(), 200);
}

#[test]
fn expiry_is_issue_time_plus_ttl_days() {
    let t0 = datetime!(2026-01-01 00:00:00);
    assert_eq!(expires_at(t0, 30), datetime!(2026-01-31 00:00:00));
    assert_eq!(expires_at(t0, 7), datetime!(2026-01-08 00:00:00));
    let with_micros = datetime!(2026-01-01 10:20:30.123456);
    assert_eq!(expires_at(with_micros, 30), datetime!(2026-01-31 10:20:30.123456));
}

#[test]
fn ut_acc_010_expiry_boundary_is_less_than_or_equal() {
    let t0 = datetime!(2026-01-01 00:00:00);
    let expiry = expires_at(t0, 30);
    let second = Duration::seconds(1);
    assert!(!is_expired(expiry, expiry - second), "T0+30d minus 1s");
    assert!(is_expired(expiry, expiry), "exactly T0+30d");
    assert!(is_expired(expiry, expiry + second), "T0+30d plus 1s");
    assert!(is_expired(expiry, expiry + Duration::microseconds(1)));
    assert!(!is_expired(expiry, expiry - Duration::microseconds(1)));
}

fn bearer(header: &str) -> Option<String> {
    parse_bearer(Some(header))
}

#[test]
fn bearer_header_cases() {
    assert_eq!(bearer("Bearer abc"), Some("abc".into()));
    assert_eq!(bearer("bearer abc"), Some("abc".into()));
    assert_eq!(bearer("bEaReR abc"), Some("abc".into()));
    assert_eq!(bearer("BEARER abc"), Some("abc".into()));
    // Token is stripped with the Python whitespace set.
    assert_eq!(bearer("Bearer   abc  "), Some("abc".into()));
    assert_eq!(bearer("Bearer \tabc\t"), Some("abc".into()));
    assert_eq!(bearer("Bearer \u{1c}abc\u{1f}"), Some("abc".into()));
    // Only the prefix needs a single space after it.
    assert_eq!(bearer("Bearer a b"), Some("a b".into()));
}

#[test]
fn bearer_header_means_anonymous() {
    assert_eq!(parse_bearer(None), None);
    assert_eq!(bearer(""), None);
    assert_eq!(bearer("Bearer"), None, "no space after the prefix");
    assert_eq!(bearer("Bearer\tabc"), None, "tab instead of the space");
    assert_eq!(bearer("Basic abc"), None);
    assert_eq!(bearer("Token abc"), None);
    assert_eq!(bearer(" Bearer abc"), None, "leading space");
    assert_eq!(bearer("Bearer\u{a0}abc"), None);
    assert_eq!(bearer("Bearerabc"), None);
}

#[test]
fn bearer_with_only_whitespace_gives_an_empty_token_that_matches_nothing() {
    // Python looks up the empty string and finds nothing. An empty token is the same as none.
    assert_eq!(bearer("Bearer "), None);
    assert_eq!(bearer("Bearer    "), None);
}

#[test]
fn latin_one_header_bytes_are_decoded_like_starlette() {
    // Starlette decodes header bytes as latin-1, so 0xA0 is U+00A0 and 0x85 is U+0085. Python
    // `strip()` removes both, so `Bearer <token><0xA0>` authenticates in Python.
    let text = latin1_to_string(b"Bearer tok\xa0");
    assert_eq!(text, "Bearer tok\u{a0}");
    assert_eq!(parse_bearer(Some(&text)), Some("tok".into()));
    let text = latin1_to_string(b"Bearer \x85tok\x85");
    assert_eq!(parse_bearer(Some(&text)), Some("tok".into()));
    // A non-whitespace latin-1 byte stays part of the token, which then matches nothing.
    let text = latin1_to_string(b"Bearer to\xe9k");
    assert_eq!(parse_bearer(Some(&text)), Some("to\u{e9}k".into()));
}
