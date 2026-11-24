//! The mid level: frames over SSE (or NDJSON) with automatic resume, backfill of late layers,
//! world revisions and duplicates; the long-poll equivalent; the raw packet stream (spec 2.3).
//!
//! Every stream here is endless while `resume` is on: a connection that reached its `limit` or
//! `max_s` reconnects at once with `Last-Event-ID` (the SDK counts both and closes the connection
//! itself: the 20.20 unit closes a finished stream only about 10 s later), and so does one that
//! sent something and then nothing for the client's stream idle timeout (30 s: a half-open
//! connection); a broken one reconnects after the backoff. Drop the stream to stop; that closes
//! its connection. Late layers are read back while the stream goes on. An error that ends a
//! stream comes after the frames held for backfill.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::{BoxStream, FuturesUnordered};
use tokio::time::Instant;

use crate::api::{FrameQuery, PollQuery, StreamQuery};
use crate::client::FactoryClient;
use crate::error::{Error, Result};
use crate::models::{
    ClockEvent, Frame, GapEvent, Layer, LiveReply, Precision, ResetEvent, Sip1Packet, StreamFormat, WorkerStatus,
};
use crate::shape::{self, decode_event};
use crate::sse::{EventStream, SseEvent};

/// A frame of a stream with what the stream knows about it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameItem {
    /// The frame.
    pub frame: Frame,
    /// A higher `world.rev` of a frame already delivered (a paused frame refined).
    pub is_revision: bool,
    /// Its late layers were read back with `getFrame` and merged in.
    pub is_backfill: bool,
    /// The SSE event id (`<boot>-<cursor>`).
    pub event_id: Option<String>,
}

/// One item of a stream: a frame (or a raw packet) or a side event.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// A frame (`frames`, `poll`).
    Frame(Box<FrameItem>),
    /// A raw packet (`raw_packets`).
    Packet(Box<Sip1Packet>),
    /// The shared clock changed (`clock`), or was read after a connect: the state and the
    /// server's time.
    Clock(ClockEvent),
    /// The worker's status (`worker`).
    Worker(WorkerStatus),
    /// The server skipped this many frames for a slow consumer (`gap`).
    Gap(i64),
    /// The server restarted (`reset`); the stream continues from the newest.
    Reset(ResetEvent),
}

/// Options of [`Streams::frames`] and [`Streams::raw_packets`].
#[derive(Debug, Clone, PartialEq)]
pub struct FramesOptions {
    /// Layers (empty: the server's default `overlay,scene,cards,world`; not used by raw packets).
    pub layers: Vec<Layer>,
    /// Start after this cursor (default: the newest).
    pub after: Option<i64>,
    /// Start after this SSE event id (`<boot>-<cursor>`, a [`FrameItem::event_id`]) instead of
    /// `after`, as a reconnect does (SSE only).
    pub last_event_id: Option<String>,
    /// `true` does not count as a viewer; `None` takes the client's default.
    pub observe: Option<bool>,
    /// Number precision (default: the server's, `rounded`).
    pub precision: Option<Precision>,
    /// Reconnect when a connection ends (default on).
    pub resume: bool,
    /// Hold frames with late layers and read them back (default off; the interpreter's on).
    pub backfill: bool,
    /// After every connect, read the shared state (`GET /session`: one request, at most 1 s)
    /// and pass it on as a [`StreamItem::Clock`] before the connection's frames (default off;
    /// the interpreter's on): the unit sends no `clock` event on connect, and a reconnect may
    /// cross a session change.
    pub clock_on_connect: bool,
    /// SSE (default) or NDJSON.
    pub format: StreamFormat,
    /// Frames per connection (the server's `limit`; the SDK closes the connection when it is
    /// reached).
    pub limit: Option<u32>,
    /// Seconds per connection (the server's `max_s`; the SDK closes the connection when they
    /// have passed).
    pub max_s: Option<u32>,
}

