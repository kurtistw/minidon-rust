//! Wave 4 route tests for the five Accounts endpoints (REQ-001..REQ-016, REQ-019, SC-ACC-001..
//! 029 at route level). The real router on a free port, an in-memory database with the
//! Python-made schema and a fixed clock at T0 = 2026-01-01 00:00:00. Expected values were
//! taken from the Python oracle (see the build log), not from reading the code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use reqwest::header::{HeaderValue, CONTENT_TYPE, LOCATION};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use support::App;

const PW: &str = "correct-horse";

struct Reply {
    status: StatusCode,
    headers: reqwest::header::HeaderMap,
    text: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap()
    }
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

impl App {
    async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(&str, &[u8])],
        body: Option<&[u8]>,
    ) -> Reply {
        let mut request = support::client().request(method, format!("{}{path}", self.base));
        for (name, value) in headers {
            request = request.header(*name, HeaderValue::from_bytes(value).unwrap());
        }
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        let response = request.send().await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap();
        Reply {
            status,
            headers,
            text,
        }
    }

    async fn post_json(&self, path: &str, body: &str) -> Reply {
        self.send(
            Method::POST,
            path,
            &[("content-type", b"application/json")],
            Some(body.as_bytes()),
        )
        .await
    }

    async fn register(&self, name: &str) -> Reply {
        self.post_json(
            "/api/v1/accounts",
            &json!({"username": name, "email": format!("{name}@x.org"), "password": PW})
                .to_string(),
        )
        .await
    }

    async fn token(&self, name: &str) -> String {
        let reply = self
            .post_json(
                "/api/v1/auth/token",
                &json!({"username": name, "password": PW}).to_string(),
            )
            .await;
        assert_eq!(reply.status, 200, "{}", reply.text);
        reply.json()["access_token"].as_str().unwrap().to_owned()
    }

    async fn get(&self, path: &str) -> Reply {
        self.send(Method::GET, path, &[], None).await
    }

    async fn get_auth(&self, path: &str, token: &str) -> Reply {
        let value = format!("Bearer {token}");
        self.send(
            Method::GET,
            path,
            &[("authorization", value.as_bytes())],
            None,
        )
        .await
    }

    async fn patch(&self, token: Option<&str>, body: &str) -> Reply {
        let value = token.map(|t| format!("Bearer {t}"));
        let mut headers: Vec<(&str, &[u8])> = vec![("content-type", b"application/json")];
        if let Some(value) = &value {
            headers.push(("authorization", value.as_bytes()));
        }
        self.send(
            Method::PATCH,
            "/api/v1/accounts/me",
            &headers,
            Some(body.as_bytes()),
        )
        .await
    }

    async fn suspend(&self, name: &str) {
        sqlx::query(
            "UPDATE accounts SET suspended_at = '2026-01-01 00:00:00.000000' WHERE username = ?",
        )
        .bind(name)
        .execute(&self.pool)
        .await
        .unwrap();
    }
}

fn detail_shape(reply: &Reply) -> bool {
    let body = reply.json();
    body.as_object().is_some_and(|o| o.len() == 1) && body["detail"].is_array()
}

// ---- register ----

#[tokio::test]
async fn register_answers_201_with_python_body_text_and_first_account_is_admin() {
    let app = support::spawn_app().await;
    let reply = app.register("Alice").await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.header("content-type"), Some("application/json"));
    assert_eq!(
        reply.text,
        r#"{"id":1,"username":"alice","acct":"alice@minidon.local","display_name":"","bio":"","locked":false,"role":"admin","created_at":"2026-01-01T00:00:00","followers_count":0,"following_count":0,"statuses_count":0}"#
    );
    let second = app.register("bob").await;
    assert_eq!(second.json()["role"], "user");
    assert_eq!(second.json()["id"], 2);
}

#[tokio::test]
async fn register_dates_keep_the_fraction_only_when_microseconds_are_not_zero() {
    let app = support::spawn_app().await;
    app.clock
        .set(time::macros::datetime!(2026-03-04 05:06:07.5));
    assert_eq!(
        app.register("alice").await.json()["created_at"],
        "2026-03-04T05:06:07.500000"
    );
    app.clock
        .set(time::macros::datetime!(2026-03-04 05:06:08.000001));
    assert_eq!(
        app.register("bob").await.json()["created_at"],
        "2026-03-04T05:06:08.000001"
    );
    app.clock.set(time::macros::datetime!(2026-03-04 05:06:09));
    assert_eq!(
        app.register("cy").await.json()["created_at"],
        "2026-03-04T05:06:09"
    );
}

