//! Version 4 of the tray rules (SDK-DESIGN.md 4.6.1): a certain removal needs a hand on the stone,
//! and a visit continues the open tray only on the camera's word: of the stones no hand holds at
//! its first frame, the tray saw one the world detects, and nine tenths of the detected ones or of
//! them all.

use pdigger_factory_sdk::{Frame, InterpreterCore, Tray, TrayFactKind, TrayStoneStatus};
use serde_json::{Value, json};

fn stone(id: i64, x: f64) -> Value {
    json!({"id": id, "mode": "kept", "family": "rect", "shape": "frozen", "detected": true, "uncertain": false,
           "side": null, "outline": [[10.0, 10.0], [20.0, 10.0], [20.0, 20.0], [10.0, 20.0]],
           "solid": {"pos": [x, 0.0, 0.046], "q": [0.0, 0.0, 0.0, 1.0], "dims": [0.3, 0.2, 0.09]}})
}

/// One frame of each step: its stones, its events, a hand touching (or free), static or moving.
struct Step {
    stones: Vec<Value>,
    events: Vec<Value>,
    touch: bool,
    moving: bool,
}

fn step(stones: Vec<Value>) -> Step {
    Step { stones, events: vec![], touch: false, moving: false }
}

fn play(steps: Vec<Step>) -> Vec<Tray> {
    let mut core = InterpreterCore::new();
    for (g, s) in steps.into_iter().enumerate() {
        let n = s.stones.len();
        let left = if s.touch { "touch" } else { "free" };
        let world = json!({"schema": "stw1", "session": 1, "video": "dry1", "g": g, "t": g as f64 / 5.0,
            "motion": if s.moving { "moving" } else { "static" }, "logF": 0.08, "stones": s.stones,
            "hands": {"left": {"state": left, "stone": null}, "right": {"state": "free", "stone": null}},
            "events": s.events, "stats": {"stones": n, "confirmed": n}});
        let frame = json!({"cursor": g + 1, "session": 1, "video": "dry1", "g": g, "i": 2 * g, "t": g as f64 / 5.0,
            "past_valid": true, "dt_past": 0.6, "world": world});
        core.feed(serde_json::from_value::<Frame>(frame).unwrap());
    }
    core.end();
    core.trays()
}

/// Stones 1 and 2 arrive; `touch` lands on stone 2 at `touch_g`; stone 2 leaves the world at
/// `loss_g` with `loss`; a hand works away from the stones at g20 and g40, so a calm moment follows
/// at g43, more than 3 s after the loss.
fn removal(touch: Option<Value>, touch_g: usize, loss: Value, loss_g: usize) -> Tray {
    let mut steps = Vec::new();
    for g in 0..50 {
        let stones = if g >= loss_g { vec![stone(1, -0.6)] } else { vec![stone(1, -0.6), stone(2, 0.6)] };
        let mut s = step(stones);
        if g == touch_g && touch.is_some() {
            s.events = vec![touch.clone().unwrap()];
            s.touch = true;
        } else if g == loss_g {
            s.events = vec![loss.clone()];
        } else {
            s.touch = g == 20 || g == 40;
        }
        steps.push(s);
    }
    play(steps).remove(0)
}

fn removed(tray: &Tray) -> Vec<(Option<bool>, i64)> {
    tray.facts.iter().filter(|f| f.kind == TrayFactKind::Removed).map(|f| (f.certain, f.g)).collect()
}

const ABSENT: fn() -> Value = || json!({"type": "delete", "id": 2, "why": "absent"});

#[test]
fn a_loss_by_a_hand_is_a_hand_touch_and_one_by_no_hand_is_not() {
    for (loss, touched) in [
        (json!({"type": "drop", "id": 2, "side": "right", "why": "t-release"}), true),
        (json!({"type": "gone", "id": 2, "side": "right"}), true),
        (json!({"type": "delete", "id": 2, "why": "hand"}), true),
        (json!({"type": "delete", "id": 2, "why": "carried-away"}), true),
        (ABSENT(), false),
    ] {
        let want = if touched { (Some(true), 43) } else { (Some(false), 49) };
        assert_eq!(removed(&removal(None, usize::MAX, loss.clone(), 30)), vec![want], "{loss}");
    }
}

#[test]
fn a_touch_before_a_silent_loss_counts_only_when_it_is_a_hand_touch() {
    for (touch, touched) in [
        (json!({"type": "lift", "id": 2, "side": "right"}), true),
        (json!({"type": "push", "id": 2, "side": "left"}), true),
        (json!({"type": "push", "id": 2, "why": "reseat"}), false),
        (json!({"type": "freeze", "id": 2}), false),
    ] {
        let want = if touched { (Some(true), 43) } else { (Some(false), 49) };
        assert_eq!(removed(&removal(Some(touch.clone()), 24, ABSENT(), 28)), vec![want], "{touch}");
    }
}

