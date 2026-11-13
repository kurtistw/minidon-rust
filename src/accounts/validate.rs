//! Pure input rules for Accounts (BR-001 STATED, BR-002, BR-003, BR-013 DERIVED).
//!
//! Python facts (`accounts/service.py`, confirmed by reading and by running Python 3):
//! - username: `str.lower()` (full Unicode, no strip), then `re.match(r"^[a-z0-9_]{1,30}$")`.
//! - email: `str.strip().lower()`, only `"@" in email` is checked.
//! - password: `len(password) >= 8`, counted in code points.
//! - display name and bio: `len()` limits of 30 and 500, code points.

use std::sync::LazyLock;

use regex::Regex;

pub const USERNAME_MESSAGE: &str = "Username must be 1-30 letters, digits or underscores";
pub const EMAIL_MESSAGE: &str = "Email address is not valid";
pub const PASSWORD_MESSAGE: &str = "Password must be at least 8 characters";
pub const DISPLAY_NAME_MESSAGE: &str = "Display name must be at most 30 characters";
pub const BIO_MESSAGE: &str = "Bio must be at most 500 characters";

const MIN_PASSWORD_LENGTH: usize = 8;
const MAX_DISPLAY_NAME_LENGTH: usize = 30;
const MAX_BIO_LENGTH: usize = 500;

// DELIBERATE, not a mistake (STATED: human decision 2026-11-13, kurtis, review check RC-001).
// Python's `$` also matches just before ONE final newline, so `alice\n` is a valid username in
// the Python build. This pattern copies that on purpose for parity, with `\n?\z` in place of
// `$`: exactly one optional final newline, and `\z` only matches at the very end of the text.
// `alice\n\n`, `\n`, `ali\nce` and `alice\r\n` stay invalid. `alice\n` and `alice` become two
// look-alike accounts, a known weakness copied as is. FOLLOW-UP after cutover: reject the
// trailing newline in both builds. Until then, a change here must be an explicit decision, and
// `tests/unit_validate.rs` (UT-ACC-004) and the parity test PT-ACC-004a..f must change with it.
const USERNAME_PATTERN: &str = r"^[a-z0-9_]{1,30}\n?\z";

static USERNAME_REGEX: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(USERNAME_PATTERN).ok());

/// Python `str.strip()` whitespace: the Unicode White_Space set (what Rust's `char::is_whitespace`
/// knows) plus U+001C to U+001F, which Python also counts as whitespace.
fn is_python_whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python `str.strip()` with no argument.
pub fn python_strip(text: &str) -> &str {
    text.trim_matches(is_python_whitespace)
}

/// `username.lower()`. Full Unicode lower-casing (`str::to_lowercase`, incl. final sigma),
/// no strip. Every username lookup applies this too (review M-01).
pub fn normalise_username(raw: &str) -> String {
    raw.to_lowercase()
}

/// Match the username pattern on an already lower-cased username. Never panics: if the pattern
/// could not compile, nothing is valid.
pub fn username_valid(normalised: &str) -> bool {
    USERNAME_REGEX
        .as_ref()
        .is_some_and(|re| re.is_match(normalised))
}

/// `email.strip().lower()`.
pub fn normalise_email(raw: &str) -> String {
    python_strip(raw).to_lowercase()
}

pub fn email_valid(normalised: &str) -> bool {
    normalised.contains('@')
}

pub fn password_valid(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_LENGTH
}

pub fn display_name_valid(display_name: &str) -> bool {
    display_name.chars().count() <= MAX_DISPLAY_NAME_LENGTH
}

pub fn bio_valid(bio: &str) -> bool {
    bio.chars().count() <= MAX_BIO_LENGTH
}
