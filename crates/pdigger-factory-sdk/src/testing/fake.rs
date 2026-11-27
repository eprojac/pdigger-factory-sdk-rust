//! The fake server's state and its handlers of the v2 operations.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{ErrorKind, Read};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::Stream;
use serde_json::{Map, Value, json};

use super::{CONNECT_S, DelayFn, Fault, KEEP_PICTURES, LIVE_CAMERA, LIVE_LAG_S, ManualClock, Reply};
use crate::client::Clock;
use crate::error::{Error, Result};
use crate::models::{ChangedBy, ClockEvent, ControlReply, Focus, Live, LivePhase, SessionReply, State};
use crate::transport::{Body, Headers, Method, Request, Response};

const LAYERS: [&str; 5] = ["raw", "overlay", "scene", "cards", "world"];
const DEFAULT_LAYERS: [&str; 4] = ["overlay", "scene", "cards", "world"];
const EVENT_TYPES: [&str; 13] =
    ["new", "confirm", "freeze", "push", "lift", "hinge", "carry", "place", "drop", "gone", "delete", "split", "sheet"];
/// What a recording answers: without one, a recorded reply answers these (else the empty session does; the long
/// poll and the streams have nothing to send).
const FROM_FRAMES: [&str; 9] = [
    "getWorld",
    "getWorldEvents",
    "getStone",
    "getFocus",
    "getFrame",
    "rawPacket",
    "pollFrames",
    "streamFrames",
    "rawStream",
];
/// On a camera, `hold_until` stays this far ahead of the select until the session goes live (as
/// on the unit).
const LIVE_HOLD: f64 = 3600.0;
/// How many grid frame numbers a camera session skips at a break, as the unit's worker does: the
/// first frame after a pause is the last one's `g` + 1 + `LIVE_GAP`.
const LIVE_GAP: i64 = 4;
/// How often a stream that waits for a camera phase on a linked clock looks again (tokio time).
const PHASE_LOOK: Duration = Duration::from_millis(10);
/// A 16 × 9 grey JPEG: the picture of every frame of the fake's camera (with the frame in a
/// comment segment).
const JPEG: &str = concat!(
    "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDABQODxIPDRQSEBIXFRQYHjIhHhwcHj0sLiQySUBMS0dARkVQWnNiUFVtVkVGZIhlbXd7gYKBTmCNl4x9",
    "lnN+gXz/2wBDARUXFx4aHjshITt8U0ZTfHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHx8fHz/wAARCAAJABAD",
    "ASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAP/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/8QAFQEBAQAAAAAAAAAAAAAAAAAAAAH/xAAUEQEA",
    "AAAAAAAAAAAAAAAAAAAA/9oADAMBAAIRAxEAPwCYCD//2Q==",
);

/// The catalog entry of the fake's live camera, sized like `like` (the first catalog entry),
/// shaped as a plant camera's.
fn camera_entry(like: Option<&Value>) -> Value {
    let like = like.cloned().unwrap_or_default();
    let grid_fps = match like["fps"].as_f64() {
        Some(fps) if fps > 0.0 => fps / like["grid_step"].as_f64().unwrap_or(0.0).max(1.0),
        _ => 5.0,
    };
    let size = |key: &str, w: i64, h: i64| if like[key].is_array() { like[key].clone() } else { json!([w, h]) };
    json!({
        "id": LIVE_CAMERA, "title": "Live camera", "subtitle": "The recording, replayed as a live camera",
        "duration": null, "fps": (grid_fps * 10.0).round() / 10.0, "frames": null, "grid_step": 1,
        "default_start": null, "hold": 3.0, "proxy": "", "proxy_wh": size("proxy_wh", 960, 540),
        "native_wh": size("native_wh", 1920, 1080), "held_out": [], "kind": "camera", "live": true,
        "seekable": false, "station": "FakeFactory",
    })
}

/// The catalog entry of a recording whose header names no source, made from its first frame's
/// video as the unit lists its recordings (`None`: no frame, or one without a video).
fn recording_entry(base: &[Arc<Value>]) -> Option<Value> {
    let video = base.first()?["video"].as_str().filter(|v| !v.is_empty())?;
    Some(json!({
        "id": video, "title": video, "duration": null, "fps": 10.0, "frames": null, "grid_step": 2,
        "default_start": 0.0, "hold": 2.2, "proxy": format!("/api/v2/videos/{video}/media"),
        "proxy_wh": [960, 540], "native_wh": [1920, 1080],
    }))
}

/// The fake camera's JPEG of frame `g`: [`JPEG`] with `FakeFactory <video> g <g>` in a comment
/// segment.
fn fake_picture(video: &str, g: i64) -> Bytes {
    let jpeg = STANDARD.decode(JPEG).expect("the fake's JPEG is base64");
    let note = format!("FakeFactory {video} g {g}");
    let length = u16::try_from(note.len() + 2).unwrap_or(u16::MAX);
    Bytes::from([&jpeg[..2], &[0xff, 0xfe], &length.to_be_bytes(), note.as_bytes(), &jpeg[2..]].concat())
}

/// Moves a frame to session `session` of `video`: the frame and its `world` and `raw` layers.
fn retag(f: &mut Value, session: i64, video: &str) {
    f["session"] = session.into();
    f["video"] = video.into();
    for layer in ["world", "raw"] {
        if let Some(l) = f.get_mut(layer).filter(|l| l.is_object()) {
            l["session"] = session.into();
            l["video"] = video.into();
        }
    }
}

/// Makes a recorded frame, already moved to the camera's `g` and `t`, a frame of camera `video`
/// in session `session`, numbered as a camera's frames are: `i` is `g`, the world's and the raw
/// packet's `g` (and `i`) and `t` are the frame's, and the overlay's `prev_g` moves with `g` (from
/// `recorded_g`, the frame's `g` in the recording).
fn camera_frame(f: &mut Value, recorded_g: i64, session: i64, video: &str) {
    retag(f, session, video);
    let (g, t) = (f["g"].clone(), f["t"].clone());
    f["i"] = g.clone();
    if let Some(world) = f.get_mut("world").filter(|w| w.is_object()) {
        (world["g"], world["t"]) = (g.clone(), t.clone());
    }
    if let Some(raw) = f.get_mut("raw").filter(|r| r.is_object()) {
        (raw["g"], raw["i"], raw["t"]) = (g.clone(), g.clone(), t);
    }
    let shift = g.as_i64().unwrap_or(0) - recorded_g;
    if let Some(prev) = f.get_mut("overlay").and_then(|o| o.get_mut("prev_g")).filter(|p| p.is_i64()) {
        *prev = (prev.as_i64().unwrap_or(0) + shift).into();
    }
}

/// A frame's raw packet with the frame's `cursor`, `session`, `video`, `g`, `i` and `t` (`None`:
/// the frame has no raw layer).
fn packet_of(frame: &Value) -> Option<Value> {
    let mut packet = frame.get("raw").filter(|r| r.is_object())?.clone();
    for key in ["cursor", "session", "video", "g", "i", "t"] {
        if let Some(value) = frame.get(key) {
            packet[key] = value.clone();
        }
    }
    Some(packet)
}

/// The lines of a recording (gzip or not) as JSON values.
pub(super) fn read_lines(bytes: &[u8]) -> Result<Vec<Value>> {
    let mut text = Vec::new();
    if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::MultiGzDecoder::new(bytes)
            .read_to_end(&mut text)
            .map_err(|e| Error::decode(None, format!("not gzip: {e}")))?;
    } else {
        text.extend_from_slice(bytes);
    }
    let text = String::from_utf8(text).map_err(|_| Error::decode(None, "the recording is not UTF-8"))?;
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).map_err(Error::from))
        .collect()
}

/// The replies of a `fixtures/api` directory: `INDEX.json` maps keys to
/// `{file, status, content_type, headers}`.
pub(super) fn read_replies(dir: &Path) -> Result<Vec<(String, Reply)>> {
    let read = |name: &str| {
        std::fs::read(dir.join(name))
            .map_err(|e| Error::decode(None, format!("cannot read {}: {e}", dir.join(name).display())))
    };
    let index: Map<String, Value> = serde_json::from_slice(&read("INDEX.json")?)?;
    let mut replies = Vec::new();
    for (key, entry) in index {
        let file = entry["file"].as_str().unwrap_or_default();
        let status = entry["status"].as_u64().and_then(|s| u16::try_from(s).ok()).unwrap_or(200);
        let content_type = entry["content_type"].as_str().unwrap_or("application/octet-stream");
        let mut reply = Reply::new(status, content_type, read(file)?);
        for (name, value) in entry["headers"].as_object().into_iter().flatten() {
            if !name.eq_ignore_ascii_case("content-length") {
                reply = reply.header(name, value.as_str().unwrap_or_default());
            }
        }
        replies.push((key, reply));
    }
    Ok(replies)
}

/// The path and the query of a URL.
pub(super) fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |i| &rest[i..]);
    match path.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (path.to_string(), String::new()),
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (b'+', _) => {
                out.push(b' ');
                i += 1;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(p), String::new()),
        })
        .collect()
}

/// The `operationId` of a request and its path parameters (`-methodNotAllowed` or
/// `-notFound` when there is none).
pub(super) fn route(method: Method, path: &str) -> (String, Vec<String>) {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (op, params) = match exact_route(method, &segs) {
        Some(found) => found,
        None if [Method::Get, Method::Post, Method::Put, Method::Delete]
            .into_iter()
            .any(|m| exact_route(m, &segs).is_some()) =>
        {
            ("-methodNotAllowed", vec![])
        }
        None => ("-notFound", vec![]),
    };
    (op.to_string(), params.into_iter().map(str::to_string).collect())
}

