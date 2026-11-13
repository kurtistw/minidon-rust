//! Wave 5 parity tests, one per rule BR-001..BR-014 except BR-007 (random salt: UNIT plus COMPAT,
//! see test-plan section 4), plus SC-ACC-028 and SC-ACC-029 (PT-ACC-001..014).
//!
//! Every test sends the same requests to the Python oracle and to the Rust build, each on its
//! own copy of one Python-made database, with a fixed clock at T0 = 2026-01-01 00:00:00 UTC.
//! Each test also asserts the expected status and body facts on the PYTHON answer, so it can be
//! run against the oracle alone (`PARITY_PYTHON_ONLY=1`) before Rust is compared. Expected
//! values come from running the oracle, not from reading its source.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod parity;

use parity::{dataset, Observed, Pair, Req, Tokens, PASSWORD, T0};
use serde_json::{json, Value};
use time::macros::datetime;
use time::Duration;

const USERNAME_MSG: &str = "Username must be 1-30 letters, digits or underscores";

fn error(message: &str) -> Value {
    json!({ "error": message })
}

async fn reg(pair: &Pair, user: &str, email: &str, password: &str, status: u16) -> Observed {
    let (py, _) = pair.register_with(user, email, password).await;
    assert_eq!(py.status, status, "{user:?} {email:?}: {}", py.body());
    py
}

async fn ok(pair: &Pair, user: &str) -> Observed {
    reg(pair, user, &format!("{user}@x.org"), PASSWORD, 201).await
}

async fn me(pair: &Pair, tokens: &Tokens, status: u16) -> Observed {
    pair.same_status(&Req::me(tokens), status).await.0
}

async fn patch(pair: &Pair, tokens: &Tokens, body: &str, status: u16) -> Observed {
    pair.same_status(&Req::patch_me(tokens, body), status)
        .await
        .0
}

// ---- BR-001 ----

#[tokio::test]
async fn pt_acc_001_usernames_are_validated_and_lower_cased() {
    let pair = Pair::new().await;
    let py = ok(&pair, "Alice_01").await;
    assert_eq!(py.json()["username"], "alice_01");
    let long31 = "a".repeat(31);
    for (i, bad) in [
        "",
        &long31,
        "bad-name",
        "has space",
        "１２３",
        "Ålice",
        "用户",
        "İstanbul",
        "a.b",
        "a@b",
        "名前",
    ]
    .iter()
    .enumerate()
    {
        let py = reg(&pair, bad, &format!("bad{i}@x.org"), PASSWORD, 422).await;
        assert_eq!(py.json(), error(USERNAME_MSG), "{bad:?}");
    }
    let py = reg(&pair, &"b".repeat(30), "b30@x.org", PASSWORD, 201).await;
    assert_eq!(py.json()["username"].as_str().unwrap().len(), 30);
    // SC-ACC-003: the Kelvin sign lower-cases to ASCII k and is accepted.
    let py = reg(&pair, "\u{212A}elvin", "k@x.org", PASSWORD, 201).await;
    assert_eq!(py.json()["username"], "kelvin");
    // a final sigma and upper-case non-ASCII fold the way Python does
    for (i, name) in ["ΑΣ", "ǅ", "ß", "ǈ"].iter().enumerate() {
        pair.register_with(name, &format!("fold{i}@x.org"), PASSWORD)
            .await;
    }
    // login and lookup in other cases
    pair.same_status(&Req::lookup("ALICE_01"), 200).await;
    // the Kelvin sign in the path folds to k as well
    pair.same_status(&Req::lookup("%E2%84%AAELVIN"), 200).await;
    pair.login("ALICE_01", PASSWORD).await;
    assert!(pair
        .db_same("SELECT group_concat(username, '|') FROM accounts")
        .await
        .unwrap()
        .starts_with("alice_01|"));
}

// ---- BR-002 ----

#[tokio::test]
async fn pt_acc_002_email_needs_an_at_sign_and_is_trimmed_and_lower_cased() {
    let pair = Pair::new().await;
    let py = reg(&pair, "u0", "not-an-email", PASSWORD, 422).await;
    assert_eq!(py.json(), error("Email address is not valid"));
    reg(&pair, "u1", "  Bob@Example.COM ", PASSWORD, 201).await;
    reg(&pair, "u2", "@", PASSWORD, 201).await;
    reg(&pair, "u3", "ÄÖ@Ü.de", PASSWORD, 201).await;
    reg(&pair, "u4", "\u{a0}nb@x.org\u{a0}", PASSWORD, 201).await;
    reg(&pair, "u5", "\u{1c}fs@x.org\u{1f}", PASSWORD, 201).await;
    reg(&pair, "u6", "\u{2003}em@x.org\u{3000}", PASSWORD, 201).await;
    reg(&pair, "u7", "İ@x.org", PASSWORD, 201).await;
    reg(&pair, "u8", "ΑΣ@x.org", PASSWORD, 201).await;
    reg(&pair, "u9", "\u{212A}@x.org", PASSWORD, 201).await;
    reg(&pair, "u10", "\u{200b}zw@x.org", PASSWORD, 201).await;
    let py = reg(&pair, "u11", " \t ", PASSWORD, 422).await;
    assert_eq!(py.json(), error("Email address is not valid"));
    let emails = pair
        .db_same("SELECT group_concat(email, '|') FROM accounts ORDER BY id")
        .await
        .unwrap();
    // Stored lower-cased and trimmed with Python's whitespace set (no-break space, U+001C,
    // U+001F, em space and ideographic space go; the zero width space stays). Sorted by the
    // unique index SQLite scans. `i` plus U+0307 is Python's lower-case of the dotted capital I.
    let expected = [
        "@",
        "bob@example.com",
        "em@x.org",
        "fs@x.org",
        "i\u{307}@x.org",
        "k@x.org",
        "nb@x.org",
        "äö@ü.de",
        "ας@x.org",
        "\u{200b}zw@x.org",
    ]
    .join("|");
    assert_eq!(emails, expected);
    // email is checked after username, password after email
    let py = reg(&pair, "bad name", "nope", "short", 422).await;
    assert_eq!(py.json(), error(USERNAME_MSG));
    let py = reg(&pair, "ok_name", "nope", "short", 422).await;
    assert_eq!(py.json(), error("Email address is not valid"));
}

