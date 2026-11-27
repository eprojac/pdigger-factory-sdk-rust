//! The high level: typed events, activities (moves and push bursts), hand states, the stone
//! inventory, trays with stone ids that hold over time, the summary and human-readable text
//! (spec 2.4 and 4).
//!
//! [`Interpreter`] follows the shared session over the frame stream; [`InterpreterCore`] is the
//! synchronous state machine it (and [`crate::Replay`]) feeds.

mod core;
mod looks;
mod summary;
pub mod text;
mod trays;

use std::collections::{BTreeMap, VecDeque};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use futures::stream::BoxStream;
use futures::{FutureExt, Stream, StreamExt};
use serde::{Deserialize, Serialize};

pub use self::core::{BURST_GAP, FrameParts, InterpreterCore, TrackPoint};
pub use self::looks::{
    LOOK_CELLS, LOOK_MIN, LOOK_SHIFT, LOOK_SQUARE, look_pairing, look_permille, look_score, usable_look,
};
pub use self::summary::{EventCounts, HandTimes, HandsTimes, MoveCounts, PushCounts, StoneCounts, Summary};
use self::text::{EventText, clock, event_sentence, fmt, hand_word, stones_text, verb};
pub use self::trays::{
    CONFIDENCE, MOVED, MOVED_CONFIDENCE, SPECKLE_CLEAN, SPECKLE_CODES, SPECKLE_KEPT, SPECKLE_NEAR, SpeckleSide,
    StoneSpeckle, TRAYS_KEPT, Tray, TrayCandidate, TrayChange, TrayChangeKind, TrayCounts, TrayEvidence, TrayFact,
    TrayFactKind, TrayOpen, TrayPoint, TrayQuestion, TrayReport, TraySpeckle, TrayStone, TrayStoneStatus,
    describe_open_txt, describe_speckle_txt, iso,
};
use crate::client::FactoryClient;
use crate::error::{Error, ErrorCode, Result};
use crate::models::{EventKind, Frame, HandStateKind, Layer, Rect, Side, State, StoneView, WorldStats};
use crate::streams::{FramesOptions, StreamItem};
use crate::wire::wire_enum;

/// A stone of the world (`id, mode, family, shape, side, outline, solid {pos, q, dims},
/// detected, uncertain, source, placed`).
pub type Stone = StoneView;

/// A moment: grid frame, media time and wall time (epoch seconds, `None` without a clock).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct TimeRef {
    /// Grid frame.
    pub g: i64,
    /// Media time, seconds.
    pub t: f64,
    /// Wall time, epoch seconds.
    pub wall: Option<f64>,
}

/// What the world says happened in a frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// The world IDs affected by a sheet action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<i64>>,
    /// The type (`lift`, `place`, ...).
    #[serde(rename = "type")]
    pub kind: EventKind,
    /// Grid frame.
    pub g: i64,
    /// Media time, seconds.
    pub t: f64,
    /// Wall time, epoch seconds.
    pub wall: Option<f64>,
    /// Session.
    pub session: i64,
    /// Source (video id).
    pub video: String,
    /// The stone.
    pub stone_id: Option<i64>,
    /// The hand.
    pub hand: Option<Side>,
    /// Why (`course`, `reseat`, `absent`, ...).
    pub why: Option<String>,
    /// The parts of a split.
    pub into: Option<Vec<i64>>,
    /// `image` for a stone found in the image.
    pub source: Option<String>,
    /// The stone in this frame's world.
    pub stone: Option<Stone>,
    /// The id of the tray stone this world id stood for on the open tray before this frame (spec
    /// 4.6); `stone_id` is the world's id, which the engine's tracker renames and reuses.
    pub tray_stone: Option<String>,
}

impl Event {
    /// The sentence without the clock: `Right hand lifts stone 2660`.
    pub fn sentence(&self) -> String {
        event_sentence(EventText {
            kind: &self.kind,
            id: self.stone_id,
            side: self.hand.as_ref(),
            why: self.why.as_deref(),
            into: self.into.as_deref(),
            ids: self.ids.as_deref(),
            source: self.source.as_deref(),
        })
    }

    /// `{clock} {sentence}`: `21:17.8 Right hand lifts stone 2660`.
    pub fn describe(&self) -> String {
        format!("{} {}", clock(self.t), self.sentence())
    }

