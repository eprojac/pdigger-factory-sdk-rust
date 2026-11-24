//! Test doubles for code that uses this SDK (spec 12), behind the `testing` feature:
//! [`FakeFactory`], an in-process [`Transport`] (and an optional local HTTP server) answering
//! the v2 API from a recording and/or recorded replies, with fault injection; and
//! [`ManualClock`]. Everything is deterministic: no network, no real clock (the fake's clock moves
//! when told, when the client waits on it, and when a stream or a long poll waits for a camera's
//! next phase; [`FakeFactory::with_linked_clock`] leaves it to its owner).
//!
//! ```
//! use futures::StreamExt;
//! use pdigger_factory_sdk::testing::FakeFactory;
//!
//! # #[tokio::main(flavor = "current_thread")] async fn main() -> pdigger_factory_sdk::Result<()> {
//! let fake = FakeFactory::from_values(vec![
//!     serde_json::json!({"recording": "pfr1", "video": "dry1"}),
//!     serde_json::json!({"cursor": 1, "session": 7, "video": "dry1", "g": 1505, "i": 3010, "t": 301.6,
//!         "past_valid": true, "dt_past": 0.6,
//!         "world": {"schema": "stw1", "session": 7, "video": "dry1", "g": 1505, "t": 301.6,
//!             "motion": "static", "logF": 0.0, "hands": {}, "stats": {},
//!             "events": [{"type": "lift", "id": 643, "side": "right"}], "stones": [{"id": 643}]}}),
//! ]);
//! let factory = fake.client();
//! let mut events = factory.interpreter().events();
//! let first = events.next().await.unwrap()?;
//! assert_eq!(first.describe(), "5:01.6 Right hand lifts stone 643");
//! # Ok(()) }
//! ```
//!
//! A live camera ([`FakeFactory::with_live`], [`LIVE_CAMERA`]) connects, warms up and goes live
//! on the fake's clock, which the fake's clients share, as a camera of build live3 does:
//!
//! ```
//! use std::time::Duration;
//! use pdigger_factory_sdk::testing::{FakeFactory, LIVE_CAMERA};
//!
//! # #[tokio::main(flavor = "current_thread")] async fn main() -> pdigger_factory_sdk::Result<()> {
//! # let recording = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/recordings/dry1-0000-0130.ndjson.gz");
//! let fake = FakeFactory::from_recording(recording)?.with_live();
//! let factory = fake.client();
//! factory.session().watch(LIVE_CAMERA).await?; // connecting
//! let state = factory.session().until_live(Duration::from_secs(30)).await?; // warming (3 s of frames), live
//! assert_eq!(state.live.map(|l| l.lag), Some(pdigger_factory_sdk::testing::LIVE_LAG_S));
//! # Ok(()) }
//! ```

mod fake;
mod server;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use futures::future::BoxFuture;
use serde::Serialize;

pub use self::server::FakeServer;
use crate::client::{ClientBuilder, Clock, FactoryClient, SeededRandom};
use crate::error::Result;
use crate::models::State;
use crate::transport::{Request, Response, Transport};

/// The id of the fake's live camera ([`FakeFactory::with_live`]).
pub const LIVE_CAMERA: &str = "cam-live";

/// Seconds the fake's camera takes to send its first frame after a select (`connecting`).
pub const CONNECT_S: f64 = 0.5;

/// Seconds the fake's live camera shows its frames behind their arrival (`state.live.lag`).
pub const LIVE_LAG_S: f64 = 0.25;

/// How many camera pictures the fake keeps, as the unit's worker does: those of the newest
/// released frames of the camera session; an older frame that is still retained has none (404
/// `not_retained`).
pub const KEEP_PICTURES: usize = 64;

fn read_error(path: &Path, e: std::io::Error) -> crate::Error {
    crate::Error::new(crate::ErrorCode::Decode, format!("cannot read {}: {e}", path.display())).with_source(e)
}

/// A wall clock that moves only when told: [`ManualClock::set`], [`ManualClock::advance`], or a
/// wait on it ([`Clock::sleep`], what [`crate::Session::until_live`] waits with), which moves it on
/// by the wait at once.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<f64>,
}

impl ManualClock {
    /// A clock at `now` (epoch seconds).
    pub fn new(now: f64) -> Arc<ManualClock> {
        Arc::new(ManualClock { now: Mutex::new(now) })
    }

    /// Sets the time.
    pub fn set(&self, now: f64) {
        *self.lock() = now;
    }

    /// Moves the time on by `seconds`.
    pub fn advance(&self, seconds: f64) {
        *self.lock() += seconds;
    }

    fn lock(&self) -> MutexGuard<'_, f64> {
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Clock for ManualClock {
    fn now(&self) -> f64 {
        *self.lock()
    }

    /// Moves the clock on by `duration` at once (and yields to the runtime once).
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        self.advance(duration.as_secs_f64());
        Box::pin(tokio::task::yield_now())
    }
}

