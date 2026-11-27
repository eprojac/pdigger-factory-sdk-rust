//! The unit's speckle answers, read as a tray's (spec 4.6.12), and the leaving's answer (spec
//! 4.6.13). The tray rules read only the leaving's: its pairs confirm, relink and find stones
//! again, and it gives each stone its evidence; the other answers change no fact, count, report or
//! open account.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{OpenTray, P, Sink, St, TrayEvidence, TrayFact, TrayFactKind, TrayLedger, TrayPoint, TrayStoneStatus};
use super::{TrayWorld, Unid, inside_board, trusted};
use crate::interpreter::look_permille;
use crate::interpreter::text::{clock, fmt};
use crate::models::{SpeckleFind, WorldSpeckle};

/// A clean find's least score, in permille (the score rounded as the tray rules round a look's
/// score, [`look_permille`]): a clean find's score is also at least twice its second.
pub const SPECKLE_CLEAN: i64 = 300;

/// How many frames an answer's reference may lie from its tray's arrival for the tray's links as
/// it arrived to stand for the reference's, and the later moment of the leaving's answer from the
/// tray's leaving point (spec 4.6.13).
pub const SPECKLE_NEAR: i64 = 2;

/// The codes of the `identified` facts of the leaving's speckle answer (spec 4.6.13): a stone
/// confirmed on its world stone, recognised on another, found again.
pub const SPECKLE_CODES: [&str; 3] = ["speckle_same", "speckle_moved", "speckle_found"];

/// How many answers [`InterpreterCore::speckle`](crate::InterpreterCore::speckle) keeps (the
/// newest).
pub const SPECKLE_KEPT: usize = 1024;

/// The moment a [`StoneSpeckle`] read its stone at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpeckleSide {
    /// The stone as it was at the answer's reference, checked at its later moment.
    Arrival,
    /// The stone as it was at the later moment, checked at the reference.
    Later,
}

impl SpeckleSide {
    /// `arrival` or `later`.
    pub fn as_str(self) -> &'static str {
        match self {
            SpeckleSide::Arrival => "arrival",
            SpeckleSide::Later => "later",
        }
    }
}

/// A tray stone in one of the unit's speckle answers (spec 4.6.12).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoneSpeckle {
    /// The tray stone's id.
    pub stone: String,
    /// The moment it was read at.
    pub side: SpeckleSide,
    /// The score as the unit sent it (`None`: the stone was not checked).
    pub score: Option<f64>,
    /// The second score as the unit sent it (`None`: not checked; -1: no second place).
    pub second: Option<f64>,
    /// The score is at least 0.300 and at least twice the second (in permille:
    /// [`SPECKLE_CLEAN`]).
    pub clean: bool,
    /// The tray stone that stood for `found_world` at the other moment (`None` when none did).
    pub found: Option<String>,
    /// The world stone at the other moment the best place lies on (`None` when none).
    pub found_world: Option<i64>,
    /// The tray stone the answer pairs it with (`None` when there is none).
    pub pair: Option<String>,
}

impl StoneSpeckle {
    /// The answer pairs the stone with itself: the same stone at both moments.
    pub fn same(&self) -> bool {
        self.pair.as_deref() == Some(self.stone.as_str())
    }
}

/// One of the unit's speckle answers, read as a tray's (spec 4.6.12): the tray, the frame it came
/// in, the two moments it compares and the tray stones in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraySpeckle {
    /// The SDK's tray number ([`Tray`](super::Tray)'s `n`).
    pub tray: i64,
    /// Its session.
    pub session: i64,
    /// Its video.
    pub video: String,
    /// The frame the answer came in.
    pub g: i64,
    /// That frame's media time.
    pub t: f64,
    /// That frame's wall time (`None` without a clock state).
    pub wall: Option<f64>,
    /// The answer's own tray, the unit's count.
    pub unit_tray: i64,
    /// The reference moment, the tray's arrival (`g`, `t`).
    pub reference: TrayPoint,
    /// The later moment (`g`, `t`).
    pub at: TrayPoint,
    /// The GPU the unit checked on.
    pub device: String,
    /// GPU time of the searches, ms.
    pub gpu_ms: f64,
    /// Time the unit's speckle service spent on the request, ms.
    pub wall_ms: f64,
    /// The tray stones in the answer, in tray order.
    pub stones: Vec<StoneSpeckle>,
}