    /// The moment of the event.
    pub fn at(&self) -> TimeRef {
        TimeRef { g: self.g, t: self.t, wall: self.wall }
    }
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

wire_enum! {
    /// How a move ended.
    MoveOutcome {
        /// Put down on the tray.
        Placed = "placed",
        /// Dropped.
        Dropped = "dropped",
        /// Taken out of view.
        CarriedAway = "carried_away",
        /// Removed by the world.
        Removed = "removed",
        /// Still open at a session change or the end.
        Interrupted = "interrupted",
    }
}

wire_enum! {
    /// The kind of an activity.
    ActivityKind {
        /// One stone taken and put somewhere.
        Move = "move",
        /// A hand pushing stones about.
        Push = "push",
    }
}

/// One stone taken and put somewhere.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Move {
    /// The stone.
    pub stone_id: Option<i64>,
    /// The hand (`both` once both took part).
    pub hand: Option<Side>,
    /// The lift (or, for a partial move, the end).
    pub start: TimeRef,
    /// The end.
    pub end: TimeRef,
    /// `end.t - start.t`, seconds.
    pub duration: f64,
    /// The stone's position at the start (scene, template units).
    pub from: Option<Vec<f64>>,
    /// The stone's position at the end, when placed or dropped.
    pub to: Option<Vec<f64>>,
    /// Distance from `from` to `to` in x, y, rounded to 3 decimals.
    pub distance: Option<f64>,
    /// Carried.
    pub carried: bool,
    /// Tipped up on its edge.
    pub tipped: bool,
    /// Joined mid-move (the start was before the stream joined).
    pub partial: bool,
    /// How it ended.
    pub outcome: MoveOutcome,
    /// Session.
    pub session: i64,
    /// Source.
    pub video: String,
}

/// A hand pushing stones about: pushes of one side at most 1 s apart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushBurst {
    /// The hand.
    pub hand: Option<Side>,
    /// The first push.
    pub start: TimeRef,
    /// The last push.
    pub end: TimeRef,
    /// `end.t - start.t`, seconds.
    pub duration: f64,
    /// The stones pushed, unique, ascending.
    pub stones: Vec<i64>,
    /// Push events.
    pub pushes: u64,
    /// Session.
    pub session: i64,
    /// Source.
    pub video: String,
}

/// Events joined into one human-scale action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Activity {
    /// A move.
    Move(Move),
    /// A push burst.
    Push(PushBurst),
}

impl Activity {
    /// `move` or `push`.
    pub fn kind(&self) -> ActivityKind {
        match self {
            Activity::Move(_) => ActivityKind::Move,
            Activity::Push(_) => ActivityKind::Push,
        }
    }

    /// The hand.
    pub fn hand(&self) -> Option<&Side> {
        match self {
            Activity::Move(m) => m.hand.as_ref(),
            Activity::Push(p) => p.hand.as_ref(),
        }
    }

    /// The start.
    pub fn start(&self) -> TimeRef {
        match self {
            Activity::Move(m) => m.start,
            Activity::Push(p) => p.start,
        }
    }

    /// The end.
    pub fn end(&self) -> TimeRef {
        match self {
            Activity::Move(m) => m.end,
            Activity::Push(p) => p.end,
        }
    }

    /// Seconds from start to end.
    pub fn duration(&self) -> f64 {
        match self {
            Activity::Move(m) => m.duration,
            Activity::Push(p) => p.duration,
        }
    }

    /// Session.
    pub fn session(&self) -> i64 {
        match self {
            Activity::Move(m) => m.session,
            Activity::Push(p) => p.session,
        }
    }

    /// Source.
    pub fn video(&self) -> &str {
        match self {
            Activity::Move(m) => &m.video,
            Activity::Push(p) => &p.video,
        }
    }

    /// The order of spec 4.3: end g, start g, move before push, stone id, hand.
    pub fn sort_key(&self) -> (i64, i64, u8, Option<i64>, &str) {
        let hand = self.hand().map_or("", crate::wire::WireEnum::as_str);
        match self {
            Activity::Move(m) => (m.end.g, m.start.g, 0, m.stone_id, hand),
            Activity::Push(p) => (p.end.g, p.start.g, 1, Some(p.stones.first().copied().unwrap_or(-1)), hand),
        }
    }