fn exact_route<'a>(method: Method, segs: &[&'a str]) -> Option<(&'static str, Vec<&'a str>)> {
    use Method::{Delete, Get, Head, Post, Put};
    Some(match (method, segs) {
        (Get, ["metrics"]) => ("getMetrics", vec![]),
        (Get, ["api", "v2"]) => ("getApiRoot", vec![]),
        (Get, ["api", "v2", "openapi.yaml"]) => ("getOpenApi", vec![]),
        (Get, ["api", "v2", "model"]) => ("getModel", vec![]),
        (Get, ["api", "v2", "topology"]) => ("getTopology", vec![]),
        (Get, ["api", "v2", "health"]) => ("getHealth", vec![]),
        (Get, ["api", "v2", "ready"]) => ("getReady", vec![]),
        (Get, ["api", "v2", "status"]) => ("getStatus", vec![]),
        (Get, ["api", "v2", "videos"]) => ("listVideos", vec![]),
        (Get, ["api", "v2", "videos", id]) => ("getVideo", vec![*id]),
        (Get | Head, ["api", "v2", "videos", id, "media"]) => ("getVideoMedia", vec![*id]),
        (Get, ["api", "v2", "session"]) => ("getSession", vec![]),
        (Post, ["api", "v2", "session", "select"]) => ("sessionSelect", vec![]),
        (Post, ["api", "v2", "session", "seek"]) => ("sessionSeek", vec![]),
        (Post, ["api", "v2", "session", "play"]) => ("sessionPlay", vec![]),
        (Post, ["api", "v2", "session", "pause"]) => ("sessionPause", vec![]),
        (Get, ["api", "v2", "session", "world"]) => ("getWorld", vec![]),
        (Get, ["api", "v2", "session", "world", "events"]) => ("getWorldEvents", vec![]),
        (Get, ["api", "v2", "session", "world", "stones", id]) => ("getStone", vec![*id]),
        (Get, ["api", "v2", "session", "focus"]) => ("getFocus", vec![]),
        (Put, ["api", "v2", "session", "focus"]) => ("putFocus", vec![]),
        (Delete, ["api", "v2", "session", "focus"]) => ("deleteFocus", vec![]),
        (Get, ["api", "v2", "session", "frames", "stream"]) => ("streamFrames", vec![]),
        (Get, ["api", "v2", "session", "frames"]) => ("pollFrames", vec![]),
        (Get, ["api", "v2", "session", "frames", g]) => ("getFrame", vec![*g]),
        (Get, ["api", "v2", "session", "frames", g, "image.jpg"]) => ("getFramePicture", vec![*g]),
        (Get, ["api", "v2", "session", "frames", g, raster]) => ("getFrameRaster", vec![*g, *raster]),
        (Get, ["api", "v2", "session", "render", "overlay2d"]) => ("renderOverlay2d", vec![]),
        (Get, ["api", "v2", "session", "render", "scene3d"]) => ("renderScene3d", vec![]),
        (Get, ["api", "v2", "raw", "stream"]) => ("rawStream", vec![]),
        (Get, ["api", "v2", "raw", "packets", g]) => ("rawPacket", vec![*g]),
        (Get, ["api", "v2", "raw", "tensors", g]) => ("rawTensors", vec![*g]),
        _ => return None,
    })
}

#[derive(Debug, Clone)]
enum Item {
    Frame(Arc<Value>),
    Side(String, Arc<Value>),
}

#[derive(Debug, Clone)]
struct Entry {
    seq: i64,
    item: Item,
}

/// A request as the handlers see it.
struct Req {
    method: Method,
    params: Vec<String>,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Option<Bytes>,
}

impl Req {
    fn q(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

type Answered = std::result::Result<Answer, Reply>;

fn bad(field: &str, message: impl Into<String>) -> Reply {
    Reply::error(400, "invalid_param", &message.into(), Some(field))
}

fn int_param(req: &Req, name: &str, lo: i64, hi: i64) -> std::result::Result<Option<i64>, Reply> {
    let Some(text) = req.q(name) else { return Ok(None) };
    match text.parse::<i64>() {
        Ok(v) if (lo..=hi).contains(&v) => Ok(Some(v)),
        _ if hi == i64::MAX => Err(bad(name, format!("{name} must be an integer >= {lo}"))),
        _ => Err(bad(name, format!("{name} must be within {lo}..{hi}"))),
    }
}

fn choice_param<'a>(req: &'a Req, name: &str, allowed: &[&str]) -> std::result::Result<Option<&'a str>, Reply> {
    match req.q(name) {
        Some(v) if !allowed.contains(&v) => Err(bad(name, format!("{name} must be {}", join_or(allowed)))),
        other => Ok(other),
    }
}

fn join_or(words: &[&str]) -> String {
    let n = words.len();
    if n <= 2 { words.join(" or ") } else { format!("{} or {}", words[..n - 1].join(", "), words[n - 1]) }
}

fn layers_param(req: &Req, default: &[&str]) -> std::result::Result<Vec<String>, Reply> {
    let Some(text) = req.q("layers") else { return Ok(default.iter().map(|s| s.to_string()).collect()) };
    let layers: Vec<String> = text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
    match layers.iter().find(|l| !LAYERS.contains(&l.as_str())) {
        Some(unknown) => Err(Reply::error(400, "unknown_layer", &format!("unknown layer '{unknown}'"), Some("layers"))),
        None => Ok(layers),
    }
}

/// The grid frame a path names: `latest` (`None`) or an integer of 1 to 10 digits, as on the
/// unit.
fn frame_ref(text: &str) -> std::result::Result<Option<i64>, Reply> {
    if text == "latest" {
        return Ok(None);
    }
    if (1..=10).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(text.parse().ok());
    }
    Err(bad("g", "g must be an integer or latest"))
}

/// A frame with only the requested layers (and `missing` cut to them).
fn filter_layers(frame: &Value, layers: &[String]) -> Value {
    let mut f = frame.clone();
    let Some(obj) = f.as_object_mut() else { return f };
    obj.retain(|k, _| !LAYERS.contains(&k.as_str()) || layers.contains(k));
    if let Some(missing) = obj.get_mut("missing").and_then(Value::as_array_mut) {
        missing.retain(|l| l.as_str().is_some_and(|l| layers.iter().any(|x| x == l)));
        if missing.is_empty() {
            obj.remove("missing");
        }
    }
    f
}

fn int(v: &Value, key: &str) -> Option<i64> {
    v.get(key)?.as_i64()
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

fn fnv(text: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("\"{h:016x}\"")
}

/// A JSON value other than a string as the unit's messages show it (Python's `repr`): `None`,
/// `True`, `False`, else its JSON.
fn py_repr(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        other => other.to_string(),
    }
}

/// The unit's message for a time outside a source.
fn time_range(source: &Value) -> String {
    match source["duration"].as_f64() {
        Some(d) => format!("time must be within 0..{d:.3} s"),
        None => "time must be at least 0 s".to_string(),
    }
}

/// The `time` of a control body in `source`, if it has one: a number (`invalid_type` otherwise,
/// `null` too, as on the unit) within the source (`time_out_of_range`).
fn media_time_of(body: &Map<String, Value>, source: &Value) -> std::result::Result<Option<f64>, Reply> {
    let Some(value) = body.get("time") else { return Ok(None) };
    let Some(time) = value.as_f64() else {
        return Err(Reply::error(400, "invalid_type", &time_range(source), Some("time")));
    };
    if time < 0.0 || source["duration"].as_f64().is_some_and(|d| time > d) {
        return Err(Reply::error(400, "time_out_of_range", &time_range(source), Some("time")));
    }
    Ok(Some(time))
}

/// 503 `world_unavailable`: the session has no world yet.
fn world_unavailable() -> Reply {
    Reply::json(503, &json!({"error": "the Stable sim is not running", "code": "world_unavailable", "retry_ms": 1000}))
}

/// The unit's Stable sim, as the session and status replies give it.
fn stable() -> Value {
    json!({"state": "running", "fits": 4, "evidence": true, "fixer": "thin-v1"})
}

/// A side event of a stream (`None`: NDJSON sends only `gap`s).
fn side_event(ndjson: bool, name: &str, data: &Value) -> Option<Bytes> {
    match (ndjson, name) {
        (false, _) => Some(Bytes::from(format!("event: {name}\ndata: {data}\n\n"))),
        (true, "gap") => Some(Bytes::from(format!("{data}\n"))),
        (true, _) => None,
    }
}

fn lock(fake: &Mutex<Fake>) -> MutexGuard<'_, Fake> {
    fake.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the transport answers: a reply (maybe streamed), or no reply at all.
pub(super) enum Answer {
    Broken(Error),
    Reply(Reply, Option<StreamPlan>),
}

/// A stone's path over a session: (stone, path2d, path3d, first g, last g).
type Track = (Value, Vec<Value>, Vec<Value>, i64, i64);

/// A streamed body: what it sends, and the faults that apply to it.
pub(super) struct StreamPlan {
    operation: String,
    source: Source,
    /// A session stream: whether it counts as a viewer (`None`: a file).
    viewer: Option<bool>,
    drop_after: Option<usize>,
    delay: Option<DelayFn>,
    close_after: Option<Duration>,
}

/// What a streamed body sends.
enum Source {
    /// A file's chunks.
    Chunks(VecDeque<Bytes>),
    /// A session stream.
    Follow(Follow),
}

/// The next thing a streamed body does.
enum Next {
    Event(Bytes),
    /// Wait a moment, then look again (a camera phase is due on a linked clock).
    Wait,
    End,
}

/// A session stream (`frames/stream`, `raw/stream`): the current program's entries from where it
/// starts, then whatever the program gets while the stream is open (a new program, from its
/// start; the camera's next phases, which a clock of the fake's own moves on to).
struct Follow {
    /// The next playlist entry.
    pos: usize,
    raw: bool,
    ndjson: bool,
    layers: Vec<String>,
    /// Resuming: frames up to this cursor are skipped, and side events before the first frame.
    after: Option<i64>,
    started: bool,
    /// Frames sent, and the most to send.
    frames: usize,
    limit: Option<usize>,
    /// The `reset` of an event id of another boot, sent first.
    reset: Option<Bytes>,
    /// Injected side events, by the index of the event they are.
    injected: Vec<(usize, Bytes)>,
}

