//! Known differences from Python (STATED: adr-001 section 5 decisions 1, 2 and 6, test-plan
//! section 6b), each pinned by an exact input and exact statuses on BOTH builds, and the Python
//! behaviour that no decision covers, pinned as UNRESOLVED and reported for a human.
//!
//! Nothing here is a general whitelist. A pinned case fails if Python or Rust answers anything
//! else, so a difference cannot grow or shrink unseen. The cases next to each deviation that
//! must still MATCH Python are compared in full (`same`), which keeps the deviations narrow.
//!
//! Deviations (Python 500 or 2xx, Rust 4xx):
//! - D1 (decision 1): a lone-surrogate password. The out-of-range id half of decision 1 has no
//!   Accounts route (review m-05) and is pinned in the first story that has an id path.
//! - D2 (decision 2): a body above 1 MiB, Python accepts, Rust 413.
//! - D6 (decision 6): email, display name, bio with a lone surrogate that passed validation.
//!
//! Unresolved (Python's real behaviour, not anticipated by the spec or an ADR, see the build log):
//! - U-01: control bytes in a header value (in `parity_accounts.rs`).
//! - U-02: a sign-in USERNAME with a lone surrogate, Python 500, Rust 401.
//! - U-04: an email with a capital letter that Rust's newer Unicode tables lower-case and
//!   Python 3.12 does not (`unit_unicode_tables.rs` pins the 55 code points).
//! - U-03: JSON that Python's parser accepts and serde_json rejects (NaN and Infinity, a UTF-8
//!   BOM, UTF-16 bodies, numbers out of range), Python accepts, Rust 422.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod parity;

use parity::{dataset, Deviation, Pair, Req, PASSWORD};
use serde_json::json;

