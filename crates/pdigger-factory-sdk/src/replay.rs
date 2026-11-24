//! Replay: the interpreter over a recording (`pfr1`, spec 2.5 and 6), offline.
//!
//! A recording is NDJSON, optionally gzip: a header line, then frames exactly as the API sent
//! them and side lines (`{"sse": "clock" | "worker" | "gap" | "reset", "data": …, "recv": …}`)
//! in the order received. [`Replay`] reads a local file synchronously; [`ReplayStream`] reads
//! a file or an HTTP(S) URL asynchronously (streamed, never loaded whole) and can pace frames
//! in real time.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, ErrorCode, Result};
use crate::interpreter::{Activity, Event, InterpreterCore, Summary, Tray, TrayFact, TrayReport, TraySpeckle, Update};
use crate::models::{ClockEvent, Extra, Frame, GapEvent, Layer, ResetEvent, VideoEntry};
use crate::shape;
use crate::transport::{Body, Method, Request, ReqwestTransport, Transport};

/// The first line of a recording (its first line that is not blank, when that has `recording`
/// and decodes; spec 6).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingHeader {
    /// `pfr1`.
    pub recording: String,
    /// Name of the excerpt.
    pub name: Option<String>,
    /// Source (video id); `None` when it spans several.
    pub video: Option<String>,
    /// The catalog entry of the source (`None` also when it is no catalog entry, spec 6).
    #[serde(deserialize_with = "source_if_it_decodes")]
    pub source: Option<VideoEntry>,
    /// Server build.
    pub build: Option<String>,
    /// API version.
    pub api: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// Inference backend.
    pub backend: Option<String>,
    /// Layers recorded (empty also when they are not an array of strings, spec 6).
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub layers: Vec<Layer>,
    /// Fields removed (face joints, appearance masks, depth textures; empty also when they are
    /// not an array of strings).
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub redacted: Vec<String>,
    /// Where it was recorded.
    pub recorded_from: Option<String>,
    /// When (ISO time).
    pub recorded_at: Option<String>,
    /// Frames (`None` also when it is past an `i64`, spec 6; so for `g_first` and `g_last`).
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub frames: Option<i64>,
    /// First grid frame.
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub g_first: Option<i64>,
    /// Last grid frame.
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub g_last: Option<i64>,
    /// Sessions (empty also when they are not an array of whole numbers).
    #[serde(deserialize_with = "left_out_unless_it_fits")]
    pub sessions: Vec<i64>,
    /// Notes.
    pub notes: Option<String>,
    /// Unknown fields (also `complete` and the recorder's counts, kept as they are).
    #[serde(flatten)]
    pub extra: Extra,
}

/// A side line of a recording: a stream's side event as received.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SideLine {
    /// `clock`, `worker`, `gap` or `reset`.
    pub sse: String,
    /// The event's data.
    pub data: serde_json::Value,
    /// When it was received (epoch seconds).
    pub recv: Option<f64>,
}

/// A line of a recording.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// The header.
    Header(Box<RecordingHeader>),
    /// A frame.
    Frame(Box<Frame>),
    /// A side line.
    Side(SideLine),
}

/// A header's `source`, left out when it is no catalog entry ([`shape::SOURCE`]) or does not
/// decode (spec 6).
fn source_if_it_decodes<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<VideoEntry>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let entry = value.filter(|value| shape::problem(&value.to_string(), shape::SOURCE).is_none());
    Ok(entry.and_then(|value| serde_json::from_value(value).ok()))
}

/// A header member that the model cannot hold is left out (its default), so that it never keeps
/// the line from being the header (spec 6); null counts as missing.
fn left_out_unless_it_fits<'de, D: Deserializer<'de>, T: DeserializeOwned + Default>(
    deserializer: D,
) -> std::result::Result<T, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| serde_json::from_value(value).ok()).unwrap_or_default())
}

fn decode_error(e: serde_json::Error) -> Error {
    Error::decode(None, format!("a recording line is not JSON: {e}"))
}

/// The `decode` error of a line that does not decode as the `what` (header, side line, frame) it
/// claims to be.
fn undecodable(what: &str, why: impl std::fmt::Display) -> Error {
    Error::decode(None, format!("a recording's {what} that does not decode: {why}"))
}

