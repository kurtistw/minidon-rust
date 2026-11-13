//! Wave 5 self-test of the parity harness (REQ-018, REQ-017, I-29, review M-05, M-08).
//! The harness must not write to the Python reference repo, must freeze and move the clock on
//! both builds, must keep the databases apart, and its comparison must report differences.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod parity;

use std::path::Path;
use std::process::Command;

use parity::{compare, Observed, Pair, Req, T0};
use time::macros::datetime;

type Snapshot = Vec<(String, u64, u128)>;

/// Every entry below the repo (ignored files too, `.git` excluded) with size and mtime. A new
/// `.pyc`, a changed database or a new directory shows up.
fn tree(root: &Path) -> Snapshot {
    fn walk(dir: &Path, root: &Path, out: &mut Snapshot) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.file_name().is_some_and(|n| n == ".git") && dir == root {
                continue;
            }
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let mtime = meta
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            out.push((
                path.strip_prefix(root).unwrap().display().to_string(),
                meta.len(),
                mtime,
            ));
            if meta.is_dir() {
                walk(&path, root, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

fn git_status(root: &Path) -> String {
    let out = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--short", "--ignored"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn the_reference_repo_is_never_written_by_a_harness_run() {
    let repo = parity::reference_repo();
    let (tree_before, status_before) = (tree(&repo), git_status(&repo));
    {
        let pair = Pair::new().await;
        pair.register("alice").await;
        let tokens = pair.login("alice", parity::PASSWORD).await;
        pair.same_status(&Req::get("/api/v1/accounts/me").bearer(&tokens), 200)
            .await;
    }
    assert_eq!(
        tree(&repo),
        tree_before,
        "a file in the reference repo changed"
    );
    assert_eq!(git_status(&repo), status_before);
}

#[tokio::test]
async fn the_oracle_runs_from_a_copy_outside_the_reference_repo() {
    let oracle = parity::oracle();
    let repo = std::fs::canonicalize(parity::reference_repo()).unwrap();
    let copy = std::fs::canonicalize(&oracle.dir).unwrap();
    assert!(!copy.starts_with(&repo) && !repo.starts_with(&copy));
    assert_eq!(
        std::fs::read_to_string(copy.join(".oracle-ready"))
            .unwrap()
            .trim(),
        parity::ORACLE_SHA
    );
}

#[tokio::test]
async fn the_clock_is_frozen_and_movable_on_both_builds() {
    let pair = Pair::new().await;
    let created = |o: &Observed| o.json()["created_at"].as_str().unwrap().to_owned();
    // At T0 the microseconds are zero: Python prints no fraction (review M-08, on the oracle).
    let (py, rs) = pair.register("a1").await;
    assert_eq!(
        (created(&py), created(&rs)),
        ("2026-01-01T00:00:00".into(), "2026-01-01T00:00:00".into())
    );
    // Frozen: a second later in real time changes nothing.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (py, rs) = pair.register("a2").await;
    assert_eq!(
        (created(&py), created(&rs)),
        ("2026-01-01T00:00:00".into(), "2026-01-01T00:00:00".into())
    );
    for (at, text) in [
        (
            datetime!(2026-03-04 05:06:07.5),
            "2026-03-04T05:06:07.500000",
        ),
        (
            datetime!(2026-03-04 05:06:08.000001),
            "2026-03-04T05:06:08.000001",
        ),
        (
            datetime!(2026-12-31 23:59:59.123456),
            "2026-12-31T23:59:59.123456",
        ),
        (datetime!(2027-01-01 00:00:00), "2027-01-01T00:00:00"),
    ] {
        pair.set_clock(at).await;
        let (py, rs) = pair
            .register(&format!("u{}", text.len() + at.microsecond() as usize))
            .await;
        assert_eq!(
            (created(&py), created(&rs)),
            (text.to_owned(), text.to_owned())
        );
    }
    pair.set_clock(T0).await;
}

#[tokio::test]
async fn the_two_builds_use_separate_database_copies_of_one_python_made_file() {
    let pair = Pair::new().await;
    assert_eq!(
        pair.scalar("SELECT CAST(COUNT(*) AS TEXT) FROM accounts")
            .await,
        (Some("0".into()), Some("0".into()))
    );
    pair.register("alice").await;
    pair.sql("UPDATE accounts SET bio = 'seeded'").await;
    let (py, rs) = pair.scalar("SELECT bio FROM accounts").await;
    assert_eq!(
        (py.as_deref(), rs.as_deref()),
        (Some("seeded"), Some("seeded"))
    );
    // Stored datetime text is Python's `YYYY-MM-DD HH:MM:SS.ffffff`, on both sides.
    let (py, rs) = pair.scalar("SELECT created_at FROM accounts").await;
    assert_eq!(
        (py.as_deref(), rs.as_deref()),
        (
            Some("2026-01-01 00:00:00.000000"),
            Some("2026-01-01 00:00:00.000000")
        )
    );
    assert_eq!(
        parity::oracle()
            .template_db
            .metadata()
            .map(|m| m.len() > 0)
            .ok(),
        Some(true)
    );
}

fn obs(status: u16, headers: &[(&str, &str)], body: &str) -> Observed {
    Observed {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        raw_body: body.as_bytes().to_vec(),
    }
}

#[test]
fn the_comparison_reports_every_kind_of_difference() {
    let json = [("content-type", "application/json")];
    let ok = obs(200, &json, r#"{"id":1,"created_at":"2026-01-01T00:00:00"}"#);
    assert!(compare(&ok, &ok.clone(), 1, 2).is_ok());
    // status
    assert!(compare(&ok, &obs(201, &json, &ok.body()), 1, 2).is_err());
    // headers
    assert!(compare(
        &ok,
        &obs(200, &[("content-type", "text/plain")], &ok.body()),
        1,
        2
    )
    .is_err());
    assert!(compare(&ok, &obs(200, &[], &ok.body()), 1, 2).is_err());
    // body text, key order, whitespace
    for body in [
        r#"{"id":1,"created_at":"2026-01-01T00:00:00","x":1}"#,
        r#"{"created_at":"2026-01-01T00:00:00","id":1}"#,
        r#"{"id": 1,"created_at":"2026-01-01T00:00:00"}"#,
    ] {
        assert!(
            compare(&ok, &obs(200, &json, body), 1, 2).is_err(),
            "{body}"
        );
    }
    // a date format change is reported even though dates are normalised
    for date in [
        "2026-01-01T00:00:00.000000",
        "2026-01-01T00:00:00Z",
        "2026-01-01 00:00:00",
    ] {
        let body = format!(r#"{{"id":1,"created_at":"{date}"}}"#);
        assert!(
            compare(&ok, &obs(200, &json, &body), 1, 2).is_err(),
            "{date}"
        );
    }
    // a different but well formed date is reported (the clock is fixed on both sides)
    let other = obs(200, &json, r#"{"id":1,"created_at":"2026-01-01T00:00:01"}"#);
    assert!(compare(&ok, &other, 1, 2).is_err());
    // ids are generated, so a different id alone is not a difference
    assert!(compare(
        &ok,
        &obs(
            200,
            &json,
            r#"{"id":99,"created_at":"2026-01-01T00:00:00"}"#
        ),
        1,
        2
    )
    .is_ok());
}

#[test]
fn only_a_422_detail_list_is_compared_by_shape() {
    let json = [("content-type", "application/json")];
    let a = obs(
        422,
        &json,
        r#"{"detail":[{"type":"x","loc":["body"],"msg":"one text"}]}"#,
    );
    let b = obs(422, &json, r#"{"detail":[{"msg":"another"}]}"#);
    assert_eq!(compare(&a, &b, 1, 2), Ok(parity::Kind::DetailShape));
    // the error body shape is contract: `{"error": ...}` against `{"detail": ...}` differs
    let error = obs(422, &json, r#"{"error":"Username is taken"}"#);
    assert!(compare(&a, &error, 1, 2).is_err());
    assert!(compare(&error, &obs(422, &json, r#"{"error":"Other text"}"#), 1, 2).is_err());
    // a detail body that is not a list, or has more keys, is not the framework shape
    assert!(compare(&a, &obs(422, &json, r#"{"detail":"x"}"#), 1, 2).is_err());
    assert!(compare(&a, &obs(422, &json, r#"{"detail":[],"extra":1}"#), 1, 2).is_err());
    // a 404 detail body is compared as text
    let nf = obs(404, &json, r#"{"detail":"Not Found"}"#);
    assert!(compare(&nf, &obs(404, &json, r#"{"detail":"Gone"}"#), 1, 2).is_err());
}

#[test]
fn location_and_allow_are_compared_with_the_host_removed_and_methods_as_a_set() {
    let py = obs(307, &[("location", "http://127.0.0.1:111/a")], "");
    let rs = obs(307, &[("location", "http://127.0.0.1:222/a")], "");
    assert!(compare(&py, &rs, 111, 222).is_ok());
    let rs_other = obs(307, &[("location", "http://127.0.0.1:222/b")], "");
    assert!(compare(&py, &rs_other, 111, 222).is_err());
    let a = obs(405, &[("allow", "GET, PATCH")], "x");
    let b = obs(405, &[("allow", "PATCH,GET")], "x");
    assert!(compare(&a, &b, 1, 2).is_ok());
    assert!(compare(&a, &obs(405, &[("allow", "GET")], "x"), 1, 2).is_err());
}
