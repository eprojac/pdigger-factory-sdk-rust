//! Trays and tray stones (spec 4.6): the SDK's own stone ids, one per stone, never reused.
//!
//! The world's stone ids are the engine tracker's: it renames a stone it lost and found again,
//! and it can keep an id on another stone (a stone set down where a removed one lay, a neighbour
//! pushed into its place, a row of equal stones pushed one place along while the ids stay on the
//! places). The interpreter follows each tray from the moment it has arrived and settled to the
//! moment it leaves, and keeps its own stones on it, the tray stones. A tray stone says whether
//! its identity is still sure; a fact about it is certain or unclear: an identity stays sure, and
//! a fact is certain, only when what the world shows at calm moments leaves no other reading. A
//! certain fact is never revised.
//!
//! A stone without looks of its own ([`TrayStone::looks`] empty) that the world stops detecting
//! while nothing could have changed it is not given up at once: its doubt is a question (a
//! presence question, spec 4.6.1: the stone is not sure meanwhile, and no fact is given), which
//! seeing it again where it was settles when no hand was at work, no push came on the tray, no
//! frame went unread and no other stone of its size stood at its place while it was out of sight.
//!
//! With looks (spec 4.6.3-4.6.10; the 20.20 unit sends them since build `claude-20261120-looks1`,
//! the published recordings carry them) a doubt about a stone that has a look is a question a later
//! look can settle (a stone with looks that is not seen at two calm moments included: only a look
//! settles its question), a look can find a lost stone again or tell which stone a world id stands
//! for now (fact `identified`), and the account of a tray that leaves lists every stone the rules
//! leave open with the readings the evidence leaves (its open account). A look never makes a fact
//! certain: a tray's certain facts with looks are the first of those without them (4.6.7).
//! Without usable looks the rules are those of the first version (4.6.1, presence questions
//! included), line for line.
//!
//! What the SDK can not see: a box the model sees on an empty slot when the tray arrives becomes a
//! tray stone, and a stone set into that slot later keeps its id.
//!
//! With the unit's speckle answers (spec 4.6.12) the leaving reads the tray's last answer, the
//! leaving's (spec 4.6.13, version 8): the stones the unit pairs are sure, the ones the rules had
//! lost or swapped are linked again (fact `identified`), and every stone gets the answer's
//! evidence ([`TrayStone::evidence`]).
//!
//! A tray's report ([`Tray::report`], spec 4.7) is simple mode: which stones were removed,
//! added and moved, when and where (each one's final place on the tray), how likely each change
//! is ([`TrayChange::confidence`]), and whether that is the tray's whole account. The facts say
//! why, stone by stone.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::TimeRef;
use super::looks::{Cells, look_permille, pair_cells, tray_cells};
use super::text::{clock, fmt, round3};
use crate::models::{
    EventKind, HandStateKind, Motion, Rect, StoneMode, StoneShape, StoneView, WorldLayer, WorldSpeckle,
};
use crate::wire::wire_enum;

/// A static stop of at least this many seconds is a tray visit.
const STOP_MIN: f64 = 2.0;
/// A calm moment: this many frames in a row with both hands free, no hand event, no change.
const CALM: u32 = 3;
/// Frames since the last `new` before a tray can arrive.
const QUIET: u32 = 5;
/// A world rename: an unidentified stone first seen within this many seconds of the loss ...
const RENAME_T: f64 = 3.0;
/// ... and this close to the lost stone's place.
const RENAME_D: f64 = 0.15;
/// Two sizes are compatible when each side differs by at most this share of the larger.
const SIZE_TOL: f64 = 0.25;
/// An unidentified stone at the same place at two calm moments: within this distance.
const NEAR: f64 = 0.12;
/// "Next to": hand work, a pushed neighbour, a stone that appeared.
const HAND_R: f64 = 0.40;
/// When a tray leaves, what is left is paired within this distance.
const LEAVE_D: f64 = 1.0;
/// Calm moments in a row a stone's world stone may go unseen before the stone is not sure.
const GHOST: u32 = 2;
/// Share of a stone's box another world box may cover before the stone is not sure.
const OVERLAP: f64 = 0.30;
/// Margin of the empty-footprint check of a removal.
const MARGIN: f64 = 0.03;

// The rules by look (spec 4.6.11); scores are permille integers.

/// The tray's bar while its gauge holds fewer than [`GAUGE_MIN`] pairs.
const T_ID_START: i64 = 900;
/// The lowest bar: `max(T_ID_MIN, alike + GAUGE_GAP)`.
const T_ID_MIN: i64 = 750;
/// How far the bar lies above the most alike pair of two different stones of the tray.
const GAUGE_GAP: i64 = 100;
/// Gauge pairs a tray needs before its bar can drop.
const GAUGE_MIN: u32 = 20;
/// Gauge partners each taken look is scored with, both ways.
const GAUGE_K: usize = 2;
/// The bar for a link change, or with a stone without a chain near.
const T_FIRM: i64 = 800;
/// The margin a decision needs over every scored rival.
const M_ID: i64 = 150;
/// Paired cells a decision or a doubt by look needs.
const N_ID: i64 = 100;
/// Below this an untouched stone's own look changed.
const T_DIFF: i64 = 400;
/// How far a pair reaches.
const D_MATCH: f64 = 1.0;
/// Neighbours of a column, and stones without a chain near.
const D_NEIGH: f64 = 0.40;
/// Pairs per row.
const K_PAIR: usize = 4;
/// Neighbours per column.
const K_NEIGH: usize = 4;
/// Looks taken per look work.
const N_TAKE: usize = 16;

/// The codes of unsure facts, the causes of open unsure entries (spec 4.6.8).
const UNSURE_CODES: [&str; 9] = [
    "overlap",
    "unseen",
    "pushed",
    "neighbour_pushed",
    "appeared",
    "size_changed",
    "found_again",
    "matched_at_leaving",
    "look_changed",
];
/// Why an open removal was not certain ...
const REMOVED_CAUSES: [&str; 9] =
    ["look_lost", "by_look", "twins", "relinked", "lost_late", "covered", "rivals", "no_hand", "unchecked"];
/// ... and an open addition.
const ADDED_CAUSES: [&str; 6] = ["freed", "size_changed", "relinked", "doubtful", "seen_once", "unchecked"];
/// Why the looks could not settle an open unsure or removed entry ...
const STONE_LOOKS: [&str; 6] = ["none", "late", "box", "stale", "tie", "low"];
/// ... and an open addition.
const ADDED_LOOKS: [&str; 3] = ["none", "tie", "new"];

/// How many trays that have left a new [`InterpreterCore`](super::InterpreterCore) keeps for
/// `trays()` (the newest); see `keep_trays`.
pub const TRAYS_KEPT: usize = 256;

/// A point of a tray: grid frame, media time and wall time (`None` before a clock state is known).
pub type TrayPoint = TimeRef;

mod sheets;
mod speckle;
use sheets::check_sheets;
pub use speckle::{
    SPECKLE_CLEAN, SPECKLE_CODES, SPECKLE_KEPT, SPECKLE_NEAR, SpeckleSide, StoneSpeckle, TraySpeckle,
    describe_speckle_txt,
};

wire_enum! {
    /// What a [`TrayFact`] says.
    TrayFactKind {
        /// The tray has arrived and settled.
        Arrived = "arrived",
        /// A tray stone is not sure any more.
        Unsure = "unsure",
        /// A look (spec 4.6.5), or the leaving's speckle answer (spec 4.6.13), told which world
        /// stone a tray stone stands for now: it was found again, recognised on another world
        /// stone, or (by its speckle) confirmed on its own. Never certain.
        Identified = "identified",
        /// A tray stone was removed (certain) or was not found when the tray left (unclear).
        Removed = "removed",
        /// A tray stone was set down (certain) or was not recognised when the tray left (unclear).
        Added = "added",
        /// The tray is leaving: its final account.
        Leaving = "leaving",
        /// Paper was laid over or lifted off tray stones.
        Sheet = "sheet",
    }
}

wire_enum! {
    /// Where a [`TrayStone`] is.
    TrayStoneStatus {
        /// On the tray, standing for a world stone.
        On = "on",
        /// Its world stone went; not (yet) found again.
        Lost = "lost",
        /// Removed: for certain, or not found when the tray left.
        Removed = "removed",
    }
}

wire_enum! {
    /// What the leaving's speckle answer found of a tray stone at the other moment
    /// ([`TrayStone::evidence`], spec 4.6.13): for a stone the tray arrived with, its arrival
    /// world id's find (is it on the tray as it left?); for one added and on the tray as it left,
    /// its world id's find (was it on the tray at the arrival?).
    TrayEvidence {
        /// A clean find ([`SPECKLE_CLEAN`] or more and at least twice its second), in one of the
        /// unit's pairs or not.
        Found = "found",
        /// Checked, and no find of [`SPECKLE_CLEAN`] or more.
        NotFound = "not_found",
        /// No leaving answer, the stone not in it or not checked, or a find of
        /// [`SPECKLE_CLEAN`] or more that is not twice its second (look-alikes); an added stone
        /// removed again.
        Undecided = "undecided",
    }
}

/// A doubt held open for a look, or for the stone's next sight (a presence question, spec 4.6.1),
/// to settle (spec 4.6.4): its cause code (`pushed`, `unseen`, ...), why, `since` (the first grid
/// frame whose looks count for it) and where it opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayQuestion {
    /// The code of the doubt (the code of the unsure fact it ends in, when it ends undecided).
    pub cause: String,
    /// Why, as text.
    pub why: String,
    /// The first grid frame whose looks count for it.
    pub since: i64,
    /// Where it opened.
    pub opened: TrayPoint,
}

/// One reading the evidence leaves for an open stone (spec 4.6.8): `world` (a world stone, with
/// the tray stone linked to it, if any), `stone` (a tray stone), `gone` (with why:
/// `carried_away`, `emptied`, `dropped` or `hand`), `unseen` or `new`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TrayCandidate {
    /// `world`, `stone`, `gone`, `unseen` or `new`.
    pub kind: String,
    /// The world stone (`world`).
    pub world_id: Option<i64>,
    /// The tray stone (`stone`, and `world` when one stands for it).
    pub stone: Option<String>,
    /// Its look score in permille, when the last look run had one.
    pub score: Option<i64>,
    /// How it could be gone (`gone`).
    pub why: Option<String>,
}

impl TrayCandidate {
    /// The candidate as text: `world 95 0.248`, `stone 20261122T023847.0Z-1-87`, `gone (hand)`,
    /// `unseen` or `new`.
    pub fn describe(&self) -> String {
        let head = match self.kind.as_str() {
            "world" => format!("world {}", id_text(self.world_id)),
            "stone" => format!("stone {}", self.stone.as_deref().unwrap_or("?")),
            "gone" => return self.why.as_ref().map_or_else(|| "gone".to_string(), |w| format!("gone ({w})")),
            other => return other.to_string(),
        };
        match self.score {
            Some(p) => format!("{head} {}", milli(p)),
            None => head,
        }
    }
}

/// A stone the rules leave open when its tray leaves (spec 4.6.8): `unsure` (on the tray, not
/// sure), `removed` (removed unclear) or `added` (added unclear), with the cause, why the looks
/// could not settle it (`look`, with its `score` in permille), and every reading the evidence
/// leaves (`candidates`, ranked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayOpen {
    /// The tray stone.
    pub stone: String,
    /// The world stone it stands for (`None` once removed).
    pub world_id: Option<i64>,
    /// `unsure`, `removed` or `added`.
    pub kind: TrayFactKind,
    /// Why the rules leave it open (spec 4.6.8).
    pub cause: String,
    /// Why the looks could not settle it.
    pub look: String,
    /// The look score that came closest, in permille.
    pub score: Option<i64>,
    /// Added only: a hand worked next to it just before it was first seen.
    pub hand_near: Option<bool>,
    /// Added only: its world stone was in the world when the tray arrived.
    pub seen_at_arrival: Option<bool>,
    /// Every reading the evidence leaves, ranked.
    pub candidates: Vec<TrayCandidate>,
}

impl TrayOpen {
    /// The entry as one line: `stone 20261122T023847.0Z-1-91: unsure neighbour_pushed, look low
    /// 0.248: world 91 or world 95 0.248 or ... or gone`.
    pub fn describe(&self) -> String {
        let mut s = format!("stone {}: {} {}, look {}", self.stone, self.kind, self.cause, self.look);
        if let Some(p) = self.score {
            s += &format!(" {}", milli(p));
        }
        if self.hand_near == Some(true) {
            s += ", hand near";
        }
        if self.seen_at_arrival == Some(true) {
            s += ", seen at arrival";
        }
        let cands: Vec<String> = self.candidates.iter().map(TrayCandidate::describe).collect();
        s + ": " + &cands.join(" or ")
    }
}

/// A stone of a tray, with the SDK's own id: `{when first seen}-{tray}-{world id}`
/// (`20261122T023847.0Z-2-84`; without a clock `s{session}t{media seconds}-{tray}-{world id}`; a
/// second stone first seen on the tray in the same tenth of a second under the same world id takes
/// `-2` at the end, a third `-3`). An id is never given to two stones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayStone {
    /// Paper currently covers this tray stone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub under_sheet: bool,
    /// First calm observation of the current cover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet_since: Option<TrayPoint>,
    /// The tray stone's id.
    pub id: String,
    /// The world stone it stands for now (`None` while lost and once removed).
    pub world_id: Option<i64>,
    /// On, lost or removed.
    pub status: TrayStoneStatus,
    /// Its identity is sure now: false while a `question` is open, and once the stone is not sure
    /// (for good, unless the leaving's speckle answer makes it sure again, spec 4.6.13).
    pub sure: bool,
    /// Entered the account after arrival. A certain addition records a set-down. An unclear addition may be an
    /// arrival stone whose identity could not be recovered.
    pub added: bool,
    /// Whether its fact is certain: for a removed stone its removal's (also when it was added
    /// first), for an added stone that was not removed its addition's, `None` for the others. So
    /// an added and removed stone that was set down for certain can be `Some(false)`: count
    /// certain additions from the facts.
    pub certain: Option<bool>,
    /// When it was first seen on the tray.
    pub born: TrayPoint,
    /// Its place (x, y) as last known, in scene units (for a stone counted from the leaving point's
    /// world, boxed or kept there, as that world has it).
    pub place: [f64; 2],
    /// Its size (long side, short side) as last known, in scene units (for a stone counted from
    /// the leaving point's world, boxed or kept there, as that world has it).
    pub size: [f64; 2],
    /// The doubt a look, or the stone's next sight (a presence question, spec 4.6.1), may still
    /// settle (`None` when none).
    #[serde(default)]
    pub question: Option<TrayQuestion>,
    /// The grid frames of its anchor and latest look (with looks only).
    #[serde(default)]
    pub looks: Vec<i64>,
    /// A look decision linked it.
    #[serde(default)]
    pub by_look: bool,
    /// A look decision ended its link.
    #[serde(default)]
    pub look_lost: bool,
    /// Its open entry once the tray has left (`None` when it was settled).
    #[serde(default)]
    pub open: Option<TrayOpen>,
    /// Its place (x, y) when the tray stone was made: at the arrival for the stones the tray
    /// arrived with.
    #[serde(default)]
    pub first_place: [f64; 2],
    /// Its place (x, y) at the tray's leaving point, when it was seen there (`None` otherwise,
    /// and while the tray is on).
    #[serde(default)]
    pub left_place: Option<[f64; 2]>,
    /// The grid frame where something last touched it on this tray: a hand, a push on it or next
    /// to it, a link to another world stone (`None` when nothing did). The report reads these.
    #[serde(default)]
    pub touched: Option<i64>,
    /// The stone in the newest of the unit's speckle answers read while its tray was on (spec
    /// 4.6.12), `None` without one.
    #[serde(default)]
    pub speckle: Option<StoneSpeckle>,
    /// The leaving's speckle answer linked or confirmed it (spec 4.6.13).
    #[serde(default)]
    pub by_speckle: bool,
    /// What the leaving's speckle answer found of it at the other moment (spec 4.6.13), set as the
    /// tray leaves (`None` while the tray is on).
    #[serde(default)]
    pub evidence: Option<TrayEvidence>,
}

/// Something the interpreter says about a tray: arrived, unsure, identified, removed, added or
/// leaving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayFact {
    /// Sorted tray-stone IDs affected by a sheet action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tray_stones: Vec<String>,
    /// Stones leaving under paper.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under_sheet: Option<u64>,
    /// What it says.
    pub kind: TrayFactKind,
    /// The tray's number in its session, from 1.
    pub tray: i64,
    /// Session.
    pub session: i64,
    /// Source (video id).
    pub video: String,
    /// The tray stone's id (`None` for arrived and leaving).
    pub stone: Option<String>,
    /// The world id the stone stands for now (`None` when none).
    pub world_id: Option<i64>,
    /// For removed and added: the fact is certain. An unclear one is reported as such, never as
    /// a fact (`None` for the other kinds).
    pub certain: Option<bool>,
    /// The reason of unsure, identified, removed and added.
    pub why: Option<String>,
    /// Its code (`pushed`, `look_found`, `carried_away`, ...).
    #[serde(default)]
    pub code: Option<String>,
    /// Grid frame (the leaving facts carry the leaving point).
    pub g: i64,
    /// Media time, seconds.
    pub t: f64,
    /// Wall time, epoch seconds.
    pub wall: Option<f64>,
    /// Arrived: the tray stones at the arrival; leaving: those on the tray.
    pub stones: Option<u64>,
    /// Leaving: the tray stones still sure.
    pub sure: Option<u64>,
    /// Leaving: its certain additions.
    pub added: Option<u64>,
    /// Leaving: its certain removals.
    pub removed: Option<u64>,
    /// Leaving: its unclear facts.
    pub unclear: Option<u64>,
    /// Unsure: where the question it ends opened (`None` when it ends none).
    #[serde(default)]
    pub opened: Option<TrayPoint>,
    /// Identified: the world id the stone stood for before (`look_moved`, `speckle_moved`; `None`
    /// when it was lost, or confirmed on its own).
    #[serde(default)]
    pub from_world: Option<i64>,
    /// Identified: the look score, or the score of the clean find of the leaving's speckle answer,
    /// in permille.
    #[serde(default)]
    pub score: Option<i64>,
    /// Identified: its margin over the best scored rival, or over the find's second (`None`
    /// without one).
    #[serde(default)]
    pub margin: Option<i64>,
    /// Identified: the bar it cleared ([`SPECKLE_CLEAN`] for the leaving's speckle answer).
    #[serde(default)]
    pub bar: Option<i64>,
    /// Leaving: the tray's bar, in permille, when it took a look.
    #[serde(default)]
    pub t_id: Option<i64>,
    /// Leaving: the open account.
    #[serde(default)]
    pub open: Option<Vec<TrayOpen>>,
}

/// An optional id as text (`?` for none).
fn id_text(id: Option<i64>) -> String {
    id.map_or_else(|| "?".to_string(), |i| i.to_string())
}

/// An integer in permille with three decimals, from integers only: 853 is `0.853`, 1000 is
/// `1.000`, -5 is `-0.005`.
fn milli(p: i64) -> String {
    if p < 0 {
        return format!("-{}", milli(-p));
    }
    format!("{}.{:03}", p / 1000, p % 1000)
}

impl TrayFact {
    /// The fact's media time as `M:SS.d`.
    pub fn clock(&self) -> String {
        clock(self.t)
    }

    /// The fact as one line: `0:45.5 tray 1: stone 20261122T023847.0Z-1-117 was removed`.
    pub fn describe(&self) -> String {
        let head = format!("{} tray {}", clock(self.t), self.tray);
        let id = self.stone.as_deref().unwrap_or("?");
        let n = |v: Option<u64>| v.unwrap_or(0);
        let certain = self.certain == Some(true);
        let why = self.why.as_deref().unwrap_or("");
        match &self.kind {
            TrayFactKind::Arrived => format!("{head} arrived: {} stones", n(self.stones)),
            TrayFactKind::Sheet => {
                let action = if self.code.as_deref() == Some("sheet_laid") { "laid over" } else { "lifted off" };
                format!("{head}: a sheet was {action} {}", super::text::stones_text(&self.tray_stones))
            }
            TrayFactKind::Leaving => format!(
                "{head} leaving: {} stones ({} sure), {} added, {} removed, {} unclear{}",
                n(self.stones),
                n(self.sure),
                n(self.added),
                n(self.removed),
                n(self.unclear),
                self.under_sheet.filter(|v| *v > 0).map(|v| format!(", {v} under a sheet")).unwrap_or_default()
            ),
            TrayFactKind::Unsure => match self.opened {
                Some(o) => format!("{head}: stone {id} is not sure any more ({why}; open since {})", clock(o.t)),
                None => format!("{head}: stone {id} is not sure any more ({why})"),
            },
            TrayFactKind::Identified => {
                let (how, by) = match self.code.as_deref() {
                    Some("speckle_same") => ("confirmed", "speckle"),
                    Some("speckle_moved") => ("recognised", "speckle"),
                    Some("speckle_found") => ("found again", "speckle"),
                    Some("look_found") => ("found again", "look"),
                    _ => ("recognised", "look"),
                };
                let score = self.score.map_or_else(|| "-".to_string(), milli);
                format!("{head}: stone {id} {how} by its {by} on world stone {} ({score})", id_text(self.world_id))
            }
            TrayFactKind::Removed if !certain => format!("{head}: stone {id} not found (unclear)"),
            TrayFactKind::Removed if why == "carried away" => format!("{head}: stone {id} was removed (carried away)"),
            TrayFactKind::Removed => format!("{head}: stone {id} was removed"),
            _ if certain => format!("{head}: stone {id} was set down"),
            _ => format!("{head}: stone {id} not recognised (unclear)"),
        }
    }
}

impl std::fmt::Display for TrayFact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

/// The golden `open.txt` form (spec 4.6.9): every leaving fact's [`describe`](TrayFact::describe)
/// line (with ` (bar 0.xxx)` when the tray took a look), then its open entries indented two
/// spaces, `\n` after every line.
pub fn describe_open_txt(facts: &[TrayFact]) -> String {
    let mut out = String::new();
    for f in facts.iter().filter(|f| f.kind == TrayFactKind::Leaving) {
        out += &f.describe();
        if let Some(b) = f.t_id {
            out += &format!(" (bar {})", milli(b));
        }
        out.push('\n');
        for o in f.open.iter().flatten() {
            out += &format!("  {}\n", o.describe());
        }
    }
    out
}

/// A tray: `arrived` (it has arrived and settled; `None` until then) and `leaving` (just before it
/// moved out; `None` while it is on, and for a tray that never arrived), its stones in tray order
/// and every fact about it. `open` is the open account of its leaving (`None` until it has left)
/// and `t_id` its bar by look now (permille; `None` while it has taken no look).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tray {
    /// Session.
    pub session: i64,
    /// Source (video id).
    pub video: String,
    /// The tray's number in its session, from 1.
    pub n: i64,
    /// Where it had arrived and settled.
    pub arrived: Option<TrayPoint>,
    /// Just before it moved out.
    pub leaving: Option<TrayPoint>,
    /// Its tray stones, in tray order.
    pub stones: Vec<TrayStone>,
    /// Every fact about it, in order.
    pub facts: Vec<TrayFact>,
    /// The open account of its leaving.
    #[serde(default)]
    pub open: Option<Vec<TrayOpen>>,
    /// Its bar by look, in permille.
    #[serde(default)]
    pub t_id: Option<i64>,
}

impl Tray {
    /// The media span from the arrival to the leaving point, the span to replay or to show for
    /// this tray; `None` until it has left.
    pub fn window(&self) -> Option<(f64, f64)> {
        Some((self.arrived?.t, self.leaving?.t))
    }

    /// The tray stone with that id.
    pub fn stone(&self, id: &str) -> Option<&TrayStone> {
        self.stones.iter().find(|s| s.id == id)
    }

    /// The self-check of a tray that has left (spec 4.6.10): the codes (`C1` ... `C9`) it finds
    /// broken in the tray's facts and final tray stones, sorted; empty when the account is
    /// consistent. A tray that never arrived passes; one that has not left yet breaks C1 (it has
    /// no leaving fact).
    pub fn check(&self) -> Vec<String> {
        check_tray(self)
    }

    /// The tray's report (spec 4.7): which stones were removed, added and moved, and whether that
    /// is its whole account. `None` until the tray has left, and for a tray that never arrived.
    pub fn report(&self) -> Option<TrayReport> {
        tray_report(self)
    }
}

/// A stone the tray arrived with, that the SDK is sure of, saw at the leaving point and that
/// something touched, moved when, once the tray's own shift is taken out, its place there is more
/// than this many times its short side from where it was first seen (spec 4.7).
pub const MOVED: f64 = 0.75;

