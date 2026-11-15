//! Server-Sent Events (`text/event-stream`), parsed as the HTML spec's
//! event-stream interpretation: `data` lines join with `\n`, a blank line
//! dispatches, `:` lines are comments, `event` names the type (default
//! `message`), `id` and `retry` carry over. Lines end in LF, CRLF or CR.

use std::io::{self, BufRead};

/// One dispatched event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` (`"message"` when absent).
    pub event: String,
    /// The `data:` lines joined with `\n`.
    pub data: String,
    /// The last event id seen so far in the stream.
    pub id: Option<String>,
    /// A `retry:` in this event, in milliseconds.
    pub retry: Option<u64>,
}

impl SseEvent {
    /// `data` as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.data)
    }
}

/// An iterator of events over a buffered reader (from
/// [`super::Streamed::sse`]). It ends with the stream; a read error (a
/// stall, a reset) is yielded once, then it ends.
pub struct SseReader<R> {
    inner: R,
    last_id: Option<String>,
    /// A CR ended the last line: drop a LF that follows it.
    after_cr: bool,
    done: bool,
}

impl<R: BufRead> SseReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            last_id: None,
            after_cr: false,
            done: false,
        }
    }

    /// One line without its terminator; `None` at the end of the stream.
    fn line(&mut self) -> io::Result<Option<Vec<u8>>> {
        let mut line = Vec::new();
        loop {
            let buf = self.inner.fill_buf()?;
            if buf.is_empty() {
                return Ok((!line.is_empty()).then_some(line));
            }
            if self.after_cr {
                self.after_cr = false;
                if buf[0] == b'\n' {
                    self.inner.consume(1);
                    continue;
                }
            }
            match buf.iter().position(|&b| b == b'\n' || b == b'\r') {
                Some(i) => {
                    line.extend_from_slice(&buf[..i]);
                    self.after_cr = buf[i] == b'\r';
                    self.inner.consume(i + 1);
                    return Ok(Some(line));
                }
                None => {
                    let n = buf.len();
                    line.extend_from_slice(buf);
                    self.inner.consume(n);
                }
            }
        }
    }
}

impl<R: BufRead> Iterator for SseReader<R> {
    type Item = io::Result<SseEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let mut event = String::new();
        let mut data = String::new();
        let mut has_data = false;
        let mut retry = None;
        loop {
            let line = match self.line() {
                Ok(Some(l)) => l,
                // The spec drops an event the stream ended in the middle of.
                Ok(None) => {
                    self.done = true;
                    return None;
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            };
            if line.is_empty() {
                if !has_data {
                    event.clear();
                    retry = None;
                    continue;
                }
                return Some(Ok(SseEvent {
                    event: if event.is_empty() {
                        "message".into()
                    } else {
                        event
                    },
                    data,
                    id: self.last_id.clone(),
                    retry,
                }));
            }
            let line = String::from_utf8_lossy(&line);
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (&*line, ""),
            };
            match field {
                "event" => event = value.to_string(),
                "data" => {
                    if has_data {
                        data.push('\n');
                    }
                    data.push_str(value);
                    has_data = true;
                }
                "id" if !value.contains('\0') => self.last_id = Some(value.to_string()),
                "retry" => retry = value.parse().ok().or(retry),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(s: &[u8]) -> Vec<SseEvent> {
        SseReader::new(s).map(Result::unwrap).collect()
    }

    #[test]
    fn batobrowse_search_stream() {
        // internal/api/search.go: one `data:` per match, then `event: done`.
        let got = events(
            b"data: {\"path\":\"/a\"}\n\ndata: {\"path\":\"/b\"}\n\nevent: done\ndata: {\"count\":2}\n\n",
        );
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].event, "message");
        assert_eq!(got[0].data, r#"{"path":"/a"}"#);
        assert_eq!(got[2].event, "done");
        let done: serde_json::Value = got[2].json().unwrap();
        assert_eq!(done["count"], 2);
    }

    #[test]
    fn spec_details() {
        let got = events(
            b": comment\r\nid: 7\r\nretry: 3000\r\ndata:first\r\ndata:  second\r\n\r\n\
              event: x\r\n\r\n\
              data\rdata: cr\r\rid: 8\ndata: tail without blank line",
        );
        assert_eq!(
            got,
            vec![
                SseEvent {
                    event: "message".into(),
                    data: "first\n second".into(),
                    id: Some("7".into()),
                    retry: Some(3000),
                },
                // `event: x` with no data dispatches nothing; empty `data` counts.
                SseEvent {
                    event: "message".into(),
                    data: "\ncr".into(),
                    id: Some("7".into()),
                    retry: None,
                },
            ]
        );
    }

    #[test]
    fn events_arrive_before_the_stream_ends() {
        use super::super::test_server::{Reply, serve};
        use std::time::{Duration, Instant};
        let (base, _rx) = serve(vec![
            Reply::new(200, "data: one\n\n")
                .content_type("text/event-stream")
                .then(Duration::from_millis(1500), "data: two\n\n"),
        ]);
        let t = Instant::now();
        let mut it = crate::http::Http::new(&base).open("/s").unwrap().sse();
        assert_eq!(it.next().unwrap().unwrap().data, "one");
        assert!(
            t.elapsed() < Duration::from_millis(1400),
            "the first event waited for the second"
        );
        assert_eq!(it.next().unwrap().unwrap().data, "two");
        assert!(it.next().is_none());
    }
}
