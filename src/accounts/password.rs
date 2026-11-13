//! Password hashing (BR-007, DERIVED from `accounts/service.py:21-35`).
//!
//! PBKDF2-HMAC-SHA256, 100,000 iterations, 16-byte random salt, UTF-8 password bytes, stored as
//! `<salt hex>$<digest hex>` in lower-case hex. Hashes are byte-compatible with Python in both
//! directions. No rehash on sign-in.
//!
//! Known small difference (not reproduced): Python's `bytes.fromhex` skips ASCII whitespace
//! inside the salt text. Rust accepts strict hex only, so such a (never written) row is an error.

use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::platform::error::AppError;

pub const ITERATIONS: u32 = 100_000;
pub const SALT_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;

/// The stored hash is not in `<salt hex>$<digest hex>` form. Python raises here, which is a 500.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PasswordError;

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stored password hash is malformed")
    }
}

impl std::error::Error for PasswordError {}

impl From<PasswordError> for AppError {
    /// Fixed 500 body, nothing about the row is echoed.
    fn from(_: PasswordError) -> Self {
        Self::Internal
    }
}

fn derive(password: &str, salt: &[u8]) -> [u8; DIGEST_BYTES] {
    let mut out = [0u8; DIGEST_BYTES];
    pbkdf2::pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, ITERATIONS, &mut out);
    out
}

/// Hash with a given salt. For known-answer tests. Production code calls [`hash_password`].
pub fn hash_password_with_salt(password: &str, salt: &[u8; SALT_BYTES]) -> String {
    format!("{}${}", hex::encode(salt), hex::encode(derive(password, salt)))
}

/// Hash with a fresh OS-random 16-byte salt.
pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; SALT_BYTES];
    OsRng.fill_bytes(&mut salt);
    hash_password_with_salt(password, &salt)
}

/// Check a password against a stored hash. `Ok(false)` is a wrong password. `Err` is a row that
/// cannot be read. The digest comparison is constant time over the two lower-case hex texts, as
/// Python's `hmac.compare_digest(candidate.hex(), digest)` does.
pub fn verify_password(password: &str, stored: &str) -> Result<bool, PasswordError> {
    let mut parts = stored.split('$');
    let (Some(salt_hex), Some(digest_hex), None) = (parts.next(), parts.next(), parts.next())
    else {
        return Err(PasswordError);
    };
    let salt = hex::decode(salt_hex).map_err(|_| PasswordError)?;
    // Python's compare_digest raises on non-ASCII text, a 500 there as well.
    if !digest_hex.is_ascii() {
        return Err(PasswordError);
    }
    let candidate = hex::encode(derive(password, &salt));
    Ok(candidate.as_bytes().ct_eq(digest_hex.as_bytes()).into())
}