wire_enum! {
    /// What a [`TrayChange`] says.
    TrayChangeKind {
        /// The stone was removed.
        Removed = "removed",
        /// The stone was added.
        Added = "added",
        /// The stone moved on the tray.
        Moved = "moved",
    }
}

/// The confidence of a removal or an addition (spec 4.6.13): its kind, whether it is certain and
/// the stone's [`TrayStone::evidence`], and the probability that the change happened,
/// `(right + 1) / (checked + 2)` over the four recordings' changes of that class checked by the
/// maintainers' offline look tracker (each stone followed by its look both ways) and camera check.
/// A removal or an addition is supported by `not_found` and contradicted by `found`; a certain
/// addition takes the unclear additions' numbers.
pub const CONFIDENCE: [(TrayChangeKind, bool, TrayEvidence, f64); 12] = [
    (TrayChangeKind::Removed, true, TrayEvidence::NotFound, 0.98),
    (TrayChangeKind::Removed, true, TrayEvidence::Undecided, 0.50),
    (TrayChangeKind::Removed, true, TrayEvidence::Found, 0.25),
    (TrayChangeKind::Removed, false, TrayEvidence::NotFound, 0.71),
    (TrayChangeKind::Removed, false, TrayEvidence::Undecided, 0.58),
    (TrayChangeKind::Removed, false, TrayEvidence::Found, 0.09),
    (TrayChangeKind::Added, true, TrayEvidence::NotFound, 0.96),
    (TrayChangeKind::Added, true, TrayEvidence::Undecided, 0.52),
    (TrayChangeKind::Added, true, TrayEvidence::Found, 0.02),
    (TrayChangeKind::Added, false, TrayEvidence::NotFound, 0.96),
    (TrayChangeKind::Added, false, TrayEvidence::Undecided, 0.52),
    (TrayChangeKind::Added, false, TrayEvidence::Found, 0.02),
];

/// The confidence of a move (spec 4.6.13): 19 of 21 paired moves right.
pub const MOVED_CONFIDENCE: f64 = 0.87;

/// The confidence of a removal or an addition (spec 4.6.13), from [`CONFIDENCE`]: a stone without
/// evidence, or with one this version does not know, counts as undecided.
fn confidence(kind: &TrayChangeKind, certain: bool, evidence: Option<&TrayEvidence>) -> f64 {
    let evidence = match evidence {
        Some(e @ (TrayEvidence::Found | TrayEvidence::NotFound)) => e.clone(),
        _ => TrayEvidence::Undecided,
    };
    CONFIDENCE.iter().find(|(k, c, e, _)| k == kind && *c == certain && *e == evidence).map_or(0.0, |row| row.3)
}

/// One stone of a tray report (spec 4.7). For removed and added, `at`, `certain` and `code` are
/// its fact's (`carried_away`, `missing`, `not_found`; `set_down`, `not_recognised`); for moved,
/// `at` and `code` are `None` and `certain` is true: only a stone the SDK is sure of, seen at the
/// leaving point and touched on the tray is reported moved (one that is not sure may be another
/// stone at that place). `confidence` is the probability that the change happened (spec 4.6.13),
/// for a threshold of your own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrayChange {
    /// Removed, added or moved.
    pub kind: TrayChangeKind,
    /// The tray stone's id.
    pub stone: String,
    /// Its fact's point (`None` for moved).
    pub at: Option<TrayPoint>,
    /// Its fact is certain; always true for moved.
    pub certain: bool,
    /// Its fact's code (`None` for moved).
    pub code: Option<String>,
    /// Its final place (x, y) on the tray, in the world's scene units (x right and y up as the
    /// camera sees the tray), for matching a report with another system that looks at the trays:
    /// for a removal, where the stone was last seen before it was taken; for an addition or a
    /// move, its place as the tray left (its `left_place` when it was seen at the leaving point,
    /// else where it was last seen or, for a stone counted because the leaving point's world kept
    /// it, its place in that world).
    pub place: [f64; 2],
    /// Its size (long side, short side) as last known, in scene units (for a stone counted from
    /// the leaving point's world, boxed or kept there, as that world has it).
    pub size: [f64; 2],
    /// How far a moved stone moved once the tray's own shift is taken out (`None` for the
    /// others).
    pub distance: Option<f64>,
    /// The probability that the change happened (spec 4.6.13): [`CONFIDENCE`] by its kind,
    /// whether it is certain and the stone's [`TrayStone::evidence`]; [`MOVED_CONFIDENCE`] for a
    /// move.
    #[serde(default)]
    pub confidence: f64,
}

impl TrayChange {
    /// One line: for removed and added, the clock of its fact, the kind, the stone and its final
    /// place (`from` it for a removal, `at` it for an addition), then its confidence, ` (0.98)`, or
    /// ` (unclear, 0.71)` when it is not certain (`0:45.5 removed 20261122T023847.0Z-1-117 from
    /// (0.68, -1.06) (0.98)`); for moved, `moved`, the stone, the distance, `to` its place and its
    /// confidence. A coordinate has two decimals, and one that rounds to zero has no sign.
    pub fn describe(&self) -> String {
        let coord = |v: f64| {
            let s = fmt(v, 2);
            if s == "-0.00" { "0.00".to_string() } else { s }
        };
        let place = format!("({}, {})", coord(self.place[0]), coord(self.place[1]));
        let confidence = fmt(self.confidence, 2);
        if self.kind == TrayChangeKind::Moved {
            let distance = fmt(self.distance.unwrap_or(0.0), 2);
            return format!("moved {} {distance} to {place} ({confidence})", self.stone);
        }
        let at = self.at.map(|p| format!("{} ", clock(p.t))).unwrap_or_default();
        let prep = if self.kind == TrayChangeKind::Removed { "from" } else { "at" };
        let unclear = if self.certain { "" } else { "unclear, " };
        format!("{at}{} {} {prep} {place} ({unclear}{confidence})", self.kind, self.stone)
    }
}

/// A tray's report, the SDK's simple mode (spec 4.7): on tray `tray`, from `arrived` to `left`,
/// which stones were `removed`, `added` and `moved`. `complete` says that is the tray's whole
/// account: every change is certain and no stone is open. Otherwise `unclear` counts the changes
/// that are not certain and `unsure` the stones on the tray as it left whose identity is not sure
/// (those added unclear aside); the tray's facts and its open account ([`Tray::open`]) say why,
/// stone by stone. `arrived.wall` and `left.wall` are the unit's wall clock (UTC epoch seconds)
/// when it is known.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrayReport {
    /// Sorted tray IDs still under paper at departure.
    #[serde(default)]
    pub under_sheet: Vec<String>,
    /// Sheet lays observed on this tray.
    #[serde(default)]
    pub sheets: u64,
    /// The sheet actions in observation order.
    #[serde(default)]
    pub sheet_facts: Vec<TrayFact>,
    /// Session.
    pub session: i64,
    /// Source (video id).
    pub video: String,
    /// The tray's number in its session, from 1.
    pub tray: i64,
    /// Where it had arrived and settled.
    pub arrived: TrayPoint,
    /// Its leaving point.
    pub left: TrayPoint,
    /// How many stones it arrived with.
    pub stones: u64,
    /// A change for each removal, in order.
    pub removed: Vec<TrayChange>,
    /// Additions in order. A certain entry records a set-down. An unclear entry has an unresolved origin.
    pub added: Vec<TrayChange>,
    /// A change for each stone the tray arrived with that moved on it, in tray order.
    pub moved: Vec<TrayChange>,
    /// Removals and additions that are not certain.
    pub unclear: u64,
    /// Stones on the tray as it left whose identity is not sure (those added unclear aside).
    pub unsure: u64,
    /// Every change is certain and no stone is open.
    pub complete: bool,
}

impl TrayReport {
    /// `tray 1 (2026-11-22T02:38:47.0Z), 0:22.5 to 2:24.0: 11 removed, 13 added, 8 moved, not
    /// complete: 18 unclear, 1 not sure`, then a line for each change, indented two spaces:
    /// removed, then added, then moved.
    pub fn describe(&self) -> String {
        let wall = self.arrived.wall.map(|w| format!(" ({})", iso(w))).unwrap_or_default();
        let mut head = format!(
            "tray {}{wall}, {} to {}: {} removed, {} added, {} moved",
            self.tray,
            clock(self.arrived.t),
            clock(self.left.t),
            self.removed.len(),
            self.added.len(),
            self.moved.len()
        );
        if !self.under_sheet.is_empty() {
            head += &format!(", {} under a sheet", self.under_sheet.len());
        }
        if !self.complete {
            let mut parts = Vec::new();
            if self.unclear > 0 {
                parts.push(format!("{} unclear", self.unclear));
            }
            if self.unsure > 0 {
                parts.push(format!("{} not sure", self.unsure));
            }
            head += &format!(", not complete: {}", parts.join(", "));
        }
        let lines = self.removed.iter().chain(&self.added).chain(&self.moved).map(|c| format!("\n  {}", c.describe()));
        let sheets = self.sheet_facts.iter().map(|f| format!("\n  {}", f.describe()));
        head + &lines.chain(sheets).collect::<String>()
    }
}

fn median(xs: &[f64]) -> f64 {
    let mut v = xs.to_vec();
    v.sort_by(f64::total_cmp);
    let k = v.len() / 2;
    if v.len() % 2 == 1 { v[k] } else { (v[k - 1] + v[k]) / 2.0 }
}