impl Default for FramesOptions {
    fn default() -> Self {
        FramesOptions {
            layers: Vec::new(),
            after: None,
            last_event_id: None,
            observe: None,
            precision: None,
            resume: true,
            backfill: false,
            clock_on_connect: false,
            format: StreamFormat::Sse,
            limit: None,
            max_s: None,
        }
    }
}

/// Options of [`Streams::poll`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PollOptions {
    /// Layers.
    pub layers: Vec<Layer>,
    /// Start after this cursor (0: the newest frames at once).
    pub cursor: Option<i64>,
    /// `true` does not count as a viewer (no `cid` is sent); `None` takes the client's default.
    pub observe: Option<bool>,
    /// The per-tab id (`^[A-Za-z0-9_-]{8,40}$`) that counts this client as a viewer (default:
    /// an id of the poll's own, unless observing).
    pub cid: Option<String>,
    /// Frames per reply (1..16).
    pub limit: Option<u32>,
    /// Longest wait per call in ms (0..1000).
    pub wait_ms: Option<u32>,
    /// Number precision.
    pub precision: Option<Precision>,
    /// Hold frames with late layers and read them back.
    pub backfill: bool,
}

/// The mid level. Get it with [`FactoryClient::streams`].
#[derive(Debug, Clone)]
pub struct Streams {
    client: FactoryClient,
}

/// When a frame with late layers is read back: 1.5 s after it arrived, then 1 s apart, 3 times
/// at most. Each read is one request (no retries) bounded by 1 s, and runs while the stream goes
/// on, so a late world holds the frames behind it for about 4.5 s at the most.
const BACKFILL_FIRST: Duration = Duration::from_millis(1500);
const BACKFILL_EVERY: Duration = Duration::from_secs(1);
const BACKFILL_READ: Duration = Duration::from_secs(1);
const BACKFILL_TRIES: u32 = 3;
/// The longest the state read after a connect may take.
const CLOCK_READ: Duration = Duration::from_secs(1);
const KEEP_G: i64 = 2048;

/// Where a held frame's backfill stands.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fill {
    /// Nothing (more) to read: the frame goes out when it reaches the head.
    Done,
    /// The next read is due at this time.
    Due(Instant),
    /// A read is running.
    Reading,
}

#[derive(Debug)]
struct Held {
    id: u64,
    item: FrameItem,
    arrived: Instant,
    tries: u32,
    fill: Fill,
}

/// A running backfill read: the held frame's id and the reply (`None`: it took too long).
type Read = BoxFuture<'static, (u64, Option<Result<Frame>>)>;

/// Order, revisions, duplicates and backfill of frames, shared by the SSE and long-poll loops.
#[derive(Debug)]
struct Gate {
    backfill: bool,
    precision: Option<Precision>,
    held: VecDeque<Held>,
    reads: FuturesUnordered<Read>,
    next_id: u64,
    delivered: HashMap<(i64, i64), i64>,
    newest: (i64, i64),
    prune_at: usize,
    ready: VecDeque<Result<StreamItem>>,
    /// The error that ended the stream, given out once the held frames are out.
    end: Option<Error>,
}

impl Gate {
    fn new(backfill: bool, precision: Option<Precision>) -> Gate {
        Gate {
            backfill,
            precision,
            held: VecDeque::new(),
            reads: FuturesUnordered::new(),
            next_id: 0,
            delivered: HashMap::new(),
            newest: (0, 0),
            prune_at: 4096,
            ready: VecDeque::new(),
            end: None,
        }
    }

    fn side(&mut self, item: StreamItem) {
        self.ready.push_back(Ok(item));
    }

    /// A frame of the stream: out at once, or held (it has late layers to read back, or frames
    /// before it are held).
    fn admit(&mut self, item: FrameItem) {
        let late = self.backfill && !item.frame.missing.is_empty();
        if !late && self.held.is_empty() {
            self.deliver(item);
            return;
        }
        let arrived = Instant::now();
        let fill = if late { Fill::Due(arrived + BACKFILL_FIRST) } else { Fill::Done };
        self.next_id += 1;
        self.held.push_back(Held { id: self.next_id, item, arrived, tries: 0, fill });
    }

