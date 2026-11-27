//! [`InterpreterCore`]: the synchronous state machine of spec 2.4 and 4. The streams and
//! [`crate::Replay`] feed it; it does no I/O, so its rules are unit- and golden-tested.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use super::summary::{HandTimes, HandsTimes, Summary};
use super::text::round3;
use super::trays::{SPECKLE_KEPT, Sink, TRAYS_KEPT, Tray, TrayFact, TrayLedger, TraySpeckle};
use super::{
    Activity, Event, HandView, HandsView, InterpreterState, Move, MoveOutcome, OpenMove, PushBurst, Stone, TimeRef,
    Update,
};
use crate::models::{EventKind, Frame, Rect, Side, State, WorldEvent, WorldLayer, WorldStats};
use crate::streams::FrameItem;

/// A push joins its hand's open burst when it comes at most this many seconds after the
/// burst's latest push.
pub const BURST_GAP: f64 = 1.0;

/// How many points of a stone's track the core keeps (the server keeps 512 per path), and how
/// many stones' tracks (the most recently moved ones).
const TRACK_POINTS: usize = 512;
const TRACK_STONES: usize = 4096;

/// The grid frames whose revisions and events the core remembers behind the newest, once it
/// remembers twice as many (a stream that runs for days stays bounded).
const KEEP_FRAMES: i64 = 4096;

/// One point of a stone's track: where its solid was at a grid frame (scene x, y, z in units
/// of the template gauge).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TrackPoint {
    /// The grid frame.
    pub g: i64,
    /// Its media time.
    pub t: f64,
    /// The solid's centre.
    pub pos: [f64; 3],
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
enum EventKey {
    World(EventKind, Option<i64>, Option<Side>, Option<Vec<i64>>),
    Sheet(Option<String>, Vec<i64>),
}

fn event_key(e: &WorldEvent) -> EventKey {
    if e.kind == EventKind::Sheet {
        let mut ids = e.ids.clone().unwrap_or_default();
        ids.sort_unstable();
        ids.dedup();
        return EventKey::Sheet(e.why.clone(), ids);
    }
    EventKey::World(e.kind.clone(), e.id, e.side.clone(), e.into.clone())
}

#[derive(Debug, Clone)]
struct OpenBurst {
    hand: Side,
    start: TimeRef,
    last: TimeRef,
    stones: Vec<i64>,
    pushes: u64,
    session: i64,
    video: String,
}

impl OpenBurst {
    fn close(self) -> Activity {
        Activity::Push(PushBurst {
            hand: Some(self.hand),
            start: self.start,
            end: self.last,
            duration: self.last.t - self.start.t,
            stones: self.stones,
            pushes: self.pushes,
            session: self.session,
            video: self.video,
        })
    }
}

/// How many trays that have left the core keeps (the newest; 0 keeps every one).
#[derive(Debug, Clone, Copy)]
struct Kept(usize);

impl Default for Kept {
    fn default() -> Kept {
        Kept(TRAYS_KEPT)
    }
}

/// The interpreter's rules, without I/O: feed it frames in the order received, clock states as
/// they come ([`InterpreterCore::set_clock`]) and the stream's `gap` and `reset` events as they
/// come ([`InterpreterCore::gap`]), then call [`InterpreterCore::end`] (or
/// [`InterpreterCore::finish`]) at the end of a replay or stream.
///
/// ```
/// use pdigger_factory_sdk::{Frame, InterpreterCore};
/// let mut core = InterpreterCore::new();
/// let frame: Frame = serde_json::from_str(
///     r#"{"cursor":1,"session":7,"video":"dry1","g":1505,"t":301.6,
///         "world":{"events":[{"type":"lift","id":643,"side":"right"}],"stones":[{"id":643}]}}"#,
/// ).unwrap();
/// let update = core.feed(frame);
/// assert_eq!(update.events[0].describe(), "5:01.6 Right hand lifts stone 643");
/// assert_eq!(core.finish()[0].describe(), "5:01.6 Right hand is still holding stone 643 (0.0 s so far)");
/// ```
#[derive(Debug, Clone, Default)]
pub struct InterpreterCore {
    session: Option<i64>,
    video: Option<String>,
    g: Option<i64>,
    t: Option<f64>,
    stones: Arc<BTreeMap<i64, Stone>>,
    hands: HandsView,
    board: Option<Rect>,
    stats: Option<WorldStats>,
    moves: BTreeMap<Option<i64>, OpenMove>,
    bursts: Vec<OpenBurst>,
    emitted: HashMap<i64, HashSet<EventKey>>,
    revs: HashMap<i64, i64>,
    /// The tray links before each kept grid frame, for its revisions (one copy while they do not
    /// change).
    links: HashMap<i64, Arc<BTreeMap<i64, String>>>,
    last_links: Arc<BTreeMap<i64, String>>,
    last_world_ref: Option<TimeRef>,
    /// The newest clock state of each of the last 8 sessions, oldest first.
    clocks: Vec<State>,
    summary: Summary,
    session_first_t: f64,
    session_last_t: f64,
    prev_g: Option<i64>,
    /// The sums and counts of the move means (summed in order, as the other languages do).
    dur_sum: f64,
    dur_n: u64,
    dist_sum: f64,
    dist_n: u64,
    /// Milliseconds per hand and state: each new frame with a world adds its step ([`hand_step`]).
    hand_ms: [[u64; 5]; 2],
    /// The session's previous new frame (`g`, `t`, its step), for the steps, and the newest frame's step.
    step_prev: Option<(i64, f64, u64)>,
    step_ms: u64,
    /// Each stone's track, with when it last moved (on `track_clock`).
    tracks: HashMap<i64, (u64, Vec<TrackPoint>)>,
    track_clock: u64,
    /// The trays of the session (spec 4.6).
    ledger: TrayLedger,
    /// The trays that have left, oldest first.
    trays: Vec<Tray>,
    trays_kept: Kept,
    /// The unit's speckle answers read, oldest first (the newest [`SPECKLE_KEPT`]; spec 4.6.12).
    speckle: VecDeque<TraySpeckle>,
}

