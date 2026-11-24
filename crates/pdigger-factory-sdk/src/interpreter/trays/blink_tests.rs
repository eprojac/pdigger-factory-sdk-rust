//! The Python SDK's tests/test_tray_blinks.py on the ledger itself: a brief silent omission can
//! preserve identity only while no hand could have worked on the stones (spec 4.6.1). Also its
//! version 7 leaving tests from tests/test_tray_leaving.py (positive evidence at departure).

use std::io::BufRead;
use std::sync::OnceLock;

use serde_json::{Value, json};

use super::*;

/// The first world of the synthetic tray recording, the base of every world here.
fn base() -> Value {
    static BASE: OnceLock<Value> = OnceLock::new();
    BASE.get_or_init(|| {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/recordings/trays-synthetic.ndjson.gz");
        let file = std::fs::File::open(path).expect("the synthetic tray recording");
        std::io::BufReader::new(flate2::read::GzDecoder::new(file))
            .lines()
            .map(|line| serde_json::from_str::<Value>(&line.expect("a line")).expect("a JSON row"))
            .find_map(|mut row| row.get_mut("world").map(Value::take))
            .expect("a world row")
    })
    .clone()
}

fn at(g: i64, t: f64) -> TimeRef {
    TimeRef { g, t, wall: None }
}

fn parse(world: Value) -> WorldLayer {
    serde_json::from_value(world).expect("a world")
}

/// Puts these stones in the world, its counts to match.
fn with_stones(world: &mut Value, stones: Vec<Value>) {
    let n = stones.len();
    world["stones"] = Value::Array(stones);
    for key in ["stones", "confirmed", "frozen", "fitted"] {
        world["stats"][key] = json!(n);
    }
}

/// A stone counted as a possible addition: its world id, place, size and place seen at the leaving
/// point.
type Added = (i64, [f64; 2], [f64; 2], Option<[f64; 2]>);

/// The tray's added stones, in tray order, each checked to be an unclear addition with only a
/// `not_recognised` fact.
fn additions(tray: &Tray) -> Vec<Added> {
    let added: Vec<&TrayStone> = tray.stones.iter().filter(|s| s.added).collect();
    for s in &added {
        assert_eq!((s.sure, s.certain), (false, Some(false)), "{:?}", s.world_id);
        let said: Vec<Option<&str>> =
            tray.facts.iter().filter(|f| f.stone.as_ref() == Some(&s.id)).map(|f| f.code.as_deref()).collect();
        assert_eq!(said, [Some("not_recognised")], "{:?}", s.world_id);
    }
    added.iter().map(|s| (s.world_id.expect("a world id"), s.place, s.size, s.left_place)).collect()
}

/// A copy of the synthetic recording's first stone as world stone `id` at (x, y), in `mode`,
/// detected or not.
fn rock(id: i64, x: f64, y: f64, mode: &str, detected: bool) -> Value {
    let mut s = base()["stones"][0].take();
    s["id"] = json!(id);
    (s["solid"]["pos"][0], s["solid"]["pos"][1]) = (json!(x), json!(y));
    (s["mode"], s["detected"]) = (json!(mode), json!(detected));
    s
}

/// Stone 3 of the world, not detected in it.
fn undetect_3(world: &mut WorldLayer) {
    for stone in world.stones.iter_mut().filter(|s| s.id == 3) {
        stone.detected = false;
    }
}

/// Feeds one world: the facts it gave.
fn feed(ledger: &mut TrayLedger, world: &WorldLayer, r: TimeRef) -> Vec<TrayFact> {
    let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
    ledger.feed(world, r, &mut Sink { counts: &mut counts, facts: &mut facts });
    facts
}

/// Ends the session: every tray that left, oldest first.
fn close(ledger: &mut TrayLedger) -> Vec<Tray> {
    let (mut counts, mut facts) = (TrayCounts::default(), Vec::new());
    ledger.close(&mut Sink { counts: &mut counts, facts: &mut facts });
    ledger.take_closed()
}

/// The world ids of the tray's stones that are (or are not) sure, in tray order.
fn world_ids(tray: &Tray, sure: bool) -> Vec<Option<i64>> {
    tray.stones.iter().filter(|s| s.sure == sure).map(|s| s.world_id).collect()
}

/// The codes of the facts of this kind, in order.
fn codes(facts: &[TrayFact], kind: TrayFactKind) -> Vec<&str> {
    facts.iter().filter(|f| f.kind == kind).map(|f| f.code.as_deref().unwrap_or("")).collect()
}