    /// The sentence of spec 4.2: `5:01.6 Right hand moves stone 643 1.09 units in 1.2 s`.
    pub fn describe(&self) -> String {
        let side = self.hand();
        let hand = hand_word(side);
        let v = |word| verb(side, word);
        let m = match self {
            Activity::Push(p) => {
                let tail = if p.pushes == 1 {
                    "(1 push)".to_string()
                } else {
                    format!("({} pushes in {} s)", p.pushes, fmt(p.duration, 1))
                };
                return format!("{} {hand} {} {} {tail}", clock(p.start.t), v("pushes"), stones_text(&p.stones));
            }
            Activity::Move(m) => m,
        };
        let (i, dur) = (text::id_text(m.stone_id), fmt(m.duration, 1));
        let sentence = match m.outcome {
            MoveOutcome::Placed if m.carried => match m.distance {
                None => format!("{hand} {} stone {i} in {dur} s", v("moves")),
                Some(d) => format!("{hand} {} stone {i} {} units in {dur} s", v("moves"), fmt(d, 2)),
            },
            MoveOutcome::Placed if m.tipped => format!("{hand} {} stone {i} and sets it down ({dur} s)", v("tips")),
            MoveOutcome::Placed => format!("{hand} {} stone {i} and sets it down ({dur} s)", v("lifts")),
            MoveOutcome::Dropped => format!("{hand} {} stone {i} after {dur} s", v("drops")),
            MoveOutcome::CarriedAway => format!("{hand} {} stone {i} away ({dur} s)", v("takes")),
            MoveOutcome::Removed => format!("{hand} {} stone {i}; it is removed after {dur} s", v("lifts")),
            _ => format!("{hand} {} still holding stone {i} ({dur} s so far)", v("is")),
        };
        if m.partial {
            format!("{} {sentence} (joined mid-move)", clock(m.end.t))
        } else {
            format!("{} {sentence}", clock(m.start.t))
        }
    }
}

impl std::fmt::Display for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

/// A move that is still open (lifted, not yet put down).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenMove {
    /// The stone.
    pub stone_id: Option<i64>,
    /// The hand.
    pub hand: Option<Side>,
    /// The lift.
    pub start: TimeRef,
    /// The stone's position at the lift.
    pub from: Option<Vec<f64>>,
    /// Carried so far.
    pub carried: bool,
    /// Tipped so far.
    pub tipped: bool,
    /// Session.
    pub session: i64,
    /// Source.
    pub video: String,
}

/// A hand in the interpreter's state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HandView {
    /// `free`, `touch`, `grasp`, `hinge` or `carry` (`None` before the first world).
    pub state: Option<HandStateKind>,
    /// The stone it holds or touches.
    pub stone: Option<i64>,
    /// The card title of the cards layer (`Holding a stone`).
    pub card: Option<String>,
}

/// Both hands.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct HandsView {
    /// Left.
    pub left: HandView,
    /// Right.
    pub right: HandView,
}

/// The interpreter's state after a frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InterpreterState {
    /// Session.
    pub session: Option<i64>,
    /// Source.
    pub video: Option<String>,
    /// The newest grid frame.
    pub g: Option<i64>,
    /// Its media time.
    pub t: Option<f64>,
    /// Its wall time (epoch seconds), from the newest clock state.
    pub wall: Option<f64>,
    /// The stones of the newest world by id (shared, cheap to clone).
    pub stones: Arc<BTreeMap<i64, Stone>>,
    /// The hands.
    pub hands: HandsView,
    /// The board (`world.draw.board`).
    pub board: Option<Rect>,
    /// The world's stats.
    pub stats: Option<WorldStats>,
    /// Moves still open, by stone id.
    pub open_moves: Vec<OpenMove>,
    /// The newest clock state.
    pub clock: Option<State>,
    /// The open tray with its tray stones (spec 4.6), `None` between trays.
    pub tray: Option<Tray>,
}

