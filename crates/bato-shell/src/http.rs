//! HTTP(S) to the app's backend, bato-auth or the registry: JSON calls, byte
//! bodies in and out, streams, Server-Sent Events and a streaming relay.
//!
//! Blocking (ureq 3), meant for worker threads: wrap calls in
//! [`crate::worker::spawn_blocking`] (or `iced_ext::blocking`,
//! `iced_ext::with_progress`). TLS is rustls with the platform verifier, so
//! certificates are checked against the system trust store, which is where the
//! mkcert root for `https://*.bato.lan` lives.
//!
//! **Timeouts.** A JSON call has a 5 s budget for the whole exchange (the
//! PoC's; Electron's `net:request` had none, so "Test connection" to an
//! unroutable address hung for minutes). Raise it per client
//! ([`Http::with_timeout`]) or per call ([`Call::timeout`]). Streams and
//! streamed uploads ([`Call::stream`], [`Call::body_reader`], [`relay`]) have
//! no total budget: they fail when nothing moves for [`STALL_TIMEOUT`]
//! ([`Call::stall_timeout`]), and connecting is bounded by
//! [`CONNECT_TIMEOUT`]. [`Call::no_timeout`] keeps only the connect bound (a
//! long server-side copy that answers when done).
//!
//! **Session secret.** The supervisor registers the bundled server's origin
//! with its per-session secret ([`register_session`]); every request to that
//! origin, from any `Http`, carries it as [`SESSION_HEADER`], and no other
//! request does.
//!
//! ```no_run
//! use bato_shell::http::{Http, Method};
//! use std::time::Duration;
//! #[derive(serde::Deserialize)]
//! struct Note { id: u64, title: String }
//!
//! let api = Http::new("http://127.0.0.1:42100").with_bearer(Some("token".into()));
//! // JSON, as before.
//! let notes: Vec<Note> = api.get("/api/notes")?;
//! let created: Note = api.post("/api/notes", &serde_json::json!({ "title": "Hi" }))?;
//! // Per call: a long operation, a header, a byte range.
//! let _: serde_json::Value = api
//!     .call(Method::Post, "/api/fs/copy")
//!     .json(&serde_json::json!({ "from": "/a", "to": "/b" }))
//!     .timeout(Duration::from_secs(600))
//!     .decode()?;
//! let head: Vec<u8> = api.call(Method::Get, "/api/fs/read?path=/x").range(0, Some(262_143)).bytes()?;
//! // Server-Sent Events, one at a time as they arrive.
//! for ev in api.open("/api/fs/search?root=/&q=x")?.sse() {
//!     let Ok(ev) = ev else { break };
//!     if ev.event == "done" { break; }
//! }
//! # Ok::<(), bato_shell::http::HttpError>(())
//! ```

mod relay;
mod sse;

use std::io::Read;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

pub use relay::{Cancel, PROGRESS_INTERVAL, Progress, RelayError, Relayed, relay};
pub use sse::{SseEvent, SseReader};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Connecting (DNS, TCP, TLS) for streams and calls without a total budget.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// A stream or streamed upload fails when no byte moves for this long.
pub const STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// [`Call::bytes`] refuses bodies larger than this; stream bigger ones.
pub const MAX_BYTES: u64 = 64 << 20;
/// The bundled server's per-session secret travels in this header
/// (`BATO_SESSION_SECRET` on the server side).
pub const SESSION_HEADER: &str = "X-Bato-Session";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as JSON; an empty or non-JSON body reads as `null`, as in
    /// `bridge.ts` `apiFetch`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, HttpError> {
        let v: Value = serde_json::from_str(&self.body).unwrap_or(Value::Null);
        serde_json::from_value(v).map_err(|e| HttpError::Decode(e.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// No response: DNS, refused, TLS, timeout.
    Transport(String),
    /// A non-2xx answer. `message` is the body's `{"error": …}` when there is
    /// one, else `Request failed: <status>` (`bridge.ts` `parseError`).
    Status { status: u16, message: String },
    /// A 2xx answer whose body does not fit the requested type.
    Decode(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => f.write_str(e),
            Self::Status { message, .. } => f.write_str(message),
            Self::Decode(e) => write!(f, "unexpected response: {e}"),
        }
    }
}

impl std::error::Error for HttpError {}

