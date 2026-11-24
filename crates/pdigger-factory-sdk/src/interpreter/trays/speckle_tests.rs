//! The leaving's speckle answer (spec 4.6.13) on the open tray and the ledger themselves: which
//! answer is the leaving's, which pairs link a tray stone and what linking does, a moved stone's
//! world stone freed, and the evidence of a stone. The rules as a whole are golden-tested over
//! `trays-speckle-synthetic` (its tray 3) and `dry1-0018-0153`; tests/speckle.rs checks tray 3 as
//! values and the self-check on it.

use serde_json::{Value, json};

use super::speckle::find_evidence;
use super::*;
use crate::models::{Solid, SpeckleFind, SpeckleMoment};

fn at(g: i64, t: f64) -> TimeRef {
    TimeRef { g, t, wall: None }
}

/// A find of world stone `id`: its score and second, or `None` when it was not checked.
fn find(id: i64, scores: Option<(f64, f64)>) -> SpeckleFind {
    SpeckleFind { id, score: scores.map(|s| s.0), second: scores.map(|s| s.1), ..SpeckleFind::default() }
}

/// An answer whose later moment is frame `g`.
fn answer_at(g: i64) -> WorldSpeckle {
    WorldSpeckle { at: SpeckleMoment { g, ..SpeckleMoment::default() }, ..WorldSpeckle::default() }
}

/// World stone `id`, a 0.3 x 0.2 brick at (id / 10, 0), held or not.
fn rock(id: i64, held: bool) -> Ws {
    Ws::of(&StoneView {
        id,
        mode: if held { StoneMode::Carry } else { StoneMode::Kept },
        shape: StoneShape::Frozen,
        detected: true,
        solid: Some(Solid {
            pos: vec![id as f64 / 10.0, 0.0, 0.046],
            q: vec![0.0, 0.0, 0.0, 1.0],
            dims: vec![0.3, 0.2, 0.09],
            ..Solid::default()
        }),
        ..StoneView::default()
    })
    .expect("a usable stone")
}

/// Frame `g`'s world of these world stones, those in `held` held.
fn world(g: i64, ids: &[i64], held: &[i64]) -> TrayWorld {
    let stones: Vec<Ws> = ids.iter().map(|&id| rock(id, held.contains(&id))).collect();
    let by_id = stones.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
    TrayWorld {
        r: at(g, g as f64 / 5.0),
        stones,
        by_id,
        events: Vec::new(),
        free: true,
        untrusted: false,
        confirmed: true,
        board: None,
    }
}

/// A tray stone `s-{wid}` the tray arrived with on world stone `wid`, sure (linked when on);
/// `doubted`: not trusted. Its index.
fn stone(tr: &mut OpenTray, wid: i64, status: TrayStoneStatus, doubted: bool) -> usize {
    let (i, on) = (tr.stones.len(), status == TrayStoneStatus::On);
    tr.stones.push(St {
        id: format!("s-{wid}"),
        index: i,
        world: on.then_some(wid),
        first_world: Some(wid),
        status,
        sure: true,
        doubted,
        size: [0.3, 0.2],
        ..St::default()
    });
    tr.arrival_links.insert(wid, format!("s-{wid}"));
    if on {
        tr.link.insert(wid, i);
    }
    i
}

#[test]
fn the_leavings_answer_is_the_newest_near_the_leaving_point() {
    let mut tr = OpenTray::new(1, "v", 1);
    assert_eq!(tr.leaving_answer_at(at(30, 6.0)), None, "no answers");
    tr.answers = [31, 27, 28, 33].map(answer_at).to_vec();
    for (g, want) in [(30, Some(2)), (33, Some(3)), (29, Some(2)), (26, Some(2)), (25, Some(1)), (36, None), (20, None)]
    {
        assert_eq!(tr.leaving_answer_at(at(g, 0.0)), want, "leaving point g {g}");
    }
}

