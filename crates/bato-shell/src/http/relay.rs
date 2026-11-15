//! The transfer engine's core: stream a GET into a PUT (BatoBrowse
//! `transfer.js` `copyFile`), counting bytes, with throttled progress and
//! cancellation, and the same for a download into any writer.
//!
//! Every byte passes through this process once, in a 64 KiB buffer: nothing
//! is held in memory or spooled to disk. The PUT is sent with the source's
//! `Content-Length` when it has one, else chunked (what `transfer.js` did
//! with `duplex: 'half'`).
//!
//! Cancelling ([`Cancel::cancel`], from any thread) stops at the next read:
//! the PUT is aborted mid-body, so the destination sees a reset connection
//! and is left with a partial file (as in Electron; the caller decides
//! whether to delete it).

use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{Call, HttpError, Response, STALL_TIMEOUT, Streamed, status_error};

/// Progress is reported at most this often (plus once at the end), as
/// `transfer.js`'s emitter.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// A cancellation flag shared between the UI and a worker.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Bytes moved so far, and the total when the source announced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
}

impl Progress {
    /// `done / total` in 0..=1, when the total is known.
    pub fn fraction(&self) -> Option<f32> {
        match self.total {
            Some(0) => Some(1.0),
            Some(t) => Some((self.done as f64 / t as f64).min(1.0) as f32),
            None => None,
        }
    }
}

/// A finished relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relayed {
    pub bytes: u64,
    /// The destination's answer (2xx), e.g. `{"ok":true}`.
    pub response: Response,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayError {
    Cancelled,
    /// The source failed: a non-2xx answer, or the stream broke or stalled.
    Source(HttpError),
    /// The destination failed: refused, a non-2xx answer, or a write error.
    Dest(HttpError),
}

impl std::fmt::Display for RelayError {
    /// `transfer.js`'s messages: `read failed: 404`, `write failed: <server
    /// error>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("cancelled"),
            Self::Source(HttpError::Status { status, .. }) => write!(f, "read failed: {status}"),
            Self::Source(e) => write!(f, "read failed: {e}"),
            Self::Dest(e) => write!(f, "write failed: {e}"),
        }
    }
}

impl std::error::Error for RelayError {}

/// A reader that counts, reports and can be cancelled. It remembers its own
/// read error, so a failed PUT can say which side broke.
struct Counting<'a, R> {
    inner: R,
    cancel: &'a Cancel,
    progress: Progress,
    last: Instant,
    on_progress: &'a mut dyn FnMut(Progress),
    source_error: Option<String>,
}

impl<'a, R: Read> Counting<'a, R> {
    fn new(
        inner: R,
        total: Option<u64>,
        cancel: &'a Cancel,
        on_progress: &'a mut dyn FnMut(Progress),
    ) -> Self {
        Self {
            inner,
            cancel,
            progress: Progress { done: 0, total },
            last: Instant::now(),
            on_progress,
            source_error: None,
        }
    }

    fn emit(&mut self) {
        self.last = Instant::now();
        (self.on_progress)(self.progress);
    }
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::other("cancelled"));
        }
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => return Err(e),
            Err(e) => {
                self.source_error = Some(e.to_string());
                return Err(e);
            }
        };
        self.progress.done += n as u64;
        if n > 0 && self.last.elapsed() >= PROGRESS_INTERVAL {
            self.emit();
        }
        Ok(n)
    }
}