/// The report of a tray that has left (spec 4.7), from its facts and its final tray stones;
/// `None` for any other.
fn tray_report(tray: &Tray) -> Option<TrayReport> {
    tray.arrived?;
    tray.leaving?;
    let arrival = tray.facts.first()?;
    let leaving = tray.facts.last().filter(|x| x.kind == TrayFactKind::Leaving)?;
    let change = |kind: TrayChangeKind, want: TrayFactKind| -> Vec<TrayChange> {
        tray.facts
            .iter()
            .filter(|x| x.kind == want)
            .filter_map(|x| {
                // the last stone with the id, as the reference reads it
                let id = x.stone.as_deref()?;
                let s = tray.stones.iter().rev().find(|s| s.id == id)?;
                // its final place: where it was last seen before it was taken, or as the tray left
                let place = if kind == TrayChangeKind::Removed { s.place } else { s.left_place.unwrap_or(s.place) };
                let certain = x.certain == Some(true);
                Some(TrayChange {
                    kind: kind.clone(),
                    stone: s.id.clone(),
                    at: Some(TimeRef { g: x.g, t: x.t, wall: x.wall }),
                    certain,
                    code: x.code.clone(),
                    place,
                    size: s.size,
                    distance: None,
                    confidence: confidence(&kind, certain, s.evidence.as_ref()),
                })
            })
            .collect()
    };
    // moved: only stones the SDK is sure of (a stone not sure may be another stone at that place: it
    // counts as not sure), each at its place at the leaving point, where it was seen (a place seen
    // earlier misses the tray's shift), and only one that something touched (a stone nothing
    // touched does not move: an outline the world refines does)
    let kept: Vec<(&TrayStone, [f64; 2])> = tray
        .stones
        .iter()
        .filter(|s| !s.added && s.status == TrayStoneStatus::On && s.sure)
        .filter_map(|s| Some((s, s.left_place?)))
        .collect();
    let mut moved = Vec::new();
    if !kept.is_empty() {
        // the tray's own shift: the median over the half of them that moved least (ties in tray order)
        let mut order: Vec<(f64, usize)> =
            kept.iter().enumerate().map(|(i, (s, p))| (dist(s.first_place, *p), i)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let least: Vec<(&TrayStone, [f64; 2])> =
            order[..kept.len().div_ceil(2)].iter().map(|&(_, i)| kept[i]).collect();
        let mx = median(&least.iter().map(|(s, p)| p[0] - s.first_place[0]).collect::<Vec<_>>());
        let my = median(&least.iter().map(|(s, p)| p[1] - s.first_place[1]).collect::<Vec<_>>());
        for &(s, p) in kept.iter().filter(|(s, _)| s.touched.is_some()) {
            let d = dist([s.first_place[0] + mx, s.first_place[1] + my], p);
            if d > MOVED * s.size[1] {
                moved.push(TrayChange {
                    kind: TrayChangeKind::Moved,
                    stone: s.id.clone(),
                    at: None,
                    certain: true,
                    code: None,
                    place: p,
                    size: s.size,
                    distance: Some(round3(d)),
                    confidence: MOVED_CONFIDENCE,
                });
            }
        }
    }
    let unsure = tray
        .stones
        .iter()
        .filter(|s| s.status == TrayStoneStatus::On && !s.sure && !(s.added && s.certain == Some(false)))
        .count() as u64;
    let unclear = leaving.unclear.unwrap_or(0);
    Some(TrayReport {
        session: tray.session,
        video: tray.video.clone(),
        tray: tray.n,
        arrived: TimeRef { g: arrival.g, t: arrival.t, wall: arrival.wall },
        left: TimeRef { g: leaving.g, t: leaving.t, wall: leaving.wall },
        stones: arrival.stones.unwrap_or(0),
        removed: change(TrayChangeKind::Removed, TrayFactKind::Removed),
        added: change(TrayChangeKind::Added, TrayFactKind::Added),
        moved,
        unclear,
        unsure,
        complete: unclear == 0 && leaving.sure == leaving.stones,
        under_sheet: tray
            .stones
            .iter()
            .filter(|s| s.status == TrayStoneStatus::On && s.under_sheet)
            .map(|s| s.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        sheets: tray.facts.iter().filter(|f| f.code.as_deref() == Some("sheet_laid")).count() as u64,
        sheet_facts: tray.facts.iter().filter(|f| f.kind == TrayFactKind::Sheet).cloned().collect(),
    })
}

/// The trays counts of the summary: trays opened and arrived, tray stones at the arrivals,
/// certain and unclear facts, and tray stones that stopped being sure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct TrayCounts {
    /// Newly laid sheets.
    pub sheets: u64,
    /// Stones leaving under paper.
    pub under_sheet: u64,
    /// Trays (static stops of at least 2 s).
    pub total: u64,
    /// Trays that arrived and settled.
    pub arrived: u64,
    /// Tray stones at the arrivals.
    pub stones: u64,
    /// Certain additions.
    pub added: u64,
    /// Unclear additions.
    pub added_unclear: u64,
    /// Certain removals.
    pub removed: u64,
    /// Unclear removals.
    pub removed_unclear: u64,
    /// Tray stones that stopped being sure.
    pub unsure: u64,
}

// ---- the self-check (spec 4.6.10) -------------------------------------------------------------

/// On the tray and sure, or removed for certain: what the account settled.
fn settled(s: &TrayStone) -> bool {
    (s.status == TrayStoneStatus::On && s.sure) || (s.status == TrayStoneStatus::Removed && s.certain == Some(true))
}

/// An `identified` fact of the leaving's speckle answer (a `speckle_*` code, spec 4.6.13).
fn speckle_fact(x: &TrayFact) -> bool {
    x.kind == TrayFactKind::Identified && x.code.as_deref().is_some_and(|c| SPECKLE_CODES.contains(&c))
}

fn check_tray(t: &Tray) -> Vec<String> {
    use TrayFactKind::{Added, Arrived, Identified, Leaving, Removed, Unsure};
    let mut bad: BTreeSet<&'static str> = BTreeSet::new();
    let facts = &t.facts;
    if t.arrived.is_none() && facts.is_empty() && t.stones.is_empty() {
        return Vec::new();
    }
    let ends = facts.first().is_some_and(|f| f.kind == Arrived) && facts.last().is_some_and(|f| f.kind == Leaving);
    if !ends || facts[..facts.len().saturating_sub(1)].iter().any(|f| f.kind == Leaving) {
        bad.insert("C1");
    }
    let by_id: BTreeMap<&str, &TrayStone> = t.stones.iter().map(|s| (s.id.as_str(), s)).collect();
    if by_id.len() != t.stones.len() {
        bad.insert("C2");
    }
    let mut life: BTreeMap<&str, Vec<&TrayFact>> = BTreeMap::new();
    for x in facts {
        let Some(id) = x.stone.as_deref() else { continue };
        if !by_id.contains_key(id) {
            bad.insert("C2");
        }
        life.entry(id).or_default().push(x);
    }
    for xs in life.values() {
        let kinds: Vec<&TrayFactKind> = xs.iter().map(|x| &x.kind).collect();
        let count = |k: TrayFactKind| kinds.iter().filter(|x| ***x == k).count();
        if kinds[1..].contains(&&Added) || count(Unsure) > 1 || count(Removed) > 1 {
            bad.insert("C3");
        }
        for (a, x) in xs.iter().enumerate() {
            let (before, after) = (&xs[..a], &xs[a + 1..]);
            let wrong = match x.kind {
                // after unsure only a removal, or an identified of the leaving's speckle (spec 4.6.13)
                Unsure => after.iter().any(|y| y.kind != Removed && !speckle_fact(y)),
                Removed => !after.is_empty(),
                // never after a removal, and at most one of the leaving's speckle; one of a look never
                // after unsure
                Identified if speckle_fact(x) => before.iter().any(|y| y.kind == Removed || speckle_fact(y)),
                Identified => before.iter().any(|y| y.kind == Unsure || y.kind == Removed),
                _ => false,
            };
            if wrong {
                bad.insert("C3");
            }
        }
    }
    let mut on: Vec<&TrayStone> = Vec::new();
    for s in &t.stones {
        let xs = life.get(s.id.as_str()).map_or(&[][..], Vec::as_slice);
        let rem: Vec<&&TrayFact> = xs.iter().filter(|x| x.kind == Removed).collect();
        // an unsure fact counts after the stone's identified of the leaving's speckle (spec 4.6.13)
        let since = xs.iter().rposition(|x| speckle_fact(x)).map_or(0, |a| a + 1);
        let unsure = xs[since..].iter().any(|x| x.kind == Unsure);
        let removed = s.status == TrayStoneStatus::Removed;
        if removed == rem.is_empty() || rem.first().is_some_and(|x| x.certain != s.certain) {
            bad.insert("C4");
        }
        let is_on = s.status == TrayStoneStatus::On;
        if (is_on && s.sure == (unsure || (s.added && s.certain != Some(true)))) || s.question.is_some() {
            bad.insert("C4");
        }
        if is_on {
            on.push(s);
        }
    }
    if facts.first().is_some_and(|f| f.kind == Arrived) {
        let n = t.stones.iter().filter(|s| !s.added).count() as u64;
        if facts[0].stones != Some(n) {
            bad.insert("C5");
        }
    }
    if let Some(leave) = facts.last().filter(|f| f.kind == Leaving) {
        check_leaving(t, leave, &on, &by_id, &mut bad);
    }
    let lifted = check_sheets(t, &by_id, &mut bad);
    let mut worlds = BTreeSet::new();
    for s in &on {
        if !s.world_id.map_or(!s.sure && !s.under_sheet && lifted.contains(s.id.as_str()), |w| worlds.insert(w)) {
            bad.insert("C6");
        }
    }
    for x in facts.iter().filter(|x| x.kind == Identified) {
        // one of the leaving's speckle clears SPECKLE_CLEAN (spec 4.6.13); one of a look a bar of at
        // least T_ID_MIN, by at least M_ID over its rivals
        let cleared = if speckle_fact(x) {
            matches!((x.score, x.bar), (Some(s), Some(b)) if s >= b && b == SPECKLE_CLEAN)
        } else {
            matches!((x.score, x.bar), (Some(s), Some(b)) if s >= b && b >= T_ID_MIN)
                && x.margin.is_none_or(|m| m >= M_ID)
        };
        if !cleared || x.certain.is_some() {
            bad.insert("C9");
        }
    }
    bad.into_iter().map(String::from).collect()
}

/// Checks the leaving fact: its counts and open list (C5), its entries (C7) and their candidates
/// (C8).
fn check_leaving(
    t: &Tray,
    leave: &TrayFact,
    on: &[&TrayStone],
    by_id: &BTreeMap<&str, &TrayStone>,
    bad: &mut BTreeSet<&'static str>,
) {
    use TrayFactKind::{Added, Removed, Unsure};
    let count = |n: usize| Some(n as u64);
    let sure = on.iter().filter(|s| s.sure).count();
    let certain = |kind: &TrayFactKind| t.facts.iter().filter(|x| &x.kind == kind && x.certain == Some(true)).count();
    let (added, removed) = (certain(&Added), certain(&Removed));
    let unclear = t.facts.iter().filter(|x| (x.kind == Removed || x.kind == Added) && x.certain != Some(true)).count();
    if (leave.stones, leave.sure, leave.added, leave.removed, leave.unclear)
        != (count(on.len()), count(sure), count(added), count(removed), count(unclear))
    {
        bad.insert("C5");
    }
    let entries = leave.open.as_deref().unwrap_or(&[]);
    // one entry per open stone, in tray order
    let open: Vec<&str> = entries.iter().map(|o| o.stone.as_str()).collect();
    let want: Vec<&str> = t.stones.iter().filter(|s| !settled(s)).map(|s| s.id.as_str()).collect();
    if open != want {
        bad.insert("C5");
    }
    let sure_world: BTreeSet<Option<i64>> = on.iter().filter(|s| s.sure).map(|s| s.world_id).collect();
    for o in entries {
        let c = &o.candidates;
        let keys: BTreeSet<(&str, Option<i64>, Option<&str>)> =
            c.iter().map(|x| (x.kind.as_str(), x.world_id, x.stone.as_deref())).collect();
        let causes: &[&str] = match o.kind {
            Unsure => &UNSURE_CODES,
            Removed => &REMOVED_CAUSES,
            _ => &ADDED_CAUSES,
        };
        let looks: &[&str] = match o.kind {
            Unsure | Removed => &STONE_LOOKS,
            Added => &ADDED_LOOKS,
            _ => &[],
        };
        if !causes.contains(&o.cause.as_str())
            || !looks.contains(&o.look.as_str())
            || c.is_empty()
            || keys.len() != c.len()
        {
            bad.insert("C7");
        }
        if c.is_empty() {
            continue; // the checks below read the first and last candidate; C8 has nothing to check
        }
        let last = c[c.len() - 1].kind.as_str();
        if (o.kind == Unsure
            && (c[0].kind != if o.world_id.is_some() { "world" } else { "unseen" } || c[0].world_id != o.world_id))
            || (o.kind != Added && !c.iter().any(|x| x.kind == "gone"))
            || (o.kind == Removed && last != "unseen")
            || (o.kind == Added && last != "new")
        {
            bad.insert("C7");
        }
        for x in c {
            let stone = x.stone.as_deref().filter(|_| x.kind == "stone").and_then(|id| by_id.get(id));
            if stone.is_some_and(|z| settled(z)) || (x.kind == "world" && sure_world.contains(&x.world_id)) {
                bad.insert("C8");
            }
        }
    }
}

// ---- geometry ----------------------------------------------------------------------------------

type P = [f64; 2];

fn dist(a: P, b: P) -> f64 {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    (dx * dx + dy * dy).sqrt()
}

fn compatible(a: P, b: P) -> bool {
    (a[0] - b[0]).abs() <= SIZE_TOL * a[0].max(b[0]) && (a[1] - b[1]).abs() <= SIZE_TOL * a[1].max(b[1])
}

/// Nearest first, then by key.
fn by_near<K: Ord>(a: &(f64, K), b: &(f64, K)) -> std::cmp::Ordering {
    a.0.total_cmp(&b.0).then(a.1.cmp(&b.1))
}

/// A world stone as the rules see it.
#[derive(Debug, Clone)]
struct Ws {
    under_sheet: bool,
    mode: StoneMode,
    outline: Option<[f64; 4]>,
    id: i64,
    held: bool,
    frozen: bool,
    detected: bool,
    pos: P,
    /// The quaternion's z and w.
    zw: [f64; 2],
    dims: P,
    /// The cells of its look, when the tray rules use it (spec 4.6.3).
    look: Option<Arc<Cells>>,
}

impl Ws {
    /// A world stone with a usable solid: at least 2 numbers in pos and dims and 4 in q.
    fn of(s: &StoneView) -> Option<Ws> {
        let solid = s.solid.as_ref().filter(|o| o.pos.len() >= 2 && o.dims.len() >= 2 && o.q.len() >= 4)?;
        Some(Ws {
            id: s.id,
            under_sheet: s.under_sheet,
            mode: s.mode.clone(),
            outline: sheets::extent(&s.outline),
            held: matches!(s.mode, StoneMode::Carry | StoneMode::Hinge),
            frozen: s.shape == StoneShape::Frozen,
            detected: s.detected,
            pos: [solid.pos[0], solid.pos[1]],
            zw: [solid.q[2], solid.q[3]],
            dims: [solid.dims[0], solid.dims[1]],
            look: s.look.as_ref().and_then(tray_cells).map(Arc::new),
        })
    }

    fn size(&self) -> P {
        [self.dims[0].max(self.dims[1]), self.dims[0].min(self.dims[1])]
    }

    /// The cosine and sine of the stone's turn about the vertical, from its quaternion, without
    /// trigonometry.
    fn turn(&self) -> (f64, f64) {
        let [z, w] = self.zw;
        let n = w * w + z * z;
        if n == 0.0 {
            return (1.0, 0.0);
        }
        ((w * w - z * z) / n, (2.0 * z * w) / n)
    }

    /// The stone's box, grown by `m`, covers the point `p`.
    fn covers(&self, p: P, m: f64) -> bool {
        let (c, s) = self.turn();
        let (dx, dy) = (p[0] - self.pos[0], p[1] - self.pos[1]);
        (c * dx + s * dy).abs() <= self.dims[0] / 2.0 + m && (-s * dx + c * dy).abs() <= self.dims[1] / 2.0 + m
    }

    /// The largest share of this stone's box (5 x 5 points) that one other stone's box covers.
    fn overlap_share(&self, world: &[&Ws]) -> f64 {
        let (c, s) = self.turn();
        let d = self.dims;
        let [x0, y0] = self.pos;
        let mut pts = [[0.0; 2]; 25];
        for i in 0..5 {
            for j in 0..5 {
                let u = (i as f64 + 0.5) / 5.0 * d[0] - d[0] / 2.0;
                let v = (j as f64 + 0.5) / 5.0 * d[1] - d[1] / 2.0;
                pts[i * 5 + j] = [x0 + c * u - s * v, y0 + s * u + c * v];
            }
        }
        let reach = d[0].max(d[1]);
        let mut best = 0;
        for o in world {
            if o.under_sheet || o.id == self.id || dist(o.pos, self.pos) > (reach + o.dims[0].max(o.dims[1])) / 2.0 {
                continue;
            }
            best = best.max(pts.iter().filter(|p| o.covers(**p, 0.0)).count());
        }
        best as f64 / 25.0
    }
}

fn inside_board(b: Option<&Rect>, p: P) -> bool {
    let Some(b) = b else { return true };
    let (c, s) = (b.theta.cos(), b.theta.sin());
    let (dx, dy) = (p[0] - b.cx, p[1] - b.cy);
    (c * dx + s * dy).abs() <= b.hw * 1.05 + 0.02 && (-s * dx + c * dy).abs() <= b.hh * 1.05 + 0.02
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// When a stone was first seen: the UTC wall time to the tenth (`20261122T023847.0Z`), else the
/// session and the media seconds (`s1795314878t22.5`: world ids start again in a new session, and
/// a seek can show the same time again).
pub(crate) fn stamp(wall: Option<f64>, t: f64, session: i64) -> String {
    let Some(wall) = wall else { return format!("s{session}t{}", fmt(t, 1)) };
    let n = (wall * 10.0 + 0.5).floor() as i64;
    let (secs, tenth) = (n.div_euclid(10), n.rem_euclid(10));
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let r = secs.rem_euclid(86_400);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}.{tenth}Z", r / 3600, r % 3600 / 60, r % 60)
}

/// A wall time (UTC epoch seconds) to the tenth of a second, rounded as `stamp()` rounds it:
/// `2026-11-22T02:38:47.0Z`.
pub fn iso(wall: f64) -> String {
    let n = (wall * 10.0 + 0.5).floor() as i64;
    let (secs, tenth) = (n.div_euclid(10), n.rem_euclid(10));
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let r = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{tenth}Z", r / 3600, r % 3600 / 60, r % 60)
}

// ---- the rules ---------------------------------------------------------------------------------

/// A hand touched the lost stone's link from its last calm moment (or RENAME_T before it was last
/// seen, if that is earlier) up to its loss: a stone does not leave a tray by itself (spec 4.6.1,
/// certain removals).
fn hand_took(s: &St) -> bool {
    s.hand_t.is_some_and(|h| h >= s.lost_calm_t.min(s.seen_t - RENAME_T))
}

/// A world event as the rules see it.
#[derive(Debug, Clone)]
struct Ev {
    why: Option<String>,
    ids: Vec<i64>,
    kind: EventKind,
    id: Option<i64>,
    /// It has a side.
    side: bool,
    /// A delete because a hand emptied the place.
    by_hand: bool,
    /// A delete because a hand emptied the place or carried the stone away (why `hand` or
    /// `carried-away`): a busy frame (spec 4.6.1, presence questions).
    took: bool,
}

impl Ev {
    /// A hand event: lift, hinge, carry, place, drop, gone, or a push with a side.
    fn hand(&self) -> bool {
        matches!(
            self.kind,
            EventKind::Lift
                | EventKind::Hinge
                | EventKind::Carry
                | EventKind::Place
                | EventKind::Drop
                | EventKind::Gone
        ) || (self.kind == EventKind::Push && self.side)
    }

    /// After this event a world stone's stored look no longer describes it (spec 4.6.3).
    fn clears_look(&self) -> bool {
        matches!(
            self.kind,
            EventKind::Push
                | EventKind::Lift
                | EventKind::Hinge
                | EventKind::Carry
                | EventKind::Place
                | EventKind::Drop
                | EventKind::Gone
                | EventKind::Delete
                | EventKind::Split
        )
    }
}

/// A frame's world as the rules see it: the stones with a usable solid, and of a world id the
/// frame has twice only its first stone.
#[derive(Debug, Clone)]
struct TrayWorld {
    r: TimeRef,
    stones: Vec<Ws>,
    /// The index of each id's stone.
    by_id: BTreeMap<i64, usize>,
    events: Vec<Ev>,
    /// Both hands free.
    free: bool,
    /// A hand is flagged `untrusted`.
    untrusted: bool,
    /// Every stone confirmed (`stats.confirmed == stats.stones`).
    confirmed: bool,
    board: Option<Rect>,
}

impl TrayWorld {
    fn new(w: &WorldLayer, r: TimeRef) -> TrayWorld {
        let mut seen = BTreeSet::new();
        let stones: Vec<Ws> = w
            .stones
            .iter()
            .filter_map(|s| {
                let first = seen.insert(s.id);
                Ws::of(s).filter(|_| first)
            })
            .collect();
        let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let events = w
            .events
            .iter()
            .map(|e| Ev {
                kind: e.kind.clone(),
                id: e.id,
                side: e.side.is_some(),
                why: e.why.clone(),
                ids: e.ids.clone().unwrap_or_default(),
                by_hand: e.kind == EventKind::Delete && e.why.as_deref() == Some("hand"),
                took: e.kind == EventKind::Delete && matches!(e.why.as_deref(), Some("hand" | "carried-away")),
            })
            .collect();
        TrayWorld {
            r,
            stones,
            by_id,
            events,
            free: w.hands.left.state == HandStateKind::Free && w.hands.right.state == HandStateKind::Free,
            untrusted: w.hands.left.untrusted == Some(true) || w.hands.right.untrusted == Some(true),
            confirmed: w.stats.confirmed == w.stats.stones,
            board: w.board().cloned(),
        }
    }

    fn observed_stone(&self, s: &St) -> Option<&Ws> {
        self.get(s.world?).filter(|w| s.on() && !w.under_sheet && w.detected && !w.held)
    }

    /// Its stones by id, held or not, detected or kept: a point's world as the leaving reads it.
    fn stones_by_id(&self) -> BTreeMap<i64, Ws> {
        self.stones.iter().map(|w| (w.id, w.clone())).collect()
    }

    fn get(&self, id: i64) -> Option<&Ws> {
        self.by_id.get(&id).map(|&i| &self.stones[i])
    }

    /// At either end of an omission, hand work may have changed which stone occupies a place.
    fn undisturbed(&self) -> bool {
        self.free
            && !self.untrusted
            && !self.stones.iter().any(|w| w.held)
            && !self.events.iter().any(|e| e.hand() || e.kind == EventKind::Push || e.took)
    }

    /// Other detected stones that could occupy the missing world's last observed place.
    fn omission_neighbours(&self, id: i64, place: P, size: P, include_kept: bool) -> BTreeSet<i64> {
        self.stones
            .iter()
            .filter(|w| {
                w.id != id
                    && !w.under_sheet
                    && (w.detected || include_kept)
                    && !w.held
                    && compatible(w.size(), size)
                    && dist(w.pos, place) <= RENAME_D
            })
            .map(|w| w.id)
            .collect()
    }
}

/// Evidence immediately before a short gap in a world id. A parked unidentified stone
/// stays out of matching and settling until a compatible detection brings it back.
#[derive(Debug, Clone)]
struct MissingWorld {
    first: TimeRef,
    place: P,
    size: P,
    neighbours: BTreeSet<i64>,
    /// Real positions before the omission, for rejecting swapped or equidistant returns.
    other_stones: Vec<(P, P)>,
    unidentified: Option<Unid>,
}

/// A positive sight beside a tray's leaving point. Its stone's origin is still unknown.
#[derive(Debug, Clone)]
struct BoundarySight {
    anchor_g: i64,
    t: f64,
    stone: Ws,
    latest: Ws,
    neighbours: BTreeSet<i64>,
}

/// A world stone no tray stone stands for (yet). `freed`: a look decision unlinked it; `resized`: a
/// size change of a linked world stone made it (neither is ever added for certain).
#[derive(Debug, Clone, Default)]
struct Unid {
    born: TimeRef,
    place: P,
    size: P,
    seen: u32,
    last: Option<P>,
    freed: bool,
    resized: bool,
    /// Earlier detections in this uninterrupted run, capped at two.
    detections: u8,
    boundary: Option<BoundarySight>,
}

impl Unid {
    fn of(ws: &Ws, born: TimeRef) -> Unid {
        Unid { born, place: ws.pos, size: ws.size(), ..Unid::default() }
    }
}

/// A stored look of a world stone (spec 4.6.3): the frame it came on, the stone's place and size
/// there.
#[derive(Debug, Clone)]
struct Look {
    wid: i64,
    cells: Arc<Cells>,
    g: i64,
    t: f64,
    place: P,
    size: P,
    taken: bool,
    /// The world stone was detected on the look's frame: only such a look is an anchor.
    detected: bool,
}

/// An unseen run's first sight (spec 4.6.1, presence questions): where its stone was first
/// detected after the run's first calm moment, whether a busy frame or a push frame came from its
/// last sight up to then, and whether another world stone of its size stood within [`NEAR`] of
/// the place where it was last seen.
#[derive(Debug, Clone, Copy)]
struct Again {
    place: P,
    busy: bool,
    push: bool,
    crowd: bool,
}

/// A stone's unseen run (spec 4.6.1, presence questions): its world id, the place and the time it
/// was last seen (`s1`), the time of the run's first calm moment (`c1`), at which its world stone
/// was not detected, and its first sight after that calm moment.
#[derive(Debug, Clone)]
struct Watch {
    wid: i64,
    place: P,
    seen_t: f64,
    c1_t: f64,
    again: Option<Again>,
    /// A busy or push frame later than `c1` came while its world stone was not detected, or frames
    /// the rules could not read came (spec 4.6.1).
    spoilt: bool,
}

impl Watch {
    fn new(wid: i64, place: P, seen_t: f64, c1_t: f64) -> Watch {
        Watch { wid, place, seen_t, c1_t, again: None, spoilt: false }
    }
}

/// A stone's open unseen runs: its own, and its presence question's.
fn runs(s: &mut St) -> impl Iterator<Item = &mut Watch> {
    s.run1.iter_mut().chain(s.question.iter_mut().filter_map(|q| q.watch.as_mut()))
}

/// Another world stone of the stone's size (`size`) within [`NEAR`] of the place where it was
/// last seen before its unseen run: a sight there can not tell the two apart (spec 4.6.1).
fn crowded(size: P, w: &Watch, tw: &TrayWorld) -> bool {
    tw.stones.iter().any(|ws| {
        ws.id != w.wid && !ws.under_sheet && !ws.held && compatible(ws.size(), size) && dist(ws.pos, w.place) <= NEAR
    })
}

/// The sight half of a settle (spec 4.6.1, presence questions): linked to the run's world stone;
/// first seen after the run's first calm moment within [`NEAR`] of where it was last seen before
/// the run, with no busy or push frame from that last sight up to then and no other stone of its
/// size within [`NEAR`] of that place then; seen within [`NEAR`] of that place last, after that
/// calm moment; and no other stone of its size within [`NEAR`] of it now.
fn seen_again(s: &St, w: &Watch, tw: &TrayWorld) -> bool {
    let Some(a) = w.again else { return false };
    if dist(a.place, w.place) > NEAR || a.busy || a.push || a.crowd {
        return false;
    }
    s.on() && s.world == Some(w.wid) && s.seen_t > w.c1_t && dist(s.place, w.place) <= NEAR && !crowded(s.size, w, tw)
}

/// A stone of `s`'s size appeared next to it: one of `births` (the places and sizes of the world
/// stones first seen since the last snapshot) within [`HAND_R`] of its place or of its place in
/// the last snapshot.
fn appeared(s: &St, births: &[(P, P)]) -> bool {
    births
        .iter()
        .any(|(place, size)| compatible(*size, s.size) && dist(*place, s.place).min(dist(*place, s.at)) <= HAND_R)
}

/// A doubt held open for a look, or for the stone's next sight (a presence question), to settle.
#[derive(Debug, Clone)]
struct Question {
    cause: &'static str,
    why: &'static str,
    since: i64,
    opened: TimeRef,
    /// A presence question (spec 4.6.1): the unseen run it was opened with, kept until the
    /// question is settled or ends; seeing the stone again settles it.
    watch: Option<Watch>,
}

/// A pair of the look step: a world id (a row's pairs) or a tray index (a column's neighbours),
/// and its score and pair count (`None`: no score).
#[derive(Debug, Clone, Copy)]
struct Pair {
    key: i64,
    s: Option<(i64, i64)>,
}

/// A decided pair: the row, the world stone, its score, margin and bar.
#[derive(Debug, Clone, Copy)]
struct Best {
    row: usize,
    wid: i64,
    score: i64,
    margin: Option<i64>,
    bar: i64,
}

/// One run of the look step (spec 4.6.5), as the open account reads it: its rows (with their
/// since) and columns, each row's pairs, each pair's bar and best rival, and the decided pair with
/// the highest score.
#[derive(Debug, Clone, Default)]
struct Run {
    rows: Vec<(usize, i64)>,
    cols: Vec<i64>,
    pairs: BTreeMap<usize, Vec<Pair>>,
    info: BTreeMap<(usize, i64), (i64, Option<i64>)>,
    best: Option<Best>,
}

/// The scores of a tray, each pair of looks scored once: `[a.wid, a.g, b.wid, b.g]`.
type Memo = BTreeMap<[i64; 4], Option<(i64, i64)>>;

/// An open entry's candidates being ranked: those with a score (the negated score, best first) and
/// those without (by distance, nearest first).
type Ranked<K> = (Vec<(i64, K)>, Vec<(f64, K)>);

/// The permille score and pair count of look `a` against look `b`.
fn p_score(memo: &mut Memo, a: &Look, b: &Look) -> Option<(i64, i64)> {
    *memo
        .entry([a.wid, a.g, b.wid, b.g])
        .or_insert_with(|| pair_cells(&a.cells, &b.cells).map(|(s, n)| (look_permille(s), n)))
}

/// A tray stone's chain against a look L: against its anchor, or its latest when that scores
/// higher; a chain look that is L itself (the same world stone and frame) is not scored.
fn chain_score(memo: &mut Memo, s: &St, look: &Look) -> Option<(i64, i64)> {
    let mut best: Option<(i64, i64)> = None;
    for c in s.chain.iter().filter(|c| c.wid != look.wid || c.g != look.g) {
        let Some(x) = p_score(memo, look, c) else { continue };
        if best.is_none_or(|b| x.0 > b.0) {
            best = Some(x);
        }
    }
    best
}

/// Sure, not in question, never lifted on this tray and not disturbed since the last snapshot.
fn untouched(s: &St) -> bool {
    s.sure && s.question.is_none() && !s.lifted && s.disturbed.is_none()
}

/// Makes `look` the latest of the stone's chain, after its anchor (only a stone with a chain gets
/// one: a sure stone re-read, or a row of the look run).
fn set_latest(s: &mut St, look: Look) {
    s.chain.truncate(1);
    s.chain.push(look);
}

/// A stored look still describes its stone: the stone is here, not held, within [`NEAR`] of where
/// the look was read, and no hand mark after the look lies within [`HAND_R`] of that place.
fn fresh(marks: &[(f64, P)], e: &Look, tw: &TrayWorld) -> bool {
    if !tw.get(e.wid).is_some_and(|w| !w.under_sheet && !w.held && dist(w.pos, e.place) <= NEAR) {
        return false;
    }
    for &(t, p) in marks.iter().rev() {
        if t <= e.t {
            break;
        }
        if dist(p, e.place) <= HAND_R {
            return false;
        }
    }
    true
}

/// The first frame whose looks count for a hand doubt: after the stone's latest disturbance.
fn since_of(s: &St, r: TimeRef) -> i64 {
    s.disturbed.map_or(r.g, |d| d + 1)
}

/// A tray stone's working state.
#[derive(Debug, Clone, Default)]
struct St {
    under_sheet: bool,
    sheet_since: Option<TimeRef>,
    sheet_loss: bool,
    id: String,
    index: usize,
    world: Option<i64>,
    /// The world id it was made with: its arrival world id for a stone the tray arrived with.
    first_world: Option<i64>,
    world_t: f64,
    size: P,
    frozen: bool,
    place: P,
    seen_t: f64,
    born: TimeRef,
    /// Its place when it was made.
    first_place: P,
    /// Its place in the latest calm frame and at the leaving point, when it was seen there.
    calm_place: Option<P>,
    left_place: Option<P>,
    /// The frame where something last touched it on this tray (spec 4.7).
    touched: Option<i64>,
    status: TrayStoneStatus,
    /// Not given up: a stone in question keeps it (its view's `sure` is false meanwhile).
    sure: bool,
    added: bool,
    certain: Option<bool>,
    lost_t: f64,
    lost_k: i64,
    carried_away: bool,
    dropped: bool,
    emptied: bool,
    handled: bool,
    pushed: bool,
    ghost: u32,
    /// The snapshot the stone was last on the tray at, and its place there.
    at_k: i64,
    at: P,
    // by look (spec 4.6.3-4.6.8)
    chain: Vec<Look>,
    question: Option<Question>,
    doubted: bool,
    by_look: bool,
    look_lost: bool,
    lost_by_look: bool,
    lifted: bool,
    /// The frame of its latest disturbance since the snapshot.
    disturbed: Option<i64>,
    lost_g: i64,
    late: bool,
    boxed: bool,
    unsure_code: &'static str,
    unsure_from_t: f64,
    unid_rec: Option<Unid>,
    left_now: bool,
    added_now: bool,
    open: Option<TrayOpen>,
    /// A world stone a look unlinked was pushed next to it since the snapshot.
    freed_push: bool,
    // presence questions (spec 4.6.1)
    /// Its current unseen run; it ends when the stone is seen at a calm moment or lost.
    run1: Option<Watch>,
    /// Its current unseen run is on probation: it began at a calm moment that settled the stone's
    /// presence question without showing it, so the calm moment that shows it again must settle
    /// that run too.
    once: bool,
    /// The latest time it was handled (a `lift`, `carry`, `hinge` or `place` on its link) or held.
    handled_t: Option<f64>,
    /// The latest time a hand touched its link: a hand event, a `push` with a `side`, or a `delete`
    /// with why `hand` or `carried-away` on its world id (spec 4.6.1, certain removals).
    hand_t: Option<f64>,
    /// The time of the last calm moment before its latest loss (spec 4.6.1, certain removals).
    lost_calm_t: f64,
    /// The stone in the newest speckle answer read while the tray was on (spec 4.6.12).
    speckle: Option<StoneSpeckle>,
    // the leaving's speckle answer (spec 4.6.13)
    /// The leaving's answer linked or confirmed it.
    by_speckle: bool,
    /// What the leaving's answer found of it at the other moment, set as the tray leaves.
    evidence: Option<TrayEvidence>,
}

impl St {
    fn view(&self) -> TrayStone {
        TrayStone {
            id: self.id.clone(),
            world_id: self.world,
            status: self.status.clone(),
            sure: self.sure && self.question.is_none(),
            added: self.added,
            certain: self.certain,
            born: self.born,
            place: self.place,
            size: self.size,
            question: self.question.as_ref().map(|q| TrayQuestion {
                cause: q.cause.to_string(),
                why: q.why.to_string(),
                since: q.since,
                opened: q.opened,
            }),
            looks: self.chain.iter().map(|l| l.g).collect(),
            by_look: self.by_look,
            look_lost: self.look_lost,
            open: self.open.clone(),
            first_place: self.first_place,
            left_place: self.left_place,
            touched: self.touched,
            speckle: self.speckle.clone(),
            by_speckle: self.by_speckle,
            evidence: self.evidence.clone(),
            under_sheet: self.under_sheet,
            sheet_since: self.sheet_since,
        }
    }

    fn on(&self) -> bool {
        self.status == TrayStoneStatus::On
    }

    fn lost(&self) -> bool {
        self.status == TrayStoneStatus::Lost
    }
}

/// On the tray and sure, never in question or not sure on this tray, and not linked by a look:
/// without looks exactly v1's "on the tray and sure".
fn trusted(s: &St) -> bool {
    s.on() && s.sure && !s.doubted && !s.by_look
}

/// Where the rules' output goes: the summary's counts and the facts of the frame.
pub(crate) struct Sink<'a> {
    pub(crate) counts: &'a mut TrayCounts,
    pub(crate) facts: &'a mut Vec<TrayFact>,
}

#[derive(Debug, Clone)]
struct VisitSnapshot {
    at: TimeRef,
    places: BTreeMap<usize, (i64, P)>,
    world_stones: BTreeMap<i64, Ws>,
}

#[derive(Debug, Clone, Default)]
struct VisitEvidence {
    start_g: i64,
    quiet_frames: u32,
    epoch: u64,
    clean: Option<Arc<VisitSnapshot>>,
    fallback: Option<Arc<VisitSnapshot>>,
    /// Tray index -> linked world id, grid frame and quiet run of the incomplete snapshot.
    unseen: BTreeMap<usize, (i64, i64, u64)>,
}

/// The open tray's working state (dropped when it leaves; what is kept is its [`Tray`]).
#[derive(Debug, Clone, Default)]
struct OpenTray {
    session: i64,
    video: String,
    n: i64,
    ids: BTreeSet<i64>,
    arrived: Option<TimeRef>,
    leaving: Option<TimeRef>,
    board: Option<Rect>,
    stones: Vec<St>,
    link: BTreeMap<i64, usize>,
    unid: BTreeMap<i64, Unid>,
    missing: BTreeMap<i64, MissingWorld>,
    /// Omissions from the world's inventory, including unlinked and undetected stones. Used only for calm timing.
    inventory_missing: BTreeMap<i64, i64>,
    visit: VisitEvidence,
    marks: Vec<(f64, P)>,
    k: i64,
    settle_t: f64,
    calm_ref: Option<TimeRef>,
    /// The world stones at that point, held or not, detected or kept, including stones first identified at close.
    calm_stones: BTreeMap<i64, Ws>,
    /// The leaving point's world stones, held or not, detected or kept (set as the tray leaves: its evidence at
    /// departure; the leaving reads those not held).
    point: BTreeMap<i64, Ws>,
    worked: bool,
    last: Option<TrayWorld>,
    unclear: u64,
    /// Certain added facts: the leaving's count, also of a stone removed unclear later.
    added: u64,
    facts: Vec<TrayFact>,
    // by look: the look store, the gauge, twins (tray indexes), the scores, the leaving's last
    // run, the open account
    store: BTreeMap<i64, Look>,
    alike: Option<i64>,
    alike_n: u32,
    took: bool,
    twins: BTreeSet<usize>,
    memo: Memo,
    ids_at_arrival: BTreeSet<i64>,
    last_run: Option<Run>,
    left: bool,
    open: Vec<TrayOpen>,
    /// A look changed a link, doubted a stone v1 trusts, or kept a twin from v1's certain removal:
    /// links and trust may differ from v1's from here on, so no certain removal or addition after it.
    relinked: bool,
    /// The time of the tray's latest hand mark: a look older than it settles no question.
    last_mark_t: Option<f64>,
    /// The time of the latest busy frame: a hand not free or flagged `untrusted`, a hand event, a
    /// held world stone, or a `delete` why `hand` or `carried-away` (spec 4.6.1, presence
    /// questions); a frame after frames the rules could not read is one too.
    busy_t: Option<f64>,
    /// The time of the latest push frame: a `push` event (any id, with a side or not), or a frame
    /// after frames the rules could not read (spec 4.6.1).
    push_t: Option<f64>,
    /// The grid frame of the last snapshot.
    settle_g: Option<i64>,
    /// The links as the tray arrived (world id -> tray stone id), for the speckle answers (spec
    /// 4.6.12).
    arrival_links: BTreeMap<i64, String>,
    /// The unit's speckle answers read while the tray was on, oldest first (spec 4.6.13) ...
    answers: Vec<WorldSpeckle>,
    /// ... and the index of the leaving's, once the tray has left.
    leaving_answer: Option<usize>,
}