    fn deliver(&mut self, mut item: FrameItem) {
        let key = (item.frame.session, item.frame.g);
        let rev = item.frame.world.as_ref().map_or(-1, |w| w.revision());
        match self.delivered.get(&key) {
            Some(&seen) if rev <= seen => return,
            Some(_) => item.is_revision = true,
            None => {}
        }
        self.delivered.insert(key, rev);
        self.newest = key;
        if self.delivered.len() > self.prune_at {
            let (session, g) = self.newest;
            self.delivered.retain(|&(s, fg), _| s == session && fg + KEEP_G > g);
        }
        self.ready.push_back(Ok(StreamItem::Frame(Box::new(item))));
    }

    /// Delivers the complete frames at the head.
    fn release(&mut self) {
        while let Some(held) = self.held.pop_front() {
            if held.fill != Fill::Done {
                self.held.push_front(held);
                return;
            }
            self.deliver(held.item);
        }
    }

    /// Releases every held frame as it is and drops their reads (a reset).
    fn flush(&mut self) {
        self.reads = FuturesUnordered::new();
        while let Some(held) = self.held.pop_front() {
            self.deliver(held.item);
        }
    }

    /// When the next read is due.
    fn next_due(&self) -> Option<Instant> {
        self.held
            .iter()
            .filter_map(|h| match h.fill {
                Fill::Due(at) => Some(at),
                _ => None,
            })
            .min()
    }

    /// Starts the reads that are due.
    fn start_reads(&mut self, client: &FactoryClient) {
        let now = Instant::now();
        for held in &mut self.held {
            if !matches!(held.fill, Fill::Due(at) if at <= now) {
                continue;
            }
            held.fill = Fill::Reading;
            let frame = &held.item.frame;
            let query = FrameQuery {
                layers: frame.missing.clone(),
                session: Some(frame.session),
                precision: self.precision.clone(),
            };
            let (api, id, g) = (client.api(), held.id, frame.g);
            self.reads.push(Box::pin(async move {
                (id, tokio::time::timeout(BACKFILL_READ, api.get_frame_once(g, &query)).await.ok())
            }));
        }
    }

    /// A read finished: merges in what it brought, sets the next read (or none), and releases
    /// the complete frames at the head.
    fn finish_read(&mut self, id: u64, reply: Option<Result<Frame>>) {
        if let Some(held) = self.held.iter_mut().find(|h| h.id == id) {
            held.tries += 1;
            let done = match reply {
                Some(Ok(fresh)) => {
                    held.item.is_backfill |= !held.item.frame.merge_missing(fresh).is_empty();
                    held.item.frame.missing.is_empty()
                }
                Some(Err(error)) => error.is_not_retained() || error.is_session_gone(),
                None => false,
            };
            held.fill = if done || held.tries >= BACKFILL_TRIES {
                Fill::Done
            } else {
                Fill::Due(held.arrived + BACKFILL_FIRST + BACKFILL_EVERY * held.tries)
            };
        }
        self.release();
    }