/// ID 1 disappears at g 13 for `missing` frames, next to ID 2; the run ends on its recovery frame
/// with an unrelated new event (or at g 22 with a rival, ID 3, from `rival_from`). The tray, and
/// its links at g 13.
fn two_stones(
    hand_g: Option<i64>,
    missing: i64,
    rival_from: Option<i64>,
    co_blink: bool,
) -> (Tray, BTreeMap<i64, String>) {
    let mut ledger = TrayLedger::new(1, "example");
    let mut links = BTreeMap::new();
    for g in 0..if rival_from.is_none() { 14 + missing } else { 23 } {
        let mut w = base();
        let t = g as f64 * 0.2;
        (w["g"], w["t"], w["motion"]) = (json!(g), json!(t), json!("static"));
        let (mut a, mut b) = (w["stones"][0].take(), w["stones"][1].take());
        a["solid"]["pos"][0] = json!(0.0);
        b["solid"]["pos"][0] = json!(0.35);
        if rival_from.is_some() {
            a["solid"]["dims"] = json!([0.09, 0.05, 0.046]);
        }
        let absent = (13..13 + missing).contains(&g);
        let mut stones = if absent { vec![b] } else { vec![a.clone(), b] };
        if rival_from.is_some_and(|from| g >= from) && !(co_blink && absent) {
            let mut rival = a;
            rival["id"] = json!(3);
            rival["solid"]["pos"][0] = json!(0.12);
            stones.push(rival);
        }
        with_stones(&mut w, stones);
        let mut events = Vec::new();
        if g == 13 + missing {
            events.push(json!({"type": "new", "id": 100}));
        }
        if hand_g == Some(g) {
            events.push(json!({"type": "place", "id": 2, "side": "left"}));
        }
        w["events"] = Value::Array(events);
        feed(&mut ledger, &parse(w), at(g, t));
        if g == 13 {
            links = ledger.links();
        }
    }
    (close(&mut ledger).remove(0), links)
}

#[test]
fn silent_omissions_preserve_links_and_the_last_complete_leaving_frame() {
    for missing in [1, 2] {
        let (tray, links) = two_stones(None, missing, None, false);
        assert_eq!(tray.leaving.map(|r| r.g), Some(12), "{missing} missing");
        assert_eq!(links[&1], tray.stones[0].id);
        assert_eq!(world_ids(&tray, true), [Some(1), Some(2)]);
        assert_eq!(tray.stones[0].touched, None);
        let kinds: Vec<TrayFactKind> = tray.facts.iter().map(|f| f.kind.clone()).collect();
        assert_eq!(kinds, [TrayFactKind::Arrived, TrayFactKind::Leaving]);
    }
}

#[test]
fn hand_work_at_either_end_keeps_the_identity_in_doubt() {
    for hand_g in [12, 14] {
        let (tray, _) = two_stones(Some(hand_g), 1, None, false);
        assert_eq!(world_ids(&tray, false), [Some(1), Some(2)], "hand work at g {hand_g}");
        assert_eq!(codes(&tray.facts, TrayFactKind::Unsure), ["found_again", "appeared"]);
    }
}

#[test]
fn new_nearby_stone_competes_with_a_returning_id() {
    let (tray, _) = two_stones(None, 1, Some(14), false);
    let stone = &tray.stones[0];
    assert!(!stone.sure);
    assert_eq!(stone.touched, Some(22));
    assert!(
        tray.facts
            .iter()
            .any(|f| f.code.as_deref() == Some("matched_at_leaving") && f.stone.as_ref() == Some(&stone.id))
    );
}

#[test]
fn new_nearby_stone_on_the_first_missing_frame_gets_ordinary_rename_handling() {
    let (tray, links) = two_stones(None, 1, Some(13), false);
    assert!(!links.contains_key(&1));
    assert_eq!(links[&3], tray.stones[0].id);
}

#[test]
fn neighbour_continuously_seen_through_the_blink_is_not_a_rival() {
    let (tray, _) = two_stones(None, 2, Some(0), false);
    assert!(tray.stones[0].sure);
    assert_eq!(tray.stones[0].touched, None);
    assert!(!tray.facts.iter().any(|f| matches!(f.kind, TrayFactKind::Removed | TrayFactKind::Added)));
}

#[test]
fn two_adjacent_ids_that_disappear_together_can_compete_on_return() {
    let (tray, _) = two_stones(None, 2, Some(0), true);
    let unsure: BTreeSet<Option<i64>> = world_ids(&tray, false).into_iter().collect();
    assert_eq!(unsure, BTreeSet::from([Some(1), Some(3)]));
    assert_eq!(codes(&tray.facts, TrayFactKind::Unsure), ["matched_at_leaving"; 2]);
}

/// A world of small stones at these x places, in this order, with at most one event.
fn ledger_world(positions: &[(i64, f64)], g: i64, event: Option<(&str, i64)>) -> WorldLayer {
    let mut raw = base();
    let seed = raw["stones"][0].take();
    let stones = positions
        .iter()
        .map(|&(id, x)| {
            let mut stone = seed.clone();
            stone["id"] = json!(id);
            stone["solid"]["pos"][0] = json!(x);
            stone["solid"]["dims"] = json!([0.05, 0.04, 0.03]);
            stone
        })
        .collect();
    with_stones(&mut raw, stones);
    raw["events"] = event.into_iter().map(|(kind, id)| json!({"type": kind, "id": id})).collect();
    (raw["g"], raw["t"]) = (json!(g), json!(g as f64 / 5.0));
    parse(raw)
}

/// Session 42 with the tray of stones 1 and 2 (at 0 and `second_x`) arrived, g 0 to 11.
fn arrived_ledger(second_x: f64) -> TrayLedger {
    let mut ledger = TrayLedger::new(42, "example");
    for g in 0..12 {
        feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, second_x)], g, None), at(g, g as f64 / 5.0));
    }
    ledger
}