/// Stream `source` (a GET) into `dest` (a PUT or POST; its own body is
/// replaced by the stream), reporting [`Progress`] at most every
/// [`PROGRESS_INTERVAL`] and once at the end.
///
/// Both sides default to a stall limit ([`STALL_TIMEOUT`]) and no total
/// budget, whatever the clients' defaults; set one on either call to change
/// it. The source must answer 2xx (206 counts).
///
/// ```no_run
/// use bato_shell::http::{Cancel, Http, Method, relay};
/// let (src, dst) = (Http::new("http://nas.lan:42110"), Http::new("http://127.0.0.1:40123"));
/// let cancel = Cancel::new(); // keep a clone in the UI for "Stop"
/// let done = relay(
///     src.call(Method::Get, "/api/fs/read?path=/films/a.mkv").bearer(Some("tok")),
///     dst.call(Method::Put, "/api/fs/write?path=/home/me/a.mkv"),
///     &cancel,
///     |p| eprintln!("{} of {:?}", p.done, p.total),
/// )?;
/// # Ok::<(), bato_shell::http::RelayError>(())
/// ```
pub fn relay(
    source: Call,
    dest: Call,
    cancel: &Cancel,
    mut on_progress: impl FnMut(Progress),
) -> Result<Relayed, RelayError> {
    if cancel.is_cancelled() {
        return Err(RelayError::Cancelled);
    }
    let src = source
        .stream()
        .and_then(Streamed::ok_or_status)
        .map_err(RelayError::Source)?;
    let total = src.content_length();
    let content_type = src
        .header("content-type")
        .unwrap_or("application/octet-stream")
        .to_string();
    let (parts, _) = dest.into_parts(super::Limit::Stall(STALL_TIMEOUT));
    let mut counting = Counting::new(src, total, cancel, &mut on_progress);
    counting.emit();
    let sent = parts.run(
        ureq::SendBody::from_reader(&mut counting),
        total,
        Some(&content_type),
    );
    let failed = |e: HttpError, counting: &Counting<'_, Streamed>| {
        if cancel.is_cancelled() {
            RelayError::Cancelled
        } else if let Some(s) = &counting.source_error {
            RelayError::Source(HttpError::Transport(s.clone()))
        } else {
            RelayError::Dest(e)
        }
    };
    let mut resp = match sent {
        Ok(r) => r,
        Err(e) => return Err(failed(e, &counting)),
    };
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    let response = Response { status, body };
    if !response.ok() {
        // A destination that answered early (413, 403) may have stopped
        // reading; the answer is what counts.
        return Err(RelayError::Dest(status_error(&response)));
    }
    if let Some(s) = counting.source_error.take() {
        return Err(RelayError::Source(HttpError::Transport(s)));
    }
    counting.emit();
    Ok(Relayed {
        bytes: counting.progress.done,
        response,
    })
}

/// [`Streamed::copy_to`].
pub(super) fn copy_to(
    src: Streamed,
    out: &mut dyn Write,
    cancel: &Cancel,
    mut on_progress: impl FnMut(Progress),
) -> Result<u64, RelayError> {
    let total = src.content_length();
    let mut counting = Counting::new(src, total, cancel, &mut on_progress);
    counting.emit();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        let n = match counting.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(if cancel.is_cancelled() {
                    RelayError::Cancelled
                } else {
                    RelayError::Source(HttpError::Transport(e.to_string()))
                });
            }
        };
        out.write_all(&buf[..n])
            .map_err(|e| RelayError::Dest(HttpError::Transport(e.to_string())))?;
    }
    out.flush()
        .map_err(|e| RelayError::Dest(HttpError::Transport(e.to_string())))?;
    counting.emit();
    Ok(counting.progress.done)
}

#[cfg(test)]
mod tests {
    use super::super::test_server::{Reply, serve};
    use super::super::{Http, Method};
    use super::*;