/// One frame's worth of interpretation.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    /// The frame; `None` only for the final update (`is_final`).
    pub frame: Option<Frame>,
    /// Its new events, in world order.
    pub events: Vec<Event>,
    /// The activities it completed, in the order of spec 4.3.
    pub activities: Vec<Activity>,
    /// The tray facts it made (spec 4.6; after a session change, first those of the old session's
    /// tray leaving); the final update has those of the open tray leaving.
    pub tray_facts: Vec<TrayFact>,
    /// The unit's speckle answer its world carried, read as a tray's (spec 4.6.12): at most one.
    pub speckle: Vec<TraySpeckle>,
    /// A revision of a frame already seen (a paused frame refined).
    pub is_revision: bool,
    /// Its late layers were read back by the stream (backfill).
    pub is_backfill: bool,
    /// The last update of a replay, of a stream the server ended (resume off), and
    /// [`InterpreterCore::end`]: no frame, the activities still open at the end and the facts of
    /// the open tray leaving.
    pub is_final: bool,
    /// The frame repeated one already seen (the same or a lower world.rev) and was dropped:
    /// nothing in it, not counted. The replays and [`Interpreter::updates`] skip such updates.
    pub is_duplicate: bool,
    /// The state after it.
    pub state: InterpreterState,
}

/// Follows the shared session: frames (layers `world` and `cards`, counted as a viewer,
/// backfill on, the shared state read after every connect) through an [`InterpreterCore`]. Get
/// it with [`FactoryClient::interpreter`].
///
/// Loops can follow one another: [`Interpreter::wait_for`], then [`Interpreter::events`],
/// [`Interpreter::activities`] or [`Interpreter::updates`], and so on. A later loop goes on where
/// the last one stopped and misses nothing in between: it first yields the rest of the frame the
/// last loop stopped in (the events, activities and tray facts that loop did not yield; after
/// `wait_for`, the events after the match and all of that frame's activities and tray facts),
/// then the frames after it
/// (the stream resumes after the last frame fed, while the server still holds it). Use one loop
/// at a time; dropping a loop's stream closes its connection.
#[derive(Debug, Clone)]
pub struct Interpreter {
    client: FactoryClient,
    options: FramesOptions,
    core: Arc<Mutex<InterpreterCore>>,
    follow: Arc<Mutex<Follow>>,
}

/// Where the last loop stopped.
#[derive(Debug, Default)]
struct Follow {
    /// The rest of the frame it stopped in (what it did not yield).
    pending: Option<Update>,
    /// The event id of the newest frame fed.
    last_id: Option<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The events (or activities) of a loop's updates, one at a time. Dropped inside an update (after
/// one of its items, before asking for the next), it leaves the rest of that update (what it did
/// not yield) to the interpreter's next loop; dropped after the stream ended, after an error, or
/// while it waits for the next update, nothing.
struct Parts<T> {
    updates: BoxStream<'static, Result<Update>>,
    follow: Arc<Mutex<Follow>>,
    /// The update being split (its part taken out) and the part's items not yet yielded.
    current: Option<(Update, VecDeque<T>)>,
    part: fn(&mut Update) -> &mut Vec<T>,
}

fn events_of(update: &mut Update) -> &mut Vec<Event> {
    &mut update.events
}

fn activities_of(update: &mut Update) -> &mut Vec<Activity> {
    &mut update.activities
}

fn tray_facts_of(update: &mut Update) -> &mut Vec<TrayFact> {
    &mut update.tray_facts
}

impl<T: Unpin> Stream for Parts<T> {
    type Item = Result<T>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<T>>> {
        let this = self.get_mut();
        loop {
            if let Some((_, items)) = &mut this.current
                && let Some(item) = items.pop_front()
            {
                return Poll::Ready(Some(Ok(item)));
            }
            // asked for more: the update is done with, and nothing of it is left to the next loop
            this.current = None;
            match ready!(this.updates.poll_next_unpin(cx)) {
                Some(Ok(mut update)) => {
                    let items = std::mem::take((this.part)(&mut update)).into();
                    this.current = Some((update, items));
                }
                Some(Err(error)) => return Poll::Ready(Some(Err(error))),
                None => return Poll::Ready(None),
            }
        }
    }
}

impl<T> Drop for Parts<T> {
    fn drop(&mut self) {
        let Some((mut update, items)) = self.current.take() else { return };
        *(self.part)(&mut update) = items.into();
        if !update.events.is_empty() || !update.activities.is_empty() || !update.tray_facts.is_empty() {
            lock(&self.follow).pending = Some(update);
        }
    }
}

impl Interpreter {
    pub(crate) fn new(client: FactoryClient) -> Interpreter {
        let options = FramesOptions {
            layers: vec![Layer::World, Layer::Cards],
            observe: Some(client.observe_default()),
            backfill: true,
            clock_on_connect: true,
            ..FramesOptions::default()
        };
        Interpreter { client, options, core: Arc::default(), follow: Arc::default() }
    }