#[test]
fn departure_position_requires_a_detection_on_the_selected_frame() {
    for hand_work in [false, true] {
        let mut ledger = arrived_ledger(0.1);
        let r = at(12, 11.0 / 5.0); // the same media time as g 11
        let mut world = ledger_world(&[(1, 0.0), (2, 0.1)], r.g, hand_work.then_some(("place", 2)));
        world.t = r.t;
        world.stones[0].detected = false;
        feed(&mut ledger, &world, r);
        let tray = close(&mut ledger).remove(0);
        assert_eq!(tray.leaving, Some(r), "hand work {hand_work}");
        assert_eq!(tray.stones[0].place, [0.0, 0.0]);
        assert_eq!(tray.stones[0].left_place, None);
        assert_eq!(tray.stones[1].left_place, Some([0.1, 0.0]));
    }
}

#[test]
fn arrival_frame_can_supply_the_departure_positions() {
    let mut ledger = TrayLedger::new(42, "example");
    for g in 0..13 {
        let mut world = ledger_world(&[(1, 0.0), (2, 0.1)], g, None);
        world.stats.confirmed = if g < 12 { 1 } else { 2 };
        world.stones[0].detected = false;
        feed(&mut ledger, &world, at(g, g as f64 / 5.0));
    }
    let tray = close(&mut ledger).remove(0);
    assert_eq!((tray.arrived, tray.leaving), (Some(at(12, 2.4)), Some(at(12, 2.4))));
    let left: Vec<Option<[f64; 2]>> = tray.stones.iter().map(|s| s.left_place).collect();
    assert_eq!(left, [None, Some([0.1, 0.0])]);
}

#[test]
fn separated_stones_can_disappear_together_and_keep_their_original_links() {
    let mut ledger = arrived_ledger(0.35);
    let initial = ledger.links();
    for g in [12, 13] {
        feed(&mut ledger, &ledger_world(&[], g, None), at(g, g as f64 / 5.0));
        assert_eq!(ledger.links(), initial, "g {g}");
    }
    feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, 0.35)], 14, Some(("new", 100))), at(14, 2.8));
    let tray = close(&mut ledger).remove(0);
    assert_eq!(tray.leaving, Some(at(11, 2.2)));
    let ids: BTreeSet<Option<i64>> = tray.stones.iter().map(|s| s.world_id).collect();
    assert_eq!(ids, BTreeSet::from([Some(1), Some(2)]));
    assert!(tray.stones.iter().all(|s| s.sure && s.touched.is_none()));
}

#[test]
fn return_must_be_uniquely_nearer_its_original_place() {
    for return_x in [0.05, 0.1] {
        // a tie, then stone 2's own place
        let mut ledger = arrived_ledger(0.1);
        feed(&mut ledger, &ledger_world(&[(2, 0.1)], 12, None), at(12, 2.4));
        feed(&mut ledger, &ledger_world(&[(1, return_x), (2, 0.0)], 13, None), at(13, 2.6));
        let tray = ledger.current().expect("an open tray");
        // the ordinary rename rules link it anew at g 13; silent continuity would leave `touched` unset
        assert_eq!(tray.stones[0].touched, Some(13), "return at {return_x}");
    }
}

#[test]
fn unrecovered_unidentified_and_linked_stones_at_close() {
    let mut ledger = arrived_ledger(0.1);
    feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, 0.1), (3, 0.8)], 12, None), at(12, 2.4));
    feed(&mut ledger, &ledger_world(&[(2, 0.1)], 13, None), at(13, 2.6));
    assert!(ledger.links().contains_key(&1));
    let tray = close(&mut ledger).remove(0);
    assert_eq!(tray.leaving, Some(at(12, 2.4)));
    let on: Vec<Option<i64>> =
        tray.stones.iter().filter(|s| s.status == TrayStoneStatus::On).map(|s| s.world_id).collect();
    assert_eq!(on, [Some(2)]);
    let changes: Vec<(TrayFactKind, Option<bool>)> = tray
        .facts
        .iter()
        .filter(|f| matches!(f.kind, TrayFactKind::Added | TrayFactKind::Removed))
        .map(|f| (f.kind.clone(), f.certain))
        .collect();
    assert_eq!(changes, [(TrayFactKind::Removed, Some(false))]);
}

#[test]
fn grace_expires_or_gives_way_to_contrary_evidence() {
    for ending in ["expiry", "gap", "delete", "far"] {
        let mut ledger = arrived_ledger(0.1);
        feed(&mut ledger, &ledger_world(&[(2, 0.1)], 12, None), at(12, 2.4));
        assert!(ledger.links().contains_key(&1), "{ending}");
        if ending == "gap" {
            ledger.skipped();
        }
        if ending == "expiry" {
            feed(&mut ledger, &ledger_world(&[(2, 0.1)], 13, None), at(13, 2.6));
            assert!(ledger.links().contains_key(&1));
        }
        let g = if ending == "expiry" { 14 } else { 13 };
        let rows: &[(i64, f64)] = if ending == "far" { &[(2, 0.1), (1, 0.7)] } else { &[(2, 0.1)] };
        let event = (ending == "delete").then_some(("delete", 1));
        feed(&mut ledger, &ledger_world(rows, g, event), at(g, g as f64 / 5.0));
        assert!(!ledger.links().contains_key(&1), "{ending}");
    }
}