impl OpenTray {
    fn new(session: i64, video: &str, n: i64) -> OpenTray {
        OpenTray { session, video: video.to_string(), n, k: -1, ..OpenTray::default() }
    }

    fn view(&self) -> Tray {
        Tray {
            session: self.session,
            video: self.video.clone(),
            n: self.n,
            arrived: self.arrived,
            leaving: self.leaving,
            stones: self.stones.iter().map(St::view).collect(),
            facts: self.facts.clone(),
            open: self.left.then(|| self.open.clone()),
            t_id: self.took.then(|| self.tid()),
        }
    }

    /// A fact about this tray (and the stone at `stone`), at `r`.
    fn fact(&self, kind: TrayFactKind, stone: Option<usize>, r: TimeRef) -> TrayFact {
        let (id, world_id) = stone.map_or((None, None), |i| (Some(self.stones[i].id.clone()), self.stones[i].world));
        TrayFact {
            kind,
            tray_stones: Vec::new(),
            under_sheet: None,
            tray: self.n,
            session: self.session,
            video: self.video.clone(),
            stone: id,
            world_id,
            certain: None,
            why: None,
            code: None,
            g: r.g,
            t: r.t,
            wall: r.wall,
            stones: None,
            sure: None,
            added: None,
            removed: None,
            unclear: None,
            opened: None,
            from_world: None,
            score: None,
            margin: None,
            bar: None,
            t_id: None,
            open: None,
        }
    }

    /// Says a fact: in the tray's own list and in the frame's.
    fn say(&mut self, fact: TrayFact, sink: &mut Sink<'_>) {
        self.facts.push(fact.clone());
        sink.facts.push(fact);
    }

    /// The tray stone id `id`, or, when the tray already has a stone with it (a stone and its world
    /// id seen again within the same tenth of a second), the first of `id`-2, `id`-3, ... that the
    /// tray does not have (4.6).
    fn unique_id(&self, id: String) -> String {
        let taken = |c: &str| self.stones.iter().any(|s| s.id == c);
        if !taken(&id) {
            return id;
        }
        (2..).map(|k| format!("{id}-{k}")).find(|c| !taken(c)).expect("a free id")
    }

    fn new_stone(&self, id: String, ws: &Ws, born: TimeRef) -> St {
        St {
            id: self.unique_id(id),
            index: self.stones.len(),
            world: Some(ws.id),
            first_world: Some(ws.id),
            world_t: born.t,
            size: ws.size(),
            frozen: ws.frozen,
            place: ws.pos,
            seen_t: born.t,
            born,
            first_place: ws.pos,
            status: TrayStoneStatus::On,
            sure: true,
            at_k: -1,
            ..St::default()
        }
    }

    fn snapshot(&mut self, r: TimeRef) {
        self.k += 1;
        for s in &mut self.stones {
            s.handled = false;
            s.pushed = false;
            s.disturbed = None;
            s.freed_push = false;
            if s.on() {
                s.at_k = self.k;
                s.at = s.place;
            }
        }
        self.settle_t = r.t;
        self.settle_g = Some(r.g);
        self.calm_ref = Some(r);
        self.worked = false;
    }

    /// A push on tray stone `i`'s world stone: it is pushed, and every tray stone of a compatible
    /// size next to it in the last snapshot is disturbed.
    fn push(&mut self, i: usize, r: TimeRef) {
        let k = self.k;
        let s = &mut self.stones[i];
        s.pushed = true;
        s.disturbed = Some(r.g);
        s.touched = Some(r.g);
        let (at_k, at, size) = (s.at_k, s.at, s.size);
        for o in self.stones.iter_mut().filter(|o| o.index != i && at_k == k) {
            if o.at_k == k && compatible(o.size, size) && dist(o.at, at) <= HAND_R {
                o.disturbed = Some(r.g);
                o.touched = Some(r.g);
            }
        }
    }

    /// A push on a world stone a look unlinked (freed): every tray stone of a compatible size next
    /// to it in the last snapshot is doubted at the settle as a pushed stone's neighbour is.
    fn freed_push(&mut self, place: P, size: P, r: TimeRef) {
        let k = self.k;
        for o in self.stones.iter_mut() {
            if o.at_k == k && compatible(o.size, size) && dist(o.at, place) <= HAND_R {
                o.freed_push = true;
                o.disturbed = Some(r.g);
                o.touched = Some(r.g);
            }
        }
    }

    fn inventory_gap(&mut self, tw: &TrayWorld, hole: bool) -> bool {
        let Some(previous) = self.last.as_ref().filter(|previous| {
            !hole && previous.r.g.checked_add(1) == Some(tw.r.g) && tw.events.is_empty() && tw.undisturbed()
        }) else {
            self.inventory_missing.clear();
            return false;
        };
        self.inventory_missing.retain(|id, first| tw.get(*id).is_none() && tw.r.g.saturating_sub(*first) < 2);
        for w in previous.stones.iter().filter(|w| !w.under_sheet && tw.get(w.id).is_none()) {
            self.inventory_missing.insert(w.id, tw.r.g);
        }
        !self.inventory_missing.is_empty()
    }

    /// Closing or changing visits cannot confirm an outstanding omission.
    fn finish_omissions(&mut self) {
        for (id, gap) in std::mem::take(&mut self.missing) {
            if let Some(i) = self.link.remove(&id) {
                self.lose(i, gap.first);
            }
        }
        self.inventory_missing.clear();
    }

    /// Resolve past omissions, then park newly missing IDs. Only consecutive quiet frames
    /// may bridge a gap. The first missing frame dates an eventual loss.
    fn omissions(&mut self, tw: &TrayWorld, hole: bool) {
        if self.arrived.is_none() {
            return;
        }
        let contiguous = !hole && self.last.as_ref().is_some_and(|w| w.r.g.checked_add(1) == Some(tw.r.g));
        let safe = contiguous && tw.undisturbed();
        let quiet = safe && tw.events.is_empty();
        for (id, mut gap) in std::mem::take(&mut self.missing) {
            let current = tw.get(id);
            let neighbours = tw.omission_neighbours(id, gap.place, gap.size, false);
            let no_new_rival = neighbours.is_subset(&gap.neighbours);
            if quiet && no_new_rival && current.is_none() && gap.first.g.checked_add(1) == Some(tw.r.g) {
                gap.neighbours = neighbours;
                self.missing.insert(id, gap);
                continue;
            }
            let recovered =
                safe && no_new_rival
                    && current.is_some_and(|w| {
                        let from_own = dist(gap.place, w.pos);
                        let size = w.size();
                        w.detected
                            && !w.under_sheet
                            && !w.held
                            && compatible(gap.size, size)
                            && from_own <= RENAME_D
                            && gap.other_stones.iter().all(|&(place, other_size)| {
                                !compatible(size, other_size) || from_own < dist(place, w.pos)
                            })
                    })
                    && !tw
                        .events
                        .iter()
                        .any(|e| e.id == Some(id) && !matches!(e.kind, EventKind::Confirm | EventKind::Freeze));
            if recovered {
                if let Some(u) = gap.unidentified {
                    self.unid.insert(id, u);
                }
            } else if !tw
                .events
                .iter()
                .any(|e| e.id == Some(id) && matches!(e.kind, EventKind::Gone | EventKind::Drop | EventKind::Delete))
                && let Some(i) = self.link.remove(&id)
            {
                self.lose(i, gap.first);
            }
        }
        if let Some(previous) = self.last.as_ref().filter(|w| quiet && w.undisturbed()) {
            for w in &previous.stones {
                if w.detected
                    && !w.under_sheet
                    && !w.held
                    && tw.get(w.id).is_none()
                    && (self.link.contains_key(&w.id) || self.unid.contains_key(&w.id))
                {
                    let neighbours = tw.omission_neighbours(w.id, w.pos, w.size(), false);
                    if !neighbours.is_subset(&previous.omission_neighbours(w.id, w.pos, w.size(), false)) {
                        continue;
                    }
                    let mut unidentified = self.unid.remove(&w.id);
                    if let Some(u) = &mut unidentified {
                        u.detections = 0;
                        u.boundary = None;
                    }
                    self.missing.insert(
                        w.id,
                        MissingWorld {
                            first: tw.r,
                            place: w.pos,
                            size: w.size(),
                            neighbours,
                            other_stones: previous
                                .stones
                                .iter()
                                .filter(|o| o.id != w.id && !o.under_sheet && o.detected && !o.held)
                                .map(|o| (o.pos, o.size()))
                                .collect(),
                            unidentified,
                        },
                    );
                }
            }
        }
    }

    fn observe_unidentified(&mut self, tw: &TrayWorld, hole: bool) {
        let previous = self.last.as_ref().filter(|w| !hole && w.r.g.checked_add(1) == Some(tw.r.g));
        let clear = previous.is_some() && tw.undisturbed();
        let anchor = previous.filter(|w| {
            clear && !self.missing.is_empty() && tw.events.is_empty() && self.calm_ref == Some(w.r) && w.undisturbed()
        });
        self.unid.retain(|id, u| {
            let Some(ws) = tw.get(*id).filter(|w| !w.under_sheet) else { return false };
            if previous.is_none() {
                u.detections = 0;
            }
            let overlap = || ws.overlap_share(&tw.stones.iter().collect::<Vec<_>>());
            let neighbours = || tw.omission_neighbours(*id, ws.pos, ws.size(), true);
            if let Some(sight) = &mut u.boundary {
                let valid = clear
                    && self.calm_ref.is_some_and(|r| r.g == sight.anchor_g)
                    && (0.0..=RENAME_T).contains(&(tw.r.t - sight.t))
                    && ws.frozen
                    && compatible(ws.size(), sight.stone.size())
                    && dist(ws.pos, sight.stone.pos) <= RENAME_D
                    && overlap() < OVERLAP
                    && !tw
                        .events
                        .iter()
                        .any(|e| e.id == Some(*id) && !matches!(e.kind, EventKind::Confirm | EventKind::Freeze))
                    && neighbours().is_subset(&sight.neighbours);
                if !valid {
                    u.boundary = None;
                } else if ws.detected {
                    sight.latest = ws.clone();
                }
            }
            if ws.detected {
                if let Some(prev) = anchor
                    && let Some(old) = prev.get(*id)
                    && u.detections >= 2
                    && u.born.g < prev.r.g
                    && !old.detected
                    && old.frozen
                    && ws.frozen
                    && compatible(ws.size(), old.size())
                    && dist(ws.pos, old.pos) <= NEAR
                    && overlap() < OVERLAP
                {
                    let old_neighbours = prev.omission_neighbours(*id, old.pos, old.size(), true);
                    if neighbours().is_subset(&old_neighbours) {
                        u.boundary = Some(BoundarySight {
                            anchor_g: prev.r.g,
                            t: tw.r.t,
                            stone: ws.clone(),
                            latest: ws.clone(),
                            neighbours: old_neighbours,
                        });
                    }
                }
                u.detections = (u.detections + 1).min(2);
                u.place = ws.pos;
            }
            u.size = ws.size();
            true
        });
    }

    fn lose(&mut self, i: usize, r: TimeRef) {
        let (k, calm_t) = (self.k, self.settle_t);
        let s = &mut self.stones[i];
        s.under_sheet = false;
        s.sheet_since = None;
        s.sheet_loss = false;
        s.run1 = None; // a lost stone's unseen run ends (4.6.1): never spoilt while unlinked; on probation, it fails
        s.world = None;
        s.status = TrayStoneStatus::Lost;
        s.lost_t = r.t;
        s.lost_k = k;
        s.lost_g = r.g;
        s.lost_by_look = false;
        s.lost_calm_t = calm_t;
    }

    /// The tray's bar by look (spec 4.6.3), in permille.
    fn tid(&self) -> i64 {
        match self.alike {
            Some(a) if self.alike_n >= GAUGE_MIN => T_ID_MIN.max(a + GAUGE_GAP),
            _ => T_ID_START,
        }
    }

    // ---- doubts and questions (spec 4.6.4) -----------------------------------------------------

    /// Where a reason of 4.6.1 holds: a question for a stone with a chain (not for a box doubt),
    /// else unsure (a doubt without a watch: see [`OpenTray::doubt_watched`]).
    fn doubt(&mut self, i: usize, why: &'static str, cause: &'static str, r: TimeRef, since: i64, sink: &mut Sink<'_>) {
        self.doubt_watched(i, why, cause, r, since, None, sink);
    }

    /// Where a reason of 4.6.1 holds: a question for a stone with a chain (not for a box doubt),
    /// else unsure; an `unseen` doubt with a watch (a stone without a chain that nothing could
    /// have changed since it was last seen) is a presence question (4.6.1, 4.6.4). A later
    /// `unseen` doubt changes nothing in a presence question; a doubt of any other cause ends it.
    #[allow(clippy::too_many_arguments)]
    fn doubt_watched(
        &mut self,
        i: usize,
        why: &'static str,
        cause: &'static str,
        r: TimeRef,
        since: i64,
        watch: Option<Watch>,
        sink: &mut Sink<'_>,
    ) {
        let s = &mut self.stones[i];
        if !s.sure {
            return;
        }
        s.doubted = true;
        if let Some(q) = s.question.as_mut().filter(|q| q.watch.is_some()) {
            if cause == "unseen" {
                return; // still not seen: nothing new
            }
            q.watch = None; // any other doubt: seeing it again no longer settles it
        }
        if (!s.chain.is_empty() && cause != "overlap") || (watch.is_some() && s.question.is_none()) {
            if let Some(q) = s.question.as_mut() {
                q.since = q.since.max(since);
            } else {
                s.question = Some(Question { cause, why, since, opened: r, watch });
            }
            return;
        }
        s.boxed |= cause == "overlap";
        self.unsure(i, why, cause, r, sink);
    }

    /// The fact unsure: the stone is not sure for good; a question ends undecided with its own
    /// why and code.
    fn unsure(&mut self, i: usize, why: &'static str, cause: &'static str, r: TimeRef, sink: &mut Sink<'_>) {
        let s = &mut self.stones[i];
        let q = s.question.take();
        s.sure = false;
        sink.counts.unsure += 1;
        let (why, cause, opened) = q.map_or((why, cause, None), |q| (q.why, q.cause, Some(q.opened)));
        s.unsure_from_t = opened.map_or(r.t, |o| o.t);
        s.unsure_code = cause;
        let fact = TrayFact {
            why: Some(why.to_string()),
            code: Some(cause.to_string()),
            opened,
            ..self.fact(TrayFactKind::Unsure, Some(i), r)
        };
        self.say(fact, sink);
    }

    /// Ends a question undecided (before a certain removal, or when the tray leaves).
    fn end_question(&mut self, i: usize, r: TimeRef, sink: &mut Sink<'_>) {
        if let Some((why, cause)) = self.stones[i].question.as_ref().map(|q| (q.why, q.cause)) {
            self.unsure(i, why, cause, r, sink);
        }
    }

    fn hands_near(&self, since: f64, places: [P; 2]) -> bool {
        for &(t, p) in self.marks.iter().rev() {
            if t < since {
                break;
            }
            if places.iter().any(|q| dist(p, *q) <= HAND_R) {
                return true;
            }
        }
        false
    }

    /// The leaving's evidence (spec 4.6.1, Leaving; 4.6.7, step 4) that the unidentified world
    /// stone W is the lost tray stone S, which a pair needs: their looks agree (S's chain against
    /// W's stored look: at least the tray's bar over at least N_ID cells); or, when the looks do not
    /// disagree (below T_DIFF over at least N_ID cells), nothing could have put another stone there:
    /// W was first seen at or after S was last seen, less RENAME_T, and no hand mark from the
    /// earliest of S's last sight, S's loss and W's first sight, less RENAME_T, lies within HAND_R
    /// of S's place or of W's.
    fn same_stone(&mut self, i: usize, wid: i64) -> bool {
        let bar = self.tid();
        let s = &self.stones[i];
        let u = &self.unid[&wid];
        let x = match self.store.get(&wid) {
            Some(e) if !s.chain.is_empty() => chain_score(&mut self.memo, s, e),
            _ => None,
        };
        match x.filter(|&(_, n)| n >= N_ID) {
            Some((sc, _)) if sc >= bar => true,
            Some((sc, _)) if sc < T_DIFF => false,
            // the looks do not disagree, or there are none to compare: nothing could have put another stone there
            _ => {
                u.born.t >= s.seen_t - RENAME_T
                    && !self.hands_near(s.seen_t.min(s.lost_t).min(u.born.t) - RENAME_T, [s.place, u.place])
            }
        }
    }

    // ---- presence questions (spec 4.6.1) --------------------------------------------------------

    /// Frames the rules could not read came before the frame at `t`: it counts as a busy and a
    /// push frame, and every open unseen run (a stone's, and a presence question's) is spoilt, its
    /// stone detected or not.
    fn unread(&mut self, t: f64) {
        self.busy_t = Some(self.busy_t.map_or(t, |b| b.max(t)));
        self.push_t = Some(self.push_t.map_or(t, |p| p.max(t)));
        for s in &mut self.stones {
            for run in runs(s) {
                run.spoilt = true;
            }
        }
    }

    /// A stone of the size of tray stone `s` next to it in the last snapshot was pushed.
    fn pushed_next(&self, s: &St, k: i64) -> bool {
        self.stones.iter().any(|o| {
            o.index != s.index && o.pushed && o.at_k == k && compatible(o.size, s.size) && dist(o.at, s.at) <= HAND_R
        })
    }

    /// A look of the world stone of `w` read since tray stone `i` was last seen, still fresh, that
    /// reaches the bar against the chain of another tray stone the look step pairs looks with
    /// (lost, or on the tray in question): that stone may be on this world stone, so no sight
    /// settles the question.
    fn look_against(&mut self, i: usize, w: &Watch, tw: &TrayWorld) -> bool {
        let Some(e) = self.store.get(&w.wid) else { return false };
        if e.t <= w.seen_t || !fresh(&self.marks, e, tw) {
            return false;
        }
        let bar = self.tid();
        for o in &self.stones {
            if o.index == i
                || o.chain.is_empty()
                || !o.sure
                || !compatible(o.size, e.size)
                || !(o.lost() || (o.on() && o.question.is_some()))
            {
                continue;
            }
            if chain_score(&mut self.memo, o, e).is_some_and(|(sc, n)| sc >= bar && n >= N_ID) {
                return true;
            }
        }
        false
    }

    /// Nothing could have changed tray stone `i` since it was last seen before the run `w`: the
    /// run is not spoilt, the stone was not handled or held since, no hand mark since lies within
    /// [`HAND_R`] of its last place or of its place now, and no look of its world stone since shows
    /// another open stone ([`OpenTray::look_against`]).
    fn watch_open(&mut self, i: usize, w: &Watch, tw: &TrayWorld) -> bool {
        let s = &self.stones[i];
        if s.handled_t.is_some_and(|h| h >= w.seen_t) {
            return false;
        }
        if w.spoilt {
            return false;
        }
        if self.hands_near(w.seen_t, [w.place, s.place]) {
            return false;
        }
        !self.look_against(i, w, tw)
    }

    /// The leaving may leave the last snapshot's own frame uncounted for tray stone `i` (4.6.1,
    /// Leaving): its unseen run is open ([`OpenTray::watch_open`]), and no busy frame and no push
    /// frame came since it was last seen.
    fn clean_since(&mut self, i: usize, tw: &TrayWorld) -> bool {
        let Some(r) = self.stones[i].run1.clone() else { return false };
        if !self.watch_open(i, &r, tw) {
            return false;
        }
        !(self.busy_t.is_some_and(|b| b >= r.seen_t) || self.push_t.is_some_and(|p| p >= r.seen_t))
    }

    /// The watch of an unseen doubt of tray stone `i` (a copy of its unseen run): none for a stone
    /// with a chain (only a look settles its question), when another cause of the settle's step 1
    /// holds now, or when something could have changed the stone since it was last seen.
    fn watch_of(&mut self, i: usize, k: i64, births: &[(P, P)], tw: &TrayWorld) -> Option<Watch> {
        let s = &self.stones[i];
        let r = s.run1.as_ref()?;
        if !s.chain.is_empty() || s.world != Some(r.wid) || r.spoilt || s.pushed || s.freed_push {
            return None;
        }
        if self.pushed_next(s, k) || (s.handled && appeared(s, births)) {
            return None;
        }
        let w = Watch { spoilt: false, ..r.clone() };
        self.watch_open(i, &w, tw).then_some(w)
    }

    /// Links a lost tray stone to an unidentified world stone of this frame (its fact names the
    /// new world id).
    #[allow(clippy::too_many_arguments)]
    fn relink(
        &mut self,
        i: usize,
        wid: i64,
        tw: &TrayWorld,
        sure: bool,
        why: &'static str,
        cause: &'static str,
        r: TimeRef,
        sink: &mut Sink<'_>,
    ) {
        let u = self.unid.remove(&wid).expect("an unidentified stone");
        let ws = tw.get(wid).expect("in this frame's world");
        let s = &mut self.stones[i];
        s.sheet_loss = false;
        s.world = Some(wid);
        s.status = TrayStoneStatus::On;
        s.touched = Some(r.g);
        s.world_t = u.born.t;
        s.carried_away = false;
        s.dropped = false;
        s.emptied = false;
        s.place = ws.pos;
        if ws.detected {
            s.seen_t = r.t;
        }
        self.link.insert(wid, i);
        if sure {
            return;
        }
        if cause == "matched_at_leaving" {
            // never a question
            if self.stones[i].sure {
                self.stones[i].doubted = true;
                self.unsure(i, why, cause, r, sink);
            }
            return;
        }
        self.doubt(i, why, cause, r, r.g + 1, sink);
    }

    /// A lost stone and one unidentified stone at its place, within RENAME_T s of the loss: the
    /// world renamed it.
    fn renames(&mut self, tw: &TrayWorld, sink: &mut Sink<'_>) {
        let lost: Vec<usize> =
            (0..self.stones.len()).filter(|&i| self.stones[i].lost() && !self.stones[i].dropped).collect();
        if lost.is_empty() || self.unid.is_empty() {
            return;
        }
        let fits = |o: &St, u: &Unid| {
            (u.born.t - o.lost_t).abs() <= RENAME_T && compatible(u.size, o.size) && dist(u.place, o.place) <= RENAME_D
        };
        for &i in &lost {
            let near: Vec<i64> =
                self.unid.iter().filter(|(_, u)| fits(&self.stones[i], u)).map(|(id, _)| *id).collect();
            let &[wid] = near.as_slice() else { continue };
            let u = &self.unid[&wid];
            if lost.iter().any(|&o| o != i && self.stones[o].lost() && fits(&self.stones[o], u)) {
                continue;
            }
            let s = &self.stones[i];
            let sure = s.sure
                && s.question.is_none()
                && s.lost_k == self.k
                && !s.carried_away
                && !s.emptied
                && !self.hands_near(s.lost_t.min(u.born.t) - RENAME_T, [s.place, u.place]);
            self.relink(i, wid, tw, sure, "found again, not for sure", "found_again", tw.r, sink);
        }
    }

    /// A world stone could be this lost stone: unidentified, or standing for a tray stone that is
    /// not trusted, of a compatible size, first seen after it was last seen (less RENAME_T) or at
    /// its place.
    fn rivals(&self, s: &St) -> bool {
        let near = |size: P, t: f64, place: P| {
            compatible(size, s.size) && (t >= s.seen_t - RENAME_T || dist(place, s.place) <= RENAME_D)
        };
        self.unid.values().any(|u| near(u.size, u.born.t, u.place))
            || self.stones.iter().any(|o| o.on() && !trusted(o) && near(o.size, o.world_t, o.place))
    }