#[test]
fn an_open_removal_no_hand_touched_is_named_no_hand() {
    let tray = removal(None, usize::MAX, ABSENT(), 30);
    let codes: Vec<_> = tray.facts.iter().filter(|f| f.kind == TrayFactKind::Removed).map(|f| f.code.clone()).collect();
    assert_eq!(codes, vec![Some("not_found".to_string())]);
    let open = tray.facts.last().unwrap().open.clone().unwrap();
    assert_eq!(
        open.iter().map(|o| (o.kind.clone(), o.cause.as_str())).collect::<Vec<_>>(),
        [(TrayFactKind::Removed, "no_hand")]
    );
}

#[test]
fn a_touch_before_both_the_last_calm_moment_and_the_three_seconds_does_not_count() {
    // pushed at g5 (t 1.0); a calm moment at g23 (t 4.6); lost silently at g30 (t 6.0), last seen
    // at g29: the window starts at min(4.6, 5.8 - 3.0) = 2.8, after the push
    let tray = removal(Some(json!({"type": "push", "id": 2, "side": "left"})), 5, ABSENT(), 30);
    assert_eq!(removed(&tray), vec![(Some(false), 49)]);
}

#[test]
fn a_touch_after_the_last_calm_moment_counts_while_hands_keep_working_until_the_loss() {
    // a calm moment at g23 (t 4.6); pushed at g25 (t 5.0); hands work until stone 2 is lost
    // silently at g44 (t 8.8), last seen at g43: the window starts at min(4.6, 8.6 - 3.0) = 4.6
    let mut steps = Vec::new();
    for g in 0..66 {
        let stones = if g >= 44 { vec![stone(1, -0.6)] } else { vec![stone(1, -0.6), stone(2, 0.6)] };
        let mut s = step(stones);
        s.touch = g == 20 || (25..=44).contains(&g) || g == 55;
        if g == 25 {
            s.events = vec![json!({"type": "push", "id": 2, "side": "left"})];
        } else if g == 44 {
            s.events = vec![ABSENT()];
        }
        steps.push(s);
    }
    assert_eq!(removed(&play(steps).remove(0)), vec![(Some(true), 58)]);
}

/// Ten stones arrive; the conveyor moves; then a board whose first frame shows `known` of them and
/// 10 - `known` stones the tray never saw.
fn board(known: i64) -> Vec<Tray> {
    let first: Vec<Value> = (0..10).map(|i| stone(i, -0.9 + 0.2 * i as f64)).collect();
    let next: Vec<Value> = (0..10)
        .map(|i| if i < known { stone(i, -0.9 + 0.2 * i as f64) } else { stone(100 + i, -0.9 + 0.2 * i as f64) })
        .collect();
    let mut steps: Vec<Step> = (0..12).map(|_| step(first.clone())).collect();
    for _ in 0..3 {
        steps.push(Step { moving: true, ..step(first.clone()) });
    }
    steps.push(Step {
        events: (known..10).map(|i| json!({"type": "new", "id": 100 + i})).collect(),
        ..step(next.clone())
    });
    steps.extend((0..11).map(|_| step(next.clone())));
    play(steps)
}

#[test]
fn a_visit_with_nine_tenths_of_its_stones_seen_goes_on_as_the_same_tray() {
    assert_eq!(board(9).iter().map(|t| t.n).collect::<Vec<_>>(), [1]);
}

#[test]
fn a_visit_with_fewer_than_nine_tenths_of_its_stones_seen_is_a_new_tray() {
    let trays = board(8);
    assert_eq!(trays.iter().map(|t| t.n).collect::<Vec<_>>(), [1, 2]);
    let on: Vec<usize> =
        trays.iter().map(|t| t.stones.iter().filter(|s| s.status == TrayStoneStatus::On).count()).collect();
    assert_eq!(on, [10, 10]);
}

/// Ten stones arrive; the conveyor moves; the next stop's first frame shows seven of them where they
/// stood, detected or only remembered, and the world loses them one after another.
fn drove_off(detected: bool) -> Vec<Tray> {
    let first: Vec<Value> = (0..10).map(|i| stone(i, -0.9 + 0.2 * i as f64)).collect();
    let mut steps: Vec<Step> = (0..12).map(|_| step(first.clone())).collect();
    for _ in 0..3 {
        steps.push(Step { moving: true, ..step(first.clone()) });
    }
    let mut left: Vec<Value> = first[3..]
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s["detected"] = json!(detected);
            s
        })
        .collect();
    steps.extend((0..2).map(|_| step(left.clone())));
    for i in 3..10 {
        left.remove(0);
        steps.push(Step { events: vec![json!({"type": "delete", "id": i, "why": "absent"})], ..step(left.clone()) });
    }
    steps.extend((0..10).map(|_| step(left.clone())));
    play(steps)
}

