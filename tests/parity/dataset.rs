//! Synthetic data for the parity runs (STATED OP-09: no real user data). Accounts are made
//! through the API of both builds. States the API cannot reach (a suspended account, a deleted
//! account row, a token that expires at an exact instant) are seeded with the same SQL on both
//! database copies, so Accounts is proven without Moderation code (assumption A-02).
#![allow(dead_code)]

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Suspend an account at T0 (the column holds Python's stored datetime text).
pub fn suspend(username: &str) -> String {
    format!(
        "UPDATE accounts SET suspended_at = '2026-01-01 00:00:00.000000' WHERE username = {}",
        quote(username)
    )
}

pub fn unsuspend(username: &str) -> String {
    format!(
        "UPDATE accounts SET suspended_at = NULL WHERE username = {}",
        quote(username)
    )
}

/// Delete the account row but leave its tokens (no foreign key is enforced).
pub fn delete_account(username: &str) -> String {
    format!("DELETE FROM accounts WHERE username = {}", quote(username))
}

pub fn set_role(username: &str, stored_role: &str) -> String {
    format!(
        "UPDATE accounts SET role = {} WHERE username = {}",
        quote(stored_role),
        quote(username)
    )
}

/// An accepted or pending follow, for the profile counts.
pub fn follow(follower_id: i64, followee_id: i64, accepted: bool) -> String {
    format!(
        "INSERT INTO follows (follower_id, followee_id, accepted, created_at) VALUES ({follower_id}, {followee_id}, {}, '2026-01-01 00:00:00.000000')",
        i32::from(accepted)
    )
}

/// A status row. `deleted` and `boost_of` cover the rows the count must skip.
pub fn status(account_id: i64, deleted: bool, boost_of: Option<i64>) -> String {
    format!(
        "INSERT INTO statuses (account_id, content, spoiler_text, visibility, reblog_of_id, created_at, deleted_at) VALUES ({account_id}, 'hello', '', 'PUBLIC', {}, '2026-01-01 00:00:00.000000', {})",
        boost_of.map_or_else(|| "NULL".to_owned(), |id| id.to_string()),
        if deleted { "'2026-01-01 00:00:00.000000'" } else { "NULL" }
    )
}
