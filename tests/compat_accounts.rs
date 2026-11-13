//! Wave 6 COMPAT tests (REQ-016, REQ-017, NFR-002, NFR-004, NFR-011, STATED OP-05 and OP-09).
//!
//! The Python oracle and the Rust build serve ONE database file, which Python made and Rust
//! never alters. Whatever one build writes, the other must read, with no migration. All data is
//! synthetic and made through the API of the builds or by seed SQL (OP-09, no real data).
//!
//! Tests: CT-ACC-006 (stored role names), CT-ACC-007 and SC-ACC-015 (password hash and token
//! cross-verification, BR-007), CT-ACC-014 (locked flag), SC-ACC-029 (stored formats, column
//! defaults), SC-ACC-030 (synthetic dataset round trip), NFR-011 (schema unchanged).
//!
//! Test-first: `COMPAT_PYTHON_ONLY=1` sends the Rust side of every request to the oracle, so each
//! expectation is proven on Python alone. The compat tests only compare the two builds, they
//! never assert on a value no oracle produced.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod parity;

use parity::shared::SharedPair;
use parity::{dataset, Req, Side, PASSWORD, T0};
use serde_json::Value;
use time::macros::datetime;
use time::Duration;

use Side::{Python as Py, Rust as Rs};

/// `<salt hex>$<digest hex>`: 16 salt bytes, 32 digest bytes (BR-007).
fn assert_hash_shape(hash: &str) {
    let (salt, digest) = hash.split_once('$').expect("one $");
    let hex = |s: &str, n: usize| {
        s.len() == n && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    };
    assert!(hex(salt, 32) && hex(digest, 64), "bad stored hash {hash}");
}

fn is_stored_datetime(text: &str) -> bool {
    // YYYY-MM-DD HH:MM:SS.ffffff
    let b = text.as_bytes();
    b.len() == 26
        && b[4] == b'-'
        && b[7] == b'-'
        && b[10] == b' '
        && b[13] == b':'
        && b[16] == b':'
        && b[19] == b'.'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7 | 10 | 13 | 16 | 19) || c.is_ascii_digit())
}

fn other(side: Side) -> Side {
    match side {
        Side::Python => Side::Rust,
        Side::Rust => Side::Python,
    }
}

// ---- CT-ACC-007 / SC-ACC-015 (BR-007) ----

#[tokio::test]
async fn ct_acc_007_password_hashes_cross_verify_in_both_directions() {
    let pair = SharedPair::new().await;
    let passwords = [
        "password1",
        "pässwörd-密码-🔑-ünï",
        "  spaces inside and edges  ",
        &"x".repeat(200),
        "tab\there-and-newline\nok",
    ];
    for (i, password) in passwords.iter().enumerate() {
        for maker in [Py, Rs] {
            let user = format!("{}_{i}", if maker == Py { "py" } else { "rs" });
            pair.register(maker, &user, &format!("{user}@x.org"), password)
                .await;
            let reader = other(maker);
            // the other build verifies the hash this build made
            let answer = pair.sign_in(reader, &user, password).await;
            assert_eq!(
                answer.status,
                200,
                "{maker:?} -> {reader:?} {user}: {}",
                answer.body()
            );
            // and a wrong password is refused by the other build, with the one generic error
            let wrong = pair.sign_in(reader, &user, &format!("{password}!")).await;
            assert_eq!(wrong.status, 401, "{user}");
            assert_eq!(wrong.json()["error"], "Invalid username or password");
            // the build that made the hash still accepts it
            assert_eq!(pair.sign_in(maker, &user, password).await.status, 200);
        }
    }
    // both directions again, with a user name written in other case than it was registered
    pair.register(Py, "MixedCasePy", "mixedpy@x.org", PASSWORD)
        .await;
    pair.register(Rs, "MixedCaseRs", "mixedrs@x.org", PASSWORD)
        .await;
    assert_eq!(pair.sign_in(Rs, "MIXEDCASEPY", PASSWORD).await.status, 200);
    assert_eq!(pair.sign_in(Py, "mixedcasers", PASSWORD).await.status, 200);

    // stored rows: the same shape from both builds, random salts (no two equal), no plaintext
    let rows = pair
        .rows("SELECT username, password_hash FROM accounts ORDER BY id")
        .await;
    assert_eq!(rows.len(), passwords.len() * 2 + 2);
    let mut salts = std::collections::BTreeSet::new();
    for row in &rows {
        let (user, hash) = (row[0].clone().unwrap(), row[1].clone().unwrap());
        assert_hash_shape(&hash);
        assert!(
            salts.insert(hash.split_once('$').unwrap().0.to_owned()),
            "salt reused: {user}"
        );
        for password in passwords {
            assert!(
                !hash.contains(password),
                "plaintext in the stored hash of {user}"
            );
        }
    }
    // whole rows hold no plaintext either
    for password in passwords {
        let like = format!("%{}%", password.replace('\'', "''"));
        let hit = pair
            .scalar(&format!(
                "SELECT CAST(COUNT(*) AS TEXT) FROM accounts WHERE username || email || password_hash || display_name || bio LIKE '{like}'"
            ))
            .await;
        assert_eq!(
            hit.as_deref(),
            Some("0"),
            "plaintext password stored: {password:?}"
        );
    }
}

