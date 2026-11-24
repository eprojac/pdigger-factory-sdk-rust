//! A grid frame and its five layers: raw (`sip1`), overlay (`ovl1`), scene (`scn1`), cards
//! (`crd1`) and world (`stw1`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Extra, Layer, Rect, SessionReply, StableStatus, State, WorkerStatus, WorldLayer};
use crate::wire::wire_enum;

/// One grid frame `g` (5 per media second) with the layers asked for.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// Server cursor of this frame.
    pub cursor: i64,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Grid frame.
    pub g: i64,
    /// Source frame index.
    pub i: i64,
    /// Media time in seconds.
    pub t: f64,
    /// Whether the temporal past was valid.
    pub past_valid: bool,
    /// Temporal past span.
    pub dt_past: f64,
    /// Layers requested but not ready at the release deadline.
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "super::null_as_default")]
    pub missing: Vec<Layer>,
    /// The model's packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Sip1Packet>,
    /// 2-D per packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overlay: Option<OverlayLayer>,
    /// 3-D per packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneLayer>,
    /// The hand cards.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cards: Option<CardsLayer>,
    /// The Stable sim's world.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub world: Option<WorldLayer>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl Frame {
    /// Whether the frame has this layer.
    pub fn has(&self, layer: &Layer) -> bool {
        match layer {
            Layer::Raw => self.raw.is_some(),
            Layer::Overlay => self.overlay.is_some(),
            Layer::Scene => self.scene.is_some(),
            Layer::Cards => self.cards.is_some(),
            Layer::World => self.world.is_some(),
            Layer::Other(_) => self.extra.contains_key(crate::wire::WireEnum::as_str(layer)),
        }
    }

    /// Moves the layers of `other` that this frame lists as missing into it, and removes them
    /// from `missing`. Returns the layers merged.
    pub fn merge_missing(&mut self, other: Frame) -> Vec<Layer> {
        let Frame { mut raw, mut overlay, mut scene, mut cards, mut world, .. } = other;
        let mut merged = Vec::new();
        for layer in std::mem::take(&mut self.missing) {
            let filled = match layer {
                Layer::Raw if raw.is_some() => {
                    self.raw = raw.take();
                    true
                }
                Layer::Overlay if overlay.is_some() => {
                    self.overlay = overlay.take();
                    true
                }
                Layer::Scene if scene.is_some() => {
                    self.scene = scene.take();
                    true
                }
                Layer::Cards if cards.is_some() => {
                    self.cards = cards.take();
                    true
                }
                Layer::World if world.is_some() => {
                    self.world = world.take();
                    true
                }
                _ => false,
            };
            if filled {
                merged.push(layer);
            } else {
                self.missing.push(layer);
            }
        }
        merged
    }
}

/// `GET /session/frames`: the long-poll reply.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveReply {
    /// Server wall time of the reply.
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
    /// Continue from this cursor (never from a frame's).
    pub cursor: i64,
    /// New frames, oldest first.
    pub frames: Vec<Frame>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

impl LiveReply {
    /// The session part of the reply.
    pub fn session(&self) -> SessionReply {
        SessionReply {
            server_now: self.server_now,
            server_recv: self.server_recv,
            state: self.state.clone(),
            worker: self.worker.clone(),
            viewers: self.viewers,
            stable: self.stable.clone(),
            extra: Extra::new(),
        }
    }
}