impl TraySpeckle {
    /// The answer for that tray stone.
    pub fn stone(&self, id: &str) -> Option<&StoneSpeckle> {
        self.stones.iter().find(|s| s.stone == id)
    }

    /// `tray 2: speckle of 0:22.5 and 2:24.2: 42 stones, 33 clean, 26 paired, 24 the same`: the
    /// stones, those clean, those with a pair and those paired with themselves.
    pub fn describe(&self) -> String {
        let count = |f: fn(&StoneSpeckle) -> bool| self.stones.iter().filter(|s| f(s)).count();
        format!(
            "tray {}: speckle of {} and {}: {} stones, {} clean, {} paired, {} the same",
            self.tray,
            clock(self.reference.t),
            clock(self.at.t),
            self.stones.len(),
            count(|s| s.clean),
            count(|s| s.pair.is_some()),
            count(StoneSpeckle::same)
        )
    }
}

impl std::fmt::Display for TraySpeckle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

/// The golden `speckle.txt` form of answers (spec 4.6.12): for each, `{g} {clock} ` and its
/// [`describe`](TraySpeckle::describe); then for each of its stones two spaces, the stone, its
/// side, its score and second to 3 places (`-` when not checked), ` clean` when clean,
/// ` found {tray stone}` (or ` found world {id}` when no tray stone stood there), ` pair {tray
/// stone}` when it has one; a newline after every line.
pub fn describe_speckle_txt(answers: &[TraySpeckle]) -> String {
    let mut out = String::new();
    for a in answers {
        out += &format!("{} {} {}\n", a.g, clock(a.t), a.describe());
        for e in &a.stones {
            let scores = match (e.score, e.second) {
                (Some(s), Some(q)) => format!("{} {}", fmt(s, 3), fmt(q, 3)),
                _ => "-".to_string(),
            };
            out += &format!("  {} {} {scores}", e.stone, e.side.as_str());
            if e.clean {
                out += " clean";
            }
            if let Some(f) = &e.found {
                out += &format!(" found {f}");
            } else if let Some(w) = e.found_world {
                out += &format!(" found world {w}");
            }
            if let Some(p) = &e.pair {
                out += &format!(" pair {p}");
            }
            out.push('\n');
        }
    }
    out
}

/// The links to these tray stones (world id -> tray stone id).
fn own(stones: &[St], links: BTreeMap<i64, String>) -> BTreeMap<i64, String> {
    links.into_iter().filter(|(_, sid)| stones.iter().any(|s| &s.id == sid)).collect()
}

/// A clean find (spec 4.6.12): its score at least [`SPECKLE_CLEAN`] and at least twice its
/// second, in permille.
fn speckle_clean(x: &SpeckleFind) -> bool {
    match (x.score, x.second) {
        (Some(s), Some(q)) => look_permille(s) >= SPECKLE_CLEAN && look_permille(s) >= 2 * look_permille(q),
        _ => false,
    }
}

/// A tray stone's entry: find `x`, read with the links at the other moment (`other`) and its
/// partner's world id there.
fn entry(
    sid: &str,
    side: SpeckleSide,
    x: &SpeckleFind,
    other: &BTreeMap<i64, String>,
    pair: Option<i64>,
) -> StoneSpeckle {
    StoneSpeckle {
        stone: sid.to_string(),
        side,
        score: x.score,
        second: x.second,
        clean: speckle_clean(x),
        found: x.at.and_then(|w| other.get(&w).cloned()),
        found_world: x.at,
        pair: pair.and_then(|w| other.get(&w).cloned()),
    }
}

