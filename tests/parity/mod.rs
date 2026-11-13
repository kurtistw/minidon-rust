//! Parity harness core (REQ-018, REQ-017, STATED OP-07, adr-001 D4, plan review M-04, M-05).
//!
//! One black-box suite: the SAME request bytes go to the Python oracle and to the Rust build,
//! each on its own copy of one Python-made database, with a fixed clock, and the answers are
//! compared.
//!
//! - Oracle: a `git archive` copy of the reference repo at the pinned commit under /tmp, with
//!   dependencies from `uv sync --frozen` inside the copy and `PYTHONDONTWRITEBYTECODE=1`. The
//!   reference repo is only read by `git archive` (I-29). `oracle/oracle_app.py` starts the
//!   unmodified app with a frozen clock (review M-05).
//! - Rust: the real router (`build_app`) served in this process on a free port with a
//!   `FixedClock` (the binary has no clock override and must not get one). The real binary is
//!   covered separately by `startup_schema.rs` and by one smoke parity test.
//! - Requests go over a plain socket so both servers receive identical bytes.
//! - Comparison: status, content-type, allow and location, and the body after the normaliser.
//!   Nothing is whitelisted implicitly. A difference is either an `exchange_same` failure, or it
//!   is pinned by name: `deviation` (adr-001 decisions 1, 2 and 6) or `unresolved` (Python
//!   behaviour no decision covers, kept visible for a human). Every comparison is appended to
//!   `parity-report.jsonl` in the cargo target tmp dir.
#![allow(dead_code)]

pub mod dataset;
pub mod normalise;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use minidon_rust::platform::app::{build_app, AppState};
use minidon_rust::platform::clock::{format_wire, FixedClock, NaiveUtc};
use minidon_rust::platform::config::Settings;
use minidon_rust::platform::db::connect_and_verify;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use sqlx::Connection;
use time::macros::datetime;

/// The pinned reference commit.
pub const ORACLE_SHA: &str = "e1fbd3a9c8d863086cb9e4005b411d7c33c91488";
/// The fixed clock every run starts at: 2026-01-01 00:00:00 UTC.
pub const T0: NaiveUtc = datetime!(2026-01-01 00:00:00);
pub const PASSWORD: &str = "password1";

// ---- the oracle ----