    fn settle(&mut self, tw: &TrayWorld, r: TimeRef, last: bool, sink: &mut Sink<'_>) {
        let (t, k) = (r.t, self.k);
        let world: Vec<&Ws> = tw.stones.iter().filter(|w| !w.under_sheet && !w.held).collect();
        let mut births: Vec<(P, P)> =
            self.unid.values().filter(|u| u.born.t > self.settle_t).map(|u| (u.place, u.size)).collect();
        births.extend(
            self.stones
                .iter()
                .filter(|s| s.on() && !trusted(s) && s.world_t > self.settle_t)
                .map(|s| (s.place, s.size)),
        );
        // 1. sure stones (and stones in question), one by one in tray order: is the world still
        //    showing them? The leaving tests the last frame again only when it is not the last
        //    snapshot's own frame, or when something could have changed the stone since it was last
        //    seen (4.6.1, Leaving)
        let same_frame = last && self.settle_g == Some(tw.r.g);
        // at the leaving, the stones its speckle answer pairs are not tested: the answer settles them (4.6.13)
        let answer = if last { self.leaving_answer_at(r) } else { None };
        let paired: BTreeSet<usize> = answer
            .map(|a| self.speckle_targets(&self.answers[a], tw).iter().map(|t| t.i).collect())
            .unwrap_or_default();
        for i in 0..self.stones.len() {
            let s = &self.stones[i];
            if s.under_sheet || !s.on() || !s.sure || s.at_k != k || paired.contains(&i) {
                continue;
            }
            let ws = s.world.and_then(|id| tw.get(id));
            if let Some(&(id, g, epoch)) = self.visit.unseen.get(&i)
                && g != tw.r.g
                && (last || epoch != self.visit.epoch)
            {
                self.visit.unseen.remove(&i);
                if s.world == Some(id) && ws.is_some_and(|w| !w.detected) {
                    self.doubt(i, "not seen at two calm moments", "unseen", r, r.g, sink);
                    continue;
                }
            }
            let mut settled = false;
            if let Some(w) = s.question.as_ref().and_then(|q| q.watch.clone()) {
                // a presence question (4.6.1)
                if !self.watch_open(i, &w, tw) {
                    // something could have changed it: it ends
                    self.end_question(i, r, sink);
                    continue;
                }
                if seen_again(&self.stones[i], &w, tw) {
                    // seen again: settled, sure again, no fact
                    let s = &mut self.stones[i];
                    (s.question, s.ghost, s.run1, settled) = (None, 0, None, true);
                }
            }
            if ws.is_some_and(|w| w.overlap_share(&world) >= OVERLAP) {
                self.doubt(i, "its box overlaps another stone", "overlap", r, r.g, sink);
                continue;
            }
            if let Some(w) = ws.filter(|w| !w.detected) {
                // the leaving on the last snapshot's own frame, nothing since its last sight: tested
                // there
                if !(same_frame && self.clean_since(i, tw)) {
                    let s = &mut self.stones[i];
                    s.ghost += 1;
                    if s.ghost == 1 {
                        s.run1 = Some(Watch::new(w.id, s.place, s.seen_t, t));
                        if settled {
                            // settled on a calm moment that does not show it: its next run is on probation
                            s.once = true;
                        }
                    }
                    if s.ghost >= GHOST {
                        if s.question.as_ref().is_some_and(|q| q.watch.is_some()) {
                            // still watched, still not seen
                            self.doubt(i, "not seen at two calm moments", "unseen", r, r.g, sink);
                        } else {
                            let wt = self.watch_of(i, k, &births, tw);
                            self.stones[i].once = false;
                            self.doubt_watched(i, "not seen at two calm moments", "unseen", r, r.g, wt.clone(), sink);
                            let s = &self.stones[i];
                            let asked = s.question.as_ref().is_some_and(|q| q.watch.is_some());
                            if wt.is_some_and(|wt| asked && seen_again(s, &wt, tw)) {
                                // seen since the first unseen calm moment: settled; not detected now, so
                                // this calm moment starts its next unseen run, on probation
                                let s = &mut self.stones[i];
                                (s.question, s.ghost, s.once) = (None, 1, true);
                                s.run1 = Some(Watch::new(w.id, s.place, s.seen_t, t));
                            }
                            continue;
                        }
                    }
                }
            } else {
                let s = &mut self.stones[i];
                let (run, once) = (s.run1.take(), s.once);
                (s.ghost, s.once) = (0, false);
                if once && !run.is_some_and(|run| self.watch_open(i, &run, tw) && seen_again(&self.stones[i], &run, tw))
                {
                    // its run on probation ends with a sight that does not settle it: something could
                    // have changed the stone while it was out of sight (4.6.1)
                    self.doubt(i, "not seen at two calm moments", "unseen", r, r.g, sink);
                    continue;
                }
            }
            let s = &self.stones[i];
            let since = since_of(s, r);
            if s.pushed {
                self.doubt(i, "it was pushed", "pushed", r, since, sink);
                continue;
            }
            if self.pushed_next(s, k) || s.freed_push {
                self.doubt(i, "a stone next to it was pushed", "neighbour_pushed", r, since, sink);
                continue;
            }
            if s.handled && appeared(s, &births) {
                self.doubt(i, "a stone appeared next to it while a hand held it", "appeared", r, since, sink);
            }
        }
        // 2. the look work
        self.look_work(tw, r, sink);
        if last {
            // the leaving's speckle answer (4.6.13): the unit's pairs confirm, relink and find stones again
            self.speckle_leaving(tw, r, sink);
            self.leave(tw, r, sink);
            return;
        }
        // 3. certain removals, on a tray that is not relinked: v1's rule (lost before this calm moment
        //    or RENAME_T ago, its place empty, nothing that could be it); a twin keeps a stone v1
        //    removes, and relinks the tray
        for i in 0..self.stones.len() {
            if self.relinked {
                break;
            }
            let s = &self.stones[i];
            if !s.lost() || s.sheet_loss || (s.lost_k >= k && t - s.lost_t < RENAME_T) {
                continue;
            }
            if world.iter().any(|w| w.covers(s.place, MARGIN)) || self.rivals(s) || !hand_took(s) {
                continue;
            }
            if self.twins.contains(&i) {
                self.relinked = true;
                break;
            }
            let code = if s.carried_away { "carried_away" } else { "missing" };
            self.end_question(i, r, sink);
            self.stones[i].status = TrayStoneStatus::Removed;
            self.stones[i].certain = Some(true);
            sink.counts.removed += 1;
            let fact = TrayFact {
                certain: Some(true),
                why: Some(code.replace('_', " ")),
                code: Some(code.to_string()),
                ..self.fact(TrayFactKind::Removed, Some(i), r)
            };
            self.say(fact, sink);
        }
        // 4. certain additions: an unidentified stone at the same place at two calm moments in a
        //    row, not freed by a look nor made by a size change, while no tray stone of its size is
        //    lost or not trusted, on a tray that is not relinked
        let doubtful: Vec<P> =
            self.stones.iter().filter(|s| s.lost() || (s.on() && !trusted(s))).map(|s| s.size).collect();
        let (ids, relinked): (Vec<i64>, bool) = (self.unid.keys().copied().collect(), self.relinked);
        for wid in ids {
            let u = self.unid.get_mut(&wid).expect("an unidentified stone");
            let Some(ws) = tw.get(wid).filter(|w| w.detected) else {
                u.seen = 0;
                u.last = None;
                continue;
            };
            u.seen = if u.last.is_some_and(|l| dist(l, u.place) <= NEAR) { u.seen + 1 } else { 1 };
            u.last = Some(u.place);
            if u.seen >= 2 && !u.freed && !u.resized && !relinked && !doubtful.iter().any(|z| compatible(u.size, *z)) {
                self.mint(wid, ws, true, r, sink);
            }
        }
        // 5. the snapshot
        self.snapshot(r);
    }

    /// The rest of the leaving (spec 4.6.7): questions end, what is left is paired nearest first
    /// (never sure, and only a pair with evidence), the rest is unclear, then the open account and
    /// the snapshot.
    fn leave(&mut self, tw: &TrayWorld, r: TimeRef, sink: &mut Sink<'_>) {
        for i in 0..self.stones.len() {
            self.end_question(i, r, sink);
        }
        let mut pairs: Vec<(f64, usize, i64)> = Vec::new();
        for s in self.stones.iter().filter(|s| s.lost()) {
            for (&wid, u) in &self.unid {
                if compatible(u.size, s.size) && dist(u.place, s.place) <= LEAVE_D {
                    pairs.push((dist(u.place, s.place), s.index, wid));
                }
            }
        }
        pairs.retain(|&(_, i, wid)| self.same_stone(i, wid));
        // nearest first, then in tray order, then by world id
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let (mut used_s, mut used_w) = (BTreeSet::new(), BTreeSet::new());
        for (_, i, wid) in pairs {
            if used_s.contains(&i) || used_w.contains(&wid) {
                continue;
            }
            used_s.insert(i);
            used_w.insert(wid);
            self.relink(i, wid, tw, false, "matched when the tray left", "matched_at_leaving", r, sink);
        }
        for i in 0..self.stones.len() {
            if self.stones[i].lost() {
                let s = &mut self.stones[i];
                if s.sheet_loss {
                    s.status = TrayStoneStatus::On;
                    if s.sure {
                        self.unsure(i, "its track ended when the paper lifted", "unseen", r, sink);
                    }
                    continue;
                }
                s.status = TrayStoneStatus::Removed;
                s.certain = Some(false);
                s.left_now = true;
                self.unclear += 1;
                sink.counts.removed_unclear += 1;
                let fact = TrayFact {
                    certain: Some(false),
                    why: Some("not found when the tray left".to_string()),
                    code: Some("not_found".to_string()),
                    ..self.fact(TrayFactKind::Removed, Some(i), r)
                };
                self.say(fact, sink);
            }
        }
        let ids: Vec<i64> = self.unid.keys().copied().collect();
        for wid in ids {
            let Some(ws) = tw.get(wid) else { continue };
            if ws.detected {
                self.mint(wid, ws, false, r, sink);
            } else if let Some(sight) =
                self.unid.get_mut(&wid).and_then(|u| u.boundary.take()).filter(|s| s.anchor_g == r.g)
            {
                self.mint(wid, &sight.latest, false, r, sink);
            } else if let Some(at) = self
                .point
                .get(&wid)
                .filter(|at| {
                    // detected, not held, in the leaving point's world: positive evidence too (no hand worked since);
                    // or kept there, not held, less than OVERLAP of its box under another unheld stone's: the world
                    // kept it on the tray although the model did not box it there (version 7)
                    let unheld: Vec<&Ws> = self.point.values().filter(|o| !o.under_sheet && !o.held).collect();
                    !at.held && (at.detected || at.overlap_share(&unheld) < OVERLAP)
                })
                .cloned()
            {
                self.mint(wid, &at, false, r, sink);
            }
        }
        self.open = self.open_account(tw);
        self.unid.clear();
        self.left = true;
        self.snapshot(r);
    }

    fn mint(&mut self, wid: i64, ws: &Ws, certain: bool, r: TimeRef, sink: &mut Sink<'_>) {
        let u = self.unid.remove(&wid).expect("an unidentified stone");
        let id = format!("{}-{}-{wid}", stamp(u.born.wall, u.born.t, self.session), self.n);
        let mut s = self.new_stone(id, ws, u.born);
        s.sure = certain;
        s.added = true;
        s.certain = Some(certain);
        s.doubted = !certain;
        s.added_now = !certain;
        s.unid_rec = Some(u);
        self.link.insert(wid, s.index);
        let i = s.index;
        self.stones.push(s);
        let (why, code) = if certain {
            self.added += 1;
            sink.counts.added += 1;
            ("set down", "set_down")
        } else {
            self.unclear += 1;
            sink.counts.added_unclear += 1;
            ("not recognised when the tray left", "not_recognised")
        };
        let fact = TrayFact {
            certain: Some(certain),
            why: Some(why.to_string()),
            code: Some(code.to_string()),
            ..self.fact(TrayFactKind::Added, Some(i), r)
        };
        self.say(fact, sink);
    }

    // ---- looks on the tray (spec 4.6.3) ---------------------------------------------------------

    /// The look store: forgets the ids of this frame's look clearers and of stones gone, then keeps
    /// this frame's usable looks of stones not held inside the board (any place before the
    /// arrival).
    fn store_looks(&mut self, tw: &TrayWorld) {
        for id in tw.events.iter().filter(|e| e.clears_look()).filter_map(|e| e.id) {
            self.store.remove(&id);
        }
        self.store.retain(|id, _| tw.by_id.contains_key(id));
        for ws in &tw.stones {
            if ws.under_sheet || ws.held || !inside_board(self.board.as_ref(), ws.pos) {
                continue;
            }
            if let Some(cells) = &ws.look {
                let look = Look {
                    wid: ws.id,
                    cells: Arc::clone(cells),
                    g: tw.r.g,
                    t: tw.r.t,
                    place: ws.pos,
                    size: ws.size(),
                    taken: false,
                    detected: ws.detected,
                };
                self.store.insert(ws.id, look);
            }
        }
    }

    // ---- the look work (spec 4.6.5) -------------------------------------------------------------

    fn look_work(&mut self, tw: &TrayWorld, r: TimeRef, sink: &mut Sink<'_>) {
        self.take_looks(tw, r, sink);
        self.anchors(tw);
        loop {
            let run = self.look_run(tw);
            let best = run.best;
            self.last_run = Some(run);
            let Some(best) = best else { return };
            self.apply(best, tw, r, sink);
        }
    }

    fn take_looks(&mut self, tw: &TrayWorld, r: TimeRef, sink: &mut Sink<'_>) {
        let take: Vec<i64> =
            self.store.values().filter(|e| !e.taken && fresh(&self.marks, e, tw)).map(|e| e.wid).take(N_TAKE).collect();
        for id in take {
            let e = self.store.get_mut(&id).expect("a stored look");
            e.taken = true;
            let e = e.clone();
            self.took = true;
            let ws = tw.get(id).expect("a fresh look's stone is here");
            let (here, size) = (ws.pos, ws.size());
            // the gauge: how alike two different stones of this tray look, both ways
            let mut partners: Vec<(f64, i64)> = Vec::new();
            for (&oid, &oi) in &tw.by_id {
                let o = &tw.stones[oi];
                let taken = self.store.get(&oid).is_some_and(|x| x.taken && fresh(&self.marks, x, tw));
                if oid == id || !taken || !inside_board(self.board.as_ref(), o.pos) || !compatible(o.size(), size) {
                    continue;
                }
                let d = dist(o.pos, here);
                if d >= 0.5 * size[0] {
                    partners.push((d, oid));
                }
            }
            partners.sort_by(by_near);
            for &(_, pid) in partners.iter().take(GAUGE_K) {
                let p = &self.store[&pid];
                let both = [p_score(&mut self.memo, &e, p), p_score(&mut self.memo, p, &e)];
                if let Some(m) = both.into_iter().flatten().map(|x| x.0).max() {
                    self.alike = Some(self.alike.map_or(m, |a| a.max(m)));
                    self.alike_n += 1;
                }
            }
            // the own compare of a sure stone with a chain
            let Some(&si) = self.link.get(&id) else { continue };
            let s = &self.stones[si];
            if s.chain.is_empty() || !untouched(s) {
                continue;
            }
            let Some((sc, n)) = chain_score(&mut self.memo, s, &e) else { continue };
            if sc >= self.tid() && n >= N_ID {
                set_latest(&mut self.stones[si], e);
            } else if sc < T_DIFF && n >= N_ID {
                if trusted(&self.stones[si]) {
                    // v1 trusts it: from here on what the tray trusts may differ from v1's
                    self.relinked = true;
                }
                self.doubt(si, "its look changed", "look_changed", r, e.g, sink);
            }
        }
    }

    fn anchors(&mut self, tw: &TrayWorld) {
        for s in &mut self.stones {
            let Some(w) = s.world.filter(|_| !s.under_sheet && s.on() && s.chain.is_empty()) else { continue };
            let Some(e) = self.store.get(&w).filter(|e| e.taken && fresh(&self.marks, e, tw)) else { continue };
            if untouched(s) && e.detected && s.ghost == 0 {
                s.chain = vec![e.clone()];
            } else {
                s.late = true;
            }
        }
    }

    /// One run of the look step: rows (tray stones with a chain, in question or lost), columns
    /// (open world stones with a fresh taken look), their pairs, neighbours, hits and twins, and
    /// the decided pair with the highest score.
    fn look_run(&mut self, tw: &TrayWorld) -> Run {
        let tid = self.tid();
        let mut rows: Vec<(usize, i64)> = Vec::new();
        for s in &self.stones {
            if s.under_sheet || s.chain.is_empty() || !s.sure {
                continue;
            }
            let since = match (&s.status, &s.question) {
                (TrayStoneStatus::On, Some(q)) => q.since,
                (TrayStoneStatus::Lost, Some(q)) if s.lost_by_look => q.since,
                (TrayStoneStatus::Lost, Some(q)) => q.since.max(s.lost_g + 1),
                (TrayStoneStatus::Lost, None) => s.lost_g + 1,
                _ => continue,
            };
            rows.push((s.index, since));
        }
        let mut cols: Vec<&Ws> = Vec::new();
        for ws in &tw.stones {
            if ws.under_sheet || ws.held || !inside_board(self.board.as_ref(), ws.pos) {
                continue;
            }
            if !self.store.get(&ws.id).is_some_and(|e| e.taken && fresh(&self.marks, e, tw)) {
                continue;
            }
            let open = match self.link.get(&ws.id) {
                Some(&y) => !self.stones[y].sure || self.stones[y].question.is_some(),
                None => self.unid.contains_key(&ws.id),
            };
            if open {
                cols.push(ws);
            }
        }
        // each row's pairs: (world id, S, n), nearest first
        let mut pairs: BTreeMap<usize, Vec<Pair>> = BTreeMap::new();
        for &(i, since) in &rows {
            let s = &self.stones[i];
            // a stone carried away or emptied pairs only with world stones first seen after its loss
            let gone = s.lost() && (s.carried_away || s.emptied);
            let mut ns: Vec<(f64, i64)> = cols
                .iter()
                .filter(|ws| {
                    let e = &self.store[&ws.id];
                    compatible(s.size, ws.size())
                        && dist(s.place, ws.pos) <= D_MATCH
                        && e.g >= since
                        && self.last_mark_t.is_none_or(|m| e.t >= m)
                        && !(gone && self.first_seen(ws.id) <= s.lost_t)
                })
                .map(|ws| (dist(s.place, ws.pos), ws.id))
                .collect();
            ns.sort_by(by_near);
            let mut ps = Vec::new();
            for &(_, wid) in ns.iter().take(K_PAIR) {
                ps.push(Pair { key: wid, s: chain_score(&mut self.memo, s, &self.store[&wid]) });
            }
            pairs.insert(i, ps);
        }
        // each column's neighbours: (tray index, S, n), nearest first; its hits
        let mut neigh: BTreeMap<i64, Vec<Pair>> = BTreeMap::new();
        let mut hits: BTreeMap<i64, BTreeSet<usize>> = BTreeMap::new();
        let hit = |p: &Pair| p.s.is_some_and(|(sc, n)| sc >= tid && n >= N_ID);
        for ws in &cols {
            let (id, here, size) = (ws.id, ws.pos, ws.size());
            let mut cs: Vec<(f64, usize)> = self
                .stones
                .iter()
                .filter(|o| o.on() && !o.chain.is_empty() && compatible(o.size, size) && dist(o.place, here) <= D_NEIGH)
                .map(|o| (dist(o.place, here), o.index))
                .collect();
            cs.sort_by(by_near);
            let mut ns = Vec::new();
            let mut h = BTreeSet::new();
            for &(_, oi) in cs.iter().take(K_NEIGH) {
                let p = Pair { key: oi as i64, s: chain_score(&mut self.memo, &self.stones[oi], &self.store[&id]) };
                if hit(&p) {
                    h.insert(oi);
                }
                ns.push(p);
            }
            neigh.insert(id, ns);
            h.extend(pairs.iter().filter(|(_, ps)| ps.iter().any(|p| p.key == id && hit(p))).map(|(&i, _)| i));
            hits.insert(id, h);
        }
        let mut twins = BTreeSet::new();
        for (&id, h) in &hits {
            if h.len() >= 2 {
                twins.insert(id);
                self.twins.extend(h.iter().copied());
            }
        }
        // decided pairs
        let mut info = BTreeMap::new();
        let mut best: Option<Best> = None;
        for &(i, _) in &rows {
            let s = &self.stones[i];
            for p in &pairs[&i] {
                let ws = tw.get(p.key).expect("a column");
                let (here, size) = (ws.pos, ws.size());
                let firm = !(s.on() && s.world == Some(p.key))
                    || self.stones.iter().any(|o| {
                        o.index != i
                            && o.status != TrayStoneStatus::Removed
                            && o.chain.is_empty()
                            && compatible(o.size, size)
                            && dist(o.place, here) <= D_NEIGH
                    });
                let bar = if firm { tid.max(T_FIRM) } else { tid };
                let score = |q: &Pair| q.s.map(|x| x.0);
                let mut rivals: Vec<i64> = pairs[&i].iter().filter(|q| q.key != p.key).filter_map(score).collect();
                for (_, ps) in pairs.iter().filter(|(j, _)| **j != i) {
                    rivals.extend(ps.iter().filter(|q| q.key == p.key).filter_map(score));
                }
                rivals.extend(neigh[&p.key].iter().filter(|q| q.key != i as i64).filter_map(score));
                let top = rivals.iter().copied().max();
                info.insert((i, p.key), (bar, top));
                let Some((sc, n)) = p.s else { continue };
                if twins.contains(&p.key)
                    || n < N_ID
                    || sc < bar
                    || rivals.iter().any(|&rv| sc - rv < M_ID)
                    || self.unscored(i, p.key, here, size)
                {
                    continue;
                }
                if best.is_none_or(|b| sc > b.score || (sc == b.score && (i, p.key) < (b.row, b.wid))) {
                    best = Some(Best { row: i, wid: p.key, score: sc, margin: top.map(|x| sc - x), bar });
                }
            }
        }
        Run { rows, cols: cols.iter().map(|w| w.id).collect(), pairs, info, best }
    }

    /// When an open world stone was first seen on this tray: its unidentified record, or its tray
    /// stone's link.
    fn first_seen(&self, id: i64) -> f64 {
        match self.unid.get(&id) {
            Some(u) => u.born.t,
            None => self.stones[self.link[&id]].world_t,
        }
    }

    /// A tray stone without a chain (not tray stone `i`) could be on world stone `wid`: linked to
    /// it, within [`D_NEIGH`] of it, or disturbed since the last snapshot within [`D_MATCH`] of it.
    /// No score rules it out, so no decision on `wid`.
    fn unscored(&self, i: usize, wid: i64, here: P, size: P) -> bool {
        self.stones.iter().any(|o| {
            let d = dist(o.place, here);
            o.index != i
                && o.status != TrayStoneStatus::Removed
                && o.chain.is_empty()
                && compatible(o.size, size)
                && ((o.on() && o.world == Some(wid)) || d <= D_NEIGH || (o.disturbed.is_some() && d <= D_MATCH))
        })
    }

    /// Makes a decided pair: a question settled, or a stone linked by its look (fact identified).
    fn apply(&mut self, best: Best, tw: &TrayWorld, r: TimeRef, sink: &mut Sink<'_>) {
        let (i, id) = (best.row, best.wid);
        let look = self.store[&id].clone();
        if self.stones[i].on() && self.stones[i].world == Some(id) {
            let s = &mut self.stones[i];
            s.question = None;
            set_latest(s, look);
            return;
        }
        let was_lost = self.stones[i].lost();
        let y = self.link.remove(&id);
        if let Some(y) = y {
            self.lose(y, r);
            self.stones[y].lost_by_look = true;
            self.stones[y].look_lost = true;
        }
        let from = self.stones[i].world.filter(|_| self.stones[i].on());
        if let Some(w0) = from {
            self.link.remove(&w0);
            if let Some(ws0) =
                tw.get(w0).filter(|w| !w.under_sheet && !w.held && inside_board(self.board.as_ref(), w.pos))
            {
                self.unid.insert(w0, Unid { freed: true, ..Unid::of(ws0, r) });
            }
        }
        // a column is unidentified or linked
        let world_t = match self.unid.remove(&id) {
            Some(u) => u.born.t,
            None => self.stones[y.expect("a linked column")].world_t,
        };
        let (ws, k) = (tw.get(id).expect("a column"), self.k);
        let s = &mut self.stones[i];
        s.world = Some(id);
        s.status = TrayStoneStatus::On;
        s.touched = Some(r.g);
        s.world_t = world_t;
        s.carried_away = false;
        s.dropped = false;
        s.emptied = false;
        s.place = ws.pos;
        if ws.detected {
            s.seen_t = r.t;
        }
        s.question = None;
        s.by_look = true;
        (s.at_k, s.at) = (k, s.place); // its place in the last snapshot: where the look found it
        set_latest(s, look);
        self.link.insert(id, i);
        self.relinked = true;
        let (why, code) =
            if was_lost { ("found again by its look", "look_found") } else { ("recognised by its look", "look_moved") };
        let fact = TrayFact {
            why: Some(why.to_string()),
            code: Some(code.to_string()),
            from_world: from,
            score: Some(best.score),
            margin: best.margin,
            bar: Some(best.bar),
            ..self.fact(TrayFactKind::Identified, Some(i), r)
        };
        self.say(fact, sink);
    }