// ---- BR-003 ----

#[tokio::test]
async fn pt_acc_003_password_length_counts_code_points() {
    let pair = Pair::new().await;
    let msg = "Password must be at least 8 characters";
    for (i, (pw, status)) in [
        ("1234567", 422),
        ("12345678", 201),
        ("😀😀😀😀😀😀😀", 422),
        ("😀😀😀😀😀😀😀😀", 201),
        ("ééééééé", 422),
        ("éééééééé", 201),
        ("e\u{301}e\u{301}e\u{301}e\u{301}", 201), // 8 code points, 4 graphemes
        ("       ", 422),
        ("        ", 201), // spaces are not trimmed
        ("\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}", 201),
    ]
    .iter()
    .enumerate()
    {
        let py = reg(
            &pair,
            &format!("pw{i}"),
            &format!("pw{i}@x.org"),
            pw,
            *status,
        )
        .await;
        if *status == 422 {
            assert_eq!(py.json(), error(msg), "{pw:?}");
        }
    }
    // A password with a NUL, emoji or spaces still signs in, byte for byte.
    pair.login("pw1", "12345678").await;
    pair.login("pw3", "😀😀😀😀😀😀😀😀").await;
    pair.login("pw8", "        ").await;
    pair.login("pw9", "\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}")
        .await;
    pair.same_status(&Req::sign_in("pw8", "       "), 401).await;
}

// ---- BR-004 ----

#[tokio::test]
async fn pt_acc_004_username_is_unique_case_insensitively_suspended_included() {
    let pair = Pair::new().await;
    ok(&pair, "carol").await;
    ok(&pair, "dave").await;
    pair.sql(&dataset::suspend("dave")).await;
    for name in ["CAROL", "Carol", "carol", "dave", "DAVE"] {
        let py = reg(&pair, name, &format!("new-{name}@x.org"), PASSWORD, 409).await;
        assert_eq!(py.json(), error("Username is taken"), "{name}");
    }
    // The Kelvin sign folds onto an existing k name.
    ok(&pair, "kim").await;
    let py = reg(&pair, "\u{212A}im", "kk@x.org", PASSWORD, 409).await;
    assert_eq!(py.json(), error("Username is taken"));
    // username is checked before email
    let py = reg(&pair, "carol", "carol@x.org", PASSWORD, 409).await;
    assert_eq!(py.json(), error("Username is taken"));
}

// ---- BR-005 ----

#[tokio::test]
async fn pt_acc_005_email_is_unique_trimmed_lower_cased_suspended_included() {
    let pair = Pair::new().await;
    reg(&pair, "erin", "erin@x.org", PASSWORD, 201).await;
    reg(&pair, "frank", "frank@x.org", PASSWORD, 201).await;
    pair.sql(&dataset::suspend("frank")).await;
    for (i, email) in [
        "ERIN@x.org",
        " erin@x.org ",
        "\u{a0}Erin@X.ORG",
        "frank@x.org",
        "FRANK@X.ORG",
    ]
    .iter()
    .enumerate()
    {
        let py = reg(&pair, &format!("new{i}"), email, PASSWORD, 409).await;
        assert_eq!(py.json(), error("Email is already registered"), "{email:?}");
    }
}

// ---- BR-006 ----

#[tokio::test]
async fn pt_acc_006_first_account_is_admin_and_the_rest_are_users() {
    let pair = Pair::new().await;
    let first = ok(&pair, "first").await;
    let second = ok(&pair, "second").await;
    assert_eq!(
        (first.json()["role"].clone(), second.json()["role"].clone()),
        (json!("admin"), json!("user"))
    );
    let tokens = pair.login("first", PASSWORD).await;
    assert_eq!(me(&pair, &tokens, 200).await.json()["role"], "admin");
    assert_eq!(
        pair.db_same("SELECT group_concat(role) FROM accounts")
            .await
            .unwrap(),
        "ADMIN,USER"
    );
    // SC-ACC-012: the count is over all accounts, so a dataset that is all suspended still
    // gives a user.
    pair.sql("UPDATE accounts SET suspended_at = '2026-01-01 00:00:00.000000'")
        .await;
    let third = ok(&pair, "third").await;
    assert_eq!(third.json()["role"], "user");
    // role stored by Python for a moderator is read back by both
    pair.sql(&dataset::set_role("second", "MODERATOR")).await;
    pair.sql(&dataset::unsuspend("second")).await;
    assert_eq!(
        pair.same_status(&Req::lookup("second"), 200).await.0.json()["role"],
        "moderator"
    );
}

// ---- BR-008 ----

#[tokio::test]
async fn pt_acc_008_wrong_credentials_look_the_same_and_never_lock_out() {
    let pair = Pair::new().await;
    ok(&pair, "gina").await;
    let bad = "Invalid username or password";
    let wrong = pair
        .same_status(&Req::sign_in("gina", "wrong-password"), 401)
        .await
        .0;
    let unknown = pair
        .same_status(&Req::sign_in("nobody", "wrong-password"), 401)
        .await
        .0;
    assert_eq!(wrong.json(), error(bad));
    assert_eq!(unknown.body(), wrong.body());
    pair.same_status(&Req::sign_in("GINA", "wrong-password"), 401)
        .await;
    pair.same_status(&Req::sign_in("gina", ""), 401).await;
    pair.same_status(&Req::sign_in("", PASSWORD), 401).await;
    pair.same_status(&Req::sign_in("gina\n", PASSWORD), 401)
        .await;
    for i in 0..50 {
        pair.same_status(&Req::sign_in("gina", &format!("wrong-{i}")), 401)
            .await;
    }
    // no lockout: the right password still works after 50 failures
    pair.login("gina", PASSWORD).await;
}