/// A base URL plus optional Bearer token and default headers. Cheap to
/// clone; all instances share one connection pool.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Http {
    base: String,
    bearer: Option<String>,
    timeout: Duration,
    headers: Vec<(String, String)>,
}

impl Http {
    /// `base` is normalised (trimmed, no trailing slash). Paths are appended
    /// verbatim, so they start with `/`. An absolute `http(s)://` path is used
    /// as is.
    pub fn new(base: &str) -> Self {
        Self {
            base: crate::backend::normalize(base),
            bearer: None,
            timeout: DEFAULT_TIMEOUT,
            headers: Vec::new(),
        }
    }

    pub fn with_bearer(mut self, token: Option<String>) -> Self {
        self.bearer = token.filter(|t| !t.is_empty());
        self
    }

    /// The total budget of every call from this client that sets none.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// A header sent with every request from this client (replaces an
    /// earlier one of the same name).
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        set_header(&mut self.headers, name, value);
        self
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}{path}", self.base)
        }
    }

    /// A request to build (headers, body, timeout), then [`Call::send`],
    /// [`Call::decode`], [`Call::bytes`] or [`Call::stream`].
    pub fn call(&self, method: Method, path: &str) -> Call {
        let mut headers = self.headers.clone();
        if let Some(t) = &self.bearer {
            set_header(&mut headers, "Authorization", &format!("Bearer {t}"));
        }
        Call {
            method,
            url: self.url(path),
            headers,
            limit: None,
            default_total: self.timeout,
            body: Body::None,
        }
    }

    /// One request, any status. `Err` only when no response arrived.
    pub fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Response, HttpError> {
        let mut call = self.call(method, path);
        if let Some(b) = body {
            call = call.json(b);
        }
        call.send()
    }

    /// `send` + status check + JSON decode.
    pub fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T, HttpError> {
        let r = self.send(method, path, body)?;
        if !r.ok() {
            return Err(status_error(&r));
        }
        r.json()
    }

    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        self.request(Method::Get, path, None)
    }

    pub fn post<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, HttpError> {
        self.request(Method::Post, path, Some(&to_value(body)?))
    }

    pub fn put<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, HttpError> {
        self.request(Method::Put, path, Some(&to_value(body)?))
    }

    pub fn patch<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, HttpError> {
        self.request(Method::Patch, path, Some(&to_value(body)?))
    }

    pub fn delete<T: DeserializeOwned>(&self, path: &str) -> Result<T, HttpError> {
        self.request(Method::Delete, path, None)
    }

    /// `GET path` as bytes, 2xx only (206 included), at most [`MAX_BYTES`].
    pub fn get_bytes(&self, path: &str) -> Result<Vec<u8>, HttpError> {
        self.call(Method::Get, path).bytes()
    }

    /// `GET path` as a stream, 2xx only; the body is read as you go.
    pub fn open(&self, path: &str) -> Result<Streamed, HttpError> {
        self.call(Method::Get, path).stream()?.ok_or_status()
    }

    /// `GET /api/health` answers 2xx (`bridge.ts` `testConnection`).
    pub fn health(&self) -> bool {
        matches!(self.send(Method::Get, "/api/health", None), Ok(r) if r.ok())
    }

    /// `GET /api/info` → `version`.
    pub fn info_version(&self) -> Option<String> {
        let v: Value = self.get("/api/info").ok()?;
        v.get("version")?.as_str().map(str::to_string)
    }
}

/// How long a call may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Limit {
    /// The whole exchange, body included.
    Total(Duration),
    /// Connect within [`CONNECT_TIMEOUT`], then fail only when one read or
    /// write waits this long.
    Stall(Duration),
    /// Connect within [`CONNECT_TIMEOUT`], then wait as long as it takes.
    Unbounded,
}

enum Body {
    None,
    Json(Result<Vec<u8>, String>),
    Bytes(Vec<u8>, String),
    Reader(Box<dyn Read + Send>, Option<u64>, String),
}