impl Follow {
    fn next(&mut self, fake: &mut Fake, clock: &ManualClock, sent: usize) -> Next {
        if self.injected.first().is_some_and(|(at, _)| *at <= sent) {
            return Next::Event(self.injected.remove(0).1);
        }
        if let Some(reset) = self.reset.take() {
            return Next::Event(reset);
        }
        fake.advance(clock.now());
        while self.limit.is_none_or(|l| self.frames < l) {
            self.pos = self.pos.max(fake.epoch);
            if let Some(entry) = fake.playlist.get(self.pos).cloned() {
                self.pos += 1;
                // playback passes the entry: a clock line of the recording is the state from here
                fake.pass(self.pos);
                if let Some(event) = self.render(&entry, &fake.boot) {
                    return Next::Event(event);
                }
            } else if !fake.skip_to_phase(clock, f64::INFINITY) {
                if fake.phase_pending() {
                    return Next::Wait;
                }
                break;
            }
        }
        // injected events beyond the last one come last
        if self.injected.is_empty() { Next::End } else { Next::Event(self.injected.remove(0).1) }
    }

    /// A playlist entry as this stream sends it (`None`: it skips it).
    fn render(&mut self, entry: &Entry, boot: &str) -> Option<Bytes> {
        match &entry.item {
            Item::Frame(_) if !self.started && self.after.is_some_and(|a| entry.seq <= a) => None,
            Item::Side(..) if !self.started => None,
            Item::Frame(frame) => {
                self.started = true;
                let data = if self.raw {
                    let mut packet = packet_of(frame)?;
                    packet["cursor"] = entry.seq.into();
                    packet
                } else {
                    filter_layers(frame, &self.layers)
                };
                self.frames += 1;
                let text = if self.ndjson {
                    format!("{data}\n")
                } else {
                    format!("id: {boot}-{}\nevent: frame\ndata: {data}\n\n", entry.seq)
                };
                Some(Bytes::from(text))
            }
            Item::Side(name, data) => side_event(self.ndjson, name, data),
        }
    }
}

/// An open session stream, counted in the fake's open streams (and its viewers, unless it
/// observes) until it ends or its reader drops it.
struct Open {
    fake: Arc<Mutex<Fake>>,
    viewer: bool,
}

impl Drop for Open {
    fn drop(&mut self) {
        let mut fake = lock(&self.fake);
        fake.streams -= 1;
        if self.viewer {
            fake.viewers -= 1;
        }
    }
}

/// A streamed body being sent.
struct Running {
    plan: StreamPlan,
    fake: Arc<Mutex<Fake>>,
    clock: Arc<ManualClock>,
    sent: usize,
    _open: Option<Open>,
}

impl Running {
    /// The next chunk: an event, the error of a dropped connection (its last), or `None` at its
    /// end.
    async fn next_chunk(&mut self) -> Option<Result<Bytes>> {
        loop {
            let next = if self.plan.drop_after.is_some_and(|k| self.sent >= k) { Next::End } else { self.next() };
            match next {
                Next::Event(chunk) => {
                    if let Some(wait) = self.plan.delay.as_ref().and_then(|d| d(&self.plan.operation, self.sent)) {
                        tokio::time::sleep(wait).await;
                    }
                    self.sent += 1;
                    return Some(Ok(chunk));
                }
                Next::Wait => tokio::time::sleep(PHASE_LOOK).await,
                Next::End => {
                    if let Some(wait) = self.plan.close_after {
                        tokio::time::sleep(wait).await;
                    }
                    return self.plan.drop_after.map(|_| {
                        Err(Error::transport(std::io::Error::new(
                            ErrorKind::ConnectionReset,
                            "the fake dropped the connection",
                        )))
                    });
                }
            }
        }
    }

    fn next(&mut self) -> Next {
        match &mut self.plan.source {
            Source::Chunks(chunks) => chunks.pop_front().map_or(Next::End, Next::Event),
            Source::Follow(follow) => follow.next(&mut lock(&self.fake), &self.clock, self.sent),
        }
    }
}

impl StreamPlan {
    fn into_stream(
        self,
        fake: Arc<Mutex<Fake>>,
        clock: Arc<ManualClock>,
    ) -> impl Stream<Item = Result<Bytes>> + Send + 'static {
        let open = self.viewer.map(|viewer| Open { fake: fake.clone(), viewer });
        let running = Running { plan: self, fake, clock, sent: 0, _open: open };
        // nothing after the end, or after the error of a dropped connection
        futures::stream::unfold(Some(running), |running| async move {
            let mut running = running?;
            let chunk = running.next_chunk().await?;
            let more = chunk.is_ok().then_some(running);
            Some((chunk, more))
        })
    }
}

impl Answer {
    pub(super) fn into_response(self, fake: &Arc<Mutex<Fake>>, clock: &Arc<ManualClock>) -> Result<Response> {
        match self {
            Answer::Broken(error) => Err(error),
            Answer::Reply(reply, plan) => {
                let body = match plan {
                    Some(plan) => Body::from_stream(plan.into_stream(fake.clone(), clock.clone())),
                    None => Body::from_bytes(reply.body),
                };
                Ok(Response { status: reply.status, headers: Headers(reply.headers), body })
            }
        }
    }
}

impl From<Reply> for Answer {
    fn from(reply: Reply) -> Answer {
        Answer::Reply(reply, None)
    }
}

pub(super) struct Fake {
    pub(super) boot: String,
    /// Where the fake's clock starts: the recording's first clock line's `server_now`, else the
    /// epoch it was made with.
    pub(super) started: f64,
    /// A clock the fake never moves (`FakeFactory::with_linked_clock`).
    pub(super) linked: bool,
    header: Option<Value>,
    /// The catalog, as JSON: the recording's source, or a recorded catalog's, then the camera.
    catalog: Vec<Value>,
    /// The id of the recording's own source (its header's, else its first frame's video).
    rec_src: Option<String>,
    live: bool,
    base: Vec<Arc<Value>>,
    playlist: Vec<Entry>,
    epoch: usize,
    next_seq: i64,
    state: State,
    /// The recording's clock lines, each with its playlist index, and how many of them playback
    /// has passed (`Fake::pass`): the state is the newest of those until a control takes over.
    recorded: Vec<(usize, State)>,
    passed: usize,
    /// The rate a recording selected later plays at.
    rate: f64,
    /// The newest session id so far: a new session takes the next.
    newest_session: i64,
    /// The recording's last worker line.
    worker: Option<Value>,
    replies: HashMap<String, Reply>,
    /// The keys of the replies given one by one (`FakeFactory::with_reply`).
    given: HashSet<String>,
    pub(super) files: HashMap<String, (Bytes, usize)>,
    viewers: i64,
    /// Session streams open now.
    streams: i64,
    pub(super) faults: HashMap<String, VecDeque<Fault>>,
    pub(super) drops: HashMap<String, VecDeque<usize>>,
    pub(super) injections: HashMap<String, VecDeque<(String, Value, usize)>>,
    pub(super) delay: Option<DelayFn>,
    pub(super) close_after: Option<Duration>,
    pub(super) requests: Vec<Request>,
    pub(super) streams_opened: usize,
    /// The camera's phases still to come, each with the wall time it is due.
    phases: Vec<(f64, State)>,
    /// The camera's frames after its warm-up, released when it goes live.
    held: Vec<Value>,
    /// The `g` and the session time after the camera's newest frame.
    cam_g: i64,
    cam_t: f64,
}