/// Reads one line as spec 6 says a reader does. Pass `have_header` false only for the
/// first line that is not blank, the only one a header can be: then a line with `recording` is
/// the header. Otherwise a line with `sse` is a side line and one with `g` and `cursor` a frame.
/// Each must decode (spec 2.3 rule 9: its fields there, not null, of their types; a side line's
/// `sse` a string and `recv` a number when there). A line that is not JSON, or one that does not
/// decode as the header, side line or frame it claims to be, is an error (`decode`); a blank line,
/// JSON that is not an object and any other line give `None`. A replay skips all of these.
pub fn parse_line(text: &str, have_header: bool) -> Result<Option<Line>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let Some(fields) = shape::fields(text) else {
        // JSON that is not an object is skipped
        return serde_json::from_str::<IgnoredAny>(text).map(|_| None).map_err(decode_error);
    };
    if fields.contains_key("recording") && !have_header {
        if let Some(why) = shape::problem_of(&fields, shape::HEADER) {
            return Err(undecodable("header", why));
        }
        return serde_json::from_str(text)
            .map(|h| Some(Line::Header(Box::new(h))))
            .map_err(|e| undecodable("header", e));
    }
    if fields.contains_key("sse") {
        if let Some(why) = shape::side_line_problem(&fields) {
            return Err(undecodable("side line", why));
        }
        return serde_json::from_str(text).map(|s| Some(Line::Side(s))).map_err(|e| undecodable("side line", e));
    }
    if fields.contains_key("g") && fields.contains_key("cursor") {
        if let Some(why) = shape::problem_of(&fields, shape::FRAME) {
            return Err(undecodable("frame", why));
        }
        return serde_json::from_str(text).map(|f| Some(Line::Frame(Box::new(f)))).map_err(|e| undecodable("frame", e));
    }
    Ok(None)
}

/// A line of a recording for its reader: `None` for a blank line (no line of the file), else the
/// line as [`parse_line`] reads it, the first one (`first`) able to be the header. Bytes that are
/// not UTF-8 read as U+FFFD.
fn read_line(raw: &[u8], first: bool) -> Option<Result<Option<Line>>> {
    let text = String::from_utf8_lossy(raw);
    (!text.trim().is_empty()).then(|| parse_line(&text, !first))
}

/// The lines of a reader (gzip detected by its magic bytes); lines it does not understand are
/// skipped. A read error ends them: it is given once, and `failed` is set (the replay then has no
/// final update, as in the other languages).
struct SyncLines {
    reader: Box<dyn BufRead + Send>,
    /// Whether the first line that is not blank was read (the only one a header can be).
    past_first: bool,
    buffer: Vec<u8>,
    failed: Arc<AtomicBool>,
}

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

impl SyncLines {
    fn new(mut reader: Box<dyn Read + Send>) -> Result<SyncLines> {
        // the first two bytes tell gzip (a reader may give them one at a time)
        let (mut head, mut n) = ([0u8; 2], 0);
        while n < 2 {
            match reader.read(&mut head[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(io_error(e)),
            }
        }
        let inner = BufReader::new(std::io::Cursor::new(head[..n].to_vec()).chain(reader));
        let reader: Box<dyn BufRead + Send> = if head[..n] == GZIP_MAGIC {
            Box::new(BufReader::new(flate2::read::MultiGzDecoder::new(inner)))
        } else {
            Box::new(inner)
        };
        Ok(SyncLines { reader, past_first: false, buffer: Vec::new(), failed: Arc::default() })
    }
}

fn open_error(path: &Path, e: std::io::Error) -> Error {
    Error::new(ErrorCode::Decode, format!("cannot open {}: {e}", path.display())).with_source(e)
}

fn io_error(e: std::io::Error) -> Error {
    Error::new(ErrorCode::Decode, format!("cannot read the recording: {e}")).with_source(e)
}

impl Iterator for SyncLines {
    type Item = Result<Line>;

    fn next(&mut self) -> Option<Result<Line>> {
        if self.failed.load(Ordering::Relaxed) {
            return None;
        }
        loop {
            self.buffer.clear();
            match self.reader.read_until(b'\n', &mut self.buffer) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) => {
                    // a read error repeats (a cut gzip file): it ends the lines
                    self.failed.store(true, Ordering::Relaxed);
                    return Some(Err(io_error(e)));
                }
            }
            // a line it does not understand (not JSON, not what it says it is) is skipped
            let Some(line) = read_line(&self.buffer, !self.past_first) else { continue };
            self.past_first = true;
            if let Ok(Some(line)) = line {
                return Some(Ok(line));
            }
        }
    }
}