/// One request being built ([`Http::call`]).
#[must_use = "a Call does nothing until it is sent"]
pub struct Call {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    limit: Option<Limit>,
    default_total: Duration,
    body: Body,
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Call")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl Call {
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Set a header (replaces an earlier one of the same name, the client's
    /// defaults included).
    pub fn header(mut self, name: &str, value: &str) -> Self {
        set_header(&mut self.headers, name, value);
        self
    }

    /// `Authorization: Bearer <token>` for this call; `None` or empty removes
    /// the client's.
    pub fn bearer(mut self, token: Option<&str>) -> Self {
        self.headers
            .retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            self.headers
                .push(("Authorization".into(), format!("Bearer {t}")));
        }
        self
    }

    /// `Range: bytes=start-end` (`end` inclusive; `None`: to the end).
    pub fn range(self, start: u64, end: Option<u64>) -> Self {
        let v = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };
        self.header("Range", &v)
    }

    /// The whole exchange must finish within `d`, body included.
    pub fn timeout(mut self, d: Duration) -> Self {
        self.limit = Some(Limit::Total(d));
        self
    }

    /// No total budget; fail when one read or write waits `d`. The default
    /// for [`stream`](Self::stream) and [`body_reader`](Self::body_reader)
    /// is [`STALL_TIMEOUT`].
    pub fn stall_timeout(mut self, d: Duration) -> Self {
        self.limit = Some(Limit::Stall(d));
        self
    }

    /// No limit once connected.
    pub fn no_timeout(mut self) -> Self {
        self.limit = Some(Limit::Unbounded);
        self
    }

    /// A JSON body (`Content-Type: application/json`).
    pub fn json<B: Serialize + ?Sized>(mut self, body: &B) -> Self {
        self.body = Body::Json(serde_json::to_vec(body).map_err(|e| e.to_string()));
        self
    }

    /// A byte body.
    pub fn body_bytes(mut self, bytes: Vec<u8>, content_type: &str) -> Self {
        self.body = Body::Bytes(bytes, content_type.into());
        self
    }

    /// A streamed body, read while it is sent: chunked unless `len` is known.
    pub fn body_reader(
        mut self,
        reader: impl Read + Send + 'static,
        len: Option<u64>,
        content_type: &str,
    ) -> Self {
        self.body = Body::Reader(Box::new(reader), len, content_type.into());
        self
    }

    /// Send; any status. The body is read as text (at most 10 MB).
    pub fn send(self) -> Result<Response, HttpError> {
        let mut resp = self.accept_json().execute(None)?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| HttpError::Transport(transport_message(&e)))?;
        Ok(Response { status, body })
    }

    /// Send, check for 2xx, decode the JSON body.
    pub fn decode<T: DeserializeOwned>(self) -> Result<T, HttpError> {
        let r = self.send()?;
        if !r.ok() {
            return Err(status_error(&r));
        }
        r.json()
    }

    /// Send, check for 2xx (206 included), return the body's bytes (at most
    /// [`MAX_BYTES`]). The client's total budget applies unless the call set
    /// another limit.
    pub fn bytes(self) -> Result<Vec<u8>, HttpError> {
        let resp = self.execute(None)?;
        Streamed::from_response(resp)
            .ok_or_status()?
            .read_all(MAX_BYTES)
    }

    /// Send and return once the headers are in; read the body from the
    /// result. Any status: see [`Streamed::ok_or_status`].
    pub fn stream(self) -> Result<Streamed, HttpError> {
        let resp = self.execute(Some(Limit::Stall(STALL_TIMEOUT)))?;
        Ok(Streamed::from_response(resp))
    }

    fn accept_json(mut self) -> Self {
        if !self
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("accept"))
        {
            self.headers
                .push(("Accept".into(), "application/json".into()));
        }
        self
    }

    /// `stream_default`: the limit for a stream when the call set none.
    /// Otherwise a reader body gets a stall limit, anything else the
    /// client's total budget.
    fn execute(
        self,
        stream_default: Option<Limit>,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        let reader_body = matches!(self.body, Body::Reader(..));
        let fallback = match stream_default {
            Some(l) => l,
            None if reader_body => Limit::Stall(STALL_TIMEOUT),
            None => Limit::Total(self.default_total),
        };
        let (parts, body) = self.into_parts(fallback);
        match body {
            Body::None => parts.run(ureq::SendBody::none(), None, None),
            Body::Json(Ok(b)) => parts.run_bytes(b, "application/json"),
            Body::Json(Err(e)) => Err(HttpError::Decode(e)),
            Body::Bytes(b, ct) => parts.run_bytes(b, &ct),
            Body::Reader(mut r, len, ct) => {
                parts.run(ureq::SendBody::from_reader(&mut r), len, Some(&ct))
            }
        }
    }

    /// Everything but the body, and the body.
    fn into_parts(self, fallback: Limit) -> (Parts, Body) {
        (
            Parts {
                method: self.method,
                url: self.url,
                headers: self.headers,
                limit: self.limit.unwrap_or(fallback),
            },
            self.body,
        )
    }
}

