//! Presence questions (SDK-DESIGN.md 4.6.1) beyond the golden files: every tray of the four
//! presence recordings ends as its generator says; a stone held in question reads as one and is
//! sure again once it is seen; a settle at once holds only if the next calm moment shows the stone;
//! a stone with a look chain is never held in question; held stones, hand deletes, revisions, a
//! crowd of stones of its size and another stone's look keep a question from settling; the leaving
//! counts its own frame again when something could have changed the stone; a hand on a stone since
//! its last sight makes its doubt a plain fact; the frames the tray rules can not read (the core's
//! `gap()`, a replay's `gap` and `reset` lines, a live stream's `gap` and `reset` events) end the
//! questions open then; and a core of your own fed every line of a recording (`feed_line`) ends as
//! its replay does. The rules themselves are golden-tested
//! over `trays-presence-synthetic` (35 trays), `trays-presence-looks-synthetic` (8 trays, with
//! made-up looks), `trays-presence-attacks-synthetic` (28 trays) and
//! `trays-presence-attacks-looks-synthetic` (7 trays, with made-up looks).

mod common;

use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};

use futures::StreamExt;
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::models::ClockEvent;
use pdigger_factory_sdk::replay::{Line, SideLine};
use pdigger_factory_sdk::testing::FakeFactory;
use pdigger_factory_sdk::{
    InterpreterCore, Replay, ReplayStream, Tray, TrayFact, TrayFactKind, TrayQuestion, TrayStone, TrayStoneStatus,
    Update,
};
use serde_json::{Value, json};

use common::{fixtures, recording};

const PRESENCE: &str = "trays-presence-synthetic";
const LOOKS: &str = "trays-presence-looks-synthetic";
const ATTACKS: &str = "trays-presence-attacks-synthetic";
const ATTACK_LOOKS: &str = "trays-presence-attacks-looks-synthetic";

/// A tray as its generator writes it: its number, its name there, the world ids of the stones that
/// end sure (the leaving counts exactly these), of those that end not sure, and of those among
/// them whose `unsure` fact ends a question ("open since", `opened` set); every other stone that
/// ends not sure gets its fact at its doubt.
type End = (i64, &'static str, &'static [i64], &'static [i64], &'static [i64]);