/// A line for the core: a clock line sets its clock, a `gap` or `reset` line tells it that frames
/// were dropped there (spec 4.6.1); a frame is returned to be fed.
fn frame_or_side(core: &mut InterpreterCore, line: Line) -> Option<Frame> {
    match line {
        Line::Frame(frame) => Some(*frame),
        Line::Side(side) if side.sse == "clock" && side.data.is_object() => {
            if let Ok(clock) = serde_json::from_value::<ClockEvent>(side.data) {
                core.set_clock(clock.state);
            }
            None
        }
        Line::Side(side) if side.sse == "gap" || side.sse == "reset" => {
            // frames dropped here; a line whose data does not decode is ignored
            let decodes = if side.sse == "gap" {
                serde_json::from_value::<GapEvent>(side.data).is_ok()
            } else {
                serde_json::from_value::<ResetEvent>(side.data).is_ok()
            };
            if decodes {
                core.gap();
            }
            None
        }
        _ => None,
    }
}

impl InterpreterCore {
    /// Feeds one line of a recording ([`Replay::lines`], [`ReplayStream::lines`]) as a replay
    /// does, and returns the update a replay gives for it: a frame's, as [`InterpreterCore::feed`]
    /// makes it, unless the frame is a duplicate. A clock line goes to
    /// [`InterpreterCore::set_clock`] (the wall times), and a `gap` or `reset` line whose data
    /// decodes ([`GapEvent`], [`ResetEvent`]) to [`InterpreterCore::gap`]: frames were dropped
    /// there, which ends the trays' presence questions open then (spec 4.6.1). The header and any
    /// other line change nothing. A core of your own fed only a recording's frames misses its
    /// clock, `gap` and `reset` lines: give it every line.
    ///
    /// ```no_run
    /// use pdigger_factory_sdk::{InterpreterCore, Replay};
    /// # fn demo() -> pdigger_factory_sdk::Result<()> {
    /// let mut core = InterpreterCore::new();
    /// for line in Replay::open("dry1.world.ndjson.gz")?.lines() {
    ///     for fact in core.feed_line(line?).into_iter().flat_map(|update| update.tray_facts) {
    ///         println!("{}", fact.describe());
    ///     }
    /// }
    /// for fact in core.end().tray_facts {
    ///     println!("{}", fact.describe()); // the last tray leaving
    /// }
    /// # Ok(()) }
    /// ```
    pub fn feed_line(&mut self, line: Line) -> Option<Update> {
        frame_or_side(self, line).map(|frame| self.feed(frame)).filter(|u| !u.is_duplicate)
    }
}

/// Everything a replay produces, at once.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayOutput {
    /// The header.
    pub header: Option<RecordingHeader>,
    /// All events.
    pub events: Vec<Event>,
    /// All activities, the ones closed at the end included.
    pub activities: Vec<Activity>,
    /// All tray facts (spec 4.6), those of the last tray leaving included.
    pub tray_facts: Vec<TrayFact>,
    /// The unit's speckle answers read, in order (spec 4.6.12).
    pub speckle: Vec<TraySpeckle>,
    /// Every tray of the recording, oldest first.
    pub trays: Vec<Tray>,
    /// The summary.
    pub summary: Summary,
}

/// A synchronous replay of a local recording.
///
/// ```no_run
/// use pdigger_factory_sdk::Replay;
/// # fn demo() -> pdigger_factory_sdk::Result<()> {
/// let replay = Replay::open("dry1.world.ndjson.gz")?;
/// println!("{:?}", replay.header().map(|h| &h.video));
/// print!("{}", replay.summary()?.describe());
/// # Ok(()) }
/// ```
pub struct Replay {
    header: Option<RecordingHeader>,
    lines: SyncLines,
    first: Option<Line>,
}

impl std::fmt::Debug for Replay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Replay").field("header", &self.header).finish_non_exhaustive()
    }
}

impl Replay {
    /// A replay of the file at `path` (`.ndjson` or `.ndjson.gz`).
    pub fn open(path: impl AsRef<Path>) -> Result<Replay> {
        Replay::open_path(path.as_ref())
    }