#[tokio::test]
async fn register_domain_errors_use_the_error_body() {
    let app = support::spawn_app().await;
    app.register("carol").await;
    let cases = [
        (
            r#"{"username":"bad-name","email":"a@x.org","password":"password1"}"#,
            422,
            "Username must be 1-30 letters, digits or underscores",
        ),
        (
            r#"{"username":"dora","email":"nope","password":"password1"}"#,
            422,
            "Email address is not valid",
        ),
        (
            r#"{"username":"dora","email":"d@x.org","password":"short"}"#,
            422,
            "Password must be at least 8 characters",
        ),
        (
            r#"{"username":"CAROL","email":"c2@x.org","password":"password1"}"#,
            409,
            "Username is taken",
        ),
        (
            r#"{"username":"dora","email":" CAROL@x.org ","password":"password1"}"#,
            409,
            "Email is already registered",
        ),
    ];
    for (body, status, message) in cases {
        let reply = app.post_json("/api/v1/accounts", body).await;
        assert_eq!(reply.status, status, "{body}");
        assert_eq!(reply.json(), json!({ "error": message }), "{body}");
    }
}

#[tokio::test]
async fn register_framework_errors_use_the_detail_body() {
    let app = support::spawn_app().await;
    for body in [
        "{bad",
        "",
        "[]",
        "null",
        r#"{"username":"a"}"#,
        r#"{"username":1,"email":"a@x.org","password":"password1"}"#,
    ] {
        let reply = app.post_json("/api/v1/accounts", body).await;
        assert_eq!(reply.status, 422, "{body}");
        assert!(detail_shape(&reply), "{body}: {}", reply.text);
    }
    // Missing and non-JSON content types are not parsed as JSON by Python either.
    let ok = r#"{"username":"eve","email":"e@x.org","password":"password1"}"#;
    for headers in [vec![], vec![("content-type", &b"text/plain"[..])]] {
        let reply = app
            .send(
                Method::POST,
                "/api/v1/accounts",
                &headers,
                Some(ok.as_bytes()),
            )
            .await;
        assert_eq!(reply.status, 422);
        assert!(detail_shape(&reply));
    }
    // These content types are JSON for Python.
    for (i, content_type) in [
        "application/json; charset=latin-1",
        "application/vnd.api+json",
        "APPLICATION/JSON",
    ]
    .iter()
    .enumerate()
    {
        let body =
            format!(r#"{{"username":"ct{i}","email":"ct{i}@x.org","password":"password1"}}"#);
        let reply = app
            .send(
                Method::POST,
                "/api/v1/accounts",
                &[("content-type", content_type.as_bytes())],
                Some(body.as_bytes()),
            )
            .await;
        assert_eq!(reply.status, 201, "{content_type}");
    }
}

#[tokio::test]
async fn register_ignores_extra_fields_and_takes_the_last_duplicate() {
    let app = support::spawn_app().await;
    let reply = app
        .post_json(
            "/api/v1/accounts",
            r#"{"username":"first","email":"f@x.org","password":"password1","role":"admin","username":"second"}"#,
        )
        .await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.json()["username"], "second");
}

#[tokio::test]
async fn register_body_above_one_mib_is_413() {
    let app = support::spawn_app().await;
    let big = format!(
        r#"{{"username":"big","email":"b@x.org","password":"password1","pad":"{}"}}"#,
        "x".repeat(1024 * 1024)
    );
    let reply = app.post_json("/api/v1/accounts", &big).await;
    assert_eq!(reply.status, 413);
}

#[tokio::test]
async fn register_non_ascii_text_round_trips_raw_in_the_body() {
    let app = support::spawn_app().await;
    let reply = app
        .post_json(
            "/api/v1/accounts",
            &json!({"username": "\u{212A}elvin", "email": "  \u{a0}K@X.org\u{a0} ", "password": "pässwörd-😀"}).to_string(),
        )
        .await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.json()["username"], "kelvin");
    let stored: String = sqlx::query_scalar("SELECT email FROM accounts")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(stored, "k@x.org");
}

// ---- trailing newline (SC-ACC-004), at route level ----

