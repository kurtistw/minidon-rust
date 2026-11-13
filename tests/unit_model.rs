//! Wave 2 unit tests for `accounts::model` (REQ-007, REQ-015, REQ-016, REQ-017, review m-02).
//!
//! Stored enum names are the upper-case member NAMES (`USER`, `MODERATOR`, `ADMIN`), the wire
//! values are lower-case (Python `StrEnum` values).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::accounts::model::{Account, AccountOut, ProfileCounts, Role};
use time::macros::datetime;

#[test]
fn role_stored_names_are_upper_case_member_names() {
    assert_eq!(Role::User.stored_name(), "USER");
    assert_eq!(Role::Moderator.stored_name(), "MODERATOR");
    assert_eq!(Role::Admin.stored_name(), "ADMIN");
}

#[test]
fn role_wire_names_are_lower_case_values() {
    assert_eq!(Role::User.wire_name(), "user");
    assert_eq!(Role::Moderator.wire_name(), "moderator");
    assert_eq!(Role::Admin.wire_name(), "admin");
}

#[test]
fn role_round_trips_through_the_stored_name_for_all_three() {
    for role in [Role::User, Role::Moderator, Role::Admin] {
        assert_eq!(Role::from_stored(role.stored_name()), Some(role));
    }
}

#[test]
fn role_rejects_anything_that_python_would_not_read() {
    // SQLAlchemy reads the member name only: lower-case values and junk are an error there.
    for bad in ["", "user", "Admin", "ADMIN ", "OWNER", "ADMIN\n"] {
        assert_eq!(Role::from_stored(bad), None, "{bad:?}");
    }
}

#[test]
fn is_staff_is_moderator_and_admin() {
    assert!(!Role::User.is_staff());
    assert!(Role::Moderator.is_staff());
    assert!(Role::Admin.is_staff());
}

fn sample() -> Account {
    Account {
        id: 7,
        username: "alice".into(),
        email: "a@x.org".into(),
        password_hash: "h".into(),
        display_name: "Al".into(),
        bio: "hi".into(),
        locked: true,
        role: Role::Moderator,
        suspended_at: None,
        created_at: datetime!(2026-01-01 00:00:00),
    }
}

#[test]
fn profile_json_has_python_keys_in_python_order() {
    let out = AccountOut::new(
        &sample(),
        "minidon.local",
        ProfileCounts {
            followers: 2,
            following: 3,
            statuses: 4,
        },
    );
    let text = serde_json::to_string(&out).unwrap();
    assert_eq!(
        text,
        concat!(
            r#"{"id":7,"username":"alice","acct":"alice@minidon.local","display_name":"Al","#,
            r#""bio":"hi","locked":true,"role":"moderator","#,
            r#""created_at":"2026-01-01T00:00:00","#,
            r#""followers_count":2,"following_count":3,"statuses_count":4}"#
        )
    );
}

#[test]
fn profile_never_contains_secrets() {
    let out = AccountOut::new(&sample(), "d", ProfileCounts::default());
    let text = serde_json::to_string(&out).unwrap();
    assert!(!text.contains("password"));
    assert!(!text.contains("email"));
    assert!(!text.contains("suspended"));
}

#[test]
fn profile_date_keeps_microseconds_when_present() {
    let mut account = sample();
    account.created_at = datetime!(2026-01-01 00:00:00.5);
    let out = AccountOut::new(&account, "d", ProfileCounts::default());
    assert_eq!(out.created_at, "2026-01-01T00:00:00.500000");
}
