//! A tiny HTTP/1.1 server for the tests: answers scripted replies in order,
//! one connection each, and hands back what it received.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

/// One received request.
#[derive(Debug, Clone)]
pub struct Received {
    /// Request line and headers, as sent.
    pub head: String,
    /// `(lowercased name, value)`.
    pub headers: Vec<(String, String)>,
    /// The body (de-chunked).
    pub body: Vec<u8>,
}

impl Received {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn header_count(&self, name: &str) -> usize {
        self.headers.iter().filter(|(k, _)| k == name).count()
    }
}

/// A scripted answer.
pub struct Reply {
    status: u16,
    body: Vec<u8>,
    content_length: Option<usize>,
    content_type: &'static str,
    delay: Duration,
    hang: bool,
    /// Body in pieces, with a pause before each.
    chunks: Vec<(Duration, Vec<u8>)>,
}

impl Reply {
    pub fn new(status: u16, body: impl AsRef<[u8]>) -> Self {
        Self {
            status,
            body: body.as_ref().to_vec(),
            content_length: None,
            content_type: "application/json",
            delay: Duration::ZERO,
            hang: false,
            chunks: Vec::new(),
        }
    }

    /// Wait before answering.
    pub fn delay(mut self, d: Duration) -> Self {
        self.delay = d;
        self
    }

    /// Announce this length instead of the body's.
    pub fn content_length(mut self, n: usize) -> Self {
        self.content_length = Some(n);
        self
    }

    pub fn content_type(mut self, ct: &'static str) -> Self {
        self.content_type = ct;
        self
    }

    /// After the body, keep the connection open and silent for a minute.
    pub fn hang(mut self) -> Self {
        self.hang = true;
        self
    }

    /// Then send these pieces, each after its pause (no Content-Length:
    /// the body ends when the connection closes).
    pub fn then(mut self, pause: Duration, piece: impl AsRef<[u8]>) -> Self {
        self.chunks.push((pause, piece.as_ref().to_vec()));
        self
    }
}

/// Answer one request with `status`/`body`.
pub fn serve_once(status: u16, body: &str) -> (String, mpsc::Receiver<Received>) {
    serve(vec![Reply::new(status, body)])
}

/// Answer one request per reply, in order, each on its own connection.
pub fn serve(replies: Vec<Reply>) -> (String, mpsc::Receiver<Received>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let Ok((s, _)) = l.accept() else { return };
            let tx = tx.clone();
            // Each on its own thread, so a hanging reply does not block the next.
            std::thread::spawn(move || answer(s, reply, &tx));
        }
    });
    (base, rx)
}

fn answer(s: TcpStream, reply: Reply, tx: &mpsc::Sender<Received>) {
    let mut r = BufReader::new(s.try_clone().unwrap());
    let Some(req) = read_request(&mut r) else {
        return;
    };
    let _ = tx.send(req);
    std::thread::sleep(reply.delay);
    let mut s = s;
    let len = if reply.chunks.is_empty() {
        format!(
            "Content-Length: {}\r\n",
            reply.content_length.unwrap_or(reply.body.len())
        )
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: {}\r\n{len}Connection: close\r\n\r\n",
        reply.status, reply.content_type
    );
    if s.write_all(head.as_bytes()).is_err() || s.write_all(&reply.body).is_err() {
        return;
    }
    let _ = s.flush();
    for (pause, piece) in reply.chunks {
        std::thread::sleep(pause);
        if s.write_all(&piece).and_then(|_| s.flush()).is_err() {
            return;
        }
    }
    if reply.hang {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Received> {
    let mut head = String::new();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        head.push_str(&line);
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
    let mut body = Vec::new();
    if get("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size).ok()?;
            let n = usize::from_str_radix(size.trim().split(';').next()?, 16).ok()?;
            let mut chunk = vec![0; n + 2];
            r.read_exact(&mut chunk).ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
    } else if let Some(n) = get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        body.resize(n, 0);
        r.read_exact(&mut body).ok()?;
    }
    Some(Received {
        head,
        headers,
        body,
    })
}