    fn open_path(path: &Path) -> Result<Replay> {
        let file = File::open(path).map_err(|e| open_error(path, e))?;
        Replay::from_reader(file)
    }

    /// A replay of the recording bytes (gzip or not).
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Replay> {
        Replay::from_reader(std::io::Cursor::new(bytes.into()))
    }

    /// A replay of a reader of the recording (gzip or not).
    pub fn from_reader(reader: impl Read + Send + 'static) -> Result<Replay> {
        Replay::from_boxed(Box::new(reader))
    }

    fn from_boxed(reader: Box<dyn Read + Send>) -> Result<Replay> {
        let mut lines = SyncLines::new(reader)?;
        let (header, first) = match lines.next().transpose()? {
            Some(Line::Header(header)) => (Some(*header), None),
            other => (None, other),
        };
        Ok(Replay { header, lines, first })
    }

    /// The header.
    pub fn header(&self) -> Option<&RecordingHeader> {
        self.header.as_ref()
    }

    /// Every line after the header (a `recording` line further down is read like any other line:
    /// the header can only be the first line that is not blank).
    pub fn lines(self) -> impl Iterator<Item = Result<Line>> + Send {
        self.first.map(Ok).into_iter().chain(self.lines)
    }

    /// The frames.
    pub fn frames(self) -> impl Iterator<Item = Result<Frame>> + Send {
        self.lines().filter_map(|line| match line {
            Ok(Line::Frame(frame)) => Some(Ok(*frame)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
    }

    /// An [`Update`] per frame (duplicates skipped), then a final one ([`Update::is_final`])
    /// with what was still open at the end; a read error (a cut file) ends them without it.
    pub fn updates(self) -> Updates {
        let failed = self.lines.failed.clone();
        Updates { lines: Box::new(self.lines()), core: InterpreterCore::new(), done: false, failed }
    }

    /// The events.
    pub fn events(self) -> impl Iterator<Item = Result<Event>> + Send {
        self.updates().flat_map(|u| split(u, |u| u.events))
    }

    /// The activities, the ones closed at the end included.
    pub fn activities(self) -> impl Iterator<Item = Result<Activity>> + Send {
        self.updates().flat_map(|u| split(u, |u| u.activities))
    }

    /// The tray facts (spec 4.6): trays arriving and leaving, tray stones removed, set down or no
    /// longer sure.
    pub fn tray_facts(self) -> impl Iterator<Item = Result<TrayFact>> + Send {
        self.updates().flat_map(|u| split(u, |u| u.tray_facts))
    }

    /// Every tray of the recording, oldest first, with its `arrived` and `leaving` points, its
    /// tray stones and its facts (reads the whole recording). [`Tray::window`] is the span to
    /// replay or to show; on the shared session, [`Session::seek`](crate::Session::seek) shows a
    /// point.
    pub fn trays(self) -> Result<Vec<Tray>> {
        Ok(self.consume(0)?.trays())
    }

    /// The report of every tray that arrived, oldest first (reads the whole recording; spec 4.7):
    /// which stones were removed, added and moved, and whether that is the tray's whole account.
    pub fn tray_reports(self) -> Result<Vec<TrayReport>> {
        Ok(self.trays()?.iter().filter_map(Tray::report).collect())
    }

    /// The summary (reads the whole recording).
    pub fn summary(self) -> Result<Summary> {
        Ok(self.consume(crate::interpreter::TRAYS_KEPT)?.summary())
    }

    /// Feeds a new core the whole recording (keeping `kept` trays) and nothing else, then ends it.
    fn consume(self, kept: usize) -> Result<InterpreterCore> {
        let mut core = InterpreterCore::new();
        core.keep_trays(kept);
        for line in self.lines() {
            if let Some(frame) = frame_or_side(&mut core, line?) {
                core.feed_parts(&frame);
            }
        }
        core.end();
        Ok(core)
    }

    /// Events, activities, tray facts, the speckle answers, the trays and the summary in one pass.
    pub fn run(self) -> Result<ReplayOutput> {
        let header = self.header.clone();
        let mut core = InterpreterCore::new();
        core.keep_trays(0);
        let (mut events, mut activities, mut tray_facts, mut speckle) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for line in self.lines() {
            if let Some(frame) = frame_or_side(&mut core, line?) {
                let parts = core.feed_parts(&frame);
                events.extend(parts.events);
                activities.extend(parts.activities);
                tray_facts.extend(parts.tray_facts);
                speckle.extend(parts.speckle);
            }
        }
        let last = core.end();
        activities.extend(last.activities);
        tray_facts.extend(last.tray_facts);
        let (trays, summary) = (core.trays(), core.summary());
        Ok(ReplayOutput { header, events, activities, tray_facts, speckle, trays, summary })
    }

    /// The asynchronous form (to pace it, or to mix it with streams). It reads the recording
    /// as it goes, yielding to the runtime every 256 lines.
    pub fn into_stream(self) -> ReplayStream {
        let header = self.header.clone();
        let failed = self.lines.failed.clone();
        let lines = futures::stream::iter(self.lines().enumerate())
            .then(|(i, line)| async move {
                if i % 256 == 255 {
                    tokio::task::yield_now().await;
                }
                line
            })
            .boxed();
        ReplayStream { header, lines, speed: None, failed }
    }
}

fn split<T>(update: Result<Update>, part: impl FnOnce(Update) -> Vec<T>) -> Vec<Result<T>> {
    match update {
        Ok(update) => part(update).into_iter().map(Ok).collect(),
        Err(error) => vec![Err(error)],
    }
}

/// The updates of a [`Replay`].
pub struct Updates {
    lines: Box<dyn Iterator<Item = Result<Line>> + Send>,
    core: InterpreterCore,
    done: bool,
    failed: Arc<AtomicBool>,
}

impl Updates {
    /// The core being fed (its state and summary so far).
    pub fn core(&self) -> &InterpreterCore {
        &self.core
    }
}

impl Iterator for Updates {
    type Item = Result<Update>;

    fn next(&mut self) -> Option<Result<Update>> {
        while !self.done {
            match self.lines.next() {
                Some(Ok(line)) => {
                    if let Some(update) = self.core.feed_line(line) {
                        return Some(Ok(update));
                    }
                }
                Some(Err(e)) => return Some(Err(e)),
                None => {
                    self.done = true;
                    return (!self.failed.load(Ordering::Relaxed)).then(|| Ok(self.core.end()));
                }
            }
        }
        None
    }
}

/// Incremental line splitting (and gunzip) of an HTTP body.
struct BodyLines {
    body: Body,
    gzip: Option<flate2::write::MultiGzDecoder<Vec<u8>>>,
    started: bool,
    head: Vec<u8>,
    buffer: Vec<u8>,
    ready: VecDeque<Vec<u8>>,
    done: bool,
    /// Set by a read error, which ends the lines.
    failed: Arc<AtomicBool>,
}

impl BodyLines {
    fn new(body: Body, failed: Arc<AtomicBool>) -> BodyLines {
        BodyLines {
            body,
            gzip: None,
            started: false,
            head: Vec::new(),
            buffer: Vec::new(),
            ready: VecDeque::new(),
            done: false,
            failed,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<()> {
        let chunk = if self.started {
            chunk.to_vec()
        } else {
            self.head.extend_from_slice(chunk);
            if self.head.len() < 2 {
                return Ok(());
            }
            self.started = true;
            if self.head.starts_with(&GZIP_MAGIC) {
                self.gzip = Some(flate2::write::MultiGzDecoder::new(Vec::new()));
            }
            std::mem::take(&mut self.head)
        };
        match self.gzip.as_mut() {
            Some(gzip) => {
                gzip.write_all(&chunk).map_err(io_error)?;
                self.buffer.append(gzip.get_mut());
            }
            None => self.buffer.extend_from_slice(&chunk),
        }
        self.split();
        Ok(())
    }

    fn split(&mut self) {
        while let Some(i) = self.buffer.iter().position(|&b| b == b'\n') {
            let rest = self.buffer.split_off(i + 1);
            self.ready.push_back(std::mem::replace(&mut self.buffer, rest));
        }
    }

    fn finish(&mut self) -> Result<()> {
        if !self.started {
            self.started = true;
            self.buffer = std::mem::take(&mut self.head);
        }
        if let Some(mut gzip) = self.gzip.take() {
            gzip.try_finish().map_err(io_error)?;
            self.buffer.append(gzip.get_mut());
        }
        self.split();
        if !self.buffer.is_empty() {
            self.ready.push_back(std::mem::take(&mut self.buffer));
        }
        Ok(())
    }

    async fn next_line(&mut self) -> Option<Result<Vec<u8>>> {
        loop {
            if let Some(line) = self.ready.pop_front() {
                return Some(Ok(line));
            }
            if self.done {
                return None;
            }
            let step = match self.body.next().await {
                Some(Ok(chunk)) => self.push(&chunk),
                Some(Err(e)) => Err(e),
                None => {
                    self.done = true;
                    self.finish()
                }
            };
            if let Err(e) = step {
                self.done = true;
                self.failed.store(true, Ordering::Relaxed);
                return Some(Err(e));
            }
        }
    }
}

/// The lines of an HTTP body, fused: the open reads the first one, and a body with none (empty, or
/// nothing a reader understands) has then already ended when the replay asks again.
fn body_lines(body: Body, failed: Arc<AtomicBool>) -> BoxStream<'static, Result<Line>> {
    let state = (BodyLines::new(body, failed), false);
    futures::stream::unfold(state, |(mut lines, mut past_first)| async move {
        loop {
            let raw = match lines.next_line().await? {
                Ok(raw) => raw,
                Err(e) => return Some((Err(e), (lines, past_first))),
            };
            // a line it does not understand (not JSON, not what it says it is) is skipped
            let Some(line) = read_line(&raw, !past_first) else { continue };
            past_first = true;
            if let Ok(Some(line)) = line {
                return Some((Ok(line), (lines, past_first)));
            }
        }
    })
    .fuse()
    .boxed()
}

/// An asynchronous replay of a local file or an HTTP(S) URL, optionally paced in real time.
///
/// ```no_run
/// use futures::StreamExt;
/// use pdigger_factory_sdk::ReplayStream;
/// # async fn demo() -> pdigger_factory_sdk::Result<()> {
/// let url = "https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz";
/// let mut events = ReplayStream::open_url(url).await?.realtime(4.0).events();
/// while let Some(event) = events.next().await {
///     println!("{}", event?.describe());
/// }
/// # Ok(()) }
/// ```
pub struct ReplayStream {
    header: Option<RecordingHeader>,
    lines: BoxStream<'static, Result<Line>>,
    speed: Option<f64>,
    /// Set by a read error, which ends the lines without a final update.
    failed: Arc<AtomicBool>,
}

impl std::fmt::Debug for ReplayStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReplayStream").field("header", &self.header).field("speed", &self.speed).finish_non_exhaustive()
    }
}

impl ReplayStream {
    /// A replay of the local file at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<ReplayStream> {
        Ok(Replay::open(path)?.into_stream())
    }

    /// A replay of the recording at `url`, streamed with the default transport.
    pub async fn open_url(url: &str) -> Result<ReplayStream> {
        let transport: Arc<dyn Transport> = Arc::new(ReqwestTransport::new(Duration::from_secs(5))?);
        ReplayStream::open_url_with(transport, url).await
    }

    /// A replay of the recording at `url`, streamed through `transport`.
    pub async fn open_url_with(transport: Arc<dyn Transport>, url: &str) -> Result<ReplayStream> {
        let headers = vec![("User-Agent".to_string(), crate::client::USER_AGENT.to_string())];
        let reply = transport
            .send(Request { method: Method::Get, url: url.to_string(), headers, body: None, timeout: None })
            .await?;
        if !(200..300).contains(&reply.status) {
            return Err(crate::client::reply_error(reply).await);
        }
        let failed = Arc::<AtomicBool>::default();
        let mut lines = body_lines(reply.body, failed.clone());
        let mut first = None;
        let header = match lines.next().await.transpose()? {
            Some(Line::Header(header)) => Some(*header),
            other => {
                first = other;
                None
            }
        };
        let lines = futures::stream::iter(first.map(Ok)).chain(lines).boxed();
        Ok(ReplayStream { header, lines, speed: None, failed })
    }

    /// The header.
    pub fn header(&self) -> Option<&RecordingHeader> {
        self.header.as_ref()
    }

    /// Paces frames at their media times divided by `speed` (1.0: real time). A new session or
    /// a step back in time restarts the pacing.
    pub fn realtime(mut self, speed: f64) -> ReplayStream {
        self.speed = Some(speed).filter(|s| *s > 0.0 && s.is_finite());
        self
    }

    /// The lines after the header, paced when [`ReplayStream::realtime`] was set; a read error
    /// (a cut file, a broken connection) is the last of them.
    pub fn lines(self) -> BoxStream<'static, Result<Line>> {
        let Some(speed) = self.speed else { return self.lines };
        let pacer = Pacer { speed, base: None };
        futures::stream::unfold((self.lines, pacer), |(mut lines, mut pacer)| async move {
            let line = lines.next().await?;
            if let Ok(Line::Frame(frame)) = &line {
                pacer.wait(frame).await;
            }
            Some((line, (lines, pacer)))
        })
        .boxed()
    }