/// What the leaving's answer `sp` says of world id `wid` on its `side` (`refs` for an arrival
/// world id, `stones` for a later one; spec 4.6.13): found (a clean find, or one of the unit's
/// pairs), not found (checked, and no find of [`SPECKLE_CLEAN`] or more) or undecided (no answer,
/// the stone not in it or not checked, or a find of [`SPECKLE_CLEAN`] or more that is not twice its
/// second: look-alikes).
pub(super) fn find_evidence(sp: Option<&WorldSpeckle>, side: SpeckleSide, wid: Option<i64>) -> TrayEvidence {
    let (Some(sp), Some(wid)) = (sp, wid) else { return TrayEvidence::Undecided };
    let finds = if side == SpeckleSide::Arrival { &sp.refs } else { &sp.stones };
    let Some(x) = finds.iter().find(|x| x.id == wid) else { return TrayEvidence::Undecided };
    let Some(score) = x.score else { return TrayEvidence::Undecided };
    let paired = sp.pairs.iter().any(|&[c, r]| wid == if side == SpeckleSide::Arrival { r } else { c });
    if paired || speckle_clean(x) {
        TrayEvidence::Found
    } else if look_permille(score) >= SPECKLE_CLEAN {
        TrayEvidence::Undecided
    } else {
        TrayEvidence::NotFound
    }
}

/// A pair of the leaving's answer that links a tray stone (spec 4.6.13): the tray stone `i`, the
/// later world id `c` with its world stone's place and whether it was detected there (in the
/// leaving point's world, else in the tray's last frame), the score and second of the pair's clean
/// find (`None` without one: a sure stone already on `c`, which needs no fact), and the stone's
/// status (`on`, else lost) and world id before.
#[derive(Debug, Clone, Copy)]
pub(super) struct Target {
    pub(super) i: usize,
    pub(super) c: i64,
    pub(super) place: P,
    pub(super) detected: bool,
    pub(super) score: Option<f64>,
    pub(super) second: Option<f64>,
    pub(super) on: bool,
    pub(super) w0: Option<i64>,
}

impl OpenTray {
    /// The leaving's answer (spec 4.6.13): the newest answer read while the tray was on whose
    /// later moment is within [`SPECKLE_NEAR`] frames of the leaving point `r`, as its index in
    /// `answers`. (A later moment further from the leaving point comes before hand work on the
    /// tray: the leaving point is a calm frame unless hands worked after it.)
    pub(super) fn leaving_answer_at(&self, r: TrayPoint) -> Option<usize> {
        self.answers.iter().rposition(|sp| sp.at.g.abs_diff(r.g) <= SPECKLE_NEAR.unsigned_abs())
    }

    /// The pairs `[c, r]` of the leaving's answer `sp` that link a tray stone (spec 4.6.13), in the
    /// answer's order: `r` is linked at the arrival to a tray stone that was not added and is on
    /// the tray or lost; `c` is a world stone of the leaving point's world (of the tray's last
    /// frame when it is not there), not held; neither `c` nor the stone is in an earlier such pair;
    /// and the pair neither moves a trusted stone to another world stone nor gives a trusted
    /// stone's world stone to another. The pair's clean find is `r`'s in `refs` when clean, else
    /// `c`'s in `stones`; a pair whose clean find has no score (no find of `c`, or one without a
    /// score) links nothing, unless its stone is sure and already on `c` (confirmed, with no fact):
    /// a unit pairs only stones it found, so the answer is malformed, and it is read without failing.
    pub(super) fn speckle_targets(&self, sp: &WorldSpeckle, tw: &TrayWorld) -> Vec<Target> {
        let by_id: BTreeMap<&str, usize> = self.stones.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
        let refs: BTreeMap<i64, &SpeckleFind> = sp.refs.iter().map(|x| (x.id, x)).collect();
        let later: BTreeMap<i64, &SpeckleFind> = sp.stones.iter().map(|x| (x.id, x)).collect();
        let (mut targets, mut claimed, mut used) = (Vec::new(), BTreeSet::new(), BTreeSet::new());
        for &[c, r] in &sp.pairs {
            let Some(&i) = self.arrival_links.get(&r).and_then(|sid| by_id.get(sid.as_str())) else { continue };
            let s = &self.stones[i];
            let Some(ws) = self.point.get(&c).or_else(|| tw.get(c)).filter(|ws| !ws.under_sheet && !ws.held) else {
                continue;
            };
            if s.under_sheet
                || s.added
                || s.status == TrayStoneStatus::Removed
                || claimed.contains(&c)
                || used.contains(&i)
            {
                continue;
            }
            // a stone nothing disturbed (trusted) keeps its world stone, and keeps it from others: on
            // look-alike pavers a pair can join two neighbours, while an undisturbed stone has not moved
            let y = self.link.get(&c).copied();
            if (trusted(s) && s.world != Some(c)) || y.is_some_and(|y| y != i && trusted(&self.stones[y])) {
                continue;
            }
            let x = refs.get(&r).filter(|x| speckle_clean(x)).or_else(|| later.get(&c)).copied();
            // a pair whose finds give no score is skipped where it would need a fact: unless its stone is sure and
            // already on c (confirmed, no fact)
            if x.is_none_or(|x| x.score.is_none()) && !(s.on() && s.world == Some(c) && s.sure) {
                continue;
            }
            targets.push(Target {
                i,
                c,
                place: ws.pos,
                detected: ws.detected,
                score: x.and_then(|x| x.score),
                second: x.and_then(|x| x.second),
                on: s.on(),
                w0: s.world,
            });
            claimed.insert(c);
            used.insert(i);
        }
        targets
    }