/// A tray at its leaving and an answer with a pair for each rule of spec 4.6.13, in the answer's
/// order: a pair links a stone the tray arrived with (on the tray or lost) to a world stone of the
/// leaving point's world (or of the last frame's) that is not held, never a world stone or a stone
/// twice, never to move a trusted stone or to take a trusted stone's world stone, and only with a
/// scored find (but for a sure stone that stays, which needs no fact); then the links it makes and
/// the facts it gives.
#[test]
fn which_pairs_link_a_tray_stone_and_what_linking_does() {
    let r = at(30, 6.0);
    let mut tr = OpenTray { k: 4, ..OpenTray::new(7, "v", 1) };
    let opened = at(20, 4.0);
    let unseen = Question { cause: "unseen", why: "not seen at two calm moments", since: 22, opened, watch: None };
    let a = stone(&mut tr, 1, TrayStoneStatus::On, false); // trusted
    let b = stone(&mut tr, 2, TrayStoneStatus::On, true); // doubted, in question
    tr.stones[b].question = Some(Question { cause: "pushed", why: "it was pushed", since: 21, opened, watch: None });
    let lost = stone(&mut tr, 3, TrayStoneStatus::Lost, false);
    stone(&mut tr, 4, TrayStoneStatus::Removed, false);
    let e = stone(&mut tr, 5, TrayStoneStatus::On, false); // trusted
    let added = stone(&mut tr, 6, TrayStoneStatus::On, true);
    tr.stones[added].added = true; // never among the arrival links on a real tray
    let seven = stone(&mut tr, 7, TrayStoneStatus::On, true); // no find for its pair
    let eight = stone(&mut tr, 8, TrayStoneStatus::On, true); // its pair's later find not checked
    let asked = stone(&mut tr, 9, TrayStoneStatus::On, true); // in question on its pair's world stone
    tr.stones[asked].question = Some(unseen.clone());
    let unsure = stone(&mut tr, 15, TrayStoneStatus::On, true); // not sure on its pair's world stone
    tr.stones[unsure].sure = false;
    let sixteen = stone(&mut tr, 16, TrayStoneStatus::On, true); // loses its world stone to 18
    tr.stones[sixteen].world_t = 3.0;
    let stays = stone(&mut tr, 17, TrayStoneStatus::On, true); // in question on its pair's world stone, no find
    tr.stones[stays].question = Some(unseen);
    let eighteen = stone(&mut tr, 18, TrayStoneStatus::On, true);
    let nineteen = stone(&mut tr, 19, TrayStoneStatus::On, true); // not sure on its pair's world stone, no find
    tr.stones[nineteen].sure = false;
    tr.unid.insert(10, Unid { born: at(25, 5.0), ..Unid::default() });
    // 11 is held at the leaving point, 12 only in the last frame's world, where 18 is held
    let ids = [1, 2, 5, 6, 7, 8, 9, 10, 13, 14, 15, 16, 17, 18, 19];
    tr.point = world(30, &[&ids[..], &[11]].concat(), &[11]).stones_by_id();
    // 16 is kept from memory at the leaving point (not detected there): the stone linked to it keeps its last sight
    tr.point.get_mut(&16).expect("world stone 16").detected = false;
    let tw = world(31, &[&ids[..], &[12]].concat(), &[18]);
    let sp = WorldSpeckle {
        at: SpeckleMoment { g: 30, t: 6.0, ..SpeckleMoment::default() },
        refs: vec![
            find(1, Some((0.6, 0.1))),
            find(2, Some((0.7, 0.1))),
            find(3, Some((0.4, 0.3))),
            find(7, None),
            find(9, Some((0.65, 0.2))),
            find(15, Some((0.55, 0.05))),
            find(18, Some((0.5, 0.2))),
        ],
        stones: vec![find(10, Some((0.5, 0.1))), find(12, Some((0.6, 0.1))), find(14, None)],
        pairs: vec![
            [10, 99], // no stone linked to 99 at the arrival
            [10, 6],  // an added stone
            [10, 4],  // a removed stone
            [20, 2],  // no world stone 20, in either world
            [11, 2],  // 11 is held
            [10, 1],  // 1 is trusted, on another world stone
            [5, 2],   // 5 is the trusted stone e's
            [12, 3],  // links the lost stone; its find is not clean, so the later find is the pair's
            [10, 2],  // links b
            [10, 3],  // 10 is taken, and so is the lost stone
            [13, 2],  // b is taken
            [13, 7],  // no find scored: 7 not checked, 13 not in the later finds
            [14, 8],  // no find scored: 8 not in the arrival finds, 14 not checked
            [16, 18], // takes 16 from the stone on it, which is not trusted
            [1, 1],   // a stays where it is
            [9, 9],   // in question, stays: its question ends
            [15, 15], // not sure, stays: sure again
            [17, 17], // in question, stays, with no find: its question ends, no fact needs one
            [19, 19], // not sure, stays, with no find: being sure again would need a fact
        ],
        ..WorldSpeckle::default()
    };
    let targets: Vec<_> = tr
        .speckle_targets(&sp, &tw)
        .iter()
        .map(|t| (tr.stones[t.i].id.clone(), t.c, t.score, t.second, t.on, t.w0))
        .collect();
    let want = [
        ("s-3", 12, Some(0.6), Some(0.1), false, None),
        ("s-2", 10, Some(0.7), Some(0.1), true, Some(2)),
        ("s-18", 16, Some(0.5), Some(0.2), true, Some(18)),
        ("s-1", 1, Some(0.6), Some(0.1), true, Some(1)),
        ("s-9", 9, Some(0.65), Some(0.2), true, Some(9)),
        ("s-15", 15, Some(0.55), Some(0.05), true, Some(15)),
        ("s-17", 17, None, None, true, Some(17)),
    ]
    .map(|(id, c, score, second, on, w0)| (id.to_string(), c, score, second, on, w0));
    assert_eq!(targets, want);
    let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
    let mut sink = Sink { counts: &mut counts, facts: &mut facts };
    tr.speckle_leaving(&tw, r, &mut sink);
    assert_eq!(tr.leaving_answer, None, "no answer read: none is the leaving's");
    tr.answers = vec![answer_at(36), sp];
    tr.speckle_leaving(&tw, r, &mut sink);
    assert_eq!(tr.leaving_answer, Some(1));
    let texts: Vec<String> = facts.iter().map(TrayFact::to_string).collect();
    assert_eq!(
        texts,
        [
            "0:06.0 tray 1: stone s-3 found again by its speckle on world stone 12 (0.600)",
            "0:06.0 tray 1: stone s-2 recognised by its speckle on world stone 10 (0.700)",
            "0:06.0 tray 1: stone s-18 recognised by its speckle on world stone 16 (0.500)",
            "0:06.0 tray 1: stone s-15 confirmed by its speckle on world stone 15 (0.550)",
        ]
    );
    let values: Vec<_> = facts.iter().map(|x| (x.code.as_deref(), x.from_world, x.score, x.margin)).collect();
    assert_eq!(
        values,
        [
            (Some("speckle_found"), None, Some(600), Some(500)),
            (Some("speckle_moved"), Some(2), Some(700), Some(600)),
            (Some("speckle_moved"), Some(18), Some(500), Some(300)),
            (Some("speckle_same"), None, Some(550), Some(500)),
        ]
    );
    assert!(facts.iter().all(|x| x.bar == Some(SPECKLE_CLEAN) && x.certain.is_none()));
    assert_eq!(tr.facts, facts);
    // the lost stone is on 12 as the last frame shows it, from the leaving point on; b on 10, from its
    // first sight; 18 on 16 from the time of the stone that stood there, which is lost, and seen when it was last
    // (16 was not detected at the leaving point)
    let place = |s: &St| (s.world, s.status.clone(), s.world_t, s.place, s.seen_t, s.touched, s.at_k, s.at);
    let on = TrayStoneStatus::On;
    assert_eq!(place(&tr.stones[lost]), (Some(12), on.clone(), 6.0, [1.2, 0.0], 6.0, Some(30), 4, [1.2, 0.0]));
    assert_eq!(place(&tr.stones[b]), (Some(10), on.clone(), 5.0, [1.0, 0.0], 6.0, Some(30), 4, [1.0, 0.0]));
    assert_eq!(place(&tr.stones[eighteen]), (Some(16), on, 3.0, [1.6, 0.0], 0.0, Some(30), 4, [1.6, 0.0]));
    let gone = &tr.stones[sixteen];
    assert_eq!(
        (gone.status.clone(), gone.world, gone.lost_t, gone.by_speckle),
        (TrayStoneStatus::Lost, None, 6.0, false)
    );
    let links: Vec<String> = tr.link.iter().map(|(w, &i)| format!("{w} {}", tr.stones[i].id)).collect();
    let want = "1 s-1, 5 s-5, 6 s-6, 7 s-7, 8 s-8, 9 s-9, 10 s-2, 12 s-3, 15 s-15, 16 s-18, 17 s-17, 19 s-19";
    assert_eq!(links.join(", "), want);
    // the moved stones' world stones are unidentified again (freed) when on the board, not held and in no
    // pair: 2; 18 is held in the last frame
    let freed: Vec<(i64, bool, TimeRef, P, P)> =
        tr.unid.iter().map(|(w, u)| (*w, u.freed, u.born, u.place, u.size)).collect();
    assert_eq!(freed, [(2, true, r, [0.2, 0.0], [0.3, 0.2])]);
    // every stone of a pair that links is sure and by speckle, its question ended; the others are as they were:
    // the pairs without a scored find link nothing where a fact would need one
    for (i, s) in tr.stones.iter().enumerate() {
        let linked = [lost, b, eighteen, a, asked, unsure, stays].contains(&i);
        assert_eq!(s.by_speckle, linked, "{}", s.id);
        assert!(!linked || (s.sure && s.question.is_none()), "{}", s.id);
    }
    for (i, w) in [(seven, 7), (eight, 8), (nineteen, 19)] {
        let s = &tr.stones[i];
        assert_eq!((s.world, s.touched, s.sure), (Some(w), None, i != nineteen), "{}", s.id);
    }
    assert!([e, a, asked, stays].iter().all(|&i| tr.stones[i].touched.is_none()));
}