    /// The frames.
    pub fn frames(self) -> BoxStream<'static, Result<Frame>> {
        self.lines()
            .filter_map(|line| async move {
                match line {
                    Ok(Line::Frame(frame)) => Some(Ok(*frame)),
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .boxed()
    }

    /// An [`Update`] per frame (duplicates skipped), then the final one; a read error ends them
    /// without it.
    pub fn updates(self) -> BoxStream<'static, Result<Update>> {
        let failed = self.failed.clone();
        let lines = self.lines();
        futures::stream::unfold((lines, InterpreterCore::new(), false), move |(mut lines, mut core, done)| {
            let failed = failed.clone();
            async move {
                if done {
                    return None;
                }
                loop {
                    match lines.next().await {
                        Some(Ok(line)) => {
                            if let Some(update) = core.feed_line(line) {
                                return Some((Ok(update), (lines, core, false)));
                            }
                        }
                        Some(Err(e)) => return Some((Err(e), (lines, core, false))),
                        None if failed.load(Ordering::Relaxed) => return None,
                        None => {
                            let last = core.end();
                            return Some((Ok(last), (lines, core, true)));
                        }
                    }
                }
            }
        })
        .boxed()
    }

    /// The events.
    pub fn events(self) -> BoxStream<'static, Result<Event>> {
        self.updates().flat_map(|u| futures::stream::iter(split(u, |u| u.events))).boxed()
    }

