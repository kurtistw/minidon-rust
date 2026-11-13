//! Wave 4 unit tests for `platform::path`: how Python (uvicorn and Starlette) read and rewrite
//! request paths. Expected values come from running `urllib.parse.unquote` and `quote` and from
//! the Starlette 1.10 source in the oracle (`routing.py` redirect_slashes, `datastructures.py`
//! URL, `responses.py` RedirectResponse).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::platform::path::{
    canonical_path, decode_path, location_for, pattern_matches, toggle_slash,
};

#[test]
fn decode_follows_python_unquote_with_replacement() {
    for (raw, decoded) in [
        ("/a%20b%0A%E7%94%A8", "/a b\n用"),
        ("%FF", "\u{fffd}"),
        ("%C3", "\u{fffd}"),
        ("%E7%94", "\u{fffd}"),
        ("%E7a%94", "\u{fffd}a\u{fffd}"),
        ("a+b", "a+b"),
        ("%2F%2f", "//"),
        ("%zz%4", "%zz%4"),
        ("%41%", "A%"),
        ("%u0041", "%u0041"),
        ("%00", "\0"),
        ("/plain/path", "/plain/path"),
        ("", ""),
    ] {
        assert_eq!(decode_path(raw), decoded, "{raw}");
    }
}

#[test]
fn canonical_path_re_encodes_everything_but_unreserved_and_slash() {
    assert_eq!(canonical_path("/a b\n用"), "/a%20b%0A%E7%94%A8");
    assert_eq!(canonical_path("/Az09-._~/x"), "/Az09-._~/x");
    assert_eq!(canonical_path("a+b%c?d#e\\f"), "a%2Bb%25c%3Fd%23e%5Cf");
    assert_eq!(canonical_path("\u{fffd}"), "%EF%BF%BD");
}

#[test]
fn decoding_then_canonical_is_stable() {
    for raw in [
        "/api/v1/accounts/alice%0A",
        "/api/v1%2Faccounts/x",
        "/%FF/%41",
    ] {
        let once = canonical_path(&decode_path(raw));
        assert_eq!(canonical_path(&decode_path(&once)), once, "{raw}");
    }
}

#[test]
fn toggle_slash_is_starlette_redirect_slashes() {
    assert_eq!(toggle_slash("/a/"), Some("/a".to_owned()));
    assert_eq!(toggle_slash("/a//"), Some("/a".to_owned()));
    assert_eq!(toggle_slash("/a"), Some("/a/".to_owned()));
    assert_eq!(toggle_slash("/"), None);
    assert_eq!(toggle_slash("//"), Some(String::new()));
}

#[test]
fn pattern_matching_is_segment_wise_with_non_empty_parameters() {
    assert!(pattern_matches("/api/v1/accounts", "/api/v1/accounts"));
    assert!(!pattern_matches("/api/v1/accounts", "/api/v1/accounts/"));
    assert!(pattern_matches(
        "/api/v1/accounts/{username}",
        "/api/v1/accounts/alice"
    ));
    assert!(pattern_matches(
        "/api/v1/accounts/{username}",
        "/api/v1/accounts/a\nb"
    ));
    assert!(!pattern_matches(
        "/api/v1/accounts/{username}",
        "/api/v1/accounts/"
    ));
    assert!(!pattern_matches(
        "/api/v1/accounts/{username}",
        "/api/v1/accounts/a/b"
    ));
    assert!(!pattern_matches(
        "/api/v1/accounts/me",
        "/api/v1/accounts/ME"
    ));
    assert!(!pattern_matches("/a", ""));
}

#[test]
fn location_is_the_decoded_path_quoted_as_starlette_does() {
    assert_eq!(
        location_for("h:1", "/api/v1/accounts/alice\n", None),
        "http://h:1/api/v1/accounts/alice%0A"
    );
    assert_eq!(
        location_for("h", "/a b/用", Some("x=1&y=%20")),
        "http://h/a%20b/%E7%94%A8?x=1&y=%20"
    );
    // ? # and backslash in the decoded path are escaped first, a percent sign is left alone.
    assert_eq!(
        location_for("h", "/a?b#c\\d%x", None),
        "http://h/a%3Fb%23c%5Cd%x"
    );
    assert_eq!(location_for("h", "/x\u{fffd}", None), "http://h/x%EF%BF%BD");
    assert_eq!(
        location_for("h", "/q", Some("a b\"")),
        "http://h/q?a%20b%22"
    );
}