/// A brick of world stone `id` at (x, y), detected, in `mode`.
fn brick(id: i64, x: f64, y: f64, mode: &str) -> Value {
    json!({"id": id, "mode": mode, "family": "rect", "shape": "frozen", "detected": true,
        "solid": {"pos": [x, y, 0.046], "q": [0.0, 0.0, 0.0, 1.0], "dims": [0.3, 0.2, 0.09]}})
}

fn feed(ledger: &mut TrayLedger, world: Value, r: TimeRef) {
    let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
    let w: WorldLayer = serde_json::from_value(world).expect("a world");
    ledger.feed(&w, r, &mut Sink { counts: &mut counts, facts: &mut facts });
}

/// A tray at 5 frames a second: stones 1 (x -0.6), 2 (x 0) and 3 (x 0.6) arrive at g 4; a hand
/// pushes 2 at g 15, so it is not sure from the settle at g 18; world stone 7 comes at (0, -0.4) at
/// g 20, where the hand pushed 2, while 2's box stays (held in the last frame, when `held`); calm to
/// g 30, the leaving point; g 31 moves and carries the answer, read while the tray is on: 1, 2 and 3
/// at the arrival are 1, 7 and 3 at g 30.
fn speckle_scene(held: bool) -> Tray {
    let mut ledger = TrayLedger::new(1, "dry1");
    for g in 0..=30 {
        let two = if held && g == 30 { "carry" } else { "kept" };
        let mut stones = vec![brick(1, -0.6, 0.0, "kept"), brick(2, 0.0, 0.0, two), brick(3, 0.6, 0.0, "kept")];
        let mut events = Vec::new();
        if g == 15 {
            events.push(json!({"type": "push", "id": 2, "side": "left"}));
        }
        if g >= 20 {
            stones.push(brick(7, 0.0, -0.4, "kept"));
        }
        if g == 20 {
            events.push(json!({"type": "new", "id": 7}));
        }
        let n = stones.len();
        let left = if g == 15 { "touch" } else { "free" };
        let t = g as f64 / 5.0;
        let world = json!({"g": g, "t": t, "motion": "static", "stones": stones, "events": events,
            "hands": {"left": {"state": left}, "right": {"state": "free"}}, "stats": {"stones": n, "confirmed": n}});
        feed(&mut ledger, world, at(g, t));
    }
    let here = at(31, 6.2);
    feed(&mut ledger, json!({"g": 31, "t": 6.2, "motion": "moving"}), here);
    let answer = WorldSpeckle {
        v: 1,
        tray: 1,
        reference: SpeckleMoment { g: 4, t: 0.8, ..SpeckleMoment::default() },
        at: SpeckleMoment { g: 30, t: 6.0, ..SpeckleMoment::default() },
        refs: vec![find(1, Some((0.6, 0.1))), find(2, Some((0.62, 0.1))), find(3, Some((0.6, 0.1)))],
        stones: vec![
            find(1, Some((0.6, 0.1))),
            find(7, Some((0.62, 0.1))),
            find(3, Some((0.6, 0.1))),
            find(2, Some((0.1, 0.05))),
        ],
        pairs: vec![[1, 1], [7, 2], [3, 3]],
        ..WorldSpeckle::default()
    };
    let read = ledger.speckle(&answer, here, |_| BTreeMap::new()).expect("the tray's answer");
    assert_eq!((read.tray, read.stones.len()), (1, 3));
    let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
    ledger.close(&mut Sink { counts: &mut counts, facts: &mut facts });
    let mut trays = ledger.take_closed();
    assert_eq!(trays.len(), 1);
    trays.remove(0)
}