pub struct Oracle {
    pub dir: PathBuf,
    pub script: PathBuf,
    pub template_db: PathBuf,
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn tmp_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

/// The reference repo. Only `git archive` reads it.
pub fn reference_repo() -> PathBuf {
    std::env::var_os("MINIDON_REFERENCE_REPO")
        .map_or_else(|| manifest().join("../minidon"), PathBuf::from)
}

static ORACLE: OnceLock<Oracle> = OnceLock::new();

/// Make (once) the oracle copy and a Python-made empty database to copy from.
pub fn oracle() -> &'static Oracle {
    ORACLE.get_or_init(|| {
        let dir = std::env::var_os("MINIDON_ORACLE_DIR").map_or_else(
            || PathBuf::from(format!("/tmp/minidon-oracle-{}", &ORACLE_SHA[..7])),
            PathBuf::from,
        );
        let setup = manifest().join("tests/parity/oracle/setup.sh");
        let status = Command::new("sh")
            .arg(&setup)
            .arg(reference_repo())
            .arg(ORACLE_SHA)
            .arg(&dir)
            .status()
            .expect("run oracle setup.sh");
        assert!(status.success(), "oracle setup failed: {status}");
        let script = manifest().join("tests/parity/oracle/oracle_app.py");
        let template_db = tmp_dir().join(format!("parity-template-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&template_db);
        let status = Command::new(dir.join(".venv/bin/python"))
            .args(["-B"])
            .arg(&script)
            .arg("--create-schema")
            .current_dir(&dir)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("MINIDON_DATABASE_URL", sqlite_url(&template_db))
            .status()
            .expect("run oracle --create-schema");
        assert!(status.success(), "oracle create_tables failed");
        write_versions(&dir);
        Oracle {
            dir,
            script,
            template_db,
        }
    })
}

fn write_versions(dir: &Path) {
    let out = Command::new(dir.join(".venv/bin/python"))
        .args(["-B", "-c"])
        .arg(
            "import sys, importlib.metadata as m; \
             print(sys.version.split()[0], *[f'{p}=={m.version(p)}' for p in \
             ('fastapi','starlette','uvicorn','pydantic','sqlalchemy','h11')])",
        )
        .output();
    if let Ok(out) = out {
        let _ = std::fs::write(tmp_dir().join("oracle-versions.txt"), out.stdout);
    }
}

fn sqlite_url(path: &Path) -> String {
    // SQLAlchemy form: four slashes for an absolute path.
    format!("sqlite:///{}", path.display())
}

// ---- servers ----

static START: Mutex<()> = Mutex::new(());

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind a free port")
        .local_addr()
        .expect("local addr")
        .port()
}

pub struct PythonServer {
    child: Child,
    pub port: u16,
}

impl PythonServer {
    fn start(db: &Path, log: &Path) -> Self {
        let oracle = oracle();
        let _lock = START
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let port = free_port();
        let stderr = std::fs::File::create(log).expect("create python log");
        let mut child = Command::new(oracle.dir.join(".venv/bin/python"))
            .arg("-B")
            .arg(&oracle.script)
            .arg(port.to_string())
            .current_dir(&oracle.dir)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("MINIDON_DATABASE_URL", sqlite_url(db))
            .env_remove("MINIDON_DOMAIN")
            .stdout(Stdio::null())
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn the oracle");
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if let Ok(Some(status)) = child.try_wait() {
                let text = std::fs::read_to_string(log).unwrap_or_default();
                panic!("oracle exited early ({status}): {text}");
            }
            assert!(Instant::now() < deadline, "oracle did not start");
            std::thread::sleep(Duration::from_millis(40));
        }
        Self { child, port }
    }
}

impl Drop for PythonServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

enum Backend {
    InProcess(tokio::task::JoinHandle<()>),
    Binary(Child),
}

pub struct RustServer {
    pub port: u16,
    /// The clock of the in-process build. The real binary has none (it uses the system clock).
    pub clock: Arc<FixedClock>,
    backend: Backend,
}

impl RustServer {
    async fn start(db: &Path) -> Self {
        let settings = Settings {
            database_url: sqlite_url(db),
            instance_domain: "minidon.local".into(),
            token_ttl_days: 30,
        };
        let pool = connect_and_verify(&settings).await.expect("open rust db");
        let clock = Arc::new(FixedClock::new(T0));
        let state = AppState {
            pool,
            clock: clock.clone(),
            settings: Arc::new(settings),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind rust server");
        let addr: SocketAddr = listener.local_addr().expect("rust addr");
        let app = build_app(state);
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            port: addr.port(),
            clock,
            backend: Backend::InProcess(task),
        }
    }

    /// The real `minidon-rust` binary on a free port (`MINIDON_BIND`), on the system clock.
    fn start_binary(db: &Path) -> Self {
        let _lock = START
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let port = free_port();
        let mut child = Command::new(env!("CARGO_BIN_EXE_minidon-rust"))
            .env("MINIDON_DATABASE_URL", sqlite_url(db))
            .env("MINIDON_BIND", format!("127.0.0.1:{port}"))
            .env_remove("MINIDON_DOMAIN")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the rust binary");
        let deadline = Instant::now() + Duration::from_secs(30);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if let Ok(Some(status)) = child.try_wait() {
                panic!("rust binary exited early ({status})");
            }
            assert!(Instant::now() < deadline, "rust binary did not start");
            std::thread::sleep(Duration::from_millis(40));
        }
        Self {
            port,
            clock: Arc::new(FixedClock::new(T0)),
            backend: Backend::Binary(child),
        }
    }
}