    /// The layers to stream (`world` is needed for events; `cards` adds hand card titles).
    pub fn layers(mut self, layers: impl IntoIterator<Item = Layer>) -> Interpreter {
        self.options.layers = layers.into_iter().collect();
        self
    }

    /// `true` does not count as a viewer (an idle worker interprets nothing: "nobody is
    /// watching").
    pub fn observe(mut self, observe: bool) -> Interpreter {
        self.options.observe = Some(observe);
        self
    }

    /// Hold frames whose world is late and read it back (default on).
    pub fn backfill(mut self, backfill: bool) -> Interpreter {
        self.options.backfill = backfill;
        self
    }

    /// Reconnect when the stream ends (default on; off ends [`Interpreter::updates`] with the
    /// first connection, e.g. over a finite fake or a `limit`, with the final update).
    pub fn resume(mut self, resume: bool) -> Interpreter {
        self.options.resume = resume;
        self
    }

    /// Start after this cursor instead of the newest frame.
    pub fn after(mut self, cursor: i64) -> Interpreter {
        self.options.after = Some(cursor);
        self
    }

    /// The stream options in use.
    pub fn options(&self) -> &FramesOptions {
        &self.options
    }

    /// Every frame's [`Update`], endlessly (the stream resumes, and reads the shared state after
    /// every connect, so wall times are known from the first frame); duplicates are skipped. The
    /// first update of a loop that follows another may be the rest of the frame that loop stopped
    /// in: only the events and activities it did not yield. A stream the server ends with resume
    /// off ends with the final update, as a replay does (`is_final`, no frame: the activities
    /// still open, closed, and the open tray's leaving); a loop that stops early, or whose stream
    /// ends on an error, gets none. An event that does not decode is yielded as an `Err` (code
    /// `decode`) and the stream goes on: it does not take the final update away.
    pub fn updates(&self) -> BoxStream<'static, Result<Update>> {
        let (client, options, core, follow) =
            (self.client.clone(), self.options.clone(), self.core.clone(), self.follow.clone());
        async move {
            let (rest, last_id) = {
                let mut follow = lock(&follow);
                (follow.pending.take(), follow.last_id.clone())
            };
            let options = FramesOptions { last_event_id: last_id.or(options.last_event_id), ..options };
            // an error that ends the stream means no final update; an event that does not decode
            // is yielded and the stream goes on (spec 2.3, rule 9)
            let failed = Arc::new(AtomicBool::new(false));
            let (seen_core, seen_failed) = (core.clone(), failed.clone());
            let frames = client.streams().frames(options).filter_map(move |item| {
                let (core, follow, failed) = (seen_core.clone(), follow.clone(), seen_failed.clone());
                async move {
                    if item.as_ref().is_err_and(|error| !error.is_event()) {
                        failed.store(true, Ordering::Relaxed);
                    }
                    match item {
                        Ok(StreamItem::Frame(frame)) => {
                            // a frame without an id (NDJSON) keeps the last one
                            let mut seen = lock(&follow);
                            seen.last_id = frame.event_id.clone().or(seen.last_id.take());
                            drop(seen);
                            let update = lock(&core).feed_item(*frame);
                            (!update.is_duplicate).then_some(Ok(update))
                        }
                        Ok(StreamItem::Clock(clock)) => {
                            lock(&core).set_clock(clock.state);
                            None
                        }
                        Ok(StreamItem::Gap(_) | StreamItem::Reset(_)) => {
                            // frames dropped here: the tray rules count the next frame as one after
                            // frames they could not read (spec 4.6.1)
                            lock(&core).gap();
                            None
                        }
                        Ok(_) => None,
                        Err(error) => Some(Err(error)),
                    }
                }
            });
            let last =
                futures::stream::once(async move { (!failed.load(Ordering::Relaxed)).then(|| Ok(lock(&core).end())) })
                    .filter_map(std::future::ready);
            futures::stream::iter(rest.map(Ok)).chain(frames).chain(last)
        }
        .flatten_stream()
        .boxed()
    }

