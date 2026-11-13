//! Wave 2 unit tests for `accounts::password` (UT-ACC-007, SC-ACC-013, SC-ACC-014, REQ-008).
//!
//! Known-answer digests were generated once with Python 3
//! `hashlib.pbkdf2_hmac("sha256", pw.encode(), bytes.fromhex(salt), 100000)`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use minidon_rust::accounts::password::*;

const KATS: [(&str, &str, &str); 3] = [
    (
        "00000000000000000000000000000000",
        "password123",
        "a4708b812eae503fb81bb8a4a7b2759b80ed1c4b9e04bb75cb9087f192b9345f",
    ),
    (
        "0123456789abcdef0123456789abcdef",
        "pässwörd-密码-😀",
        "2dfc698cc10d7eb3f2d16c4e4f238ab4e059c676d0d1b60e9568c6b3ffee5f6a",
    ),
    (
        "ffffffffffffffffffffffffffffffff",
        "        ",
        "1691d09864cf6449d6612fc6f97417d9175122c3164bf628cf8d4cb9e622f421",
    ),
];

fn salt_bytes(hex_salt: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&hex::decode(hex_salt).unwrap());
    out
}

#[test]
fn sc_acc_013_known_answers_match_python() {
    for (salt, password, digest) in KATS {
        let stored = hash_password_with_salt(password, &salt_bytes(salt));
        assert_eq!(stored, format!("{salt}${digest}"));
    }
}

#[test]
fn python_made_hashes_verify() {
    for (salt, password, digest) in KATS {
        let stored = format!("{salt}${digest}");
        assert_eq!(verify_password(password, &stored), Ok(true));
        assert_eq!(verify_password("wrong-password", &stored), Ok(false));
    }
}

#[test]
fn rust_made_hashes_verify() {
    let stored = hash_password("correct horse");
    assert_eq!(verify_password("correct horse", &stored), Ok(true));
    assert_eq!(verify_password("correct horse ", &stored), Ok(false));
    assert_eq!(verify_password("", &stored), Ok(false));
}

#[test]
fn sc_acc_014_salts_differ_and_shape_is_32_dollar_64() {
    let a = hash_password("same password");
    let b = hash_password("same password");
    assert_ne!(a, b);
    for stored in [&a, &b] {
        let (salt, digest) = stored.split_once('$').unwrap();
        assert_eq!(salt.len(), 32);
        assert_eq!(digest.len(), 64);
        assert!(stored
            .chars()
            .all(|c| c == '$' || c.is_ascii_digit() || ('a'..='f').contains(&c)));
        assert_eq!(hex::decode(salt).unwrap().len(), 16);
    }
    assert!(!a.contains("same password"));
}

#[test]
fn ut_acc_007_tampered_hash_rows_never_panic() {
    let good = hash_password("pw-12345678");
    let (salt, digest) = good.split_once('$').unwrap();
    let rows = [
        String::new(),
        "$".to_owned(),
        "nodollar".to_owned(),
        format!("{salt}${digest}$extra"),
        format!("zz{}${digest}", &salt[2..]),
        format!("{}${digest}", &salt[..31]),
        format!("{salt}$\u{e9}{}", &digest[1..]),
        "\u{0}$\u{0}".to_owned(),
        format!("{salt}$"),
    ];
    for row in rows {
        // Python raises (a 500) for the malformed rows. Rust reports an error, never a panic.
        let result = verify_password("pw-12345678", &row);
        assert!(
            matches!(result, Err(_) | Ok(false)),
            "{row:?} must not verify"
        );
    }
    assert!(verify_password("pw-12345678", "nodollar").is_err());
    assert!(verify_password("pw-12345678", &format!("{salt}${digest}$extra")).is_err());
    assert!(verify_password("pw-12345678", &format!("zz{}${digest}", &salt[2..])).is_err());
}

#[test]
fn wrong_length_digest_is_a_plain_mismatch() {
    // Python compares the two hex strings, so a short digest is just False, not an error.
    let good = hash_password("pw-12345678");
    let (salt, digest) = good.split_once('$').unwrap();
    assert_eq!(
        verify_password("pw-12345678", &format!("{salt}${}", &digest[..63])),
        Ok(false)
    );
    assert_eq!(
        verify_password("pw-12345678", &format!("{salt}${}", digest.to_uppercase())),
        Ok(false),
        "stored digest is compared as lower-case text, as Python does"
    );
}

#[test]
fn password_error_maps_to_the_500_body() {
    use minidon_rust::platform::error::AppError;
    assert_eq!(AppError::from(PasswordError), AppError::Internal);
}
