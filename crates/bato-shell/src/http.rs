//! JSON over HTTP(S) to the app's backend, bato-auth or the registry.
//!
//! Blocking (ureq 3), meant for worker threads: wrap calls in
//! [`crate::worker::spawn_blocking`] (or `iced_ext::blocking`). TLS is rustls
//! with the platform verifier, so certificates are checked against the system
//! trust store, which is where the mkcert root for `https://*.bato.lan` lives.
//! Every request has a 5 s timeout (the PoC's; Electron's `net:request` had
//! none, so "Test connection" to an unroutable address hung for minutes).
//!
//! ```no_run
//! use bato_shell::http::Http;
//! #[derive(serde::Deserialize)]
//! struct Note { id: u64, title: String }
//!
//! let api = Http::new("http://127.0.0.1:42100").with_bearer(Some("token".into()));
//! let notes: Vec<Note> = api.get("/api/notes")?;
//! let created: Note = api.post("/api/notes", &serde_json::json!({ "title": "Hi" }))?;
//! api.delete::<serde_json::Value>(&format!("/api/notes/{}", created.id))?;
//! # Ok::<(), bato_shell::http::HttpError>(())
//! ```

use std::sync::OnceLock;
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use ureq::Agent;
use ureq::tls::{RootCerts, TlsConfig};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
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

/// A base URL plus optional Bearer token. Cheap to clone; all instances share
/// one connection pool.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Http {
    base: String,
    bearer: Option<String>,
    timeout: Duration,
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
        }
    }

    pub fn with_bearer(mut self, token: Option<String>) -> Self {
        self.bearer = token.filter(|t| !t.is_empty());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
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

    /// One request, any status. `Err` only when no response arrived.
    pub fn send(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Response, HttpError> {
        let url = self.url(path);
        let agent = agent();
        let auth = self.bearer.as_ref().map(|t| format!("Bearer {t}"));
        let body = body.map(Value::to_string);

        macro_rules! prepare {
            ($req:expr) => {{
                let mut r = $req.header("Accept", "application/json");
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                if self.timeout != DEFAULT_TIMEOUT {
                    r = r.config().timeout_global(Some(self.timeout)).build();
                }
                r
            }};
        }
        macro_rules! with_body {
            ($req:expr) => {{
                let r = prepare!($req);
                match &body {
                    Some(b) => r.content_type("application/json").send(b.as_str()),
                    None => r.send_empty(),
                }
            }};
        }

        let result = match method {
            Method::Get => prepare!(agent.get(&url)).call(),
            Method::Delete => prepare!(agent.delete(&url)).call(),
            Method::Post => with_body!(agent.post(&url)),
            Method::Put => with_body!(agent.put(&url)),
            Method::Patch => with_body!(agent.patch(&url)),
        };
        let mut resp = result.map_err(|e| HttpError::Transport(transport_message(&e)))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| HttpError::Transport(transport_message(&e)))?;
        Ok(Response { status, body })
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
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A one-shot HTTP server: answers one request with `status`/`body` and
    /// hands back what it received (request line, headers, body).
    fn serve_once(status: u16, body: &'static str) -> (String, mpsc::Receiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut head = String::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut b = vec![0; len];
            r.read_exact(&mut b).unwrap();
            head.push_str(&String::from_utf8(b).unwrap());
            let mut s = s;
            write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            tx.send(head).unwrap();
        });
        (base, rx)
    }

    #[test]
    fn get_json_with_bearer() {
        let (base, rx) = serve_once(200, r#"{"version":"1.2.3"}"#);
        let api = Http::new(&format!("{base}/")).with_bearer(Some("tok".into()));
        assert_eq!(api.info_version().as_deref(), Some("1.2.3"));
        let req = rx.recv().unwrap();
        assert!(req.starts_with("GET /api/info HTTP/1.1"), "{req}");
        assert!(
            req.contains("authorization: Bearer tok") || req.contains("Authorization: Bearer tok"),
            "{req}"
        );
    }

    #[test]
    fn post_sends_json_body() {
        let (base, rx) = serve_once(201, r#"{"id":7}"#);
        let v: Value = Http::new(&base)
            .post("/api/notes", &serde_json::json!({"title": "Hi"}))
            .unwrap();
        assert_eq!(v["id"], 7);
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /api/notes"), "{req}");
        assert!(
            req.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{req}"
        );
        assert!(req.ends_with(r#"{"title":"Hi"}"#), "{req}");
    }

    #[test]
    fn put_and_delete() {
        let (base, rx) = serve_once(200, "");
        let v: Value = Http::new(&base).put("/api/x/1", &[1, 2]).unwrap();
        assert_eq!(v, Value::Null, "empty body reads as null");
        assert!(rx.recv().unwrap().starts_with("PUT /api/x/1"));
        let (base, rx) = serve_once(204, "");
        let _: Value = Http::new(&base).delete("/api/x/1").unwrap();
        assert!(rx.recv().unwrap().starts_with("DELETE /api/x/1"));
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
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
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