    /// The activities, the ones closed at the end included.
    pub fn activities(self) -> BoxStream<'static, Result<Activity>> {
        self.updates().flat_map(|u| futures::stream::iter(split(u, |u| u.activities))).boxed()
    }

    /// The tray facts (spec 4.6), those of the last tray leaving included.
    pub fn tray_facts(self) -> BoxStream<'static, Result<TrayFact>> {
        self.updates().flat_map(|u| futures::stream::iter(split(u, |u| u.tray_facts))).boxed()
    }

    /// Every tray of the recording, oldest first (reads the whole recording); see
    /// [`Replay::trays`].
    pub async fn trays(self) -> Result<Vec<Tray>> {
        Ok(self.consume(0).await?.trays())
    }

    /// The report of every tray that arrived, oldest first (reads the whole recording); see
    /// [`Replay::tray_reports`].
    pub async fn tray_reports(self) -> Result<Vec<TrayReport>> {
        Ok(self.trays().await?.iter().filter_map(Tray::report).collect())
    }

    /// The summary (reads the whole recording).
    pub async fn summary(self) -> Result<Summary> {
        let core = self.consume(crate::interpreter::TRAYS_KEPT).await?;
        Ok(core.summary())
    }

    /// Feeds a new core the whole recording (keeping `kept` trays), then ends it.
    async fn consume(self, kept: usize) -> Result<InterpreterCore> {
        let mut lines = self.lines();
        let mut core = InterpreterCore::new();
        core.keep_trays(kept);
        while let Some(line) = lines.next().await {
            if let Some(frame) = frame_or_side(&mut core, line?) {
                core.feed_parts(&frame);
            }
        }
        core.end();
        Ok(core)
    }
}

/// Real-time pacing of frames.
struct Pacer {
    speed: f64,
    base: Option<(tokio::time::Instant, f64, i64)>,
}

impl Pacer {
    /// Waits until `frame` is due; a frame of another session, an earlier one, or one too far
    /// ahead to wait for starts a new base.
    async fn wait(&mut self, frame: &Frame) {
        if let Some((at, t0, _)) = self.base.filter(|&(_, t0, session)| session == frame.session && frame.t >= t0) {
            let ahead = Duration::try_from_secs_f64((frame.t - t0) / self.speed).ok();
            if let Some(target) = ahead.and_then(|ahead| at.checked_add(ahead)) {
                tokio::time::sleep_until(target).await;
                return;
            }
        }
        self.base = Some((tokio::time::Instant::now(), frame.t, frame.session));
    }
}