#[test]
fn unidentified_return_keeps_birth_and_geometry_at_selected_calm_frame() {
    let mut ledger = arrived_ledger(0.1);
    feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, 0.1), (3, 0.6)], 12, None), at(12, 2.4));
    feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, 0.1)], 13, None), at(13, 2.6));
    feed(&mut ledger, &ledger_world(&[(1, 0.0), (2, 0.1), (3, 0.65)], 14, Some(("new", 4))), at(14, 2.8));
    let tray = close(&mut ledger).remove(0);
    let stone = tray.stones.iter().find(|s| s.world_id == Some(3)).expect("stone 3 on the tray");
    assert_eq!(stone.born, at(12, 2.4));
    assert_eq!(stone.left_place.map(|p| p[0]), Some(0.6));
    assert_eq!(stone.place[0], 0.65);
}

/// The extra observation never proves where the stone came from, or makes up a place in the leaving
/// frame. Where the sight cannot be used, the stone is counted at the leaving point's own place for
/// it, where the world kept it (not held, less than 0.30 of its box under another unheld stone's:
/// version 7).
#[test]
fn a_real_sight_beside_the_leaving_point_only_supports_uncertain_presence() {
    for case in [
        "positive",
        "ghost",
        "hand",
        "gap",
        "replacement",
        "move",
        "resize",
        "rival",
        "source_rival",
        "expiry",
        "rewind",
        "same_time",
        "latest",
        "drift",
    ] {
        let mut ledger = TrayLedger::new(1, "sample");
        for g in 0..18 {
            let mut w = base();
            let mut t = g as f64 * 0.2 + if case == "expiry" && g > 15 { 4.0 } else { 0.0 };
            if case == "rewind" && g == 16 {
                t = 2.8; // frames advance, but this one predates the positive sight at 3.0 s
            }
            if case == "same_time" && g == 16 {
                t = 3.0;
            }
            (w["g"], w["t"], w["motion"]) = (json!(g), json!(t), json!("static"));
            let (mut a, mut b) = (w["stones"][0].take(), w["stones"][1].take());
            a["solid"]["pos"][0] = json!(-0.6);
            b["solid"]["pos"][0] = json!(0.6);
            let mut stones = if g == 15 { vec![b] } else { vec![a.clone(), b] };
            if g >= 10 {
                let mut stone = a.clone();
                stone["id"] = json!(if case == "replacement" && g >= 16 { 4 } else { 3 });
                let mut x = if case == "move" && g >= 16 { 0.3 } else { 0.0 };
                let mut detected = ((g == 10 || g == 11) && case != "ghost") || g == 15;
                if g == 15 {
                    x = 0.02;
                }
                if g == 16 && (case == "latest" || case == "drift") {
                    detected = true;
                    x = if case == "latest" { 0.06 } else { 0.13 };
                }
                if g == 17 && case == "drift" {
                    x = 0.21;
                }
                stone["solid"]["pos"][0] = json!(x);
                stone["detected"] = json!(detected);
                stone["mode"] = json!(if detected { "seen" } else { "kept" });
                if case == "resize" && g >= 16 {
                    let long = stone["solid"]["dims"][0].as_f64().expect("a size");
                    stone["solid"]["dims"][0] = json!(long * 2.0);
                }
                stones.push(stone);
            }
            let mut events = Vec::new();
            if g == 16 {
                events.push(json!({"type": "new", "id": 100}));
                if case == "hand" {
                    events.push(json!({"type": "place", "side": "left", "id": 2}));
                }
                if case == "gap" {
                    ledger.skipped();
                }
            }
            w["events"] = Value::Array(events);
            if case == "rival" && g >= 16 {
                let mut rival = a.clone();
                rival["id"] = json!(4);
                rival["solid"]["pos"][0] = json!(0.10);
                stones.push(rival);
            }
            if case == "source_rival" && g == 15 {
                let mut rival = a;
                rival["id"] = json!(4);
                (rival["solid"]["pos"][0], rival["solid"]["pos"][1]) = (json!(0.02), json!(0.145));
                stones.push(rival);
            }
            with_stones(&mut w, stones);
            feed(&mut ledger, &parse(w), at(g, t));
        }
        let tray = close(&mut ledger).remove(0);
        let sightings: Vec<&TrayStone> = tray.stones.iter().filter(|s| s.world_id == Some(3)).collect();
        if case == "replacement" {
            // world stone 3 is gone by the end: nothing to count
            assert!(sightings.is_empty(), "{case}: {sightings:?}");
            continue;
        }
        let x = match case {
            "positive" | "same_time" => 0.02,
            "latest" => 0.06,
            _ => 0.0,
        };
        assert_eq!(sightings.len(), 1, "{case}");
        let seen = sightings[0];
        assert_eq!((seen.sure, seen.added, seen.certain), (false, true, Some(false)), "{case}");
        assert_eq!(seen.left_place, None, "{case}");
        assert_eq!(seen.place, [x, 0.0], "{case}");
        let last = tray.facts.last().expect("the leaving fact");
        assert_eq!((last.stones, last.added), (Some(3), Some(0)), "{case}");
        let said: Vec<Option<&str>> =
            tray.facts.iter().filter(|f| f.stone.as_ref() == Some(&seen.id)).map(|f| f.code.as_deref()).collect();
        assert_eq!(said, [Some("not_recognised")], "{case}");
    }
}