#[tokio::test]
async fn trailing_newline_username_routes_register_look_up_and_sign_in() {
    let app = support::spawn_app().await;
    let reply = app
        .post_json(
            "/api/v1/accounts",
            r#"{"username":"alice\n","email":"nl@x.org","password":"password1"}"#,
        )
        .await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.json()["username"], "alice\n");
    assert_eq!(app.register("alice").await.status, 201);
    assert_eq!(
        app.get("/api/v1/accounts/alice%0A").await.json()["username"],
        "alice\n"
    );
    assert_eq!(
        app.get("/api/v1/accounts/alice").await.json()["username"],
        "alice"
    );
    for bad in [
        r#""alice\n\n""#,
        r#""\n""#,
        r#""ali\nce""#,
        r#""alice\r\n""#,
    ] {
        let body = format!(r#"{{"username":{bad},"email":"z@x.org","password":"password1"}}"#);
        assert_eq!(
            app.post_json("/api/v1/accounts", &body).await.status,
            422,
            "{bad}"
        );
    }
}

// ---- sign-in ----

#[tokio::test]
async fn sign_in_returns_python_keys_a_43_char_token_and_the_ttl() {
    let app = support::spawn_app().await;
    app.register("alice").await;
    let reply = app
        .post_json(
            "/api/v1/auth/token",
            &json!({"username": "ALICE", "password": PW}).to_string(),
        )
        .await;
    assert_eq!(reply.status, 200);
    assert!(
        reply.text.starts_with(r#"{"access_token":""#),
        "{}",
        reply.text
    );
    assert!(
        reply
            .text
            .ends_with(r#"","token_type":"bearer","expires_at":"2026-01-31T00:00:00"}"#),
        "{}",
        reply.text
    );
    let body = reply.json();
    assert_eq!(body["token_type"], "bearer");
    assert_eq!(body["expires_at"], "2026-01-31T00:00:00");
    assert_eq!(body["access_token"].as_str().unwrap().len(), 43);
}

#[tokio::test]
async fn sign_in_failures() {
    let app = support::spawn_app().await;
    app.register("alice").await;
    app.register("hank").await;
    app.suspend("hank").await;
    for (user, pw, status, message) in [
        (
            "alice",
            "wrong-password",
            401,
            "Invalid username or password",
        ),
        ("nobody", PW, 401, "Invalid username or password"),
        ("hank", PW, 403, "This account is suspended"),
        (
            "hank",
            "wrong-password",
            401,
            "Invalid username or password",
        ),
    ] {
        let reply = app
            .post_json(
                "/api/v1/auth/token",
                &json!({"username": user, "password": pw}).to_string(),
            )
            .await;
        assert_eq!(reply.status, status, "{user}");
        assert_eq!(reply.json(), json!({ "error": message }));
    }
}

// ---- me ----

#[tokio::test]
async fn me_needs_a_token_and_patch_updates_partially() {
    let app = support::spawn_app().await;
    app.register("mia").await;
    let token = app.token("mia").await;
    let reply = app.get("/api/v1/accounts/me").await;
    assert_eq!(reply.status, 401);
    assert_eq!(reply.json(), json!({"error": "Sign in required"}));
    let me = app.get_auth("/api/v1/accounts/me", &token).await;
    assert_eq!(me.status, 200);
    assert_eq!(me.json()["username"], "mia");

    let patched = app
        .patch(
            Some(&token),
            r#"{"display_name":"Mia","bio":"hi","locked":true}"#,
        )
        .await;
    assert_eq!(patched.status, 200);
    let body = patched.json();
    assert_eq!(
        (
            body["display_name"].as_str(),
            body["bio"].as_str(),
            body["locked"].as_bool()
        ),
        (Some("Mia"), Some("hi"), Some(true))
    );
    let only_bio = app
        .patch(Some(&token), r#"{"bio":"new","display_name":null}"#)
        .await
        .json();
    assert_eq!(
        (
            only_bio["display_name"].as_str(),
            only_bio["bio"].as_str(),
            only_bio["locked"].as_bool()
        ),
        (Some("Mia"), Some("new"), Some(true))
    );
    assert_eq!(app.patch(Some(&token), "{}").await.status, 200);
}

#[tokio::test]
async fn patch_limits_and_lax_bool() {
    let app = support::spawn_app().await;
    app.register("mia").await;
    let token = app.token("mia").await;
    let long = "a".repeat(31);
    let reply = app
        .patch(Some(&token), &json!({"display_name": long}).to_string())
        .await;
    assert_eq!(reply.status, 422);
    assert_eq!(
        reply.json(),
        json!({"error": "Display name must be at most 30 characters"})
    );
    let reply = app
        .patch(Some(&token), &json!({"bio": "b".repeat(501)}).to_string())
        .await;
    assert_eq!(
        reply.json(),
        json!({"error": "Bio must be at most 500 characters"})
    );
    // Valid display name next to an invalid bio changes nothing.
    let reply = app
        .patch(
            Some(&token),
            &json!({"display_name": "Ok", "bio": "b".repeat(501)}).to_string(),
        )
        .await;
    assert_eq!(reply.status, 422);
    assert_eq!(
        app.get_auth("/api/v1/accounts/me", &token).await.json()["display_name"],
        ""
    );
    for (value, expect) in [
        (json!("yes"), true),
        (json!(1), true),
        (json!("False"), false),
        (json!(0), false),
        (json!(1.0), true),
    ] {
        let reply = app
            .patch(Some(&token), &json!({"locked": value}).to_string())
            .await;
        assert_eq!(reply.json()["locked"], expect, "{value}");
    }
    for bad in [
        json!(2),
        json!(" true "),
        json!("maybe"),
        json!(0.5),
        json!([]),
    ] {
        let reply = app
            .patch(Some(&token), &json!({"locked": bad}).to_string())
            .await;
        assert_eq!(reply.status, 422, "{bad}");
        assert!(detail_shape(&reply), "{bad}");
    }
}

#[tokio::test]
async fn patch_check_order_is_json_syntax_then_auth_then_field_types() {
    let app = support::spawn_app().await;
    app.register("mia").await;
    let token = app.token("mia").await;
    // Broken JSON is a 422 even without a token (Python parses the body first).
    let reply = app.patch(None, "{bad").await;
    assert_eq!(reply.status, 422);
    assert!(detail_shape(&reply));
    // Valid JSON of the wrong type without a token: auth runs before field validation.
    let reply = app.patch(None, r#"{"display_name": 5}"#).await;
    assert_eq!(reply.status, 401);
    assert_eq!(reply.json(), json!({"error": "Sign in required"}));
    let reply = app.patch(None, "[]").await;
    assert_eq!(reply.status, 401);
    // With a token the field error shows.
    let reply = app.patch(Some(&token), r#"{"display_name": 5}"#).await;
    assert_eq!(reply.status, 422);
    assert!(detail_shape(&reply));
    assert_eq!(
        app.patch(None, r#"{"display_name": "x"}"#).await.status,
        401
    );
}

#[tokio::test]
async fn expired_and_suspended_tokens_are_signed_out() {
    let app = support::spawn_app().await;
    app.register("ivy").await;
    app.register("jay").await;
    let ivy = app.token("ivy").await;
    let jay = app.token("jay").await;
    app.clock.set(time::macros::datetime!(2026-01-30 23:59:59));
    assert_eq!(app.get_auth("/api/v1/accounts/me", &ivy).await.status, 200);
    app.suspend("jay").await;
    assert_eq!(app.get_auth("/api/v1/accounts/me", &jay).await.status, 401);
    app.clock.set(time::macros::datetime!(2026-01-31 00:00:00));
    assert_eq!(app.get_auth("/api/v1/accounts/me", &ivy).await.status, 401);
    // The public lookup ignores the header altogether.
    assert_eq!(app.get_auth("/api/v1/accounts/ivy", &ivy).await.status, 200);
}

// ---- profile lookup ----

#[tokio::test]
async fn lookup_has_no_auth_and_is_case_insensitive() {
    let app = support::spawn_app().await;
    app.register("kim").await;
    app.register("lou").await;
    app.suspend("lou").await;
    let reply = app.get("/api/v1/accounts/KIM").await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["username"], "kim");
    for bad_header in [&b"Bearer nonsense"[..], b"Bearer \xff\xfe", b"Basic x"] {
        let reply = app
            .send(
                Method::GET,
                "/api/v1/accounts/kim",
                &[("authorization", bad_header)],
                None,
            )
            .await;
        assert_eq!(reply.status, 200, "{bad_header:?}");
    }
    for name in ["lou", "LOU", "nobody", "%FF", "ali+ce", "kim%00", "kim%3Fx"] {
        let reply = app.get(&format!("/api/v1/accounts/{name}")).await;
        assert_eq!(reply.status, 404, "{name}");
        assert_eq!(
            reply.json(),
            json!({"error": "Account not found"}),
            "{name}"
        );
    }
    assert_eq!(app.get("/api/v1/accounts/%E7%94%A8").await.status, 404);
    assert_eq!(app.get("/api/v1/accounts/kim?x=1").await.status, 200);
}

#[tokio::test]
async fn profile_counts_come_from_the_tables() {
    let app = support::spawn_app().await;
    app.register("kim").await;
    app.register("lou").await;
    sqlx::query("INSERT INTO follows (follower_id, followee_id, accepted, created_at) VALUES (2, 1, 1, '2026-01-01 00:00:00.000000')")
        .execute(&app.pool)
        .await
        .unwrap();
    let body = app.get("/api/v1/accounts/kim").await.json();
    assert_eq!(
        (
            body["followers_count"].as_i64(),
            body["following_count"].as_i64()
        ),
        (Some(1), Some(0))
    );
}

// ---- lone surrogates, end to end (decisions 1 and 6) ----

#[tokio::test]
async fn lone_surrogates_follow_the_decided_outcomes() {
    let app = support::spawn_app().await;
    app.register("alice").await;
    let post = |body: String| {
        let app = &app;
        async move { app.post_json("/api/v1/accounts", &body).await }
    };
    // username: Python's 422 with the error body, never the generic detail body.
    let reply =
        post(r#"{"username":"sur\ud800","email":"s@x.org","password":"password1"}"#.into()).await;
    assert_eq!(reply.status, 422);
    assert_eq!(
        reply.json(),
        json!({"error": "Username must be 1-30 letters, digits or underscores"})
    );
    // email and password that pass every rule: a 4xx where Python crashes with a 500.
    for body in [
        r#"{"username":"sur2","email":"s\ud800@x.org","password":"password1"}"#,
        r#"{"username":"sur3","email":"s3@x.org","password":"pass\ud800word"}"#,
    ] {
        let reply = post(body.into()).await;
        assert_eq!(reply.status, 422, "{body}");
    }
    // Earlier rules still win.
    let reply =
        post(r#"{"username":"alice","email":"s\ud800@x.org","password":"password1"}"#.into()).await;
    assert_eq!(reply.json(), json!({"error": "Username is taken"}));
    // A tainted extra field is ignored, as Python ignores it.
    let reply = post(
        r#"{"username":"sur4","email":"s4@x.org","password":"password1","zzz":"\ud800"}"#.into(),
    )
    .await;
    assert_eq!(reply.status, 201);
    // Sign-in: unknown user 401, known user 4xx, username with a surrogate 401 (unresolved).
    let sign = |u: &str, p: &str| format!(r#"{{"username":"{u}","password":"{p}"}}"#);
    let reply = app
        .post_json("/api/v1/auth/token", &sign("nobody", "pass\\ud800word"))
        .await;
    assert_eq!(reply.status, 401);
    let reply = app
        .post_json("/api/v1/auth/token", &sign("alice", "pass\\ud800word"))
        .await;
    assert_eq!(reply.status, 422);
    let reply = app
        .post_json("/api/v1/auth/token", &sign("al\\ud800ice", PW))
        .await;
    assert_eq!(reply.status, 401);
    // Profile text.
    let signed = app
        .post_json("/api/v1/auth/token", &sign("sur4", "password1"))
        .await;
    let token = signed.json()["access_token"].as_str().unwrap().to_owned();
    for body in [r#"{"display_name":"\ud800"}"#, r#"{"bio":"a\udc00b"}"#] {
        assert_eq!(app.patch(Some(&token), body).await.status, 422, "{body}");
    }
    let reply = app.patch(Some(&token), r#"{"display_name":"😀"}"#).await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["display_name"], "\u{1f600}");
}

// ---- framework behaviour: 404, 405, trailing slash, paths ----

#[tokio::test]
async fn wrong_methods_are_405_with_the_allow_header_of_the_first_matching_route() {
    let app = support::spawn_app().await;
    app.register("alice").await;
    let cases = [
        (Method::HEAD, "/api/v1/accounts/alice", "GET"),
        (Method::OPTIONS, "/api/v1/accounts/alice", "GET"),
        (Method::POST, "/api/v1/accounts/me", "GET"),
        (Method::PUT, "/api/v1/accounts/me", "GET"),
        (Method::HEAD, "/api/v1/accounts/me", "GET"),
        (Method::DELETE, "/api/v1/auth/token", "POST"),
        (Method::GET, "/api/v1/auth/token", "POST"),
        (Method::GET, "/api/v1/accounts", "POST"),
        (Method::PATCH, "/api/v1/accounts/alice", "GET"),
        (Method::PUT, "/api/v1/accounts/alice", "GET"),
    ];
    for (method, path, allow) in cases {
        let reply = app.send(method.clone(), path, &[], None).await;
        assert_eq!(reply.status, 405, "{method} {path}");
        assert_eq!(reply.header("allow"), Some(allow), "{method} {path}");
        if method != Method::HEAD {
            assert_eq!(reply.json(), json!({"detail": "Method Not Allowed"}));
        }
    }
}

#[tokio::test]
async fn unknown_paths_are_404_and_docs_are_not_served() {
    let app = support::spawn_app().await;
    for path in [
        "/",
        "/nope",
        "/nope/",
        "/api/v1",
        "/api/v1/accounts/a/b",
        "/docs",
        "/redoc",
        "/openapi.json",
        "//api/v1/accounts",
        "/api/v1/accounts/ali%2Fce",
    ] {
        let reply = app.get(path).await;
        assert_eq!(reply.status, 404, "{path}");
        assert_eq!(reply.json(), json!({"detail": "Not Found"}), "{path}");
    }
}

#[tokio::test]
async fn trailing_slash_redirects_with_307_as_python_does() {
    let app = support::spawn_app().await;
    let host = app.base.trim_start_matches("http://").to_owned();
    for (method, path, target) in [
        (
            Method::GET,
            "/api/v1/accounts/alice/",
            "/api/v1/accounts/alice",
        ),
        (Method::GET, "/api/v1/accounts/me/", "/api/v1/accounts/me"),
        (Method::POST, "/api/v1/accounts/", "/api/v1/accounts"),
        (Method::PUT, "/api/v1/accounts/", "/api/v1/accounts"),
        (Method::POST, "/api/v1/auth/token/", "/api/v1/auth/token"),
        (Method::GET, "/api/v1/accounts//", "/api/v1/accounts"),
        (
            Method::GET,
            "/api/v1/accounts/alice//",
            "/api/v1/accounts/alice",
        ),
        (
            Method::GET,
            "/api/v1/accounts/alice/?x=1&y=%20",
            "/api/v1/accounts/alice?x=1&y=%20",
        ),
        (
            Method::GET,
            "/api/v1/accounts/alice%0A/",
            "/api/v1/accounts/alice%0A",
        ),
        (
            Method::GET,
            "/api/v1/accounts/%E7%94%A8/",
            "/api/v1/accounts/%E7%94%A8",
        ),
    ] {
        let reply = app.send(method.clone(), path, &[], None).await;
        assert_eq!(reply.status, 307, "{method} {path}");
        assert_eq!(
            reply
                .headers
                .get(LOCATION)
                .map(|v| v.to_str().unwrap().to_owned()),
            Some(format!("http://{host}{target}")),
            "{method} {path}"
        );
        assert!(reply.text.is_empty());
    }
}

#[tokio::test]
async fn paths_are_decoded_before_routing_as_uvicorn_does() {
    let app = support::spawn_app().await;
    app.register("alice").await;
    // An encoded letter or slash in the fixed part still reaches the route.
    assert_eq!(app.get("/api/v1/accounts/%6De").await.status, 401); // /accounts/me
    assert_eq!(app.get("/api/v1%2Faccounts/alice").await.status, 200);
    assert_eq!(app.get("/api/v1/accounts/al%69ce").await.status, 200);
    assert_eq!(app.get("/api/v1/accounts/ALICE").await.status, 200);
    // Redirect targets use the decoded path (percent is left alone, like Python).
    let reply = app.get("/api/v1%2Faccounts/alice/").await;
    assert_eq!(reply.status, 307);
    assert!(reply
        .header("location")
        .unwrap()
        .ends_with("/api/v1/accounts/alice"));
}

#[tokio::test]
async fn errors_and_ok_responses_are_json_content_type() {
    let app = support::spawn_app().await;
    for path in ["/nope", "/api/v1/accounts/me", "/api/v1/accounts/nobody"] {
        let reply = app.get(path).await;
        assert_eq!(
            reply.header(CONTENT_TYPE.as_str()),
            Some("application/json"),
            "{path}"
        );
    }
}