    fn payload(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 7 % 256) as u8).collect()
    }

    #[test]
    fn relays_get_into_put_with_progress() {
        let data = payload(3 << 20);
        let (src, src_rx) = serve(vec![
            Reply::new(200, &data).content_type("application/octet-stream"),
        ]);
        let (dst, dst_rx) = serve(vec![Reply::new(200, r#"{"ok":true}"#)]);
        let mut seen = Vec::new();
        let done = relay(
            Http::new(&src)
                .call(Method::Get, "/api/fs/read?path=/a")
                .bearer(Some("src-token")),
            Http::new(&dst)
                .with_bearer(Some("dst-token".into()))
                .call(Method::Put, "/api/fs/write?path=/b"),
            &Cancel::new(),
            |p| seen.push(p),
        )
        .unwrap();
        assert_eq!(done.bytes, data.len() as u64);
        assert_eq!(done.response.body, r#"{"ok":true}"#);
        let got = dst_rx.recv().unwrap();
        assert!(got.head.starts_with("PUT /api/fs/write?path=/b"));
        assert_eq!(got.header("authorization"), Some("Bearer dst-token"));
        assert_eq!(got.header("content-length"), Some("3145728"));
        assert_eq!(got.header("content-type"), Some("application/octet-stream"));
        assert!(got.body == data, "relayed bytes differ");
        assert_eq!(
            src_rx.recv().unwrap().header("authorization"),
            Some("Bearer src-token")
        );
        let total = Some(data.len() as u64);
        assert_eq!(seen.first(), Some(&Progress { done: 0, total }));
        assert_eq!(
            seen.last(),
            Some(&Progress {
                done: data.len() as u64,
                total
            })
        );
        assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
    }

    #[test]
    fn cancel_stops_mid_stream() {
        // 64 KiB, then a pause long enough to cancel in, then more.
        let (src, _) = serve(vec![
            Reply::new(200, payload(64 << 10))
                .then(Duration::from_millis(800), payload(64 << 10))
                .then(Duration::from_millis(800), payload(64 << 10)),
        ]);
        let (dst, _) = serve(vec![Reply::new(200, "{}")]);
        let cancel = Cancel::new();
        let c2 = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c2.cancel();
        });
        let e = relay(
            Http::new(&src).call(Method::Get, "/a"),
            Http::new(&dst).call(Method::Put, "/b"),
            &cancel,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(e, RelayError::Cancelled);
        assert_eq!(
            relay(
                Http::new(&src).call(Method::Get, "/a"),
                Http::new(&dst).call(Method::Put, "/b"),
                &cancel,
                |_| {}
            ),
            Err(RelayError::Cancelled),
            "an already cancelled job does not start"
        );
    }

    #[test]
    fn errors_name_the_failing_side() {
        let (src, _) = serve(vec![Reply::new(404, r#"{"error":"not found"}"#)]);
        let e = relay(
            Http::new(&src).call(Method::Get, "/a"),
            Http::new("http://127.0.0.1:1").call(Method::Put, "/b"),
            &Cancel::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "read failed: 404");

        let (src, _) = serve(vec![Reply::new(200, "abc")]);
        let (dst, _) = serve(vec![Reply::new(403, r#"{"error":"read-only"}"#)]);
        let e = relay(
            Http::new(&src).call(Method::Get, "/a"),
            Http::new(&dst).call(Method::Put, "/b"),
            &Cancel::new(),
            |_| {},
        )
        .unwrap_err();
        assert_eq!(e.to_string(), "write failed: read-only");

        // The source promises 100 bytes and stalls after 3.
        let (src, _) = serve(vec![Reply::new(200, "abc").content_length(100).hang()]);
        let (dst, _) = serve(vec![Reply::new(200, "{}")]);
        let e = relay(
            Http::new(&src)
                .call(Method::Get, "/a")
                .stall_timeout(Duration::from_millis(300)),
            Http::new(&dst).call(Method::Put, "/b"),
            &Cancel::new(),
            |_| {},
        )
        .unwrap_err();
        assert!(matches!(e, RelayError::Source(_)), "{e:?}");
    }

    #[test]
    fn download_into_a_writer() {
        let data = payload(200_000);
        let (src, _) = serve(vec![Reply::new(200, &data)]);
        let mut out = Vec::new();
        let mut last = Progress::default();
        let n = Http::new(&src)
            .open("/f")
            .unwrap()
            .copy_to(&mut out, &Cancel::new(), |p| last = p)
            .unwrap();
        assert_eq!(n, 200_000);
        assert!(out == data);
        assert_eq!(last.fraction(), Some(1.0));
    }
}