impl Fake {
    pub(super) fn new(lines: Vec<Value>, epoch: f64) -> Fake {
        let mut header = None;
        let mut playlist = Vec::new();
        let mut seq = 0;
        for line in lines {
            if line.get("recording").is_some() && header.is_none() {
                header = Some(line);
            } else if let Some(name) = line.get("sse").and_then(Value::as_str) {
                playlist.push(Entry { seq, item: Item::Side(name.to_string(), Arc::new(line["data"].clone())) });
            } else if line.get("g").is_some() && line.get("cursor").is_some() {
                seq = (seq + 1).max(int(&line, "cursor").unwrap_or(0));
                playlist.push(Entry { seq, item: Item::Frame(Arc::new(line)) });
            }
        }
        let base: Vec<Arc<Value>> =
            playlist.iter().filter_map(|e| if let Item::Frame(f) = &e.item { Some(f.clone()) } else { None }).collect();
        let source =
            header.as_ref().map(|h| h["source"].clone()).filter(Value::is_object).or_else(|| recording_entry(&base));
        let rec_src = source.as_ref().and_then(|s| s["id"].as_str()).map(str::to_string);
        let catalog: Vec<Value> = source.into_iter().collect();
        // the recording's clock lines, each with its playlist index: the state is the first one's
        // until playback passes the next (`Fake::pass`)
        let clocks: Vec<(usize, ClockEvent)> = playlist
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match &e.item {
                Item::Side(name, data) if name == "clock" => {
                    Some((i, serde_json::from_value::<ClockEvent>((**data).clone()).ok()?))
                }
                _ => None,
            })
            .collect();
        let started = clocks.first().and_then(|(_, c)| c.server_now).unwrap_or(epoch);
        let recorded: Vec<(usize, State)> = clocks.into_iter().map(|(i, c)| (i, c.state)).collect();
        let worker = playlist.iter().rev().find_map(|e| match &e.item {
            Item::Side(name, data) if name == "worker" => Some((**data).clone()),
            _ => None,
        });
        let last = base.last().cloned();
        let state = match recorded.first() {
            Some((_, first)) => first.clone(),
            None => {
                let video = match (&last, catalog.first()) {
                    (Some(f), _) if f["video"].is_string() => f["video"].as_str().unwrap_or_default().to_string(),
                    (_, Some(c)) => c["id"].as_str().unwrap_or_default().to_string(),
                    _ => "fake".to_string(),
                };
                State {
                    session: last.as_ref().and_then(|f| int(f, "session")).unwrap_or(1),
                    revision: 1,
                    duration: catalog.first().and_then(|c| c["duration"].as_f64()),
                    video,
                    playing: true,
                    rate: 0.8,
                    anchor_media: last.as_ref().and_then(|f| num(f, "t")).unwrap_or(0.0),
                    anchor_wall: started,
                    changed_by: ChangedBy::Start,
                    ..State::default()
                }
            }
        };
        let sessions = base.iter().filter_map(|f| int(f, "session")).chain(recorded.iter().map(|(_, s)| s.session));
        let newest_session = sessions.fold(state.session, i64::max);
        let passed = recorded.len().min(1);
        let mut fake = Fake {
            boot: "fakeb00t01".to_string(),
            started,
            linked: false,
            header,
            catalog,
            rec_src,
            live: false,
            base,
            next_seq: seq + 1,
            playlist,
            epoch: 0,
            state,
            recorded,
            passed,
            rate: 0.8,
            newest_session,
            worker,
            replies: HashMap::new(),
            given: HashSet::new(),
            files: HashMap::new(),
            viewers: 0,
            streams: 0,
            faults: HashMap::new(),
            drops: HashMap::new(),
            injections: HashMap::new(),
            delay: None,
            close_after: None,
            requests: Vec::new(),
            streams_opened: 0,
            phases: Vec::new(),
            held: Vec::new(),
            cam_g: 0,
            cam_t: 0.0,
        };
        fake.rate = fake.start_rate(fake.recorded.last().map_or(&fake.state, |(_, s)| s));
        fake
    }

    /// The rate a recording selected later plays at: that of `state` (the recording's last clock
    /// line, or a recorded session reply), or 0.8 when `state` shows a camera, whose rate is the
    /// camera's own 1.0 on the unit. 0.8 is the rate of every recording on build
    /// `claude-20261114-int17`; build `claude-20261122-hr4x1` sets the rate itself (`auto-rate`:
    /// `dry1-0000-0130` goes from 0.8 to 0.74 and 0.75 in its first 23 s), which the fake never
    /// does.
    fn start_rate(&self, state: &State) -> f64 {
        if self.is_live_video(&state.video) { 0.8 } else { state.rate }
    }

    /// Adds replies (`given`: one by one, with `FakeFactory::with_reply`). Without a recording's
    /// source, a recorded catalog is the fake's (the camera after it, sized like its first entry);
    /// without frames, and with no camera shown, a recorded session reply gives the state.
    pub(super) fn add_replies(&mut self, replies: Vec<(String, Reply)>, given: bool) {
        if given {
            self.given.extend(replies.iter().map(|(key, _)| key.clone()));
        }
        self.replies.extend(replies);
        let listed = self.replies.get("listVideos").and_then(|r| serde_json::from_slice::<Value>(&r.body).ok());
        if let Some(Value::Array(videos)) = listed.map(|list| list["videos"].clone()).filter(|_| self.rec_src.is_none())
        {
            self.catalog = videos;
            if self.live {
                let like = self.catalog.first().cloned();
                self.catalog.push(camera_entry(like.as_ref()));
            }
        }
        if !self.base.is_empty() || self.live {
            return;
        }
        if let Some(reply) =
            self.replies.get("getSession").and_then(|r| serde_json::from_slice::<SessionReply>(&r.body).ok())
        {
            self.state = reply.state;
            self.passed = self.recorded.len(); // the recording's clock lines no longer apply
            self.worker = serde_json::to_value(reply.worker).ok();
            self.newest_session = self.newest_session.max(self.state.session);
            self.rate = self.start_rate(&self.state);
        }
    }

    /// Offers the live camera (after the recordings) and shows it, as if it had been selected
    /// just long enough ago to be live now.
    pub(super) fn offer_camera(&mut self, now: f64) {
        if self.live {
            return;
        }
        self.live = true;
        let like = self.catalog.first().cloned();
        self.catalog.push(camera_entry(like.as_ref()));
        let session = self.base.first().and_then(|f| int(f, "session")).unwrap_or(1);
        let (frames, live_t, next_g, next_t) = self.camera_run(LIVE_CAMERA, session, 0, 0.0);
        let since = now - live_t - LIVE_LAG_S;
        let start = State { session, revision: 1, playing: true, changed_by: ChangedBy::Start, ..State::default() };
        let [_, (_, on_air)] = camera_phases(&self.connecting(LIVE_CAMERA, start, since), since, live_t);
        self.state = on_air;
        self.passed = self.recorded.len(); // the recording's clock lines no longer apply
        (self.cam_g, self.cam_t) = (next_g, next_t);
        self.rate = 0.8;
        self.newest_session = self.newest_session.max(session);
        self.epoch = self.playlist.len();
        self.push_clock(now);
        self.release(frames);
    }

    /// The warm-up of camera `video`: its `hold`, 3 s by default.
    fn warmup(&self, video: &str) -> f64 {
        self.entry(video).and_then(|s| s["hold"].as_f64()).filter(|h| *h > 0.0).unwrap_or(3.0)
    }

    /// A run of camera `video`'s frames in session `session`: the recording's frames, its sessions
    /// end to end, as the camera sends them once connected at session time `t0` (seconds since
    /// the session started), numbered from `g0` and arriving from [`CONNECT_S`] after `t0` at the
    /// recording's pace. Returns the frames released after the run's warm-up (those of its first
    /// `hold` seconds are numbered but never released), the `t` the run goes live at (its first
    /// released frame's; the warm-up's end when there is none), and the `g` and `t` after its last
    /// frame.
    fn camera_run(&self, video: &str, session: i64, g0: i64, t0: f64) -> (Vec<Value>, f64, i64, f64) {
        let warm_end = t0 + CONNECT_S + self.warmup(video);
        let mut rebase = Rebase { next_g: g0, next_t: t0 + CONNECT_S, ..Rebase::default() };
        let mut released = Vec::new();
        for frame in &self.base {
            let mut f = (**frame).clone();
            let recorded_g = int(&f, "g").unwrap_or(0);
            rebase.apply(&mut f);
            if num(&f, "t").unwrap_or(0.0) >= warm_end - 1e-9 {
                camera_frame(&mut f, recorded_g, session, video);
                released.push(f);
            }
        }
        let live_t = released.first().and_then(|f| num(f, "t")).unwrap_or(warm_end);
        (released, live_t, rebase.next_g, rebase.next_t)
    }

    /// The state of a new session of camera `video` that starts at wall time `since` (`start`
    /// gives its `session`, `revision`, `playing` and `changed_by`): `connecting`, from media time
    /// 0 at rate 1.0, held until it goes live.
    fn connecting(&self, video: &str, start: State, since: f64) -> State {
        let live = Live { phase: LivePhase::Connecting, warmup: self.warmup(video), since, ..Live::default() };
        State {
            video: video.to_string(),
            rate: 1.0,
            anchor_wall: since,
            hold_until: since + LIVE_HOLD,
            live: Some(live),
            ..start
        }
    }

    /// A playing camera session takes the phases whose wall time has come (nothing runs while it
    /// is paused: a play plans the phases again).
    pub(super) fn advance(&mut self, now: f64) {
        while self.state.playing && self.phases.first().is_some_and(|(at, _)| *at <= now) {
            let (at, phase) = self.phases.remove(0);
            self.take_phase(at, phase);
        }
    }

    /// How a stream or a long poll waits for the camera's next phase on the fake's own clock: it
    /// moves the clock on to the phase, if it comes by wall time `until`, and takes it, while the
    /// session plays. False when there is none to take (a linked clock is never moved).
    fn skip_to_phase(&mut self, clock: &ManualClock, until: f64) -> bool {
        match self.phases.first() {
            Some(&(at, _)) if !self.linked && self.state.playing && at <= until => {
                clock.set(clock.now().max(at));
                self.advance(clock.now());
                true
            }
            _ => false,
        }
    }

    /// Whether a stream waiting on a linked clock should look again for the camera's next phase.
    fn phase_pending(&self) -> bool {
        self.linked && self.state.playing && !self.phases.is_empty()
    }

    /// Takes a camera phase due at wall time `at`, after any control made since it was planned
    /// (a higher revision; the playing state, the focus and what changed it kept). Going live
    /// releases the frames after the warm-up.
    fn take_phase(&mut self, at: f64, phase: State) {
        let kept = std::mem::take(&mut self.state);
        let revision = phase.revision.max(kept.revision + 1);
        self.state = State { revision, playing: kept.playing, focus: kept.focus, changed_by: kept.changed_by, ..phase };
        self.push_clock(at);
        if self.state.live_phase() == Some(LivePhase::Live) {
            let held = std::mem::take(&mut self.held);
            self.release(held);
        }
    }

    pub(super) fn state_now(&self, now: f64) -> State {
        let mut state = self.state.clone();
        state.media_time = media_time(&state, now);
        state
    }

    fn take_fault(&mut self, op: &str) -> Option<Fault> {
        let queue = self.faults.get_mut(op)?;
        let fault = queue.front_mut()?;
        fault.times = fault.times.saturating_sub(1);
        let taken = fault.clone();
        if fault.times == 0 {
            queue.pop_front();
        }
        Some(taken)
    }

    pub(super) fn answer(&mut self, request: Request, clock: &ManualClock) -> Answer {
        self.advance(clock.now());
        let (path, query) = split_url(&request.url);
        let (op, params) = if self.files.contains_key(&path) {
            ("getFile".to_string(), vec![path.clone()])
        } else {
            route(request.method, &path)
        };
        self.requests.push(request.clone());
        if let Some(fault) = self.take_fault(&op) {
            return match fault.reply {
                Some(reply) => reply.into(),
                None => Answer::Broken(Error::transport(std::io::Error::new(
                    ErrorKind::ConnectionRefused,
                    "the fake refused the connection",
                ))),
            };
        }
        let req = Req {
            method: request.method,
            params,
            query: parse_query(&query),
            headers: request.headers,
            body: request.body,
        };
        self.handle(&op, &req, clock).unwrap_or_else(Answer::from)
    }

    fn recorded(&self, key: &str) -> Option<Reply> {
        self.replies.get(key).cloned()
    }

    fn recorded_or_404(&self, key: &str) -> Reply {
        self.recorded(key).unwrap_or_else(|| {
            Reply::error(404, "not_found", &format!("the fake has no recorded reply of {key}"), None)
        })
    }

    fn handle(&mut self, op: &str, req: &Req, clock: &ManualClock) -> Answered {
        let now = clock.now();
        if self.base.is_empty() && FROM_FRAMES.contains(&op) {
            if let Some(reply) = self.recorded_variant(op, req) {
                return Ok(reply.into());
            }
            if matches!(op, "pollFrames" | "streamFrames" | "rawStream") {
                return Ok(self.recorded_or_404(op).into());
            }
        }
        let reply = match op {
            "getApiRoot" => self.recorded(op).unwrap_or_else(|| self.api_root()),
            "getHealth" => self.recorded(op).unwrap_or_else(|| {
                Reply::json(200, &json!({"ok": true, "worker": "running", "viewers": self.viewers}))
            }),
            "getReady" => self.recorded(op).unwrap_or_else(|| Reply::json(200, &json!({"ready": true, "waiting": []}))),
            "getStatus" => self.recorded(op).unwrap_or_else(|| self.status()),
            "getOpenApi" | "getModel" | "getTopology" | "getMetrics" => self.recorded_or_404(op),
            "listVideos" => self.list_videos(req),
            "getVideo" => self.get_video(req)?,
            "getVideoMedia" => return self.media(req),
            "getSession" => self.session_reply(now),
            "sessionSelect" | "sessionSeek" | "sessionPlay" | "sessionPause" | "putFocus" | "deleteFocus" => {
                self.control(op, req, now)?
            }
            "getWorld" => self.world(req)?,
            "getWorldEvents" => self.world_events(req)?,
            "getStone" => self.stone(req)?,
            "getFocus" => self.focus_reply(),
            "getFrame" | "rawPacket" => self.frame(op, req)?,
            "pollFrames" => self.poll(req, clock)?,
            "streamFrames" | "rawStream" => return self.stream(op, req),
            "getFile" => return Ok(self.file(&req.params[0])),
            "getFrameRaster" => self.raster(req)?,
            "getFramePicture" => match self.recorded(op) {
                Some(reply) => reply,
                None => self.picture(req)?,
            },
            "renderOverlay2d" | "renderScene3d" => self.render(op, req)?,
            "rawTensors" => self.tensors(req)?,
            "-methodNotAllowed" => {
                Reply::error(405, "method_not_allowed", &format!("{} is not allowed here", req.method.as_str()), None)
            }
            _ => Reply::error(404, "not_found", "no such route", None),
        };
        Ok(reply.into())
    }

    /// Without a recording: the recorded reply of an operation, by its usual variant.
    fn recorded_variant(&self, op: &str, req: &Req) -> Option<Reply> {
        let variant = match op {
            "getFrame" | "rawPacket" => req.params.first().map(|g| format!("{op}-{g}")),
            "pollFrames" => Some("pollFrames-world".to_string()),
            "streamFrames" | "rawStream" => {
                Some(format!("{op}-{}", if req.q("format") == Some("ndjson") { "ndjson" } else { "sse" }))
            }
            _ => None,
        };
        variant.and_then(|v| self.recorded(&v)).or_else(|| self.recorded(op))
    }

    fn api_root(&self) -> Reply {
        let build = self.header.as_ref().and_then(|h| h["build"].as_str()).unwrap_or("fake");
        Reply::json(
            200,
            &json!({
                "api": "2.2.0", "build": build, "boot": self.boot, "backend": "fake",
                "spec": "/api/v2/openapi.yaml", "docs": "/api/docs/",
                "layers": {"raw": "sip1", "overlay": "ovl1", "scene": "scn1", "cards": "crd1", "world": "stw1"},
                "render": {"overlay2d": "r2d1", "scene3d": "r3d1"},
            }),
        )
    }

    /// The unit's status, made up: the worker, the Stable sim, the viewers, the open streams and the
    /// retained window.
    fn status(&self) -> Reply {
        let build = self.header.as_ref().and_then(|h| h["build"].as_str()).unwrap_or_default();
        Reply::json(
            200,
            &json!({
                "build": build, "boot": self.boot, "uptime_s": 0.0, "page": "next", "worker": self.worker_json(),
                "stable": stable(), "hosts": {}, "viewers": self.viewers, "long_polls": 0, "streams": self.streams,
                "retained": self.retained(),
            }),
        )
    }

    /// The worker: the recording's last worker line, else running on the current session.
    fn worker_json(&self) -> Value {
        self.worker.clone().unwrap_or_else(
            || json!({"state": "running", "reason": "", "session": self.state.session, "video": self.state.video}),
        )
    }

    /// The catalog: a `listVideos` reply given with `FakeFactory::with_reply` as it is, else the
    /// fake's own (with an `ETag`, and 304 when it has not changed).
    fn list_videos(&self, req: &Req) -> Reply {
        if let Some(given) = self.replies.get("listVideos").filter(|_| self.given.contains("listVideos")) {
            return given.clone();
        }
        let default = self.catalog.first().map(|c| c["id"].clone());
        let body = json!({"schema": "cat1", "title": "pDigger Factory (fake)", "default_video": default, "videos": self.catalog});
        let text = body.to_string();
        let etag = fnv(text.as_bytes());
        if req.header("if-none-match") == Some(etag.as_str()) {
            return Reply::new(304, "application/json", Bytes::new()).header("ETag", &etag);
        }
        Reply::new(200, "application/json; charset=utf-8", text).header("ETag", &etag)
    }

    /// The catalog entry of source `id`.
    fn entry(&self, id: &str) -> Option<&Value> {
        self.catalog.iter().find(|c| c["id"] == id)
    }

    /// Whether `id` is a live camera: one the catalog lists as live, or the design's old id `live`.
    fn is_live_video(&self, id: &str) -> bool {
        id == crate::session::LIVE || self.entry(id).is_some_and(|e| e["live"] == true)
    }

    /// The source a control's `video` names, or the unit's error: missing (`missing_field`), not a
    /// string (`invalid_type`, `null` too), not in the catalog (`unknown_video`).
    fn video_of(&self, body: &Map<String, Value>) -> std::result::Result<Value, Reply> {
        let id = match body.get("video") {
            None => return Err(Reply::error(400, "missing_field", "Unknown video None", Some("video"))),
            Some(Value::String(id)) => id,
            Some(other) => {
                let message = format!("Unknown video {}", py_repr(other));
                return Err(Reply::error(400, "invalid_type", &message, Some("video")));
            }
        };
        self.entry(id)
            .cloned()
            .ok_or_else(|| Reply::error(400, "unknown_video", &format!("Unknown video '{id}'"), Some("video")))
    }

    fn get_video(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        let id = &req.params[0];
        self.entry(id)
            .map(|s| Reply::json(200, s))
            .ok_or_else(|| Reply::error(404, "unknown_video", &format!("Unknown video '{id}'"), Some("id")))
    }

    fn media(&self, req: &Req) -> Answered {
        let id = &req.params[0];
        if self.entry(id).is_none() {
            return Err(Reply::error(404, "unknown_video", &format!("Unknown video '{id}'"), Some("id")));
        }
        if self.is_live_video(id) {
            let message = format!(
                "{id} is a live camera: it has no proxy video; each frame's picture is \
                 /api/v2/session/frames/{{g}}/image.jpg"
            );
            return Err(Reply::error(404, "not_found", &message, Some("id")));
        }
        let recorded = self.recorded("getVideoMedia-range").or_else(|| self.recorded("getVideoMedia"));
        let recorded =
            recorded.ok_or_else(|| Reply::error(404, "not_found", &format!("the fake has no media of {id}"), None))?;
        let etag = recorded.header_value("etag").unwrap_or("\"fake\"").to_string();
        let total = recorded
            .header_value("content-range")
            .and_then(|r| r.rsplit('/').next())
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(recorded.body.len() as u64);
        let have = recorded.body.len() as u64;
        let base = |status: u16, body: Bytes| {
            Reply::new(status, "video/mp4", body).header("ETag", &etag).header("Accept-Ranges", "bytes")
        };
        if req.header("if-none-match") == Some(etag.as_str()) {
            return Ok(base(304, Bytes::new()).into());
        }
        if req.method == Method::Head {
            return Ok(base(200, Bytes::new()).header("Content-Length", &total.to_string()).into());
        }
        let Some(range) = req.header("range") else {
            return Ok(base(200, recorded.body).header("Content-Length", &have.to_string()).into());
        };
        let span = range.trim().strip_prefix("bytes=").and_then(|r| r.split_once('-')).and_then(|(a, b)| {
            match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
                (Some(a), Some(b)) => Some((a, b.min(total.saturating_sub(1)))),
                (Some(a), None) => Some((a, total.saturating_sub(1))),
                (None, Some(n)) => Some((total.saturating_sub(n), total.saturating_sub(1))),
                (None, None) => None,
            }
        });
        match span {
            Some((start, end)) if start <= end && start < have => {
                let end = end.min(have - 1);
                let body = recorded.body.slice(start as usize..=end as usize);
                let length = body.len().to_string();
                Ok(base(206, body)
                    .header("Content-Range", &format!("bytes {start}-{end}/{total}"))
                    .header("Content-Length", &length)
                    .into())
            }
            _ => Err(Reply::new(416, "text/plain", "range not satisfiable")
                .header("Content-Range", &format!("bytes */{total}"))),
        }
    }

    fn session_reply(&self, now: f64) -> Reply {
        Reply::json(
            200,
            &json!({"server_now": now, "server_recv": now, "state": self.state_now(now), "worker": self.worker_json(), "viewers": self.viewers, "stable": stable()}),
        )
    }

    // ---- frames of the playlist ---------------------------------------------------------------

    /// Frames of the current epoch (what streams and the long poll serve).
    fn epoch_frames(&self) -> Vec<(i64, Arc<Value>)> {
        self.playlist[self.epoch..]
            .iter()
            .filter_map(|e| if let Item::Frame(f) = &e.item { Some((e.seq, f.clone())) } else { None })
            .collect()
    }

    /// The newest frame of each grid frame of the current session.
    fn session_frames(&self) -> BTreeMap<i64, Arc<Value>> {
        let session = self.state.session;
        self.epoch_frames()
            .into_iter()
            .filter(|(_, f)| int(f, "session") == Some(session))
            .filter_map(|(_, f)| Some((int(&f, "g")?, f)))
            .collect()
    }

    fn world_frames(&self) -> Vec<Arc<Value>> {
        self.session_frames().into_values().filter(|f| f["world"].is_object()).collect()
    }

    fn newest_world(&self) -> Option<Arc<Value>> {
        self.world_frames().pop()
    }

    fn retained(&self) -> Value {
        let frames = self.session_frames();
        let first = frames.values().next();
        let last = frames.values().next_back();
        json!({
            "session": self.state.session,
            "g0": first.and_then(|f| int(f, "g")), "g1": last.and_then(|f| int(f, "g")),
            "t0": first.and_then(|f| num(f, "t")), "t1": last.and_then(|f| num(f, "t")),
            "n": frames.len(),
        })
    }

    /// 410 for a `session` that is not the current one (an empty value is no session).
    fn session_guard(&self, req: &Req) -> std::result::Result<(), Reply> {
        let Some(text) = req.q("session").filter(|s| !s.is_empty()) else { return Ok(()) };
        let session: i64 = text.parse().map_err(|_| bad("session", "session must be an integer"))?;
        if session == self.state.session {
            return Ok(());
        }
        Err(Reply::error(410, "session_gone", &format!("session {session} has ended"), Some("session")))
    }

    fn world(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        choice_param(req, "precision", &["rounded", "full"])?;
        let frame = self.newest_world().ok_or_else(world_unavailable)?;
        Ok(Reply::json(200, &frame["world"]))
    }

    fn world_events(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        let after = int_param(req, "after", -1, i64::MAX)?.unwrap_or(-1);
        let limit = int_param(req, "limit", 1, 1024)?.unwrap_or(256) as usize;
        let types: Option<Vec<&str>> = req.q("type").map(|t| t.split(',').filter(|s| !s.is_empty()).collect());
        if let Some(bad_type) = types.iter().flatten().find(|t| !EVENT_TYPES.contains(t)) {
            return Err(bad("type", format!("unknown event type '{bad_type}'")));
        }
        let frames = self.world_frames();
        let mut events = Vec::new();
        let mut next_after = after;
        for frame in frames.iter().filter(|f| int(f, "g").is_some_and(|g| g > after)) {
            if events.len() >= limit {
                break;
            }
            for event in frame["world"]["events"].as_array().into_iter().flatten() {
                if types.as_ref().is_none_or(|t| event["type"].as_str().is_some_and(|k| t.contains(&k))) {
                    let mut entry = json!({"g": frame["g"], "t": frame["t"]});
                    entry
                        .as_object_mut()
                        .into_iter()
                        .for_each(|o| o.extend(event.as_object().cloned().unwrap_or_default()));
                    events.push(entry);
                }
            }
            next_after = int(frame, "g").unwrap_or(after);
        }
        let mut body = json!({"session": self.state.session, "video": self.state.video, "events": events, "next_after": next_after});
        if let Some(oldest) = self.session_frames().keys().next() {
            body["oldest_g"] = (*oldest).into();
        }
        Ok(Reply::json(200, &body))
    }

    /// The path of stone `id` over the session.
    fn track(&self, id: i64) -> Option<Track> {
        let mut found: Option<Track> = None;
        for frame in self.world_frames() {
            let Some(stone) =
                frame["world"]["stones"].as_array().into_iter().flatten().find(|s| int(s, "id") == Some(id))
            else {
                continue;
            };
            let g = int(&frame, "g").unwrap_or(0);
            let entry = found.get_or_insert_with(|| (Value::Null, Vec::new(), Vec::new(), g, g));
            entry.0 = stone.clone();
            entry.4 = g;
            let outline: Vec<(f64, f64)> = stone["outline"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?)))
                .collect();
            if !outline.is_empty() {
                let n = outline.len() as f64;
                let (x, y) = outline.iter().fold((0.0, 0.0), |(x, y), p| (x + p.0, y + p.1));
                entry.1.push(json!([x / n, y / n]));
            }
            if stone["solid"]["pos"].is_array() {
                entry.2.push(stone["solid"]["pos"].clone());
            }
        }
        found
    }

    fn stone(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        let text = &req.params[0];
        let id: i64 = text.parse().map_err(|_| bad("id", "id must be an integer"))?;
        let unknown = || Reply::error(404, "unknown_stone", &format!("no stone {id} in the world"), Some("id"));
        let newest = self.newest_world().ok_or_else(unknown)?;
        let in_world = newest["world"]["stones"].as_array().into_iter().flatten().any(|s| int(s, "id") == Some(id));
        let (stone, path2d, path3d, first_g, last_g) = self.track(id).filter(|_| in_world).ok_or_else(unknown)?;
        Ok(Reply::json(
            200,
            &json!({"schema": "sts1", "session": self.state.session, "video": self.state.video, "id": id, "g": newest["g"], "t": newest["t"], "stone": stone, "first_g": first_g, "last_g": last_g, "path2d": path2d, "path3d": path3d}),
        ))
    }

    fn focus_reply(&self) -> Reply {
        let Some(focus) = &self.state.focus else { return Reply::json(200, &json!({"focus": null, "resolved": null})) };
        let resolved = self
            .track(focus.id)
            .map(|(_, path2d, path3d, _, _)| json!({"id": focus.id, "path2d": path2d, "path3d": path3d}));
        Reply::json(200, &json!({"focus": focus, "resolved": resolved}))
    }

    /// A frame (`getFrame`) or its raw packet (`rawPacket`), checked in the unit's order: the
    /// layers and the precision (a frame's), a stale session, `g`, the frame.
    fn frame(&self, op: &str, req: &Req) -> std::result::Result<Reply, Reply> {
        let text = &req.params[0];
        let layers = if op == "getFrame" { layers_param(req, &DEFAULT_LAYERS)? } else { Vec::new() };
        if op == "getFrame" {
            choice_param(req, "precision", &["rounded", "full"])?;
        }
        self.session_guard(req)?;
        let g = frame_ref(text)?;
        let frames = self.session_frames();
        let found = match g {
            None => frames.values().next_back(),
            Some(g) => frames.get(&g),
        };
        match found {
            Some(frame) if op == "getFrame" => Ok(Reply::json(200, &filter_layers(frame, &layers))),
            Some(frame) => {
                packet_of(frame).map(|packet| Reply::json(200, &packet)).ok_or_else(|| self.not_retained(text, &frames))
            }
            None => Err(self.not_retained(text, &frames)),
        }
    }

    /// 404 `not_retained` of frame `text`, with the retained window (`latest` before any frame of
    /// the session is released: none is retained yet).
    fn not_retained(&self, text: &str, frames: &BTreeMap<i64, Arc<Value>>) -> Reply {
        let error = if text == "latest" && frames.is_empty() {
            "no frame is retained yet".to_string()
        } else {
            format!("frame {text} is not retained")
        };
        Reply::json(404, &json!({"error": error, "code": "not_retained", "field": "g", "retained": self.retained()}))
    }

    /// The camera picture of a released frame, checked in the unit's order: `g`, a stale session
    /// (gone whatever it showed), a recording, the frame, its picture (only the newest
    /// [`KEEP_PICTURES`] released frames have one).
    fn picture(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        let text = &req.params[0];
        let g = frame_ref(text)?;
        self.session_guard(req)?;
        if self.state.live.is_none() {
            let message = format!("{} is a recording: its picture is the proxy video", self.state.video);
            return Err(Reply::error(404, "not_found", &message, Some("g")));
        }
        let frames = self.session_frames();
        let found = match g {
            None => frames.values().next_back(),
            Some(g) => frames.get(&g),
        };
        let frame = found.ok_or_else(|| self.not_retained(text, &frames))?;
        let g = int(frame, "g").unwrap_or(0);
        let gs: Vec<i64> = frames.keys().copied().collect();
        if gs.len() > KEEP_PICTURES && g < gs[gs.len() - KEEP_PICTURES] {
            let message =
                format!("the picture of frame {g} is no longer kept: only the newest {KEEP_PICTURES} pictures are");
            return Err(Reply::error(404, "not_retained", &message, Some("g")));
        }
        let jpeg = fake_picture(frame["video"].as_str().unwrap_or_default(), g);
        let length = jpeg.len().to_string();
        Ok(Reply::new(200, "image/jpeg", jpeg)
            .header("Cache-Control", "no-store")
            .header("X-Interp-G", &g.to_string())
            .header("Content-Length", &length))
    }

    fn raster(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        req.params[0].parse::<i64>().map_err(|_| bad("g", "g must be a grid frame"))?;
        let raster = req.params[1].strip_suffix(".png").unwrap_or(&req.params[1]);
        if !["regions", "mask", "depth"].contains(&raster) {
            return Err(Reply::error(404, "not_found", "no such raster", None));
        }
        int_param(req, "scale", 1, 8)?;
        Ok(self.recorded_or_404(&format!("getFrameRaster-{raster}")))
    }

    fn render(&self, op: &str, req: &Req) -> std::result::Result<Reply, Reply> {
        let g = int_param(req, "g", 0, i64::MAX)?;
        let t = match req.q("t") {
            Some(t) => {
                Some(t.parse::<f64>().ok().filter(|t| *t >= 0.0).ok_or_else(|| bad("t", "t must be a number >= 0"))?)
            }
            None => None,
        };
        if g.is_some() == t.is_some() {
            return Err(bad("t", "give exactly one of g and t"));
        }
        choice_param(req, "mode", &["stable", "raw"])?;
        let draw = choice_param(req, "draw", &["shapes", "primitives"])?;
        let surface =
            if op == "renderScene3d" { choice_param(req, "surface", &["vertices", "compact", "none"])? } else { None };
        self.session_guard(req)?;
        let variant = match (surface, draw) {
            (Some("compact"), _) => "-compact",
            (Some("none"), _) => "-none",
            (_, Some("primitives")) => "-primitives",
            _ => "",
        };
        Ok(self
            .recorded(&format!("{op}{variant}"))
            .unwrap_or_else(|| Reply::error(404, "not_found", "the fake has no recorded render", None)))
    }

    fn tensors(&self, req: &Req) -> std::result::Result<Reply, Reply> {
        req.params[0].parse::<i64>().map_err(|_| bad("g", "g must be a grid frame"))?;
        Ok(self
            .recorded("rawTensors")
            .unwrap_or_else(|| Reply::error(404, "debug_disabled", "tensor capture is not enabled", None)))
    }

    /// The long poll. Time is not simulated: the reply comes at once with the next frames (up to
    /// the recording's next clock line, which the next reply carries), after the camera's phases
    /// due within its `wait` (a clock of the fake's own moves on to each).
    fn poll(&mut self, req: &Req, clock: &ManualClock) -> std::result::Result<Reply, Reply> {
        let cursor = int_param(req, "cursor", 0, i64::MAX)?.unwrap_or(0);
        let rev = int_param(req, "rev", 0, i64::MAX)?;
        if let Some(cid) = req.q("cid").filter(|c| {
            !(8..=40).contains(&c.len()) || !c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        }) {
            return Err(bad("cid", format!("cid must match ^[A-Za-z0-9_-]{{8,40}}$, not {cid:?}")));
        }
        let layers = layers_param(req, &DEFAULT_LAYERS)?;
        let limit = int_param(req, "limit", 1, 16)?.unwrap_or(8) as usize;
        let wait = match req.q("wait") {
            None => 1000,
            Some(text) => text
                .parse::<i64>()
                .ok()
                .filter(|w| (0..=1000).contains(w))
                .ok_or_else(|| bad("wait", "wait must be within 0..1000 ms"))?,
        };
        choice_param(req, "precision", &["rounded", "full"])?;
        let until = clock.now() + wait as f64 / 1000.0;
        while self.skip_to_phase(clock, until) {}
        let frames = self.epoch_frames();
        let last = frames.last().map_or(0, |(seq, _)| *seq);
        let mut taken: Vec<&(i64, Arc<Value>)> = frames.iter().filter(|(seq, _)| *seq > cursor).take(limit).collect();
        // Playback passes what the reply serves: the recording's clock lines before its first frame
        // are the state, and the reply ends before the next line, which the next reply carries.
        let first = taken.first().map(|(seq, _)| *seq);
        let at = self.playlist[self.epoch..]
            .iter()
            .position(|e| matches!(e.item, Item::Frame(_)) && Some(e.seq) == first)
            .map_or(self.playlist.len(), |p| self.epoch + p);
        self.pass(at);
        if let Some(&(line, _)) = self.recorded.get(self.passed) {
            let before = self.playlist[line].seq;
            taken.retain(|(seq, _)| *seq <= before);
        }
        // Nothing new for this client: the unit's poll waits out its `wait`. On the fake's own
        // clock that moves the clock on while a camera phase is to come, so a polling client
        // reaches it.
        let idle = taken.is_empty() && rev == Some(self.state.revision);
        if idle && !self.linked && self.state.playing && !self.phases.is_empty() {
            clock.set(clock.now().max(until));
        }
        let reply_cursor = taken.last().map_or(cursor.min(last), |(seq, _)| *seq);
        let out: Vec<Value> = taken.iter().map(|(_, f)| filter_layers(f, &layers)).collect();
        let mut body: Value = serde_json::from_slice(&self.session_reply(clock.now()).body).unwrap_or_default();
        body["cursor"] = reply_cursor.into();
        body["frames"] = out.into();
        Ok(Reply::json(200, &body))
    }

    fn stream(&mut self, op: &str, req: &Req) -> Answered {
        let raw = op == "rawStream";
        let format = match req.q("format") {
            Some(_) => choice_param(req, "format", &["ndjson", "sse"])?.unwrap_or("sse"),
            None if req.header("accept").is_some_and(|a| a.contains("ndjson")) => "ndjson",
            None => "sse",
        };
        let ndjson = format == "ndjson";
        let layers = if raw { Vec::new() } else { layers_param(req, &DEFAULT_LAYERS)? };
        let mut after = int_param(req, "after", 0, i64::MAX)?;
        let limit = int_param(req, "limit", 1, 4096)?;
        int_param(req, "max_s", 1, 3600)?;
        let observe = choice_param(req, "observe", &["0", "1"])? == Some("1");
        if !raw {
            choice_param(req, "precision", &["rounded", "full"])?;
        }
        let mut reset = None;
        if let Some(id) = req.header("last-event-id") {
            match id
                .rsplit_once('-')
                .filter(|(boot, _)| *boot == self.boot)
                .and_then(|(_, seq)| seq.parse::<i64>().ok())
            {
                Some(seq) => after = Some(seq),
                None => {
                    let newest = self.playlist.last().map_or(0, |e| e.seq);
                    reset = side_event(ndjson, "reset", &json!({"boot": self.boot, "cursor": newest}));
                    after = None;
                }
            }
        }
        // a stream after a cursor starts there: playback passed what comes before its first frame
        if let Some(after) = after {
            let start =
                self.playlist[self.epoch..].iter().position(|e| matches!(e.item, Item::Frame(_)) && e.seq > after);
            self.pass(start.map_or(self.playlist.len(), |p| self.epoch + p));
        }
        // each injected event goes out after `at` events of the stream, those injected before it
        // counted
        let mut injected: Vec<(usize, Bytes)> = Vec::new();
        for (name, data, at) in self.injections.remove(op).into_iter().flatten() {
            if let Some(event) = side_event(ndjson, &name, &data) {
                for (later, _) in injected.iter_mut().filter(|(p, _)| *p >= at) {
                    *later += 1;
                }
                injected.push((at, event));
            }
        }
        injected.sort_by_key(|(at, _)| *at);
        let drop_after = self.drops.get_mut(op).and_then(VecDeque::pop_front);
        self.streams_opened += 1;
        self.streams += 1;
        if !observe {
            self.viewers += 1;
        }
        let follow = Follow {
            pos: self.epoch,
            raw,
            ndjson,
            layers,
            started: after.is_none(),
            after,
            frames: 0,
            limit: limit.map(|l| l as usize),
            reset,
            injected,
        };
        let content_type = if ndjson { "application/x-ndjson" } else { "text/event-stream" };
        let plan = StreamPlan {
            operation: op.to_string(),
            source: Source::Follow(follow),
            viewer: Some(!observe),
            drop_after,
            delay: self.delay.clone(),
            close_after: self.close_after,
        };
        Ok(Answer::Reply(Reply::new(200, content_type, Bytes::new()), Some(plan)))
    }

    fn file(&mut self, path: &str) -> Answer {
        let (bytes, chunk) = self.files[path].clone();
        let chunks = bytes.chunks(chunk.max(1)).map(Bytes::copy_from_slice).collect();
        let drop_after = self.drops.get_mut("getFile").and_then(VecDeque::pop_front);
        let plan = StreamPlan {
            operation: "getFile".to_string(),
            source: Source::Chunks(chunks),
            viewer: None,
            drop_after,
            delay: self.delay.clone(),
            close_after: None,
        };
        Answer::Reply(Reply::new(200, "application/octet-stream", Bytes::new()), Some(plan))
    }

    // ---- controls -------------------------------------------------------------------------------

    fn body(&self, req: &Req, required: bool) -> std::result::Result<Map<String, Value>, Reply> {
        let body = req.body.as_deref().unwrap_or_default();
        if body.is_empty() && !required {
            return Ok(Map::new());
        }
        if !req.header("content-type").is_some_and(|t| t.starts_with("application/json")) {
            return Err(Reply::error(415, "json_only", "send the body as application/json", None));
        }
        match serde_json::from_slice::<Value>(body) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(Reply::error(400, "not_an_object", "the body must be a JSON object", None)),
            Err(_) => Err(Reply::error(400, "invalid_json", "the body is not JSON", None)),
        }
    }

    /// A control, checked in the unit's order (the body, `dry_run`, then its fields) and, unless
    /// it is a dry run, applied.
    fn control(&mut self, op: &str, req: &Req, now: f64) -> std::result::Result<Reply, Reply> {
        let body = match op {
            "deleteFocus" => Map::new(),
            "sessionPlay" | "sessionPause" => self.body(req, false)?,
            _ => self.body(req, true)?,
        };
        let dry = match req.q("dry_run") {
            None | Some("" | "0") => false,
            Some("1") => true,
            Some(_) => return Err(bad("dry_run", "dry_run must be 0 or 1")),
        };
        let before = (self.state.clone(), self.playlist.len(), self.epoch, self.next_seq, self.newest_session);
        let camera = (self.phases.clone(), self.held.clone(), self.cam_g, self.cam_t);
        let passed = self.passed;
        match op {
            "sessionSelect" => {
                let source = self.video_of(&body)?;
                let id = source["id"].as_str().unwrap_or_default().to_string();
                let is_camera = self.is_live_video(&id);
                if is_camera && body.contains_key("time") {
                    let message = format!("{id} is a live camera: leave time out");
                    return Err(Reply::error(400, "not_seekable", &message, Some("time")));
                }
                if is_camera {
                    self.start_camera(&id, now);
                } else {
                    let time = media_time_of(&body, &source)?;
                    self.start_session(&source, time, now);
                }
            }
            "sessionSeek" => {
                // the video it names, else the one shown; a camera refuses it, a recording is
                // sought even while the session shows a camera
                let source = if body.contains_key("video") {
                    Some(self.video_of(&body)?)
                } else {
                    self.entry(&self.state.video).cloned()
                };
                let source = source
                    .ok_or_else(|| Reply::error(409, "not_seekable", "the session shows no video", Some("video")))?;
                let id = source["id"].as_str().unwrap_or_default().to_string();
                if self.is_live_video(&id) {
                    let message = format!("{id} is a live camera and cannot seek");
                    return Err(Reply::error(409, "not_seekable", &message, Some("video")));
                }
                let time = media_time_of(&body, &source)?
                    .ok_or_else(|| Reply::error(400, "missing_field", &time_range(&source), Some("time")))?;
                self.start_session(&source, Some(time), now);
            }
            "sessionPlay" | "sessionPause" => {
                let playing = op == "sessionPlay";
                if self.state.playing != playing {
                    // a camera connects again on a play: its next frames start a new warm-up (a live
                    // one plays on at its live edge)
                    let edge = match self.state.live.as_mut() {
                        Some(live) if playing => {
                            let edge = if live.phase == LivePhase::Live {
                                Some((now - live.since - live.lag).max(0.0))
                            } else {
                                None
                            };
                            live.phase = LivePhase::Connecting;
                            edge
                        }
                        _ => None,
                    };
                    let media = edge.unwrap_or_else(|| media_time(&self.state, now));
                    self.state.playing = playing;
                    self.state.anchor_media = media;
                    self.state.anchor_wall = now;
                    self.changed(now);
                    if playing && self.state.live.is_some() {
                        self.restart_camera(now);
                    }
                }
            }
            "putFocus" => {
                let id = match body.get("stone_id") {
                    None => return Err(Reply::error(400, "missing_field", "stone_id is required", Some("stone_id"))),
                    Some(v) => v.as_i64().ok_or_else(|| {
                        Reply::error(400, "invalid_type", "stone_id must be an integer", Some("stone_id"))
                    })?,
                };
                if let Some(session) = body.get("session").and_then(Value::as_i64).filter(|s| *s != self.state.session)
                {
                    return Err(Reply::error(
                        409,
                        "stale_session",
                        &format!("session {session} is not the current session"),
                        Some("session"),
                    ));
                }
                let newest = self.newest_world().ok_or_else(world_unavailable)?;
                let stone = newest["world"]["stones"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|s| int(s, "id") == Some(id))
                    .cloned();
                let stone = stone.ok_or_else(|| {
                    Reply::error(404, "unknown_stone", &format!("no stone {id} in the world"), Some("stone_id"))
                })?;
                let at = |v: &Value, i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0);
                let (pos, dims, q) = (&stone["solid"]["pos"], &stone["solid"]["dims"], &stone["solid"]["q"]);
                let yaw = crate::codec::quat_to_yaw([at(q, 0), at(q, 1), at(q, 2), at(q, 3)]);
                self.take_over(); // the focus's `n` is its revision: after the recording's lines too
                self.state.focus = Some(Focus {
                    n: self.state.revision + 1,
                    session: self.state.session,
                    video: self.state.video.clone(),
                    id,
                    g: int(&newest, "g").unwrap_or(0),
                    t: num(&newest, "t").unwrap_or(0.0),
                    x: at(pos, 0),
                    y: at(pos, 1),
                    z: at(pos, 2),
                    length: at(dims, 0),
                    width: at(dims, 1),
                    yaw,
                    ..Focus::default()
                });
                self.changed(now);
            }
            _ => {
                if self.state.focus.take().is_some() {
                    self.changed(now);
                }
            }
        }
        let reply = ControlReply {
            state: self.state_now(now),
            server_now: now,
            server_recv: now,
            dry_run: Some(dry),
            ..ControlReply::default()
        };
        if dry {
            (self.state, self.epoch, self.next_seq, self.newest_session) = (before.0, before.2, before.3, before.4);
            self.playlist.truncate(before.1);
            (self.phases, self.held, self.cam_g, self.cam_t) = camera;
            self.passed = passed;
        }
        Ok(Reply::json(200, &reply))
    }

    /// A state change at wall time `now`: the recording's clock lines still to come no longer
    /// apply, the revision moves on (past theirs) and streams get a clock event.
    fn changed(&mut self, now: f64) {
        self.take_over();
        self.state.revision += 1;
        self.state.changed_by = ChangedBy::Viewer;
        self.push_clock(now);
    }

    /// Playback has passed the playlist up to entry `pos` (not included): each of the recording's
    /// clock lines before it is the state in turn, as it was on the unit at that point.
    fn pass(&mut self, pos: usize) {
        while let Some((_, state)) = self.recorded.get(self.passed).filter(|(at, _)| *at < pos) {
            self.state = state.clone();
            self.passed += 1;
        }
    }

    /// A control takes over from the recording: its clock lines still to come no longer apply,
    /// and the revision is lifted to theirs, so that a client that read them takes the control's
    /// state for the newer one.
    fn take_over(&mut self) {
        for (_, state) in &self.recorded[self.passed..] {
            self.state.revision = self.state.revision.max(state.revision);
        }
        self.passed = self.recorded.len();
    }

    /// Streams get a clock event of the state, sent at server wall time `server_now` (its media
    /// time then).
    fn push_clock(&mut self, server_now: f64) {
        let clock = json!(ClockEvent { state: self.state_now(server_now), server_now: Some(server_now) });
        self.playlist.push(Entry { seq: self.next_seq - 1, item: Item::Side("clock".to_string(), Arc::new(clock)) });
    }

    /// Releases `frames` to the playlist (streams and long polls serve them), each with the next
    /// cursor (a camera's raw layer too).
    fn release(&mut self, frames: Vec<Value>) {
        let camera = self.state.live.is_some();
        for mut f in frames {
            let seq = self.next_seq;
            self.next_seq += 1;
            f["cursor"] = seq.into();
            if camera && f["raw"].is_object() {
                f["raw"]["cursor"] = seq.into();
            }
            self.playlist.push(Entry { seq, item: Item::Frame(Arc::new(f)) });
        }
    }

    /// The id of a new session: the newest so far + 1.
    fn next_session(&mut self) -> i64 {
        self.newest_session += 1;
        self.newest_session
    }

    /// A new fake session on recording `source` (its catalog entry) at `at`: it replays the
    /// recording's frames from `at`, at the fake's rate for recordings, held for the source's
    /// `hold` (1.4 s when it has none).
    fn start_session(&mut self, source: &Value, at: Option<f64>, now: f64) {
        let video = source["id"].as_str().unwrap_or_default().to_string();
        let start = at.or(source["default_start"].as_f64()).unwrap_or(0.0);
        let session = self.next_session();
        self.state = State {
            session,
            revision: self.state.revision,
            video: video.clone(),
            playing: self.state.playing,
            rate: if self.rate <= 0.0 { 1.0 } else { self.rate },
            anchor_media: start,
            anchor_wall: now,
            hold_until: now + source["hold"].as_f64().unwrap_or(1.4),
            duration: source["duration"].as_f64(),
            changed_by: ChangedBy::Viewer,
            focus: None,
            ..State::default()
        };
        (self.phases, self.held) = (Vec::new(), Vec::new());
        self.epoch = self.playlist.len();
        self.changed(now);
        let from: Vec<&Arc<Value>> =
            self.base.iter().filter(|f| at.is_none() || num(f, "t").is_some_and(|t| t >= start)).collect();
        let frames = if from.is_empty() { self.base.iter().collect() } else { from };
        let frames = frames
            .into_iter()
            .map(|frame| {
                let mut f = (**frame).clone();
                retag(&mut f, session, &video);
                f
            })
            .collect();
        self.release(frames);
    }

    /// A new session of camera `video`: `connecting` now, its phases to come, its first run's
    /// frames held until it goes live.
    fn start_camera(&mut self, video: &str, now: f64) {
        let session = self.next_session();
        let start = State { session, revision: self.state.revision, playing: self.state.playing, ..State::default() };
        self.state = self.connecting(video, start, now);
        self.epoch = self.playlist.len();
        self.changed(now);
        let (frames, live_t, next_g, next_t) = self.camera_run(video, session, 0, 0.0);
        self.phases = camera_phases(&self.state, now, live_t).into();
        self.held = frames;
        (self.cam_g, self.cam_t) = (next_g, next_t);
    }

    /// Plans the camera session's next run after a play (the state is `connecting` again): the
    /// camera connects now and warms up again on its next frames. Once frames were released, the
    /// run is numbered after a break of [`LIVE_GAP`] frames (`g` jumps, as on the unit) and arrives
    /// after the frames already sent (the fake's camera sends its whole run when it goes live).
    fn restart_camera(&mut self, now: f64) {
        let since = self.state.live.as_ref().map_or(now, |l| l.since);
        let (mut g0, mut t0) = (0, now - since);
        if !self.session_frames().is_empty() {
            (g0, t0) = (self.cam_g + LIVE_GAP, t0.max(self.cam_t));
        }
        let video = self.state.video.clone();
        let (frames, live_t, next_g, next_t) = self.camera_run(&video, self.state.session, g0, t0);
        self.phases = camera_phases(&self.state, now, live_t).into();
        self.held = frames;
        (self.cam_g, self.cam_t) = (next_g, next_t);
    }
}

