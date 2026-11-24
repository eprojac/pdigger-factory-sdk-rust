//! The shared session: its state (the clock), the worker, the Stable sim, controls and focus.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::Extra;
use crate::wire::wire_enum;

wire_enum! {
    /// What changed the shared state last.
    ChangedBy {
        /// The server started.
        Start = "start",
        /// A viewer's control.
        Viewer = "viewer",
        /// Auto-rate lowered the rate because the interpreter fell behind.
        AutoRate = "auto-rate",
        /// The video ended.
        EndOfVideo = "end-of-video",
    }
}

wire_enum! {
    /// Where a live camera session is (`state.live.phase`).
    LivePhase {
        /// No camera frame yet (the worker's reason says why).
        Connecting = "connecting",
        /// The interpreter runs on the camera's first `warmup` seconds; nothing is released.
        Warming = "warming",
        /// Frames are released as they are interpreted.
        Live = "live",
    }
}

wire_enum! {
    /// The interpreter worker's state.
    WorkerState {
        /// Interpreting frames.
        Running = "running",
        /// Nobody is watching: nothing is interpreted.
        Idle = "idle",
        /// Waiting.
        Waiting = "waiting",
        /// Starting.
        Starting = "starting",
        /// Offline.
        Offline = "offline",
        /// Failed.
        Error = "error",
        /// The session is paused.
        Paused = "paused",
    }
}

wire_enum! {
    /// The Stable sim's state.
    StableState {
        /// Running.
        Running = "running",
        /// Starting.
        Starting = "starting",
        /// Failed.
        Failed = "failed",
        /// Off.
        Off = "off",
        /// Unusable.
        Unusable = "unusable",
    }
}

/// The shared stone focus.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Focus {
    /// Focus revision.
    pub n: i64,
    /// Session it was made in.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Stone id.
    pub id: i64,
    /// Grid frame it was picked on.
    pub g: i64,
    /// Media time.
    pub t: f64,
    /// Scene x.
    pub x: f64,
    /// Scene y.
    pub y: f64,
    /// Scene z.
    pub z: f64,
    /// Stone length.
    #[serde(rename = "L")]
    pub length: f64,
    /// Stone width.
    #[serde(rename = "W")]
    pub width: f64,
    /// Yaw.
    pub yaw: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A live camera session's phase (`state.live`; builds before `claude-20261120-live2` send none).
///
/// After a select of a camera the session is `connecting` until the camera's first frame, then
/// `warming` while the interpreter runs on its first `warmup` seconds (nothing is released), then
/// `live`. Any break in the stream (a pause, a camera that stopped) warms up again. Each phase
/// change bumps the state's `revision`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Live {
    /// The phase.
    pub phase: LivePhase,
    /// Seconds of camera frames interpreted before any is shown (3 on the plant cameras).
    pub warmup: f64,
    /// Server wall time (epoch seconds) the session started; a frame's `t` is its arrival in
    /// seconds since then.
    pub since: f64,
    /// Seconds the shown picture is behind its arrival, interpreter included, when the session
    /// went live; play resumes at `now - since - lag`.
    pub lag: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The shared state: the clock of the one shared session.