#[test]
fn a_visit_whose_first_frame_only_remembers_the_trays_stones_is_a_new_tray() {
    // a board that drove off: the stones the world remembers where it stood say nothing about the belt
    let trays = drove_off(false);
    assert_eq!(trays.iter().map(|t| t.n).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(trays[0].stones.iter().filter(|s| s.status == TrayStoneStatus::On).count(), 10);
    assert!(!trays[0].facts.iter().any(|f| matches!(f.kind, TrayFactKind::Removed | TrayFactKind::Added)));
}

#[test]
fn a_visit_whose_first_frame_detects_the_trays_stones_goes_on_as_the_same_tray() {
    let trays = drove_off(true);
    assert_eq!(trays.iter().map(|t| t.n).collect::<Vec<_>>(), [1]);
    assert_eq!(trays[0].stones.iter().filter(|s| s.status == TrayStoneStatus::Removed).count(), 10);
    let removals: Vec<_> = trays[0].facts.iter().filter(|f| f.kind == TrayFactKind::Removed).collect();
    assert!(!removals.is_empty() && removals.iter().all(|f| f.certain == Some(false)));
}

/// Ten stones arrive; the conveyor jolts; the next stop's first frame shows `kept` of them only
/// remembered (not detected) and the next `detected` of them detected where they stood, `fresh`
/// detected stones with ids the tray never saw (a rename, or another board's), `unknown`
/// remembered ids it never saw and `held` detected stones it never saw in a hand (mode `carry`,
/// gone after that frame); after it the world detects them all.
fn jolt(kept: i64, detected: i64, fresh: i64, unknown: i64, held: i64) -> Vec<Tray> {
    let first: Vec<Value> = (0..10).map(|i| stone(i, -0.9 + 0.2 * i as f64)).collect();
    let mut steps: Vec<Step> = (0..12).map(|_| step(first.clone())).collect();
    for _ in 0..3 {
        steps.push(Step { moving: true, ..step(first.clone()) });
    }
    let mut next: Vec<Value> = (0..kept + detected)
        .map(|i| {
            let mut s = stone(i, -0.9 + 0.2 * i as f64);
            s["detected"] = json!(i >= kept);
            s
        })
        .collect();
    next.extend((0..fresh).map(|k| stone(100 + k, -0.9 + 0.2 * (kept + detected + k) as f64)));
    next.extend((0..unknown).map(|k| {
        let mut s = stone(200 + k, -0.9 + 0.2 * k as f64);
        s["solid"]["pos"][1] = json!(0.4);
        s["detected"] = json!(false);
        s
    }));
    let mut in_hand = next.clone();
    in_hand.extend((0..held).map(|k| {
        let mut s = stone(300 + k, -0.9 + 0.2 * k as f64);
        s["solid"]["pos"][1] = json!(-0.4);
        s["mode"] = json!("carry");
        s
    }));
    steps.push(Step { events: (0..fresh).map(|k| json!({"type": "new", "id": 100 + k})).collect(), ..step(in_hand) });
    let all: Vec<Value> = next
        .iter()
        .map(|s| {
            let mut s = s.clone();
            s["detected"] = json!(true);
            s
        })
        .collect();
    steps.extend((0..11).map(|_| step(all.clone())));
    play(steps)
}

fn numbers(trays: &[Tray]) -> Vec<i64> {
    trays.iter().map(|t| t.n).collect()
}

#[test]
fn a_visit_goes_on_when_its_detected_stones_were_seen_whatever_else_it_remembers() {
    // nine detected tray stones and two remembered ids the tray never saw: all the detected stones were seen
    assert_eq!(numbers(&jolt(0, 9, 0, 2, 0)), [1]);
}

#[test]
fn a_visit_whose_only_detected_stone_is_new_is_a_new_tray_however_many_it_remembers() {
    // nine remembered tray stones and one detected stone the tray never saw: no detected stone says it is the same board
    assert_eq!(numbers(&jolt(9, 0, 1, 0, 0)), [1, 2]);
}

#[test]
fn a_jolt_that_leaves_most_stones_remembered_and_renames_one_it_sees_goes_on() {
    // two of the three detected stones were seen (less than nine tenths), but nine of all ten were: the same board
    assert_eq!(numbers(&jolt(7, 2, 1, 0, 0)), [1]);
}

#[test]
fn a_visit_with_few_of_its_stones_seen_is_a_new_tray_although_some_were_detected() {
    // two of the four detected stones and two of all ten were seen
    assert_eq!(numbers(&jolt(0, 2, 2, 6, 0)), [1, 2]);
}

#[test]
fn a_visit_leaves_out_the_stones_a_hand_holds() {
    // nine detected tray stones and two held stones the tray never saw: only the nine count, and all were seen
    assert_eq!(numbers(&jolt(0, 9, 0, 0, 2)), [1]);
}
