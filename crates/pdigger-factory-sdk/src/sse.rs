//! Stream decoding: server-sent events (as the SSE spec parses them) and NDJSON, over a
//! response body. [`EventStream`] is what the low-level `stream_frames` and `raw_stream`
//! return; the mid level builds resume, backfill and deduplication on top.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;

use crate::error::{Error, Result};
use crate::models::StreamFormat;
use crate::transport::Body;

/// The longest line, and the most data of one event, a stream may send: 64 MiB (a full frame
/// is well under 1 MiB). More is a `decode` error that ends the connection.
pub const MAX_EVENT_BYTES: usize = 64 << 20;

fn too_large(what: &str, limit: usize) -> Error {
    Error::decode(None, format!("{what} of the stream is larger than {limit} bytes"))
}

/// One event of a stream. For NDJSON each line is an event of type `frame` (or `gap` for a
/// `{"gap": n}` line) with no id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// Event type (`frame`, `clock`, `worker`, `gap`, `reset`; `message` when none was sent).
    pub event: String,
    /// The data lines joined with `\n`.
    pub data: String,
    /// The last event id seen so far (`<boot>-<cursor>` on this API).
    pub id: Option<String>,
    /// A `retry` field sent with this event, in milliseconds.
    pub retry: Option<u64>,
}

/// Splits bytes into lines ending in CRLF, LF or CR.
#[derive(Debug)]
struct Lines {
    buffer: Vec<u8>,
    /// How much of `buffer` holds no line end (where the next scan starts).
    scanned: usize,
    started: bool,
    limit: usize,
}

impl Default for Lines {
    fn default() -> Lines {
        Lines { buffer: Vec::new(), scanned: 0, started: false, limit: MAX_EVENT_BYTES }
    }
}

impl Lines {
    /// Adds bytes; returns the complete lines (`decode` past the line limit).
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        const BOM: &[u8] = b"\xEF\xBB\xBF";
        self.buffer.extend_from_slice(bytes);
        if !self.started {
            // A byte order mark is ignored at the very start; wait until it can be told.
            if self.buffer.len() < BOM.len() && BOM.starts_with(&self.buffer) {
                return Ok(Vec::new());
            }
            self.started = true;
            if self.buffer.starts_with(BOM) {
                self.buffer.drain(..BOM.len());
            }
        }
        let mut lines = Vec::new();
        let mut start = 0;
        let mut i = self.scanned;
        while i < self.buffer.len() {
            match self.buffer[i] {
                b'\n' => {
                    lines.push(String::from_utf8_lossy(&self.buffer[start..i]).into_owned());
                    start = i + 1;
                }
                // A CR at the end of the buffer may be the first half of a CRLF: wait.
                b'\r' if i + 1 == self.buffer.len() => break,
                b'\r' => {
                    lines.push(String::from_utf8_lossy(&self.buffer[start..i]).into_owned());
                    start = if self.buffer[i + 1] == b'\n' { i + 2 } else { i + 1 };
                    i = start;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        self.buffer.drain(..start);
        self.scanned = i - start;
        if self.buffer.len() > self.limit || lines.iter().any(|l: &String| l.len() > self.limit) {
            return Err(too_large("a line", self.limit));
        }
        Ok(lines)
    }

    /// The end of the stream: the last line, and whether a line end (a CR waiting for a
    /// possible LF) closed it.
    fn finish(&mut self) -> Option<(String, bool)> {
        if self.buffer.is_empty() {
            return None;
        }
        let mut rest = std::mem::take(&mut self.buffer);
        self.scanned = 0;
        let ended = rest.last() == Some(&b'\r');
        if ended {
            rest.pop();
        }
        Some((String::from_utf8_lossy(&rest).into_owned(), ended))
    }
}

/// The SSE parser of the HTML spec's event-stream interpretation.
#[derive(Debug, Default)]
pub struct SseParser {
    lines: Lines,
    event: String,
    data: String,
    has_data: bool,
    /// The spec's last event ID buffer: the newest `id` field, of an event still being read.
    id_buffer: Option<String>,
    /// The id of the last event dispatched (set at every blank line, as the spec says).
    last_id: Option<String>,
    retry: Option<u64>,
}

impl SseParser {
    /// A parser at the start of a stream.
    pub fn new() -> SseParser {
        SseParser::default()
    }

    /// A parser whose lines and events may hold `limit` bytes at most (the default is
    /// [`MAX_EVENT_BYTES`]).
    pub fn with_limit(limit: usize) -> SseParser {
        SseParser { lines: Lines { limit, ..Lines::default() }, ..SseParser::default() }
    }

    /// Feeds bytes; returns the events they complete, or `decode` for a line or an event past
    /// the limit (the stream cannot go on).
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>> {
        let lines = self.lines.push(bytes)?;
        let mut events = Vec::new();
        for line in lines {
            events.extend(self.line(&line)?);
        }
        Ok(events)
    }

    /// The end of the stream. A line without a line end, and an event without its closing
    /// blank line, are discarded, as the spec says.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        match self.lines.finish() {
            Some((line, true)) => self.line(&line).ok().flatten().into_iter().collect(),
            _ => Vec::new(),
        }
    }

