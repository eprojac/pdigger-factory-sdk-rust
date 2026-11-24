//! Version 4 of the tray rules (SDK-DESIGN.md 4.6.1): a certain removal needs a hand on the stone,
//! and a visit continues the open tray only when nine tenths of its first frame's stones were seen
//! on it.

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