/// Maps of a raw packet: base64 of zlib of u8 row-major.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Maps {
    /// `[rows, cols]`.
    pub grid: Vec<usize>,
    /// Human map (redacted in public recordings).
    pub human: Option<String>,
    /// Tray map.
    pub tray: Option<String>,
    /// Stone map.
    pub stone: Option<String>,
    /// Confidence map.
    pub conf: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Depth of a raw packet: `logZ = a*(lo + q/255*(hi-lo)) + b`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Depth {
    /// Quantised log depth (redacted in public recordings).
    pub q: Option<String>,
    /// Range low.
    pub lo: f64,
    /// Range high.
    pub hi: f64,
    /// Scale.
    pub a: f64,
    /// Offset.
    pub b: f64,
    /// Anchor.
    pub anchor: String,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Boxes of a raw packet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Boxes {
    /// `x0, y0, x1, y1, rot, presence`.
    pub stone: Vec<serde_json::Value>,
    /// `x0, y0, x1, y1, rot, presence`.
    pub person: Vec<serde_json::Value>,
    /// `u, v, p`.
    pub hand_left: Vec<f64>,
    /// `u, v, p`.
    pub hand_right: Vec<f64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The person of a raw packet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Person {
    /// Present.
    pub present: bool,
    /// Crop box.
    #[serde(rename = "box")]
    pub bbox: Vec<f64>,
    /// Rotated crop.
    pub rot: bool,
    /// Grid of the masks.
    pub grid: Vec<usize>,
    /// Label mask (redacted in public recordings).
    pub mask: Option<String>,
    /// Alpha mask (redacted in public recordings).
    pub alpha: Option<String>,
    /// 70 joints `[u, v, vis, x, y, z]` or null (face joints are null in public recordings).
    pub joints: Vec<Option<Vec<f64>>>,
    /// Scale.
    pub scale: Option<f64>,
    /// Bones used.
    pub bones_used: Option<i64>,
    /// Person scale.
    pub person_scale: Option<f64>,
    /// `template`, `contact` or `contact-now`.
    pub scale_anchor: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The stones of a raw packet. On a 4x packet (`source` `hr4`) `fg` is on `grid` `[160, 256]` and
/// `tray` on `tray_grid` `[80, 128]`, and the items come from the base lattice `centre_grid`
/// `[20, 32]`; a base-decode packet (recorded before the 4x stone head) has `grid` `[40, 64]`,
/// `tray_grid` `[20, 32]` and none of `source`, `centres` and `centre_grid`. Read every map's size
/// from its own grid.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Stones {
    /// Present.
    pub present: bool,
    /// Count.
    pub count: i64,
    /// Crop box.
    #[serde(rename = "box")]
    pub bbox: Vec<f64>,
    /// Rotated crop.
    pub rot: bool,
    /// `[rows, cols]` of `fg` (`[160, 256]` on a 4x packet, `[40, 64]` on a base-decode packet).
    pub grid: Vec<usize>,
    /// Foreground map.
    pub fg: Option<String>,
    /// `[rows, cols]` of `tray` (`[80, 128]` on a 4x packet, `[20, 32]` on a base-decode packet).
    pub tray_grid: Vec<usize>,
    /// Tray map.
    pub tray: Option<String>,
    /// `[u, v, w, h, score, X, Y, Z, w_m, h_m, yaw]` per stone.
    pub items: Vec<Vec<f64>>,
    /// `hr4` on a 4x packet (the masks, tray and yaw come from the 4x stone outputs); `None` on a
    /// base-decode packet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `base` on a 4x packet: the instances come from the base centre peaks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub centres: Option<String>,
    /// `[rows, cols]` of the lattice the instances are found on (`[20, 32]`; 4x packets only).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub centre_grid: Vec<usize>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The tray plane of a raw packet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Tray {
    /// Valid.
    pub valid: bool,
    /// Plane normal.
    pub normal: Vec<f64>,
    /// Plane offset.
    pub d: f64,
    /// Anchor.
    pub anchor: String,
    /// Floor height.
    pub floor: Option<f64>,
    /// Floor anchor.
    pub floor_anchor: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A hand of a raw packet, as the models wrote it. The temporal model's fields (`p_action`,
/// `action`, `p_interaction`, `p_hold`, `contact`, `held`, `p_held`, `stone`) are written for both
/// hands on every packet; when `visible` is false they describe no hand, and the overlay and card
/// layers leave them out.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Hand {
    /// Visible.
    pub visible: bool,
    /// Action probabilities (topology `actions`).
    pub p_action: Vec<f64>,
    /// The action.
    pub action: String,
    /// P(interaction).
    pub p_interaction: f64,
    /// P(hold).
    pub p_hold: f64,
    /// Contact.
    pub contact: f64,
    /// `empty`, `stone`, `tool`, `tray`.
    pub held: String,
    /// Held probabilities.
    pub p_held: Vec<f64>,
    /// Index into this packet's stones, or -1.
    pub stone: i64,
    /// Wrist `[u, v]`.
    pub wrist: Vec<f64>,
    /// P(wrist).
    pub p_wrist: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Both hands of a raw packet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Hands {
    /// Left.
    pub left: Sip1Hand,
    /// Right.
    pub right: Sip1Hand,
}

/// The model codec packet (`sip1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Sip1Packet {
    /// `sip1`.
    pub schema: String,
    /// Maps.
    pub maps: Sip1Maps,
    /// Depth.
    pub depth: Sip1Depth,
    /// Boxes.
    pub boxes: Sip1Boxes,
    /// The person.
    pub person: Sip1Person,
    /// The stones.
    pub stones: Sip1Stones,
    /// The tray plane.
    pub tray: Sip1Tray,
    /// The hands.
    pub hands: Sip1Hands,
    /// P(both hands).
    pub both: f64,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Grid frame.
    pub g: i64,
    /// Source frame index.
    pub i: i64,
    /// Media time.
    pub t: f64,
    /// Whether the temporal past was valid.
    pub past_valid: bool,
    /// Temporal past span.
    pub dt_past: f64,
    /// Stage times.
    pub ms: BTreeMap<String, f64>,
    /// Cursor.
    pub cursor: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A wrist of the overlay.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Wrist {
    /// `[u, v]`, or `None`.
    pub uv: Option<Vec<f64>>,
    /// Visible.
    pub visible: bool,
    /// Visible and P(interaction) >= .5.
    pub filled: bool,
    /// Visible and contact >= .5.
    pub ring: bool,
    /// Index into this frame's items, or -1 (always for a hand that is not visible).
    pub stone: Option<i64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Both wrists.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Wrists {
    /// Left.
    pub left: Wrist,
    /// Right.
    pub right: Wrist,
}

/// A stone item of the overlay.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayItem {
    /// `[u, v, w, h, score, X, Y, Z, w_m, h_m, yaw]`.
    pub t: Vec<f64>,
    /// Index into the previous packet's items, or -1.
    pub prev: i64,
    /// `left`, `right` or `None`.
    pub side: Option<super::Side>,
    /// Four corners, native px.
    pub outline: Vec<Vec<f64>>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The overlay's region maps: base64 u8 `rows*cols`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayRegions {
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// Human (redacted in public recordings).
    pub human: Option<String>,
    /// Tray.
    pub tray: Option<String>,
    /// Stones.
    pub stone: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The overlay's person mask.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayMask {
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// 0 none, 1 human, 2 left, 3 right (redacted in public recordings).
    pub label: Option<String>,
    /// Alpha (redacted in public recordings).
    pub alpha: Option<String>,
    /// Crop box `[u0, v0, u1, v1]`.
    #[serde(rename = "box")]
    pub bbox: Vec<f64>,
    /// Rotated.
    pub rot: bool,
    /// Present.
    pub present: bool,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The overlay's boxes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayBoxes {
    /// `x0, y0, x1, y1, rot, presence`.
    pub person: Vec<serde_json::Value>,
    /// `x0, y0, x1, y1, rot, presence`.
    pub stone: Vec<serde_json::Value>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// 2-D per packet, raw-derived (`ovl1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayLayer {
    /// `ovl1`.
    pub schema: String,
    /// Native size `[w, h]`.
    pub native_wh: Vec<i64>,
    /// Region maps.
    pub regions: OverlayRegions,
    /// Person mask.
    pub mask: OverlayMask,
    /// Boxes.
    pub boxes: OverlayBoxes,
    /// 70 joints `[u, v, vis, x, y, z]` or null.
    pub joints: Vec<Option<Vec<f64>>>,
    /// Person scale.
    pub scale: Option<f64>,
    /// Wrists.
    pub wrists: Wrists,
    /// The previous packet of this session.
    pub prev_g: Option<i64>,
    /// Stone items.
    pub items: Vec<OverlayItem>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The scene's plane.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScenePlane {
    /// Normal.
    pub n: Vec<f64>,
    /// Offset.
    pub d: f64,
    /// Floor.
    pub floor: f64,
    /// Anchor.
    pub anchor: Option<String>,
    /// Floor anchor.
    pub floor_anchor: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The scene's board.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneBoard {
    /// The raw board.
    pub raw: Option<Rect>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The scene's quad bitsets.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneQuads {
    /// Bitset `(rows-1)*(cols-1)`: quads kept.
    pub raw: String,
    /// Same for the Stable view.
    pub stable: String,
}

/// The observed surface.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSurface {
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// Quantised log depth (redacted in public recordings).
    pub q: Option<String>,
    /// Range low.
    pub lo: f64,
    /// Range high.
    pub hi: f64,
    /// Scale.
    pub a: f64,
    /// Offset.
    pub b: f64,
    /// Bitset `rows*cols`: cells on the board.
    pub flat: String,
    /// Quads kept.
    pub quads: SceneQuads,
    /// Lift.
    pub lift: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The scene's skeleton.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSkeleton {
    /// 70 joints `[x, y, z]` or null, scene world.
    pub world: Vec<Option<Vec<f64>>>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A wrist ring.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ring {
    /// Side.
    pub side: super::Side,
    /// Position.
    pub pos: Vec<f64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A stone item of the scene.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneItem {
    /// x.
    pub x: f64,
    /// y.
    pub y: f64,
    /// Yaw.
    pub yaw: f64,
    /// Length.
    #[serde(rename = "L")]
    pub length: f64,
    /// Width.
    #[serde(rename = "S")]
    pub width: f64,
    /// Height.
    pub h: f64,
    /// Side.
    pub side: Option<super::Side>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// 3-D per packet, raw-derived (`scn1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneLayer {
    /// `scn1`.
    pub schema: String,
    /// The tray plane.
    pub plane: ScenePlane,
    /// The board.
    pub board: SceneBoard,
    /// The observed surface.
    pub surface: SceneSurface,
    /// The skeleton.
    pub skeleton: SceneSkeleton,
    /// Wrist rings.
    pub rings: Vec<Ring>,
    /// Stone items.
    pub items: Vec<SceneItem>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

wire_enum! {
    /// The tone of a hand card.
    CardTone {
        /// Not visible or nothing to show.
        None = "none",
        /// Holding.
        Hold = "hold",
        /// Interacting.
        Interact = "interact",
        /// Idle.
        Idle = "idle",
        /// Hidden.
        Hidden = "hidden",
    }
}

/// One fine action of a hand card.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FineAction {
    /// Key.
    pub key: String,
    /// Label.
    pub label: String,
    /// Probability.
    pub p: Option<f64>,
    /// Reliable.
    pub reliable: bool,
}

/// A hand card (`crd1`): what the portal's hand panel shows, the temporal model's readings of one
/// hand. For a hand that is not visible they are all `None` (`contact_on` false, `stone` -1, every
/// `fine[].p` `None`): the model's contact and action guesses there describe no hand.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HandCard {
    /// Title, e.g. `Holding a stone`.
    pub title: String,
    /// Tone.
    pub tone: CardTone,
    /// Visible.
    pub visible: bool,
    /// P(interaction).
    #[serde(rename = "pInteraction")]
    pub p_interaction: Option<f64>,
    /// Contact.
    pub contact: Option<f64>,
    /// Contact on.
    #[serde(rename = "contactOn")]
    pub contact_on: Option<bool>,
    /// Held class.
    pub held: Option<String>,
    /// P(held).
    #[serde(rename = "pHeld")]
    pub p_held: Option<f64>,
    /// Action.
    pub action: Option<String>,
    /// Fine actions.
    pub fine: Vec<FineAction>,
    /// Hint.
    pub hint: Option<String>,
    /// Stone index.
    pub stone: Option<i64>,
    /// P(wrist).
    #[serde(rename = "pWrist")]
    pub p_wrist: Option<f64>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The numbers of the cards layer.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardNumbers {
    /// Stones.
    pub stones: i64,
    /// Past valid.
    pub past_valid: bool,
    /// Past span.
    pub dt_past: f64,
    /// Tray anchor.
    pub tray_anchor: String,
    /// Floor anchor.
    pub floor_anchor: String,
    /// Depth anchor.
    pub depth_anchor: String,
    /// Grid frame.
    pub g: i64,
    /// Media time.
    pub t: f64,
    /// Session.
    pub session: i64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The cards layer (`crd1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CardsLayer {
    /// `crd1`.
    pub schema: String,
    /// Left hand card.
    pub left: HandCard,
    /// Right hand card.
    pub right: HandCard,
    /// Stage timings.
    pub stage: Option<serde_json::Value>,
    /// Numbers.
    pub numbers: CardNumbers,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_present_and_merged() {
        let mut f: Frame = serde_json::from_str(r#"{"cursor":1,"g":2,"missing":["world","scene","raw"],"cards":{"left":{"title":"Holding a stone"}},"zz":1}"#).unwrap();
        assert!(f.has(&Layer::Cards));
        assert!(!f.has(&Layer::World));
        assert!(!f.has(&Layer::Raw));
        assert!(!f.has(&Layer::Overlay));
        assert!(!f.has(&Layer::Scene));
        assert!(f.has(&Layer::Other("zz".into())));
        let other: Frame =
            serde_json::from_str(r#"{"g":2,"world":{"g":2},"raw":{"g":2},"overlay":{},"cards":{}}"#).unwrap();
        let merged = f.merge_missing(other);
        assert_eq!(merged, vec![Layer::World, Layer::Raw]);
        assert_eq!(f.missing, vec![Layer::Scene]);
        assert!(f.world.is_some() && f.raw.is_some());
        let mut g = Frame { missing: vec![Layer::Overlay, Layer::Scene, Layer::Cards], ..Frame::default() };
        let full = Frame {
            overlay: Some(OverlayLayer::default()),
            scene: Some(SceneLayer::default()),
            cards: Some(CardsLayer::default()),
            ..Frame::default()
        };
        assert_eq!(g.merge_missing(full).len(), 3);
        assert!(g.missing.is_empty());
        assert_eq!(serde_json::to_value(&g).unwrap().get("missing"), None);
    }

    #[test]
    fn live_reply_session_part() {
        let r = LiveReply { server_now: 2.0, cursor: 9, viewers: 3, ..LiveReply::default() };
        let s = r.session();
        assert_eq!(s.server_now, 2.0);
        assert_eq!(s.viewers, 3);
    }
}