/// A call without its body.
struct Parts {
    method: Method,
    url: String,
    headers: Vec<(String, String)>,
    limit: Limit,
}

impl Parts {
    fn run_bytes(
        self,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        let len = bytes.len() as u64;
        let mut cursor = std::io::Cursor::new(bytes);
        self.run(
            ureq::SendBody::from_reader(&mut cursor),
            Some(len),
            Some(content_type),
        )
    }

    /// `content_type: Some` marks a body; `len: None` then sends it chunked.
    fn run(
        self,
        body: ureq::SendBody<'_>,
        len: Option<u64>,
        content_type: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, HttpError> {
        let agent = agent();
        let has = |name: &str| {
            self.headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case(name))
        };
        let mut b = ureq::http::Request::builder()
            .method(self.method.as_str())
            .uri(&self.url);
        for (k, v) in &self.headers {
            b = b.header(k, v);
        }
        if !has(SESSION_HEADER)
            && let Some(secret) = session_for(&self.url)
        {
            b = b.header(SESSION_HEADER, secret);
        }
        if let Some(ct) = content_type {
            if !has("content-type") {
                b = b.header("Content-Type", ct);
            }
            if let Some(n) = len {
                b = b.header("Content-Length", n.to_string());
            }
        }
        let req = b
            .body(body)
            .map_err(|e| HttpError::Transport(format!("invalid request: {e}")))?;
        let cfg = agent.configure_request(req);
        let req = match self.limit {
            Limit::Total(d) => cfg.timeout_global(Some(d)),
            Limit::Stall(d) => cfg
                .timeout_global(None)
                .timeout_connect(Some(CONNECT_TIMEOUT))
                .timeout_per_read(Some(d))
                .timeout_per_write(Some(d)),
            Limit::Unbounded => cfg
                .timeout_global(None)
                .timeout_connect(Some(CONNECT_TIMEOUT)),
        }
        .build();
        agent
            .run(req)
            .map_err(|e| HttpError::Transport(transport_message(&e)))
    }
}

/// A response whose body has not been read yet ([`Call::stream`]).
pub struct Streamed {
    status: u16,
    headers: Vec<(String, String)>,
    reader: Box<dyn Read + Send>,
}

impl std::fmt::Debug for Streamed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Streamed")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

impl Streamed {
    fn from_response(resp: ureq::http::Response<ureq::Body>) -> Self {
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    String::from_utf8_lossy(v.as_bytes()).into_owned(),
                )
            })
            .collect();
        let reader = resp.into_body().into_with_config().limit(u64::MAX).reader();
        Self {
            status,
            headers,
            reader: Box::new(reader),
        }
    }

    /// A stream over any reader (adapters, tests).
    pub fn from_reader(
        status: u16,
        headers: Vec<(String, String)>,
        reader: impl Read + Send + 'static,
    ) -> Self {
        Self {
            status,
            headers,
            reader: Box::new(reader),
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// A response header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// `Content-Length`, when the server sent one.
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length")?.trim().parse().ok()
    }

    /// Itself when 2xx; else the error with the server's message (up to
    /// 64 KiB of the body is read for it).
    pub fn ok_or_status(mut self) -> Result<Self, HttpError> {
        if self.ok() {
            return Ok(self);
        }
        let mut body = Vec::new();
        let _ = (&mut self.reader).take(64 << 10).read_to_end(&mut body);
        Err(status_error(&Response {
            status: self.status,
            body: String::from_utf8_lossy(&body).into_owned(),
        }))
    }

    /// The rest of the body, at most `limit` bytes (more is an error).
    pub fn read_all(&mut self, limit: u64) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        (&mut self.reader)
            .take(limit.saturating_add(1))
            .read_to_end(&mut out)
            .map_err(|e| HttpError::Transport(e.to_string()))?;
        if out.len() as u64 > limit {
            return Err(HttpError::Transport(format!(
                "response larger than {limit} bytes"
            )));
        }
        Ok(out)
    }

    /// The body as a reader.
    pub fn into_reader(self) -> Box<dyn Read + Send> {
        self.reader
    }

    /// The body as Server-Sent Events.
    pub fn sse(self) -> SseReader<std::io::BufReader<Box<dyn Read + Send>>> {
        SseReader::new(std::io::BufReader::new(self.reader))
    }

    /// Copy the body into `out` with progress (at most every
    /// [`PROGRESS_INTERVAL`], plus a final one) until done or cancelled. A
    /// download to a file: `File::create` + `copy_to`.
    pub fn copy_to(
        self,
        out: &mut dyn std::io::Write,
        cancel: &Cancel,
        on_progress: impl FnMut(Progress),
    ) -> Result<u64, RelayError> {
        relay::copy_to(self, out, cancel, on_progress)
    }
}