///
/// While `playing`, the media time at server wall time `w` is
/// `anchor_media + (w - max(anchor_wall, hold_until)) * rate` (see [`crate::SessionClock`]).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The interpretation run; a select or seek starts a new one.
    pub session: i64,
    /// State revision.
    pub revision: i64,
    /// The source: a recording's id, or a live camera's.
    pub video: String,
    /// Playing or paused.
    pub playing: bool,
    /// Media seconds per wall second: at most 0.8 for a recording (build `claude-20261122-hr4x1`
    /// lowers it itself when its worker falls behind: 0.75 on dry1), 1.0 on a live camera.
    pub rate: f64,
    /// Media time at `anchor_wall`.
    pub anchor_media: f64,
    /// Server wall time (epoch seconds) of the anchor.
    pub anchor_wall: f64,
    /// The playhead stands still until this server wall time (the worker's warm-up hold). On a
    /// camera it stays an hour ahead while the session connects and warms up, and is the moment it
    /// went live afterwards.
    pub hold_until: f64,
    /// Duration; `None` on a live camera.
    pub duration: Option<f64>,
    /// What changed the state last.
    pub changed_by: ChangedBy,
    /// The shared focus.
    pub focus: Option<Focus>,
    /// Seconds into the source (on a live camera, since the session started, minus `live.lag`
    /// once it is live).
    pub media_time: f64,
    /// A live camera session's phase; `None` on a recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<Live>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl State {
    /// The wall time at which the playhead of this state shows (or showed) media time `t`:
    /// `start + (t - anchor_media) / rate`, where `start` is `max(anchor_wall, hold_until)` while
    /// playing and `anchor_wall` while paused; `None` when the rate is not above 0.
    pub fn wall_of(&self, t: f64) -> Option<f64> {
        if self.rate.is_nan() || self.rate <= 0.0 {
            return None;
        }
        let start = if self.playing { self.anchor_wall.max(self.hold_until) } else { self.anchor_wall };
        Some(start + (t - self.anchor_media) / self.rate)
    }
}

impl State {
    /// The session shows a live camera: the state has `live` (or the source is `live`, the id the
    /// design gave the camera before the plant cameras shipped).
    pub fn is_live(&self) -> bool {
        self.live.is_some() || self.video == crate::session::LIVE
    }

    /// The phase of a live camera session; `None` on a recording.
    pub fn live_phase(&self) -> Option<LivePhase> {
        self.live.as_ref().map(|live| live.phase.clone())
    }
}

/// The data of an SSE `clock` event: the state and the server's wall clock.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockEvent {
    /// The state.
    #[serde(flatten)]
    pub state: State,
    /// Server wall time when it was sent.
    pub server_now: Option<f64>,
}