    /// The id of the last event dispatched: the `Last-Event-ID` to resume with. The `id` of
    /// an event cut off before its blank line does not count (resuming from it would skip
    /// that event).
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_id.as_deref()
    }

    fn line(&mut self, line: &str) -> Result<Option<SseEvent>> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line.starts_with(':') {
            return Ok(None);
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if self.data.len() + value.len() + 1 > self.lines.limit {
                    return Err(too_large("an event", self.lines.limit));
                }
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.id_buffer = Some(value.to_string()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.retry = value.parse().ok()
            }
            _ => {}
        }
        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        self.last_id.clone_from(&self.id_buffer);
        let event = std::mem::take(&mut self.event);
        if !self.has_data {
            return None;
        }
        self.has_data = false;
        Some(SseEvent {
            event: if event.is_empty() { "message".to_string() } else { event },
            data: std::mem::take(&mut self.data),
            id: self.last_id.clone(),
            retry: self.retry.take(),
        })
    }
}

/// NDJSON lines as events: `frame` for an object, `gap` for `{"gap": n}`.
#[derive(Debug, Default)]
pub struct NdjsonParser {
    lines: Lines,
}

impl NdjsonParser {
    /// A parser whose lines may hold `limit` bytes at most (the default is
    /// [`MAX_EVENT_BYTES`]).
    pub fn with_limit(limit: usize) -> NdjsonParser {
        NdjsonParser { lines: Lines { limit, ..Lines::default() } }
    }

    /// Feeds bytes; returns the events of the complete lines, or `decode` for a line past the
    /// limit.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>> {
        Ok(self.lines.push(bytes)?.into_iter().filter_map(ndjson_event).collect())
    }

    /// The end of the stream: a last line without a line end still counts.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        self.lines.finish().and_then(|(line, _)| ndjson_event(line)).into_iter().collect()
    }
}

fn ndjson_event(line: String) -> Option<SseEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let event = if trimmed.starts_with("{\"gap\"") { "gap" } else { "frame" };
    Some(SseEvent { event: event.to_string(), data: trimmed.to_string(), id: None, retry: None })
}

#[derive(Debug)]
enum Decoder {
    Sse(SseParser),
    Ndjson(NdjsonParser),
}

/// The events of one HTTP stream (one connection, no resume). Dropping it closes the
/// connection.
#[derive(Debug)]
pub struct EventStream {
    body: Body,
    decoder: Decoder,
    queue: VecDeque<SseEvent>,
    done: bool,
}

impl EventStream {
    /// Decodes `body` as `format`.
    pub fn new(body: Body, format: StreamFormat) -> EventStream {
        let decoder = match format {
            StreamFormat::Ndjson => Decoder::Ndjson(NdjsonParser::default()),
            _ => Decoder::Sse(SseParser::new()),
        };
        EventStream { body, decoder, queue: VecDeque::new(), done: false }
    }