    /// The unit's pairs at the leaving (spec 4.6.13), after the leaving's look work and before
    /// its questions end. A stone already on its pair's world stone is confirmed: its question
    /// ends (no fact), and a stone that was not sure is sure again (`identified`, `speckle_same`).
    /// Any other is linked to it, sure (`identified`, `speckle_moved` from its world stone or
    /// `speckle_found` for a lost one): the stone that stood there is lost, and the world stone the
    /// moved stone stood on is unidentified again (freed, as a look's link frees it) when it is on
    /// the board, not held and in no pair. Each fact holds the clean find's score, its margin over
    /// its second and the bar [`SPECKLE_CLEAN`], in permille.
    pub(super) fn speckle_leaving(&mut self, tw: &TrayWorld, r: TrayPoint, sink: &mut Sink<'_>) {
        self.leaving_answer = self.leaving_answer_at(r);
        let Some(a) = self.leaving_answer else { return };
        let targets = self.speckle_targets(&self.answers[a], tw);
        let claimed: BTreeSet<i64> = targets.iter().map(|t| t.c).collect();
        let k = self.k;
        for t in targets.iter().filter(|t| !(t.on && t.w0 == Some(t.c))) {
            let i = t.i;
            let y = self.link.remove(&t.c);
            if let Some(y) = y.filter(|&y| y != i) {
                self.lose(y, r);
            }
            let u = self.unid.remove(&t.c);
            let s = &self.stones[i];
            if let Some(w0) = s.world.filter(|w0| s.on() && self.link.get(w0) == Some(&i)) {
                self.link.remove(&w0);
                let board = self.board.as_ref();
                let free = |ws: &&super::Ws| !claimed.contains(&w0) && !ws.held && inside_board(board, ws.pos);
                if let Some(ws0) = tw.get(w0).filter(free) {
                    self.unid.insert(w0, Unid { freed: true, ..Unid::of(ws0, r) });
                }
            }
            let world_t = match (u, y) {
                (Some(u), _) => u.born.t,
                (None, Some(y)) => self.stones[y].world_t,
                (None, None) => r.t,
            };
            let s = &mut self.stones[i];
            s.world = Some(t.c);
            s.status = TrayStoneStatus::On;
            s.world_t = world_t;
            s.touched = Some(r.g);
            s.carried_away = false;
            s.dropped = false;
            s.emptied = false;
            s.place = t.place;
            if t.detected {
                s.seen_t = r.t;
            }
            (s.at_k, s.at) = (k, s.place); // its place in the last snapshot: where the answer found it
            self.link.insert(t.c, i);
        }
        for t in &targets {
            let s = &mut self.stones[t.i];
            s.question = None;
            s.by_speckle = true;
            let (code, why, from) = if t.on && t.w0 == Some(t.c) {
                if s.sure {
                    continue; // sure, or a question settled: no fact
                }
                ("speckle_same", "confirmed by its speckle", None)
            } else if t.on {
                ("speckle_moved", "recognised by its speckle", t.w0)
            } else {
                ("speckle_found", "found again by its speckle", None)
            };
            s.sure = true;
            let (p, q) = (t.score.map(look_permille), t.second.map(look_permille));
            let fact = TrayFact {
                why: Some(why.to_string()),
                code: Some(code.to_string()),
                from_world: from,
                score: p,
                margin: p.zip(q).map(|(p, q)| p.saturating_sub(q)),
                bar: Some(SPECKLE_CLEAN),
                ..self.fact(TrayFactKind::Identified, Some(t.i), r)
            };
            self.say(fact, sink);
        }
    }