impl InterpreterCore {
    /// A core with no state.
    pub fn new() -> InterpreterCore {
        InterpreterCore::default()
    }

    /// Set the tray rectangle in native pixels, ordered left, top, right, bottom. Pass `None` for the full frame.
    pub fn set_tray_bounds(&mut self, bounds: Option<[f64; 4]>) -> Result<(), &'static str> {
        if let Some(b) = bounds
            && (!b.iter().all(|v| v.is_finite()) || b[0] >= b[2] || b[1] >= b[3])
        {
            return Err("tray bounds must be a finite rectangle with positive width and height");
        }
        self.ledger.bounds = bounds;
        Ok(())
    }

    /// A clock state (a stream's `clock` event, a recording's clock line, `getSession`): the
    /// source of `wall` times. The wall times of a session's frames come from that session's
    /// newest state, so the frames of a session that ended keep theirs after the next session's
    /// clock has come. A state with a lower `revision` than the one kept for its session is
    /// older and is ignored; the states of the last 8 sessions are kept.
    pub fn set_clock(&mut self, state: State) {
        if let Some(i) = self.clocks.iter().position(|c| c.session == state.session) {
            if state.revision < self.clocks[i].revision {
                return;
            }
            self.clocks.remove(i);
        }
        self.clocks.push(state);
        if self.clocks.len() > 8 {
            self.clocks.remove(0);
        }
    }

    /// The newest clock state taken.
    pub fn clock(&self) -> Option<&State> {
        self.clocks.last()
    }

    /// The stream dropped frames here (a `gap` or a `reset` event): the tray rules count the next
    /// frame as one after frames they could not read, as after a frame without a world, a jump in
    /// `g` or a revision (spec 4.6.1, presence questions). The [`Interpreter`](super::Interpreter)
    /// calls it for every `gap` and `reset` of its stream, and a replay for every `gap` and `reset`
    /// line that decodes.
    pub fn gap(&mut self) {
        self.ledger.skipped();
    }

    /// The wall time (epoch seconds) at which the shared playhead shows (or showed) media time
    /// `t` of `session`, from that session's newest clock state, the same as
    /// [`SessionClock::wall_of`](crate::SessionClock::wall_of): `start + (t - anchor_media) /
    /// rate`, where `start` is `max(anchor_wall, hold_until)` while playing (the playhead runs
    /// from the end of the worker's hold) and `anchor_wall` while paused. `None` without a clock
    /// state of that session, or when the rate is not above 0.
    pub fn wall(&self, session: i64, t: f64) -> Option<f64> {
        self.clocks.iter().rev().find(|c| c.session == session).and_then(|c| c.wall_of(t))
    }

    fn time_ref(&self, f: &Frame) -> TimeRef {
        TimeRef { g: f.g, t: f.t, wall: self.wall(f.session, f.t) }
    }

    /// Feeds one frame; returns its events, the activities it completed, the tray facts it made
    /// and the state after it.
    pub fn feed(&mut self, frame: Frame) -> Update {
        self.feed_item(FrameItem { frame, is_revision: false, is_backfill: false, event_id: None })
    }

    /// Feeds one stream item's frame (keeping its revision and backfill marks).
    pub fn feed_item(&mut self, item: FrameItem) -> Update {
        let parts = self.feed_parts(&item.frame);
        Update {
            events: parts.events,
            activities: parts.activities,
            tray_facts: parts.tray_facts,
            speckle: parts.speckle,
            is_revision: item.is_revision || parts.is_revision,
            is_backfill: item.is_backfill,
            is_final: false,
            is_duplicate: parts.is_duplicate,
            state: self.state(),
            frame: Some(item.frame),
        }
    }

    /// Closes what is still open at the end of a replay or stream and returns it as the final
    /// update (`is_final`, no frame): the open moves closed as `interrupted` and the push bursts
    /// ended, sorted as in spec 4.3, and the facts of the open tray leaving.
    pub fn end(&mut self) -> Update {
        let mut activities = Vec::new();
        self.close_session(&mut activities);
        let tray_facts = self.close_trays(TrayLedger::default());
        self.session = None;
        Update {
            frame: None,
            events: Vec::new(),
            activities,
            tray_facts,
            speckle: Vec::new(),
            is_revision: false,
            is_backfill: false,
            is_final: true,
            is_duplicate: false,
            state: self.state(),
        }
    }

    /// Feeds one frame; returns its parts without the state: events, activities, tray facts, and
    /// whether it was a revision.
    pub fn feed_parts(&mut self, f: &Frame) -> FrameParts {
        let mut events = Vec::new();
        let mut acts = Vec::new();
        let mut facts = Vec::new();
        let (g, t) = (f.g, f.t);
        let w = f.world.as_ref();
        let rev = w.and_then(|w| w.rev).unwrap_or(0);
        if self.session != Some(f.session) {
            self.close_session(&mut acts);
            facts = self.close_trays(TrayLedger::new(f.session, &f.video));
            self.session = Some(f.session);
            self.video = Some(f.video.clone());
            self.stones = Arc::default();
            self.hands = HandsView::default();
            self.board = None;
            self.stats = None;
            self.emitted.clear();
            self.revs.clear();
            self.links.clear();
            self.last_links = Arc::default();
            self.tracks.clear();
            self.last_world_ref = None;
            self.summary.sessions += 1;
            // The first frame of a session is always a new grid frame (`prev_g` is reset).
            self.session_first_t = t;
            self.prev_g = None;
            self.step_prev = None;
        } else if self.prev_g.is_some_and(|prev| g <= prev) {
            // An already seen grid frame: a revision (a higher world.rev) or a duplicate (dropped).
            let Some(w) = w.filter(|_| self.revs.get(&g).is_some_and(|&seen| rev > seen)) else {
                return FrameParts { is_duplicate: true, ..FrameParts::default() };
            };
            self.summary.revisions += 1;
            self.revs.insert(g, rev);
            // a revised event's tray stone is the one before its own frame
            let links = self.links.get(&g).cloned().unwrap_or_default();
            let keys = self.emitted.entry(g).or_default();
            let fresh: Vec<&WorldEvent> = w.events.iter().filter(|e| keys.insert(event_key(e))).collect();
            for e in fresh {
                events.push(self.make_event(f, e, w, &links));
            }
            let mut closed = Vec::new();
            self.apply_activities(f, w, &events, &mut closed);
            self.emit(closed, &mut acts);
            if self.g.is_none_or(|newest| g >= newest) {
                self.take_state(w);
            }
            // a frame again, which the tray rules never read: it may hold a hand at work (spec 4.6.1)
            self.ledger.skipped();
            return FrameParts {
                events,
                activities: acts,
                tray_facts: facts,
                is_revision: true,
                ..FrameParts::default()
            };
        }
        // A new grid frame of this session.
        self.summary.frames += 1;
        if self.summary.first_g.is_none() {
            self.summary.first_g = Some(g);
            self.summary.first_t = Some(t);
        }
        self.summary.last_g = Some(g);
        self.summary.last_t = Some(t);
        if self.prev_g.is_some_and(|prev| g > prev + 1) {
            self.summary.gaps += 1;
            self.ledger.skipped();
        }
        self.prev_g = Some(g);
        self.step_ms = hand_step(self.step_prev, g, t);
        self.step_prev = Some((g, t, self.step_ms));
        self.session_last_t = t;
        self.g = Some(g);
        self.t = Some(t);
        self.take_cards(f);
        // the tray links before this frame: its events', later its revisions', and the speckle answers',
        // which also read those of a frame without a world (spec 4.6.12)
        let links = self.ledger.links();
        if *self.last_links != links {
            self.last_links = Arc::new(links);
        }
        self.links.insert(g, self.last_links.clone());
        let Some(w) = w else {
            self.summary.missing_world += 1;
            self.ledger.skipped();
            return FrameParts { events, activities: acts, tray_facts: facts, ..FrameParts::default() };
        };
        self.summary.frames_with_world += 1;
        self.revs.insert(g, rev);
        let links = self.last_links.clone();
        self.prune(g);
        // Stale push bursts close before this frame's events.
        let mut closed = Vec::new();
        let (stale, open): (Vec<OpenBurst>, Vec<OpenBurst>) =
            std::mem::take(&mut self.bursts).into_iter().partition(|b| t - b.last.t > BURST_GAP);
        self.bursts = open;
        closed.extend(stale.into_iter().map(OpenBurst::close));
        let keys = self.emitted.entry(g).or_default();
        keys.extend(w.events.iter().map(event_key));
        for e in &w.events {
            events.push(self.make_event(f, e, w, &links));
        }
        self.apply_activities(f, w, &events, &mut closed);
        self.emit(closed, &mut acts);
        self.take_state(w);
        self.record_tracks(g, t, w);
        self.last_world_ref = Some(self.time_ref(f));
        let n = w.stones.len() as u64;
        self.summary.stones.last = Some(n);
        self.summary.stones.max = self.summary.stones.max.max(n);
        self.summary.stones.confirmed_last = Some(w.stats.confirmed);
        for (side, hand) in [&w.hands.left, &w.hands.right].into_iter().enumerate() {
            if let Some(i) = HandTimes::index(&hand.state) {
                self.hand_ms[side][i] += self.step_ms;
            }
        }
        let now = self.time_ref(f);
        self.ledger.feed(w, now, &mut Sink { counts: &mut self.summary.trays, facts: &mut facts });
        self.keep_trays_left();
        // the unit's speckle answer, read after the trays took the frame (spec 4.6.12)
        let links = &self.links;
        let speckle: Vec<TraySpeckle> = w
            .speckle
            .as_ref()
            .and_then(|sp| self.ledger.speckle(sp, now, |g| links.get(&g).map(|l| (**l).clone()).unwrap_or_default()))
            .into_iter()
            .collect();
        self.speckle.extend(speckle.iter().cloned());
        self.speckle.drain(..self.speckle.len().saturating_sub(SPECKLE_KEPT));
        FrameParts { events, activities: acts, tray_facts: facts, speckle, ..FrameParts::default() }
    }

    /// Makes the open tray leave and puts `next` in the ledger's place; returns the facts.
    fn close_trays(&mut self, mut next: TrayLedger) -> Vec<TrayFact> {
        let mut facts = Vec::new();
        self.ledger.close(&mut Sink { counts: &mut self.summary.trays, facts: &mut facts });
        self.keep_trays_left();
        next.bounds = self.ledger.bounds;
        self.ledger = next;
        facts
    }

    /// Keeps the trays that have left (the newest, in a new list once in a while).
    fn keep_trays_left(&mut self) {
        self.trays.extend(self.ledger.take_closed());
        let n = self.trays_kept.0;
        if n > 0 && self.trays.len() >= 2 * n {
            self.trays.drain(..self.trays.len() - n);
        }
    }

    /// Sets how many trays that have left [`InterpreterCore::trays`] keeps, the newest (0: every
    /// one); the default is [`TRAYS_KEPT`](super::trays::TRAYS_KEPT).
    pub fn keep_trays(&mut self, n: usize) {
        self.trays_kept = Kept(n);
    }

    /// The trays so far, oldest first: those that have left (the newest kept, across sessions)
    /// and the open one. Each has its `arrived` and `leaving` points, its tray stones and its
    /// facts (spec 4.6).
    pub fn trays(&self) -> Vec<Tray> {
        let n = self.trays_kept.0;
        let skip = if n > 0 { self.trays.len().saturating_sub(n) } else { 0 };
        self.trays[skip..].iter().cloned().chain(self.ledger.current()).collect()
    }

    /// The unit's speckle answers read so far, oldest first (the newest [`SPECKLE_KEPT`], across
    /// sessions), each with its tray and the tray stones in it (spec 4.6.12). An answer that comes
    /// after its tray left is here, not in the tray.
    pub fn speckle(&self) -> Vec<TraySpeckle> {
        self.speckle.iter().cloned().collect()
    }

    /// Forgets the revisions, events and tray links of grid frames more than [`KEEP_FRAMES`]
    /// behind `g`.
    fn prune(&mut self, g: i64) {
        if self.revs.len() as i64 > 2 * KEEP_FRAMES {
            self.revs.retain(|&k, _| k >= g - KEEP_FRAMES);
            self.emitted.retain(|&k, _| k >= g - KEEP_FRAMES);
            self.links.retain(|&k, _| k >= g - KEEP_FRAMES);
        }
    }

    /// Adds a point to the track of every stone whose solid moved since its last point.
    fn record_tracks(&mut self, g: i64, t: f64, w: &WorldLayer) {
        for stone in &w.stones {
            let Some(&[x, y, z, ..]) = stone.position() else { continue };
            let point = TrackPoint { g, t, pos: [x, y, z] };
            self.track_clock += 1;
            match self.tracks.get_mut(&stone.id) {
                Some((_, points)) if points.last().is_some_and(|p| p.pos == point.pos) => {}
                Some((moved, points)) => {
                    points.push(point);
                    if points.len() >= 2 * TRACK_POINTS {
                        points.drain(..TRACK_POINTS);
                    }
                    *moved = self.track_clock;
                }
                None => {
                    self.tracks.insert(stone.id, (self.track_clock, vec![point]));
                    if self.tracks.len() > TRACK_STONES {
                        let oldest = self.tracks.iter().min_by_key(|(_, (moved, _))| *moved).map(|(id, _)| *id);
                        self.tracks.remove(&oldest.unwrap_or_default());
                    }
                }
            }
        }
    }

    /// The path of a stone's solid in this session, oldest first: a point for every frame fed
    /// where it moved, at most the newest 512, for the 4096 most recently moved stones; empty
    /// for a stone the core has no track of. The server's own track since the session began is
    /// [`Api::get_stone`](crate::Api::get_stone).
    pub fn track(&self, stone_id: i64) -> Vec<TrackPoint> {
        self.tracks
            .get(&stone_id)
            .map_or_else(Vec::new, |(_, points)| points[points.len().saturating_sub(TRACK_POINTS)..].to_vec())
    }

    /// The event of a world event; `links` are the tray links before the event's frame.
    fn make_event(&mut self, f: &Frame, e: &WorldEvent, w: &WorldLayer, links: &BTreeMap<i64, String>) -> Event {
        *self.summary.events.get_mut(&e.kind) += 1;
        Event {
            kind: e.kind.clone(),
            g: f.g,
            t: f.t,
            wall: self.wall(f.session, f.t),
            session: f.session,
            video: f.video.clone(),
            stone_id: e.id,
            hand: e.side.clone(),
            why: e.why.clone(),
            into: e.into.clone(),
            ids: e.ids.clone(),
            source: e.source.clone(),
            stone: w.stones.iter().find(|s| Some(s.id) == e.id).cloned(),
            tray_stone: e.id.and_then(|id| links.get(&id).cloned()),
        }
    }

    fn apply_activities(&mut self, f: &Frame, w: &WorldLayer, events: &[Event], closed: &mut Vec<Activity>) {
        let now = self.time_ref(f);
        let pos = |sid: Option<i64>| {
            // fewer than 2 numbers: as absent
            let p = w.stones.iter().rev().find(|s| Some(s.id) == sid).and_then(|s| s.position());
            p.filter(|p| p.len() >= 2).map(<[f64]>::to_vec)
        };
        let hand_closed: HashSet<Option<i64>> = events
            .iter()
            .filter(|e| matches!(e.kind, EventKind::Place | EventKind::Drop | EventKind::Gone))
            .map(|e| e.stone_id)
            .collect();
        for ev in events {
            let (sid, side) = (ev.stone_id, ev.hand.clone());
            if ev.kind == EventKind::Delete && hand_closed.contains(&sid) {
                continue; // The same frame has the hand's put-down or carry-away of this stone: that decides.
            }
            match &ev.kind {
                EventKind::Push => {
                    let Some(side) = side.filter(|s| matches!(s, Side::Left | Side::Right | Side::Both)) else {
                        continue;
                    };
                    if ev.why.as_deref().is_some_and(|why| !why.is_empty()) {
                        continue;
                    }
                    self.push(f, side, sid, now, closed);
                }
                EventKind::Lift | EventKind::Hinge => match self.moves.get_mut(&sid) {
                    None => {
                        let open = OpenMove {
                            stone_id: sid,
                            hand: side,
                            start: now,
                            from: pos(sid),
                            carried: false,
                            tipped: ev.kind == EventKind::Hinge,
                            session: f.session,
                            video: f.video.clone(),
                        };
                        self.moves.insert(sid, open);
                    }
                    Some(open) => {
                        open.tipped |= ev.kind == EventKind::Hinge;
                        merge_hand(&mut open.hand, side);
                    }
                },
                EventKind::Carry => {
                    if let Some(open) = self.moves.get_mut(&sid) {
                        open.carried = true;
                        merge_hand(&mut open.hand, side);
                    }
                }
                EventKind::Place | EventKind::Drop | EventKind::Gone | EventKind::Delete => {
                    let outcome = match ev.kind {
                        EventKind::Place => MoveOutcome::Placed,
                        EventKind::Drop => MoveOutcome::Dropped,
                        EventKind::Gone => MoveOutcome::CarriedAway,
                        _ => MoveOutcome::Removed,
                    };
                    let (open, partial) = match self.moves.remove(&sid) {
                        Some(mut open) => {
                            merge_hand(&mut open.hand, side);
                            (open, false)
                        }
                        None if outcome == MoveOutcome::Removed => continue,
                        None => {
                            let open = OpenMove {
                                stone_id: sid,
                                hand: side,
                                start: now,
                                from: None,
                                carried: false,
                                tipped: false,
                                session: f.session,
                                video: f.video.clone(),
                            };
                            (open, true)
                        }
                    };
                    let to = matches!(outcome, MoveOutcome::Placed | MoveOutcome::Dropped).then(|| pos(sid)).flatten();
                    closed.push(Activity::Move(open.close(outcome, now, to, partial)));
                }
                _ => {}
            }
        }
    }

    fn push(&mut self, f: &Frame, side: Side, sid: Option<i64>, now: TimeRef, closed: &mut Vec<Activity>) {
        let mut index = self.bursts.iter().position(|b| b.hand == side);
        if let Some(i) = index.filter(|&i| f.t - self.bursts[i].last.t > BURST_GAP) {
            closed.push(self.bursts.remove(i).close());
            index = None;
        }
        let i = match index {
            Some(i) => i,
            None => {
                let burst = OpenBurst {
                    hand: side,
                    start: now,
                    last: now,
                    stones: Vec::new(),
                    pushes: 0,
                    session: f.session,
                    video: f.video.clone(),
                };
                self.bursts.push(burst);
                self.bursts.len() - 1
            }
        };
        let burst = &mut self.bursts[i];
        burst.last = now;
        burst.pushes += 1;
        if let Some(sid) = sid.filter(|sid| !burst.stones.contains(sid)) {
            burst.stones.push(sid);
            burst.stones.sort_unstable();
        }
    }

    fn emit(&mut self, mut closed: Vec<Activity>, out: &mut Vec<Activity>) {
        closed.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        for a in &closed {
            self.count(a);
        }
        out.extend(closed);
    }

    fn count(&mut self, a: &Activity) {
        match a {
            Activity::Push(b) => {
                let p = &mut self.summary.push_bursts;
                p.total += 1;
                p.pushes += b.pushes;
                match b.hand {
                    Some(Side::Left) => p.left += 1,
                    Some(Side::Right) => p.right += 1,
                    // Bursts are opened only for left, right and both hands.
                    _ => p.both += 1,
                }
            }
            Activity::Move(m) => {
                let c = &mut self.summary.moves;
                c.total += 1;
                match m.outcome {
                    MoveOutcome::Placed => c.placed += 1,
                    MoveOutcome::Dropped => c.dropped += 1,
                    MoveOutcome::CarriedAway => c.carried_away += 1,
                    MoveOutcome::Removed => c.removed += 1,
                    _ => c.interrupted += 1,
                }
                c.partial += u64::from(m.partial);
                c.carried += u64::from(m.carried);
                if !m.partial && m.outcome != MoveOutcome::Interrupted {
                    self.dur_sum += m.duration;
                    self.dur_n += 1;
                }
                if let Some(d) = m.distance {
                    self.dist_sum += d;
                    self.dist_n += 1;
                }
            }
        }
    }

    fn close_session(&mut self, out: &mut Vec<Activity>) {
        if self.session.is_none() {
            return;
        }
        let end = self.last_world_ref;
        let mut closed: Vec<Activity> =
            std::mem::take(&mut self.moves).into_values().map(|m| Activity::Move(m.interrupt(end))).collect();
        closed.extend(std::mem::take(&mut self.bursts).into_iter().map(OpenBurst::close));
        self.emit(closed, out);
        self.summary.media_seconds += self.session_last_t - self.session_first_t;
    }

    fn take_state(&mut self, w: &WorldLayer) {
        self.stones = Arc::new(w.stones.iter().map(|s| (s.id, s.clone())).collect());
        self.hands.left.state = Some(w.hands.left.state.clone());
        self.hands.left.stone = w.hands.left.stone;
        self.hands.right.state = Some(w.hands.right.state.clone());
        self.hands.right.stone = w.hands.right.stone;
        self.board = w.board().cloned();
        self.stats = Some(w.stats.clone());
    }

    fn take_cards(&mut self, f: &Frame) {
        if let Some(cards) = &f.cards {
            self.hands.left.card = Some(cards.left.title.clone()).filter(|t| !t.is_empty());
            self.hands.right.card = Some(cards.right.title.clone()).filter(|t| !t.is_empty());
        }
    }

    /// [`InterpreterCore::end`]'s activities: what is still open closed (moves as `interrupted`,
    /// push bursts), sorted as in spec 4.3; the open tray leaves (its facts are in
    /// [`InterpreterCore::trays`]).
    pub fn finish(&mut self) -> Vec<Activity> {
        self.end().activities
    }

    /// The summary so far (spec 4.4).
    pub fn summary(&self) -> Summary {
        let mut s = self.summary.clone();
        if self.session.is_some() {
            // The open session counts up to its newest frame.
            s.media_seconds += self.session_last_t - self.session_first_t;
        }
        if self.dur_n > 0 {
            s.moves.mean_duration = Some(round3(self.dur_sum / self.dur_n as f64));
        }
        if self.dist_n > 0 {
            s.moves.mean_distance = Some(round3(self.dist_sum / self.dist_n as f64));
        }
        s.hands =
            HandsTimes { left: HandTimes::from_ms(&self.hand_ms[0]), right: HandTimes::from_ms(&self.hand_ms[1]) };
        s.media_seconds = round3(s.media_seconds);
        s.first_t = s.first_t.map(round3);
        s.last_t = s.last_t.map(round3);
        s
    }

    /// The state after the last frame.
    pub fn state(&self) -> InterpreterState {
        InterpreterState {
            session: self.session,
            video: self.video.clone(),
            g: self.g,
            t: self.t,
            wall: self.session.zip(self.t).and_then(|(s, t)| self.wall(s, t)),
            stones: self.stones.clone(),
            hands: self.hands.clone(),
            board: self.board.clone(),
            stats: self.stats.clone(),
            open_moves: self.moves.values().cloned().collect(),
            clock: self.clocks.last().cloned(),
            tray: self.ledger.current(),
        }
    }
}