/// A reply of the fake: status, headers and body.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// Status.
    pub status: u16,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Bytes,
}

impl Reply {
    /// A reply with this content type and body.
    pub fn new(status: u16, content_type: &str, body: impl Into<Bytes>) -> Reply {
        Reply { status, headers: vec![("Content-Type".to_string(), content_type.to_string())], body: body.into() }
    }

    /// A JSON reply.
    pub fn json(status: u16, value: &impl Serialize) -> Reply {
        Reply::new(status, "application/json; charset=utf-8", serde_json::to_vec(value).unwrap_or_default())
    }

    /// An API error reply: `{"error": message, "code": code, "field": field}`.
    pub fn error(status: u16, code: &str, message: &str, field: Option<&str>) -> Reply {
        let mut body = serde_json::json!({"error": message, "code": code});
        if let Some(field) = field {
            body["field"] = field.into();
        }
        Reply::json(status, &body)
    }

    /// Adds a header.
    pub fn header(mut self, name: &str, value: &str) -> Reply {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// The value of a header (case-insensitive name).
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// A failure injected with [`FakeFactory::fail`].
#[derive(Debug, Clone, PartialEq)]
pub struct Fault {
    reply: Option<Reply>,
    times: u32,
}

impl Fault {
    /// An API error reply `{"error": "injected by FakeFactory", "code": code}`.
    pub fn status(status: u16, code: &str) -> Fault {
        Fault::reply(Reply::error(status, code, "injected by FakeFactory", None))
    }

    /// This reply.
    pub fn reply(reply: Reply) -> Fault {
        Fault { reply: Some(reply), times: 1 }
    }

    /// No reply at all: the request fails as a broken connection does (`transport`).
    pub fn broken() -> Fault {
        Fault { reply: None, times: 1 }
    }

    /// For the next `times` requests (default 1).
    pub fn times(mut self, times: u32) -> Fault {
        self.times = times;
        self
    }

    /// Adds `retry_ms` to the error body.
    pub fn retry_ms(mut self, ms: u64) -> Fault {
        if let Some(reply) = &mut self.reply {
            let mut body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap_or_default();
            body["retry_ms"] = ms.into();
            reply.body = Bytes::from(body.to_string());
        }
        self
    }

    /// Adds a `Retry-After` header.
    pub fn retry_after(mut self, value: &str) -> Fault {
        self.reply = self.reply.map(|r| r.header("Retry-After", value));
        self
    }
}

type DelayFn = Arc<dyn Fn(&str, usize) -> Option<Duration> + Send + Sync>;

/// A fake pDigger Factory server: answers the v2 API in process, as the 20.20 unit (build live3)
/// does.
///
/// - `GET`s are synthesised from a recording (the catalog from its header's `source`, else from
///   its first frame's video as the unit lists a recording; the session state from its first clock
///   line, where the fake's clock starts too, then from each later one that a stream or the long
///   poll passes, until a control takes over; frames, world, events and stone tracks from its
///   frames) and otherwise answered from recorded replies (the `fixtures/api` layout: `INDEX.json`
///   plus files; keys are the `operationId`, or `operationId-variant`). Without a world the
///   session's world is 503 `world_unavailable` (`retry_ms` 1000), a stone 404 `unknown_stone`.
/// - `frames/stream` (SSE and NDJSON) and `raw/stream` serve the session's frames with the
///   recording's clock and worker lines as side events, honouring `layers`, `after`, `Last-Event-ID`
///   and `limit`. Without `after` a stream starts at the first frame of the current program, and it
///   follows the program while it is open: a select or a seek starts it again at the new program's
///   first frame, a change of the state is a `clock` event, and so is each phase of a camera, whose
///   frames come when it goes live. A stream ends when nothing more is due. The long poll answers at
///   once with the next frames, up to the recording's next clock line (the next reply carries it),
///   after the camera's phases due within its `wait`.
/// - Controls are checked in the unit's order and with its messages (the body, `dry_run`, which
///   is `0` or `1`, then the video, a camera, the time: `"time": null` is a time of the wrong
///   type) and applied: a select or a seek starts a new fake session that replays the recording's
///   frames, its id one higher than the newest so far; a seek naming a recording is applied while
///   a camera is shown.
/// - [`FakeFactory::with_live`] offers a live camera, [`LIVE_CAMERA`], made from the recording:
///   listed after the recording and shown by the session at the start, already live. It sends the
///   recording's frames as a camera session does: `g` from 0 (`i` the same, and the raw packet's `g`,
///   `i`, `t` and `cursor` the frame's), `t` their arrival since the session started (from
///   [`CONNECT_S`]), the recording's sessions end to end. A select of it is `connecting` for
///   [`CONNECT_S`], then `warming` while the interpreter runs on its first `hold` (3) seconds of
///   frames, which are never released, then `live` ([`LIVE_LAG_S`] behind the camera), each phase a
///   new `revision`. The phases follow the fake's clock while the session plays; nothing runs while
///   it is paused (a pause keeps the phase). A play goes back to `connecting` in its own reply and
///   plans the phases again from then (only the phase changes: the `lag` stays, 0 until the camera
///   was live); on a camera that was live it goes on at the live edge (`now - since - lag`), and the
///   camera's next frames come after a break (`g` jumps by 5) and after the frames it sent already,
///   with a warm-up that is never released. On the fake's own clock a stream that waits for the next
///   phase moves the clock on to it, and so does a long poll for the phases within its `wait` (one
///   with nothing new for a client of the current revision, to the end of its `wait`); a linked clock
///   ([`FakeFactory::with_linked_clock`]) is never moved. The newest [`KEEP_PICTURES`] released
///   frames have a picture (`frames/{g}/image.jpg`, a small JPEG); an older frame's is 404
///   `not_retained`, without a `retained` window. The camera has no media (404), a seek in it is
///   refused (409 `not_seekable`, field `video`) and so is a select with any time, `null` too (400,
///   field `time`). A recording selected after the camera plays at 0.8.
/// - Unlike the unit, the fake interprets whether anyone watches or not, its camera never stops (a
///   session goes back to `connecting` only on a play), a stream ends when nothing more is due
///   (the unit's stays open), its viewers are the open streams without `observe=1` (not the long
///   polls with a `cid`), and over [`FakeFactory::serve`] a reply with a body has no
///   `Content-Length` (it is chunked).
/// - Faults: an error reply or a broken connection for the next requests of an operation, a
///   stream that breaks after some events, injected side events, a delay hook, a late close.
#[derive(Clone)]
pub struct FakeFactory {
    inner: Arc<Mutex<fake::Fake>>,
    clock: Arc<ManualClock>,
}

impl std::fmt::Debug for FakeFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeFactory").finish_non_exhaustive()
    }
}