// ---- BR-009 ----

#[tokio::test]
async fn pt_acc_009_suspended_accounts_cannot_sign_in() {
    let pair = Pair::new().await;
    ok(&pair, "hank").await;
    let tokens = pair.login("hank", PASSWORD).await;
    pair.sql(&dataset::suspend("hank")).await;
    let py = pair
        .same_status(&Req::sign_in("hank", PASSWORD), 403)
        .await
        .0;
    assert_eq!(py.json(), error("This account is suspended"));
    // the password is checked before the suspension: wrong password is 401, not 403
    let py = pair
        .same_status(&Req::sign_in("hank", "wrong-password"), 401)
        .await
        .0;
    assert_eq!(py.json(), error("Invalid username or password"));
    pair.same_status(&Req::sign_in("HANK", PASSWORD), 403).await;
    // and the token issued before is signed out on the very next request
    me(&pair, &tokens, 401).await;
    pair.sql(&dataset::unsuspend("hank")).await;
    me(&pair, &tokens, 200).await;
    pair.login("hank", PASSWORD).await;
}

// ---- BR-010 ----

#[tokio::test]
async fn pt_acc_010_token_lasts_30_days_with_an_exact_boundary() {
    let pair = Pair::new().await;
    ok(&pair, "ivy").await;
    let (py, _) = pair.same_status(&Req::sign_in("ivy", PASSWORD), 200).await;
    assert_eq!(py.json()["token_type"], "bearer");
    assert_eq!(py.json()["expires_at"], "2026-01-31T00:00:00");
    assert_eq!(py.json()["access_token"].as_str().unwrap().len(), 43);
    let keys = py.body();
    assert!(
        keys.starts_with(r#"{"access_token":""#)
            && keys.ends_with(r#","token_type":"bearer","expires_at":"2026-01-31T00:00:00"}"#),
        "{keys}"
    );
    let tokens = pair.login("ivy", PASSWORD).await;
    assert_ne!(tokens.py.len(), 0);
    let expiry = T0 + Duration::days(30);
    pair.set_clock(expiry - Duration::seconds(1)).await;
    me(&pair, &tokens, 200).await;
    pair.set_clock(expiry).await;
    me(&pair, &tokens, 401).await;
    pair.set_clock(expiry + Duration::seconds(1)).await;
    me(&pair, &tokens, 401).await;
    // microsecond boundary with a fraction in the clock
    let start = datetime!(2026-02-10 08:00:00.250000);
    pair.set_clock(start).await;
    let (py, _) = pair.same_status(&Req::sign_in("ivy", PASSWORD), 200).await;
    assert_eq!(py.json()["expires_at"], "2026-03-12T08:00:00.250000");
    let fresh = pair.login("ivy", PASSWORD).await;
    let end = start + Duration::days(30);
    pair.set_clock(end - Duration::microseconds(1)).await;
    me(&pair, &fresh, 200).await;
    pair.set_clock(end).await;
    me(&pair, &fresh, 401).await;
    // the stored text is the same in both databases
    let stored = pair
        .db_same("SELECT group_concat(expires_at, '|') FROM access_tokens")
        .await
        .unwrap();
    assert!(stored.contains("2026-01-31 00:00:00.000000"), "{stored}");
    assert!(stored.contains("2026-03-12 08:00:00.250000"), "{stored}");
}

// ---- BR-011 ----

#[tokio::test]
async fn pt_acc_011_unusable_tokens_are_signed_out() {
    let pair = Pair::new().await;
    ok(&pair, "jay").await;
    ok(&pair, "kim").await;
    let jay = pair.login("jay", PASSWORD).await;
    let kim = pair.login("kim", PASSWORD).await;
    // missing, unknown, malformed
    let py = pair
        .same_status(&Req::get("/api/v1/accounts/me"), 401)
        .await
        .0;
    assert_eq!(py.json(), error("Sign in required"));
    let nope = Tokens {
        py: "x".repeat(43),
        rs: "x".repeat(43),
    };
    me(&pair, &nope, 401).await;
    for literal in [
        &b"Bearer"[..],
        b"Bearer ",
        b"Basic abc",
        b"abc",
        b"",
        b"Bearer  ",
        b"Bearer a b",
        b"Bearer \xe9\xff",
    ] {
        pair.same_status(&Req::get("/api/v1/accounts/me").auth_literal(literal), 401)
            .await;
    }
    // suspended account (SC-ACC-021) and deleted account row
    pair.sql(&dataset::suspend("jay")).await;
    me(&pair, &jay, 401).await;
    pair.same_status(&Req::get("/api/v1/accounts/kim").bearer(&jay), 200)
        .await;
    pair.sql(&dataset::delete_account("kim")).await;
    me(&pair, &kim, 401).await;
    // expired token is signed out, and a public route still answers (SC-ACC-020)
    pair.sql(&dataset::unsuspend("jay")).await;
    me(&pair, &jay, 200).await;
    pair.set_clock(T0 + Duration::days(31)).await;
    me(&pair, &jay, 401).await;
    pair.same_status(&Req::get("/api/v1/accounts/jay").bearer(&jay), 200)
        .await;
}

#[tokio::test]
async fn pt_acc_011_authorization_header_shapes() {
    let pair = Pair::new().await;
    ok(&pair, "jay").await;
    let tok = pair.login("jay", PASSWORD).await;
    let cases: [(&[u8], &[u8], u16); 16] = [
        (b"Bearer ", b"", 200),
        (b"bearer ", b"", 200),
        (b"BEARER ", b"", 200),
        (b"bEaReR ", b"", 200),
        (b"Bearer   ", b"  ", 200),
        (b"Bearer\t", b"", 401),
        (b"Bearer ", b"\xa0", 200),
        (b"Bearer \xa0", b"", 200),
        (b"Bearer ", b"\x85", 200),
        (b"Bearer \xa0\x85", b"\x85\xa0", 200),
        (b" Bearer ", b"", 200),
        (b"Bearer ", b" extra", 401),
        (b"Bearer ", b"x", 401),
        (b"Bearer \xe9", b"", 401),
        (b"Token ", b"", 401),
        (b"", b"", 401),
    ];
    for (prefix, suffix, status) in cases {
        let req = Req::get("/api/v1/accounts/me").auth_around(prefix, &tok, suffix);
        let label = format!("auth {prefix:?} token {suffix:?}");
        pair.same_status(&req.label(&label), status).await;
    }
    // The first Authorization header wins.
    let bad = b"Bearer nope".as_slice();
    pair.same_status(
        &Req::get("/api/v1/accounts/me")
            .bearer(&tok)
            .auth_literal(bad)
            .label("good then bad"),
        200,
    )
    .await;
    pair.same_status(
        &Req::get("/api/v1/accounts/me")
            .auth_literal(bad)
            .bearer(&tok)
            .label("bad then good"),
        401,
    )
    .await;
    // The public lookup does not read the header at all.
    for literal in [&b"Bearer nonsense"[..], b"Bearer \xff\xfe", b"junk"] {
        pair.same_status(&Req::get("/api/v1/accounts/jay").auth_literal(literal), 200)
            .await;
    }
}

// ---- BR-012 ----

#[tokio::test]
async fn pt_acc_012_suspended_accounts_look_not_found() {
    let pair = Pair::new().await;
    ok(&pair, "kim").await;
    ok(&pair, "lou").await;
    pair.sql(&dataset::suspend("lou")).await;
    let py = pair.same_status(&Req::lookup("kim"), 200).await.0;
    assert_eq!(py.json()["username"], "kim");
    for name in ["lou", "LOU", "Lou", "nobody"] {
        let py = pair.same_status(&Req::lookup(name), 404).await.0;
        assert_eq!(py.json(), error("Account not found"), "{name}");
    }
    // the query string and a trailing question mark do not change the lookup
    pair.same_status(&Req::get("/api/v1/accounts/kim?x=1"), 200)
        .await;
    pair.same_status(&Req::get("/api/v1/accounts/KIM?"), 200)
        .await;
    // the suspended account still reserves its name (BR-004) and keeps its email (BR-005)
    pair.same_status(&Req::sign_in("lou", PASSWORD), 403).await;
}

// ---- BR-013 ----

#[tokio::test]
async fn pt_acc_013_display_name_and_bio_limits_count_code_points() {
    let pair = Pair::new().await;
    ok(&pair, "mia").await;
    let tokens = pair.login("mia", PASSWORD).await;
    let dn = "Display name must be at most 30 characters";
    let bio = "Bio must be at most 500 characters";
    let set = |field: &str, value: &str| json!({ field: value }).to_string();
    patch(&pair, &tokens, &set("display_name", &"a".repeat(30)), 200).await;
    let py = patch(&pair, &tokens, &set("display_name", &"a".repeat(31)), 422).await;
    assert_eq!(py.json(), error(dn));
    patch(&pair, &tokens, &set("bio", &"b".repeat(500)), 200).await;
    let py = patch(&pair, &tokens, &set("bio", &"b".repeat(501)), 422).await;
    assert_eq!(py.json(), error(bio));
    // SC-ACC-025: code points, not bytes or graphemes
    let combining = format!("{}{}", "a".repeat(15), "\u{301}".repeat(15));
    for (unit, ok_value) in [
        ("😀", "😀".repeat(30)),
        ("用", "用".repeat(30)),
        ("c", combining.clone()),
    ] {
        patch(&pair, &tokens, &set("display_name", &ok_value), 200).await;
        let over = format!("{ok_value}{unit}");
        let py = patch(&pair, &tokens, &set("display_name", &over), 422).await;
        assert_eq!(py.json(), error(dn));
    }
    patch(&pair, &tokens, &set("bio", &"😀".repeat(500)), 200).await;
    patch(&pair, &tokens, &set("bio", &"😀".repeat(501)), 422).await;
    // empty strings are values, not "leave alone"
    let py = patch(&pair, &tokens, r#"{"display_name":"","bio":""}"#, 200).await;
    assert_eq!(
        (py.json()["display_name"].clone(), py.json()["bio"].clone()),
        (json!(""), json!(""))
    );
    // a control character and whitespace are kept as they are
    patch(&pair, &tokens, &set("display_name", "  tab\there\n"), 200).await;
    assert_eq!(
        pair.db_same("SELECT display_name FROM accounts")
            .await
            .unwrap(),
        "  tab\there\n"
    );
    // m-12: a valid display name next to an invalid bio changes nothing
    patch(&pair, &tokens, r#"{"display_name":"keep"}"#, 200).await;
    let both = json!({"display_name": "changed", "bio": "x".repeat(501)}).to_string();
    patch(&pair, &tokens, &both, 422).await;
    assert_eq!(me(&pair, &tokens, 200).await.json()["display_name"], "keep");
    // the display name rule runs first when both are invalid
    let both = json!({"display_name": "d".repeat(31), "bio": "x".repeat(501)}).to_string();
    let py = patch(&pair, &tokens, &both, 422).await;
    assert_eq!(py.json(), error(dn));
}

#[tokio::test]
async fn pt_acc_013_partial_update_leaves_other_fields_alone() {
    let pair = Pair::new().await;
    ok(&pair, "mia").await;
    let tokens = pair.login("mia", PASSWORD).await;
    patch(
        &pair,
        &tokens,
        r#"{"display_name":"Mia","bio":"hello","locked":true}"#,
        200,
    )
    .await;
    let py = patch(&pair, &tokens, r#"{"bio":"only bio"}"#, 200).await;
    let body = py.json();
    assert_eq!(
        (
            body["display_name"].clone(),
            body["bio"].clone(),
            body["locked"].clone()
        ),
        (json!("Mia"), json!("only bio"), json!(true))
    );
    // omitted and null both leave a field alone (SC-ACC-026, confirmed on Python)
    for body in [
        r#"{}"#,
        r#"{"display_name":null}"#,
        r#"{"bio":null,"locked":null}"#,
        r#"{"display_name":null,"bio":null,"locked":null}"#,
        r#"{"unknown":1}"#,
    ] {
        let py = patch(&pair, &tokens, body, 200).await;
        let now = py.json();
        assert_eq!(
            (
                now["display_name"].clone(),
                now["bio"].clone(),
                now["locked"].clone()
            ),
            (json!("Mia"), json!("only bio"), json!(true)),
            "{body}"
        );
    }
}

// ---- BR-014 ----

#[tokio::test]
async fn pt_acc_014_locked_flag_round_trips_with_pydantic_lax_booleans() {
    let pair = Pair::new().await;
    ok(&pair, "nia").await;
    let tokens = pair.login("nia", PASSWORD).await;
    assert_eq!(
        pair.same_status(&Req::lookup("nia"), 200).await.0.json()["locked"],
        false
    );
    let py = patch(&pair, &tokens, r#"{"locked":true}"#, 200).await;
    assert_eq!(py.json()["locked"], true);
    assert_eq!(
        pair.same_status(&Req::lookup("nia"), 200).await.0.json()["locked"],
        true
    );
    assert_eq!(
        pair.db_same("SELECT CAST(locked AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "1"
    );
    for (value, expect) in [
        ("false", false),
        ("true", true),
        ("0", false),
        ("1", true),
        ("1.0", true),
        ("0.0", false),
        ("\"yes\"", true),
        ("\"no\"", false),
        ("\"on\"", true),
        ("\"off\"", false),
        ("\"t\"", true),
        ("\"f\"", false),
        ("\"y\"", true),
        ("\"n\"", false),
        ("\"TRUE\"", true),
        ("\"False\"", false),
        ("\"1\"", true),
        ("\"0\"", false),
        ("1e0", true),
        ("-0.0", false),
        ("0e5", false),
    ] {
        let py = patch(&pair, &tokens, &format!(r#"{{"locked":{value}}}"#), 200).await;
        assert_eq!(py.json()["locked"], expect, "{value}");
    }
    for bad in [
        "2",
        "-1",
        "0.5",
        "\" true \"",
        "\"maybe\"",
        "\"\"",
        "[]",
        "{}",
        "\"truee\"",
        "[true]",
        "1.5",
        "10000000000000000000000",
    ] {
        let (py, _) = pair
            .same(&Req::patch_me(&tokens, &format!(r#"{{"locked":{bad}}}"#)))
            .await;
        assert_eq!(py.status, 422, "{bad}");
    }
    // text fields are strict: numbers and booleans are not strings
    for bad in [
        r#"{"display_name":5}"#,
        r#"{"bio":true}"#,
        r#"{"display_name":["a"]}"#,
        r#"{"bio":1.5}"#,
        r#"{"display_name":{}}"#,
    ] {
        patch(&pair, &tokens, bad, 422).await;
    }
}

// ---- SC-ACC-028: framework level errors ----

#[tokio::test]
async fn pt_acc_028_framework_errors_on_the_accounts_endpoints() {
    let pair = Pair::new().await;
    ok(&pair, "alice").await;
    let tokens = pair.login("alice", PASSWORD).await;
    let post = |body: &str| Req::post_json("/api/v1/accounts", body);
    // 422 `{"detail": [...]}`: status and top-level shape are contract, the text is not
    for body in [
        "{bad",
        "",
        "[]",
        "null",
        "5",
        "\"text\"",
        "{}",
        r#"{"username":"a"}"#,
        r#"{"username":1,"email":"a@x.org","password":"password1"}"#,
        r#"{"username":null,"email":"a@x.org","password":"password1"}"#,
        r#"{"username":"a","email":["x"],"password":"password1"}"#,
        r#"{"username":"a","email":"a@x.org","password":{}}"#,
        r#"{"username":"abc","email":"a@x.org","password":"password1""#,
        r#"{"username":"abc","email":"a@x.org","password":"password1"} x"#,
        r#"{"username":"abc","email":"a@x.org","password":"password1",}"#,
        r#"{'username':'abc'}"#,
    ] {
        let (py, _) = pair.same(&post(body)).await;
        assert_eq!(py.status, 422, "{body}");
        assert!(py.json()["detail"].is_array(), "{body}: {}", py.body());
    }
    // content types
    let good = br#"{"username":"ct","email":"ct@x.org","password":"password1"}"#;
    for (i, ct) in [
        None,
        Some(&b"text/plain"[..]),
        Some(b"application/x-www-form-urlencoded"),
        Some(b"text/json"),
        Some(b"application/jsonx"),
        Some(b"application/xml"),
    ]
    .into_iter()
    .enumerate()
    {
        let req = Req::new("POST", "/api/v1/accounts")
            .raw_body(ct, good)
            .label(&format!("content-type case {i}"));
        let (py, _) = pair.same(&req).await;
        assert_eq!(py.status, 422, "content type {ct:?}");
    }
    for (i, ct) in [
        &b"application/json; charset=latin-1"[..],
        b"application/vnd.api+json",
        b"APPLICATION/JSON",
        b"application/json;charset=utf-8",
        b"Application/Json ; x=y",
        b"application/problem+json",
    ]
    .into_iter()
    .enumerate()
    {
        let body =
            format!(r#"{{"username":"ct{i}","email":"ct{i}@x.org","password":"password1"}}"#);
        let req = Req::new("POST", "/api/v1/accounts")
            .raw_body(Some(ct), body.as_bytes())
            .label(&format!("json content type {i}"));
        let (py, _) = pair.same(&req).await;
        assert_eq!(py.status, 201, "content type {ct:?}: {}", py.body());
    }
    // duplicate keys: the last wins; extra fields are ignored
    let (py, _) = pair.same(&post(r#"{"username":"first","email":"f@x.org","password":"password1","username":"second","role":"admin","x":[1]}"#)).await;
    assert_eq!(
        (py.status, py.json()["username"].clone()),
        (201, json!("second"))
    );
    // JSON escapes decode to the same text
    let (py, _) = pair
        .same(&post(
            r#"{"username":"Esc","email":"esc@x.org","password":"password1"}"#,
        ))
        .await;
    assert_eq!(
        (py.status, py.json()["username"].clone()),
        (201, json!("esc"))
    );
    // Whitespace around the document is fine
    let (py, _) = pair
        .same(&post(
            " \n\t{\"username\":\"ws\",\"email\":\"ws@x.org\",\"password\":\"password1\"}\r\n ",
        ))
        .await;
    assert_eq!(py.status, 201);
    // PATCH check order: syntax, then auth, then field types
    let patch_raw = |body: &str| Req::patch_json("/api/v1/accounts/me", body);
    assert_eq!(pair.same(&patch_raw("{bad")).await.0.status, 422);
    assert_eq!(
        pair.same(&patch_raw(r#"{"display_name":5}"#))
            .await
            .0
            .status,
        401
    );
    assert_eq!(pair.same(&patch_raw("[]")).await.0.status, 401);
    assert_eq!(pair.same(&patch_raw("")).await.0.status, 401);
    assert_eq!(
        pair.same(&Req::new("PATCH", "/api/v1/accounts/me").raw_body(None, b"{}"))
            .await
            .0
            .status,
        401
    );
    assert_eq!(
        pair.same(&Req::patch_me(&tokens, r#"{"display_name":5}"#))
            .await
            .0
            .status,
        422
    );
    assert_eq!(pair.same(&Req::patch_me(&tokens, "")).await.0.status, 422);
    assert_eq!(
        pair.same(&Req::patch_me(&tokens, "{bad")).await.0.status,
        422
    );
    // sign-in framework errors
    for body in [
        "{}",
        r#"{"username":"a"}"#,
        r#"{"password":"x"}"#,
        "[]",
        "{bad",
        "",
    ] {
        assert_eq!(
            pair.same(&Req::post_json("/api/v1/auth/token", body))
                .await
                .0
                .status,
            422,
            "{body}"
        );
    }
}

#[tokio::test]
async fn pt_acc_028_unknown_routes_wrong_methods_and_paths() {
    let pair = Pair::new().await;
    ok(&pair, "alice").await;
    let json_body = br#"{}"#.as_slice();
    // 404 and 405 bodies, Allow headers (first matching route), HEAD and OPTIONS
    let cases: &[(&str, &str, u16)] = &[
        ("GET", "/", 404),
        ("GET", "/nope", 404),
        ("GET", "/api", 404),
        ("GET", "/api/v1", 404),
        ("GET", "/api/v1/accounts/a/b", 404),
        ("POST", "/nope", 404),
        ("HEAD", "/nope", 404),
        ("OPTIONS", "/nope", 404),
        ("GET", "//api/v1/accounts", 404),
        ("GET", "/api/v1/accounts/ali%2Fce", 404),
        ("GET", "/api/v1/accounts/%2F", 307),
        ("GET", "/API/V1/ACCOUNTS/alice", 404),
        ("GET", "/api/v1/accounts/me/x", 404),
        ("HEAD", "/api/v1/accounts/alice", 405),
        ("OPTIONS", "/api/v1/accounts/alice", 405),
        ("POST", "/api/v1/accounts/me", 405),
        ("PUT", "/api/v1/accounts/me", 405),
        ("DELETE", "/api/v1/accounts/me", 405),
        ("HEAD", "/api/v1/accounts/me", 405),
        ("DELETE", "/api/v1/auth/token", 405),
        ("GET", "/api/v1/auth/token", 405),
        ("GET", "/api/v1/accounts", 405),
        ("PUT", "/api/v1/accounts", 405),
        ("HEAD", "/api/v1/accounts", 405),
        ("PATCH", "/api/v1/accounts/alice", 405),
        ("PUT", "/api/v1/accounts/alice", 405),
        ("POST", "/api/v1/accounts/alice", 405),
        ("DELETE", "/api/v1/accounts/alice", 405),
        ("OPTIONS", "/api/v1/accounts", 405),
        ("PATCH", "/api/v1/accounts", 405),
        ("TRACE", "/api/v1/accounts/alice", 405),
    ];
    for (method, path, status) in cases {
        let req = Req::new(method, path).raw_body(Some(b"application/json"), json_body);
        let req = if *method == "GET" || *method == "HEAD" {
            Req::new(method, path)
        } else {
            req
        };
        let (py, _) = pair.same(&req).await;
        assert_eq!(py.status, *status, "{method} {path}");
        if *status == 404 && *method != "HEAD" {
            assert_eq!(py.json(), json!({"detail": "Not Found"}), "{method} {path}");
        }
        if *status == 405 && *method != "HEAD" {
            assert_eq!(
                py.json(),
                json!({"detail": "Method Not Allowed"}),
                "{method} {path}"
            );
        }
    }
}

#[tokio::test]
async fn pt_acc_028_documentation_routes_are_out_of_contract() {
    // OP-06: `/docs`, `/redoc` and `/openapi.json` are generated by FastAPI, are not contract
    // and are not ported. Python serves them, Rust answers 404. Pinned, not hidden.
    let pair = Pair::new().await;
    for path in ["/docs", "/redoc", "/openapi.json", "/docs/oauth2-redirect"] {
        pair.out_of_contract(&Req::get(path), "OP-06-docs", 200, 404)
            .await;
    }
}

#[tokio::test]
async fn pt_acc_028_trailing_slashes_and_percent_encoded_paths() {
    let pair = Pair::new().await;
    ok(&pair, "alice").await;
    // Trailing slash: Starlette redirects with 307 when the toggled path matches a route.
    let redirects: &[(&str, &str)] = &[
        ("GET", "/api/v1/accounts/alice/"),
        ("GET", "/api/v1/accounts/me/"),
        ("POST", "/api/v1/accounts/"),
        ("PUT", "/api/v1/accounts/"),
        ("PATCH", "/api/v1/accounts/me/"),
        ("POST", "/api/v1/auth/token/"),
        ("GET", "/api/v1/auth/token/"),
        ("GET", "/api/v1/accounts//"),
        ("GET", "/api/v1/accounts/alice//"),
        ("GET", "/api/v1/accounts/alice///"),
        ("GET", "/api/v1/accounts/alice/?x=1&y=%20z"),
        ("GET", "/api/v1/accounts/alice%0A/"),
        ("GET", "/api/v1/accounts/%E7%94%A8/"),
        ("GET", "/api/v1/accounts/al%69ce/"),
        ("GET", "/api/v1/accounts/a%20b/"),
        ("GET", "/api/v1/accounts/a%25b/"),
        ("GET", "/api/v1%2Faccounts/alice/"),
        ("DELETE", "/api/v1/accounts/alice/"),
        ("HEAD", "/api/v1/accounts/alice/"),
        ("GET", "/api/v1/accounts/alice/?"),
        ("GET", "/api/v1/accounts/%3F/"),
        ("GET", "/api/v1/accounts/%23/"),
        ("GET", "/api/v1/accounts/%5C/"),
        ("GET", "/api/v1/accounts/%22%3C%3E%60/"),
        ("GET", "/api/v1/accounts/%FF/"),
    ];
    for (method, path) in redirects {
        let (py, _) = pair.same(&Req::new(method, path)).await;
        assert_eq!(
            py.status,
            307,
            "{method} {path}: {}",
            py.describe_for_test()
        );
        assert!(py.header("location").is_some(), "{method} {path}");
    }
    // Slash toggles that match nothing stay 404.
    for path in [
        "/nope/",
        "/api/v1/accounts/a/b/",
        "/api/v1/accounts/ali%2Fce/",
        "/api/v1/",
        "/api/v1/accounts/me/x/",
        "//",
    ] {
        assert_eq!(pair.same(&Req::get(path)).await.0.status, 404, "{path}");
    }
    // Percent decoding happens before routing (uvicorn).
    for (path, status) in [
        ("/api/v1/accounts/%6De", 401),
        ("/api/v1%2Faccounts/alice", 200),
        ("/api/v1/accounts/al%69ce", 200),
        ("/api/v1/accounts/ALICE", 200),
        ("/api/v1/accounts/%61lice", 200),
        ("/api%2Fv1/accounts/alice", 200),
        ("/%61pi/v1/accounts/alice", 200),
        ("/api/v1/accounts/alice%00", 404),
        ("/api/v1/accounts/%FF", 404),
        ("/api/v1/accounts/ali+ce", 404),
        ("/api/v1/accounts/alice%3Fx", 404),
        ("/api/v1/accounts/alice%23", 404),
        ("/api/v1/accounts/%25", 404),
        ("/api/v1/accounts/%", 404),
        ("/api/v1/accounts/%zz", 404),
        ("/api/v1/accounts/%E7%94%A8", 404),
        ("/api/v1/accounts/alice?x=1", 200),
        ("/api/v1/accounts/alice?x=%FF", 200),
        ("/api/v1/accounts/alice;x", 404),
        ("/api/v1/accounts/ali%63e", 200),
        ("/api/v1/accounts/%61%6C%69%63%65", 200),
        ("/api/v1/accounts/%2e", 404),
        ("/api/v1/accounts/alice%20", 404),
    ] {
        let (py, _) = pair.same(&Req::get(path)).await;
        assert_eq!(py.status, status, "{path}: {}", py.body());
    }
}

// ---- SC-ACC-029: shape, dates and profile counts ----

#[tokio::test]
async fn pt_acc_029_response_shape_dates_and_counts() {
    let pair = Pair::new().await;
    let py = ok(&pair, "alice").await;
    assert_eq!(
        py.body(),
        r#"{"id":1,"username":"alice","acct":"alice@minidon.local","display_name":"","bio":"","locked":false,"role":"admin","created_at":"2026-01-01T00:00:00","followers_count":0,"following_count":0,"statuses_count":0}"#
    );
    // Dates with and without a fraction, through registration, sign-in and storage.
    pair.set_clock(datetime!(2026-05-06 07:08:09.1)).await;
    let py = ok(&pair, "bob").await;
    assert_eq!(py.json()["created_at"], "2026-05-06T07:08:09.100000");
    pair.set_clock(datetime!(2026-05-06 07:08:10)).await;
    let py = ok(&pair, "cy").await;
    assert_eq!(py.json()["created_at"], "2026-05-06T07:08:10");
    pair.set_clock(datetime!(2026-05-06 07:08:11.000001)).await;
    let py = ok(&pair, "di").await;
    assert_eq!(py.json()["created_at"], "2026-05-06T07:08:11.000001");
    let stored = pair
        .db_same("SELECT group_concat(created_at, '|') FROM accounts")
        .await
        .unwrap();
    assert_eq!(stored, "2026-01-01 00:00:00.000000|2026-05-06 07:08:09.100000|2026-05-06 07:08:10.000000|2026-05-06 07:08:11.000001");
    // Profile counts come from the follows and statuses tables (review M-02). Accounts: alice 1,
    // bob 2, cy 3, di 4.
    pair.sql(&dataset::follow(2, 1, true)).await; // bob follows alice (accepted)
    pair.sql(&dataset::follow(3, 1, false)).await; // cy asked, pending: not counted
    pair.sql(&dataset::follow(1, 2, true)).await; // alice follows bob
    pair.sql(&dataset::follow(1, 3, true)).await;
    pair.sql(&dataset::status(1, false, None)).await; // 1: counted
    pair.sql(&dataset::status(1, false, None)).await; // 2: counted
    pair.sql(&dataset::status(1, true, None)).await; // 3: deleted, not counted
    pair.sql(&dataset::status(1, false, Some(1))).await; // 4: boost, not counted
    pair.sql(&dataset::status(2, false, None)).await;
    let tokens = pair.login("alice", PASSWORD).await;
    let alice = me(&pair, &tokens, 200).await.json();
    assert_eq!(
        (
            alice["followers_count"].clone(),
            alice["following_count"].clone(),
            alice["statuses_count"].clone()
        ),
        (json!(1), json!(2), json!(2))
    );
    pair.same_status(&Req::lookup("bob"), 200).await;
    let patched = patch(&pair, &tokens, r#"{"bio":"x"}"#, 200).await.json();
    assert_eq!(patched["following_count"], 2);
    // `acct` uses the instance domain, with the username as stored (even with a newline).
    pair.register_with("nl\n", "nl@x.org", PASSWORD).await;
    pair.same_status(&Req::lookup("nl%0A"), 200).await;
}

// ---- the real binary (not the in-process router) against the oracle ----

#[tokio::test]
async fn pt_acc_binary_smoke_the_real_binary_answers_like_the_oracle() {
    // System clock on the Rust side: ids and tokens are normalised, dates are checked for
    // format only.
    let pair = Pair::new_with_binary().await;
    pair.set_clock(T0).await;
    let py = ok(&pair, "alice").await;
    assert_eq!(py.json()["role"], "admin");
    let tokens = pair.login("alice", PASSWORD).await;
    me(&pair, &tokens, 200).await;
    patch(&pair, &tokens, r#"{"bio":"hi","locked":true}"#, 200).await;
    pair.same_status(&Req::lookup("ALICE"), 200).await;
    pair.same_status(&Req::lookup("nobody"), 404).await;
    pair.same_status(&Req::sign_in("alice", "wrong-password"), 401)
        .await;
    pair.same_status(&Req::get("/api/v1/accounts/alice/"), 307)
        .await;
    pair.same_status(&Req::new("DELETE", "/api/v1/accounts/me"), 405)
        .await;
    pair.same_status(&Req::post_json("/api/v1/accounts", "{bad"), 422)
        .await;
}

#[tokio::test]
async fn pt_acc_011_control_characters_in_the_header_are_an_unresolved_difference() {
    // UNRESOLVED U-01 (needs a human, no decision covers it). Python strips U+001C to U+001F
    // around the token (plan review m-04), and its HTTP parser (h11) lets control bytes through
    // except NUL, VT and FF (those are a 400 in both builds). hyper answers 400 for every control
    // byte except tab, before any Accounts code runs. So a token wrapped in U+001C..U+001F
    // authenticates on Python and is a 400 on Rust, and bytes 0x01 and 0x7f are a 401 on Python
    // and a 400 on Rust. The cause is the HTTP parser, not Accounts logic. Pinned per case so
    // it cannot change unseen. `token::parse_bearer` itself strips them (tests/unit_token.rs).
    let pair = Pair::new().await;
    ok(&pair, "jay").await;
    let tok = pair.login("jay", PASSWORD).await;
    let cases: [(&[u8], &[u8], u16, u16); 12] = [
        (b"Bearer \x1c", b"", 200, 400),
        (b"Bearer ", b"\x1f", 200, 400),
        (b"Bearer \x1d\x1e", b"\x1c\x1f", 200, 400),
        (b"Bearer ", b"\x1e", 200, 400),
        (b"Bearer ", b"\x01", 401, 400),
        (b"Bearer ", b"\x7f", 401, 400),
        (b"Bearer ", b"\x08", 401, 400),
        (b"Bearer ", b"\x1b", 401, 400),
        (b"Bearer ", b"\x00", 400, 400),
        (b"Bearer ", b"\x0b", 400, 400),
        (b"Bearer ", b"\x0c", 400, 400),
        (b"Bearer \x00", b"", 400, 400),
    ];
    for (prefix, suffix, py_status, rs_status) in cases {
        let req = Req::get("/api/v1/accounts/me").auth_around(prefix, &tok, suffix);
        let label = format!("control bytes {prefix:?} token {suffix:?}");
        pair.unresolved(
            &req.label(&label),
            "U-01-header-control-bytes",
            py_status,
            rs_status,
        )
        .await;
    }
    // Tab is the one control byte both accept, and both treat it as whitespace around the token.
    let req = Req::get("/api/v1/accounts/me").auth_around(b"Bearer ", &tok, b"\t");
    pair.same_status(&req.label("tab after the token"), 200)
        .await;
}
