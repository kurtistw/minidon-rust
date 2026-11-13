//! Wave 5 unit tests for the parity normaliser (REQ-018, review M-08, test-plan section 2).
//! It must replace generated ids, token values and timestamps and nothing else, and it must
//! report a date whose format is not Python's exact wire format instead of hiding it.
//! Python's wire date is naive ISO with `T`, with the fraction left out when the microseconds
//! are zero (confirmed on the oracle: `2026-01-01T00:00:00`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "parity/normalise.rs"]
mod normalise;

use normalise::{check_wire_date, normalise, NormaliseError};

const ACCOUNT: &str = r#"{"id":7,"username":"alice","acct":"alice@minidon.local","display_name":"","bio":"","locked":false,"role":"admin","created_at":"2026-01-01T00:00:00","followers_count":0,"following_count":0,"statuses_count":0}"#;

#[test]
fn ids_and_timestamps_are_replaced_and_the_rest_is_untouched() {
    let out = normalise(ACCOUNT).unwrap();
    assert_eq!(
        out,
        r#"{"id":"<ID>","username":"alice","acct":"alice@minidon.local","display_name":"","bio":"","locked":false,"role":"admin","created_at":"<TIMESTAMP>","followers_count":0,"following_count":0,"statuses_count":0}"#
    );
}

#[test]
fn tokens_are_replaced_after_a_shape_check() {
    let token = "a".repeat(43);
    let body = format!(
        r#"{{"access_token":"{token}","token_type":"bearer","expires_at":"2026-01-31T00:00:00.250000"}}"#
    );
    assert_eq!(
        normalise(&body).unwrap(),
        r#"{"access_token":"<TOKEN>","token_type":"bearer","expires_at":"<TIMESTAMP+FRACTION>"}"#
    );
    for bad in [
        "",
        "short",
        &"a".repeat(42),
        &"a".repeat(44),
        &format!("{}=", "a".repeat(42)),
        &format!("{}+", "a".repeat(42)),
    ] {
        let body = format!(r#"{{"access_token":"{bad}"}}"#);
        assert!(
            matches!(normalise(&body), Err(NormaliseError::BadToken(_))),
            "{bad}"
        );
    }
}

#[test]
fn a_date_with_and_without_fraction_normalise_differently() {
    // A formatter that starts printing `.000000` must not be hidden: that is a different value
    // shape, and `.000000` itself is refused.
    let plain = normalise(r#"{"created_at":"2026-01-01T00:00:00"}"#).unwrap();
    let frac = normalise(r#"{"created_at":"2026-01-01T00:00:00.100000"}"#).unwrap();
    assert_ne!(plain, frac);
}

#[test]
fn only_the_exact_python_wire_format_is_accepted() {
    for good in [
        "2026-01-01T00:00:00",
        "2026-12-31T23:59:59.000001",
        "2026-01-01T00:00:00.500000",
        "1999-02-03T04:05:06.999999",
    ] {
        assert!(check_wire_date(good).is_ok(), "{good}");
    }
    for bad in [
        "2026-01-01T00:00:00.000000", // Python leaves a zero fraction out
        "2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00+00:00",
        "2026-01-01 00:00:00",
        "2026-01-01 00:00:00.000001",
        "2026-01-01T00:00:00.5",
        "2026-01-01T00:00:00.500",
        "2026-01-01T00:00:00.5000000",
        "2026-01-01T00:00",
        "2026-01-01",
        "2026-1-1T00:00:00",
        " 2026-01-01T00:00:00",
        "2026-01-01T00:00:00 ",
        "",
    ] {
        assert!(check_wire_date(bad).is_err(), "{bad:?}");
        let body = format!(r#"{{"created_at":"{bad}"}}"#);
        assert!(
            matches!(normalise(&body), Err(NormaliseError::BadDate { .. })),
            "{bad:?}"
        );
        let body = format!(r#"{{"expires_at":"{bad}"}}"#);
        assert!(
            matches!(normalise(&body), Err(NormaliseError::BadDate { .. })),
            "{bad:?}"
        );
    }
}

#[test]
fn error_bodies_and_other_numbers_are_left_alone() {
    for body in [
        r#"{"error":"Username is taken"}"#,
        r#"{"detail":"Not Found"}"#,
        r#"{"account_id":7,"count":3,"ids":[1,2]}"#,
        r#"{"error":"the \"id\":5 stays"}"#,
        "",
        "Internal Server Error",
    ] {
        assert_eq!(normalise(body).unwrap(), body);
    }
}

#[test]
fn every_occurrence_is_replaced_and_key_order_is_kept() {
    let body = r#"[{"id":1,"created_at":"2026-01-01T00:00:00"},{"id":22,"created_at":"2026-01-01T00:00:00.000100"}]"#;
    assert_eq!(
        normalise(body).unwrap(),
        r#"[{"id":"<ID>","created_at":"<TIMESTAMP>"},{"id":"<ID>","created_at":"<TIMESTAMP+FRACTION>"}]"#
    );
}

#[test]
fn a_non_string_date_is_not_hidden() {
    assert!(matches!(
        normalise(r#"{"created_at":1767225600}"#),
        Err(NormaliseError::BadDate { .. })
    ));
    assert!(matches!(
        normalise(r#"{"created_at":null}"#),
        Err(NormaliseError::BadDate { .. })
    ));
}