impl Default for FakeFactory {
    fn default() -> Self {
        FakeFactory::new()
    }
}

/// The epoch second the fake's clock starts at without a recording's clock line.
pub const FAKE_EPOCH: f64 = 1_794_958_239.0;

impl FakeFactory {
    /// The base URL of [`FakeFactory::client`] (never resolved: requests stay in process).
    pub const URL: &'static str = "http://fake.factory.test";

    /// A fake with no recording and no replies.
    pub fn new() -> FakeFactory {
        FakeFactory::from_values(Vec::new())
    }

    /// A fake serving the recording at `path` (`.ndjson` or `.ndjson.gz`).
    pub fn from_recording(path: impl AsRef<Path>) -> Result<FakeFactory> {
        FakeFactory::from_recording_path(path.as_ref())
    }

    fn from_recording_path(path: &Path) -> Result<FakeFactory> {
        let bytes = std::fs::read(path).map_err(|e| read_error(path, e))?;
        FakeFactory::from_recording_bytes(&bytes)
    }

    /// A fake serving these recording bytes (gzip or not).
    pub fn from_recording_bytes(bytes: &[u8]) -> Result<FakeFactory> {
        Ok(FakeFactory::from_values(fake::read_lines(bytes)?))
    }

    /// A fake serving these recording lines (a header, frames, side lines). Its clock starts at
    /// the `server_now` of the recording's first clock line (else at [`FAKE_EPOCH`]).
    pub fn from_values(lines: Vec<serde_json::Value>) -> FakeFactory {
        let fake = fake::Fake::new(lines, FAKE_EPOCH);
        let clock = ManualClock::new(fake.started);
        FakeFactory { inner: Arc::new(Mutex::new(fake)), clock }
    }

    /// Adds the recorded replies of a `fixtures/api` directory.
    pub fn with_replies(self, dir: impl AsRef<Path>) -> Result<FakeFactory> {
        let replies = fake::read_replies(dir.as_ref())?;
        self.lock().add_replies(replies, false);
        Ok(self)
    }

    /// Adds one reply under `key` (`getModel`, `getFrame-404`, ...). A `listVideos` given so is
    /// the catalog the fake sends, as it is.
    pub fn with_reply(self, key: &str, reply: Reply) -> FakeFactory {
        self.lock().add_replies(vec![(key.to_string(), reply)], true);
        self
    }

    /// Serves `bytes` at the URL path `path` (any host) in chunks of `chunk` bytes, e.g. a
    /// recording for [`crate::ReplayStream::open_url_with`]. Faults and drops apply to it under
    /// the operation name `getFile`.
    pub fn with_file(self, path: &str, bytes: impl Into<Bytes>, chunk: usize) -> FakeFactory {
        self.lock().files.insert(path.to_string(), (bytes.into(), chunk));
        self
    }

