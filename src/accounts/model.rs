//! Account types and wire shapes (DERIVED, OP-05 and OP-06 are the STATED sources).
//!
//! Stored enum names are the upper-case member names Python's SQLAlchemy `Enum` writes. The wire
//! value is the lower-case `StrEnum` value. The enum-name helpers deferred from wave 1 live here
//! with the only enum story 1 has.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::platform::clock::{format_wire, NaiveUtc};

/// `minidon/accounts/models.py:12-15`: USER, MODERATOR, ADMIN.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Moderator,
    Admin,
}

impl Role {
    /// The name stored in the `role` column.
    pub fn stored_name(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Moderator => "MODERATOR",
            Self::Admin => "ADMIN",
        }
    }

    /// The value on the wire.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Moderator => "moderator",
            Self::Admin => "admin",
        }
    }

    /// Read a stored name. Exact match only, as SQLAlchemy does: anything else is `None`.
    pub fn from_stored(name: &str) -> Option<Self> {
        match name {
            "USER" => Some(Self::User),
            "MODERATOR" => Some(Self::Moderator),
            "ADMIN" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Moderators and admins (`STAFF_ROLES` in Python).
    pub fn is_staff(self) -> bool {
        matches!(self, Self::Moderator | Self::Admin)
    }
}

impl Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.wire_name())
    }
}

/// One row of `accounts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub display_name: String,
    pub bio: String,
    pub locked: bool,
    pub role: Role,
    pub suspended_at: Option<NaiveUtc>,
    pub created_at: NaiveUtc,
}

/// One row of `access_tokens`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessToken {
    pub token: String,
    pub account_id: i64,
    pub created_at: NaiveUtc,
    pub expires_at: NaiveUtc,
}

/// The three counts a profile carries (review M-02). Python reads them from the `follows` and
/// `statuses` tables, so story 1 reads them too, and never writes there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProfileCounts {
    pub followers: i64,
    pub following: i64,
    pub statuses: i64,
}

/// Public profile. Field order is Python's `AccountOut` order (`accounts/schemas.py:14-25`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountOut {
    pub id: i64,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub bio: String,
    pub locked: bool,
    pub role: Role,
    pub created_at: String,
    pub followers_count: i64,
    pub following_count: i64,
    pub statuses_count: i64,
}

impl AccountOut {
    pub fn new(account: &Account, instance_domain: &str, counts: ProfileCounts) -> Self {
        Self {
            id: account.id,
            username: account.username.clone(),
            acct: format!("{}@{instance_domain}", account.username),
            display_name: account.display_name.clone(),
            bio: account.bio.clone(),
            locked: account.locked,
            role: account.role,
            created_at: format_wire(account.created_at),
            followers_count: counts.followers,
            following_count: counts.following,
            statuses_count: counts.statuses,
        }
    }
}

/// `POST /auth/token` response. Field order is Python's `TokenOut`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenOut {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_at: String,
}

impl TokenOut {
    pub fn new(access: &AccessToken) -> Self {
        Self {
            access_token: access.token.clone(),
            token_type: "bearer",
            expires_at: format_wire(access.expires_at),
        }
    }
}

/// `POST /accounts` body.
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterIn {
    pub username: String,
    pub email: String,
    pub password: String,
}

/// `POST /auth/token` body.
#[derive(Debug, Clone, Deserialize)]
pub struct TokenIn {
    pub username: String,
    pub password: String,
}

/// `PATCH /accounts/me` body. A missing field and an explicit `null` both mean "leave alone",
/// as in Python (`ProfileIn` defaults to `None`, `update_profile` skips `None`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileIn {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default, deserialize_with = "lax_bool")]
    pub locked: Option<bool>,
}

/// Pydantic's lax bool, as Python answers it (checked on the oracle): `true`/`false`, the
/// integers 0 and 1, the floats 0.0 and 1.0, and the strings 0 off f false n no / 1 on t true y
/// yes in any letter case. Everything else is an error (422). `null` means leave alone.
fn lax_bool<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let parsed = match &value {
        Value::Null => return Ok(None),
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match (n.as_i64(), n.as_u64(), n.as_f64()) {
            (Some(0), _, _) | (_, Some(0), _) => Some(false),
            (Some(1), _, _) | (_, Some(1), _) => Some(true),
            (None, None, Some(0.0)) => Some(false),
            (None, None, Some(1.0)) => Some(true),
            _ => None,
        },
        Value::String(text) => match text.to_lowercase().as_str() {
            "0" | "off" | "f" | "false" | "n" | "no" => Some(false),
            "1" | "on" | "t" | "true" | "y" | "yes" => Some(true),
            _ => None,
        },
        _ => None,
    };
    parsed
        .map(Some)
        .ok_or_else(|| serde::de::Error::custom("Input should be a valid boolean"))
}

/// The top level fields of a JSON body that held a lone surrogate (see `platform::scrub`).
/// Python accepts such text and then fails with a 500 where SQLite or the hash meets it. The
/// handler passes this to the service, which keeps Python's check order and answers a 4xx at the
/// step where Python crashes (adr-001 section 5 decisions 1 and 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tainted {
    pub username: bool,
    pub email: bool,
    pub password: bool,
    pub display_name: bool,
    pub bio: bool,
}

impl Tainted {
    /// Build from the field names the scrubber reported. Unknown fields are ignored: Python
    /// ignores extra fields too.
    pub fn from_fields(fields: &[String]) -> Self {
        let has = |name: &str| fields.iter().any(|f| f == name);
        Self {
            username: has("username"),
            email: has("email"),
            password: has("password"),
            display_name: has("display_name"),
            bio: has("bio"),
        }
    }
}