const ENDS: &[(&str, &[End])] = &[
    (
        PRESENCE,
        &[
            (1, "CTRL", &[101, 102, 103], &[], &[]),
            (2, "MOT1", &[202, 203], &[201], &[201]),
            (3, "MOT2", &[302, 303], &[301], &[301]),
            (4, "MOT3", &[402, 403], &[401], &[]),
            (5, "NOW1", &[502, 503], &[501], &[501]),
            (6, "GAP1", &[602, 603], &[601], &[601]),
            (7, "REV1", &[702, 703], &[701], &[701]),
            (8, "DROP1", &[802, 803], &[801, 804], &[]),
            (9, "PSAME", &[902], &[901, 903], &[901]),
            (10, "DROP2", &[1002, 1003], &[1001, 1004], &[]),
            (11, "SIDE1", &[1102, 1103], &[1101], &[1101]),
            (12, "T15", &[1122, 1123], &[1121], &[1121]),
            (13, "T16", &[1132], &[1131], &[1131]),
            (14, "T17", &[1143], &[1141, 1142], &[1141]),
            (15, "Q1", &[1501, 1502, 1503], &[], &[]),
            (16, "Q2", &[1602, 1603], &[1601], &[]),
            (17, "Q3", &[1702, 1703, 1704], &[1701], &[]),
            (18, "QS", &[1802, 1803], &[1801, 1804], &[]),
            (19, "T13", &[1903], &[1901, 1902], &[]),
            (20, "T14", &[2002], &[2001], &[]),
            (21, "QP", &[2102, 2103], &[2101], &[]),
            (22, "PNEIGH", &[2203], &[2201, 2202], &[2201]),
            (23, "LEAVE1", &[2301, 2302, 2303], &[], &[]),
            (24, "NOARR", &[2401, 2402, 2403], &[], &[]),
            (25, "PUSHX", &[2502, 2503], &[2501], &[2501]),
            (26, "ORDER", &[2603], &[2601, 2602], &[2602]),
            (27, "MINT", &[2701, 2702], &[2707], &[]),
            (28, "REVPUSH", &[2802, 2803], &[2801], &[2801]),
            (29, "REVSAME", &[2902, 2903], &[2901], &[2901]),
            (30, "CROWD", &[3002, 3003], &[3001], &[3001]),
            (31, "ONCELOST", &[3102, 3103], &[3101], &[]),
            (32, "ONCETOUCH", &[3202, 3203], &[3201], &[]),
            (33, "CROWD1", &[3302, 3303], &[3301], &[3301]),
            (34, "LVHAND", &[3402, 3403], &[3401], &[]),
            (35, "SETTLE3", &[3501, 3502, 3503], &[], &[]),
        ],
    ),
    (
        LOOKS,
        &[
            (1, "LK1", &[102, 103], &[101], &[101]),
            (2, "LK2", &[202, 203], &[201], &[201]),
            (3, "LK0", &[302, 303], &[301], &[301]),
            (4, "QE", &[402, 403], &[401], &[401]),
            (5, "QL1", &[501, 502, 503], &[], &[]),
            (6, "QL2", &[602, 603], &[601], &[601]),
            (7, "LKNOCH", &[702, 703], &[701, 704], &[701, 704]),
            (8, "LKNOCH0", &[801, 802, 803], &[804], &[804]),
        ],
    ),
    (
        ATTACKS,
        &[
            (1, "CTRL", &[101, 102, 103], &[], &[]),
            (2, "NHELD", &[202, 203, 204], &[201], &[201]),
            (3, "NHELDF", &[302, 303], &[301], &[301]),
            (4, "XHELDD", &[402, 403], &[401], &[401]),
            (5, "DELHF", &[503], &[501], &[501]),
            (6, "DELCA", &[602, 603], &[601], &[601]),
            (7, "DELHN", &[702, 703], &[701], &[701]),
            (8, "REVHAND", &[802, 803], &[801], &[801]),
            (9, "REVUNTR", &[902, 903], &[901], &[901]),
            (10, "REVHELD", &[1002, 1003], &[1001], &[1001]),
            (11, "REVLATE", &[1101, 1102, 1103], &[], &[]),
            (12, "REVMID", &[1202, 1203], &[1201], &[1201]),
            (13, "MOTFS", &[1302, 1303], &[1301], &[1301]),
            (14, "MOTS1", &[1402, 1403], &[1401], &[1401]),
            (15, "ONCEMOT", &[1502, 1503], &[1501], &[]),
            (16, "ONCEBUSY", &[1602, 1603], &[1601], &[]),
            (17, "ONCEGAP", &[1702, 1703], &[1701], &[]),
            (18, "GAPC2", &[1802, 1803], &[1801], &[]),
            (19, "RENSWAP", &[1902, 1903], &[1901, 1904], &[1901, 1904]),
            (20, "SMALLSW", &[2002, 2003], &[2001, 2004], &[2001, 2004]),
            (21, "SMALLSWD", &[2101, 2102, 2103, 2104], &[], &[]),
            (22, "PUSHX1", &[2202, 2203], &[2201], &[2201]),
            (23, "TWOQ", &[2302, 2303], &[2301, 2304], &[2301]),
            (24, "NEWNEXT", &[2401, 2402, 2403], &[], &[]),
            (25, "LVBUSY", &[2502, 2503], &[2501], &[2501]),
            (26, "LVHOLE", &[2602, 2603], &[2601], &[2601]),
            (27, "LVUNTR", &[2702, 2703], &[2701], &[2701]),
            (28, "LVPUSH", &[2802], &[2801, 2803], &[2801]),
        ],
    ),
    (
        ATTACK_LOOKS,
        &[
            (1, "LKSET", &[102, 103], &[101], &[101]),
            (2, "LKONCE", &[202, 203], &[201], &[201]),
            (3, "LKLIFT", &[302, 303], &[301], &[301]),
            (4, "LKSWAPZ", &[402, 403, 404], &[401], &[401]),
            (5, "LKSWAPZ0", &[502, 503, 504], &[501], &[501]),
            (6, "LKGREY", &[602, 603], &[601], &[601]),
            (7, "LKSWAPZG", &[702, 703, 704], &[701], &[701]),
        ],
    ),
];

/// Every tray of a recording, as a replay gives them (read once per recording).
fn left_trays(name: &str) -> Vec<Tray> {
    static TRAYS: OnceLock<Mutex<HashMap<String, Vec<Tray>>>> = OnceLock::new();
    let mut all = TRAYS.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner());
    all.entry(name.to_string()).or_insert_with(|| Replay::open(recording(name)).unwrap().trays().unwrap()).clone()
}

fn tray_of(name: &str, n: i64) -> Tray {
    left_trays(name).into_iter().find(|t| t.n == n).unwrap_or_else(|| panic!("{name}: no tray {n}"))
}

/// The tray stone first seen under that world id (`...-1-101`).
fn stone_of(t: &Tray, wid: i64) -> &TrayStone {
    let suffix = format!("-{}-{wid}", t.n);
    t.stones.iter().find(|s| s.id.ends_with(&suffix)).unwrap_or_else(|| panic!("tray {}: no ...{suffix}", t.n))
}

/// The tray's `unsure` facts in fact order, by the world id their stone was first seen under.
fn unsure_facts(t: &Tray) -> Vec<(i64, TrayFact)> {
    let mut out: Vec<(i64, TrayFact)> = Vec::new();
    for f in t.facts.iter().filter(|f| f.kind == TrayFactKind::Unsure) {
        let Some(stone) = f.stone.as_deref() else { continue };
        let wid: i64 = stone.split('-').nth(2).unwrap().parse().unwrap();
        match out.iter_mut().find(|(w, _)| *w == wid) {
            Some(entry) => entry.1 = f.clone(),
            None => out.push((wid, f.clone())),
        }
    }
    out
}