    // ---- the open account (spec 4.6.8) ----------------------------------------------------------

    fn open_account(&mut self, tw: &TrayWorld) -> Vec<TrayOpen> {
        let run = self.last_run.take().expect("the leaving's look work ran");
        let rows: BTreeSet<usize> = run.rows.iter().map(|r| r.0).collect();
        let (mut sure_world, mut stone_on) = (BTreeSet::new(), BTreeMap::new());
        for s in self.stones.iter().filter(|s| s.on()) {
            let Some(w) = s.world else { continue };
            stone_on.insert(w, s.id.clone());
            if s.sure {
                sure_world.insert(w);
            }
        }
        let free: Vec<&Ws> = tw
            .stones
            .iter()
            .filter(|w| {
                !w.under_sheet && !w.held && inside_board(self.board.as_ref(), w.pos) && !sure_world.contains(&w.id)
            })
            .collect();
        let mut entries = Vec::new();
        for i in 0..self.stones.len() {
            let s = &self.stones[i];
            let o = if s.added_now {
                self.open_added(s, &run)
            } else if s.left_now {
                self.open_stone(s, TrayFactKind::Removed, &run, &rows, &free, &stone_on, tw)
            } else if s.on() && !s.sure {
                self.open_stone(s, TrayFactKind::Unsure, &run, &rows, &free, &stone_on, tw)
            } else {
                continue;
            };
            self.stones[i].open = Some(o.clone());
            entries.push(o);
        }
        self.last_run = Some(run);
        entries
    }

    #[allow(clippy::too_many_arguments)]
    fn open_stone(
        &self,
        s: &St,
        kind: TrayFactKind,
        run: &Run,
        rows: &BTreeSet<usize>,
        free: &[&Ws],
        stone_on: &BTreeMap<i64, String>,
        tw: &TrayWorld,
    ) -> TrayOpen {
        let i = s.index;
        let removed = kind == TrayFactKind::Removed;
        let covered = tw.stones.iter().any(|ws| !ws.under_sheet && !ws.held && ws.covers(s.place, MARGIN));
        let cause = if !removed {
            s.unsure_code // every unsure fact has its code
        } else if s.look_lost {
            "look_lost"
        } else if s.by_look {
            "by_look"
        } else if self.twins.contains(&i) {
            "twins"
        } else if self.relinked {
            "relinked"
        } else if s.lost_k == self.k {
            "lost_late"
        } else if covered {
            "covered"
        } else if self.rivals(s) {
            "rivals"
        } else if !hand_took(s) {
            "no_hand"
        } else {
            "unchecked"
        };
        let ps: &[Pair] = run.pairs.get(&i).map_or(&[], Vec::as_slice);
        let (look, score) = if s.chain.is_empty() {
            (if s.late { "late" } else { "none" }, None)
        } else if s.boxed {
            ("box", None)
        } else if !rows.contains(&i) || ps.is_empty() {
            ("stale", None)
        } else {
            let keys: Vec<(usize, i64)> = ps.iter().map(|p| (i, p.key)).collect();
            tie_or(run, &keys, ps, "low")
        };
        let score_of = |id: i64| ps.iter().find(|p| p.key == id).and_then(|p| p.s).map(|x| x.0);
        let mut cands = Vec::new();
        if s.on() && s.world.is_none() {
            cands.push(TrayCandidate { kind: "unseen".into(), world_id: None, stone: None, score: None, why: None });
        }
        if let Some(w) = s.world.filter(|_| s.on()) {
            cands.push(TrayCandidate {
                kind: "world".into(),
                world_id: Some(w),
                stone: Some(s.id.clone()),
                score: score_of(w),
                why: None,
            });
        }
        let (mut with_score, mut without): Ranked<i64> = Default::default();
        for ws in free {
            if s.world == Some(ws.id) || !compatible(s.size, ws.size()) || dist(s.place, ws.pos) > LEAVE_D {
                continue;
            }
            match score_of(ws.id) {
                Some(sc) => with_score.push((-sc, ws.id)),
                None => without.push((dist(s.place, ws.pos), ws.id)),
            }
        }
        with_score.sort();
        without.sort_by(by_near);
        for id in with_score.into_iter().map(|x| x.1).chain(without.into_iter().map(|x| x.1)) {
            cands.push(TrayCandidate {
                kind: "world".into(),
                world_id: Some(id),
                stone: stone_on.get(&id).cloned(),
                score: score_of(id),
                why: None,
            });
        }
        let why = if removed && s.carried_away {
            Some("carried_away")
        } else if removed && s.emptied {
            Some("emptied")
        } else if removed && s.dropped {
            Some("dropped")
        } else {
            let since = if removed { s.lost_t } else { s.unsure_from_t };
            self.marks.iter().any(|&(t, p)| t >= since && dist(p, s.place) <= HAND_R).then_some("hand")
        };
        cands.push(TrayCandidate { kind: "gone".into(), why: why.map(String::from), ..TrayCandidate::default() });
        if removed {
            cands.push(TrayCandidate { kind: "unseen".into(), ..TrayCandidate::default() });
        }
        TrayOpen {
            stone: s.id.clone(),
            world_id: s.world,
            kind,
            cause: cause.to_string(),
            look: look.to_string(),
            score,
            hand_near: None,
            seen_at_arrival: None,
            candidates: cands,
        }
    }

    fn open_added(&self, s: &St, run: &Run) -> TrayOpen {
        let u = s.unid_rec.as_ref().expect("an unclear addition has its unidentified record");
        let id = s.world.expect("an unclear addition is linked");
        let doubtful = self.stones.iter().any(|z| {
            z.index != s.index && !z.added_now && compatible(z.size, s.size) && (z.left_now || (z.on() && !trusted(z)))
        });
        let cause = if u.freed {
            "freed"
        } else if u.resized {
            "size_changed"
        } else if self.relinked {
            "relinked"
        } else if doubtful {
            "doubtful"
        } else if u.seen < 2 {
            "seen_once"
        } else {
            "unchecked"
        };
        // the rows' pairs with this world stone, in tray order
        let (mut keys, mut mine) = (Vec::new(), Vec::new());
        for (&i, ps) in &run.pairs {
            for p in ps.iter().filter(|p| p.key == id) {
                keys.push((i, id));
                mine.push(*p);
            }
        }
        let (look, score) = if run.cols.contains(&id) { tie_or(run, &keys, &mine, "new") } else { ("none", None) };
        let hand_near =
            self.marks.iter().any(|&(t, p)| u.born.t - RENAME_T <= t && t <= u.born.t && dist(p, u.place) <= HAND_R);
        let score_of = |i: usize| keys.iter().zip(&mine).find(|(k, _)| k.0 == i).and_then(|(_, p)| p.s).map(|x| x.0);
        let (mut with_score, mut without): Ranked<usize> = Default::default();
        for z in &self.stones {
            if z.index == s.index
                || z.added_now
                || !(z.left_now || (z.on() && !z.sure))
                || !compatible(z.size, s.size)
                || dist(z.place, u.place) > LEAVE_D
            {
                continue;
            }
            match score_of(z.index) {
                Some(sc) => with_score.push((-sc, z.index)),
                None => without.push((dist(z.place, u.place), z.index)),
            }
        }
        with_score.sort();
        without.sort_by(by_near);
        let mut cands: Vec<TrayCandidate> = with_score
            .into_iter()
            .map(|x| x.1)
            .chain(without.into_iter().map(|x| x.1))
            .map(|i| TrayCandidate {
                kind: "stone".into(),
                stone: Some(self.stones[i].id.clone()),
                score: score_of(i),
                ..TrayCandidate::default()
            })
            .collect();
        cands.push(TrayCandidate { kind: "new".into(), ..TrayCandidate::default() });
        TrayOpen {
            stone: s.id.clone(),
            world_id: Some(id),
            kind: TrayFactKind::Added,
            cause: cause.to_string(),
            look: look.to_string(),
            score,
            hand_near: Some(hand_near),
            seen_at_arrival: Some(self.ids_at_arrival.contains(&id)),
            candidates: cands,
        }
    }
}

/// Why the looks could not settle an entry from its pairs (`keys[i]` is pair i's row and world
/// stone): `tie` when a pair reached its bar, with the best such score, else `other`, with the
/// best score (`None` when none).
fn tie_or(run: &Run, keys: &[(usize, i64)], ps: &[Pair], other: &'static str) -> (&'static str, Option<i64>) {
    let (mut ties, mut scored) = (Vec::new(), Vec::new());
    for (k, p) in keys.iter().zip(ps) {
        let Some((sc, n)) = p.s else { continue };
        scored.push(sc);
        if n >= N_ID && sc >= run.info[k].0 {
            ties.push(sc);
        }
    }
    match ties.iter().max() {
        Some(&t) => ("tie", Some(t)),
        None => (other, scored.iter().max().copied()),
    }
}

/// The rules of spec 4.6 for one session: feed it the worlds of the session's frames in order
/// (not revisions), tell it where frames it could not read came (`skipped`), close it when the
/// session ends. Each tray that leaves goes to `closed`.
#[derive(Debug, Clone, Default)]
pub(crate) struct TrayLedger {
    pub(super) bounds: Option<[f64; 4]>,
    visit_seen: BTreeSet<i64>,
    session: i64,
    video: String,
    n: i64,
    tray: Option<OpenTray>,
    /// The frames of a stop not yet long enough for a visit, each with whether frames the rules
    /// could not read came before it.
    stop: Vec<(TrayWorld, bool)>,
    on: bool,
    calm: u32,
    /// The first frame of the calm run.
    calm_g: i64,
    quiet: u32,
    closed: Vec<Tray>,
    /// Frames the rules could not read came before the next world fed (4.6.1).
    hole: bool,
    /// The tray that left last: a speckle answer that comes after it left is for it (spec 4.6.12).
    last_closed: Option<OpenTray>,
}

impl TrayLedger {
    pub(crate) fn new(session: i64, video: &str) -> TrayLedger {
        TrayLedger { session, video: video.to_string(), ..TrayLedger::default() }
    }

    /// The open tray's links now: world id -> tray stone id (empty between trays).
    pub(crate) fn links(&self) -> BTreeMap<i64, String> {
        self.tray.as_ref().map_or_else(BTreeMap::new, |tray| {
            tray.link.iter().map(|(&wid, &i)| (wid, tray.stones[i].id.clone())).collect()
        })
    }

    /// The open tray (`None` between trays).
    pub(crate) fn current(&self) -> Option<Tray> {
        self.tray.as_ref().map(OpenTray::view)
    }

    /// The trays that have left since the last call, oldest first.
    pub(crate) fn take_closed(&mut self) -> Vec<Tray> {
        std::mem::take(&mut self.closed)
    }

    /// Frames the rules could not read came here (a frame without a world, a `g` gap, a `gap` or
    /// `reset` side event, a revision): the next world fed counts as a busy and a push frame
    /// (4.6.1).
    pub(crate) fn skipped(&mut self) {
        self.hole = true;
    }

    /// The world of the session's next frame; its tray facts go to `sink`.
    pub(crate) fn feed(&mut self, w: &WorldLayer, r: TimeRef, sink: &mut Sink<'_>) {
        let hole = std::mem::take(&mut self.hole);
        let tw = TrayWorld::new(w, r);
        if self.retire_sheet(&tw, w.motion == Motion::Static, hole, sink) {
            return;
        }
        if w.motion != Motion::Static {
            self.stop.clear();
            self.on = false;
            return;
        }
        if self.on {
            self.frame(tw, hole, sink);
            return;
        }
        self.stop.push((tw, hole));
        if self.stop[self.stop.len() - 1].0.r.t - self.stop[0].0.r.t < STOP_MIN {
            return;
        }
        self.begin(sink);
        for (tw, hole) in std::mem::take(&mut self.stop) {
            self.frame(tw, hole, sink);
        }
    }

    /// Makes the open tray leave (the session ended, or the replay or stream).
    pub(crate) fn close(&mut self, sink: &mut Sink<'_>) {
        self.close_tray(sink);
        self.stop.clear();
        self.on = false;
    }

    fn begin(&mut self, sink: &mut Sink<'_>) {
        self.visit_seen.clear();
        let unheld: BTreeSet<i64> =
            self.stop[0].0.stones.iter().filter(|s| !s.held && !s.under_sheet).map(|s| s.id).collect();
        let ids: BTreeSet<i64> =
            self.stop[0].0.stones.iter().filter(|s| !s.under_sheet && !s.held && s.detected).map(|s| s.id).collect();
        let goes_on = self
            .tray
            .as_ref()
            // a visit continues the open tray only on the camera's word (spec 4.6.1): at least one of its
            // first frame's detected stones was seen on the tray, and nine tenths of those detected stones
            // were, or nine tenths of all its stones that no hand holds (a jolt can leave most of a board's
            // stones only remembered and rename one of the few the camera sees). A new board under the
            // camera may carry many of the last board's world ids, and a board that drove off may leave its
            // stones remembered (not detected) where it stood.
            .is_some_and(|tr| {
                let seen = ids.iter().filter(|id| tr.ids.contains(id)).count();
                let seen_all = unheld.iter().filter(|id| tr.ids.contains(id)).count();
                seen > 0 && (10 * seen >= 9 * ids.len() || 10 * seen_all >= 9 * unheld.len())
            });
        if !goes_on {
            self.close_tray(sink);
            self.n += 1;
            sink.counts.total += 1;
            self.tray = Some(OpenTray::new(self.session, &self.video, self.n));
        } else if let Some(tr) = self.tray.as_mut().filter(|tr| tr.arrived.is_some()) {
            // the tray goes on after moving frames the rules never read: the stop's first frame counts as a busy and a
            // push frame, so no unseen run from before them is settled by a sight after them (4.6.1)
            tr.unread(self.stop[0].0.r.t);
            tr.finish_omissions();
            tr.last = None;
        }
        self.tray.as_mut().expect("a begun tray").visit =
            VisitEvidence { start_g: self.stop[0].0.r.g, ..VisitEvidence::default() };
        self.on = true;
        self.calm = 0;
        self.quiet = 0;
    }

    fn frame(&mut self, tw: TrayWorld, hole: bool, sink: &mut Sink<'_>) {
        let mut tr = self.tray.take().expect("an open tray");
        self.rules(&mut tr, &tw, hole, sink);
        tr.last = Some(tw);
        self.tray = Some(tr);
    }

    fn rules(&mut self, tr: &mut OpenTray, tw: &TrayWorld, hole: bool, sink: &mut Sink<'_>) {
        let (r, t) = (tw.r, tw.r.t);
        tr.ids.extend(tw.stones.iter().filter(|w| !w.under_sheet).map(|w| w.id));
        self.visit_seen.extend(tw.stones.iter().filter(|w| !w.under_sheet && w.detected).map(|w| w.id));
        tr.store_looks(tw);
        let hand = tw.events.iter().any(Ev::hand);
        let change = tw.events.iter().any(|e| matches!(e.kind, EventKind::New | EventKind::Delete | EventKind::Split))
            && !self.only_background_retired(tr, tw, hole);
        self.quiet = if tw.events.iter().any(|e| e.kind == EventKind::New) { 0 } else { self.quiet + 1 };
        let held = tw.stones.iter().any(|w| w.held);
        tr.omissions(tw, hole);
        let pending = tr.inventory_gap(tw, hole) || !tr.missing.is_empty();
        let quiet_frame = !hole
            && tw.free
            && !tw.untrusted
            && !held
            && !hand
            && !change
            && !tw.events.iter().any(|e| e.kind == EventKind::Push);
        if quiet_frame {
            tr.visit.quiet_frames += 1;
        } else {
            tr.visit.quiet_frames = 0;
            tr.visit.clean = None;
            tr.visit.epoch += 1;
        }
        let stale = tr.calm_ref.is_some_and(|c| c.g < tr.visit.start_g);
        if stale && pending && tr.visit.quiet_frames >= CALM && tr.visit.clean.is_some() {
            tr.visit.fallback.clone_from(&tr.visit.clean);
        }
        if !pending {
            self.calm = if tw.free && !hand && !change { self.calm + 1 } else { 0 };
            if self.calm == 1 {
                self.calm_g = r.g;
            }
        }

        let took = tw.events.iter().any(|e| e.took);
        if !tw.free || hand || tw.untrusted || held || took {
            // a busy frame (4.6.1, presence questions): a hand at work, an untrusted hand, a held world stone, or a
            // stone a hand took away
            tr.busy_t = Some(t);
        }
        if tw.events.iter().any(|e| e.kind == EventKind::Push) {
            tr.push_t = Some(t); // a push frame
        }
        if hole {
            tr.unread(t); // frames the rules could not read came before this one (4.6.1)
        }
        if tr.arrived.is_none() {
            if !pending && self.calm >= CALM && self.quiet >= QUIET && tw.confirmed {
                // looks read before the calm run that ends here may show another stone: hands worked since
                let calm_g = self.calm_g;
                tr.store.retain(|_, e| e.g >= calm_g);
                self.arrive(tr, tw, sink);
                tr.sheet_laid(&(0..tr.stones.len()).collect::<Vec<_>>(), r, sink);
                // the links as it arrived, for the speckle answers (spec 4.6.12)
                tr.arrival_links = tr.link.iter().map(|(&wid, &i)| (wid, tr.stones[i].id.clone())).collect();
                tr.ids_at_arrival = tr.ids.clone();
                tr.look_work(tw, r, sink);
            }
            return;
        }
        tr.observe_sheets(tw, !pending && self.calm >= CALM, sink);
        // 1. where hands worked
        for e in tw.events.iter().filter(|e| e.hand() || e.by_hand) {
            let Some(id) = e.id else { continue };
            if let Some(ws) = tw.get(id) {
                tr.marks.push((t, ws.pos));
            } else if let Some(&i) = tr.link.get(&id) {
                tr.marks.push((t, tr.stones[i].place));
            } else {
                continue;
            }
            tr.last_mark_t = Some(t);
        }
        // 2. the leaving point
        if hand {
            tr.worked = true;
        }
        if !pending && self.calm >= CALM {
            tr.calm_ref = Some(r);
            tr.worked = false;
        }
        // 3. the world's events on linked stones (and the disturbances and lifts the look rules
        //    read)
        let (mut ends, mut late) = (Vec::new(), Vec::new());
        for e in &tw.events {
            let Some(id) = e.id else { continue };
            let Some(&i) = tr.link.get(&id) else {
                if e.kind == EventKind::Push {
                    late.push(id);
                    if let Some(u) = tr.unid.get(&id).filter(|u| u.freed) {
                        let (place, size) = (u.place, u.size);
                        tr.freed_push(place, size, r);
                    }
                }
                continue;
            };
            if e.hand() || e.took {
                tr.stones[i].hand_t = Some(t);
            }
            match e.kind {
                EventKind::Lift | EventKind::Carry | EventKind::Hinge | EventKind::Place => {
                    let s = &mut tr.stones[i];
                    s.handled = true;
                    s.lifted = true;
                    s.disturbed = Some(r.g);
                    s.touched = Some(r.g);
                    s.handled_t = Some(t);
                }
                EventKind::Push => tr.push(i, r),
                EventKind::Gone | EventKind::Drop | EventKind::Delete if !ends.contains(&id) => ends.push(id),
                _ => {}
            }
        }
        for id in ends {
            let i = tr.link.remove(&id).expect("a linked stone");
            let s = &mut tr.stones[i];
            for e in tw.events.iter().filter(|e| e.id == Some(id)) {
                s.carried_away |= e.kind == EventKind::Gone;
                s.dropped |= e.kind == EventKind::Drop;
                s.emptied |= e.by_hand;
            }
            tr.lose(i, r);
        }
        // a busy frame or a push frame spoils the unseen run of a linked stone not detected on it (4.6.1)
        if tr.busy_t == Some(t) || tr.push_t == Some(t) {
            for (&id, &i) in &tr.link {
                if !tr.stones[i].under_sheet && tw.get(id).is_some_and(|w| !w.detected) {
                    for run in runs(&mut tr.stones[i]) {
                        // always later: a stone not detected on this frame was last seen before it (Python's guard)
                        run.spoilt |= t > run.seen_t;
                    }
                }
            }
        }
        // 4. linked stones in this frame's world
        let linked: Vec<(i64, usize)> = tr.link.iter().map(|(&id, &i)| (id, i)).collect();
        for (id, i) in linked {
            if tr.stones[i].under_sheet {
                continue;
            }
            let Some(ws) = tw.get(id) else {
                if !tr.missing.contains_key(&id) {
                    tr.link.remove(&id);
                    tr.lose(i, r);
                }
                continue;
            };
            if ws.held {
                let s = &mut tr.stones[i];
                s.handled = true;
                s.lifted = true;
                s.disturbed = Some(r.g);
                s.touched = Some(r.g);
                s.handled_t = Some(t);
                continue;
            }
            if ws.frozen {
                if tr.stones[i].frozen && !compatible(ws.size(), tr.stones[i].size) {
                    tr.link.remove(&id);
                    tr.doubt(i, "its world stone changed size", "size_changed", r, r.g + 1, sink);
                    tr.lose(i, r);
                    tr.unid.insert(id, Unid { resized: true, ..Unid::of(ws, r) });
                    continue;
                }
                if !tr.stones[i].frozen {
                    tr.stones[i].size = ws.size();
                    tr.stones[i].frozen = true;
                }
            }
            if ws.detected {
                let (busy_t, push_t) = (tr.busy_t, tr.push_t);
                let s = &mut tr.stones[i];
                s.place = ws.pos;
                s.seen_t = t;
                let (place, size) = (s.place, s.size);
                for run in runs(s) {
                    // the run's first sight (4.6.1)
                    if run.again.is_none() && t > run.c1_t && !run.spoilt {
                        run.again = Some(Again {
                            place,
                            busy: busy_t.is_some_and(|b| b >= run.seen_t),
                            push: push_t.is_some_and(|p| p >= run.seen_t),
                            crowd: crowded(size, run, tw),
                        });
                    }
                }
            }
        }
        // 5. world stones no tray stone stands for
        for ws in &tw.stones {
            if tr.link.contains_key(&ws.id) || tr.unid.contains_key(&ws.id) || ws.under_sheet || ws.held {
                continue;
            }
            if inside_board(tr.board.as_ref(), ws.pos) {
                tr.unid.insert(ws.id, Unid::of(ws, r));
            }
        }
        tr.observe_unidentified(tw, hole);
        // 6. renames, then a push on an id linked only now (lost at step 3, renamed at step 6) as
        //    step 3 reads it; 7. the settle at a calm moment, the look work in a later calm frame
        tr.renames(tw, sink);
        let mut pushed = BTreeSet::new();
        for id in late {
            if let Some(&i) = tr.link.get(&id).filter(|_| pushed.insert(id)) {
                tr.push(i, r);
            }
        }
        if pending {
            if tr.visit.quiet_frames == CALM {
                for i in 0..tr.stones.len() {
                    let s = &tr.stones[i];
                    let Some(id) = s.world.filter(|_| !s.under_sheet && s.on() && s.sure) else { continue };
                    if tw.get(id).is_none_or(|w| w.detected) {
                        continue;
                    }
                    let previous = tr.visit.unseen.insert(i, (id, r.g, tr.visit.epoch));
                    if let Some((prior, _, epoch)) = previous
                        && prior == id
                        && epoch != tr.visit.epoch
                    {
                        tr.doubt(i, "not seen at two calm moments", "unseen", r, r.g, sink);
                    }
                }
            }
            return;
        }
        if self.calm == CALM {
            tr.settle(tw, r, false, sink);
        } else if self.calm > CALM {
            tr.look_work(tw, r, sink);
        }
        if stale && quiet_frame && tr.calm_ref.is_some_and(|c| c.g < tr.visit.start_g) {
            let places =
                tr.stones.iter().filter_map(|s| tw.observed_stone(s).map(|w| (s.index, (w.id, w.pos)))).collect();
            tr.visit.clean = Some(Arc::new(VisitSnapshot { at: r, places, world_stones: tw.stones_by_id() }));
        }
        // a calm frame: each stone's place in it, if it was seen in it (spec 4.7)
        if tr.calm_ref.map(|c| c.g) == Some(r.g) {
            tr.calm_stones = tw.stones_by_id();
            for s in &mut tr.stones {
                s.calm_place = tw.observed_stone(s).map(|w| w.pos);
            }
        }
    }