/// The interpreter worker's status.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkerStatus {
    /// State.
    pub state: WorkerState,
    /// Why (empty while running).
    pub reason: String,
    /// Frames per second.
    pub fps: Option<f64>,
    /// Frame time.
    pub frame_ms: Option<f64>,
    /// Cycle time.
    pub cycle_ms: Option<f64>,
    /// Session it works on.
    pub session: Option<i64>,
    /// Grid frame it works on.
    pub g: Option<i64>,
    /// GPU clock.
    pub gpu: Option<f64>,
    /// Errors.
    pub errors: Option<i64>,
    /// Stage times.
    #[serde(deserialize_with = "super::null_as_default")]
    pub stage_ms: BTreeMap<String, f64>,
    /// Engine hashes.
    #[serde(deserialize_with = "super::null_as_default")]
    pub engines: BTreeMap<String, String>,
    /// Model checkpoint.
    pub model: Option<String>,
    /// Video.
    pub video: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The hold on a paused frame (fixer thin-v1).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HoldStatus {
    /// Session.
    pub session: i64,
    /// The held grid frame.
    pub g: i64,
    /// The frame refined when it is not `g`.
    pub at: Option<i64>,
    /// `waiting`, `running`, `converged`, ...
    pub state: String,
    /// Refine rounds run.
    pub rounds: i64,
    /// Newest world revision of the frame.
    pub rev: i64,
    /// Wall time of the rounds.
    pub ms: f64,
    /// Packets stepped again.
    pub resteps: i64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The Stable sim's status.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StableStatus {
    /// State.
    pub state: StableState,
    /// Fits.
    pub fits: Option<i64>,
    /// False: grey frames are missing.
    pub evidence: Option<bool>,
    /// Message.
    pub message: Option<String>,
    /// `legacy` or `thin-v1`.
    pub fixer: Option<String>,
    /// The hold on a paused frame.
    pub hold: Option<HoldStatus>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /api/v2/session`: the shared clock, the worker and the Stable sim.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionReply {
    /// Server wall time when the reply was made.
    pub server_now: f64,
    /// Server wall time when the request arrived.
    pub server_recv: f64,
    /// The shared state.
    pub state: State,
    /// The worker.
    pub worker: WorkerStatus,
    /// Viewers.
    pub viewers: i64,
    /// The Stable sim.
    pub stable: StableStatus,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Body of `POST /session/select`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SelectRequest {
    /// The source id.
    pub video: String,
    /// Start time; leave it out for a live camera.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
}

/// Body of `POST /session/seek`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SeekRequest {
    /// Optionally another source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
    /// Media time.
    pub time: f64,
}

/// Body of `POST /session/play` and `/pause`: any JSON object (its fields are ignored).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EmptyRequest {}

/// Body of `PUT /session/focus`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FocusRequest {
    /// The world's stone id.
    pub stone_id: i64,
    /// The frame the stone was picked on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub g: Option<i64>,
    /// Refused with 409 if not the current session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<i64>,
}

/// The reply of every control.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ControlReply {
    /// The state after the control (unchanged with `dry_run`).
    pub state: State,
    /// Server wall time of the reply.
    pub server_now: f64,
    /// Server wall time when the request arrived.
    pub server_recv: f64,
    /// True for a dry run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dry_run: Option<bool>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A focus resolved to its path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolvedFocus {
    /// Stone id.
    pub id: i64,
    /// Native px, oldest first.
    pub path2d: Vec<Vec<f64>>,
    /// Scene world, oldest first.
    pub path3d: Vec<Vec<f64>>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// `GET /session/focus`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FocusReply {
    /// The focus, or `None`.
    pub focus: Option<Focus>,
    /// Its path, or `None`.
    pub resolved: Option<ResolvedFocus>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The data of an SSE `reset` event: the server restarted (another boot).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResetEvent {
    /// The new boot id.
    pub boot: String,
    /// The cursor the stream continues from.
    pub cursor: i64,
}

/// The data of an SSE `gap` event: that many items were skipped for a slow consumer.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GapEvent {
    /// Items skipped.
    pub gap: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_events_flatten_the_state() {
        let c: ClockEvent = serde_json::from_str(r#"{"session":7,"video":"live","playing":true,"rate":1.0,"duration":null,"server_now":12.5,"changed_by":"auto-rate"}"#).unwrap();
        assert_eq!(c.state.session, 7);
        assert!(c.state.is_live());
        assert_eq!(c.server_now, Some(12.5));
        assert_eq!(c.state.changed_by, ChangedBy::AutoRate);
        let partial: ClockEvent = serde_json::from_str(r#"{"revision":418,"media_time":1278.1}"#).unwrap();
        assert_eq!(partial.state.revision, 418);
        assert!(!partial.state.is_live());
        assert_eq!(partial.state.live_phase(), None);
        let camera: State = serde_json::from_str(
            r#"{"session":9,"video":"CAM00000001","duration":null,"live":{"phase":"warming","warmup":3,"since":1794958239.5,"lag":0}}"#,
        )
        .unwrap();
        assert!(camera.is_live());
        assert_eq!(camera.live_phase(), Some(LivePhase::Warming));
        assert_eq!(camera.live.as_ref().map(|l| (l.warmup, l.since)), Some((3.0, 1_794_958_239.5)));
        assert!(serde_json::to_value(&partial.state).unwrap().get("live").is_none(), "a recording's state has none");
        assert_eq!(serde_json::to_value(&camera).unwrap()["live"]["phase"], "warming");
    }

    #[test]
    fn requests_leave_out_what_is_not_given() {
        let live = SelectRequest { video: "live".into(), time: None };
        assert_eq!(serde_json::to_string(&live).unwrap(), r#"{"video":"live"}"#);
        let seek = SeekRequest { video: None, time: 2.5 };
        assert_eq!(serde_json::to_string(&seek).unwrap(), r#"{"time":2.5}"#);
        let focus = FocusRequest { stone_id: 72, g: Some(6389), session: None };
        assert_eq!(serde_json::to_string(&focus).unwrap(), r#"{"stone_id":72,"g":6389}"#);
        assert_eq!(serde_json::to_string(&EmptyRequest::default()).unwrap(), "{}");
    }
}