/// The world ids of the tray's `unsure` facts, in fact order.
fn unsure_ids(t: &Tray) -> Vec<i64> {
    unsure_facts(t).into_iter().map(|(wid, _)| wid).collect()
}

/// The `unsure` fact of the stone first seen under `wid`.
fn unsure_fact(t: &Tray, wid: i64) -> TrayFact {
    let found = unsure_facts(t).into_iter().find(|(w, _)| *w == wid).map(|(_, f)| f);
    found.unwrap_or_else(|| panic!("tray {}: no unsure fact of {wid}", t.n))
}

fn text_of<'a>(facts: impl IntoIterator<Item = &'a TrayFact>) -> String {
    facts.into_iter().map(|f| f.describe() + "\n").collect()
}

fn golden(name: &str, part: &str) -> String {
    std::fs::read_to_string(fixtures().join("golden").join(format!("{name}.{part}"))).unwrap()
}

/// The lines of a recording as they are written, header first.
fn recording_lines(name: &str) -> Vec<String> {
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(recording(name)).unwrap()).read_to_string(&mut text).unwrap();
    text.lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()
}

fn is_frame(x: &Value) -> bool {
    x.get("g").is_some() && x.get("cursor").is_some()
}

/// A core fed the recording as a replay feeds it (its clock lines, its gap lines, its frames),
/// with `gap()` after its `gap_after`-th frame; then its end.
fn fed(name: &str, gap_after: Option<usize>) -> InterpreterCore {
    let mut core = InterpreterCore::new();
    let mut n = 0;
    for line in Replay::open(recording(name)).unwrap().lines() {
        match line.unwrap() {
            Line::Side(side) if side.sse == "clock" => {
                core.set_clock(serde_json::from_value::<ClockEvent>(side.data).unwrap().state);
            }
            Line::Side(side) if side.sse == "gap" => core.gap(),
            Line::Frame(frame) => {
                core.feed(*frame);
                n += 1;
                if Some(n) == gap_after {
                    core.gap();
                }
            }
            other => panic!("{name}: a line fed() does not take: {other:?}"),
        }
    }
    core.end();
    core
}

/// Frame by frame while tray `n` is on: how many frames were fed, its stone first seen under
/// `wid`, the tray.
fn views(name: &str, n: i64, wid: i64) -> Vec<(usize, TrayStone, Tray)> {
    let mut core = InterpreterCore::new();
    let mut out = Vec::new();
    for (k, frame) in Replay::open(recording(name)).unwrap().frames().enumerate() {
        core.feed(frame.unwrap());
        if let Some(live) = core.trays().pop().filter(|t| t.n == n && t.leaving.is_none()) {
            out.push((k + 1, stone_of(&live, wid).clone(), live));
        }
    }
    out
}

/// The stone's (sure, in question) frame by frame while its tray is on, each change once.
fn states(name: &str, n: i64, wid: i64) -> Vec<(bool, bool)> {
    let mut seen: Vec<(bool, bool)> =
        views(name, n, wid).iter().map(|(_, s, _)| (s.sure, s.question.is_some())).collect();
    seen.dedup();
    seen
}

const SURE: (bool, bool) = (true, false);
const ASKED: (bool, bool) = (false, true);
const GIVEN_UP: (bool, bool) = (false, false);

/// How many frames of `trays-presence-synthetic` were fed when tray 1's stone 101 first held its
/// question.
fn in_question() -> usize {
    static K: OnceLock<usize> = OnceLock::new();
    *K.get_or_init(|| views(PRESENCE, 1, 101).into_iter().find(|(_, s, _)| s.question.is_some()).unwrap().0)
}

/// The facts of every tray but the first, as text.
fn after_the_first(trays: &[Tray]) -> String {
    text_of(trays[1..].iter().flat_map(|t| &t.facts))
}

/// The stone's `unsure` fact, which must end its presence question of the doubt at 0:04.4 (tray
/// 1's stone 101).
fn ended_unseen(t: &Tray, wid: i64) -> TrayFact {
    let fact = unsure_fact(t, wid);
    assert_eq!((fact.why.as_deref(), fact.code.as_deref()), (Some("not seen at two calm moments"), Some("unseen")));
    assert_eq!(fact.opened.map(|o| clock(o.t)).as_deref(), Some("0:04.4"));
    assert!(!stone_of(t, wid).sure);
    fact
}

/// A copy of the recording with every line after its header passed through `change` (`None`
/// drops the line); a line `change` leaves as it is stays as it was written.
fn edited(name: &str, change: impl Fn(&Value) -> Option<Value>) -> Replay {
    let lines = recording_lines(name);
    let mut out = format!("{}\n", lines[0]);
    for line in &lines[1..] {
        let x: Value = serde_json::from_str(line).unwrap();
        match change(&x) {
            Some(y) if y == x => out.push_str(line),
            Some(y) => out.push_str(&y.to_string()),
            None => continue,
        }
        out.push('\n');
    }
    Replay::from_bytes(out).unwrap()
}