/// The phases a camera session takes once it connects at wall time `at`, from `base` (its
/// `connecting` state): `warming` [`CONNECT_S`] later, while the interpreter runs on the run's
/// first `hold` seconds (only the phase changes, as on the unit: the lag stays `base`'s, 0 after a
/// select and [`LIVE_LAG_S`] after a play), then `live` when the frame of time `live_t` is
/// released, [`LIVE_LAG_S`] behind it (the playhead starts there). Each bumps the revision, as the
/// unit's clock does.
fn camera_phases(base: &State, at: f64, live_t: f64) -> [(f64, State); 2] {
    let live = base.live.clone().unwrap_or_default();
    let warming = State {
        revision: base.revision + 1,
        live: Some(Live { phase: LivePhase::Warming, ..live.clone() }),
        ..base.clone()
    };
    let went_live = live.since + live_t + LIVE_LAG_S;
    let on_air = State {
        revision: base.revision + 2,
        anchor_media: live_t,
        anchor_wall: went_live,
        hold_until: went_live,
        live: Some(Live { phase: LivePhase::Live, lag: LIVE_LAG_S, ..live }),
        ..base.clone()
    };
    [(at + CONNECT_S, warming), (went_live, on_air)]
}

/// The recording's frames as one camera session: `g` and `t` from where the rebase starts
/// (`next_g`, `next_t`), its sessions end to end.
#[derive(Default)]
struct Rebase {
    segment: Option<Value>,
    base_g: i64,
    base_t: f64,
    g0: i64,
    t0: f64,
    next_g: i64,
    next_t: f64,
}

impl Rebase {
    fn apply(&mut self, f: &mut Value) {
        let (g, t) = (f["g"].as_i64().unwrap_or(0), f["t"].as_f64().unwrap_or(0.0));
        if self.segment.as_ref() != Some(&f["session"]) {
            self.segment = Some(f["session"].clone());
            (self.base_g, self.base_t, self.g0, self.t0) = (self.next_g, self.next_t, g, t);
        }
        let g = self.base_g + g - self.g0;
        let t = ((self.base_t + t - self.t0) * 1e6).round() / 1e6;
        self.next_g = self.next_g.max(g + 1);
        self.next_t = self.next_t.max(t + 0.2);
        f["g"] = g.into();
        f["t"] = t.into();
    }
}

/// The media time of `state` at server wall time `now`.
fn media_time(state: &State, now: f64) -> f64 {
    let mut t = state.anchor_media;
    if state.playing {
        t += (now - state.anchor_wall.max(state.hold_until)).max(0.0) * state.rate;
    }
    state.duration.map_or(t, |d| t.min(d))
}
