//! The world layer: the Stable sim's persistent stones and hands, its events, stone tracks and
//! the event log.

use serde::{Deserialize, Serialize};

use super::{Extra, ResolvedFocus, Side};
use crate::wire::wire_enum;

wire_enum! {
    /// The type of a world event.
    EventKind {
        /// A stone appears.
        New = "new",
        /// A stone is confirmed.
        Confirm = "confirm",
        /// A stone gets its final shape.
        Freeze = "freeze",
        /// A stone is pushed (by a hand, along its row when a hand pushed the row, or reseated).
        Push = "push",
        /// A hand lifts a stone.
        Lift = "lift",
        /// A hand tips a stone up on its edge.
        Hinge = "hinge",
        /// A hand carries a stone.
        Carry = "carry",
        /// A hand places a stone.
        Place = "place",
        /// A hand drops a stone.
        Drop = "drop",
        /// A hand takes a stone out of view.
        Gone = "gone",
        /// A stone is removed from the world.
        Delete = "delete",
        /// A stone splits in two (or is trimmed).
        Split = "split",
    }
}

wire_enum! {
    /// A hand's state in the world.
    HandStateKind {
        /// No contact at the board: touching nothing, or (with `HandState::away` true) in contact
        /// away from the board.
        Free = "free",
        /// Touching a stone.
        Touch = "touch",
        /// Grasping a stone.
        Grasp = "grasp",
        /// Tipping a stone up.
        Hinge = "hinge",
        /// Carrying a stone.
        Carry = "carry",
    }
}

wire_enum! {
    /// A stone's mode in the world.
    StoneMode {
        /// Seen by the model.
        Seen = "seen",
        /// Kept without a detection.
        Kept = "kept",
        /// Being pushed.
        Pushed = "pushed",
        /// Tipped up on its edge.
        Hinge = "hinge",
        /// Carried.
        Carry = "carry",
        /// Just put down.
        Placed = "placed",
        /// In transit.
        Transit = "transit",
    }
}

wire_enum! {
    /// A stone's family.
    StoneFamily {
        /// A rectangular block.
        Rect = "rect",
        /// A dogbone paver.
        Dogbone = "dogbone",
        /// A plain box (render lists only).
        Box = "box",
    }
}

wire_enum! {
    /// How final a stone's shape is.
    StoneShape {
        /// The stone model's shape.
        Model = "model",
        /// Provisional.
        Provisional = "provisional",
        /// Final.
        Frozen = "frozen",
    }
}

wire_enum! {
    /// The board's motion.
    Motion {
        /// Still.
        Static = "static",
        /// Moving.
        Moving = "moving",
        /// Settling.
        Settling = "settling",
    }
}

/// The board rectangle in the scene world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Rect {
    /// Centre x.
    pub cx: f64,
    /// Centre y.
    pub cy: f64,
    /// Rotation.
    pub theta: f64,
    /// Half width.
    pub hw: f64,
    /// Half height.
    pub hh: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Rect {
    /// The four corners in the scene world, counter-clockwise from (-hw, -hh).
    pub fn corners(&self) -> [[f64; 2]; 4] {
        let (s, c) = self.theta.sin_cos();
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(u, v)| {
            let (x, y) = (u * self.hw, v * self.hh);
            [self.cx + c * x - s * y, self.cy + s * x + c * y]
        })
    }
}

/// A stone's solid: position, orientation and size in the scene world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Solid {
    /// Centre `[x, y, z]`.
    pub pos: Vec<f64>,
    /// Quaternion `[x, y, z, w]`.
    pub q: Vec<f64>,
    /// `[L, W, h]`.
    pub dims: Vec<f64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A stone of the world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoneView {
    /// Persistent over the session, the same for every viewer.
    pub id: i64,
    /// Mode.
    pub mode: StoneMode,
    /// Family.
    pub family: StoneFamily,
    /// How final its shape is.
    pub shape: StoneShape,
    /// Detected by the model in this frame.
    pub detected: bool,
    /// Uncertain.
    pub uncertain: bool,
    /// The hand holding it.
    pub side: Option<Side>,
    /// Native px, 4 or 12 vertices.
    pub outline: Vec<Vec<f64>>,
    /// The solid.
    pub solid: Option<Solid>,
    /// Packets since a put-down.
    pub placed: Option<i64>,
    /// `model` or `image` (fixer thin-v1).
    pub source: Option<String>,
    /// The stone's look (the 20.20 unit sends looks since build `claude-20261120-looks1`), sent
    /// only in the frame it was read on, read by value (spec 4.6.2): a look that does not fit
    /// [`StoneLook`] so is `None`, and the stone and its frame read as they would without it. See
    /// [`StoneLook`] and [`look_score`](crate::interpreter::look_score).
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "look_by_value")]
    pub look: Option<StoneLook>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A stone's look: the band-pass texture of its face, format `v` 1, for telling stones apart by