/// The leaving's answer links stone 2, not sure, to world stone 7; 2's own world stone, free on the
/// board, is unidentified again, so the leaving counts it as an unclear addition (cause `freed`)
/// that the answer did not find at the arrival; held, it is not counted. Stone 2 is reported moved.
#[test]
fn a_moved_stone_frees_its_world_stone() {
    for held in [false, true] {
        let tray = speckle_scene(held);
        let mut want = vec![
            "0:00.8 tray 1 arrived: 3 stones",
            "0:03.6 tray 1: stone s1t0.8-1-2 is not sure any more (it was pushed)",
            "0:06.0 tray 1: stone s1t0.8-1-2 recognised by its speckle on world stone 7 (0.620)",
            "0:06.0 tray 1: stone s1t6.0-1-2 not recognised (unclear)",
            "0:06.0 tray 1 leaving: 4 stones (3 sure), 0 added, 0 removed, 1 unclear",
        ];
        let mut report = "tray 1, 0:00.8 to 0:06.0: 0 removed, 1 added, 1 moved, not complete: 1 unclear\n  \
                          0:06.0 added s1t6.0-1-2 at (0.00, 0.00) (unclear, 0.96)\n  \
                          moved s1t0.8-1-2 0.40 to (0.00, -0.40) (0.87)";
        if held {
            want.truncate(3);
            want.push("0:06.0 tray 1 leaving: 3 stones (3 sure), 0 added, 0 removed, 0 unclear");
            report = "tray 1, 0:00.8 to 0:06.0: 0 removed, 0 added, 1 moved\n  \
                      moved s1t0.8-1-2 0.40 to (0.00, -0.40) (0.87)";
        }
        let facts: Vec<String> = tray.facts.iter().map(TrayFact::to_string).collect();
        assert_eq!(facts, want, "held {held}");
        assert_eq!(tray.report().expect("a report").describe(), report, "held {held}");
        assert!(tray.check().is_empty(), "held {held}: {:?}", tray.check());
        let moved = &tray.stones[1];
        assert_eq!((moved.sure, moved.by_speckle, moved.world_id), (true, true, Some(7)), "held {held}");
        assert_eq!(moved.evidence, Some(TrayEvidence::Found));
        let others: Vec<_> = tray.stones.iter().map(|s| (s.by_speckle, s.evidence.clone())).collect();
        assert_eq!(others[0], (true, Some(TrayEvidence::Found)), "confirmed, sure before: no fact");
        if !held {
            let open = tray.open.as_ref().expect("its open account");
            assert_eq!((open.len(), open[0].cause.as_str()), (1, "freed"));
            assert_eq!(others[3], (false, Some(TrayEvidence::NotFound)), "the freed world stone");
        }
    }
}

