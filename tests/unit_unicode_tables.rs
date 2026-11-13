//! Wave 5: Rust's Unicode tables against the oracle's (wave-2-3.md "Unresolved"). Email is
//! trimmed with `str.strip()` and lower-cased with `str.lower()` in Python, and the username is
//! lower-cased before the pattern runs. The fixture `fixtures/python_unicode.json` is dumped by
//! `parity/oracle/gen_unicode.py` with the oracle's interpreter: every code point whose `lower()`
//! changes it, and every code point `strip()` removes. Rust must agree on all of them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use minidon_rust::accounts::validate::{normalise_email, normalise_username, python_strip};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/python_unicode.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

fn lower_map(fixture: &Value) -> BTreeMap<u32, String> {
    fixture["lower"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(cp, chars)| {
            let text: String = chars
                .as_array()
                .unwrap()
                .iter()
                .map(|v| char::from_u32(u32::try_from(v.as_u64().unwrap()).unwrap()).unwrap())
                .collect();
            (u32::from_str_radix(cp, 16).unwrap(), text)
        })
        .collect()
}

fn scalar_values() -> impl Iterator<Item = char> {
    (0..0x11_0000u32).filter_map(char::from_u32)
}

/// UNRESOLVED U-04. The oracle runs Python 3.12 (Unicode 15.0.0). Rust 1.94 knows newer Unicode
/// versions, so it lower-cases 55 code points that Python leaves alone: Georgian Mtavruli
/// U+1C89, Latin additions, Garay U+10D50..U+10D65 and capitals at U+16EA0..U+16EB8. They
/// are pinned here, so the set cannot change unseen. Effect: an email containing one of them is
/// stored lower-cased by Rust and unchanged by Python, so the uniqueness check can differ for
/// such emails across builds. Usernames are not affected (none lowers to an ASCII pattern
/// character, asserted below). A human decides: copy Python's table (an exclusion list), or
/// accept it, or move the oracle to a newer Python.
const KNOWN_NEWER_IN_RUST: [u32; 55] = [
    0x1C89, 0xA7CB, 0xA7CC, 0xA7CE, 0xA7D2, 0xA7D4, 0xA7DA, 0xA7DC, 0x10D50, 0x10D51, 0x10D52,
    0x10D53, 0x10D54, 0x10D55, 0x10D56, 0x10D57, 0x10D58, 0x10D59, 0x10D5A, 0x10D5B, 0x10D5C,
    0x10D5D, 0x10D5E, 0x10D5F, 0x10D60, 0x10D61, 0x10D62, 0x10D63, 0x10D64, 0x10D65, 0x16EA0,
    0x16EA1, 0x16EA2, 0x16EA3, 0x16EA4, 0x16EA5, 0x16EA6, 0x16EA7, 0x16EA8, 0x16EA9, 0x16EAA,
    0x16EAB, 0x16EAC, 0x16EAD, 0x16EAE, 0x16EAF, 0x16EB0, 0x16EB1, 0x16EB2, 0x16EB3, 0x16EB4,
    0x16EB5, 0x16EB6, 0x16EB7, 0x16EB8,
];

#[test]
fn lower_case_agrees_with_python_except_the_pinned_newer_code_points() {
    let fixture = fixture();
    let expected = lower_map(&fixture);
    let mut different = Vec::new();
    for c in scalar_values() {
        let want = expected
            .get(&(c as u32))
            .cloned()
            .unwrap_or_else(|| c.to_string());
        let got = c.to_string().to_lowercase();
        if got != want {
            // Rust lower-cases a code point that Python 3.12 does not know a mapping for.
            assert_eq!(
                want,
                c.to_string(),
                "U+{:04X}: python maps it differently",
                c as u32
            );
            assert!(
                got.chars().all(|l| !l.is_ascii()),
                "U+{:04X} lowers to ASCII in Rust only: usernames would differ",
                c as u32
            );
            different.push(c as u32);
        }
    }
    assert_eq!(
        different, KNOWN_NEWER_IN_RUST,
        "python {} / unicode {}: the set of differing code points changed",
        fixture["python"], fixture["unicode"]
    );
}

#[test]
fn strip_whitespace_agrees_with_python_for_every_code_point() {
    let fixture = fixture();
    let python: BTreeSet<u32> = fixture["strip_whitespace"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| u32::try_from(v.as_u64().unwrap()).unwrap())
        .collect();
    let rust: BTreeSet<u32> = scalar_values()
        .filter(|c| python_strip(&c.to_string()).is_empty())
        .map(|c| c as u32)
        .collect();
    let only_python: Vec<String> = python
        .difference(&rust)
        .map(|c| format!("U+{c:04X}"))
        .collect();
    let only_rust: Vec<String> = rust
        .difference(&python)
        .map(|c| format!("U+{c:04X}"))
        .collect();
    assert!(
        only_python.is_empty() && only_rust.is_empty(),
        "strip differs: only python {only_python:?}, only rust {only_rust:?}"
    );
}

#[test]
fn the_normalisers_use_the_same_tables() {
    assert_eq!(normalise_username("\u{212A}ELVIN"), "kelvin");
    assert_eq!(
        normalise_email("\u{a0}\u{1c}Ab@X.ORG\u{3000}\u{1f}"),
        "ab@x.org"
    );
}