    fn arrive(&mut self, tr: &mut OpenTray, tw: &TrayWorld, sink: &mut Sink<'_>) {
        let r = tw.r;
        tr.arrived = Some(r);
        tr.calm_stones = tw.stones_by_id();
        tr.board = tw.board.clone();
        let st = stamp(r.wall, r.t, self.session);
        for ws in &tw.stones {
            if ws.held
                || (ws.under_sheet && !self.visit_seen.contains(&ws.id))
                || !inside_board(tr.board.as_ref(), ws.pos)
            {
                continue;
            }
            let mut s = tr.new_stone(format!("{st}-{}-{}", tr.n, ws.id), ws, r);
            s.under_sheet = ws.under_sheet;
            s.calm_place = (ws.detected && !ws.under_sheet).then_some(ws.pos);
            tr.link.insert(ws.id, s.index);
            tr.stones.push(s);
        }
        let n = tr.stones.len() as u64;
        sink.counts.arrived += 1;
        sink.counts.stones += n;
        let fact = TrayFact { stones: Some(n), ..tr.fact(TrayFactKind::Arrived, None, r) };
        tr.say(fact, sink);
        tr.snapshot(r);
    }

    /// Makes the open tray leave: its final account (spec 4.6.7), then it goes to `closed`.
    fn close_tray(&mut self, sink: &mut Sink<'_>) {
        self.close_at(None, &[], sink);
    }

    fn close_at(&mut self, boundary: Option<TimeRef>, lifted: &[usize], sink: &mut Sink<'_>) {
        let Some(mut tr) = self.tray.take() else { return };
        let Some(tw) = tr.last.take().filter(|_| tr.arrived.is_some()) else {
            self.closed.push(tr.view()); // it never arrived
            return;
        };
        tr.finish_omissions();
        let fallback =
            tr.visit.fallback.take().filter(|_| !tr.worked && tr.calm_ref.is_some_and(|c| c.g < tr.visit.start_g));
        let leave = fallback.as_ref().map(|s| s.at).or(tr.calm_ref.filter(|_| !tr.worked)).unwrap_or(tw.r);
        let departure = boundary.unwrap_or(leave);
        tr.leaving = Some(departure);
        // each stone's place at the leaving point, if it was seen there (spec 4.7), before the last settle
        let at_calm = tr.calm_ref.is_some() && !tr.worked;
        for s in &mut tr.stones {
            s.left_place = if let Some(snapshot) = &fallback {
                snapshot.places.get(&s.index).filter(|(id, _)| s.world == Some(*id)).map(|(_, place)| *place)
            } else if at_calm {
                s.calm_place
            } else {
                tw.observed_stone(s).map(|w| w.pos)
            };
        }
        // what is unidentified is in the last world (each frame drops the rest): a guard for the pairing; and a stone
        // first seen after the leaving point is not on the tray that leaves (the next tray's, as it comes in)
        tr.unid.retain(|id, u| tw.by_id.contains_key(id) && u.born.g <= leave.g);
        let previous_links = tr.link.clone();
        tr.point = if let Some(snapshot) = &fallback {
            snapshot.world_stones.clone()
        } else if at_calm {
            std::mem::take(&mut tr.calm_stones)
        } else {
            tw.stones_by_id()
        };
        tr.settle(&tw, leave, true, sink);
        for (&id, &i) in &tr.link {
            if previous_links.get(&id) != Some(&i) {
                tr.stones[i].left_place =
                    tr.point.get(&id).filter(|w| !w.under_sheet && w.detected && !w.held).map(|w| w.pos);
            }
        }
        // what the leaving's speckle answer found of each stone (spec 4.6.13)
        let evidence: Vec<TrayEvidence> = tr.stones.iter().map(|s| tr.evidence(s)).collect();
        for (s, e) in tr.stones.iter_mut().zip(evidence) {
            s.evidence = Some(e);
        }
        let covered: Vec<_> = tr.stones.iter().filter(|s| s.on() && s.under_sheet).map(|s| s.index).collect();
        sink.counts.under_sheet += covered.len() as u64;
        tr.sheet_laid(&covered, departure, sink);
        tr.sheet_fact(lifted, "sheet_lifted", departure, sink);
        let count = |f: &dyn Fn(&St) -> bool| tr.stones.iter().filter(|s| f(s)).count() as u64;
        let fact = TrayFact {
            stones: Some(count(&|s| s.on())),
            sure: Some(count(&|s| s.on() && s.sure)),
            added: Some(tr.added),
            removed: Some(count(&|s| s.status == TrayStoneStatus::Removed && s.certain == Some(true))),
            unclear: Some(tr.unclear),
            t_id: tr.took.then(|| tr.tid()),
            open: Some(tr.open.clone()),
            under_sheet: (!covered.is_empty()).then_some(covered.len() as u64),
            ..tr.fact(TrayFactKind::Leaving, None, departure)
        };
        tr.say(fact, sink);
        self.closed.push(tr.view());
        self.last_closed = Some(tr);
    }
}

/// The Python SDK's blink tests (tests/test_tray_blinks.py) and version 7 leaving tests
/// (tests/test_tray_leaving.py), on the ledger itself.
#[cfg(test)]
mod blink_tests;