    /// Waits for `work` while the reads that come due start and the ones that finish are merged
    /// in; wakes at `until` too. `Some` with the output of `work` when that came first.
    async fn wait<T>(
        &mut self,
        client: &FactoryClient,
        work: impl Future<Output = T>,
        until: Option<Instant>,
    ) -> Option<T> {
        self.start_reads(client);
        let at = earliest(self.next_due(), until);
        tokio::select! {
            // An event that is already there goes first (deterministic order).
            biased;
            out = work => Some(out),
            Some((id, reply)) = self.reads.next(), if !self.reads.is_empty() => {
                self.finish_read(id, reply);
                None
            }
            () = tokio::time::sleep_until(at.unwrap_or_else(Instant::now)), if at.is_some() => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Frames,
    Raw,
}

/// The SSE/NDJSON loop of `frames` and `raw_packets`.
struct Pump {
    client: FactoryClient,
    kind: Kind,
    options: FramesOptions,
    conn: Option<EventStream>,
    last_event_id: Option<String>,
    max_cursor: Option<i64>,
    attempt: u32,
    got_events: bool,
    reconnect_at: Option<Instant>,
    /// Frames (or packets) of the open connection, when its `max_s` is up, and when it counts as
    /// silent.
    conn_frames: u32,
    conn_until: Option<Instant>,
    idle_until: Option<Instant>,
    finished: bool,
    gate: Gate,
}

/// The earlier of two optional instants.
fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl Pump {
    fn new(client: FactoryClient, kind: Kind, options: FramesOptions) -> Pump {
        let gate = Gate::new(options.backfill && kind == Kind::Frames, options.precision.clone());
        Pump {
            client,
            kind,
            last_event_id: options.last_event_id.clone(),
            options,
            conn: None,
            max_cursor: None,
            attempt: 0,
            got_events: false,
            reconnect_at: None,
            conn_frames: 0,
            conn_until: None,
            idle_until: None,
            finished: false,
            gate,
        }
    }

    fn query(&self) -> StreamQuery {
        let mut query = StreamQuery {
            layers: self.options.layers.clone(),
            format: self.options.format.clone(),
            after: self.options.after,
            limit: self.options.limit,
            max_s: self.options.max_s,
            observe: self.options.observe.unwrap_or(self.client.observe_default()),
            precision: self.options.precision.clone(),
            last_event_id: None,
        };
        if self.options.format == StreamFormat::Ndjson {
            query.after = self.max_cursor.or(self.options.after);
        } else if let Some(id) = &self.last_event_id {
            query.last_event_id = Some(id.clone());
            query.after = None;
        }
        query
    }

    async fn connect(&mut self) {
        let query = self.query();
        let api = self.client.api();
        let opened = match self.kind {
            Kind::Frames => api.stream_frames(&query).await,
            Kind::Raw => api.raw_stream(&query).await,
        };
        match opened {
            Ok(conn) => {
                self.conn = Some(conn);
                self.got_events = false;
                self.conn_frames = 0;
                self.idle_until = None;
                self.conn_until = self.options.max_s.map(|s| Instant::now() + Duration::from_secs(u64::from(s)));
                if self.options.clock_on_connect {
                    self.read_clock().await;
                }
            }
            Err(error) => self.lost(error),
        }
    }

    /// The shared state after a connect, passed on as a clock event (nothing when the read fails
    /// or takes longer than [`CLOCK_READ`]).
    async fn read_clock(&mut self) {
        if let Ok(Ok(reply)) = tokio::time::timeout(CLOCK_READ, self.client.api().get_session_once()).await {
            self.gate.side(StreamItem::Clock(ClockEvent { state: reply.state, server_now: Some(reply.server_now) }));
        }
    }

    /// A connection could not open or broke.
    fn lost(&mut self, error: Error) {
        self.conn = None;
        if self.options.resume && (error.is_retryable() || self.got_events) {
            if self.got_events {
                self.attempt = 0;
            }
            self.reconnect_at = Some(Instant::now() + self.client.retry_delay(self.attempt, &error));
            self.attempt += 1;
            self.got_events = false;
        } else {
            self.finished = true;
            self.gate.end = Some(error);
        }
    }

    /// A connection ended: by the server, or by the SDK at its `limit`, its `max_s` or the idle
    /// timeout.
    fn ended(&mut self) {
        self.conn = None;
        if !self.options.resume {
            self.finished = true;
        } else if self.got_events {
            self.attempt = 0;
        } else {
            let cap = self.client.backoff().cap(self.attempt);
            self.reconnect_at = Some(Instant::now() + cap);
            self.attempt += 1;
        }
    }

    fn on_event(&mut self, mut event: SseEvent) {
        self.got_events = true;
        self.idle_until = self.client.stream_idle_timeout().map(|idle| Instant::now() + idle);
        if event.id.is_some() {
            self.last_event_id = event.id.clone();
        }
        let kind = std::mem::take(&mut event.event);
        let item = match kind.as_str() {
            "frame" => self.on_frame(event),
            "clock" => decode_event(&event.data, "a clock event", shape::CLOCK).map(StreamItem::Clock).map(Some),
            "worker" => decode_event(&event.data, "a worker event", shape::WORKER).map(StreamItem::Worker).map(Some),
            "gap" => {
                decode_event::<GapEvent>(&event.data, "a gap event", shape::GAP).map(|g| Some(StreamItem::Gap(g.gap)))
            }
            "reset" => self.on_reset(&event.data),
            _ => return,
        };
        match item {
            Ok(Some(item)) => self.gate.side(item),
            Ok(None) => {}
            Err(error) => self.gate.ready.push_back(Err(error)),
        }
    }

    /// A frame (or packet) event: frames go through the gate, packets straight out. It counts
    /// toward the connection's `limit` even when it does not decode.
    fn on_frame(&mut self, event: SseEvent) -> Result<Option<StreamItem>> {
        self.count_frame();
        if self.kind == Kind::Raw {
            let packet: Sip1Packet = decode_event(&event.data, "a packet event", shape::PACKET)?;
            self.track(packet.cursor);
            return Ok(Some(StreamItem::Packet(Box::new(packet))));
        }
        let frame: Frame = decode_event(&event.data, "a frame event", shape::FRAME)?;
        self.track(Some(frame.cursor));
        self.gate.admit(FrameItem { frame, is_revision: false, is_backfill: false, event_id: event.id });
        Ok(None)
    }

    /// A `reset` (the unit restarted): the held frames go out as they are, and a reconnect goes
    /// on in the new boot after the reset's cursor.
    fn on_reset(&mut self, data: &str) -> Result<Option<StreamItem>> {
        // a reset that does not decode is skipped (rule 9): it releases no held frame either
        let reset: ResetEvent = decode_event(data, "a reset event", shape::RESET)?;
        self.gate.flush();
        self.last_event_id = Some(format!("{}-{}", reset.boot, reset.cursor));
        self.max_cursor = Some(reset.cursor);
        Ok(Some(StreamItem::Reset(reset)))
    }

    fn track(&mut self, cursor: Option<i64>) {
        self.max_cursor = self.max_cursor.max(cursor);
    }

    /// Counts a frame of the connection and closes it at its `limit`.
    fn count_frame(&mut self) {
        self.conn_frames += 1;
        if self.options.limit.is_some_and(|limit| self.conn_frames >= limit) {
            self.ended();
        }
    }

    async fn next(&mut self) -> Option<Result<StreamItem>> {
        loop {
            if let Some(item) = self.gate.ready.pop_front() {
                return Some(item);
            }
            if self.finished {
                if self.gate.held.is_empty() {
                    return self.gate.end.take().map(Err);
                }
                self.gate.wait(&self.client, std::future::pending::<()>(), None).await;
                continue;
            }
            let Some(conn) = self.conn.as_mut() else {
                match self.reconnect_at.filter(|at| *at > Instant::now()) {
                    Some(at) => {
                        self.gate.wait(&self.client, std::future::pending::<()>(), Some(at)).await;
                    }
                    None => {
                        self.reconnect_at = None;
                        self.connect().await;
                    }
                }
                continue;
            };
            let until = earliest(self.conn_until, self.idle_until);
            match self.gate.wait(&self.client, conn.next(), until).await {
                // A read finished or came due, or the connection's time is up.
                None => {
                    if until.is_some_and(|until| until <= Instant::now()) {
                        self.ended();
                    }
                }
                Some(Some(Ok(event))) => self.on_event(event),
                Some(Some(Err(error))) => self.lost(error),
                Some(None) => self.ended(),
            }
        }
    }
}

/// The long-poll loop of `poll`.
struct Poller {
    client: FactoryClient,
    options: PollOptions,
    cid: Option<String>,
    cursor: i64,
    rev: i64,
    attempt: u32,
    worker: Option<WorkerStatus>,
    wait_until: Option<Instant>,
    /// The call in flight (held frames are read back while it waits).
    call: Option<BoxFuture<'static, Result<LiveReply>>>,
    finished: bool,
    gate: Gate,
}

impl Poller {
    async fn next(&mut self) -> Option<Result<StreamItem>> {
        loop {
            if let Some(item) = self.gate.ready.pop_front() {
                return Some(item);
            }
            if self.finished {
                if self.gate.held.is_empty() {
                    return self.gate.end.take().map(Err);
                }
                self.gate.wait(&self.client, std::future::pending::<()>(), None).await;
                continue;
            }
            if self.call.is_none() {
                if let Some(at) = self.wait_until.filter(|at| *at > Instant::now()) {
                    self.gate.wait(&self.client, std::future::pending::<()>(), Some(at)).await;
                    continue;
                }
                self.wait_until = None;
                self.call = Some(self.start_call());
            }
            let Some(call) = self.call.as_mut() else { continue };
            if let Some(reply) = self.gate.wait(&self.client, call, None).await {
                self.call = None;
                self.on_reply(reply);
            }
        }
    }

    fn start_call(&self) -> BoxFuture<'static, Result<LiveReply>> {
        let query = PollQuery {
            cursor: Some(self.cursor),
            rev: Some(self.rev),
            cid: self.cid.clone(),
            layers: self.options.layers.clone(),
            limit: self.options.limit,
            wait_ms: self.options.wait_ms,
            precision: self.options.precision.clone(),
        };
        let api = self.client.api();
        Box::pin(async move { api.poll_frames(&query).await })
    }

    fn on_reply(&mut self, reply: Result<LiveReply>) {
        match reply {
            Ok(reply) => {
                self.attempt = 0;
                let changed = reply.state.revision != self.rev;
                if changed {
                    self.gate.side(StreamItem::Clock(ClockEvent {
                        state: reply.state.clone(),
                        server_now: Some(reply.server_now),
                    }));
                }
                if self.worker.as_ref() != Some(&reply.worker) {
                    self.worker = Some(reply.worker.clone());
                    self.gate.side(StreamItem::Worker(reply.worker));
                }
                // A reply that moved neither the cursor nor the state waits a little before the
                // next call (a server answers at once only when something changed).
                let stalled = reply.cursor == self.cursor && !changed;
                for frame in reply.frames {
                    self.gate.admit(FrameItem { frame, ..FrameItem::default() });
                }
                self.cursor = reply.cursor;
                self.rev = reply.state.revision;
                if stalled {
                    self.wait_until = Some(Instant::now() + self.client.backoff().base);
                }
            }
            Err(error) if error.is_retryable() => {
                self.wait_until = Some(Instant::now() + self.client.retry_delay(self.attempt, &error));
                self.attempt += 1;
            }
            Err(error) => {
                self.finished = true;
                self.gate.end = Some(error);
            }
        }
    }
}

impl Streams {
    pub(crate) fn new(client: FactoryClient) -> Streams {
        Streams { client }
    }

