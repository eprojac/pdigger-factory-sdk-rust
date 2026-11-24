//! A tray's visits and its presence evidence (SDK-DESIGN.md 4.6.1) through the public core: motion
//! separates two visits, and the second stop uses its own clean frame and its own unseen evidence;
//! partial calm readings, each of a visit of its own, can raise a doubt before the tray leaves. The
//! ledger's own tests (src/interpreter/trays/blink_tests.rs) check the same on the ledger itself.

mod common;

use std::io::BufRead;
use std::sync::OnceLock;

use pdigger_factory_sdk::{Frame, InterpreterCore, Tray, TrayFactKind};
use serde_json::{Value, json};

/// The first world of the synthetic tray recording, the base of every world here.
fn base() -> Value {
    static BASE: OnceLock<Value> = OnceLock::new();
    BASE.get_or_init(|| {
        let file = std::fs::File::open(common::recording("trays-synthetic")).expect("the synthetic tray recording");
        std::io::BufReader::new(flate2::read::GzDecoder::new(file))
            .lines()
            .map(|line| serde_json::from_str::<Value>(&line.expect("a line")).expect("a JSON row"))
            .find_map(|mut row| row.get_mut("world").map(Value::take))
            .expect("a world row")
    })
    .clone()
}

/// The frame of a world of small stones at these x places, in this order, with at most one event;
/// moving or static; stone `undetected` (if any) not detected in it.
fn frame(positions: &[(i64, f64)], g: i64, event: Option<(&str, i64)>, moving: bool, undetected: Option<i64>) -> Frame {
    let mut world = base();
    let seed = world["stones"][0].take();
    let stones: Vec<Value> = positions
        .iter()
        .map(|&(id, x)| {
            let mut stone = seed.clone();
            stone["id"] = json!(id);
            stone["solid"]["pos"][0] = json!(x);
            stone["solid"]["dims"] = json!([0.05, 0.04, 0.03]);
            if undetected == Some(id) {
                stone["detected"] = json!(false);
            }
            stone
        })
        .collect();
    let n = stones.len();
    world["stones"] = Value::Array(stones);
    for key in ["stones", "confirmed", "frozen", "fitted"] {
        world["stats"][key] = json!(n);
    }
    world["events"] = event.into_iter().map(|(kind, id)| json!({"type": kind, "id": id})).collect();
    let t = g as f64 / 5.0;
    (world["g"], world["t"]) = (json!(g), json!(t));
    if moving {
        world["motion"] = json!("moving");
    }
    let frame = json!({"cursor": g + 1, "session": 42, "video": "sample", "g": g, "i": 2 * g, "t": t,
        "past_valid": true, "dt_past": 0.6, "world": world});
    serde_json::from_value(frame).expect("a frame")
}

fn codes(tray: &Tray, kind: TrayFactKind) -> Vec<&str> {
    tray.facts.iter().filter(|f| f.kind == kind).map(|f| f.code.as_deref().unwrap_or("")).collect()
}

/// Motion separates two visits. At the second stop a silent absence interrupts its one three-frame
/// quiet run: stone 3 is still unseen when the tray leaves, seen in between, or seen at leaving.
#[test]
fn a_second_stop_uses_its_own_clean_frame_and_its_actual_unseen_evidence() {
    for sight_g in [None, Some(24), Some(25)] {
        let mut core = InterpreterCore::new();
        for g in 0..26 {
            let mut positions = vec![(1, if g <= 22 { -0.5 } else { -0.48 }), (2, 0.0), (3, 0.6)];
            if g == 23 {
                positions.remove(1);
            }
            let event = ((14..=20).contains(&g) || g >= 24).then_some(("new", 100));
            let undetected = (g >= 21 && Some(g) != sight_g).then_some(3);
            core.feed(frame(&positions, g, event, g == 13, undetected));
        }
        core.end();
        let closed = core.trays();
        assert_eq!(closed.len(), 1, "{sight_g:?}");
        let tray = &closed[0];
        assert_eq!(tray.leaving.as_ref().map(|p| (p.g, p.t)), Some((22, 4.4)), "{sight_g:?}");
        assert_eq!(tray.stones[0].left_place, Some([-0.5, 0.0]));
        assert_eq!(tray.stones[2].left_place, None);
        assert_eq!(tray.stones[2].sure, sight_g == Some(25), "{sight_g:?}");
        let want: &[&str] = if sight_g == Some(25) { &[] } else { &["unseen"] };
        assert_eq!(codes(tray, TrayFactKind::Unsure), want, "{sight_g:?}");
    }
}

/// Two partial calm readings, each of a visit of its own, raise the doubt on stone 3 before the
/// tray leaves.
#[test]
fn independent_partial_calm_readings_can_raise_a_doubt_before_leaving() {
    let mut core = InterpreterCore::new();
    let mut unsure = Vec::new();
    for g in 0..20 {
        let partial = g == 15 || g == 19;
        let mut positions = vec![(1, -0.6), (2, 0.0), (3, 0.6)];
        if partial {
            positions.remove(1);
        }
        let update =
            core.feed(frame(&positions, g, (g == 12 || g == 16).then_some(("new", 100)), false, partial.then_some(3)));
        unsure
            .extend(update.tray_facts.iter().filter(|f| f.kind == TrayFactKind::Unsure).map(|f| (f.g, f.code.clone())));
    }
    assert_eq!(unsure, [(19, Some("unseen".to_owned()))]);
    core.end();
    assert!(!core.trays()[0].stones[2].sure);
}