    /// [`TrayStone::evidence`](super::TrayStone::evidence) (spec 4.6.13), as the tray leaves: what
    /// the leaving's answer found of the stone at the other moment, of a stone the tray arrived
    /// with by its arrival world id's find, of one added and on the tray by its world id's; an
    /// added stone removed again is undecided.
    pub(super) fn evidence(&self, s: &St) -> TrayEvidence {
        let sp = self.leaving_answer.map(|a| &self.answers[a]);
        if !s.added {
            find_evidence(sp, SpeckleSide::Arrival, s.first_world)
        } else if s.on() {
            find_evidence(sp, SpeckleSide::Later, s.world)
        } else {
            TrayEvidence::Undecided
        }
    }
}

impl TrayLedger {
    /// Reads one of the unit's speckle answers, from the world of frame `here` (fed already): its
    /// tray and the tray stones in it, or `None` when it is no tray's. `links_before(g)` gives the
    /// tray links before frame `g` (the core keeps them for revisions; empty when it does not).
    /// The open tray keeps it: when it leaves, the tray rules read the leaving's (spec 4.6.13).
    pub(crate) fn speckle(
        &mut self,
        sp: &WorldSpeckle,
        here: TrayPoint,
        links_before: impl Fn(i64) -> BTreeMap<i64, String>,
    ) -> Option<TraySpeckle> {
        let (rg, ag) = (sp.reference.g, sp.at.g);
        let open = self.tray.as_ref().and_then(|tr| tr.arrived).is_some_and(|a| rg >= a.g - SPECKLE_NEAR);
        let tr = if open { self.tray.as_mut()? } else { self.last_closed.as_mut()? };
        let arrived = tr.arrived?;
        // a tray that left: the reference lies from its arrival to its leaving point
        if !open && (rg < arrived.g - SPECKLE_NEAR || rg > tr.leaving?.g) {
            return None;
        }
        let near = (rg - arrived.g).abs() <= SPECKLE_NEAR;
        let ref_links = own(&tr.stones, if near { tr.arrival_links.clone() } else { links_before(rg) });
        let at_links = own(&tr.stones, links_before(ag));
        let (mut by_ref, mut by_at) = (BTreeMap::new(), BTreeMap::new());
        for &[c, r] in &sp.pairs {
            by_at.insert(c, r);
            by_ref.insert(r, c);
        }
        let mut got: BTreeMap<String, StoneSpeckle> = BTreeMap::new();
        for (finds, side, links, other, pairs) in [
            // a stone as it was at the reference, checked at the later moment
            (&sp.refs, SpeckleSide::Arrival, &ref_links, &at_links, &by_ref),
            // a stone at the later moment, checked at the reference
            (&sp.stones, SpeckleSide::Later, &at_links, &ref_links, &by_at),
        ] {
            for x in finds {
                let Some(sid) = links.get(&x.id).filter(|sid| !got.contains_key(*sid)) else { continue };
                got.insert(sid.clone(), entry(sid, side, x, other, pairs.get(&x.id).copied()));
            }
        }
        let mut stones = Vec::new();
        for s in &mut tr.stones {
            if let Some(e) = got.remove(&s.id) {
                if open {
                    s.speckle = Some(e.clone()); // while the tray is on, the newest answer is its stones'
                }
                stones.push(e);
            }
        }
        if open {
            tr.answers.push(sp.clone()); // the leaving reads the leaving's (spec 4.6.13)
        }
        Some(TraySpeckle {
            tray: tr.n,
            session: tr.session,
            video: tr.video.clone(),
            g: here.g,
            t: here.t,
            wall: here.wall,
            unit_tray: sp.tray,
            reference: TrayPoint { g: rg, t: sp.reference.t, wall: None },
            at: TrayPoint { g: ag, t: sp.at.t, wall: None },
            device: sp.device.clone(),
            gpu_ms: sp.gpu_ms,
            wall_ms: sp.wall_ms,
            stones,
        })
    }
}