/// Motion separates two visits. At the second stop a silent absence interrupts its one
/// three-frame quiet run.
#[test]
fn second_stop_uses_its_own_clean_frame_and_its_actual_unseen_evidence() {
    for sight_g in [None, Some(24), Some(25)] {
        // still unseen, a sight in between, seen at leaving
        let mut ledger = TrayLedger::new(42, "sample");
        for g in 0..26 {
            let mut positions = vec![(1, if g <= 22 { -0.5 } else { -0.48 }), (2, 0.0), (3, 0.6)];
            if g == 23 {
                positions.remove(1);
            }
            let event = ((14..=20).contains(&g) || g >= 24).then_some(("new", 100));
            let mut world = ledger_world(&positions, g, event);
            if g == 13 {
                world.motion = Motion::Moving;
            }
            if g >= 21 && Some(g) != sight_g {
                undetect_3(&mut world);
            }
            feed(&mut ledger, &world, at(g, g as f64 / 5.0));
        }
        let closed = close(&mut ledger);
        assert_eq!(closed.len(), 1, "{sight_g:?}");
        let tray = &closed[0];
        assert_eq!(tray.leaving, Some(at(22, 4.4)), "{sight_g:?}");
        assert_eq!(tray.stones[0].left_place, Some([-0.5, 0.0]));
        assert_eq!(tray.stones[2].left_place, None);
        assert_eq!(tray.stones[2].sure, sight_g == Some(25), "{sight_g:?}");
        let want: &[&str] = if sight_g == Some(25) { &[] } else { &["unseen"] };
        assert_eq!(codes(&tray.facts, TrayFactKind::Unsure), want, "{sight_g:?}");
    }
}

/// The second stop of second_stop_uses_its_own_clean_frame_and_its_actual_unseen_evidence leaves
/// at its nominated clean frame, g 22. An unidentified stone 5, boxed at (0.30, 0.30) in the first
/// visit, kept at (0.32, 0.30) through g 22 and at (0.36, 0.30) after, is counted at its place
/// there (version 7); boxed there, with that place as its left place too (version 6); under a kept
/// stone 6 that comes at g 21, it is not, and nor is 6.
#[test]
fn a_second_stop_counts_what_its_clean_frame_kept() {
    let small = |id: i64, x: f64, y: f64, seen: bool| {
        let mut s = rock(id, x, y, if seen { "seen" } else { "kept" }, seen);
        s["solid"]["dims"] = json!([0.05, 0.04, 0.03]);
        s
    };
    let cases: [(&str, &[Added]); 3] = [
        ("kept", &[(5, [0.32, 0.3], [0.05, 0.04], None)]),
        ("seen", &[(5, [0.32, 0.3], [0.05, 0.04], Some([0.32, 0.3]))]),
        ("hidden", &[]),
    ];
    for (case, counted) in cases {
        let mut ledger = TrayLedger::new(1, "sample");
        for g in 0..26 {
            let mut positions = vec![(1, if g <= 22 { -0.5 } else { -0.48 }), (2, 0.0), (3, 0.6)];
            if g == 23 {
                positions.remove(1);
            }
            let mut stones: Vec<Value> =
                positions.iter().map(|&(id, x)| small(id, x, 0.0, id != 3 || g < 21)).collect();
            if g >= 8 {
                let x = if g <= 12 {
                    0.30
                } else if g <= 22 {
                    0.32
                } else {
                    0.36
                };
                stones.push(small(5, x, 0.30, g <= 12 || (case == "seen" && g == 22)));
                if case == "hidden" && g >= 21 {
                    stones.push(small(6, 0.33, 0.30, false));
                }
            }
            let mut w = base();
            (w["g"], w["t"]) = (json!(g), json!(g as f64 / 5.0));
            w["motion"] = json!(if g == 13 { "moving" } else { "static" });
            let new = (14..=20).contains(&g) || g >= 24;
            w["events"] = if new { json!([{"type": "new", "id": 100}]) } else { json!([]) };
            with_stones(&mut w, stones);
            feed(&mut ledger, &parse(w), at(g, g as f64 / 5.0));
        }
        let closed = close(&mut ledger);
        assert_eq!(closed.len(), 1, "{case}");
        let leaving = closed[0].facts.last().expect("the leaving fact");
        let n = counted.len() as u64;
        assert_eq!(
            (leaving.g, leaving.stones, leaving.sure, leaving.unclear),
            (22, Some(3 + n), Some(2), Some(n)),
            "{case}"
        );
        assert_eq!(additions(&closed[0]), counted, "{case}");
    }
}

