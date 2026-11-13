//! Wave 1 unit tests for `platform::config` (REQ-011-AC2, REQ-017, review M-09, m-15).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::path::PathBuf;

use minidon_rust::platform::config::{
    parse_database_url, DatabaseLocation, Settings, MAX_BODY_BYTES,
};

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |key| map.get(key).cloned()
}

#[test]
fn defaults_match_python_config() {
    let settings = Settings::from_lookup(lookup(&[]));
    assert_eq!(settings.database_url, "sqlite:///./minidon.db");
    assert_eq!(settings.instance_domain, "minidon.local");
    assert_eq!(settings.token_ttl_days, 30);
}

#[test]
fn env_variables_override_url_and_domain() {
    let settings = Settings::from_lookup(lookup(&[
        ("MINIDON_DATABASE_URL", "sqlite:////var/data/m.db"),
        ("MINIDON_DOMAIN", "example.test"),
    ]));
    assert_eq!(settings.database_url, "sqlite:////var/data/m.db");
    assert_eq!(settings.instance_domain, "example.test");
}

#[test]
fn ttl_has_no_env_variable_and_is_overridden_in_code_only() {
    // m-15: Python has no env var for the ttl, so Rust adds none.
    let settings = Settings::from_lookup(lookup(&[("MINIDON_TOKEN_TTL_DAYS", "7")]));
    assert_eq!(settings.token_ttl_days, 30);
    let seven = Settings {
        token_ttl_days: 7,
        ..settings
    };
    assert_eq!(seven.token_ttl_days, 7);
}

#[test]
fn body_limit_is_one_mebibyte() {
    assert_eq!(MAX_BODY_BYTES, 1_048_576);
}

#[test]
fn relative_sqlalchemy_url_becomes_relative_path() {
    assert_eq!(
        parse_database_url("sqlite:///./minidon.db").unwrap(),
        DatabaseLocation::File(PathBuf::from("./minidon.db"))
    );
    assert_eq!(
        parse_database_url("sqlite:///minidon.db").unwrap(),
        DatabaseLocation::File(PathBuf::from("minidon.db"))
    );
}

#[test]
fn absolute_sqlalchemy_url_has_four_slashes() {
    assert_eq!(
        parse_database_url("sqlite:////tmp/minidon.db").unwrap(),
        DatabaseLocation::File(PathBuf::from("/tmp/minidon.db"))
    );
}

#[test]
fn memory_forms_are_recognised() {
    assert_eq!(
        parse_database_url("sqlite://").unwrap(),
        DatabaseLocation::Memory
    );
    assert_eq!(
        parse_database_url("sqlite:///:memory:").unwrap(),
        DatabaseLocation::Memory
    );
}

#[test]
fn unsupported_urls_are_rejected_with_a_clear_message() {
    for url in [
        "",
        "postgresql://u@h/db",
        "sqlite+pysqlite:///x.db",
        "sqlite://host/x.db",
        "sqlite:///x.db?mode=ro",
        "sqlite:///",
        "/tmp/minidon.db",
    ] {
        let err = parse_database_url(url).unwrap_err();
        assert!(
            err.to_string().contains("MINIDON_DATABASE_URL"),
            "message for {url:?} must name the variable: {err}"
        );
    }
}
