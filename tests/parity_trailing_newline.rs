//! PT-ACC-004a to PT-ACC-004f: the REQUIRED trailing-newline username parity test (scenario
//! SC-ACC-004, rule BR-001, requirement REQ-002, review check RC-001, plan section 5).
//!
//! STATED, human decision 2026-11-13 (kurtis): Python's `$` also matches just before one final
//! newline, so `alice\n` is a valid username there. Rust copies that on purpose for parity. It
//! is a known weakness (look-alike accounts), the fix is a follow-up after cutover. When that
//! fix lands this file changes in both builds, by an explicit decision and never silently.
//!
//! Each test runs the same requests on the Python oracle and on Rust and asserts the expected
//! outcome on the Python answer too. The neighbouring cases make an over-broad fix fail as well
//! as a rejection of `alice\n`. The test names carry the full id, so a report cannot mix them
//! up with PT-ACC-004 (BR-004, duplicate usernames).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod parity;

use parity::{Pair, Req, PASSWORD};
use serde_json::json;

async fn register(pair: &Pair, username: &str, email: &str, status: u16) -> parity::Observed {
    let (py, _) = pair.register_with(username, email, PASSWORD).await;
    assert_eq!(py.status, status, "{username:?}: {}", py.body());
    py
}

#[tokio::test]
async fn pt_acc_004a_alice_newline_is_accepted_and_kept() {
    let pair = Pair::new().await;
    let py = register(&pair, "alice\n", "a@x.org", 201).await;
    assert_eq!(py.json()["username"], "alice\n");
    assert_eq!(py.json()["acct"], "alice\n@minidon.local");
    assert_eq!(
        pair.db_same("SELECT username FROM accounts").await.unwrap(),
        "alice\n"
    );
    // the pattern runs on the lower-cased name, so upper case and a newline work together
    let py = register(&pair, "BOB\n", "b@x.org", 201).await;
    assert_eq!(py.json()["username"], "bob\n");
    // `ALICE\n` is the same name as `alice\n`
    let py = register(&pair, "ALICE\n", "c@x.org", 409).await;
    assert_eq!(py.json(), json!({"error": "Username is taken"}));
}

#[tokio::test]
async fn pt_acc_004b_alice_and_alice_newline_are_two_accounts_in_either_order() {
    let pair = Pair::new().await;
    register(&pair, "alice", "plain@x.org", 201).await;
    let second = register(&pair, "alice\n", "newline@x.org", 201).await;
    assert_eq!(second.json()["username"], "alice\n");
    // the reverse order
    register(&pair, "carl\n", "carl-nl@x.org", 201).await;
    let plain = register(&pair, "carl", "carl@x.org", 201).await;
    assert_eq!(plain.json()["username"], "carl");
    assert_eq!(
        pair.db_same("SELECT CAST(COUNT(*) AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "4"
    );
    // a repeat of either is a conflict, and the emails stay separate
    register(&pair, "alice", "other@x.org", 409).await;
    register(&pair, "alice\n", "other@x.org", 409).await;
    // sign-in (review m-10): the newline name reaches the look-alike, the plain one does not
    let nl = pair.login("alice\n", PASSWORD).await;
    let plain = pair.login("alice", PASSWORD).await;
    let nl_me = pair.same_status(&Req::me(&nl), 200).await.0;
    let plain_me = pair.same_status(&Req::me(&plain), 200).await.0;
    assert_eq!(nl_me.json()["username"], "alice\n");
    assert_eq!(plain_me.json()["username"], "alice");
    assert_ne!(nl_me.json()["id"], plain_me.json()["id"]);
    // `alice\n` with the other account's password does not work
    let body = json!({"username": "alice\n", "password": "wrong-password"});
    pair.same_status(
        &Req::post_json("/api/v1/auth/token", &body.to_string()),
        401,
    )
    .await;
}

#[tokio::test]
async fn pt_acc_004c_lookup_with_and_without_the_newline_gives_different_accounts() {
    let pair = Pair::new().await;
    register(&pair, "alice", "plain@x.org", 201).await;
    register(&pair, "alice\n", "newline@x.org", 201).await;
    let plain = pair.same_status(&Req::lookup("alice"), 200).await.0;
    let newline = pair.same_status(&Req::lookup("alice%0A"), 200).await.0;
    assert_eq!(plain.json()["username"], "alice");
    assert_eq!(newline.json()["username"], "alice\n");
    assert_ne!(plain.json()["id"], newline.json()["id"]);
    pair.same_status(&Req::lookup("ALICE%0A"), 200).await;
    pair.same_status(&Req::lookup("alice%0a"), 200).await;
    // two newlines and a carriage return are not the account
    for path in [
        "alice%0A%0A",
        "alice%0D%0A",
        "alice%0D",
        "%0Aalice",
        "ali%0Ace",
    ] {
        pair.same_status(&Req::lookup(path), 404).await;
    }
    // only the plain account exists: the newline form is not found
    let pair = Pair::new().await;
    register(&pair, "bob", "b@x.org", 201).await;
    pair.same_status(&Req::lookup("bob%0A"), 404).await;
}

#[tokio::test]
async fn pt_acc_004d_two_final_newlines_are_rejected() {
    let pair = Pair::new().await;
    for (i, name) in ["alice\n\n", "alice\n\n\n", "a\n\n"].iter().enumerate() {
        let py = register(&pair, name, &format!("d{i}@x.org"), 422).await;
        assert_eq!(
            py.json(),
            json!({"error": "Username must be 1-30 letters, digits or underscores"})
        );
    }
    assert_eq!(
        pair.db_same("SELECT CAST(COUNT(*) AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "0"
    );
}

#[tokio::test]
async fn pt_acc_004e_a_lone_newline_and_a_newline_inside_are_rejected() {
    let pair = Pair::new().await;
    for (i, name) in ["\n", "ali\nce", "\nalice", "a\nb\n", "\n\n"]
        .iter()
        .enumerate()
    {
        let py = register(&pair, name, &format!("e{i}@x.org"), 422).await;
        assert_eq!(
            py.json(),
            json!({"error": "Username must be 1-30 letters, digits or underscores"})
        );
    }
}

#[tokio::test]
async fn pt_acc_004f_thirty_characters_plus_a_newline_pass_and_crlf_fails() {
    let pair = Pair::new().await;
    let thirty = format!("{}\n", "a".repeat(30));
    let py = register(&pair, &thirty, "f1@x.org", 201).await;
    assert_eq!(py.json()["username"].as_str().unwrap().len(), 31);
    // 31 characters before the newline is too long, with or without it
    register(&pair, &format!("{}\n", "a".repeat(31)), "f2@x.org", 422).await;
    register(&pair, &"a".repeat(31), "f3@x.org", 422).await;
    // CR LF, CR, and other line breaks are not the final-newline case
    for (i, name) in [
        "alice\r\n",
        "alice\r",
        "alice\u{85}",
        "alice\u{2028}",
        "alice\u{b}",
        "alice\u{c}",
        "alice\n ",
    ]
    .iter()
    .enumerate()
    {
        register(&pair, name, &format!("g{i}@x.org"), 422).await;
    }
    // a 31 character name is accepted by lookup of the stored form
    pair.same_status(&Req::lookup(&format!("{}%0A", "a".repeat(30))), 200)
        .await;
    // and a stored name longer than the column length is read back by both (no length check)
    assert_eq!(
        pair.db_same("SELECT CAST(LENGTH(username) AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "31"
    );
}