    /// The id of the last SSE event dispatched on this connection (the `Last-Event-ID` to
    /// resume with).
    pub fn last_event_id(&self) -> Option<&str> {
        match &self.decoder {
            Decoder::Sse(parser) => parser.last_event_id(),
            Decoder::Ndjson(_) => None,
        }
    }

    fn decode(&mut self, bytes: Option<&[u8]>) -> Result<()> {
        let events = match (&mut self.decoder, bytes) {
            (Decoder::Sse(parser), Some(bytes)) => parser.push(bytes)?,
            (Decoder::Sse(parser), None) => parser.finish(),
            (Decoder::Ndjson(parser), Some(bytes)) => parser.push(bytes)?,
            (Decoder::Ndjson(parser), None) => parser.finish(),
        };
        self.queue.extend(events);
        Ok(())
    }
}

impl Stream for EventStream {
    type Item = Result<SseEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if let Some(event) = self.queue.pop_front() {
                return Poll::Ready(Some(Ok(event)));
            }
            if self.done {
                return Poll::Ready(None);
            }
            let decoded = match Pin::new(&mut self.body).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => self.decode(Some(&chunk)),
                Poll::Ready(Some(Err(error))) => Err(error),
                Poll::Ready(None) => {
                    self.done = true;
                    self.decode(None)
                }
            };
            if let Err(error) = decoded {
                self.done = true;
                return Poll::Ready(Some(Err(error)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use bytes::Bytes;
    use futures::StreamExt;

    fn events(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut p = SseParser::new();
        let mut out = Vec::new();
        for c in chunks {
            out.extend(p.push(c).unwrap());
        }
        out.extend(p.finish());
        out
    }

    #[test]
    fn sse_fields_comments_and_line_ends() {
        let got = events(&[
            b"\xEF\xBB\xBF: hello\nid: b-1\nevent: frame\ndata: {\"a\":\ndata:1}\n\n",
            b"event: clock\r\ndata: x\r\n\r\n",
            b"data: y\rretry: 3000\r\r",
        ]);
        assert_eq!(got.len(), 3);
        assert_eq!(
            got[0],
            SseEvent { event: "frame".into(), data: "{\"a\":\n1}".into(), id: Some("b-1".into()), retry: None }
        );
        assert_eq!(got[1].event, "clock");
        assert_eq!(got[1].id.as_deref(), Some("b-1"));
        assert_eq!(got[2].event, "message");
        assert_eq!(got[2].retry, Some(3000));
    }

    #[test]
    fn sse_edge_cases() {
        // CRLF split over two chunks, a field without a colon, an empty data line, ids with NUL
        // ignored, a non-numeric retry ignored, an empty event dispatching nothing, an
        // unfinished event at the end discarded.
        let got = events(&[b"data: a\r", b"\ndata\n\nid: x\0y\nretry: 1s\ndata: b\nunknown: z\n\n\n", b"data: never"]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].data, "a\n");
        assert_eq!(got[1].data, "b");
        assert_eq!(got[1].id, None);
        assert_eq!(got[1].retry, None);
        // An id takes effect when its event is dispatched (a blank line), even without data;
        // the id of an event cut off at the end does not.
        let mut p = SseParser::new();
        assert!(p.push(b"id: 7\n").unwrap().is_empty());
        assert_eq!(p.last_event_id(), None);
        assert!(p.push(b"\n").unwrap().is_empty());
        assert_eq!(p.last_event_id(), Some("7"));
        let mut cut = SseParser::new();
        assert_eq!(cut.push(b"id: b-1\nevent: frame\ndata: {}\n\nid: b-2\ndata: {\"partial").unwrap().len(), 1);
        assert!(cut.finish().is_empty());
        assert_eq!(cut.last_event_id(), Some("b-1"));
        // A short stream that ends without a line end, BOM and CR included.
        let mut q = SseParser::new();
        assert!(q.push(b"\xEF\xBB").unwrap().is_empty());
        assert!(q.finish().is_empty());
        let mut r = SseParser::new();
        r.push(b"id").unwrap();
        r.finish();
        assert_eq!(r.last_event_id(), None);
        let mut s = SseParser::new();
        s.push(b"id: 9\r").unwrap();
        s.finish();
        assert_eq!(s.last_event_id(), None, "no blank line came: nothing was dispatched");
    }

    #[test]
    fn lines_and_events_are_bounded() {
        let mut long = SseParser::with_limit(16);
        assert!(long.push(b"data: 0123").unwrap().is_empty());
        let e = long.push(b"456789abcdef").unwrap_err();
        assert!(e.message.contains("a line of the stream is larger than 16 bytes"), "{e}");
        let mut whole = SseParser::with_limit(16);
        assert!(whole.push(b"data: 0123456789abcdef\n").is_err(), "a complete line past the limit");
        let mut event = SseParser::with_limit(16);
        assert!(event.push(b"data: 01234\ndata: 56789\n").is_ok());
        let e = event.push(b"data: abcdef\n").unwrap_err();
        assert!(e.message.contains("an event of the stream is larger than 16 bytes"), "{e}");
        let mut nd = NdjsonParser::with_limit(8);
        assert_eq!(nd.push(b"{\"g\":1}\n").unwrap().len(), 1);
        assert!(nd.push(b"{\"g\":12345}").is_err());
        // a line split over many chunks is found without rescanning, and still parses
        let mut p = SseParser::new();
        for chunk in [&b"data: a"[..], b"b", b"c", b"\r", b"\n", b"\n"] {
            let got = p.push(chunk).unwrap();
            if !got.is_empty() {
                assert_eq!(got[0].data, "abc");
            }
        }
    }

    #[test]
    fn ndjson_lines() {
        let mut p = NdjsonParser::default();
        let mut got = p.push(b"{\"cursor\":1}\n\n{\"gap\":3}\r\n{\"cur").unwrap();
        got.extend(p.push(b"sor\":2}").unwrap());
        got.extend(p.finish());
        let kinds: Vec<&str> = got.iter().map(|e| e.event.as_str()).collect();
        assert_eq!(kinds, ["frame", "gap", "frame"]);
        assert_eq!(got[2].data, "{\"cursor\":2}");
        assert!(NdjsonParser::default().finish().is_empty());
    }

    #[tokio::test]
    async fn event_streams_over_bodies() {
        let body = Body::from_stream(futures::stream::iter(vec![
            Ok(Bytes::from("id: a-1\nevent: frame\ndata: 1\n")),
            Ok(Bytes::from("\nid: a-2\ndata: 2\n\n")),
        ]));
        let mut s = EventStream::new(body, StreamFormat::Sse);
        assert_eq!(s.next().await.unwrap().unwrap().data, "1");
        assert_eq!(s.last_event_id(), Some("a-2"), "both events of the chunk are dispatched");
        assert_eq!(s.next().await.unwrap().unwrap().event, "message");
        assert!(s.next().await.is_none());
        assert!(s.next().await.is_none());
        let body = Body::from_stream(futures::stream::iter(vec![
            Ok(Bytes::from("{\"g\":1}\n{\"g\":2}")),
            Err(Error::decode(None, "cut")),
        ]));
        let mut n = EventStream::new(body, StreamFormat::Ndjson);
        assert_eq!(n.last_event_id(), None);
        assert_eq!(n.next().await.unwrap().unwrap().data, "{\"g\":1}");
        assert!(n.next().await.unwrap().is_err());
        assert!(n.next().await.is_none());
        let body = Body::from_bytes("{\"g\":3}");
        let all: Vec<_> = EventStream::new(body, StreamFormat::Ndjson).collect().await;
        assert_eq!(all.len(), 1);
        let pending = Body::from_stream(futures::stream::pending());
        let mut p = EventStream::new(pending, StreamFormat::Sse);
        assert!(futures::poll!(p.next()).is_pending());
        // a line past the limit ends the connection with decode
        let huge = Body::from_stream(futures::stream::iter(vec![Ok(Bytes::from(vec![b'x'; MAX_EVENT_BYTES + 1]))]));
        let mut h = EventStream::new(huge, StreamFormat::Ndjson);
        assert_eq!(h.next().await.unwrap().unwrap_err().code, crate::error::ErrorCode::Decode);
        assert!(h.next().await.is_none());
    }
}
