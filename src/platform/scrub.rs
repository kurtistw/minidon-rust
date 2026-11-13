//! Lone-surrogate pre-parse (STATED: adr-001 section 5 decision 6, plan review M-03).
//!
//! Python's JSON parser accepts a lone surrogate escape such as `"\ud800"`, `serde_json` does
//! not. Without this step Rust would answer the generic `{"detail": ...}` 422 for every field,
//! where Python's behaviour depends on the field (username: its own 422 `{"error": ...}`, sign-in
//! password of an unknown user: 401, other stored text: a 500 that Rust answers with a 4xx).
//!
//! The scan replaces every lone surrogate escape with the escape for U+FFFD and records the top
//! level fields whose final value was a string that held one. A handler then applies the Python
//! check order and decides. Nothing is logged and the text is not echoed.

/// The body text with lone surrogate escapes replaced, and the fields that held one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scrubbed {
    pub text: String,
    /// Top level fields whose last value is a string with a lone surrogate (duplicates: the
    /// last one counts, as in Python).
    pub tainted: Vec<String>,
}

const REPLACEMENT: &[u8] = b"\\uFFFD";

fn hex4(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 4 {
        return None;
    }
    let mut value = 0u32;
    for &b in &bytes[..4] {
        value = value * 16 + char::from(b).to_digit(16)?;
    }
    Some(value)
}

fn is_high(unit: u32) -> bool {
    (0xD800..=0xDBFF).contains(&unit)
}

fn is_low(unit: u32) -> bool {
    (0xDC00..=0xDFFF).contains(&unit)
}

/// Result of scanning one string token that starts after its opening quote.
struct StringScan {
    /// Index just after the closing quote (or the end of the input if it never closes).
    end: usize,
    had_lone: bool,
}

/// Copy one string body into `out`, replacing lone surrogate escapes. `start` is the index
/// after the opening quote.
fn scan_string(bytes: &[u8], start: usize, out: &mut Vec<u8>) -> StringScan {
    let mut i = start;
    let mut had_lone = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                out.push(b'"');
                return StringScan {
                    end: i + 1,
                    had_lone,
                };
            }
            b'\\' => {
                let Some(&next) = bytes.get(i + 1) else {
                    out.push(b'\\');
                    return StringScan {
                        end: bytes.len(),
                        had_lone,
                    };
                };
                if next != b'u' {
                    out.extend_from_slice(&bytes[i..i + 2]);
                    i += 2;
                    continue;
                }
                match hex4(&bytes[i + 2..]) {
                    Some(unit) if is_high(unit) => {
                        let low = (bytes.get(i + 6) == Some(&b'\\')
                            && bytes.get(i + 7) == Some(&b'u'))
                        .then(|| hex4(&bytes[i + 8..]))
                        .flatten()
                        .filter(|&next_unit| is_low(next_unit));
                        if low.is_some() {
                            out.extend_from_slice(&bytes[i..i + 12]);
                            i += 12;
                        } else {
                            out.extend_from_slice(REPLACEMENT);
                            had_lone = true;
                            i += 6;
                        }
                    }
                    Some(unit) if is_low(unit) => {
                        out.extend_from_slice(REPLACEMENT);
                        had_lone = true;
                        i += 6;
                    }
                    // Not a surrogate, or malformed: leave it for the JSON parser to judge.
                    _ => {
                        out.extend_from_slice(&bytes[i..i + 2]);
                        i += 2;
                    }
                }
            }
            other => {
                out.push(other);
                i += 1;
            }
        }
    }
    StringScan {
        end: bytes.len(),
        had_lone,
    }
}

/// What the scanner expects next at the top level object.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Key,
    Value,
}

/// Replace lone surrogate escapes and record the tainted top level fields.
pub fn scrub_lone_surrogates(text: &str) -> Scrubbed {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut depth = 0usize;
    let mut slot = Slot::Key;
    let mut current_key: Option<String> = None;
    let mut tainted: Vec<(String, bool)> = Vec::new();

    let mut mark = |key: &str, value: bool| {
        if let Some(entry) = tainted.iter_mut().find(|(name, _)| name == key) {
            entry.1 = value;
        } else {
            tainted.push((key.to_owned(), value));
        }
    };

    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => {
                let token_start = out.len();
                out.push(b'"');
                let scan = scan_string(bytes, i + 1, &mut out);
                i = scan.end;
                if depth == 1 {
                    if slot == Slot::Key {
                        // The key may itself hold escapes, so it is decoded as JSON.
                        let token = std::str::from_utf8(&out[token_start..]).unwrap_or("\"\"");
                        current_key = serde_json::from_str::<String>(token).ok();
                    } else if let Some(key) = &current_key {
                        mark(key, scan.had_lone);
                    }
                }
            }
            b'{' | b'[' => {
                depth += 1;
                out.push(b);
                i += 1;
                if depth == 1 {
                    slot = Slot::Key;
                }
            }
            b'}' | b']' => {
                depth = depth.saturating_sub(1);
                out.push(b);
                i += 1;
            }
            b':' => {
                out.push(b);
                i += 1;
                if depth == 1 {
                    slot = Slot::Value;
                    // Any value starts as untainted. A string value may flip it.
                    if let Some(key) = &current_key {
                        mark(key, false);
                    }
                }
            }
            b',' => {
                out.push(b);
                i += 1;
                if depth == 1 {
                    slot = Slot::Key;
                }
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }

    Scrubbed {
        // Only ASCII escapes were inserted, so the bytes stay valid UTF-8. The fallback is for
        // the impossible case and keeps this function free of a panic path.
        text: String::from_utf8(out).unwrap_or_else(|_| text.to_owned()),
        tainted: tainted
            .into_iter()
            .filter_map(|(name, hit)| hit.then_some(name))
            .collect(),
    }
}