impl Drop for RustServer {
    fn drop(&mut self) {
        match &mut self.backend {
            Backend::InProcess(task) => task.abort(),
            Backend::Binary(child) => {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

// ---- requests ----

#[derive(Clone, Debug)]
pub struct Tokens {
    pub py: String,
    pub rs: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Python,
    Rust,
}

#[derive(Clone, Debug)]
enum AuthSpec {
    Literal(Vec<u8>),
    Token {
        prefix: Vec<u8>,
        tokens: Tokens,
        suffix: Vec<u8>,
    },
}

/// One request, sent byte for byte to both builds. Tokens differ per build, so a token header is
/// described and filled in per side.
#[derive(Clone, Debug)]
pub struct Req {
    pub label: String,
    method: String,
    path: String,
    headers: Vec<(String, Vec<u8>)>,
    auth: Vec<AuthSpec>,
    body: Option<Vec<u8>>,
}

impl Req {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            label: format!("{method} {path}"),
            method: method.to_owned(),
            path: path.to_owned(),
            headers: Vec::new(),
            auth: Vec::new(),
            body: None,
        }
    }
    pub fn get(path: &str) -> Self {
        Self::new("GET", path)
    }
    pub fn post_json(path: &str, body: &str) -> Self {
        Self::new("POST", path).json(body)
    }
    pub fn patch_json(path: &str, body: &str) -> Self {
        Self::new("PATCH", path).json(body)
    }
    pub fn json(mut self, body: &str) -> Self {
        self.headers
            .push(("content-type".into(), b"application/json".to_vec()));
        self.body = Some(body.as_bytes().to_vec());
        self.label = format!("{} {}", self.label, short(body));
        self
    }
    pub fn raw_body(mut self, content_type: Option<&[u8]>, body: &[u8]) -> Self {
        if let Some(ct) = content_type {
            self.headers.push(("content-type".into(), ct.to_vec()));
        }
        self.body = Some(body.to_vec());
        self
    }
    pub fn header(mut self, name: &str, value: &[u8]) -> Self {
        self.headers.push((name.to_owned(), value.to_vec()));
        self
    }
    pub fn bearer(mut self, tokens: &Tokens) -> Self {
        self.auth.push(AuthSpec::Token {
            prefix: b"Bearer ".to_vec(),
            tokens: tokens.clone(),
            suffix: Vec::new(),
        });
        self
    }
    /// `prefix + token + suffix` as the Authorization value.
    pub fn auth_around(mut self, prefix: &[u8], tokens: &Tokens, suffix: &[u8]) -> Self {
        self.auth.push(AuthSpec::Token {
            prefix: prefix.to_vec(),
            tokens: tokens.clone(),
            suffix: suffix.to_vec(),
        });
        self
    }
    pub fn auth_literal(mut self, value: &[u8]) -> Self {
        self.auth.push(AuthSpec::Literal(value.to_vec()));
        self
    }
    pub fn register(username: &str, email: &str, password: &str) -> Self {
        let body = serde_json::json!({"username": username, "email": email, "password": password});
        Self::post_json("/api/v1/accounts", &body.to_string())
    }
    pub fn sign_in(username: &str, password: &str) -> Self {
        let body = serde_json::json!({"username": username, "password": password});
        Self::post_json("/api/v1/auth/token", &body.to_string())
    }
    pub fn me(tokens: &Tokens) -> Self {
        Self::get("/api/v1/accounts/me").bearer(tokens)
    }
    pub fn patch_me(tokens: &Tokens, body: &str) -> Self {
        Self::patch_json("/api/v1/accounts/me", body).bearer(tokens)
    }
    pub fn lookup(username: &str) -> Self {
        Self::get(&format!("/api/v1/accounts/{username}"))
    }
    pub fn label(mut self, label: &str) -> Self {
        self.label = label.to_owned();
        self
    }

    fn bytes(&self, side: Side, port: u16) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(format!("{} {} HTTP/1.1\r\n", self.method, self.path).as_bytes());
        out.extend_from_slice(format!("Host: 127.0.0.1:{port}\r\n").as_bytes());
        for (name, value) in &self.headers {
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(value);
            out.extend_from_slice(b"\r\n");
        }
        for auth in &self.auth {
            out.extend_from_slice(b"Authorization: ");
            match auth {
                AuthSpec::Literal(value) => out.extend_from_slice(value),
                AuthSpec::Token {
                    prefix,
                    tokens,
                    suffix,
                } => {
                    out.extend_from_slice(prefix);
                    let token = match side {
                        Side::Python => &tokens.py,
                        Side::Rust => &tokens.rs,
                    };
                    out.extend_from_slice(token.as_bytes());
                    out.extend_from_slice(suffix);
                }
            }
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"Connection: close\r\n");
        if let Some(body) = &self.body {
            out.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
        }
        out.extend_from_slice(b"\r\n");
        if let Some(body) = &self.body {
            out.extend_from_slice(body);
        }
        out
    }
}

fn short(text: &str) -> String {
    let flat: String = text.chars().take(70).collect();
    if text.chars().count() > 70 {
        format!("{flat}...")
    } else {
        flat
    }
}

// ---- answers ----

#[derive(Clone, Debug)]
pub struct Observed {
    pub status: u16,
    /// Lower-case names, values decoded as latin-1. Repeated headers appear repeatedly.
    pub headers: Vec<(String, String)>,
    pub raw_body: Vec<u8>,
}

impl Observed {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
    pub fn body(&self) -> String {
        String::from_utf8_lossy(&self.raw_body).into_owned()
    }
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.raw_body).unwrap_or(serde_json::Value::Null)
    }
    pub fn describe_for_test(&self) -> String {
        self.describe()
    }
    fn describe(&self) -> String {
        let selected: Vec<String> = self
            .headers
            .iter()
            .filter(|(n, _)| matches!(n.as_str(), "content-type" | "allow" | "location"))
            .map(|(n, v)| format!("{n}: {v}"))
            .collect();
        format!(
            "{} [{}] {}",
            self.status,
            selected.join("; "),
            short(&self.body())
        )
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn decode_chunked(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(end) = find(data, b"\r\n") {
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&data[..end])
                .split(';')
                .next()
                .unwrap_or("0")
                .trim(),
            16,
        )
        .unwrap_or(0);
        data = &data[end + 2..];
        if size == 0 || data.len() < size {
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or_default();
    }
    out
}

