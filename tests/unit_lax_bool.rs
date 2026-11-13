//! Wave 4 unit tests for the `locked` field of `ProfileIn` (review follow-up in wave-2-3.md,
//! OP-06). Pydantic's lax bool is what Python answers, checked against the Python oracle:
//! `true false`, the integers 0 and 1, the floats 0.0 and 1.0, and the strings 0 off f false n no
//! and 1 on t true y yes in any letter case. Anything else is a 422. `null` and a missing field
//! both mean leave alone. Strings are strict (a number is not a string).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::accounts::model::ProfileIn;
use serde_json::{json, Value};

fn locked(value: Value) -> Result<Option<bool>, String> {
    serde_json::from_value::<ProfileIn>(json!({ "locked": value }))
        .map(|p| p.locked)
        .map_err(|e| e.to_string())
}

#[test]
fn bools_and_null() {
    assert_eq!(locked(json!(true)), Ok(Some(true)));
    assert_eq!(locked(json!(false)), Ok(Some(false)));
    assert_eq!(locked(Value::Null), Ok(None));
    let none: ProfileIn = serde_json::from_value(json!({})).unwrap();
    assert_eq!(none.locked, None);
}

#[test]
fn numbers_zero_and_one_only() {
    assert_eq!(locked(json!(1)), Ok(Some(true)));
    assert_eq!(locked(json!(0)), Ok(Some(false)));
    assert_eq!(locked(json!(1.0)), Ok(Some(true)));
    assert_eq!(locked(json!(0.0)), Ok(Some(false)));
    for bad in [json!(2), json!(-1), json!(0.5), json!(1.5), json!(1e30), json!(u64::MAX)] {
        assert!(locked(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn strings_follow_the_pydantic_table_in_any_case() {
    for yes in ["1", "on", "t", "true", "y", "yes", "TRUE", "True", "YES", "On", "T", "Y"] {
        assert_eq!(locked(json!(yes)), Ok(Some(true)), "{yes}");
    }
    for no in ["0", "off", "f", "false", "n", "no", "FALSE", "False", "NO", "Off"] {
        assert_eq!(locked(json!(no)), Ok(Some(false)), "{no}");
    }
    for bad in ["", " true ", "true ", "2", "maybe", "tru", "truee", "\u{ff34}rue"] {
        assert!(locked(json!(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn containers_are_refused() {
    for bad in [json!([]), json!({}), json!([true])] {
        assert!(locked(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn text_fields_stay_strict() {
    for body in [
        json!({"display_name": 5}),
        json!({"bio": true}),
        json!({"bio": ["a"]}),
        json!({"display_name": 1.5}),
    ] {
        assert!(serde_json::from_value::<ProfileIn>(body.clone()).is_err(), "{body}");
    }
    let ok: ProfileIn = serde_json::from_value(json!({"display_name": null, "bio": "b"})).unwrap();
    assert_eq!(ok.display_name, None);
    assert_eq!(ok.bio.as_deref(), Some("b"));
}