fn reg(username: &str, email: &str, password: &str) -> Req {
    // Raw JSON text, so a lone surrogate escape can be written.
    Req::post_json(
        "/api/v1/accounts",
        &format!(r#"{{"username":"{username}","email":"{email}","password":"{password}"}}"#),
    )
}

fn sign(username: &str, password: &str) -> Req {
    Req::post_json(
        "/api/v1/auth/token",
        &format!(r#"{{"username":"{username}","password":"{password}"}}"#),
    )
}

// ---- decision 1: lone-surrogate password ----

#[tokio::test]
async fn d1_a_lone_surrogate_password_is_a_500_on_python_and_a_422_on_rust() {
    let pair = Pair::new().await;
    pair.deviation(
        &reg("sur1", "s1@x.org", r"pass\ud800word"),
        Deviation::D1Password,
        500,
        422,
    )
    .await;
    pair.deviation(
        &reg(
            "sur2",
            "s2@x.org",
            r"\udc00\udc00\udc00\udc00\udc00\udc00\udc00\udc00",
        ),
        Deviation::D1Password,
        500,
        422,
    )
    .await;
    pair.deviation(
        &reg("sur3", "s3@x.org", r"ok-password-\ud83d"),
        Deviation::D1Password,
        500,
        422,
    )
    .await;
    // nothing was stored by either build
    assert_eq!(
        pair.db_same("SELECT CAST(COUNT(*) AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "0"
    );
    // sign-in of a KNOWN user with a lone-surrogate password
    pair.register("alice").await;
    pair.deviation(
        &sign("alice", r"pass\ud800word"),
        Deviation::D1Password,
        500,
        422,
    )
    .await;
    // also for a known suspended user: Python hashes before it looks at the suspension
    pair.register("hank").await;
    pair.sql(&dataset::suspend("hank")).await;
    pair.deviation(
        &sign("hank", r"pass\ud800word"),
        Deviation::D1Password,
        500,
        422,
    )
    .await;
    assert_eq!(
        pair.db_same("SELECT CAST(COUNT(*) AS TEXT) FROM access_tokens")
            .await
            .unwrap(),
        "0"
    );
    // a valid surrogate PAIR is an emoji and is not a deviation
    pair.same_status(&reg("pair", "pair@x.org", r"pass\ud83d\ude00word"), 201)
        .await;
    pair.login("pair", "pass😀word").await;
}

#[tokio::test]
async fn d1_neighbours_that_must_still_match_python() {
    let pair = Pair::new().await;
    pair.register("alice").await;
    // decision 6: an unknown user with a lone-surrogate password is 401 on both (no hash)
    let py = pair
        .same_status(&sign("nobody", r"pass\ud800word"), 401)
        .await
        .0;
    assert_eq!(py.json(), json!({"error": "Invalid username or password"}));
    // earlier rules still win: a short password with a surrogate is the 422 error body
    let py = pair
        .same_status(&reg("short", "s@x.org", r"p\ud800"), 422)
        .await
        .0;
    assert_eq!(
        py.json(),
        json!({"error": "Password must be at least 8 characters"})
    );
    // uniqueness is checked before the hash
    let py = pair
        .same_status(&reg("alice", "n1@x.org", r"pass\ud800word"), 409)
        .await
        .0;
    assert_eq!(py.json(), json!({"error": "Username is taken"}));
    let py = pair
        .same_status(&reg("fresh", "alice@x.org", r"pass\ud800word"), 409)
        .await
        .0;
    assert_eq!(py.json(), json!({"error": "Email is already registered"}));
    // invalid username and email come first
    pair.same_status(&reg(r"bad name", "b@x.org", r"pass\ud800word"), 422)
        .await;
    pair.same_status(&reg("okname", "no-at-sign", r"pass\ud800word"), 422)
        .await;
    // an unknown user: the surrogate sits in an ignored field
    pair.same_status(
        &Req::post_json(
            "/api/v1/auth/token",
            r#"{"username":"nobody","password":"x","z":"\ud800"}"#,
        ),
        401,
    )
    .await;
}

// ---- decision 6: username, email, display name, bio ----

#[tokio::test]
async fn d6_a_username_with_a_lone_surrogate_keeps_pythons_422_error_body() {
    let pair = Pair::new().await;
    for (i, name) in [
        r"sur\ud800",
        r"\udc00",
        r"a\ud800b",
        r"\ud800\ud800",
        r"sur\ud83d",
    ]
    .iter()
    .enumerate()
    {
        let py = pair
            .same_status(&reg(name, &format!("u{i}@x.org"), PASSWORD), 422)
            .await
            .0;
        assert_eq!(
            py.json(),
            json!({"error": "Username must be 1-30 letters, digits or underscores"}),
            "{name}"
        );
    }
    // not the generic `{"detail": ...}` body: the shape is contract (OP-06)
    let py = pair
        .same_status(&reg(r"x\ud800", "ux@x.org", PASSWORD), 422)
        .await
        .0;
    assert!(py.json().get("detail").is_none());
}

#[tokio::test]
async fn d6_an_email_with_a_lone_surrogate_is_a_500_on_python_and_a_422_on_rust() {
    let pair = Pair::new().await;
    pair.deviation(
        &reg("sur1", r"s\ud800@x.org", PASSWORD),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    pair.deviation(
        &reg("sur2", r"\udc00@x.org", PASSWORD),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    pair.deviation(
        &reg("sur3", r"a@\ud800", PASSWORD),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    assert_eq!(
        pair.db_same("SELECT CAST(COUNT(*) AS TEXT) FROM accounts")
            .await
            .unwrap(),
        "0"
    );
    // Python checks the username first, then the email rules, then the password, so these match
    pair.register("alice").await;
    let py = pair
        .same_status(&reg("alice", r"s\ud800@x.org", PASSWORD), 409)
        .await
        .0;
    assert_eq!(py.json(), json!({"error": "Username is taken"}));
    let py = pair
        .same_status(&reg("sur4", r"s\ud800.x.org", PASSWORD), 422)
        .await
        .0;
    assert_eq!(py.json(), json!({"error": "Email address is not valid"}));
    let py = pair
        .same_status(&reg("sur5", r"s\ud800@x.org", "short"), 422)
        .await
        .0;
    assert_eq!(
        py.json(),
        json!({"error": "Password must be at least 8 characters"})
    );
    // an ignored extra field and a key with a surrogate do not matter
    pair.same_status(
        &Req::post_json(
            "/api/v1/accounts",
            r#"{"username":"extra1","email":"e1@x.org","password":"password1","z":"\ud800"}"#,
        ),
        201,
    )
    .await;
    pair.same_status(
        &Req::post_json(
            "/api/v1/accounts",
            r#"{"username":"extra2","email":"e2@x.org","password":"password1","\ud800":1}"#,
        ),
        201,
    )
    .await;
    pair.same_status(&Req::post_json("/api/v1/accounts", r#"{"username":"extra3","email":"e3@x.org","password":"password1","z":["\ud800",{"y":"\udc00"}]}"#), 201).await;
}

#[tokio::test]
async fn d6_display_name_and_bio_with_a_lone_surrogate() {
    let pair = Pair::new().await;
    pair.register("mia").await;
    let tokens = pair.login("mia", PASSWORD).await;
    let patch = |body: &str| Req::patch_me(&tokens, body);
    pair.deviation(
        &patch(r#"{"display_name":"\ud800"}"#),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    pair.deviation(
        &patch(r#"{"display_name":"a\udc00b"}"#),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    pair.deviation(
        &patch(r#"{"bio":"\ud800"}"#),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    pair.deviation(
        &patch(r#"{"bio":"text \udfff text","locked":true}"#),
        Deviation::D6StoredText,
        500,
        422,
    )
    .await;
    // nothing changed on either side, including the locked flag sent with the bad bio
    assert_eq!(
        pair.db_same(
            "SELECT display_name || '|' || bio || '|' || CAST(locked AS TEXT) FROM accounts"
        )
        .await
        .unwrap(),
        "||0"
    );
    // the length rules run first, as in Python: these are the same 422 error body
    let long = format!(r#"{{"display_name":"\ud800{}"}}"#, "a".repeat(30));
    let py = pair.same_status(&patch(&long), 422).await.0;
    assert_eq!(
        py.json(),
        json!({"error": "Display name must be at most 30 characters"})
    );
    let long_bio = format!(r#"{{"bio":"\ud800{}"}}"#, "b".repeat(500));
    pair.same_status(&patch(&long_bio), 422).await;
    // a tainted display name next to a bio that is too long: the bio rule shows first
    let both = format!(r#"{{"display_name":"\ud800","bio":"{}"}}"#, "b".repeat(501));
    let py = pair.same_status(&patch(&both), 422).await.0;
    assert_eq!(
        py.json(),
        json!({"error": "Bio must be at most 500 characters"})
    );
    // without a token: 401 on both, whatever the body holds
    pair.same_status(
        &Req::patch_json("/api/v1/accounts/me", r#"{"display_name":"\ud800"}"#),
        401,
    )
    .await;
    // a valid pair is an emoji, stored and returned the same
    let py = pair
        .same_status(
            &patch(r#"{"display_name":"\ud83d\ude00","bio":"\ud83d\ude00\ud83d\ude00"}"#),
            200,
        )
        .await
        .0;
    assert_eq!(py.json()["display_name"], "😀");
    // a lone surrogate in an ignored field does not matter
    pair.same_status(&patch(r#"{"bio":"ok","z":"\ud800"}"#), 200)
        .await;
}

// ---- decision 2: body limit ----

#[tokio::test]
async fn d2_a_body_above_one_mebibyte_is_201_on_python_and_413_on_rust() {
    let pair = Pair::new().await;
    // Just under the limit both accept (the Rust limit is 1 MiB of body bytes).
    let head = r#"{"username":"big1","email":"big1@x.org","password":"password1","pad":""#;
    let pad = "x".repeat(1024 * 1024 - head.len() - 2);
    let under = format!(r#"{head}{pad}"}}"#);
    assert_eq!(under.len(), 1024 * 1024);
    pair.same_status(
        &Req::post_json("/api/v1/accounts", &under).label("body of exactly 1 MiB"),
        201,
    )
    .await;
    // Above it: Python has no limit.
    let head = r#"{"username":"big2","email":"big2@x.org","password":"password1","pad":""#;
    let over = format!(r#"{head}{}"}}"#, "x".repeat(1024 * 1024 + 1));
    pair.deviation(
        &Req::post_json("/api/v1/accounts", &over).label("body above 1 MiB"),
        Deviation::D2BodyLimit,
        201,
        413,
    )
    .await;
    let head = r#"{"username":"big3","email":"big3@x.org","password":"password1","pad":""#;
    let huge = format!(r#"{head}{}"}}"#, "x".repeat(3 * 1024 * 1024));
    pair.deviation(
        &Req::post_json("/api/v1/accounts", &huge).label("body of 3 MiB"),
        Deviation::D2BodyLimit,
        201,
        413,
    )
    .await;
    // Rust stored nothing for the rejected requests, and still serves
    let (py, rs) = pair
        .scalar("SELECT group_concat(username) FROM accounts")
        .await;
    assert_eq!(py.as_deref(), Some("big1,big2,big3"));
    if !pair.python_only() {
        assert_eq!(rs.as_deref(), Some("big1"));
    }
    pair.same_status(&Req::lookup("big1"), 200).await;
}

// ---- UNRESOLVED: Python behaviour no decision covers ----

#[tokio::test]
async fn u02_a_sign_in_username_with_a_lone_surrogate_is_a_500_on_python_and_a_401_on_rust() {
    // UNRESOLVED U-02. Decision 6 covers the username in registration (422 error body) and the
    // password at sign-in. A lone surrogate in the sign-in USERNAME is a Python 500 (SQLite
    // cannot bind it), with or without an account of that name. Rust finds no account (U+FFFD
    // matches nothing) and answers 401. A human must say whether Rust copies the 500, answers
    // 401 as now, or answers a 4xx like decision 1.
    let pair = Pair::new().await;
    pair.register("alice").await;
    for (i, name) in [r"al\ud800ice", r"\udc00", r"nobody\ud83d"]
        .iter()
        .enumerate()
    {
        let req = sign(name, PASSWORD).label(&format!("sign-in username with surrogate {i}"));
        pair.unresolved(&req, "U-02-signin-username-surrogate", 500, 401)
            .await;
    }
}

#[tokio::test]
async fn u03_json_that_python_accepts_and_serde_json_rejects() {
    // UNRESOLVED U-03. Python's `json.loads` accepts NaN, Infinity, -Infinity, huge exponents,
    // a UTF-8 byte order mark, and UTF-8, UTF-16 and UTF-32 bodies it can detect. Each is fine
    // for Python even where the value sits in a field that is ignored. serde_json refuses all
    // of them, so Rust answers 422. A human must say whether such bodies are contract (OP-06)
    // and whether Rust must copy them (a hand written JSON reader) or may refuse them.
    let pair = Pair::new().await;
    pair.register("alice").await;
    let good = r#"{"username":"alice","password":"password1"}"#;
    let with_extra =
        |extra: &str| format!(r#"{{"username":"alice","password":"password1","x":{extra}}}"#);
    for extra in ["NaN", "Infinity", "-Infinity", "1e999", "-1e999"] {
        let req = Req::post_json("/api/v1/auth/token", &with_extra(extra))
            .label(&format!("non-standard number {extra}"));
        pair.unresolved(&req, "U-03-python-only-json", 200, 422)
            .await;
    }
    let bom = [b"\xef\xbb\xbf".as_slice(), good.as_bytes()].concat();
    let req = Req::new("POST", "/api/v1/auth/token")
        .raw_body(Some(b"application/json"), &bom)
        .label("utf-8 BOM");
    pair.unresolved(&req, "U-03-python-only-json", 200, 422)
        .await;
    let utf16: Vec<u8> = [
        vec![0xff, 0xfe],
        good.encode_utf16().flat_map(u16::to_le_bytes).collect(),
    ]
    .concat();
    let req = Req::new("POST", "/api/v1/auth/token")
        .raw_body(Some(b"application/json"), &utf16)
        .label("utf-16 body");
    pair.unresolved(&req, "U-03-python-only-json", 200, 422)
        .await;
    let utf16_be: Vec<u8> = [
        vec![0xfe, 0xff],
        good.encode_utf16().flat_map(u16::to_be_bytes).collect(),
    ]
    .concat();
    let req = Req::new("POST", "/api/v1/auth/token")
        .raw_body(Some(b"application/json"), &utf16_be)
        .label("utf-16 BE body");
    pair.unresolved(&req, "U-03-python-only-json", 200, 422)
        .await;
    // Invalid UTF-8: Python answers 400 `{"detail": "There was an error parsing the body"}` (not
    // the 422 list), Rust 422. Same class, same unresolved item.
    for body in [
        &b"\xc3\x28"[..],
        b"{\"username\":\"\xff\"}",
        b"{\"username\":\"\xc0\xaf\"}",
    ] {
        let req = Req::new("POST", "/api/v1/auth/token")
            .raw_body(Some(b"application/json"), body)
            .label(&format!("invalid utf-8 {body:?}"));
        pair.unresolved(&req, "U-03-python-only-json", 400, 422)
            .await;
    }
    // What both refuse the same way (422, detail list) stays compared as parity.
    for body in [
        &b"\xff\xfe"[..],
        b"\x00",
        b"{\"a\":\"\x01\"}",
        b"{\"a\":\"\\x\"}",
        b"",
        b"{",
        b"nul\x00l",
    ] {
        let req = Req::new("POST", "/api/v1/auth/token")
            .raw_body(Some(b"application/json"), body)
            .label(&format!("bad bytes {body:?}"));
        let (py, _) = pair.same(&req).await;
        assert_eq!(py.status, 422, "{body:?}: {}", py.describe_for_test());
    }
}

#[tokio::test]
async fn u04_an_email_with_a_code_point_only_newer_unicode_lower_cases() {
    // UNRESOLVED U-04, see `unit_unicode_tables.rs`. U+10D50 (Garay capital) is lower-cased to
    // U+10D70 by Rust and left alone by the oracle's Python 3.12. The HTTP answers are the same,
    // the stored email is not.
    let pair = Pair::new().await;
    pair.same_status(&Req::register("garay", "\u{10D50}@x.org", PASSWORD), 201)
        .await;
    let (py, rs) = pair.scalar("SELECT email FROM accounts").await;
    assert_eq!(py.as_deref(), Some("\u{10D50}@x.org"));
    if !pair.python_only() {
        assert_eq!(rs.as_deref(), Some("\u{10D70}@x.org"));
    }
}