fn raw_http(port: u16, request: &[u8], head_only: bool) -> Result<Observed, String> {
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    // The server may answer and close before it read a large body: ignore a write error and
    // read the answer.
    let _ = stream.write_all(request);
    let mut data = Vec::new();
    let _ = stream.read_to_end(&mut data);
    let split = find(&data, b"\r\n\r\n")
        .ok_or_else(|| format!("no response head: {:?}", String::from_utf8_lossy(&data)))?;
    let head = String::from_utf8_lossy(&data[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line {status_line:?}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let mut body = data[split + 4..].to_vec();
    let get = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    if head_only {
        body.clear();
    } else if get("transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        body = decode_chunked(&body);
    } else if let Some(length) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.truncate(length);
    }
    Ok(Observed {
        status,
        headers,
        raw_body: body,
    })
}

async fn send_raw(port: u16, request: Vec<u8>, head_only: bool) -> Observed {
    tokio::task::spawn_blocking(move || raw_http(port, &request, head_only))
        .await
        .expect("join")
        .unwrap_or_else(|e| panic!("request failed: {e}"))
}

// ---- comparison ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Same status, headers and normalised body.
    Same,
    /// A 422 `{"detail": [...]}` on both sides: status and top-level shape only (adr-001
    /// section 5 decision 3). The messages differ by design.
    DetailShape,
}

fn is_detail_shape(body: &serde_json::Value) -> bool {
    body.as_object()
        .is_some_and(|o| o.len() == 1 && o.get("detail").is_some_and(serde_json::Value::is_array))
}

fn host_free(value: &str, port: u16) -> String {
    value.replace(&format!("127.0.0.1:{port}"), "<HOST>")
}

fn allow_set(value: Option<&str>) -> Option<Vec<String>> {
    value.map(|v| {
        let mut methods: Vec<String> = v.split(',').map(|m| m.trim().to_owned()).collect();
        methods.sort();
        methods
    })
}

