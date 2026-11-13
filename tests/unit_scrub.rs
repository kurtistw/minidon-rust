//! Wave 4 unit tests for `platform::scrub`, the lone-surrogate pre-parse (adr-001 section 5
//! decision 6, plan review M-03). `serde_json` refuses a lone surrogate escape, Python accepts
//! it, so the body is scrubbed first: every lone surrogate escape becomes U+FFFD and the top
//! level fields whose final string value held one are recorded.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::platform::scrub::scrub_lone_surrogates;
use serde_json::{json, Value};

fn scrub(text: &str) -> (Value, Vec<String>) {
    let scrubbed = scrub_lone_surrogates(text);
    let value: Value = serde_json::from_str(&scrubbed.text).unwrap();
    let mut tainted = scrubbed.tainted;
    tainted.sort();
    (value, tainted)
}

#[test]
fn text_without_a_surrogate_is_returned_unchanged() {
    let text = r#"{"username":"alice","bio":"é 😀 \\ud800 plain"}"#;
    let scrubbed = scrub_lone_surrogates(text);
    assert_eq!(scrubbed.text, text);
    assert!(scrubbed.tainted.is_empty());
}

#[test]
fn a_lone_high_surrogate_becomes_u_fffd_and_marks_the_field() {
    let (value, tainted) = scrub(r#"{"password":"a\ud800b","username":"x"}"#);
    assert_eq!(value, json!({"password": "a\u{fffd}b", "username": "x"}));
    assert_eq!(tainted, ["password"]);
}

#[test]
fn a_lone_low_surrogate_counts_too() {
    let (value, tainted) = scrub(r#"{"bio":"\udc00"}"#);
    assert_eq!(value, json!({"bio": "\u{fffd}"}));
    assert_eq!(tainted, ["bio"]);
}

#[test]
fn every_lone_shape_is_replaced() {
    for body in [
        r#"{"a":"\ud800x"}"#,      // high then plain
        r#"{"a":"\ud800\ud800"}"#, // high then high
        r#"{"a":"\udc00\ud800"}"#, // reversed pair
        r#"{"a":"\ud800A"}"#,      // high then another escape
        r#"{"a":"\udfff"}"#,       // last low
        r#"{"a":"\uDBFF"}"#,       // upper case hex, last high
    ] {
        let (value, tainted) = scrub(body);
        assert!(value["a"].as_str().unwrap().contains('\u{fffd}'), "{body}");
        assert_eq!(tainted, ["a"], "{body}");
    }
}

#[test]
fn a_valid_pair_is_kept_and_not_marked() {
    let (value, tainted) = scrub(r#"{"bio":"x😀y"}"#);
    assert_eq!(value, json!({"bio": "x\u{1f600}y"}));
    assert!(tainted.is_empty());
}

#[test]
fn a_pair_next_to_a_lone_one_keeps_the_pair_and_replaces_the_lone() {
    let (value, tainted) = scrub(r#"{"bio":"😀\ud800"}"#);
    assert_eq!(value, json!({"bio": "\u{1f600}\u{fffd}"}));
    assert_eq!(tainted, ["bio"]);
}

#[test]
fn an_escaped_backslash_before_u_is_not_a_surrogate() {
    let (value, tainted) = scrub(r#"{"bio":"\\ud800"}"#);
    assert_eq!(value, json!({"bio": "\\ud800"}));
    assert!(tainted.is_empty());
}

#[test]
fn an_extra_field_is_marked_so_the_handler_can_ignore_it() {
    let (_, tainted) = scrub(r#"{"username":"a","zzz":"\ud800"}"#);
    assert_eq!(tainted, ["zzz"]);
}

#[test]
fn a_surrogate_in_a_key_marks_no_field() {
    let (value, tainted) = scrub(r#"{"\ud800":1,"username":"a"}"#);
    assert_eq!(value, json!({"\u{fffd}": 1, "username": "a"}));
    assert!(tainted.is_empty());
}

#[test]
fn an_escaped_key_is_decoded_before_it_is_recorded() {
    let (_, tainted) = scrub(r#"{"password":"\ud800"}"#);
    assert_eq!(tainted, ["password"]);
}

#[test]
fn only_a_string_directly_under_a_top_level_field_marks_it() {
    let (_, tainted) = scrub(r#"{"a":["\ud800"],"b":{"c":"\ud800"},"d":"\ud800"}"#);
    assert_eq!(tainted, ["d"]);
}

#[test]
fn the_last_duplicate_wins_as_in_python() {
    let (_, tainted) = scrub(r#"{"password":"\ud800","password":"fine"}"#);
    assert!(tainted.is_empty());
    let (_, tainted) = scrub(r#"{"password":"fine","password":"\ud800"}"#);
    assert_eq!(tainted, ["password"]);
    let (_, tainted) = scrub(r#"{"password":"\ud800","password":5}"#);
    assert!(tainted.is_empty());
}

#[test]
fn a_body_that_is_not_an_object_is_still_scrubbed() {
    let (value, tainted) = scrub(r#"["\ud800"]"#);
    assert_eq!(value, json!(["\u{fffd}"]));
    assert!(tainted.is_empty());
}

#[test]
fn broken_json_never_panics_and_stays_broken() {
    for body in [
        "",
        "{",
        r#"{"a":"\ud8"#,
        r#"{"a":"\ud8zz"}"#,
        r#"{"a":"\u"}"#,
        r#"{"a":"\"#,
        r#""\ud800"#,
        "}}}{{{",
        r#"{"a":"\ud800" "b":1}"#,
    ] {
        let scrubbed = scrub_lone_surrogates(body);
        assert!(
            serde_json::from_str::<Value>(&scrubbed.text).is_err(),
            "{body:?}"
        );
    }
}