/// Tray stone 1 arrives at t 9.96; world 1 is missing at t 10.00 while a hand places stone 2, and
/// is back at t 10.04 at x 0.3: nothing pairs it with stone 1, so that is removed and world 1
/// added, both unclear and both first seen in the tenth 10.0. The added one takes the id with -2
/// (4.6), and the report reads each change from its own stone.
#[test]
fn two_stones_first_seen_in_one_tenth_under_one_world_id_get_two_ids() {
    let mut times = vec![9.0, 9.2, 9.4, 9.6, 9.96, 10.0, 10.04];
    times.extend((0..12).map(|k| 10.2 + 0.2 * f64::from(k)));
    let mut ledger = TrayLedger::new(1, "sample");
    for (g, &t) in (0_i64..).zip(&times) {
        let mut w = base();
        (w["g"], w["t"], w["motion"]) = (json!(g), json!(t), json!("static"));
        let a = rock(1, if g >= 6 { 0.3 } else { -0.6 }, 0.0, "kept", true);
        let b = rock(2, 0.6, 0.0, "kept", true);
        w["events"] = if g == 5 { json!([{"type": "place", "id": 2, "side": "left"}]) } else { json!([]) };
        with_stones(&mut w, if g == 5 { vec![b] } else { vec![a, b] });
        feed(&mut ledger, &parse(w), at(g, t));
    }
    let closed = close(&mut ledger);
    assert_eq!(closed.len(), 1);
    let tray = &closed[0];
    let stones: Vec<(&str, Option<i64>, bool)> =
        tray.stones.iter().map(|s| (s.id.as_str(), s.world_id, s.added)).collect();
    assert_eq!(
        stones,
        [("s1t10.0-1-1", None, false), ("s1t10.0-1-2", Some(2), false), ("s1t10.0-1-1-2", Some(1), true)]
    );
    let said: Vec<(&TrayFactKind, Option<&str>, Option<&str>)> = tray
        .facts
        .iter()
        .filter(|f| f.stone.is_some() && f.kind != TrayFactKind::Unsure)
        .map(|f| (&f.kind, f.code.as_deref(), f.stone.as_deref()))
        .collect();
    assert_eq!(
        said,
        [
            (&TrayFactKind::Removed, Some("not_found"), Some("s1t10.0-1-1")),
            (&TrayFactKind::Added, Some("not_recognised"), Some("s1t10.0-1-1-2"))
        ]
    );
    let report = tray.report().expect("a report");
    let places: Vec<(&str, [f64; 2])> =
        report.removed.iter().chain(&report.added).map(|c| (c.stone.as_str(), c.place)).collect();
    assert_eq!(places, [("s1t10.0-1-1", [-0.6, 0.0]), ("s1t10.0-1-1-2", [0.3, 0.0])]);
    // one id on two stones (a tray put together by hand): the report reads the last of them, as the reference does
    let mut twin = tray.clone();
    twin.stones[2].id = "s1t10.0-1-1".into();
    for f in twin.facts.iter_mut().filter(|f| f.stone.as_deref() == Some("s1t10.0-1-1-2")) {
        f.stone = Some("s1t10.0-1-1".into());
    }
    let report = twin.report().expect("a report");
    let places: Vec<[f64; 2]> = report.removed.iter().chain(&report.added).map(|c| c.place).collect();
    assert_eq!(places, [[0.3, 0.0], [0.3, 0.0]]);
}

#[test]
fn independent_partial_calm_readings_can_raise_a_doubt_before_leaving() {
    let mut ledger = TrayLedger::new(42, "sample");
    let mut facts = Vec::new();
    for g in 0..20 {
        let partial = g == 15 || g == 19;
        let mut positions = vec![(1, -0.6), (2, 0.0), (3, 0.6)];
        if partial {
            positions.remove(1);
        }
        let mut world = ledger_world(&positions, g, (g == 12 || g == 16).then_some(("new", 100)));
        if partial {
            undetect_3(&mut world);
        }
        facts.extend(feed(&mut ledger, &world, at(g, g as f64 / 5.0)));
    }
    let unsure: Vec<(i64, Option<&str>)> =
        facts.iter().filter(|f| f.kind == TrayFactKind::Unsure).map(|f| (f.g, f.code.as_deref())).collect();
    assert_eq!(unsure, [(19, Some("unseen"))]);
    assert!(!close(&mut ledger)[0].stones[2].sure);
}