/// The leaving's speckle answer (spec 4.6.13) on the open tray and the ledger themselves.
#[cfg(test)]
mod speckle_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Solid;

    fn brick(id: i64, x: f64) -> Ws {
        Ws::of(&StoneView {
            id,
            shape: StoneShape::Frozen,
            detected: true,
            solid: Some(Solid {
                pos: vec![x, 0.0, 0.0],
                q: vec![0.0, 0.0, 0.0, 1.0],
                dims: vec![0.3, 0.2, 0.1],
                ..Solid::default()
            }),
            ..StoneView::default()
        })
        .expect("a usable stone")
    }

    #[test]
    fn a_tray_gives_no_id_twice() {
        // a second stone first seen under one world id in one tenth of a second takes -2, a third -3 (spec 4.6)
        let mut tr = OpenTray::new(7, "v", 1);
        for _ in 0..3 {
            let s = tr.new_stone("s7t1.0-1-7".into(), &brick(7, 0.0), TimeRef { g: 5, t: 1.0, wall: None });
            tr.stones.push(s);
        }
        let ids: Vec<&str> = tr.stones.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["s7t1.0-1-7", "s7t1.0-1-7-2", "s7t1.0-1-7-3"]);
    }

    #[test]
    fn stamps() {
        for (wall, want) in [
            (1_794_960_000.84, "20261118T000000.8Z"),
            (1_767_225_600.0, "20260101T000000.0Z"),
            (1_772_323_199.96, "20260301T000000.0Z"),
            (-0.04, "19700101T000000.0Z"),
            (-0.06, "19691231T235959.9Z"),
        ] {
            assert_eq!(stamp(Some(wall), 0.0, 1), want, "{wall}");
        }
        assert_eq!(stamp(None, 22.5, 1_795_314_878), "s1795314878t22.5");
    }

    #[test]
    fn usable_stones_and_turns() {
        let short = StoneView {
            solid: Some(Solid {
                pos: vec![0.0, 0.0],
                q: vec![0.0, 0.0, 0.0, 1.0],
                dims: vec![1.0],
                ..Solid::default()
            }),
            ..StoneView::default()
        };
        assert!(Ws::of(&StoneView::default()).is_none() && Ws::of(&short).is_none());
        let mut flat = brick(1, 0.0);
        flat.zw = [0.0, 0.0];
        assert_eq!(flat.turn(), (1.0, 0.0));
        let fact = TrayFact {
            certain: Some(true),
            why: Some("missing".into()),
            ..OpenTray::new(1, "v", 2).fact(TrayFactKind::Removed, None, TimeRef { g: 1, t: 61.2, wall: None })
        };
        assert_eq!(fact.to_string(), "1:01.2 tray 2: stone ? was removed");
        assert_eq!(fact.clock(), "1:01.2");
    }

    /// When a tray leaves, what is left is paired nearest first (in tray order, then by world id,
    /// at equal distances), never sure; the rest is unclear.
    #[test]
    fn leaving_pairs() {
        let r = TimeRef { g: 1, t: 1.0, wall: None };
        let stones = vec![brick(21, 0.5), brick(22, 0.5), brick(23, 3.0)];
        let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let tw = TrayWorld {
            r,
            stones,
            by_id,
            events: Vec::new(),
            free: true,
            untrusted: false,
            confirmed: true,
            board: None,
        };
        let mut ledger = TrayLedger::new(7, "v");
        ledger.n = 1;
        let mut tr = OpenTray { arrived: Some(r), k: 0, ..OpenTray::new(7, "v", 1) };
        // stones 0 and 1 are 0.5 from 21 and 22; stone 2 is 0.1 from both; 23 is too far from all
        for (i, x) in [0.0, 1.0, 0.6].into_iter().enumerate() {
            let s = tr.new_stone(format!("s{i}"), &brick(i as i64 + 1, x), r);
            tr.stones.push(s);
            tr.lose(i, r);
        }
        for id in [22, 21, 23] {
            tr.unid.insert(id, Unid::of(tw.get(id).expect("in the world"), r));
        }
        tr.last = Some(tw);
        ledger.tray = Some(tr);
        let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
        ledger.close(&mut Sink { counts: &mut counts, facts: &mut facts });
        let lines: Vec<String> = facts.iter().map(TrayFact::describe).collect();
        assert_eq!(
            lines,
            [
                "0:01.0 tray 1: stone s2 is not sure any more (matched when the tray left)",
                "0:01.0 tray 1: stone s0 is not sure any more (matched when the tray left)",
                "0:01.0 tray 1: stone s1 not found (unclear)",
                "0:01.0 tray 1: stone s7t1.0-1-23 not recognised (unclear)",
                "0:01.0 tray 1 leaving: 3 stones (0 sure), 0 added, 0 removed, 2 unclear",
            ]
        );
        let closed = ledger.take_closed();
        assert_eq!((closed[0].stones[2].world_id, closed[0].stones[0].world_id), (Some(21), Some(22)));
        assert_eq!((counts.removed_unclear, counts.added_unclear, counts.unsure), (1, 1, 2));
        assert_eq!(closed[0].facts, facts);
        assert_eq!(closed[0].check(), ["C1"]); // a tray made here, without its arrival fact
    }

    /// Version 5 of the tray rules (spec 4.6.1, Leaving): the leaving pairs a lost tray stone S with
    /// an unidentified world stone W only on evidence. Looks that agree (the tray's bar over at
    /// least N_ID cells) pair them and looks that disagree (below T_DIFF over at least N_ID cells)
    /// keep them apart, whatever the time and the hands; looks in between, too few cells or no look
    /// leave it to the time and the hands: W first seen no earlier than RENAME_T before S's last
    /// sight, and no hand mark near S's place or W's since the earliest of S's last sight, S's loss
    /// and W's first sight, less RENAME_T.
    #[test]
    fn the_leaving_pairs_only_on_evidence() {
        use base64::Engine as _;
        let raw: Vec<u8> = (0..144).map(|i: i64| ((i * 37 % 200) - 100) as i8 as u8).collect();
        let d = base64::engine::general_purpose::STANDARD.encode(&raw);
        let lk = crate::models::StoneLook { v: 1, nu: 12, nv: 12, d, ..Default::default() };
        let cells = Arc::new(super::super::looks::read_look(&lk).expect("a look the rules read"));
        let look = |wid: i64, g: i64| Look {
            wid,
            cells: Arc::clone(&cells),
            g,
            t: 0.0,
            place: [0.0, 0.0],
            size: [0.3, 0.2],
            taken: false,
            detected: true,
        };
        // S: last seen at 10.0, lost at 10.4, at (0, 0), its chain one look of world stone 3 at g 10;
        // W: world stone 7 at (0.1, 0), its stored look read at g 50; the pair's score from the memo
        let verdict = |x: Option<(i64, i64)>, born: f64, mark: Option<(f64, P)>, chain: bool, stored: bool| {
            let mut tr = OpenTray::new(7, "v", 1);
            let chain = if chain { vec![look(3, 10)] } else { Vec::new() };
            tr.stones.push(St { seen_t: 10.0, lost_t: 10.4, chain, ..St::default() });
            let born = TimeRef { g: 0, t: born, wall: None };
            tr.unid.insert(7, Unid { born, place: [0.1, 0.0], ..Unid::default() });
            if stored {
                tr.store.insert(7, look(7, 50));
            }
            tr.memo.insert([7, 50, 3, 10], x);
            tr.marks.extend(mark);
            tr.same_stone(0, 7)
        };
        let agree = Some((T_ID_START, N_ID));
        let disagree = Some((T_DIFF - 1, N_ID));
        assert!(verdict(agree, 7.2, None, true, true) && verdict(agree, 1.0, Some((9.0, [0.0, 0.0])), true, true));
        assert!(!verdict(disagree, 7.2, None, true, true) && !verdict(disagree, 9.0, None, true, true));
        let rest = [
            Some((T_DIFF, N_ID)),
            Some((T_ID_START - 1, N_ID)),
            Some((T_DIFF - 1, N_ID - 1)),
            Some((T_ID_START, N_ID - 1)),
            None,
        ];
        for x in rest {
            let at = |born: f64, mark: Option<(f64, P)>| verdict(x, born, mark, true, true);
            assert!(at(7.2, None), "{x:?}: W first seen 2.8 s before S's last sight, no hand near");
            assert!(at(10.0 - RENAME_T, None), "{x:?}: W first seen RENAME_T before it");
            assert!(!at(10.0 - RENAME_T - 0.1, None), "{x:?}: W first seen earlier than that");
            assert!(!at(7.2, Some((9.0, [HAND_R - 0.05, 0.0]))), "{x:?}: a hand mark near S's place");
            assert!(!at(7.2, Some((9.0, [0.1 + HAND_R - 0.05, 0.0]))), "{x:?}: near W's place only");
            assert!(at(7.2, Some((9.0, [0.1 + HAND_R + 0.05, 0.0]))), "{x:?}: near neither");
            assert!(at(7.2, Some((4.1, [0.0, 0.0]))), "{x:?}: a hand mark before the window (4.2 s)");
            assert!(!at(7.2, Some((4.3, [0.0, 0.0]))), "{x:?}: a hand mark in the window");
        }
        // without S's chain or W's stored look nothing is scored: the time and the hands decide
        assert!(verdict(disagree, 7.2, None, false, true) && verdict(disagree, 7.2, None, true, false));
        assert!(!verdict(disagree, 1.0, None, false, true));
    }

    /// A stone first seen after the leaving point is not on the tray that leaves (the next tray's,
    /// as it comes in): the final account leaves it out, while one first seen before it is an
    /// unclear addition.
    #[test]
    fn a_stone_first_seen_after_the_leaving_point_is_left_out() {
        let at = |g: i64| TimeRef { g, t: g as f64 * 0.2, wall: None };
        let stones = vec![brick(31, 0.0), brick(32, 2.0)];
        let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let tw = TrayWorld {
            r: at(7),
            stones,
            by_id,
            events: Vec::new(),
            free: true,
            untrusted: false,
            confirmed: true,
            board: None,
        };
        let mut ledger = TrayLedger::new(7, "v");
        ledger.n = 1;
        // the leaving point is the calm frame g 5; the account closes on the last frame, g 7
        let mut tr = OpenTray { arrived: Some(at(1)), calm_ref: Some(at(5)), k: 0, ..OpenTray::new(7, "v", 1) };
        tr.unid.insert(31, Unid::of(tw.get(31).expect("in the world"), at(4)));
        tr.unid.insert(32, Unid::of(tw.get(32).expect("in the world"), at(6)));
        tr.last = Some(tw);
        ledger.tray = Some(tr);
        let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
        ledger.close(&mut Sink { counts: &mut counts, facts: &mut facts });
        let lines: Vec<String> = facts.iter().map(TrayFact::describe).collect();
        assert_eq!(
            lines,
            [
                "0:01.0 tray 1: stone s7t0.8-1-31 not recognised (unclear)",
                "0:01.0 tray 1 leaving: 1 stones (0 sure), 0 added, 0 removed, 1 unclear",
            ]
        );
        assert_eq!(counts.added_unclear, 1);
    }

    #[test]
    fn permille_text_and_the_small_helpers() {
        for (p, want) in [(853, "0.853"), (1000, "1.000"), (-5, "-0.005"), (-1000, "-1.000"), (0, "0.000")] {
            assert_eq!(milli(p), want);
        }
        let mut s = St::default();
        assert_eq!(since_of(&s, TimeRef { g: 5, t: 1.0, wall: None }), 5);
        s.disturbed = Some(7);
        assert_eq!(since_of(&s, TimeRef { g: 9, t: 1.8, wall: None }), 8);
        assert_eq!(id_text(None), "?");
        let candidate = |kind: &str| TrayCandidate { kind: kind.into(), ..TrayCandidate::default() };
        assert_eq!(candidate("unseen").describe(), "unseen");
        assert_eq!(candidate("stone").describe(), "stone ?");
        assert_eq!(TrayCandidate { why: Some("hand".into()), ..candidate("gone") }.describe(), "gone (hand)");
    }

    /// The look half of a settle (spec 4.6.1, presence questions): a look of the run's world stone
    /// that reaches the bar against another open stone's chain keeps the question open, but only a
    /// look read after the stone was last seen, and still fresh.
    #[test]
    fn only_a_fresh_look_read_since_the_last_sight_keeps_a_question_open() {
        use base64::Engine as _;
        // 12 x 12 cells, the same in both looks: they match over 144 cells
        let raw: Vec<u8> = (0..144).map(|i: i64| ((i * 37 % 200) - 100) as i8 as u8).collect();
        let d = base64::engine::general_purpose::STANDARD.encode(&raw);
        let lk = crate::models::StoneLook { v: 1, nu: 12, nv: 12, d, ..Default::default() };
        let cells = Arc::new(super::super::looks::read_look(&lk).expect("a look the rules read"));
        // another face (not this one turned or flipped)
        let other: Vec<u8> = (0..144).map(|i: i64| ((i * i * 7 + 3) % 200 - 100) as i8 as u8).collect();
        let lk = crate::models::StoneLook { d: base64::engine::general_purpose::STANDARD.encode(&other), ..lk };
        let other = Arc::new(super::super::looks::read_look(&lk).expect("a look the rules read"));
        let look = |wid: i64, g: i64, t: f64, place: P| Look {
            wid,
            cells: Arc::clone(&cells),
            g,
            t,
            place,
            size: [0.3, 0.2],
            taken: false,
            detected: true,
        };
        let r = TimeRef { g: 10, t: 2.0, wall: None };
        let stones = vec![brick(41, 0.0)];
        let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        let tw = TrayWorld {
            r,
            stones,
            by_id,
            events: Vec::new(),
            free: true,
            untrusted: false,
            confirmed: true,
            board: None,
        };
        // stone 0 is in question on world stone 41, last seen at 1.0 s; stone 1 is lost, and its
        // chain has the same face
        let mut tr = OpenTray::new(7, "v", 1);
        tr.stones.push(St { index: 0, world: Some(41), size: [0.3, 0.2], sure: true, ..St::default() });
        tr.stones.push(St {
            index: 1,
            size: [0.3, 0.2],
            sure: true,
            status: TrayStoneStatus::Lost,
            chain: vec![look(99, 1, 0.2, [1.0, 0.0])],
            ..St::default()
        });
        let w = Watch::new(41, [0.0, 0.0], 1.0, 1.5);
        assert!(!tr.look_against(0, &w, &tw), "no look of its world stone");
        tr.store.insert(41, look(41, 9, 1.8, [0.0, 0.0]));
        assert!(tr.look_against(0, &w, &tw), "read since, fresh: it reaches the bar against stone 1");
        tr.store.insert(41, look(41, 5, 1.0, [0.0, 0.0]));
        assert!(!tr.look_against(0, &w, &tw), "read at the last sight, not since");
        tr.store.insert(41, look(41, 9, 1.8, [0.5, 0.0]));
        assert!(!tr.look_against(0, &w, &tw), "the world stone is no longer where the look was read");
        tr.store.insert(41, look(41, 9, 1.8, [0.0, 0.0]));
        tr.marks.push((1.9, [0.2, 0.0]));
        assert!(!tr.look_against(0, &w, &tw), "a hand mark near its place since the look");
        tr.marks.clear();
        let face =
            std::mem::replace(&mut tr.stones[1].chain, vec![Look { cells: other, ..look(99, 2, 0.2, [1.0, 0.0]) }]);
        let (score, _) = chain_score(&mut Memo::default(), &tr.stones[1], &look(41, 9, 1.8, [0.0, 0.0])).unwrap();
        assert!(score < tr.tid(), "{score}");
        assert!(!tr.look_against(0, &w, &tw), "below the bar against a chain of another face");
        tr.stones[1].chain = face;
        assert!(tr.look_against(0, &w, &tw));
        tr.stones[1].sure = false;
        assert!(!tr.look_against(0, &w, &tw), "no other open stone the look step pairs looks with");
    }

    /// The leaving's own frame (spec 4.6.1, Leaving) is left uncounted only for a stone whose unseen
    /// run is open, with no busy frame and no push frame since its last sight.
    #[test]
    fn the_leaving_leaves_its_own_frame_uncounted_only_after_a_clean_run() {
        let r = TimeRef { g: 10, t: 2.0, wall: None };
        let tw = TrayWorld {
            r,
            stones: Vec::new(),
            by_id: BTreeMap::new(),
            events: Vec::new(),
            free: true,
            untrusted: false,
            confirmed: true,
            board: None,
        };
        let mut tr = OpenTray::new(7, "v", 1);
        tr.stones.push(St { index: 0, world: Some(41), ..St::default() });
        assert!(!tr.clean_since(0, &tw), "no unseen run");
        tr.stones[0].run1 = Some(Watch::new(41, [0.0, 0.0], 1.0, 1.5));
        assert!(tr.clean_since(0, &tw), "an open run, nothing at work since its last sight");
        tr.busy_t = Some(1.0);
        assert!(!tr.clean_since(0, &tw), "a busy frame at its last sight");
        tr.busy_t = Some(0.8);
        tr.push_t = Some(1.2);
        assert!(!tr.clean_since(0, &tw), "a push frame since");
        tr.push_t = None;
        assert!(tr.clean_since(0, &tw), "a busy frame before its last sight");
        tr.marks.push((1.4, [0.3, 0.0]));
        assert!(!tr.clean_since(0, &tw), "a hand near its place since");
        tr.marks.clear();
        tr.stones[0].run1.as_mut().expect("its run").spoilt = true;
        assert!(!tr.clean_since(0, &tw), "a spoilt run");
    }

    /// The leaving (spec 4.6.1, Leaving) reads the tray's last frame as a calm moment of its own when
    /// it is not the last snapshot's frame: a stone that snapshot missed and the last frame misses
    /// too is not sure. On the snapshot's own frame, with nothing at work since the stone's last
    /// sight, that frame is not counted twice.
    #[test]
    fn the_leaving_counts_a_later_calm_frame_of_its_own() {
        let at = |g: i64| TimeRef { g, t: g as f64 * 0.2, wall: None };
        let leave_after_a_snapshot_at = |snapshot_g: i64| -> Vec<String> {
            let mut missing = brick(41, 0.0);
            missing.detected = false;
            let stones = vec![missing, brick(42, 1.0)];
            let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
            let tw = TrayWorld {
                r: at(9),
                stones,
                by_id,
                events: Vec::new(),
                free: true,
                untrusted: false,
                confirmed: true,
                board: None,
            };
            let mut ledger = TrayLedger::new(7, "v");
            ledger.n = 1;
            let mut tr = OpenTray { arrived: Some(at(1)), ..OpenTray::new(7, "v", 1) };
            for (i, x) in [0.0, 1.0].into_iter().enumerate() {
                let s = tr.new_stone(format!("s{i}"), &brick(41 + i as i64, x), at(1));
                tr.link.insert(41 + i as i64, i);
                tr.stones.push(s);
            }
            // stone s0, last seen at g 4, is missed by the snapshot (its unseen run begins there);
            // the tray's last frame, g 9, is calm too
            tr.stones[0].seen_t = at(4).t;
            tr.snapshot(at(snapshot_g));
            let s = &mut tr.stones[0];
            s.ghost = 1;
            s.run1 = Some(Watch::new(41, s.place, s.seen_t, at(snapshot_g).t));
            tr.calm_ref = Some(at(9));
            tr.last = Some(tw);
            ledger.tray = Some(tr);
            let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
            ledger.close(&mut Sink { counts: &mut counts, facts: &mut facts });
            facts.iter().map(TrayFact::describe).collect()
        };
        assert_eq!(
            leave_after_a_snapshot_at(9),
            ["0:01.8 tray 1 leaving: 2 stones (2 sure), 0 added, 0 removed, 0 unclear"]
        );
        assert_eq!(
            leave_after_a_snapshot_at(7),
            [
                "0:01.8 tray 1: stone s0 is not sure any more (not seen at two calm moments; open since 0:01.8)",
                "0:01.8 tray 1 leaving: 2 stones (1 sure), 0 added, 0 removed, 0 unclear",
            ]
        );
    }

    /// A world of bricks along x (ids and places), as a frame's.
    fn world_of(bricks: &[(i64, f64)]) -> TrayWorld {
        let stones: Vec<Ws> = bricks.iter().map(|&(id, x)| brick(id, x)).collect();
        let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        TrayWorld {
            r: TimeRef { g: 10, t: 2.0, wall: None },
            stones,
            by_id,
            events: Vec::new(),
            free: true,
            untrusted: false,
            confirmed: true,
            board: None,
        }
    }

    #[test]
    fn departure_positions_require_detections_on_the_selected_frame() {
        for mode in ["calm", "worked", "arrival"] {
            let mut ledger = TrayLedger::new(1, "v");
            let mut tr = OpenTray::new(1, "v", 1);
            let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
            let mut sink = Sink { counts: &mut counts, facts: &mut facts };
            for g in 0..=12 {
                let mut w = world_of(&[(41, 0.0), (42, 1.0)]);
                w.r = TimeRef { g, t: g.min(11) as f64 / 5.0, wall: None };
                w.stones[0].detected = g != 12;
                w.confirmed = mode != "arrival" || g == 12;
                if mode == "worked" && g == 12 {
                    w.events.push(Ev {
                        why: None,
                        ids: Vec::new(),
                        kind: EventKind::Place,
                        id: Some(42),
                        side: true,
                        by_hand: false,
                        took: false,
                    });
                }
                ledger.rules(&mut tr, &w, false, &mut sink);
                tr.last = Some(w);
            }
            ledger.tray = Some(tr);
            ledger.close(&mut sink);
            let tray = ledger.take_closed().pop().unwrap();
            if mode == "arrival" {
                assert_eq!(tray.arrived.unwrap().g, 12);
            }
            assert_eq!(tray.leaving.unwrap().g, 12);
            assert_eq!((tray.stones[0].place, tray.stones[0].left_place), ([0.0, 0.0], None));
            assert_eq!(tray.stones[1].left_place, Some([1.0, 0.0]));
        }
    }

    #[test]
    fn an_incomplete_settle_in_a_resumed_visit_preserves_its_clean_point_and_negative_evidence() {
        let r = TimeRef { g: 1, t: 0.2, wall: None };
        let mut tr = OpenTray::new(1, "v", 1);
        tr.arrived = Some(r);
        for (id, x) in [(41, 0.0), (42, 1.0)] {
            let s = tr.new_stone(format!("s1t0.2-1-{id}"), &brick(id, x), r);
            tr.link.insert(id, s.index);
            tr.stones.push(s);
        }
        tr.snapshot(r);
        tr.visit.start_g = 10;
        let mut ledger = TrayLedger::new(1, "v");
        let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
        let mut sink = Sink { counts: &mut counts, facts: &mut facts };
        for g in 10..=13 {
            let mut bricks = vec![(41, 0.0), (42, g as f64 / 10.0)];
            if g < 12 {
                bricks.push((43, 2.0));
            }
            let mut w = world_of(&bricks);
            w.r = TimeRef { g, t: g as f64 / 5.0, wall: None };
            w.stones[0].detected = false;
            if g == 13 {
                w.events.push(Ev {
                    why: None,
                    ids: Vec::new(),
                    kind: EventKind::New,
                    id: Some(999),
                    side: false,
                    by_hand: false,
                    took: false,
                });
            }
            ledger.rules(&mut tr, &w, false, &mut sink);
            tr.last = Some(w);
        }
        ledger.tray = Some(tr);
        ledger.close(&mut sink);
        let t = ledger.take_closed().pop().unwrap();
        assert_eq!(t.leaving.unwrap().g, 11);
        assert_eq!(t.stones[1].left_place, Some([1.1, 0.0]));
        assert!(!t.stones[0].sure);
        assert!(
            t.facts.iter().any(|f| f.stone.as_deref() == Some("s1t0.2-1-41") && f.code.as_deref() == Some("unseen"))
        );
    }

    #[test]
    fn a_positive_boundary_sight_can_only_add_an_unidentified_stone_as_unclear() {
        for case in [
            "clean",
            "later sight",
            "time went back",
            "never seen before",
            "hand",
            "hole",
            "replacement",
            "drift",
            "new neighbour",
            "new calm",
        ] {
            let mut ledger = TrayLedger::new(7, "v");
            let mut tr = OpenTray::new(7, "v", 1);
            let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
            let mut sink = Sink { counts: &mut counts, facts: &mut facts };
            for g in 0..=if case == "new calm" { 19 } else { 17 } {
                let mut bricks = vec![(42, 0.6)];
                if g != 15 {
                    bricks.push((41, -0.6));
                }
                let id = if case == "replacement" && g >= 16 { 44 } else { 43 };
                if g >= 10 {
                    bricks.push((id, if case == "drift" && g >= 16 { 0.3 } else { 0.0 }));
                }
                if case == "new neighbour" && g >= 16 {
                    bricks.push((45, 0.1));
                }
                let mut w = world_of(&bricks);
                w.r = TimeRef { g, t: g as f64 / 5.0, wall: None };
                if case == "time went back" && g == 16 {
                    w.r.t = 2.9; // The positive sight was at g15, t=3.0.
                }
                if let Some(i) = w.by_id.get(&id) {
                    w.stones[*i].detected = (case != "never seen before" && matches!(g, 10 | 11)) || g == 15;
                    if case == "later sight" && g == 16 {
                        w.stones[*i].detected = true;
                        w.stones[*i].pos = [0.04, 0.0];
                    }
                }
                if g == 16 {
                    w.events.push(Ev {
                        why: None,
                        ids: Vec::new(),
                        kind: EventKind::New,
                        id: Some(100),
                        side: false,
                        by_hand: false,
                        took: false,
                    });
                    if case == "hand" {
                        w.events.push(Ev {
                            why: None,
                            ids: Vec::new(),
                            kind: EventKind::Place,
                            id: Some(42),
                            side: true,
                            by_hand: false,
                            took: false,
                        });
                    }
                }
                ledger.rules(&mut tr, &w, case == "hole" && g == 16, &mut sink);
                tr.last = Some(w);
            }
            let supported = matches!(case, "clean" | "later sight");
            assert_eq!(tr.unid.get(&43).and_then(|u| u.boundary.as_ref()).is_some(), supported, "{case}");
            if supported {
                ledger.tray = Some(tr);
                ledger.close(&mut sink);
                let tray = ledger.take_closed().pop().unwrap();
                let s = tray.stones.iter().find(|s| s.world_id == Some(43)).unwrap();
                assert!(s.added && !s.sure && s.certain == Some(false));
                assert_eq!(s.left_place, None);
                assert_eq!(s.place, if case == "later sight" { [0.04, 0.0] } else { [0.0, 0.0] });
                let leaving = tray.facts.last().unwrap();
                assert_eq!(
                    (leaving.g, leaving.stones, leaving.added, leaving.unclear),
                    (14, Some(3), Some(0), Some(1))
                );
            }
        }
    }

    #[test]
    fn a_short_inventory_gap_postpones_arrival_without_granting_an_identity() {
        let mut ledger = TrayLedger::new(7, "v");
        let mut tr = OpenTray::new(7, "v", 1);
        let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
        for g in 0..=7 {
            let mut tw = if (4..=6).contains(&g) { world_of(&[(41, 0.0)]) } else { world_of(&[(41, 0.0), (42, 1.0)]) };
            tw.r = TimeRef { g, t: g as f64 / 5.0, wall: None };
            if let Some(w) = tw.stones.get_mut(1) {
                w.detected = false;
            }
            ledger.rules(&mut tr, &tw, false, &mut Sink { counts: &mut counts, facts: &mut facts });
            if g <= 5 {
                assert!(tr.arrived.is_none(), "g {g}");
                assert!(tr.link.is_empty());
            } else {
                assert_eq!(tr.arrived.unwrap().g, 6, "the absent id expired on the third observation");
                assert_eq!(tr.link.keys().copied().collect::<Vec<_>>(), [41]);
            }
            tr.last = Some(tw);
        }
    }

    #[test]
    fn a_departure_records_new_uncertainty_and_preserves_an_existing_doubt_after_a_sheet_lift() {
        let tw = world_of(&[]);
        for sure in [false, true] {
            let mut tr = OpenTray::new(1, "v", 1);
            tr.last_run = Some(Run::default());
            tr.stones.push(St {
                id: "left slab".into(),
                sheet_loss: true,
                status: TrayStoneStatus::Lost,
                sure,
                unsure_code: if sure { "" } else { "pushed" },
                ..St::default()
            });
            let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
            tr.leave(&tw, tw.r, &mut Sink { counts: &mut counts, facts: &mut facts });
            assert_eq!(counts.unsure, u64::from(sure));
            assert_eq!(facts.len(), usize::from(sure));
            assert!(tr.stones[0].on());
            assert_eq!(tr.open[0].cause, if sure { "unseen" } else { "pushed" });
            assert_eq!(tr.open[0].candidates[0].kind, "unseen");
        }
    }

    #[test]
    fn only_the_retirement_of_unobserved_background_can_count_as_a_quiet_arrival() {
        let mut ledger = TrayLedger::new(1, "v");
        ledger.bounds = Some([0., 0., 100., 100.]);
        let mut previous = world_of(&[(1, 0.), (2, 1.), (9, 2.)]);
        for s in &mut previous.stones {
            s.outline = Some([1., 1., 5., 5.]);
            s.mode = StoneMode::Seen;
            s.detected = true;
        }
        let old = &mut previous.stones[2];
        old.outline = Some([110., 1., 115., 5.]);
        old.mode = StoneMode::Kept;
        old.detected = false;
        let mut next = previous.clone();
        next.r.g += 1;
        next.stones.pop();
        next.by_id.remove(&9);
        let event = |kind, id, why: &str| Ev {
            kind,
            id: Some(id),
            why: Some(why.into()),
            ids: Vec::new(),
            side: false,
            by_hand: false,
            took: false,
        };
        next.events.push(event(EventKind::Confirm, 1, ""));
        next.events.push(event(EventKind::Delete, 9, "outside"));
        let mut tr = OpenTray::new(1, "v", 1);
        tr.last = Some(previous);
        assert!(ledger.only_background_retired(&tr, &next, false));
        assert!(!ledger.only_background_retired(&tr, &next, true));
        ledger.visit_seen.insert(9);
        assert!(!ledger.only_background_retired(&tr, &next, false));
        ledger.visit_seen.clear();
        for e in [
            event(EventKind::Delete, 9, "absent"),
            event(EventKind::Delete, 99, "outside"),
            event(EventKind::New, 3, ""),
        ] {
            next.events[1] = e;
            assert!(!ledger.only_background_retired(&tr, &next, false));
        }
    }

    #[test]
    fn quiet_omissions_preserve_identity_and_birth_only_on_a_clean_return() {
        let frame = |g: i64, bricks: &[(i64, f64)]| {
            let mut w = world_of(bricks);
            w.r = TimeRef { g, t: g as f64 / 5.0, wall: None };
            w
        };
        let start = frame(10, &[(41, 0.0), (42, 1.0)]);
        let mut initial = OpenTray::new(7, "v", 1);
        initial.arrived = Some(start.r);
        initial.link.insert(41, 0);
        initial.stones.push(initial.new_stone("s7t2.0-1-41".into(), &start.stones[0], start.r));
        initial.unid.insert(42, Unid::of(&start.stones[1], start.r));
        initial.last = Some(start.clone());
        let advance = |tr: &mut OpenTray, tw: TrayWorld, hole: bool| {
            tr.omissions(&tw, hole);
            tr.last = Some(tw);
        };
        let mut tr = initial.clone();
        advance(&mut tr, frame(11, &[]), false);
        assert_eq!(tr.missing.len(), 2);
        assert!(tr.unid.is_empty());
        assert!(tr.stones[0].on());
        assert_eq!(tr.stones[0].seen_t, start.r.t);
        advance(&mut tr, frame(12, &[]), false);
        assert_eq!(tr.missing.len(), 2);
        let mut returned = frame(13, &[(41, 0.0), (42, 1.0)]);
        returned.events.push(Ev {
            why: None,
            ids: Vec::new(),
            kind: EventKind::Confirm,
            id: Some(41),
            side: false,
            by_hand: false,
            took: false,
        });
        advance(&mut tr, returned.clone(), false);
        assert_eq!(tr.unid[&42].born, start.r);
        assert_eq!(tr.link[&41], 0);
        assert!(tr.missing.is_empty());

        for g in [11, 12, 13] {
            let mut tr = initial.clone();
            for n in 11..=g {
                let mut stones = if n == 13 { vec![(41, 0.0)] } else { vec![] };
                if n == g {
                    stones.push((43, 0.1));
                }
                advance(&mut tr, frame(n, &stones), false);
            }
            assert!(!tr.missing.contains_key(&41), "a new rival on g {g}");
            if g != 11 {
                assert!(tr.stones[0].lost());
            }
        }

        let mut tr = initial.clone();
        tr.last = Some(frame(10, &[(41, 0.0), (42, 1.0), (43, 0.1)]));
        advance(&mut tr, frame(11, &[(43, 0.1)]), false);
        advance(&mut tr, frame(12, &[(41, 0.0), (43, 0.1)]), false);
        assert!(tr.stones[0].on(), "an existing neighbour stayed in place");

        // A different stone may be put down nearby. Free hands in the frame do not erase the event.
        // Check both endpoints: the last observation before the gap and the first one after it.
        for (kind, took) in [(EventKind::Place, false), (EventKind::Push, false), (EventKind::Delete, true)] {
            let event = Ev { why: None, ids: Vec::new(), kind, id: Some(52), side: false, by_hand: false, took };
            let mut tr = initial.clone();
            tr.last.as_mut().unwrap().events.push(event.clone());
            advance(&mut tr, frame(11, &[]), false);
            assert!(tr.missing.is_empty(), "hand work before the omission: {event:?}");

            let mut tr = initial.clone();
            advance(&mut tr, frame(11, &[]), false);
            let mut recovery = frame(12, &[(41, 0.0), (42, 1.0), (52, 0.1)]);
            recovery.events.push(event);
            advance(&mut tr, recovery, false);
            assert!(tr.stones[0].lost());
            assert!(tr.unid.is_empty());
        }

        let mut tr = initial.clone();
        advance(&mut tr, frame(11, &[]), false);
        let mut recovery = frame(12, &[(41, 0.0), (42, 1.0), (53, 1.4)]);
        recovery.events.push(Ev {
            why: None,
            ids: Vec::new(),
            kind: EventKind::New,
            id: Some(53),
            side: false,
            by_hand: false,
            took: false,
        });
        advance(&mut tr, recovery, false);
        assert_eq!(tr.link[&41], 0);
        assert_eq!(tr.unid[&42].born, start.r);

        for (why, g, x, hole) in [
            ("third absence", 13, None, false),
            ("new position", 13, Some(0.4), false),
            ("skipped frames", 14, Some(0.0), false),
            ("unread world", 13, Some(0.0), true),
        ] {
            let mut tr = initial.clone();
            advance(&mut tr, frame(11, &[]), false);
            advance(&mut tr, frame(12, &[]), false);
            let bricks: Vec<_> = x.map(|x| (41, x)).into_iter().collect();
            advance(&mut tr, frame(g, &bricks), hole);
            assert!(tr.link.is_empty(), "{why}");
            assert!(tr.stones[0].lost(), "{why}");
            assert_eq!(tr.stones[0].lost_g, 11, "{why}");
        }

        let mut replaced = initial.clone();
        advance(&mut replaced, frame(11, &[(42, 1.0), (99, 0.1)]), false);
        assert!(replaced.missing.is_empty(), "a new occupant precludes bridging the first omission");

        let mut neighbours = initial.clone();
        neighbours.last = Some(frame(10, &[(41, 0.0), (42, 0.1)]));
        advance(&mut neighbours, frame(11, &[]), false);
        advance(&mut neighbours, frame(12, &[]), false);
        advance(&mut neighbours, frame(13, &[(41, 0.0), (42, 0.1)]), false);
        assert!(neighbours.stones[0].lost(), "a nearby competitor returned after disappearing");
        assert!(neighbours.unid.is_empty(), "the unidentified neighbour's birth cannot be carried through the gap");
        for (returned_x, why) in
            [(0.1, "crossed"), (0.05, "equidistant"), (0.14, "a rival outside the original 0.15 radius")]
        {
            let mut swapped = initial.clone();
            let old_x = if returned_x == 0.14 { 0.22 } else { 0.1 };
            swapped.last = Some(frame(10, &[(41, 0.0), (42, old_x)]));
            advance(&mut swapped, frame(11, &[]), false);
            advance(&mut swapped, frame(12, &[(41, returned_x)]), false);
            assert!(swapped.stones[0].lost(), "{why} return cannot inherit certainty");
        }
        for return_present in [false, true] {
            let mut tr = initial.clone();
            advance(&mut tr, frame(11, &[]), false);
            let mut bricks = vec![(99, 0.1)];
            if return_present {
                bricks.push((41, 0.0));
            }
            advance(&mut tr, frame(12, &bricks), false);
            assert!(tr.stones[0].lost(), "a new occupant during or at the end of the gap breaks continuity");
        }

        let mut closing = initial.clone();
        advance(&mut closing, frame(11, &[]), false);
        closing.finish_omissions();
        assert!(closing.stones[0].lost());
        assert_eq!(closing.stones[0].lost_g, 11);
        assert!(closing.missing.is_empty() && closing.unid.is_empty());

        let mut pushed = frame(12, &[(41, 0.0), (42, 1.0)]);
        pushed.events.push(Ev {
            why: None,
            ids: Vec::new(),
            kind: EventKind::Push,
            id: Some(99),
            side: false,
            by_hand: false,
            took: false,
        });
        let mut tr = initial.clone();
        advance(&mut tr, frame(11, &[]), false);
        advance(&mut tr, pushed.clone(), false);
        assert!(tr.stones[0].lost(), "a push on the return frame breaks continuity");
        assert!(tr.unid.is_empty());
        let mut tr = initial.clone();
        tr.last = Some(pushed);
        advance(&mut tr, frame(13, &[]), false);
        assert!(tr.missing.is_empty(), "a push before disappearance precludes a grace period");

        let mut tr = initial;
        advance(&mut tr, frame(11, &[]), false);
        let mut ended = frame(12, &[]);
        ended.events.push(Ev {
            why: None,
            ids: Vec::new(),
            kind: EventKind::Gone,
            id: Some(41),
            side: false,
            by_hand: false,
            took: false,
        });
        advance(&mut tr, ended, false);
        assert!(tr.missing.is_empty());
        assert!(tr.unid.is_empty());
        assert_eq!(tr.link[&41], 0, "step 3 still sees the explicit ending");
    }

    /// The sight half of a settle (spec 4.6.1, presence questions): the run's world stone was first
    /// seen again after the run's first calm moment, on the stone's place and with nothing at work
    /// there, and the stone is on that world stone, on its place, with no other stone of its size
    /// close by.
    #[test]
    fn the_sight_half_of_a_settle() {
        let tw = world_of(&[(41, 0.05), (44, 1.0)]);
        let again = Again { place: [0.02, 0.0], busy: false, push: false, crowd: false };
        let w = Watch { again: Some(again), ..Watch::new(41, [0.0, 0.0], 1.0, 1.5) };
        let s = St {
            status: TrayStoneStatus::On,
            world: Some(41),
            place: [0.05, 0.0],
            size: [0.3, 0.2],
            seen_t: 1.8,
            ..St::default()
        };
        assert!(seen_again(&s, &w, &tw));
        let first = |a: Option<Again>| Watch { again: a, ..w.clone() };
        assert!(!seen_again(&s, &first(None), &tw), "not seen since the run's first calm moment");
        for (why, a) in [
            ("first seen elsewhere", Again { place: [0.2, 0.0], ..again }),
            ("a busy first sight", Again { busy: true, ..again }),
            ("a first sight after a push", Again { push: true, ..again }),
            ("a first sight in a crowd", Again { crowd: true, ..again }),
        ] {
            assert!(!seen_again(&s, &first(Some(a)), &tw), "{why}");
        }
        for (why, other) in [
            ("lost", St { status: TrayStoneStatus::Lost, ..s.clone() }),
            ("on another world stone", St { world: Some(44), ..s.clone() }),
            ("last seen at the run's first calm moment", St { seen_t: 1.5, ..s.clone() }),
            ("no longer on its place", St { place: [0.2, 0.0], ..s.clone() }),
        ] {
            assert!(!seen_again(&other, &w, &tw), "{why}");
        }
        assert!(!seen_again(&s, &w, &world_of(&[(41, 0.05), (43, 0.1)])), "a stone of its size close by");
    }

    /// The watch of an unseen doubt (spec 4.6.1, presence questions): a copy of the stone's unseen
    /// run, none when only a look settles its question, when another cause of the settle's step 1
    /// holds, or when something could have changed the stone since its last sight.
    #[test]
    fn the_watch_of_an_unseen_doubt() {
        use base64::Engine as _;
        let raw: Vec<u8> = (0..144).map(|i: i64| ((i * 37 % 200) - 100) as i8 as u8).collect();
        let d = base64::engine::general_purpose::STANDARD.encode(&raw);
        let lk = crate::models::StoneLook { v: 1, nu: 12, nv: 12, d, ..Default::default() };
        let cells = Arc::new(super::super::looks::read_look(&lk).expect("a look the rules read"));
        let face =
            Look { wid: 41, cells, g: 1, t: 0.2, place: [0.0, 0.0], size: [0.3, 0.2], taken: false, detected: true };
        let tw = world_of(&[(41, 0.0), (42, 0.3)]);
        let on = |index: usize, world: i64, x: f64| St {
            index,
            status: TrayStoneStatus::On,
            sure: true,
            world: Some(world),
            place: [x, 0.0],
            at: [x, 0.0],
            size: [0.3, 0.2],
            ..St::default()
        };
        let run = Watch::new(41, [0.0, 0.0], 1.0, 1.5);
        let tray = |change: &dyn Fn(&mut OpenTray)| {
            let mut tr = OpenTray::new(7, "v", 1);
            tr.stones = vec![St { run1: Some(run.clone()), ..on(0, 41, 0.0) }, on(1, 42, 0.3)];
            change(&mut tr);
            tr
        };
        let watch = tray(&|_| {}).watch_of(0, 0, &[], &tw).expect("a watch");
        assert_eq!((watch.wid, watch.place, watch.seen_t, watch.c1_t, watch.spoilt), (41, [0.0, 0.0], 1.0, 1.5, false));
        let held = |tr: &mut OpenTray| tr.stones[0].handled = true;
        assert!(tray(&held).watch_of(0, 0, &[], &tw).is_some(), "handled, and no stone of its size appeared");
        let near = [([0.1, 0.0], [0.3, 0.2])];
        assert!(tray(&held).watch_of(0, 0, &near, &tw).is_none(), "a stone of its size appeared next to it while held");
        type Change<'a> = &'a dyn Fn(&mut OpenTray);
        let none: [(&str, Change); 8] = [
            ("no unseen run", &|tr| tr.stones[0].run1 = None),
            ("a look chain", &|tr| tr.stones[0].chain = vec![face.clone()]),
            ("on another world stone", &|tr| tr.stones[0].world = Some(42)),
            ("a spoilt run", &|tr| tr.stones[0].run1.as_mut().expect("its run").spoilt = true),
            ("pushed", &|tr| tr.stones[0].pushed = true),
            ("a world stone a look unlinked was pushed next to it", &|tr| tr.stones[0].freed_push = true),
            ("a stone of its size next to it was pushed", &|tr| {
                tr.stones[1].pushed = true;
                tr.stones[1].at_k = 0;
            }),
            ("handled since its last sight", &|tr| tr.stones[0].handled_t = Some(1.2)),
        ];
        for (why, change) in none {
            assert!(tray(change).watch_of(0, 0, &[], &tw).is_none(), "{why}");
        }
    }
}