    /// Just the events. A loop that stops inside a frame leaves the rest of it (its later
    /// events and its activities) to the next loop.
    pub fn events(&self) -> BoxStream<'static, Result<Event>> {
        Parts { updates: self.updates(), follow: self.follow.clone(), current: None, part: events_of }.boxed()
    }

    /// Just the activities. A loop that stops inside a frame leaves the rest of it (its later
    /// activities and its events) to the next loop.
    pub fn activities(&self) -> BoxStream<'static, Result<Activity>> {
        Parts { updates: self.updates(), follow: self.follow.clone(), current: None, part: activities_of }.boxed()
    }

    /// Just the tray facts (spec 4.6): trays arriving and leaving, tray stones removed, set down
    /// or no longer sure. A loop that stops inside a frame leaves the rest of it to the next loop.
    pub fn tray_facts(&self) -> BoxStream<'static, Result<TrayFact>> {
        Parts { updates: self.updates(), follow: self.follow.clone(), current: None, part: tray_facts_of }.boxed()
    }

    /// A report as each tray leaves (spec 4.7): which stones were removed, added and moved, and
    /// whether that is the tray's whole account. A tray leaves when the next one arrives, the
    /// session changes or the frames end.
    pub fn tray_reports(&self) -> BoxStream<'static, Result<TrayReport>> {
        let core = Arc::clone(&self.core);
        self.tray_facts()
            .flat_map(move |fact| {
                let reports: Vec<Result<TrayReport>> = match fact {
                    Err(e) => vec![Err(e)],
                    Ok(f) => lock(&core)
                        .trays()
                        .iter()
                        .filter(|t| f.kind == TrayFactKind::Leaving && t.session == f.session && t.n == f.tray)
                        .filter_map(Tray::report)
                        .map(Ok)
                        .collect(),
                };
                futures::stream::iter(reports)
            })
            .boxed()
    }

    /// The trays so far, oldest first (see [`InterpreterCore::trays`]): jump between their
    /// `arrived` and `leaving` points with [`Session::seek`](crate::Session::seek) on a recording.
    pub fn trays(&self) -> Vec<Tray> {
        lock(&self.core).trays()
    }

    /// The unit's speckle answers read so far, oldest first (see [`InterpreterCore::speckle`]).
    pub fn speckle(&self) -> Vec<TraySpeckle> {
        lock(&self.core).speckle()
    }

    /// The path of a stone's solid in this session from the frames fed (see
    /// [`InterpreterCore::track`]); the server's own track since the session began is
    /// [`Api::get_stone`](crate::Api::get_stone).
    pub fn track(&self, stone_id: i64) -> Vec<TrackPoint> {
        lock(&self.core).track(stone_id)
    }

    /// The first event of type `kind` within `timeout`; `Ok(None)` when none came in time. The
    /// rest of its frame (its later events and its activities) is left to the next loop.
    pub async fn wait_for(&self, kind: EventKind, timeout: Duration) -> Result<Option<Event>> {
        self.wait_for_match(move |e| e.kind == kind, timeout).await
    }

    /// The first event that `predicate` accepts within `timeout`; `Ok(None)` when none came in
    /// time. It waits on past a stream event that does not decode, as the loops go on past it
    /// (spec 2.3 rule 9); an error that ends the stream is returned as it is, and the end of the
    /// stream is `stream_closed`.
    pub async fn wait_for_match(
        &self,
        mut predicate: impl FnMut(&Event) -> bool + Send,
        timeout: Duration,
    ) -> Result<Option<Event>> {
        let mut events = self.events();
        let first = async {
            while let Some(event) = events.next().await {
                match event {
                    Ok(event) if predicate(&event) => return Ok(Some(event)),
                    Ok(_) => {}
                    Err(error) if error.is_event() => {}
                    Err(error) => return Err(error),
                }
            }
            Err(Error::new(ErrorCode::StreamClosed, "the frame stream ended"))
        };
        tokio::time::timeout(timeout, first).await.unwrap_or(Ok(None))
    }

    /// The state after the last frame fed.
    pub fn state(&self) -> InterpreterState {
        lock(&self.core).state()
    }

    /// The summary so far.
    pub fn summary(&self) -> Summary {
        lock(&self.core).summary()
    }

    /// Closes what is still open (moves as `interrupted`, push bursts ended) and returns it; the
    /// open tray leaves too (its facts are in [`Interpreter::trays`]; [`Interpreter::end`] returns
    /// them).
    pub fn finish(&self) -> Vec<Activity> {
        lock(&self.core).finish()
    }

    /// [`Interpreter::finish`] as the final update: the activities still open and the facts of
    /// the open tray leaving.
    pub fn end(&self) -> Update {
        lock(&self.core).end()
    }
}