    /// Offers the live camera [`LIVE_CAMERA`], listed after the recording (`default_video` stays
    /// the recording), and shows it at the start, already live: the camera sends the recording's
    /// frames as one session (`g` from 0, `i` the same, `t` their arrival since the session
    /// started, from [`CONNECT_S`]). Offering it again changes nothing.
    pub fn with_live(self) -> FakeFactory {
        let now = self.clock.now();
        self.lock().offer_camera(now);
        self
    }

    /// Links the fake's clock to its owner: streams and long polls never move it, only the
    /// client's sleeps or [`ManualClock::set`] do (from a real-time loop, as in
    /// `examples/fake_server.rs`). A stream waiting for a camera's next phase then looks again
    /// every 10 ms (tokio time) until the clock has passed it.
    pub fn with_linked_clock(self) -> FakeFactory {
        self.lock().linked = true;
        self
    }

    /// The boot id of event ids (`<boot>-<cursor>`; default `fakeb00t01`).
    pub fn with_boot(self, boot: &str) -> FakeFactory {
        self.lock().boot = boot.to_string();
        self
    }

    fn lock(&self) -> MutexGuard<'_, fake::Fake> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The fake's wall clock (shared with [`FakeFactory::client`]).
    pub fn clock(&self) -> Arc<ManualClock> {
        self.clock.clone()
    }

    /// A builder of a client wired to this fake: its transport, a seeded random source and the
    /// fake's clock.
    pub fn client_builder(&self) -> ClientBuilder {
        FactoryClient::builder(FakeFactory::URL)
            .transport(Arc::new(self.clone()))
            .random(SeededRandom::new(0))
            .clock(self.clock.clone())
    }

    /// A client wired to this fake.
    pub fn client(&self) -> FactoryClient {
        self.client_builder().build().expect("the fake's URL is valid")
    }

    /// The next requests of `operation` (an `operationId`, e.g. `getFrame`) fail with `fault`.
    pub fn fail(&self, operation: &str, fault: Fault) {
        self.lock().faults.entry(operation.to_string()).or_default().push_back(fault);
    }

    /// The next stream of `operation` (`streamFrames`, `rawStream`) breaks after `events`
    /// events, as a dropped connection does.
    pub fn drop_after(&self, operation: &str, events: usize) {
        self.lock().drops.entry(operation.to_string()).or_default().push_back(events);
    }

    /// The next stream of `operation` sends the side event `event` (`gap`, `reset`, `clock`,
    /// `worker`) with `data` after `after` events.
    pub fn inject(&self, operation: &str, event: &str, data: serde_json::Value, after: usize) {
        self.lock().injections.entry(operation.to_string()).or_default().push_back((event.to_string(), data, after));
    }

    /// A hook called before every streamed event (and every chunk of a file) with the operation
    /// and the event's index; the stream waits the duration it returns (tokio time: pause it in
    /// tests).
    pub fn set_delay(&self, delay: impl Fn(&str, usize) -> Option<Duration> + Send + Sync + 'static) {
        self.lock().delay = Some(Arc::new(delay));
    }

    /// Streams (`streamFrames`, `rawStream`) close this long after their last event, as the
    /// 20.20 unit closes a stream about 10 s after its `limit` or `max_s`.
    pub fn set_close_delay(&self, wait: Duration) {
        self.lock().close_after = Some(wait);
    }

    /// Every request received, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.lock().requests.clone()
    }

    /// The `operationId` of every request received, in order.
    pub fn operations(&self) -> Vec<String> {
        self.lock().requests.iter().map(|r| fake::route(r.method, &fake::split_url(&r.url).0).0).collect()
    }

    /// The fake's shared state (the newest of the recording's clock lines that a stream or the long
    /// poll has passed, or what a control made of it; with the camera's phases that are due on its
    /// clock).
    pub fn state(&self) -> State {
        let now = self.clock.now();
        let mut fake = self.lock();
        fake.advance(now);
        fake.state_now(now)
    }

    /// Streams opened so far.
    pub fn streams_opened(&self) -> usize {
        self.lock().streams_opened
    }

    /// Serves this fake over HTTP/1.1 on `127.0.0.1` (a free port), for clients that need a
    /// real socket. Stops when the returned server is dropped.
    pub async fn serve(&self) -> Result<FakeServer> {
        FakeServer::start(self.clone()).await
    }
}

impl Transport for FakeFactory {
    fn send(&self, request: Request) -> BoxFuture<'static, Result<Response>> {
        let answer = self.lock().answer(request, &self.clock);
        let response = answer.into_response(&self.inner, &self.clock);
        Box::pin(async move { response })
    }
}
