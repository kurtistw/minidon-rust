//! Wave 2 unit tests for `accounts::validate` (UT-ACC-001..004, UT-ACC-013, SC-ACC-001..007,
//! SC-ACC-023..025). Expected values were checked against Python 3 `str.lower()`, `str.strip()`
//! and `re.match(r"^[a-z0-9_]{1,30}$", ...)`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::accounts::validate::*;

fn ok(raw: &str) -> bool {
    username_valid(&normalise_username(raw))
}

#[test]
fn username_is_lower_cased_not_stripped() {
    assert_eq!(normalise_username("Alice_01"), "alice_01");
    assert_eq!(normalise_username(" Bob "), " bob ");
}

#[test]
fn ut_acc_001_valid_usernames() {
    for name in ["a", "alice", "Alice_01", "_", "0", "ALICE", &"a".repeat(30)] {
        assert!(ok(name), "{name:?}");
    }
}

#[test]
fn ut_acc_002_invalid_usernames() {
    let long = "a".repeat(31);
    for name in [
        "", &long, "bad-name", "has space", " alice", "alice ", "al.ice", "１２３", "alice\t",
    ] {
        assert!(!ok(name), "{name:?}");
    }
}

#[test]
fn ut_acc_003_non_ascii_folding_matches_python() {
    // Accented, CJK and dotted capital I are rejected. The Kelvin sign folds to ASCII k.
    for name in ["Ålice", "用户", "İstanbul", "ΑΣ"] {
        assert!(!ok(name), "{name:?}");
    }
    assert_eq!(normalise_username("\u{212A}elvin"), "kelvin");
    assert!(ok("\u{212A}elvin"));
    assert_eq!(normalise_username("ΑΣ"), "ας", "final sigma, as Python");
}

#[test]
fn ut_acc_004_trailing_newline_set() {
    // DELIBERATE parity copy, human decision 2026-11-13, RC-001. See validate.rs.
    assert!(ok("alice\n"));
    assert!(ok(&format!("{}\n", "a".repeat(30))));
    assert!(!ok(&format!("{}\n", "a".repeat(31))));
    for name in ["alice\n\n", "\n", "ali\nce", "alice\r\n", "\nalice", "alice\n "] {
        assert!(!ok(name), "{name:?}");
    }
}

#[test]
fn trailing_newline_name_keeps_its_newline_after_normalising() {
    assert_eq!(normalise_username("ALICE\n"), "alice\n");
}

#[test]
fn replacement_character_username_is_invalid() {
    // Wave 4 pre-parse turns a lone surrogate escape into U+FFFD for the username field
    // (adr-001 decision 6). It must fail the pattern like Python's lone surrogate does.
    assert!(!ok("ali\u{FFFD}ce"));
    assert!(!ok("\u{FFFD}"));
}

#[test]
fn username_message_is_python_text() {
    assert_eq!(
        USERNAME_MESSAGE,
        "Username must be 1-30 letters, digits or underscores"
    );
}

#[test]
fn python_strip_uses_the_python_whitespace_set() {
    assert_eq!(python_strip("\u{A0}Bob\u{A0}"), "Bob");
    assert_eq!(python_strip("\u{85}x\u{1c}\u{1d}\u{1e}\u{1f}"), "x");
    assert_eq!(python_strip("\u{3000}\t x \r\n"), "x");
    // Zero width space is not whitespace in Python either.
    assert_eq!(python_strip("\u{200b}x"), "\u{200b}x");
    assert_eq!(python_strip(""), "");
}

#[test]
fn email_is_trimmed_and_lower_cased() {
    assert_eq!(normalise_email("  Bob@Example.COM "), "bob@example.com");
    assert_eq!(normalise_email("\u{A0}ÄÖ@Ü.de\u{A0}"), "äö@ü.de");
}

#[test]
fn email_only_needs_an_at_sign() {
    assert!(email_valid("@"));
    assert!(email_valid("a@b"));
    assert!(email_valid("äö@ü.de"));
    assert!(!email_valid("not-an-email"));
    assert!(!email_valid(""));
    assert_eq!(EMAIL_MESSAGE, "Email address is not valid");
}

#[test]
fn password_length_counts_code_points() {
    assert!(!password_valid("1234567"));
    assert!(password_valid("12345678"));
    assert!(!password_valid(&"😀".repeat(7)));
    assert!(password_valid(&"😀".repeat(8)));
    // Combining marks count as code points, not graphemes.
    assert!(password_valid("e\u{301}e\u{301}e\u{301}e\u{301}"));
    assert_eq!(PASSWORD_MESSAGE, "Password must be at least 8 characters");
}

#[test]
fn display_name_limit_is_30_code_points() {
    assert!(display_name_valid(""));
    assert!(display_name_valid(&"a".repeat(30)));
    assert!(!display_name_valid(&"a".repeat(31)));
    assert!(display_name_valid(&"😀".repeat(30)));
    assert!(!display_name_valid(&"😀".repeat(31)));
    assert!(display_name_valid(&"用".repeat(30)));
    let marks = "e\u{301}".repeat(15);
    assert!(display_name_valid(&marks));
    assert!(!display_name_valid(&format!("{marks}x")));
    assert_eq!(
        DISPLAY_NAME_MESSAGE,
        "Display name must be at most 30 characters"
    );
}

#[test]
fn bio_limit_is_500_code_points() {
    assert!(bio_valid(&"a".repeat(500)));
    assert!(!bio_valid(&"a".repeat(501)));
    assert!(bio_valid(&"😀".repeat(500)));
    assert!(!bio_valid(&"😀".repeat(501)));
    assert_eq!(BIO_MESSAGE, "Bio must be at most 500 characters");
}