    /// Frames of the shared session (`streamFrames`), endless while `options.resume`, with
    /// [`StreamItem::Clock`], `Worker`, `Gap` and `Reset` side events.
    ///
    /// ```no_run
    /// # async fn demo(factory: pdigger_factory_sdk::FactoryClient) -> pdigger_factory_sdk::Result<()> {
    /// use futures::StreamExt;
    /// use pdigger_factory_sdk::{FramesOptions, Layer, StreamItem};
    /// let options = FramesOptions { layers: vec![Layer::World], observe: Some(true), ..FramesOptions::default() };
    /// let mut frames = factory.streams().frames(options);
    /// while let Some(item) = frames.next().await {
    ///     if let StreamItem::Frame(item) = item? {
    ///         println!("g {} rev {:?}", item.frame.g, item.frame.world.map(|w| w.revision()));
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    pub fn frames(&self, options: FramesOptions) -> BoxStream<'static, Result<StreamItem>> {
        pump(Pump::new(self.client.clone(), Kind::Frames, options))
    }

    /// Raw model packets (`rawStream`) as [`StreamItem::Packet`], with the same side events and
    /// resume rules (`layers`, `precision` and `backfill` do not apply).
    pub fn raw_packets(&self, options: FramesOptions) -> BoxStream<'static, Result<StreamItem>> {
        pump(Pump::new(self.client.clone(), Kind::Raw, options))
    }

    /// Frames by long-polling (`pollFrames`), for environments without streaming: continues
    /// from each reply's `cursor` and state revision; a changed state is a
    /// [`StreamItem::Clock`], a changed worker a [`StreamItem::Worker`]. It counts as a viewer
    /// (`cid`) unless observing.
    pub fn poll(&self, options: PollOptions) -> BoxStream<'static, Result<StreamItem>> {
        let gate = Gate::new(options.backfill, options.precision.clone());
        let cursor = options.cursor.unwrap_or(0);
        let observing = options.observe.unwrap_or(self.client.observe_default());
        let cid = if observing { None } else { Some(options.cid.clone().unwrap_or_else(|| self.client.new_cid())) };
        let poller = Poller {
            client: self.client.clone(),
            options,
            cid,
            cursor,
            rev: 0,
            attempt: 0,
            worker: None,
            wait_until: None,
            call: None,
            finished: false,
            gate,
        };
        futures::stream::unfold(poller, |mut poller| async move { poller.next().await.map(|item| (item, poller)) })
            .boxed()
    }
}