/// Version 7 (spec 4.6.1, Positive evidence at departure; the Python SDK's
/// tests/test_tray_leaving.py): an unidentified world stone that the leaving point's world keeps
/// (lists, not detected), not held, with less than OVERLAP of its box under one other unheld
/// stone's box there, is counted as an unclear addition at its place there.
///
/// Stones 1 and 2 arrive with the tray; world stone 3 comes at g 10, boxed at g 10 and 11, then
/// kept (held, in case `held`), and in some cases a neighbour 5 comes with it, at (0.20, y): its
/// box (0.3 by 0.2, as 3's) then covers 6 (y 0.09: 0.24) or 8 (y 0.05: 0.32) of the 25 points of
/// 3's box. The tray leaves at its last frame, g 17, a calm one; in the `_later` cases a new event
/// at g 15 makes g 14 the leaving point, after which 3 moves 0.05 or 5 first comes.
#[test]
fn a_kept_stone_at_the_leaving_point_is_a_possible_addition() {
    /// A case: its name, the neighbour 5 (its mode from g 12 on, and its y), and the world stones counted as
    /// possible additions with their places.
    type Case = (&'static str, Option<(&'static str, f64)>, &'static [(i64, [f64; 2])]);
    let cases: [Case; 8] = [
        ("alone", None, &[(3, [0.0, 0.0])]),
        // 0.24 of each box under the other
        ("beside_kept", Some(("kept", 0.09)), &[(3, [0.0, 0.0]), (5, [0.2, 0.09])]),
        // 0.32: a kept stone hides one
        ("under_kept", Some(("kept", 0.05)), &[]),
        // so does a detected one, itself counted by its box on the last frame
        ("under_seen", Some(("seen", 0.05)), &[(5, [0.2, 0.05])]),
        // a held stone hides none, and is not counted itself
        ("under_held", Some(("carry", 0.05)), &[(3, [0.0, 0.0])]),
        ("held", None, &[]),
        // its place at the leaving point, not at the last frame
        ("moved_later", None, &[(3, [0.0, 0.0])]),
        // the stones of the leaving point hide it, not the last frame's
        ("rival_later", Some(("seen", 0.05)), &[(3, [0.0, 0.0])]),
    ];
    for (case, neighbour, counted) in cases {
        let mut ledger = TrayLedger::new(1, "sample");
        for g in 0..18 {
            let mut w = base();
            let later = case.ends_with("_later") && g >= 15;
            (w["g"], w["t"], w["motion"]) = (json!(g), json!(g as f64 * 0.2), json!("static"));
            w["events"] = if later && g == 15 { json!([{"type": "new", "id": 100}]) } else { json!([]) };
            let (mut a, mut b) = (w["stones"][0].take(), w["stones"][1].take());
            a["solid"]["pos"][0] = json!(-0.6);
            b["solid"]["pos"][0] = json!(0.6);
            let mut stones = vec![a.clone(), b];
            if g >= 10 {
                let mut stone = a.clone();
                stone["id"] = json!(3);
                stone["solid"]["pos"][0] = json!(if later && case == "moved_later" { 0.05 } else { 0.0 });
                stone["detected"] = json!(g < 12);
                stone["mode"] = json!(if g < 12 {
                    "seen"
                } else if case == "held" {
                    "carry"
                } else {
                    "kept"
                });
                stones.push(stone);
                if let Some((mode, y)) = neighbour.filter(|_| case != "rival_later" || later) {
                    let mut other = a.clone();
                    other["id"] = json!(5);
                    (other["solid"]["pos"][0], other["solid"]["pos"][1]) = (json!(0.20), json!(y));
                    other["mode"] = json!(if g < 12 { "seen" } else { mode });
                    other["detected"] = json!(g < 12 || mode == "seen");
                    stones.push(other);
                }
            }
            with_stones(&mut w, stones);
            feed(&mut ledger, &parse(w), at(g, g as f64 * 0.2));
        }
        let tray = close(&mut ledger).remove(0);
        let leaving = tray.facts.last().expect("the leaving fact");
        assert_eq!(leaving.g, if case.ends_with("_later") { 14 } else { 17 }, "{case}");
        let added: Vec<&TrayStone> = tray.stones.iter().filter(|s| s.added).collect();
        let places: Vec<(i64, [f64; 2])> = added.iter().map(|s| (s.world_id.expect("a world id"), s.place)).collect();
        assert_eq!(places, counted, "{case}");
        for s in &added {
            assert_eq!((s.sure, s.certain), (false, Some(false)), "{case}");
            let said: Vec<Option<&str>> =
                tray.facts.iter().filter(|f| f.stone.as_ref() == Some(&s.id)).map(|f| f.code.as_deref()).collect();
            assert_eq!(said, [Some("not_recognised")], "{case}");
            // only a detected stone has a place seen at the leaving point
            let seen = (case == "under_seen" && s.world_id == Some(5)).then_some([0.2, 0.05]);
            assert_eq!(s.left_place, seen, "{case}");
        }
        let n = counted.len() as u64;
        assert_eq!(
            (leaving.stones, leaving.sure, leaving.added, leaving.unclear),
            (Some(2 + n), Some(2), Some(0), Some(n)),
            "{case}"
        );
    }
}