/// Compare the two answers. `Err` explains the first difference found.
pub fn compare(py: &Observed, rs: &Observed, py_port: u16, rs_port: u16) -> Result<Kind, String> {
    compare_with(py, rs, py_port, rs_port, true)
}

/// `strict_dates`: also require equal date values (true whenever both clocks are fixed).
pub fn compare_with(
    py: &Observed,
    rs: &Observed,
    py_port: u16,
    rs_port: u16,
    strict_dates: bool,
) -> Result<Kind, String> {
    if py.status != rs.status {
        return Err(format!("status {} != {}", py.status, rs.status));
    }
    for name in ["content-type", "location", "allow"] {
        let a = py.header(name).map(|v| host_free(v, py_port));
        let b = rs.header(name).map(|v| host_free(v, rs_port));
        let same = if name == "allow" {
            allow_set(a.as_deref()) == allow_set(b.as_deref())
        } else {
            a == b
        };
        if !same {
            return Err(format!("header {name}: {a:?} != {b:?}"));
        }
    }
    let (pb, rb) = (py.body(), rs.body());
    if py.status == 422 && is_detail_shape(&py.json()) && is_detail_shape(&rs.json()) {
        return Ok(Kind::DetailShape);
    }
    let (np, nr) = (
        normalise::normalise(&pb).map_err(|e| format!("python body: {e}"))?,
        normalise::normalise(&rb).map_err(|e| format!("rust body: {e}"))?,
    );
    // With different clocks (the real binary) only the format of a date is compared, and the
    // format check already ran in `normalise`. Whether a fraction shows depends on the clock.
    let (np, nr) = if strict_dates {
        (np, nr)
    } else {
        (
            np.replace("<TIMESTAMP+FRACTION>", "<TIMESTAMP>"),
            nr.replace("<TIMESTAMP+FRACTION>", "<TIMESTAMP>"),
        )
    };
    if np != nr {
        return Err(format!("body\n  python: {np}\n  rust:   {nr}"));
    }
    // With a fixed clock the dates must be equal as well, not only well formed.
    let (dp, dr) = (
        normalise::wire_dates(&pb).map_err(|e| e.to_string())?,
        normalise::wire_dates(&rb).map_err(|e| e.to_string())?,
    );
    if strict_dates && dp != dr {
        return Err(format!("date values differ: {dp:?} != {dr:?}"));
    }
    Ok(Kind::Same)
}

// ---- the pair ----

pub struct Pair {
    pub py: PythonServer,
    pub rs: RustServer,
    py_db: PathBuf,
    rs_db: PathBuf,
    test: String,
    binary: bool,
    python_only: bool,
    _dir: tempfile::TempDir,
}

impl Pair {
    /// Two servers on two copies of the Python-made empty database, both at T0.
    pub async fn new() -> Self {
        Self::build(false).await
    }

    /// Like `new`, but Rust is the real binary on the system clock. Dates are then only checked
    /// for format (the normaliser), not for value.
    pub async fn new_with_binary() -> Self {
        Self::build(true).await
    }

    async fn build(binary: bool) -> Self {
        let oracle = oracle();
        let dir = tempfile::tempdir_in(tmp_dir()).expect("temp dir");
        let py_db = dir.path().join("python.db");
        let rs_db = dir.path().join("rust.db");
        std::fs::copy(&oracle.template_db, &py_db).expect("copy db");
        std::fs::copy(&oracle.template_db, &rs_db).expect("copy db");
        let py = PythonServer::start(&py_db, &dir.path().join("python.log"));
        let rs = if binary {
            RustServer::start_binary(&rs_db)
        } else {
            RustServer::start(&rs_db).await
        };
        let test = std::thread::current()
            .name()
            .unwrap_or("unnamed")
            .to_owned();
        let pair = Self {
            py,
            rs,
            py_db,
            rs_db,
            test,
            binary,
            // PARITY_PYTHON_ONLY=1: the test-first check. Every expectation asserted on the
            // Python answer must hold on the oracle alone, before Rust is compared.
            python_only: std::env::var_os("PARITY_PYTHON_ONLY").is_some(),
            _dir: dir,
        };
        pair.set_clock(T0).await;
        pair
    }