/// appearance. The 20.20 unit sends looks since build `claude-20261120-looks1` (fixer thin-v1, on
/// every camera unless a catalog entry's stone model turns `thin.lookOut` off); the recordings of
/// 22 November (build `claude-20261122-hr4x1`) carry them, and those of 17 November, made before
/// it, carry none. The tray rules use it when a server sends it (spec 4.6.2). The grid holds `nu`
/// x `nv` cells at a pitch of `sp` proxy px, centred on the face `at` and turned with it; `d` is
/// the cells as base64 of `nu` x `nv` int8 bytes, row after row, -128 for a cell not read.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoneLook {
    /// Format version: only 1 is read.
    pub v: i64,
    /// The packet whose frame it was read on.
    pub g: i64,
    /// Cell pitch, proxy px.
    pub sp: f64,
    /// Scale of the z values: a cell is round(z q).
    pub q: f64,
    /// Cells along the long side (the engine sends at least 3).
    pub nu: i64,
    /// Cells across it (the engine sends at least 3).
    pub nv: i64,
    /// Proxy px per native px, x and y.
    pub px: Vec<f64>,
    /// The face it was read on, proxy px.
    pub at: LookFace,
    /// `nu` x `nv` int8 cells, base64.
    pub d: String,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The face a look was read on, in proxy px: its centre, the long side's angle (radians, x right,
/// y down) and its sides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LookFace {
    /// Centre, x.
    pub cx: f64,
    /// Centre, y.
    pub cy: f64,
    /// The long side's angle, radians.
    pub theta: f64,
    /// The long side.
    #[serde(rename = "L")]
    pub l: f64,
    /// The short side.
    #[serde(rename = "S")]
    pub s: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A stone's look by value (spec 4.6.2), from its JSON text: a number is a JSON number (never a
/// boolean or a string) that a double holds, and a whole number one whose double is an integer
/// from -2^53 to 2^53 (3 and 3.0 alike). A look that does not fit [`StoneLook`] so is `None`; its
/// text is read on its own, so a number no double holds (`1e400`) drops only the look.
fn look_by_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<StoneLook>, D::Error> {
    let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
    Ok(serde_json::from_str::<serde_json::Value>(raw.get()).ok().as_ref().and_then(StoneLook::by_value))
}

/// The largest whole number a look may hold: 2^53, below which every integer is exact in a double.
const MAX_WHOLE: f64 = 9_007_199_254_740_992.0;