/// What the leaving's answer says of a world id on each side (a pair, then a clean find, then a find
/// of [`SPECKLE_CLEAN`] or more, then none; the first find of an id counts), and which world id a
/// tray stone's evidence reads.
#[test]
fn the_evidence_of_a_stone() {
    use SpeckleSide::{Arrival, Later};
    use TrayEvidence::{Found, NotFound, Undecided};
    let sp = WorldSpeckle {
        refs: vec![
            find(1, Some((0.6, 0.1))),
            find(2, Some((0.45, 0.3))),
            find(3, Some((0.2, 0.1))),
            find(4, None),
            find(5, Some((0.2, 0.15))),
            find(1, Some((0.1, 0.05))),
            find(8, Some((0.3, -1.0))),
        ],
        stones: vec![find(11, Some((0.6, 0.1))), find(12, Some((0.1, 0.05))), find(15, Some((0.25, 0.2)))],
        pairs: vec![[15, 5], [16, 6]],
        ..WorldSpeckle::default()
    };
    for (side, wid, want) in [
        (Arrival, 1, Found),     // clean; its second find does not count
        (Arrival, 2, Undecided), // 0.450 is not twice 0.300: look-alikes
        (Arrival, 3, NotFound),  // checked, no find of SPECKLE_CLEAN
        (Arrival, 4, Undecided), // not checked
        (Arrival, 5, Found),     // paired, though its own find is not clean
        (Arrival, 6, Undecided), // paired, but not in the answer's arrival finds
        (Arrival, 8, Found),     // 0.300 with no second place (-1)
        (Arrival, 11, Undecided),
        (Later, 11, Found),
        (Later, 12, NotFound),
        (Later, 15, Found), // paired by its later id
        (Later, 5, Undecided),
    ] {
        assert_eq!(find_evidence(Some(&sp), side, Some(wid)), want, "{side:?} {wid}");
    }
    assert_eq!(find_evidence(None, Arrival, Some(1)), Undecided, "no answer");
    assert_eq!(find_evidence(Some(&sp), Arrival, None), Undecided, "no world id");
    let mut tr = OpenTray { answers: vec![sp], leaving_answer: Some(0), ..OpenTray::new(1, "v", 1) };
    let st = |first: i64, world: Option<i64>, status: TrayStoneStatus, added: bool| St {
        first_world: Some(first),
        world,
        status,
        added,
        ..St::default()
    };
    for (s, want) in [
        (st(3, Some(11), TrayStoneStatus::On, false), NotFound), // arrived as 3, now on 11
        (st(1, None, TrayStoneStatus::Removed, false), Found),   // removed, found as the tray left
        (st(12, Some(11), TrayStoneStatus::On, true), Found),    // added, on 11 as the tray left
        (st(12, Some(12), TrayStoneStatus::Removed, true), Undecided), // added and removed again
        (st(12, None, TrayStoneStatus::Lost, true), Undecided),
    ] {
        assert_eq!(tr.evidence(&s), want, "{s:?}");
    }
    tr.leaving_answer = None;
    assert_eq!(tr.evidence(&st(1, Some(1), TrayStoneStatus::On, false)), Undecided, "no leaving answer");
    crate::wire::tests::round_trip::<TrayEvidence>();
}