impl Read for Streamed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

fn set_header(headers: &mut Vec<(String, String)>, name: &str, value: &str) {
    headers.retain(|(k, _)| !k.eq_ignore_ascii_case(name));
    headers.push((name.to_string(), value.to_string()));
}

fn to_value<B: Serialize + ?Sized>(body: &B) -> Result<Value, HttpError> {
    serde_json::to_value(body).map_err(|e| HttpError::Decode(e.to_string()))
}

/// `bridge.ts` `parseError`: the body's `error`, else `Request failed: <status>`.
pub fn status_error(r: &Response) -> HttpError {
    let message = serde_json::from_str::<Value>(&r.body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("Request failed: {}", r.status));
    HttpError::Status {
        status: r.status,
        message,
    }
}

fn transport_message(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "timed out".to_string(),
        ureq::Error::HostNotFound => "host not found".to_string(),
        ureq::Error::Io(io) => io.to_string(),
        other => other.to_string(),
    }
}

// ── Session secrets ─────────────────────────────────────────────────────────

static SESSIONS: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

/// `scheme://host:port` of a URL, lowercased; `None` for anything else.
fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.trim().split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}").to_ascii_lowercase())
}

/// Send `secret` as [`SESSION_HEADER`] with every request to `url`'s origin
/// (the supervisor does this for the bundled server). Replaces an earlier
/// secret for that origin.
pub fn register_session(url: &str, secret: &str) {
    let Some(o) = origin(url) else { return };
    let mut s = SESSIONS.write().unwrap_or_else(|e| e.into_inner());
    s.retain(|(k, _)| *k != o);
    s.push((o, secret.to_string()));
}

/// Stop sending a secret to `url`'s origin.
pub fn unregister_session(url: &str) {
    let Some(o) = origin(url) else { return };
    SESSIONS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(k, _)| *k != o);
}

/// The secret registered for `url`'s origin.
pub fn session_for(url: &str) -> Option<String> {
    let o = origin(url)?;
    SESSIONS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| *k == o)
        .map(|(_, v)| v.clone())
}

/// The process-wide agent: platform trust store, status codes as values,
/// the default timeout.
fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        Agent::new_with_config(
            Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(DEFAULT_TIMEOUT))
                .tls_config(tls)
                .user_agent(concat!("bato-shell/", env!("CARGO_PKG_VERSION")))
                .build(),
        )
    })
}

#[cfg(test)]
pub(crate) mod test_server;

#[cfg(test)]
mod tests {
    use super::test_server::{Reply, serve, serve_once};
    use super::*;

    #[test]
    fn get_json_with_bearer() {
        let (base, rx) = serve_once(200, r#"{"version":"1.2.3"}"#);
        let api = Http::new(&format!("{base}/")).with_bearer(Some("tok".into()));
        assert_eq!(api.info_version().as_deref(), Some("1.2.3"));
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("GET /api/info HTTP/1.1"), "{req:?}");
        assert_eq!(req.header("authorization"), Some("Bearer tok"));
        assert_eq!(req.header("accept"), Some("application/json"));
    }