/// The entries of a JSON object other than the known ones.
fn unknown(o: &serde_json::Map<String, serde_json::Value>, known: &[&str]) -> Extra {
    o.iter().filter(|(k, _)| !known.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()
}

impl StoneLook {
    /// The look a JSON value holds by value, if it fits (see [`StoneView::look`]).
    fn by_value(v: &serde_json::Value) -> Option<StoneLook> {
        let m = v.as_object()?;
        let at = m.get("at")?.as_object()?;
        let number = |x: Option<&serde_json::Value>| x?.as_f64().filter(|f| f.is_finite());
        let whole = |x| number(x).filter(|f| f.fract() == 0.0 && f.abs() <= MAX_WHOLE).map(|f| f as i64);
        let px = m.get("px")?.as_array()?.iter().map(|x| number(Some(x))).collect::<Option<Vec<f64>>>()?;
        Some(StoneLook {
            v: whole(m.get("v"))?,
            g: whole(m.get("g"))?,
            sp: number(m.get("sp"))?,
            q: number(m.get("q"))?,
            nu: whole(m.get("nu"))?,
            nv: whole(m.get("nv"))?,
            px,
            at: LookFace {
                cx: number(at.get("cx"))?,
                cy: number(at.get("cy"))?,
                theta: number(at.get("theta"))?,
                l: number(at.get("L"))?,
                s: number(at.get("S"))?,
                extra: unknown(at, &["cx", "cy", "theta", "L", "S"]),
            },
            d: m.get("d")?.as_str()?.to_string(),
            extra: unknown(m, &["v", "g", "sp", "q", "nu", "nv", "px", "at", "d"]),
        })
    }
}

impl StoneView {
    /// The solid's centre, if there is one with a position.
    pub fn position(&self) -> Option<&[f64]> {
        self.solid.as_ref().map(|s| s.pos.as_slice()).filter(|p| !p.is_empty())
    }

    /// Centre of the outline in native px.
    pub fn outline_centre(&self) -> Option<[f64; 2]> {
        let points: Vec<&Vec<f64>> = self.outline.iter().filter(|p| p.len() >= 2).collect();
        if points.is_empty() {
            return None;
        }
        let n = points.len() as f64;
        Some([points.iter().map(|p| p[0]).sum::<f64>() / n, points.iter().map(|p| p[1]).sum::<f64>() / n])
    }
}

/// A hand in the world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HandState {
    /// `free`, `touch`, `grasp`, `hinge` or `carry`. `touch` and `grasp` are the temporal model's contact readings.
    /// With fixer thin-v1 (dry videos and cameras), a hand in contact that holds no stone and whose wrist is off the
    /// board (the regions' tray map) is sent as `free`, with `away` true.
    pub state: HandStateKind,
    /// The stone it holds or touches.
    pub stone: Option<i64>,
    /// Effective ray scale.
    #[serde(rename = "rayK")]
    pub ray_k: Option<f64>,
    /// Untrusted.
    pub untrusted: Option<bool>,
    /// Present (true) only on a hand sent as `free` because it is in contact away from the board: a worker's hand
    /// busy beside the board, or a false hand on machinery. It does not hold a stone and does not work on the tray.
    pub away: Option<bool>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Both hands of the world.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldHands {
    /// The left hand.
    pub left: HandState,
    /// The right hand.
    pub right: HandState,
}

/// A world event as sent.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldEvent {
    /// The type.
    #[serde(rename = "type")]
    pub kind: EventKind,
    /// The stone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    /// The hand.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// Why (`course`, `reseat`, `absent`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// `split`: the stone's own id, then the new part's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub into: Option<Vec<i64>>,
    /// `new` of a stone the model never boxed: `image`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// World statistics.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldStats {
    /// Stones.
    pub stones: i64,
    /// Confirmed stones.
    pub confirmed: i64,
    /// Frozen stones.
    pub frozen: i64,
    /// Fitted stones.
    pub fitted: i64,
    /// Held stones.
    pub held: i64,
    /// Step time.
    #[serde(rename = "stepMs")]
    pub step_ms: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Per-packet draw terms resolved on the server.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldDraw {
    /// The board.
    pub board: Option<Rect>,
    /// Skeleton terms (`logF`, per-hand ray scale `k`, per-side `elbow`).
    pub skeleton: Option<serde_json::Value>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The Stable sim's persistent world at one grid frame (`stw1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldLayer {
    /// `stw1`.
    pub schema: String,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Grid frame.
    pub g: i64,
    /// Media time.
    pub t: f64,
    /// The board's motion.
    pub motion: Motion,
    /// Log focal scale.
    #[serde(rename = "logF")]
    pub log_f: f64,
    /// The stones.
    pub stones: Vec<StoneView>,
    /// The hands.
    pub hands: WorldHands,
    /// What happened in this frame, in world order.
    pub events: Vec<WorldEvent>,
    /// Statistics.
    pub stats: WorldStats,
    /// Draw terms.
    pub draw: Option<WorldDraw>,
    /// The shared focus with its path.
    pub focus: Option<ResolvedFocus>,
    /// Revision: absent (0) as first released; 1, 2, ... after a paused frame was refined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<i64>,
    /// The unit's speckle answer for a tray (spec 4.6.12), sent once, in the world of the first
    /// frame after it was ready (and in that frame's revisions), read by value: an answer that does
    /// not fit [`WorldSpeckle`] so is `None`, and the world and its frame read as they would
    /// without it.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "speckle_by_value")]
    pub speckle: Option<WorldSpeckle>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// One of the two moments a speckle answer compares (spec 4.6.12).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeckleMoment {
    /// Grid frame.
    pub g: i64,
    /// Media time.
    pub t: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// One stone of one moment as the speckle check found it at the other moment (spec 4.6.12).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpeckleFind {
    /// The stone's world id at its own moment.
    pub id: i64,
    /// The world id of the stone (not held) at the other moment whose outline holds the centre of
    /// the best place (`None` when no outline holds it, or when the stone could not be checked).
    pub at: Option<i64>,
    /// The zero-mean normalised cross-correlation of the best place, the highest of the ten turned
    /// templates (`None`: the stone was not checked). A find is clean when `score >= 0.30` and
    /// `score >= 2 x second`.
    pub score: Option<f64>,
    /// That of the best place outside the best's neighbourhood (-1: none; `None`: not checked).
    pub second: Option<f64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The unit's speckle stone identification of one tray (spec 4.6.12). Every stone not held at
/// either moment is checked at the other by its speckle: the high-passed full-resolution luma of
/// the inner 62 % of its outline's box (for a stone too large for the unit's GPU kernels, such
/// as a long kerb, the central part of that box the kernels take), turned 0 and 180 degrees,
/// each by -4 to 4 degrees.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldSpeckle {
    /// The version of the answer's format (1).
    pub v: i64,
    /// The unit's count of trays in this session (nearly always the SDK's tray number).
    pub tray: i64,
    /// The reference moment, the tray's arrival (or the first later calm frame that could be
    /// kept).
    #[serde(rename = "ref")]
    pub reference: SpeckleMoment,
    /// The later moment, a calm frame of the tray.
    pub at: SpeckleMoment,
    /// The GPU the check ran on, with its compute capability and multiprocessors.
    pub device: String,
    /// GPU time of the checks, ms.
    pub gpu_ms: f64,
    /// Time the unit's speckle service spent on the request, ms.
    pub wall_ms: f64,
    /// Every stone not held at `at`, in world order, checked at the reference.
    pub stones: Vec<SpeckleFind>,
    /// Every stone not held at the reference, in world order, checked at `at`.
    pub refs: Vec<SpeckleFind>,
    /// `[id at at, id at the reference]`, by the first id: the stones the unit pairs (a clean find
    /// from one lands on the other, and the other's, when clean, on the first).
    pub pairs: Vec<[i64; 2]>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The unit's speckle answer by value (spec 4.6.12), from its JSON text, as a look is read (spec
/// 4.6.2): `v` 1; `tray`, both moments' `g`, each find's `id` and `at` (or null) and both ids of
/// each pair whole numbers; both moments' `t`, `gpu_ms` and `wall_ms` numbers; `device` a string;
/// each find's `score` and `second` both numbers or both null. An answer that does not fit
/// [`WorldSpeckle`] so is `None`.
fn speckle_by_value<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<WorldSpeckle>, D::Error> {
    let raw = Box::<serde_json::value::RawValue>::deserialize(d)?;
    Ok(serde_json::from_str::<serde_json::Value>(raw.get()).ok().as_ref().and_then(WorldSpeckle::by_value))
}

/// A JSON number a double holds.
fn number(x: Option<&serde_json::Value>) -> Option<f64> {
    x?.as_f64().filter(|f| f.is_finite())
}

/// A whole number: a number whose double is an integer from -2^53 to 2^53.
fn whole(x: Option<&serde_json::Value>) -> Option<i64> {
    number(x).filter(|f| f.fract() == 0.0 && f.abs() <= MAX_WHOLE).map(|f| f as i64)
}

impl SpeckleMoment {
    fn by_value(v: Option<&serde_json::Value>) -> Option<SpeckleMoment> {
        let m = v?.as_object()?;
        Some(SpeckleMoment { g: whole(m.get("g"))?, t: number(m.get("t"))?, extra: unknown(m, &["g", "t"]) })
    }
}

impl SpeckleFind {
    fn by_value(v: &serde_json::Value) -> Option<SpeckleFind> {
        let m = v.as_object()?;
        let given = |k: &str| m.get(k).filter(|x| !x.is_null());
        let at = match given("at") {
            None => None,
            x => Some(whole(x)?),
        };
        let (score, second) = match (given("score"), given("second")) {
            (None, None) => (None, None),
            (s, q) => (Some(number(s)?), Some(number(q)?)),
        };
        let extra = unknown(m, &["id", "at", "score", "second"]);
        Some(SpeckleFind { id: whole(m.get("id"))?, at, score, second, extra })
    }
}

impl WorldSpeckle {
    /// The answer a JSON value holds by value, if it fits (see [`WorldLayer::speckle`]).
    fn by_value(v: &serde_json::Value) -> Option<WorldSpeckle> {
        let m = v.as_object()?;
        let finds = |k: &str| m.get(k)?.as_array()?.iter().map(SpeckleFind::by_value).collect::<Option<Vec<_>>>();
        let pair = |p: &serde_json::Value| match p.as_array()?.as_slice() {
            [a, b] => Some([whole(Some(a))?, whole(Some(b))?]),
            _ => None,
        };
        Some(WorldSpeckle {
            v: whole(m.get("v")).filter(|&v| v == 1)?,
            tray: whole(m.get("tray"))?,
            reference: SpeckleMoment::by_value(m.get("ref"))?,
            at: SpeckleMoment::by_value(m.get("at"))?,
            device: m.get("device")?.as_str()?.to_string(),
            gpu_ms: number(m.get("gpu_ms"))?,
            wall_ms: number(m.get("wall_ms"))?,
            stones: finds("stones")?,
            refs: finds("refs")?,
            pairs: m.get("pairs")?.as_array()?.iter().map(pair).collect::<Option<Vec<_>>>()?,
            extra: unknown(m, &["v", "tray", "ref", "at", "device", "gpu_ms", "wall_ms", "stones", "refs", "pairs"]),
        })
    }
}

impl WorldLayer {
    /// The revision, 0 when absent.
    pub fn revision(&self) -> i64 {
        self.rev.unwrap_or(0)
    }

    /// The first stone with this id.
    pub fn stone(&self, id: i64) -> Option<&StoneView> {
        self.stones.iter().find(|s| s.id == id)
    }

    /// The board rectangle.
    pub fn board(&self) -> Option<&Rect> {
        self.draw.as_ref().and_then(|d| d.board.as_ref())
    }
}

/// `GET /session/world/stones/{id}`: one stone with its path since the session began.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StoneTrack {
    /// `sts1`.
    pub schema: String,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Stone id.
    pub id: i64,
    /// Newest grid frame.
    pub g: i64,
    /// Its media time.
    pub t: f64,
    /// The stone now (`None` once deleted).
    pub stone: Option<StoneView>,
    /// First grid frame it was in.
    pub first_g: i64,
    /// Last grid frame it was in.
    pub last_g: i64,
    /// Native px path, oldest first.
    pub path2d: Vec<Vec<f64>>,
    /// Scene world path, oldest first.
    pub path3d: Vec<Vec<f64>>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// One entry of the event log.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventLogEntry {
    /// Grid frame.
    pub g: i64,
    /// Media time.
    pub t: f64,
    /// The event.
    #[serde(flatten)]
    pub event: WorldEvent,
}

/// `GET /session/world/events`: the event log of this session, oldest first.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventLog {
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Events.
    pub events: Vec<EventLogEntry>,
    /// The `after` that continues this log without loss or repeats.
    pub next_after: i64,
    /// Oldest retained grid frame.
    pub oldest_g: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_helpers() {
        let w: WorldLayer = serde_json::from_str(
            r#"{"g":5,"stones":[{"id":3,"outline":[[0,0],[2,0],[2,2],[0,2]],"solid":{"pos":[1,2,3]}},{"id":4,"solid":{"pos":[]}},{"id":5,"outline":[[1]]}],"draw":{"board":{"cx":1,"cy":2,"theta":0,"hw":1,"hh":0.5}},"rev":2}"#,
        )
        .unwrap();
        assert_eq!(w.revision(), 2);
        assert_eq!(WorldLayer::default().revision(), 0);
        assert_eq!(w.stone(3).unwrap().position(), Some(&[1.0, 2.0, 3.0][..]));
        assert_eq!(w.stone(4).unwrap().position(), None);
        assert_eq!(w.stone(5).unwrap().position(), None);
        assert!(w.stone(9).is_none());
        assert_eq!(w.stone(3).unwrap().outline_centre(), Some([1.0, 1.0]));
        assert_eq!(w.stone(5).unwrap().outline_centre(), None);
        let corners = w.board().unwrap().corners();
        assert_eq!(corners[0], [0.0, 1.5]);
        assert_eq!(corners[2], [2.0, 2.5]);
        assert!(WorldLayer::default().board().is_none());
        let e: EventLogEntry = serde_json::from_str(r#"{"g":1,"t":0.2,"type":"split","id":4,"into":[4,9]}"#).unwrap();
        assert_eq!(e.event.kind, EventKind::Split);
        assert_eq!(e.event.into, Some(vec![4, 9]));
        assert_eq!(serde_json::to_string(&e.event).unwrap(), r#"{"type":"split","id":4,"into":[4,9]}"#);
    }

    #[test]
    fn a_hand_away_from_the_board() {
        // `away` is present (true) only on a hand sent as free because it is in contact away from the board
        let hand = |s: &str| serde_json::from_str::<HandState>(s);
        assert_eq!(hand(r#"{"state":"free","stone":null}"#).unwrap().away, None);
        assert_eq!(hand(r#"{"state":"free","stone":null,"away":true}"#).unwrap().away, Some(true));
        assert_eq!(hand(r#"{"state":"free","stone":null,"away":false}"#).unwrap().away, Some(false));
        assert!(hand(r#"{"state":"free","away":"yes"}"#).is_err());
        let away = hand(r#"{"state":"free","stone":null,"away":true}"#).unwrap();
        assert!(serde_json::to_string(&away).unwrap().contains(r#""away":true"#));
    }
}