/// What [`InterpreterCore::feed_parts`] gives for a frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrameParts {
    /// Its new events, in world order.
    pub events: Vec<Event>,
    /// The activities it completed, in the order of spec 4.3.
    pub activities: Vec<Activity>,
    /// The tray facts it made (spec 4.6).
    pub tray_facts: Vec<TrayFact>,
    /// It revised a frame already seen.
    pub is_revision: bool,
    /// It repeated a frame already seen (the same or a lower world.rev) and was dropped: no
    /// events, activities or tray facts, and not counted.
    pub is_duplicate: bool,
    /// The unit's speckle answer its world carried, read as a tray's (spec 4.6.12): at most one.
    pub speckle: Vec<TraySpeckle>,
}

fn merge_hand(hand: &mut Option<Side>, side: Option<Side>) {
    let Some(side) = side else { return };
    match hand {
        None => *hand = Some(side),
        Some(current) if *current != side => *hand = Some(Side::Both),
        Some(_) => {}
    }
}

impl OpenMove {
    fn close(self, outcome: MoveOutcome, end: TimeRef, to: Option<Vec<f64>>, partial: bool) -> Move {
        let distance = match (&self.from, &to) {
            (Some(a), Some(b)) if a.len() >= 2 && b.len() >= 2 => {
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                Some(round3((dx * dx + dy * dy).sqrt()))
            }
            _ => None,
        };
        Move {
            stone_id: self.stone_id,
            hand: self.hand,
            duration: end.t - self.start.t,
            start: self.start,
            end,
            from: self.from,
            to,
            distance,
            carried: self.carried,
            tipped: self.tipped,
            partial,
            outcome,
            session: self.session,
            video: self.video,
        }
    }