/// A copy of `trays-presence-synthetic` with `line` after its `after`-th frame.
fn with_line(line: &str, after: usize) -> Replay {
    let mut out = String::new();
    let mut n = 0;
    for x in recording_lines(PRESENCE) {
        out.push_str(&x);
        out.push('\n');
        if is_frame(&serde_json::from_str(&x).unwrap()) {
            n += 1;
            if n == after {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    Replay::from_bytes(out).unwrap()
}

#[test]
fn the_recordings_hold_the_trays_of_their_generator() {
    for (name, trays) in ENDS {
        let left = left_trays(name);
        assert_eq!(left.iter().map(|t| t.n).collect::<Vec<_>>(), trays.iter().map(|e| e.0).collect::<Vec<_>>());
        for t in &left {
            assert_eq!(t.check(), Vec::<String>::new(), "{name}: tray {}", t.n);
        }
    }
}

/// The stones that end sure are on the tray, sure, out of question and without a fact `unsure`,
/// and the leaving counts exactly them; each of the others has one, which ends a question ("open
/// since", `opened` set) exactly where the stone was held in question, and else comes at its doubt.
#[test]
fn every_tray_ends_as_its_generator_says() {
    for (name, trays) in ENDS {
        for &(n, case, sure, not_sure, held) in *trays {
            let id = format!("{name}-{n}-{case}");
            let t = tray_of(name, n);
            let mut ids = unsure_ids(&t);
            ids.sort_unstable();
            let mut want = not_sure.to_vec();
            want.sort_unstable();
            assert_eq!(ids, want, "{id}");
            for &wid in sure {
                let s = stone_of(&t, wid);
                assert!(s.status == TrayStoneStatus::On && s.sure && s.question.is_none(), "{id}: {wid} {s:?}");
            }
            for &wid in not_sure {
                assert!(!stone_of(&t, wid).sure, "{id}: {wid}");
                assert_eq!(unsure_fact(&t, wid).opened.is_some(), held.contains(&wid), "{id}: {wid}");
            }
            let leaving = t.facts.last().unwrap();
            assert_eq!(leaving.kind, TrayFactKind::Leaving, "{id}");
            assert_eq!(leaving.sure, Some(sure.len() as u64), "{id}");
        }
    }
}

/// Tray 1: stone 101 is not seen at two calm moments while nothing could have changed it. It is
/// in question (not sure, `question` set, no fact) until the world sees it again on its place, and
/// then sure again.
#[test]
fn a_stone_held_in_question_is_not_sure_until_it_is_seen_again() {
    let seen = views(PRESENCE, 1, 101);
    assert_eq!(states(PRESENCE, 1, 101), [SURE, ASKED, SURE]);
    let (_, s, live) = seen.iter().find(|(_, s, _)| s.question.is_some()).unwrap();
    let q = s.question.clone().unwrap();
    let want = TrayQuestion {
        cause: "unseen".into(),
        why: "not seen at two calm moments".into(),
        since: q.since,
        opened: q.opened,
    };
    assert_eq!(q, want);
    assert_eq!((q.since, q.opened.g, clock(q.opened.t).as_str()), (22, 22, "0:04.4"));
    assert_eq!(serde_json::to_value(s).unwrap()["question"], serde_json::to_value(&q).unwrap());
    assert!(!live.facts.iter().any(|f| f.kind == TrayFactKind::Unsure));
    let checks = live.check(); // it has not left, and a question is open
    assert!(checks.iter().any(|c| c == "C1") && checks.iter().any(|c| c == "C4"), "{checks:?}");
    let left = left_trays(PRESENCE);
    assert!(stone_of(&left[0], 101).sure);
    assert!(unsure_facts(&left[0]).is_empty());
}

/// Tray 15: stone 1501 is seen again on its place between the two calm moments that miss it, with
/// nothing at work: the doubt settles at once, so the stone is never in question; its next unseen
/// run, which starts at that calm moment, is on probation, and the calm moment that shows it again
/// settles that run too: it stays sure.
#[test]
fn a_question_seen_again_before_its_doubt_is_settled_at_once() {
    assert_eq!(states(PRESENCE, 15, 1501), [SURE]);
    assert!(unsure_facts(&tray_of(PRESENCE, 15)).is_empty());
}

/// Tray 35: stone 3501's question is settled by a sight between two calm moments, and the second
/// does not show it: sure again there, its next unseen run starts there on probation, and the calm
/// moment after, which shows the stone, settles that run too: it stays sure, with no fact.
#[test]
fn a_question_settled_on_a_calm_moment_that_does_not_show_it_is_on_probation() {
    assert_eq!(states(PRESENCE, 35, 3501), [SURE, ASKED, SURE]);
    assert!(unsure_facts(&tray_of(PRESENCE, 35)).is_empty());
}

/// The stone is settled at once (never in question) and is sure until the next calm moment that
/// shows it; when its run on probation can not be settled there (the stone was lost meanwhile, its
/// first sight was busy, the tray moved or hands worked while it was out of sight, frames were
/// dropped), it gets its fact `unsure` there, a plain one, and the tray has not left yet.
#[test]
fn a_settle_at_once_holds_only_if_the_next_calm_moment_settles_its_run() {
    for (case, name, n, wid) in [
        ("lost", PRESENCE, 31, 3101),
        ("busy-first-sight", PRESENCE, 32, 3201),
        ("motion", ATTACKS, 15, 1501),
        ("hands", ATTACKS, 16, 1601),
        ("dropped-frames", ATTACKS, 17, 1701),
    ] {
        assert_eq!(states(name, n, wid), [SURE, GIVEN_UP], "{case}");
        let t = tray_of(name, n);
        let fact = unsure_fact(&t, wid);
        assert_eq!(
            (fact.code.as_deref(), fact.why.as_deref(), fact.opened),
            (Some("unseen"), Some("not seen at two calm moments"), None),
            "{case}"
        );
        assert!(fact.t < t.leaving.unwrap().t, "{case}");
    }
}

/// Tray 12: the stone is seen at a calm moment after its question opened, then a hand works while
/// it is out of sight: the question keeps the run it was opened with, which that hand spoils, so it
/// ends at the next settle and not when the tray leaves. Tray 14: at the leaving's step 1 the
/// question that ends comes first, then the doubt of the stone after it in tray order.
#[test]
fn a_question_keeps_its_run_and_ends_before_the_tray_leaves() {
    let twelve = tray_of(PRESENCE, 12);
    assert!(unsure_fact(&twelve, 1121).t < twelve.leaving.unwrap().t);
    assert_eq!(unsure_ids(&tray_of(PRESENCE, 14)), [1141, 1142]);
}

/// Tray 26: stone 2601, first in tray order, was pushed while stone 2602 was in question and out
/// of sight; one settle gives both facts, 2601's first. Tray 25: stone 2501, in question, was pushed
/// and is still unseen: its own later reason ends the question at that settle, with the question's
/// cause and why, and not when the tray leaves.
#[test]
fn the_settle_goes_stone_by_stone_and_still_tests_a_later_reason() {
    let order = unsure_facts(&tray_of(PRESENCE, 26));
    assert_eq!(order.iter().map(|(wid, _)| *wid).collect::<Vec<_>>(), [2601, 2602]);
    let (first, second) = (&order[0].1, &order[1].1);
    assert_eq!(first.t, second.t);
    assert_eq!((first.code.as_deref(), first.opened), (Some("pushed"), None));
    assert!(second.opened.is_some());
    let t = tray_of(PRESENCE, 25);
    let fact = unsure_fact(&t, 2501);
    assert_eq!((fact.code.as_deref(), fact.why.as_deref()), (Some("unseen"), Some("not seen at two calm moments")));
    assert!(fact.opened.is_some());
    assert!(fact.t < t.leaving.unwrap().t);
}

/// Tray 27: a slab a hand set down is added for certain, then not seen at two calm moments, seen
/// once between them: its last sight is its placing, where the hand worked, so the doubt is not a
/// presence question (a plain fact).
#[test]
fn a_stone_added_for_certain_was_last_seen_where_its_world_stone_was_first_seen() {
    let t = tray_of(PRESENCE, 27);
    let kinds: Vec<(TrayFactKind, Option<bool>)> = t
        .facts
        .iter()
        .filter(|f| f.stone.as_deref().is_some_and(|s| s.ends_with("-27-2707")))
        .map(|f| (f.kind.clone(), f.certain))
        .collect();
    assert_eq!(kinds, [(TrayFactKind::Added, Some(true)), (TrayFactKind::Unsure, None)]);
    assert_eq!(unsure_fact(&t, 2707).opened, None);
}

/// Trays 30 and 33: a small stone in question is seen again on its place, but another stone of its
/// size stands within NEAR of that place at the settle (tray 30) or at its first sight (tray 33): a
/// sight can not tell the two apart, so the question is never settled and ends when the tray
/// leaves.
#[test]
fn a_stone_of_its_size_near_its_place_keeps_the_question_open() {
    for (case, n, wid) in [("at-the-settle", 30, 3001), ("at-its-first-sight", 33, 3301)] {
        assert_eq!(states(PRESENCE, n, wid), [SURE, ASKED], "{case}");
        let t = tray_of(PRESENCE, n);
        let fact = unsure_fact(&t, wid);
        assert!(fact.opened.is_some(), "{case}");
        assert_eq!(fact.t, t.leaving.unwrap().t, "{case}");
    }
}

/// Looks tray 7: stone 701 (no chain) is in question; its world stone is seen again on its place
/// with the face of stone 704, which is in question too: the look reaches the bar against 704's
/// chain, so the sight does not settle 701, and its question ends at that settle. Looks tray 8: the
/// same with a face that is not 804's (its chain scores low on it): 801 is settled, 804 stays in
/// question until the tray leaves.
#[test]
fn a_look_of_another_open_stone_on_its_world_stone_ends_the_question() {
    assert_eq!(states(LOOKS, 7, 701), [SURE, ASKED, GIVEN_UP]);
    let t = tray_of(LOOKS, 7);
    let fact = unsure_fact(&t, 701);
    assert!(fact.opened.is_some());
    assert!(fact.t < t.leaving.unwrap().t);
    assert!(stone_of(&t, 701).looks.is_empty());
    assert_eq!(states(LOOKS, 8, 801), [SURE, ASKED, SURE]);
    let eight = tray_of(LOOKS, 8);
    assert_eq!(unsure_fact(&eight, 804).t, eight.leaving.unwrap().t);
}

/// A stone with a look chain that is not seen at two calm moments gets a question only a look
/// settles: in question from that doubt on, it is not settled by being seen again on its place;
/// its fact ends the question when the tray leaves. Looks tray 4: still unseen at the next calm
/// moment, the question's `since` grows to it.
#[test]
fn a_stone_with_a_chain_is_never_held_in_question() {
    for (name, n, wid) in [(LOOKS, 4, 401), (LOOKS, 3, 301), (ATTACK_LOOKS, 4, 401)] {
        let seen: Vec<TrayQuestion> = views(name, n, wid).into_iter().filter_map(|(_, s, _)| s.question).collect();
        assert!(!seen.is_empty(), "{name} {n}");
        assert!(seen.iter().all(|q| q.cause == "unseen" && q.opened == seen[0].opened), "{name} {n}: {seen:?}");
        assert_eq!(states(name, n, wid), [SURE, ASKED], "{name} {n}");
        let t = tray_of(name, n);
        assert!(!stone_of(&t, wid).looks.is_empty(), "{name} {n}");
        let fact = unsure_fact(&t, wid);
        // the views feed no clock lines: no wall there
        assert_eq!(fact.opened.map(|o| o.g), Some(seen[0].opened.g), "{name} {n}");
        assert_eq!(fact.t, t.leaving.unwrap().t, "{name} {n}");
        if (name, n) == (LOOKS, 4) {
            let since: BTreeSet<i64> = seen.iter().map(|q| q.since).collect();
            assert_eq!(since.len(), 2, "{since:?}");
            assert!(seen.last().unwrap().since > seen[0].since);
        }
    }
}

/// Attack trays 2 and 6: while stone 201 is in question and out of sight, the brick next to it
/// reads `carry` (tray 2), or the one next to 601 is deleted, why `carried-away` (tray 6), with
/// both hands free and no hand event: busy frames, so neither question is settled. The same
/// recording with that brick `kept`, and that delete why `absent`: both settled, every other tray
/// the recording's.
#[test]
fn a_world_stone_held_or_taken_by_a_hand_makes_a_busy_frame() {
    let calm = |x: &Value| {
        let mut x = x.clone();
        if let Some(stones) = x.pointer_mut("/world/stones").and_then(Value::as_array_mut) {
            for s in stones.iter_mut().filter(|s| s["id"] == 204) {
                s["mode"] = json!("kept");
            }
        }
        if let Some(events) = x.pointer_mut("/world/events").and_then(Value::as_array_mut) {
            for e in events.iter_mut().filter(|e| e["id"] == 604 && e["why"] == "carried-away") {
                e["why"] = json!("absent");
            }
        }
        Some(x)
    };
    let trays = edited(ATTACKS, calm).trays().unwrap();
    for (n, wid) in [(2, 201), (6, 601)] {
        assert!(!stone_of(&tray_of(ATTACKS, n), wid).sure, "tray {n}");
        let t = &trays[n as usize - 1];
        assert!(stone_of(t, wid).sure, "tray {n}");
        assert!(unsure_facts(t).is_empty(), "tray {n}");
    }
    let rest = |trays: &[Tray]| text_of(trays.iter().filter(|t| t.n != 2 && t.n != 6).flat_map(|t| &t.facts));
    assert_eq!(rest(&trays), rest(&left_trays(ATTACKS)));
}

/// Tray 29: a frame inside stone 2901's unseen run comes again as a revision that changes
/// nothing: the next frame counts as one after frames the rules could not read, so the question
/// ends ("open since"). Without that revision the stone is seen again and sure, and every other
/// tray is the recording's.
#[test]
fn every_revision_is_a_frame_the_rules_could_not_read() {
    assert!(unsure_fact(&tray_of(PRESENCE, 29), 2901).opened.is_some());
    let revision = recording_lines(PRESENCE)
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|x| {
            x.pointer("/world/rev") == Some(&json!(1))
                && x["world"]["stones"].as_array().unwrap().iter().any(|s| s["id"] == 2901)
        })
        .unwrap()["g"]
        .clone();
    let revised = |x: &Value| x["world"]["rev"].as_i64().is_some_and(|rev| rev != 0);
    let trays = edited(PRESENCE, |x| (x["g"] != revision || !revised(x)).then(|| x.clone())).trays().unwrap();
    assert!(stone_of(&trays[28], 2901).sure);
    assert!(unsure_facts(&trays[28]).is_empty());
    let rest = |trays: &[Tray]| text_of(trays.iter().filter(|t| t.n != 29).flat_map(|t| &t.facts));
    assert_eq!(rest(&trays), rest(&left_trays(PRESENCE)));
}

/// Attack trays 25-28: stone X is not detected at the last calm moment, whose frame is the tray's
/// last, after hands worked, frames were dropped, a hand was flagged untrusted or a stone was
/// pushed while it was out of sight: the leaving counts that frame again, so X's question opens and
/// ends there ("open since" the leaving itself). Tray 23 of `trays-presence-synthetic`, where
/// nothing happened: the frame is not counted twice, the stone stays sure.
#[test]
fn the_leaving_counts_its_own_frame_again_when_something_could_have_changed_the_stone() {
    for (case, n) in [("hands", 25), ("dropped-frames", 26), ("untrusted", 27), ("push", 28)] {
        let t = tray_of(ATTACKS, n);
        let fact = unsure_fact(&t, n * 100 + 1);
        assert!(t.leaving.is_some(), "{case}");
        assert_eq!(fact.opened, t.leaving, "{case}");
        assert_eq!(fact.t, t.leaving.unwrap().t, "{case}");
    }
    assert!(unsure_facts(&tray_of(PRESENCE, 23)).is_empty());
}

/// Tray 34: a hand lifted and set down stone 3401 while it was not detected, and the last calm
/// moment, the tray's last frame, does not show it: the leaving counts that frame again, and the
/// doubt there is no question (a hand handled it since its last sight): a plain fact at the
/// leaving.
#[test]
fn the_leaving_refuses_the_question_of_a_stone_a_hand_handled_since_its_last_sight() {
    let t = tray_of(PRESENCE, 34);
    let fact = unsure_fact(&t, 3401);
    assert_eq!((fact.code.as_deref(), fact.opened, fact.t), (Some("unseen"), None, t.leaving.unwrap().t));
}

/// Tray 1's stone 101 is not detected after its last sight (g 13). A `place` on it at g 14, while
/// its world stone stands 0.5 away: the hand mark lies there, farther than `HAND_R` (0.40) from the
/// stone's last place and from its place now, and no unseen run has begun yet for that busy frame
/// to spoil. Only the handling itself then tells that something could have changed the stone: its
/// doubt at the second calm moment that misses it (0:04.4) is a plain fact, not a question that
/// its next sight would settle. The world stone moved there with no event leaves the tray as the
/// recording has it.
#[test]
fn a_hand_on_a_stone_since_its_last_sight_makes_its_doubt_a_plain_fact() {
    let place = |event: bool| {
        move |x: &Value| {
            let mut x = x.clone();
            if is_frame(&x) && x["g"] == 14 {
                for s in x["world"]["stones"].as_array_mut().unwrap().iter_mut().filter(|s| s["id"] == 101) {
                    assert_eq!(s["detected"], json!(false), "not detected at g 14");
                    s["solid"]["pos"] = json!([-0.3, 0.5, 0.046]);
                }
                if event {
                    let place = json!({"type": "place", "id": 101, "side": "left"});
                    x["world"]["events"].as_array_mut().unwrap().push(place);
                    x["world"]["hands"]["left"]["state"] = json!("grasp");
                }
            }
            Some(x)
        }
    };
    let moved = edited(PRESENCE, place(false)).trays().unwrap();
    assert_eq!(text_of(&moved[0].facts), text_of(&left_trays(PRESENCE)[0].facts));
    assert!(stone_of(&moved[0], 101).sure);
    let trays = edited(PRESENCE, place(true)).trays().unwrap();
    let fact = unsure_fact(&trays[0], 101);
    assert_eq!(
        (fact.code.as_deref(), fact.why.as_deref(), fact.opened, clock(fact.t).as_str()),
        (Some("unseen"), Some("not seen at two calm moments"), None, "0:04.4")
    );
    assert!(!stone_of(&trays[0], 101).sure);
    assert_eq!(unsure_ids(&trays[0]), [101]);
    assert_eq!(after_the_first(&trays), after_the_first(&left_trays(PRESENCE)));
}

/// `InterpreterCore::gap()` while stone 101 of tray 1 is in question: the next frame comes after
/// frames the rules could not read, so the question ends at the next settle with the fact `unsure`
/// ("open since" its doubt). Without it the stone is seen again and sure; every other tray is the
/// recording's.
#[test]
fn the_core_s_gap_ends_the_questions_open_then() {
    let trays = fed(PRESENCE, Some(in_question() + 2)).trays();
    ended_unseen(&trays[0], 101);
    assert_eq!(unsure_ids(&trays[0]), [101]);
    assert_eq!(after_the_first(&trays), after_the_first(&left_trays(PRESENCE)));
    let plain = fed(PRESENCE, None).trays();
    assert_eq!(text_of(plain.iter().flat_map(|t| &t.facts)), golden(PRESENCE, "trays.txt"));
}

#[test]
fn a_replay_s_gap_or_reset_line_ends_the_questions_open_then() {
    for line in [
        r#"{"sse": "reset", "data": {"boot": "0123456789", "cursor": 7}, "recv": 1794960004.8}"#,
        r#"{"sse": "gap", "data": {"gap": 2}}"#,
    ] {
        let trays = with_line(line, in_question() + 2).trays().unwrap();
        ended_unseen(&trays[0], 101);
        assert_eq!(after_the_first(&trays), after_the_first(&left_trays(PRESENCE)), "{line}");
    }
}

/// A side line that does not decode is a line a reader does not understand (section 6): ignored.
/// The last three are Rust's: a whole number written with a fraction is no whole number for Rust's
/// models (spec 2.3 rule 9 leaves that edge to each SDK), and its models hold no `-0` and no whole
/// number beyond `i64` either (those lines pass the shape check, and their data then does not
/// decode).
#[test]
fn a_gap_or_reset_line_that_does_not_decode_changes_no_tray() {
    for line in [
        r#"{"sse": "reset", "data": {"boot": 5, "cursor": 7}}"#,
        r#"{"sse": "gap", "data": {"gap": 1.5}}"#,
        r#"{"sse": "gap", "data": {"gap": 2}, "recv": "late"}"#,
        r#"{"sse": "gap", "data": {"gap": 2.0}}"#,
        r#"{"sse": "gap", "data": {"gap": -0}}"#,
        r#"{"sse": "reset", "data": {"boot": "0123456789", "cursor": 9223372036854775808}}"#,
    ] {
        let facts: Vec<TrayFact> = with_line(line, in_question() + 2).tray_facts().map(Result::unwrap).collect();
        assert_eq!(text_of(&facts), golden(PRESENCE, "trays.txt"), "{line}");
    }
}

/// The Interpreter tells its core about a `gap` or `reset` event of its stream (`gap()`).
#[tokio::test(start_paused = true)]
async fn a_live_stream_s_gap_or_reset_ends_the_questions_open_then() {
    for (event, data) in [("reset", json!({"boot": "other", "cursor": 3})), ("gap", json!({"gap": 2}))] {
        let fake = FakeFactory::from_recording(recording(PRESENCE)).unwrap();
        fake.inject("streamFrames", event, data, in_question() + 2);
        let interpreter = fake.client().interpreter().resume(false);
        let got: Vec<TrayFact> = interpreter.tray_facts().map(Result::unwrap).collect().await;
        let trays = interpreter.trays();
        let fact = ended_unseen(&trays[0], 101);
        assert!(got.contains(&fact), "{event}");
        assert_eq!(after_the_first(&trays), after_the_first(&left_trays(PRESENCE)), "{event}");
    }
}

/// `InterpreterCore::feed_line` feeds a core of your own the lines of a recording as a replay
/// does (the viewer reads a recording URL with it): over `trays-presence-synthetic`, read from the
/// file or from a URL, it gives the replay's updates and trays, its one `gap` line ending tray 11's
/// question at 1:41.4 ("open since 1:39.6"). Without that line stone 1101 is seen again and sure. A
/// duplicate frame, the header and a worker line give no update.
#[tokio::test(start_paused = true)]
async fn a_core_fed_every_line_of_a_recording_ends_as_its_replay_does() {
    let feed = |lines: &[Line]| {
        let mut core = InterpreterCore::new();
        let mut updates: Vec<Update> = lines.iter().filter_map(|line| core.feed_line(line.clone())).collect();
        updates.push(core.end());
        (updates, core.trays())
    };
    let eleven = |trays: &[Tray]| trays.iter().find(|t| t.n == 11).cloned().unwrap();
    let lines: Vec<Line> = Replay::open(recording(PRESENCE)).unwrap().lines().map(Result::unwrap).collect();
    let bytes = std::fs::read(recording(PRESENCE)).unwrap();
    let fake = FakeFactory::new().with_file("/data/presence.ndjson.gz", bytes, 4096);
    let url = format!("{}/data/presence.ndjson.gz", FakeFactory::URL);
    let streamed = ReplayStream::open_url_with(Arc::new(fake), &url).await.unwrap().lines();
    assert_eq!(streamed.map(Result::unwrap).collect::<Vec<Line>>().await, lines);
    let (updates, trays) = feed(&lines);
    let replay: Vec<Update> = Replay::open(recording(PRESENCE)).unwrap().updates().map(Result::unwrap).collect();
    assert_eq!(updates, replay);
    assert_eq!(trays, left_trays(PRESENCE));
    let fact = unsure_fact(&eleven(&trays), 1101);
    assert_eq!((clock(fact.t), fact.opened.map(|o| clock(o.t))), ("1:41.4".to_string(), Some("1:39.6".to_string())));
    let is_gap = |line: &Line| matches!(line, Line::Side(side) if side.sse == "gap");
    assert_eq!(lines.iter().filter(|line| is_gap(line)).count(), 1);
    let without: Vec<Line> = lines.iter().filter(|line| !is_gap(line)).cloned().collect();
    let (_, trays) = feed(&without);
    assert!(stone_of(&eleven(&trays), 1101).sure);
    assert!(unsure_facts(&eleven(&trays)).is_empty());
    let frame = lines.iter().find(|line| matches!(line, Line::Frame(_))).unwrap();
    let mut core = InterpreterCore::new();
    assert!(core.feed_line(frame.clone()).is_some());
    assert!(core.feed_line(frame.clone()).is_none(), "a duplicate");
    assert!(core.feed_line(Line::Header(Box::default())).is_none());
    assert!(core.feed_line(Line::Side(SideLine { sse: "worker".to_string(), ..SideLine::default() })).is_none());
}
