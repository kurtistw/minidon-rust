# Crates

Every direct crate, with the locked version (see `Cargo.lock`), licence and reason.
Source of approval: `adr-001` section 5 decisions 5 (D1 list) and 7 (extras). Crates are
added only through a reviewed change to `Cargo.toml` (threat row 15). Licence policy and the
exceptions are in `deny.toml`.

## Runtime

| Crate | Version | Licence | Reason | Approved by |
|---|---|---|---|---|
| axum | 0.8.9 | MIT | HTTP router on tokio. Features: http1, json, tokio only | D1 |
| tokio | 1.54.1 | MIT | Async runtime. Features: rt-multi-thread, macros, net | D1 |
| sqlx | 0.9.0 | MIT OR Apache-2.0 | SQLite access with bound parameters. Feature `sqlite-bundled` only (no macros, no migrate, no extension loading, so no DDL tooling) | D1 |
| regex | 1.13.1 | MIT OR Apache-2.0 | Username pattern `^[a-z0-9_]{1,30}\n?\z`, no lookaround | D1 |
| pbkdf2 | 0.12.2 | MIT OR Apache-2.0 | PBKDF2-HMAC-SHA256 password hash | D1 |
| hmac | 0.12.1 | MIT OR Apache-2.0 | HMAC for PBKDF2 | D1 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | SHA-256 | D1 |
| subtle | 2.6.1 | BSD-3-Clause | Constant-time compare of hash digests | D1 |
| rand | 0.8.8 | MIT OR Apache-2.0 | OS random salts and tokens | D1 |
| serde | 1.0.229 | MIT OR Apache-2.0 | Request and response types | decision 7 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | JSON bodies | decision 7 |
| tower-http | 0.6.11 | MIT | `catch-panic` layer only, maps panics to the Python 500 body | decision 7 |
| base64 | 0.22.1 | MIT OR Apache-2.0 | URL-safe, unpadded token encoding | decision 7 |
| hex | 0.4.3 | MIT OR Apache-2.0 | Lower-case hex for salt and digest | decision 7 |
| time | 0.3.55 | MIT OR Apache-2.0 | Naive UTC datetimes and the stored and wire formats (not chrono) | decision 7 |

## Dev only

| Crate | Version | Licence | Reason | Approved by |
|---|---|---|---|---|
| reqwest | 0.12.28 | MIT OR Apache-2.0 | HTTP client for the tests and the parity harness. No TLS features (local http only) | decision 7 |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | Temporary database files | decision 7 |

## Policy deviations that need a human decision

1. Licences. The policy allows MIT, Apache-2.0 and BSD only. Two permissive licences cannot be
   avoided in the transitive tree: Unicode-3.0 (the ICU data crates, pulled by `url`, which
   `sqlx-core` and `reqwest` need) and Zlib (`foldhash`, pulled by `hashbrown`, which
   `sqlx-core` needs). They are allowed per crate in `deny.toml` `exceptions`, not globally.
2. Advisory RUSTSEC-2026-0459 (bundled SQLite before 3.54, crafted `-wal` file). `sqlx-sqlite`
   0.9.0 pins `libsqlite3-sys` below 0.38, the fix is in 0.39. Ignored in `deny.toml` and
   `.cargo/audit.toml` with the reason. Remove when sqlx allows 0.39 or later.
3. sqlx 0.9.0 needs Rust 1.94, so the toolchain is pinned to 1.94.0 (not an older stable).