    #[test]
    fn post_sends_json_body() {
        let (base, rx) = serve_once(201, r#"{"id":7}"#);
        let v: Value = Http::new(&base)
            .post("/api/notes", &serde_json::json!({"title": "Hi"}))
            .unwrap();
        assert_eq!(v["id"], 7);
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /api/notes"), "{req:?}");
        assert_eq!(req.header("content-type"), Some("application/json"));
        assert_eq!(req.body, br#"{"title":"Hi"}"#);
    }

    #[test]
    fn post_without_body() {
        let (base, rx) = serve_once(200, "{}");
        let r = Http::new(&base)
            .send(Method::Post, "/api/refresh", None)
            .unwrap();
        assert!(r.ok());
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /api/refresh"), "{req:?}");
        assert!(req.body.is_empty());
    }

    #[test]
    fn put_and_delete() {
        let (base, rx) = serve_once(200, "");
        let v: Value = Http::new(&base).put("/api/x/1", &[1, 2]).unwrap();
        assert_eq!(v, Value::Null, "empty body reads as null");
        assert!(rx.recv().unwrap().head.starts_with("PUT /api/x/1"));
        let (base, rx) = serve_once(204, "");
        let _: Value = Http::new(&base).delete("/api/x/1").unwrap();
        assert!(rx.recv().unwrap().head.starts_with("DELETE /api/x/1"));
    }

    #[test]
    fn status_errors_carry_the_server_message() {
        let (base, _rx) = serve_once(409, r#"{"error":"name taken"}"#);
        let e = Http::new(&base).get::<Value>("/api/x").unwrap_err();
        assert_eq!(
            e,
            HttpError::Status {
                status: 409,
                message: "name taken".into()
            }
        );
        let (base, _rx) = serve_once(500, "oops");
        let e = Http::new(&base).get::<Value>("/api/x").unwrap_err();
        assert_eq!(e.to_string(), "Request failed: 500");
    }

    #[test]
    fn refused_is_a_transport_error_not_a_hang() {
        // Nothing listens on port 1.
        let e = Http::new("http://127.0.0.1:1")
            .get::<Value>("/api/health")
            .unwrap_err();
        assert!(matches!(e, HttpError::Transport(_)), "{e:?}");
        assert!(!Http::new("http://127.0.0.1:1").health());
    }

    #[test]
    fn timeout_applies() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let t = std::time::Instant::now();
        let e = Http::new(&base)
            .with_timeout(Duration::from_millis(300))
            .get::<Value>("/slow")
            .unwrap_err();
        assert!(matches!(e, HttpError::Transport(_)));
        // Not a hang (a loose bound: a loaded machine adds seconds).
        assert!(t.elapsed() < Duration::from_secs(20));
        drop(l);
    }

    #[test]
    fn per_call_timeout_outlasts_the_client_default() {
        // Answers after 600 ms: the client's 200 ms budget fails, the call's
        // 10 s one does not.
        let slow = || Reply::new(200, r#"{"ok":true}"#).delay(Duration::from_millis(600));
        let (base, _rx) = serve(vec![slow(), slow()]);
        let api = Http::new(&base).with_timeout(Duration::from_millis(200));
        assert!(matches!(
            api.get::<Value>("/api/fs/copy"),
            Err(HttpError::Transport(_))
        ));
        let v: Value = api
            .call(Method::Post, "/api/fs/copy")
            .timeout(Duration::from_secs(10))
            .decode()
            .unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn stall_timeout_ends_a_silent_stream() {
        // Headers and 3 of 100 bytes, then silence.
        let (base, _rx) = serve(vec![Reply::new(200, "abc").content_length(100).hang()]);
        let mut s = Http::new(&base)
            .call(Method::Get, "/s")
            .stall_timeout(Duration::from_millis(300))
            .stream()
            .unwrap();
        let t = std::time::Instant::now();
        let mut buf = Vec::new();
        assert!(s.read_to_end(&mut buf).is_err());
        assert_eq!(buf, b"abc");
        assert!(t.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn headers_range_and_bytes() {
        let (base, rx) = serve_once(206, "0123");
        let api = Http::new(&base)
            .with_header("X-Client", "a")
            .with_header("x-client", "b");
        let got = api
            .call(Method::Get, "/api/fs/read?path=/x")
            .range(0, Some(3))
            .header("X-Trace", "1")
            .bytes()
            .unwrap();
        assert_eq!(got, b"0123");
        let req = rx.recv().unwrap();
        assert_eq!(req.header("range"), Some("bytes=0-3"));
        assert_eq!(req.header("x-client"), Some("b"), "replaced, not doubled");
        assert_eq!(req.header_count("x-client"), 1);
        assert_eq!(req.header("x-trace"), Some("1"));
        assert_ne!(
            req.header("accept"),
            Some("application/json"),
            "bytes are not JSON"
        );
        // Open-ended range, a per-call bearer replacing the client's.
        let (base, rx) = serve_once(200, "");
        Http::new(&base)
            .with_bearer(Some("client".into()))
            .call(Method::Get, "/x")
            .range(10, None)
            .bearer(Some("call"))
            .bytes()
            .unwrap();
        let req = rx.recv().unwrap();
        assert_eq!(req.header("range"), Some("bytes=10-"));
        assert_eq!(req.header("authorization"), Some("Bearer call"));
        assert_eq!(req.header_count("authorization"), 1);
    }

    #[test]
    fn bytes_reports_status_errors() {
        let (base, _rx) = serve_once(404, r#"{"error":"no such file"}"#);
        let e = Http::new(&base).get_bytes("/x").unwrap_err();
        assert_eq!(e.to_string(), "no such file");
    }

    #[test]
    fn byte_and_streamed_bodies_go_out() {
        let (base, rx) = serve_once(200, r#"{"ok":true}"#);
        let r = Http::new(&base)
            .call(Method::Put, "/api/fs/write?path=/a")
            .body_bytes(vec![0, 1, 2, 255], "application/octet-stream")
            .send()
            .unwrap();
        assert!(r.ok());
        let req = rx.recv().unwrap();
        assert_eq!(req.body, [0, 1, 2, 255]);
        assert_eq!(req.header("content-length"), Some("4"));
        assert_eq!(req.header("content-type"), Some("application/octet-stream"));

        // Unknown length: chunked.
        let (base, rx) = serve_once(200, r#"{"ok":true}"#);
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let v: Value = Http::new(&base)
            .call(Method::Put, "/api/fs/write?path=/b")
            .body_reader(
                std::io::Cursor::new(data.clone()),
                None,
                "application/octet-stream",
            )
            .decode()
            .unwrap();
        assert_eq!(v["ok"], true);
        let req = rx.recv().unwrap();
        assert_eq!(req.header("transfer-encoding"), Some("chunked"));
        assert_eq!(req.body, data);
    }

    #[test]
    fn streams_read_as_they_arrive() {
        let (base, _rx) = serve_once(200, "hello stream");
        let s = Http::new(&base).open("/big").unwrap();
        assert_eq!(s.content_length(), Some(12));
        assert_eq!(s.header("Content-Type"), Some("application/json"));
        let mut out = String::new();
        s.into_reader().read_to_string(&mut out).unwrap();
        assert_eq!(out, "hello stream");
        let (base, _rx) = serve_once(403, r#"{"error":"forbidden"}"#);
        assert_eq!(
            Http::new(&base).open("/big").unwrap_err().to_string(),
            "forbidden"
        );
    }

    #[test]
    fn session_secret_goes_to_its_origin_only() {
        let (base, rx) = serve_once(200, "{}");
        register_session(&format!("{base}/"), "s3cret");
        let _ = Http::new(&base).health();
        assert_eq!(rx.recv().unwrap().header("x-bato-session"), Some("s3cret"));
        // Another origin (another port) gets nothing.
        let (other, rx2) = serve_once(200, "{}");
        let _ = Http::new(&other).health();
        assert_eq!(rx2.recv().unwrap().header("x-bato-session"), None);
        unregister_session(&base);
        assert_eq!(session_for(&base), None);
        assert_eq!(
            origin("HTTP://127.0.0.1:5/x?y"),
            Some("http://127.0.0.1:5".into())
        );
        assert_eq!(origin("http://h"), Some("http://h".into()));
        assert_eq!(origin("nope"), None);
    }

    /// The real registry over TLS with the mkcert root from the system store.
    /// LAN only: `cargo test -p bato-shell -- --ignored registry_over_tls`.
    #[test]
    #[ignore = "needs https://registry.bato.lan"]
    fn registry_over_tls() {
        let v: Value = Http::new("https://registry.bato.lan")
            .get("/api/apps")
            .unwrap();
        assert!(v.as_array().is_some_and(|a| !a.is_empty()));
    }
}