    fn interrupt(self, end: Option<TimeRef>) -> Move {
        let end = end.unwrap_or(self.start);
        self.close(MoveOutcome::Interrupted, end, None, false)
    }
}

impl HandsView {
    /// The hand of `side` (`both` and unknown sides give `None`).
    pub fn get(&self, side: &Side) -> Option<&HandView> {
        match side {
            Side::Left => Some(&self.left),
            Side::Right => Some(&self.right),
            _ => None,
        }
    }
}

/// The milliseconds a new frame adds to its hands' states (spec 4.5): the media time since the session's previous
/// new frame `(g_prev, t_prev, step_prev)` when that frame is the grid step before it (`g == g_prev + 1`), in whole
/// milliseconds, half up, when that is at least 1 ms and below 2^31 ms; otherwise the session's previous step (a frame
/// after a gap, a `t` not after the previous one or not a finite number), and 200 (one grid frame of a recorded video)
/// for a session's first frame.
pub(crate) fn hand_step(prev: Option<(i64, f64, u64)>, g: i64, t: f64) -> u64 {
    let Some((pg, pt, step)) = prev else { return 200 };
    if pg.checked_add(1) == Some(g) {
        let x = (t - pt) * 1000.0 + 0.5;
        if (1.0..2_147_483_648.0).contains(&x) {
            return x.floor() as u64;
        }
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_steps_by_its_media_time_after_the_grid_frame_before_it_else_by_the_previous_step() {
        assert_eq!(hand_step(None, 5, 1.0), 200); // a session's first frame
        assert_eq!(hand_step(Some((4, 0.8, 200)), 5, 1.1), 300);
        assert_eq!(hand_step(Some((4, 0.8, 250)), 6, 1.1), 250); // after a gap
        assert_eq!(hand_step(Some((4, 0.8, 250)), 5, f64::NAN), 250);
        assert_eq!(hand_step(Some((4, f64::NAN, 250)), 5, 1.0), 250);
        assert_eq!(hand_step(Some((4, 0.8, 250)), 5, f64::INFINITY), 250);
        assert_eq!(hand_step(Some((4, f64::NEG_INFINITY, 250)), 5, 1.0), 250);
        assert_eq!(hand_step(Some((4, 1.0, 250)), 5, 1.0), 250); // a t not after the previous one
        assert_eq!(hand_step(Some((4, 1.0, 250)), 5, 0.5), 250);
        assert_eq!(hand_step(Some((4, 1.0, 250)), 5, 1.0004), 250); // below 1 ms
        assert_eq!(hand_step(Some((4, 1.0, 250)), 5, 1.0006), 1);
        assert_eq!(hand_step(Some((4, 0.0, 250)), 5, 2147483.6), 2147483600);
        assert_eq!(hand_step(Some((4, 0.0, 250)), 5, 2147483.7), 250); // 2^31 ms or more
        assert_eq!(hand_step(Some((4, -1e308, 250)), 5, 1e308), 250); // the difference overflows
        assert_eq!(hand_step(Some((i64::MAX, 1.0, 250)), i64::MIN, 1.1), 250); // no wrap-around
    }

    #[test]
    fn hand_times_follow_the_cameras_steps() {
        // a plant camera: g counts the frames its worker released, 0.3 s apart (then 0.31 s, a frame without a
        // world, 0.24 s); the same frame again adds nothing; after a gap (two frames the unit skipped, 8.81 s) a frame
        // adds the session's previous step, 0.24 s; a new session's first frame adds one grid frame, 0.2 s, and
        // after a gap the previous step again
        let frame = |session: i64, g: i64, t: f64, left: Option<&str>| -> Frame {
            let mut f = serde_json::json!({"cursor": g + 1, "session": session, "video": "v", "g": g, "t": t});
            if let Some(left) = left {
                f["world"] = serde_json::json!({"rev": 0, "motion": "static", "events": [], "stones": [],
                    "hands": {"left": {"state": left, "stone": null}, "right": {"state": "free", "stone": null}}});
            }
            serde_json::from_value(f).unwrap()
        };
        let mut core = InterpreterCore::new();
        for (session, g, t, left) in [
            (1, 100, 30.0, Some("grasp")),
            (1, 101, 30.3, Some("grasp")),
            (1, 102, 30.61, Some("free")),
            (1, 103, 30.95, None),
            (1, 104, 31.19, Some("grasp")),
            (1, 104, 31.19, Some("free")),
            (1, 107, 40.0, Some("grasp")),
            (2, 7, 1.4, Some("grasp")),
            (2, 9, 1.8, Some("grasp")),
        ] {
            core.feed(frame(session, g, t, left));
        }
        let s = core.summary();
        assert_eq!(
            (s.hands.left.grasp, s.hands.left.free, s.hands.right.free, s.media_seconds),
            (1.38, 0.31, 1.69, 10.4)
        );
    }

    /// A still frame of three bricks (`rev` 0 for a new frame).
    fn still(g: i64, rev: i64, events: serde_json::Value) -> Frame {
        let brick = |id: i64, x: f64| {
            serde_json::json!({"id": id, "solid": {"pos": [x, 0.0, 0.046], "q": [0.0, 0.0, 0.0, 1.0],
                "dims": [0.3, 0.2, 0.09]}})
        };
        serde_json::from_value(serde_json::json!({"cursor": g + 1, "session": 1, "video": "v", "g": g,
            "t": g as f64 * 0.2, "world": {"rev": rev, "motion": "static", "events": events,
            "stones": [brick(1, -0.6), brick(2, 0.0), brick(3, 0.6)], "stats": {"stones": 3, "confirmed": 3}}}))
        .unwrap()
    }

    #[test]
    fn the_tray_links_are_pruned_with_the_revisions() {
        // 2.4: the oldest frame kept still gives a revision the tray stone from before it, a frame older than that
        // is a duplicate, and the links kept stay bounded
        let mut core = InterpreterCore::new();
        for g in 0..=2 * KEEP_FRAMES {
            core.feed(still(g, 0, serde_json::json!([])));
        }
        // the tray arrives at g 4 (taken at g 10, its visit); more than 2 x 4096 frames kept prune at g 8192,
        // keeping g 4096 on
        let tray = core.state().tray.expect("the open tray");
        assert_eq!(tray.stones[0].id, "s1t0.8-1-1");
        assert_eq!(core.links.len(), 4097);
        assert!(core.links.contains_key(&KEEP_FRAMES) && !core.links.contains_key(&(KEEP_FRAMES - 1)));
        let confirm = serde_json::json!([{"type": "confirm", "id": 1}]);
        let edge = core.feed(still(KEEP_FRAMES, 1, confirm.clone()));
        assert!(edge.is_revision);
        let tray_stones: Vec<_> = edge.events.iter().map(|e| e.tray_stone.as_deref()).collect();
        assert_eq!(tray_stones, [Some("s1t0.8-1-1")]);
        let older = core.feed(still(KEEP_FRAMES - 1, 1, confirm));
        assert!(older.is_duplicate && older.events.is_empty());
    }
}