    /// Move both clocks. Python is frozen through the wrapper's control route.
    pub async fn set_clock(&self, at: NaiveUtc) {
        if !self.binary {
            self.rs.clock.set(at);
        }
        let body = format!(r#"{{"at":"{}"}}"#, format_wire(at));
        let request = Req::post_json("/__clock", &body).bytes(Side::Python, self.py.port);
        let answer = send_raw(self.py.port, request, false).await;
        assert_eq!(answer.status, 200, "set clock: {}", answer.body());
    }

    /// Run the same SQL on both database copies (seeding only, never DDL).
    pub async fn sql(&self, statement: &str) {
        for path in [&self.py_db, &self.rs_db] {
            let options = SqliteConnectOptions::new()
                .filename(path)
                .foreign_keys(false);
            let mut conn = SqliteConnection::connect_with(&options)
                .await
                .expect("open db copy");
            sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
                .execute(&mut conn)
                .await
                .unwrap_or_else(|e| panic!("seed {statement}: {e}"));
            conn.close().await.expect("close");
        }
    }

    /// Read one value from both database copies, for compat style checks.
    pub async fn scalar(&self, statement: &str) -> (Option<String>, Option<String>) {
        let mut out = Vec::new();
        for path in [&self.py_db, &self.rs_db] {
            let options = SqliteConnectOptions::new()
                .filename(path)
                .foreign_keys(false);
            let mut conn = SqliteConnection::connect_with(&options)
                .await
                .expect("open db copy");
            let value: Option<String> =
                sqlx::query_scalar(sqlx::AssertSqlSafe(statement.to_owned()))
                    .fetch_optional(&mut conn)
                    .await
                    .expect("scalar");
            conn.close().await.expect("close");
            out.push(value);
        }
        let rs = out.pop().flatten();
        let py = out.pop().flatten();
        (py, rs)
    }

    pub fn python_only(&self) -> bool {
        self.python_only
    }

    /// The stored value must be the same in both database copies. Returns the Python one.
    pub async fn db_same(&self, statement: &str) -> Option<String> {
        let (py, rs) = self.scalar(statement).await;
        if !self.python_only {
            assert_eq!(py, rs, "stored values differ for {statement}");
        }
        py
    }

    /// Send to both builds without comparing.
    pub async fn exchange(&self, req: &Req) -> (Observed, Observed) {
        let head_only = req.method == "HEAD";
        let py = send_raw(
            self.py.port,
            req.bytes(Side::Python, self.py.port),
            head_only,
        )
        .await;
        if self.python_only {
            return (py.clone(), py);
        }
        let rs = send_raw(self.rs.port, req.bytes(Side::Rust, self.rs.port), head_only).await;
        (py, rs)
    }

    fn record(&self, req: &Req, kind: &str, py: &Observed, rs: &Observed) {
        let line = serde_json::json!({
            "test": self.test,
            "case": req.label,
            "kind": kind,
            "python": py.status,
            "rust": rs.status,
        });
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(tmp_dir().join("parity-report.jsonl"))
        {
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    }

    /// The same request to both builds, which must answer the same. Returns both answers.
    pub async fn same(&self, req: &Req) -> (Observed, Observed) {
        let (py, rs) = self.exchange(req).await;
        if self.python_only {
            self.record(req, "python-only", &py, &py);
            return (py.clone(), py);
        }
        match compare_with(&py, &rs, self.py.port, self.rs.port, !self.binary) {
            Ok(Kind::Same) => self.record(req, "same", &py, &rs),
            Ok(Kind::DetailShape) => self.record(req, "detail_shape", &py, &rs),
            Err(why) => {
                self.record(req, "DIFFERENT", &py, &rs);
                panic!(
                    "PARITY FAILURE in {}\n  request: {}\n  python: {}\n  rust:   {}\n  why: {why}",
                    self.test,
                    req.label,
                    py.describe(),
                    rs.describe()
                );
            }
        }
        (py, rs)
    }

    /// Like `same`, and the answer must have this status on both sides.
    pub async fn same_status(&self, req: &Req, status: u16) -> (Observed, Observed) {
        let (py, rs) = self.same(req).await;
        assert_eq!(py.status, status, "{}: {}", req.label, py.describe());
        (py, rs)
    }

    /// A deviation decided in adr-001 section 5: pinned by id and by both statuses. Nothing
    /// else about the request may differ, so the pin also checks the shape of each answer.
    pub async fn deviation(&self, req: &Req, dev: Deviation, py_status: u16, rs_status: u16) {
        let (py, rs) = self.exchange(req).await;
        self.record(req, &format!("deviation:{}", dev.id()), &py, &rs);
        self.pin(req, &py, &rs, py_status, rs_status, dev.id());
    }

    /// Behaviour the stated contract leaves out on purpose (OP-06: the documentation routes are
    /// not ported). Pinned like a deviation, never silently skipped.
    pub async fn out_of_contract(&self, req: &Req, id: &str, py_status: u16, rs_status: u16) {
        let (py, rs) = self.exchange(req).await;
        self.record(req, &format!("out-of-contract:{id}"), &py, &rs);
        self.pin(req, &py, &rs, py_status, rs_status, id);
    }

    /// Python behaviour no decision covers. Pinned so it cannot change unseen, and reported as
    /// unresolved. This is NOT a whitelist entry: a human decides what Rust should do.
    pub async fn unresolved(&self, req: &Req, id: &str, py_status: u16, rs_status: u16) {
        let (py, rs) = self.exchange(req).await;
        self.record(req, &format!("unresolved:{id}"), &py, &rs);
        self.pin(req, &py, &rs, py_status, rs_status, id);
    }

    fn pin(&self, req: &Req, py: &Observed, rs: &Observed, py_s: u16, rs_s: u16, id: &str) {
        if self.python_only {
            assert_eq!(py.status, py_s, "{id} {}: {}", req.label, py.describe());
            return;
        }
        assert_eq!(
            (py.status, rs.status),
            (py_s, rs_s),
            "{id} {}\n  python: {}\n  rust:   {}",
            req.label,
            py.describe(),
            rs.describe()
        );
        if py_s == 500 {
            assert_eq!(py.body(), "Internal Server Error", "{id}");
            assert_eq!(
                py.header("content-type"),
                Some("text/plain; charset=utf-8"),
                "{id}"
            );
        }
    }

    // ---- helpers for common flows ----

    pub async fn register(&self, username: &str) -> (Observed, Observed) {
        self.register_with(username, &format!("{username}@x.org"), PASSWORD)
            .await
    }

    pub async fn register_with(
        &self,
        username: &str,
        email: &str,
        password: &str,
    ) -> (Observed, Observed) {
        let body = serde_json::json!({"username": username, "email": email, "password": password});
        self.same(&Req::post_json("/api/v1/accounts", &body.to_string()))
            .await
    }

    /// Sign in on both builds (answers compared) and keep both tokens.
    pub async fn login(&self, username: &str, password: &str) -> Tokens {
        let body = serde_json::json!({"username": username, "password": password});
        let (py, rs) = self
            .same_status(
                &Req::post_json("/api/v1/auth/token", &body.to_string()),
                200,
            )
            .await;
        let token = |o: &Observed| {
            o.json()["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        };
        Tokens {
            py: token(&py),
            rs: token(&rs),
        }
    }
}

/// The deviations adr-001 section 5 allows (decisions 1, 2 and 6). Nothing else is pinned this
/// way.
#[derive(Clone, Copy, Debug)]
pub enum Deviation {
    /// Decision 1: a lone-surrogate password (Python 500, Rust 4xx).
    D1Password,
    /// Decision 2: a request body above 1 MiB (Python accepts, Rust 413).
    D2BodyLimit,
    /// Decision 6: email, display name or bio with a lone surrogate that passed validation
    /// (Python 500, Rust 4xx).
    D6StoredText,
}

impl Deviation {
    pub fn id(self) -> &'static str {
        match self {
            Self::D1Password => "D1-password",
            Self::D2BodyLimit => "D2-body-limit",
            Self::D6StoredText => "D6-stored-text",
        }
    }
}