#[tokio::test]
async fn ct_acc_007_a_copied_stored_hash_verifies_on_both_builds() {
    // A stored hash is copied byte for byte into a second row (as a database restore would).
    // Both builds must accept it, so nothing but the stored text matters.
    let pair = SharedPair::new().await;
    pair.register(Py, "seedbase", "seedbase@x.org", PASSWORD)
        .await;
    let stored = pair
        .scalar("SELECT password_hash FROM accounts WHERE username = 'seedbase'")
        .await
        .unwrap();
    pair.exec(&format!(
        "INSERT INTO accounts (username, email, password_hash, display_name, bio, locked, role, created_at) \
         VALUES ('copied', 'copied@x.org', '{stored}', '', '', 0, 'USER', '2026-01-01 00:00:00.000000')"
    ))
    .await;
    for side in [Py, Rs] {
        assert_eq!(
            pair.sign_in(side, "copied", PASSWORD).await.status,
            200,
            "{side:?}"
        );
    }
}

// ---- CT-ACC-007: tokens across builds, expiry boundary ----

#[tokio::test]
async fn ct_acc_007_tokens_authenticate_on_the_other_build_with_the_exact_expiry_boundary() {
    let pair = SharedPair::new().await;
    let start = datetime!(2026-02-10 08:00:00.250000);
    pair.set_clock(start).await;
    for (maker, name) in [(Py, "tokpy"), (Rs, "tokrs")] {
        pair.register(maker, name, &format!("{name}@x.org"), PASSWORD)
            .await;
    }
    for maker in [Py, Rs] {
        let name = if maker == Py { "tokpy" } else { "tokrs" };
        let token = pair.token(maker, name, PASSWORD).await;
        let issued = pair.sign_in(maker, name, PASSWORD).await.json();
        assert_eq!(
            issued["expires_at"], "2026-03-12T08:00:00.250000",
            "{maker:?}"
        );
        let end = start + Duration::days(30);
        for (at, expected) in [
            (start, 200),
            (end - Duration::days(1), 200),
            (end - Duration::microseconds(1), 200),
            (end, 401),
            (end + Duration::microseconds(1), 401),
        ] {
            pair.set_clock(at).await;
            // both builds, whoever issued the token, say the same at every instant
            for side in [Py, Rs] {
                let answer = pair.me(side, &token).await;
                assert_eq!(
                    answer.status,
                    expected,
                    "{maker:?} token read by {side:?} at {at}: {}",
                    answer.body()
                );
            }
        }
        pair.set_clock(start).await;
    }
    // a Python token authenticates the right person on Rust, and a Rust token on Python
    let py_token = pair.token(Py, "tokpy", PASSWORD).await;
    let rs_token = pair.token(Rs, "tokrs", PASSWORD).await;
    assert_eq!(pair.me(Rs, &py_token).await.json()["username"], "tokpy");
    assert_eq!(pair.me(Py, &rs_token).await.json()["username"], "tokrs");
    // stored token rows: same text formats, whoever wrote them
    let rows = pair
        .rows("SELECT token, created_at, expires_at, typeof(account_id) FROM access_tokens")
        .await;
    assert!(rows.len() >= 6);
    for row in rows {
        let token = row[0].clone().unwrap();
        assert_eq!(token.len(), 43, "{token}");
        assert!(token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert!(is_stored_datetime(&row[1].clone().unwrap()), "{row:?}");
        assert!(is_stored_datetime(&row[2].clone().unwrap()), "{row:?}");
        assert_eq!(row[3].as_deref(), Some("integer"));
    }
    // a token of a suspended account is refused by both builds, and valid again after
    pair.exec(&dataset::suspend("tokpy")).await;
    for side in [Py, Rs] {
        assert_eq!(pair.me(side, &py_token).await.status, 401, "{side:?}");
        assert_eq!(
            pair.sign_in(side, "tokpy", PASSWORD).await.status,
            403,
            "{side:?}"
        );
    }
    pair.exec(&dataset::unsuspend("tokpy")).await;
    for side in [Py, Rs] {
        assert_eq!(pair.me(side, &py_token).await.status, 200, "{side:?}");
    }
}

// ---- CT-ACC-006 (BR-006) ----

#[tokio::test]
async fn ct_acc_006_stored_role_names_are_read_by_both_builds() {
    let pair = SharedPair::new().await;
    // the first account of the database is the admin, whichever build counts it
    pair.register(Py, "first", "first@x.org", PASSWORD).await;
    pair.register(Rs, "second", "second@x.org", PASSWORD).await;
    pair.register(Py, "third", "third@x.org", PASSWORD).await;
    pair.register(Rs, "fourth", "fourth@x.org", PASSWORD).await;
    let roles = pair
        .rows("SELECT username, role FROM accounts ORDER BY id")
        .await;
    let roles: Vec<(String, String)> = roles
        .into_iter()
        .map(|r| (r[0].clone().unwrap(), r[1].clone().unwrap()))
        .collect();
    assert_eq!(
        roles,
        [
            ("first", "ADMIN"),
            ("second", "USER"),
            ("third", "USER"),
            ("fourth", "USER")
        ]
        .map(|(a, b)| (a.to_owned(), b.to_owned()))
    );
    // MODERATOR cannot be made through the API (moderation is another story): seed it
    pair.exec(&dataset::set_role("third", "MODERATOR")).await;
    for (user, wire) in [
        ("first", "admin"),
        ("second", "user"),
        ("third", "moderator"),
        ("fourth", "user"),
    ] {
        let token = pair.token(Py, user, PASSWORD).await;
        for side in [Py, Rs] {
            let me = pair.me(side, &token).await;
            assert_eq!(me.status, 200);
            assert_eq!(me.json()["role"], wire, "{user} on {side:?}");
        }
        // sign-in on Rust gives a token that Python honours with the same role
        let rs_token = pair.token(Rs, user, PASSWORD).await;
        assert_eq!(pair.me(Py, &rs_token).await.json()["role"], wire, "{user}");
        assert_eq!(pair.lookup(Rs, user).await.json()["role"], wire);
        assert_eq!(pair.lookup(Py, user).await.json()["role"], wire);
    }
    // what the update route writes never changes the stored name
    let token = pair.token(Rs, "third", PASSWORD).await;
    assert_eq!(
        pair.patch_me(Rs, &token, r#"{"bio":"mod"}"#).await.status,
        200
    );
    assert_eq!(
        pair.scalar("SELECT role FROM accounts WHERE username = 'third'")
            .await
            .as_deref(),
        Some("MODERATOR")
    );
    assert_eq!(pair.me(Py, &token).await.json()["role"], "moderator");
}

#[tokio::test]
async fn ct_acc_006_the_first_account_made_by_rust_is_stored_as_admin_for_python() {
    let pair = SharedPair::new().await;
    pair.register(Rs, "rsfirst", "rsfirst@x.org", PASSWORD)
        .await;
    pair.register(Py, "pysecond", "pysecond@x.org", PASSWORD)
        .await;
    let stored = pair
        .rows("SELECT username, role FROM accounts ORDER BY id")
        .await;
    assert_eq!(stored[0][1].as_deref(), Some("ADMIN"));
    assert_eq!(stored[1][1].as_deref(), Some("USER"));
    let token = pair.token(Py, "rsfirst", PASSWORD).await;
    assert_eq!(pair.me(Py, &token).await.json()["role"], "admin");
    // suspended accounts still count as accounts: the next one is a user on both builds
    pair.exec(&dataset::suspend("rsfirst")).await;
    pair.exec(&dataset::suspend("pysecond")).await;
    pair.register(Rs, "after", "after@x.org", PASSWORD).await;
    assert_eq!(
        pair.scalar("SELECT role FROM accounts WHERE username = 'after'")
            .await
            .as_deref(),
        Some("USER")
    );
}

// ---- CT-ACC-014 (BR-014) ----

#[tokio::test]
async fn ct_acc_014_the_locked_flag_is_one_boolean_for_both_builds() {
    let pair = SharedPair::new().await;
    pair.register(Py, "lockpy", "lockpy@x.org", PASSWORD).await;
    pair.register(Rs, "lockrs", "lockrs@x.org", PASSWORD).await;
    let stored = |user: &'static str| {
        let pair = &pair;
        async move {
            pair.rows(&format!(
                "SELECT CAST(locked AS TEXT), typeof(locked) FROM accounts WHERE username = '{user}'"
            ))
            .await
            .remove(0)
        }
    };
    for user in ["lockpy", "lockrs"] {
        assert_eq!(
            stored(user).await,
            [Some("0".into()), Some("integer".into())],
            "default of {user}"
        );
    }
    for (user, maker) in [("lockpy", Py), ("lockrs", Rs)] {
        let reader = other(maker);
        let mine = pair.token(maker, user, PASSWORD).await;
        let theirs = pair.token(reader, user, PASSWORD).await;
        // this build locks, the other build sees it (profile and own account)
        let answer = pair.patch_me(maker, &mine, r#"{"locked":true}"#).await;
        assert_eq!(answer.json()["locked"], true, "{user}");
        assert_eq!(
            stored(if user == "lockpy" { "lockpy" } else { "lockrs" }).await,
            [Some("1".into()), Some("integer".into())]
        );
        assert_eq!(pair.lookup(reader, user).await.json()["locked"], true);
        assert_eq!(pair.me(reader, &theirs).await.json()["locked"], true);
        // the other build unlocks, this one sees it
        let answer = pair.patch_me(reader, &theirs, r#"{"locked":false}"#).await;
        assert_eq!(answer.json()["locked"], false);
        assert_eq!(pair.lookup(maker, user).await.json()["locked"], false);
        assert_eq!(pair.me(maker, &mine).await.json()["locked"], false);
        // a PATCH without the field leaves it alone, on both builds
        pair.patch_me(maker, &mine, r#"{"locked":true}"#).await;
        pair.patch_me(reader, &theirs, r#"{"bio":"no lock change"}"#)
            .await;
        assert_eq!(pair.lookup(maker, user).await.json()["locked"], true);
        pair.patch_me(maker, &mine, r#"{"bio":"again"}"#).await;
        assert_eq!(pair.lookup(reader, user).await.json()["locked"], true);
    }
    // a seeded 0 and 1 (the two values SQLAlchemy writes) read the same on both builds
    pair.exec("UPDATE accounts SET locked = 1 WHERE username = 'lockpy'")
        .await;
    pair.exec("UPDATE accounts SET locked = 0 WHERE username = 'lockrs'")
        .await;
    for side in [Py, Rs] {
        assert_eq!(pair.lookup(side, "lockpy").await.json()["locked"], true);
        assert_eq!(pair.lookup(side, "lockrs").await.json()["locked"], false);
    }
}

// ---- SC-ACC-029: stored formats and column defaults ----

#[tokio::test]
async fn sc_acc_029_rows_written_by_each_build_have_the_same_stored_formats_and_defaults() {
    let pair = SharedPair::new().await;
    pair.set_clock(datetime!(2026-03-04 05:06:07.080009)).await;
    pair.register(Py, "fmtpy", "fmtpy@x.org", PASSWORD).await;
    pair.register(Rs, "fmtrs", "fmtrs@x.org", PASSWORD).await;
    // a second registration at a whole second: the fraction must still be six digits in the file
    pair.set_clock(datetime!(2026-03-04 05:06:08)).await;
    pair.register(Py, "wholepy", "wholepy@x.org", PASSWORD)
        .await;
    pair.register(Rs, "wholers", "wholers@x.org", PASSWORD)
        .await;
    let rows = pair
        .rows(
            "SELECT username, typeof(id), typeof(username), typeof(email), typeof(password_hash), \
             typeof(display_name), typeof(bio), typeof(locked), typeof(role), typeof(suspended_at), \
             typeof(created_at), display_name, bio, CAST(locked AS TEXT), suspended_at, created_at \
             FROM accounts ORDER BY id",
        )
        .await;
    assert_eq!(rows.len(), 4);
    let shape = |row: &Vec<Option<String>>| row[1..=11].to_vec();
    for row in &rows {
        // same column types and defaults whoever wrote the row
        assert_eq!(shape(row), shape(&rows[0]));
        assert_eq!(
            row[1..=10]
                .iter()
                .map(|v| v.clone().unwrap())
                .collect::<Vec<_>>(),
            [
                "integer", "text", "text", "text", "text", "text", "integer", "text", "null",
                "text"
            ]
        );
        assert_eq!(row[11].as_deref(), Some(""), "display_name default");
        assert_eq!(row[12].as_deref(), Some(""), "bio default");
        assert_eq!(row[13].as_deref(), Some("0"), "locked default");
        assert_eq!(row[14], None, "suspended_at default");
        assert!(is_stored_datetime(row[15].as_deref().unwrap()), "{row:?}");
    }
    let created: Vec<String> = rows.iter().map(|r| r[15].clone().unwrap()).collect();
    assert_eq!(
        created,
        [
            "2026-03-04 05:06:07.080009",
            "2026-03-04 05:06:07.080009",
            "2026-03-04 05:06:08.000000",
            "2026-03-04 05:06:08.000000"
        ]
    );
    // ids are one sequence across both builds
    let ids = pair
        .rows("SELECT CAST(id AS TEXT) FROM accounts ORDER BY id")
        .await;
    let ids: Vec<&str> = ids.iter().map(|r| r[0].as_deref().unwrap()).collect();
    assert_eq!(ids, ["1", "2", "3", "4"]);
    // every build reads every row's dates the same way
    for user in ["fmtpy", "fmtrs", "wholepy", "wholers"] {
        let (py, rs) = pair.same(&Req::lookup(user)).await;
        assert_eq!(py.json()["created_at"], rs.json()["created_at"]);
    }
    assert_eq!(
        pair.lookup(Rs, "fmtpy").await.json()["created_at"],
        "2026-03-04T05:06:07.080009"
    );
    assert_eq!(
        pair.lookup(Py, "wholers").await.json()["created_at"],
        "2026-03-04T05:06:08"
    );
}

#[tokio::test]
async fn sc_acc_029_text_survives_a_round_trip_between_builds() {
    let pair = SharedPair::new().await;
    pair.register(Py, "textpy", "Text.Py@Example.ORG", PASSWORD)
        .await;
    pair.register(Rs, "textrs", "Text.Rs@Example.ORG", PASSWORD)
        .await;
    let long_bio =
        "line one\nline two\r\n\u{1F600} \u{e9}\u{301} \"quoted\" 'single' \\ back ".repeat(10);
    let bio: String = long_bio.chars().take(500).collect();
    let name: String = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} Ünï 密码 "
        .chars()
        .take(30)
        .collect();
    for (maker, user) in [(Py, "textpy"), (Rs, "textrs")] {
        let token = pair.token(maker, user, PASSWORD).await;
        let body = serde_json::json!({"display_name": name, "bio": bio}).to_string();
        let answer = pair.patch_me(maker, &token, &body).await;
        assert_eq!(answer.status, 200, "{}", answer.body());
        // the other build reads exactly the characters that were written
        let seen = pair.lookup(other(maker), user).await.json();
        assert_eq!(seen["display_name"], Value::String(name.clone()), "{user}");
        assert_eq!(seen["bio"], Value::String(bio.clone()), "{user}");
        // and an update by the other build keeps them readable for this one
        let other_token = pair.token(other(maker), user, PASSWORD).await;
        let body = serde_json::json!({"bio": "short"}).to_string();
        assert_eq!(
            pair.patch_me(other(maker), &other_token, &body)
                .await
                .status,
            200
        );
        assert_eq!(
            pair.lookup(maker, user).await.json()["display_name"],
            Value::String(name.clone())
        );
    }
    // stored email text is lower-cased the same way (BR-003: Python's str.lower on this input)
    let emails = pair.rows("SELECT email FROM accounts ORDER BY id").await;
    assert_eq!(emails[0][0].as_deref(), Some("text.py@example.org"));
    assert_eq!(emails[1][0].as_deref(), Some("text.rs@example.org"));
    // a duplicate e-mail is found across builds, in any case
    for (side, user) in [(Rs, "dup1"), (Py, "dup2")] {
        let answer = pair
            .send(
                side,
                &Req::register(
                    user,
                    if side == Rs {
                        "TEXT.PY@example.org"
                    } else {
                        "text.rs@EXAMPLE.org"
                    },
                    PASSWORD,
                ),
            )
            .await;
        assert_eq!(answer.status, 409, "{side:?}: {}", answer.body());
    }
    // and so is a duplicate user name
    assert_eq!(
        pair.send(Rs, &Req::register("TEXTPY", "new1@x.org", PASSWORD))
            .await
            .status,
        409
    );
    assert_eq!(
        pair.send(Py, &Req::register("textRS", "new2@x.org", PASSWORD))
            .await
            .status,
        409
    );
}

// ---- SC-ACC-030: synthetic dataset round trip ----

/// A small synthetic population (OP-09): made by Python through its API, plus states the API
/// cannot reach, seeded with plain DML. Returns the user names.
async fn python_dataset(pair: &SharedPair) -> Vec<String> {
    let people = [
        ("admin_user", "admin@x.org", "password1"),
        ("Alice_01", "alice@x.org", "password1"),
        ("bob", "bob@x.org", "pässwörd-1-密码"),
        (
            "carol_c",
            "carol@x.org",
            "a long passphrase with spaces 123",
        ),
        ("dave", "dave@x.org", "password1"),
        ("erin", "erin@x.org", "🔑🔑🔑🔑🔑🔑🔑🔑"),
        ("frank", "frank@x.org", "password1"),
        ("gina", "gina@x.org", "password1"),
        ("hank", "hank@x.org", "password1"),
        ("ivy", "ivy@x.org", "password1"),
    ];
    let mut names = Vec::new();
    for (i, (user, email, password)) in people.iter().enumerate() {
        pair.set_clock(T0 + Duration::hours(i as i64) + Duration::microseconds(i as i64 * 7))
            .await;
        pair.register(Py, user, email, password).await;
        names.push(user.to_lowercase());
    }
    let token = pair.token(Py, "bob", "pässwörd-1-密码").await;
    pair.patch_me(
        Py,
        &token,
        r#"{"display_name":"Bob B.","bio":"synthetic bio","locked":true}"#,
    )
    .await;
    let token = pair
        .token(Py, "carol_c", "a long passphrase with spaces 123")
        .await;
    pair.patch_me(Py, &token, r#"{"display_name":"Carol é"}"#)
        .await;
    pair.exec(&dataset::set_role("dave", "MODERATOR")).await;
    pair.exec(&dataset::suspend("gina")).await;
    pair.exec(&dataset::follow(2, 3, true)).await;
    pair.exec(&dataset::follow(4, 3, true)).await;
    pair.exec(&dataset::follow(5, 3, false)).await;
    pair.exec(&dataset::follow(3, 2, true)).await;
    pair.exec(&dataset::status(3, false, None)).await;
    pair.exec(&dataset::status(3, false, None)).await;
    pair.exec(&dataset::status(3, true, None)).await;
    pair.exec(&dataset::status(3, false, Some(1))).await;
    pair.set_clock(T0 + Duration::days(1)).await;
    names
}

async fn every_account_reads_the_same(pair: &SharedPair, names: &[String]) {
    for name in names {
        // a suspended account is hidden from lookup (BR-009), on both builds
        let expected = if name == "gina" { 404 } else { 200 };
        let (py, _) = pair.same(&Req::lookup(name)).await;
        assert_eq!(py.status, expected, "{name}");
    }
    let (py, _) = pair.same(&Req::lookup("nobody_here")).await;
    assert_eq!(py.status, 404);
}

#[tokio::test]
async fn sc_acc_030_rust_serves_a_python_made_dataset_and_python_reads_what_rust_writes() {
    let pair = SharedPair::new().await;
    let names = python_dataset(&pair).await;
    let passwords = [
        ("admin_user", "password1"),
        ("alice_01", "password1"),
        ("bob", "pässwörd-1-密码"),
        ("carol_c", "a long passphrase with spaces 123"),
        ("dave", "password1"),
        ("erin", "🔑🔑🔑🔑🔑🔑🔑🔑"),
        ("frank", "password1"),
        ("hank", "password1"),
        ("ivy", "password1"),
    ];
    // Rust starts on the file as it is (no migration) and every profile reads the same
    every_account_reads_the_same(&pair, &names).await;
    let counts = pair.lookup(Rs, "bob").await.json();
    assert_eq!(counts["followers_count"], 2, "{counts}");
    // sign-in and me, on both builds, for every seeded account
    for (user, password) in passwords {
        let (py, rs) = pair.same(&Req::sign_in(user, password)).await;
        assert_eq!((py.status, rs.status), (200, 200), "{user}");
        let rs_token = rs.json()["access_token"].as_str().unwrap().to_owned();
        let py_token = py.json()["access_token"].as_str().unwrap().to_owned();
        // each build's token works on the other, and the answers are equal
        let tokens = parity::Tokens {
            py: py_token.clone(),
            rs: rs_token.clone(),
        };
        let (a, _) = pair.same(&Req::me(&tokens)).await;
        assert_eq!(a.status, 200);
        assert_eq!(
            pair.me(Py, &rs_token).await.json(),
            pair.me(Rs, &rs_token).await.json(),
            "{user}"
        );
        assert_eq!(
            pair.me(Rs, &py_token).await.json(),
            pair.me(Py, &py_token).await.json(),
            "{user}"
        );
    }
    // suspended account and bad credentials: the same refusals
    let (py, _) = pair.same(&Req::sign_in("gina", "password1")).await;
    assert_eq!(py.status, 403);
    pair.same(&Req::sign_in("gina", "wrong-password")).await;
    pair.same(&Req::sign_in("nobody_here", "password1")).await;

    // Rust writes: registers, updates, signs in. Python then reads the same rows.
    let mut written = Vec::new();
    for (i, user) in ["rs_new_1", "Rs_New_2", "rs_new_3"].iter().enumerate() {
        pair.set_clock(T0 + Duration::days(2) + Duration::seconds(i as i64))
            .await;
        pair.register(Rs, user, &format!("rsnew{i}@x.org"), "pässwörd-rust-密码")
            .await;
        written.push(user.to_lowercase());
    }
    let token = pair.token(Rs, "rs_new_1", "pässwörd-rust-密码").await;
    pair.patch_me(
        Rs,
        &token,
        r#"{"display_name":"Rust Made","bio":"written by rust","locked":true}"#,
    )
    .await;
    let token = pair.token(Rs, "alice_01", "password1").await;
    pair.patch_me(Rs, &token, r#"{"bio":"alice, edited by rust"}"#)
        .await;
    every_account_reads_the_same(&pair, &names).await;
    every_account_reads_the_same(&pair, &written).await;
    for user in &written {
        assert_eq!(
            pair.sign_in(Py, user, "pässwörd-rust-密码").await.status,
            200,
            "{user}"
        );
    }
    let alice = pair.lookup(Py, "alice_01").await.json();
    assert_eq!(alice["bio"], "alice, edited by rust");
    assert_eq!(alice["display_name"], "");
    let bob = pair.lookup(Py, "bob").await.json();
    assert_eq!(
        (bob["display_name"].as_str(), bob["locked"].as_bool()),
        (Some("Bob B."), Some(true))
    );
    // Python writes again over rows Rust wrote, Rust reads them back
    let token = pair.token(Py, "rs_new_1", "pässwörd-rust-密码").await;
    pair.patch_me(Py, &token, r#"{"bio":"python over rust","locked":false}"#)
        .await;
    let back = pair.lookup(Rs, "rs_new_1").await.json();
    assert_eq!(
        (back["bio"].as_str(), back["locked"].as_bool()),
        (Some("python over rust"), Some(false))
    );
    every_account_reads_the_same(&pair, &written).await;
}

// ---- NFR-011: Rust never changes the schema ----

#[tokio::test]
async fn nfr_011_neither_starting_nor_serving_rust_changes_the_schema_or_journal_mode() {
    let pair = SharedPair::new().await;
    // both servers are running and have opened the file: it is still the bytes Python made
    let template = std::fs::read(&parity::oracle().template_db).unwrap();
    assert_eq!(
        std::fs::read(&pair.db).unwrap(),
        template,
        "starting Rust wrote to the database"
    );
    let before = pair.schema_fingerprint().await;
    assert!(before.contains("CREATE TABLE accounts"), "{before}");
    assert!(before.contains("journal_mode=delete"), "{before}");
    // a full workload on Rust only
    pair.register(Rs, "nfr_a", "nfra@x.org", PASSWORD).await;
    pair.register(Rs, "nfr_b", "nfrb@x.org", PASSWORD).await;
    let token = pair.token(Rs, "nfr_a", PASSWORD).await;
    pair.patch_me(
        Rs,
        &token,
        r#"{"display_name":"x","bio":"y","locked":true}"#,
    )
    .await;
    pair.me(Rs, &token).await;
    pair.lookup(Rs, "nfr_b").await;
    pair.sign_in(Rs, "nfr_a", "wrong-password").await;
    // wrong routes and bad bodies too
    pair.send(Rs, &Req::get("/api/v1/nothing")).await;
    pair.send(Rs, &Req::post_json("/api/v1/accounts", "{"))
        .await;
    assert_eq!(
        pair.schema_fingerprint().await,
        before,
        "Rust changed the schema"
    );
    // the same workload on Python leaves it equal too, so the fingerprint is a real baseline
    pair.register(Py, "nfr_c", "nfrc@x.org", PASSWORD).await;
    pair.token(Py, "nfr_c", PASSWORD).await;
    assert_eq!(pair.schema_fingerprint().await, before);
    // no -wal, -shm or -journal file was left next to the database
    assert_eq!(pair.files_next_to_db(), ["shared.db"]);
    // every table Rust touches has the columns it needs, and no table was added
    let tables = pair
        .rows("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .await;
    let tables: Vec<String> = tables.into_iter().map(|r| r[0].clone().unwrap()).collect();
    assert!(["accounts", "access_tokens", "follows", "statuses"]
        .iter()
        .all(|t| tables.iter().any(|n| n == t)));
}

// ---- the real binary on the shared file ----

#[tokio::test]
async fn the_real_binary_and_python_share_one_database() {
    let pair = SharedPair::new_with_binary().await;
    pair.register(Py, "binpy", "binpy@x.org", PASSWORD).await;
    pair.register(Rs, "binrs", "binrs@x.org", PASSWORD).await;
    assert_eq!(pair.sign_in(Rs, "binpy", PASSWORD).await.status, 200);
    assert_eq!(pair.sign_in(Py, "binrs", PASSWORD).await.status, 200);
    let token = pair.token(Rs, "binpy", PASSWORD).await;
    assert_eq!(pair.me(Py, &token).await.json()["username"], "binpy");
    let token = pair.token(Py, "binrs", PASSWORD).await;
    assert_eq!(pair.me(Rs, &token).await.json()["username"], "binrs");
    let created = pair
        .rows("SELECT created_at FROM accounts ORDER BY id")
        .await;
    for row in created {
        assert!(is_stored_datetime(row[0].as_deref().unwrap()), "{row:?}");
    }
}
