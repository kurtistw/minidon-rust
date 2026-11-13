//! How Python reads and rewrites request paths (DERIVED from the oracle: uvicorn 0.54, Starlette
//! 1.10; confirmed by the parity harness).
//!
//! uvicorn decodes the whole raw path with `unquote` (invalid UTF-8 becomes U+FFFD) before
//! Starlette routes on it, so `/api/v1%2Faccounts` and `/accounts/%6De` reach the same routes
//! as their plain forms. A path that matches no route but matches once a trailing slash is
//! added or all trailing slashes are removed gets a 307 whose `Location` is built from the
//! decoded path (`?`, `#` and backslash escaped), then quoted with
//! `quote(url, safe=":/%#?=@[]!$&'()*+,;")`.

/// `urllib.parse.unquote(raw)`: percent escapes decoded as UTF-8 with replacement.
pub fn decode_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hi = bytes.get(i + 1).and_then(|b| char::from(*b).to_digit(16));
            let lo = bytes.get(i + 2).and_then(|b| char::from(*b).to_digit(16));
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(u8::try_from(hi * 16 + lo).unwrap_or(b'%'));
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn push_escaped(out: &mut String, byte: u8) {
    out.push('%');
    out.push(char::from(b"0123456789ABCDEF"[usize::from(byte >> 4)]));
    out.push(char::from(b"0123456789ABCDEF"[usize::from(byte & 15)]));
}

fn quote_with(text: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') || keep(byte) {
            out.push(char::from(byte));
        } else {
            push_escaped(&mut out, byte);
        }
    }
    out
}

/// The decoded path written back as an ASCII path the router can match: everything except
/// letters, digits, `-._~` and `/` is percent-encoded. The router then sees the decoded
/// segments, and each segment decodes back to the same text.
pub fn canonical_path(decoded: &str) -> String {
    quote_with(decoded, |b| b == b'/')
}

/// Starlette's redirect_slashes candidate: with a trailing slash all of them are removed, else
/// one is added. `None` for the root path.
pub fn toggle_slash(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    if path.ends_with('/') {
        Some(path.trim_end_matches('/').to_owned())
    } else {
        Some(format!("{path}/"))
    }
}

/// Does a route pattern such as `/api/v1/accounts/{username}` match this decoded path? Segments
/// are compared one by one, a `{name}` matches any non-empty segment without a slash.
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
    let want: Vec<&str> = pattern.split('/').collect();
    let have: Vec<&str> = path.split('/').collect();
    want.len() == have.len()
        && want.iter().zip(&have).all(|(w, h)| {
            if w.starts_with('{') && w.ends_with('}') {
                !h.is_empty()
            } else {
                w == h
            }
        })
}

/// The `Location` Starlette sends: `http://{host}{path}[?{query}]`, the path first made safe for
/// a URL (`?`, `#`, backslash), the whole text then quoted.
pub fn location_for(host: &str, decoded_path: &str, query: Option<&str>) -> String {
    let path = decoded_path
        .replace('?', "%3F")
        .replace('#', "%23")
        .replace('\\', "%5C");
    let mut url = format!("http://{host}{path}");
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        url.push('?');
        url.push_str(query);
    }
    quote_with(&url, |b| b":/%#?=@[]!$&'()*+,;".contains(&b))
}