/// Where the leaving reads the leaving point's world, and which of its stones hide a kept one
/// (the Python SDK's test_the_leaving_point_world_decides_what_is_counted).
///
/// Stones 1 and 2 arrive with the tray (1 at (0.2, 0.05) in case `under_linked`); world stone 3
/// comes at g 10 (at (0.95, 0) in case `under_offboard`, beside stone 9 off the board), boxed at
/// g 10 and 11, then kept, unless the case says otherwise. A hand event at g 15 (the worked cases)
/// makes the last frame, g 17, the leaving point; a `new` event there (the others but
/// `under_linked` and `under_offboard`) makes it the calm g 14; with neither it is g 17, calm too.
#[test]
fn the_leaving_point_world_decides_what_is_counted() {
    /// A case: its name, its leaving g, its leaving fact's stones, sure, added, removed and unclear,
    /// and the stones counted as possible additions.
    type Case = (&'static str, i64, [u64; 5], &'static [Added]);
    const BOX: [f64; 2] = [0.3, 0.2];
    let cases: [Case; 10] = [
        // a hand works at g 15, after the calm g 14: the leaving point is the last frame, g 17, where 3 is kept
        // at x 0.05
        ("worked", 17, [3, 2, 0, 0, 1], &[(3, [0.05, 0.0], BOX, None)]),
        // the same, with a stone 5 boxed over 0.32 of 3's box from g 15: 5 is counted (boxed on the last frame), 3
        // is hidden
        ("worked_under_seen", 17, [3, 2, 0, 0, 1], &[(5, [0.2, 0.05], BOX, Some([0.2, 0.05]))]),
        // boxed but held at the leaving point g 14, kept after: neither version 6 nor version 7 counts it
        ("held_seen", 14, [2, 2, 0, 0, 0], &[]),
        // boxed at the leaving point g 14 with 0.32 of its box under a kept 5: version 6 counts it whatever the
        // overlap (and 5, under 3, is hidden)
        ("seen_under_kept", 14, [3, 2, 0, 0, 1], &[(3, [0.0, 0.0], BOX, Some([0.0, 0.0]))]),
        // held only after the leaving point: counted, the leaving point's row decides
        ("held_later", 14, [3, 2, 0, 0, 1], &[(3, [0.0, 0.0], BOX, None)]),
        // its box grows to 0.36 after the leaving point: counted with the leaving point's size
        ("resized_later", 14, [3, 2, 0, 0, 1], &[(3, [0.0, 0.0], BOX, None)]),
        // 0.32 of its box under a tray stone's (stone 1, there since the arrival, itself doubted for the overlap):
        // hidden
        ("under_linked", 17, [2, 1, 0, 0, 0], &[]),
        // 0.40 of its box under the box of a stone off the board (never on the tray): hidden all the same
        ("under_offboard", 17, [2, 2, 0, 0, 0], &[]),
        // 0.32 under a kept 5 at the leaving point g 14 that is gone by the last frame: hidden (and 5 is not on the
        // tray)
        ("gone_later", 14, [2, 2, 0, 0, 0], &[]),
        // kept in another mode (transit, not detected): the world keeps it on the tray all the same
        ("transit", 14, [3, 2, 0, 0, 1], &[(3, [0.0, 0.0], BOX, None)]),
    ];
    for (case, leave_g, counts, counted) in cases {
        let worked = case.starts_with("worked");
        let mut ledger = TrayLedger::new(1, "sample");
        for g in 0..18 {
            let later = g >= 15;
            let mut w = base();
            (w["g"], w["t"], w["motion"]) = (json!(g), json!(g as f64 * 0.2), json!("static"));
            w["events"] = if g == 15 && worked {
                json!([{"type": "place", "side": "left", "id": 1}])
            } else if g == 15 && case != "under_linked" && case != "under_offboard" {
                json!([{"type": "new", "id": 100}])
            } else {
                json!([])
            };
            let one = if case == "under_linked" { (0.2, 0.05) } else { (-0.6, 0.0) };
            let mut stones = vec![rock(1, one.0, one.1, "seen", true), rock(2, 0.6, 0.0, "seen", true)];
            if g >= 10 {
                let (mut mode, mut detected) = if g < 12 { ("seen", true) } else { ("kept", false) };
                if case == "held_seen" && (12..=14).contains(&g) {
                    (mode, detected) = ("carry", true);
                }
                if case == "seen_under_kept" && g <= 14 {
                    (mode, detected) = ("seen", true);
                }
                if case == "held_later" && later {
                    (mode, detected) = ("carry", false);
                }
                if case == "transit" && g >= 12 {
                    (mode, detected) = ("transit", false);
                }
                let x = if case == "under_offboard" {
                    0.95
                } else if later && case == "worked" {
                    0.05
                } else {
                    0.0
                };
                let mut stone = rock(3, x, 0.0, mode, detected);
                if case == "resized_later" && later {
                    stone["solid"]["dims"][0] = json!(0.36);
                }
                stones.push(stone);
                if case == "worked_under_seen" && later {
                    stones.push(rock(5, 0.2, 0.05, "seen", true));
                }
                if case == "seen_under_kept" || (case == "gone_later" && !later) {
                    let mode = if g < 12 { "seen" } else { "kept" };
                    stones.push(rock(5, 0.2, 0.05, mode, mode == "seen"));
                }
                if case == "under_offboard" {
                    stones.push(rock(9, 1.15, 0.0, "seen", true));
                }
            }
            with_stones(&mut w, stones);
            feed(&mut ledger, &parse(w), at(g, g as f64 * 0.2));
        }
        let closed = close(&mut ledger);
        assert_eq!(closed.len(), 1, "{case}");
        let leaving = closed[0].facts.last().expect("the leaving fact");
        let got = [leaving.stones, leaving.sure, leaving.added, leaving.removed, leaving.unclear];
        assert_eq!((leaving.g, got), (leave_g, counts.map(Some)), "{case}");
        assert_eq!(additions(&closed[0]), counted, "{case}");
    }
}