fn pump(pump: Pump) -> BoxStream<'static, Result<StreamItem>> {
    futures::stream::unfold(pump, |mut pump| async move { pump.next().await.map(|item| (item, pump)) }).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(session: i64, g: i64, rev: Option<i64>) -> FrameItem {
        let world = rev.map(|rev| crate::models::WorldLayer { rev: Some(rev), ..Default::default() });
        FrameItem { frame: Frame { session, g, world, ..Frame::default() }, ..FrameItem::default() }
    }

    fn frames(gate: &mut Gate) -> Vec<(i64, bool)> {
        gate.ready
            .drain(..)
            .map(|i| {
                let Ok(StreamItem::Frame(f)) = i else { panic!("a frame") };
                (f.frame.g, f.is_revision)
            })
            .collect()
    }

    #[test]
    fn gate_revisions_duplicates_and_pruning() {
        let mut gate = Gate::new(false, None);
        gate.prune_at = 3;
        gate.admit(item(1, 1, Some(0)));
        gate.admit(item(1, 1, Some(0)));
        gate.admit(item(1, 1, Some(2)));
        gate.admit(item(1, 2, None));
        gate.admit(item(1, 2, None));
        assert_eq!(frames(&mut gate), vec![(1, false), (1, true), (2, false)]);
        gate.admit(item(1, 3000, Some(0)));
        gate.admit(item(2, 5000, Some(0)));
        assert_eq!(gate.delivered.len(), 1);
        assert!(gate.next_due().is_none());
    }

    #[test]
    fn held_frames_wait_for_the_head_and_go_out_in_order() {
        let mut gate = Gate::new(true, None);
        let mut late = item(1, 1, None);
        late.frame.missing = vec![Layer::World];
        gate.admit(late);
        gate.admit(item(1, 2, Some(0)));
        assert!(gate.ready.is_empty() && gate.held.len() == 2);
        assert!(gate.next_due().is_some());
        // a read of a frame no longer held (dropped by a reset) changes nothing
        gate.finish_read(99, None);
        assert!(gate.ready.is_empty());
        // three answers that bring nothing use up the tries; the frames then go out as they are
        let id = gate.held[0].id;
        gate.finish_read(id, None);
        gate.finish_read(id, Some(Err(Error::new(crate::error::ErrorCode::Internal, "boom"))));
        assert!(gate.ready.is_empty());
        gate.finish_read(id, None);
        assert_eq!(frames(&mut gate), vec![(1, false), (2, false)]);
        // a read that brings the world completes the frame
        let mut late = item(1, 3, None);
        late.frame.missing = vec![Layer::World];
        gate.admit(late);
        let id = gate.held[0].id;
        gate.finish_read(id, Some(Ok(item(1, 3, Some(1)).frame)));
        let Some(Ok(StreamItem::Frame(done))) = gate.ready.pop_front() else { panic!("the frame") };
        assert!(done.is_backfill && done.frame.missing.is_empty() && gate.held.is_empty());
    }
}
