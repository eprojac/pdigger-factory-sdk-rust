//! Looks (spec 4.6.2) against the reference, on the synthetic looks of `fixtures/looks`: read by
//! value as the models read them, the same double, pair count and permille for every pair, and no
//! score where the reference has none.

mod common;

use std::collections::BTreeMap;

use pdigger_factory_sdk::models::{StoneLook, StoneView};
use pdigger_factory_sdk::{Frame, look_pairing, look_permille, look_score, usable_look};
use serde::Deserialize;
use serde_json::value::RawValue;

use common::fixtures;

/// One pair of the reference's pairing: `[a, b, score as the shortest text of the double, its pair
/// count, its permille]`.
type ScoreRow = (String, String, Option<String>, Option<i64>, Option<i64>);

/// `fixtures/looks/looks-synthetic.json`: the looks as sent, the ones the rules cannot read, the
/// ones the tray rules do not use, the ones the models drop, and the reference's pairing of pairs
/// of them.
#[derive(Deserialize)]
struct Vectors {
    looks: BTreeMap<String, Box<RawValue>>,
    unreadable: Vec<String>,
    tray_unusable: Vec<String>,
    model_none: Vec<String>,
    scores: Vec<ScoreRow>,
}

/// A stone with this look, read as a frame's stone is.
fn stone(look: &str) -> StoneView {
    let s: StoneView = serde_json::from_str(&format!(r#"{{"id": 7, "detected": true, "look": {look}}}"#)).unwrap();
    assert!(s.id == 7 && s.detected, "{s:?}");
    s
}

fn vectors() -> (Vectors, BTreeMap<String, Option<StoneLook>>) {
    let text = std::fs::read_to_string(fixtures().join("looks").join("looks-synthetic.json")).unwrap();
    let v: Vectors = serde_json::from_str(&text).unwrap();
    let looks = v.looks.iter().map(|(name, raw)| (name.clone(), stone(raw.get()).look)).collect();
    (v, looks)
}

#[test]
fn look_score_matches_the_reference() {
    let (v, looks) = vectors();
    assert!(v.scores.len() >= 300 && v.unreadable.len() >= 6 && v.tray_unusable.len() >= 5 && v.model_none.len() >= 6);
    for (name, lk) in &looks {
        assert_eq!(lk.is_none(), v.model_none.contains(name), "{name}: dropped by the models");
        if let Some(lk) = lk {
            let unusable = v.unreadable.contains(name) || v.tray_unusable.contains(name);
            assert_eq!(usable_look(lk), !unusable, "{name}: used on a tray");
        }
    }
    let mut scored = 0;
    for (a, b, want, n, p) in &v.scores {
        let got = match (&looks[a], &looks[b]) {
            (Some(x), Some(y)) => {
                assert_eq!(look_score(x, y), look_pairing(x, y).map(|g| g.0), "{a} against {b}");
                look_pairing(x, y)
            }
            _ => None,
        };
        let Some(w) = want else {
            assert!(got.is_none() && n.is_none() && p.is_none(), "{a} against {b}: {got:?}, want no score");
            continue;
        };
        let w: f64 = w.parse().unwrap();
        let (g, gn) = got.unwrap_or_else(|| panic!("{a} against {b}: no score, want {w}"));
        assert_eq!(g.to_bits(), w.to_bits(), "{a} against {b}: {g}, want {w}");
        assert_eq!((Some(gn), Some(look_permille(g))), (*n, *p), "{a} against {b}: n and permille");
        scored += 1;
    }
    assert!(scored >= 150, "only {scored} pairs scored");
}

#[test]
fn what_is_not_a_look() {
    let (_, looks) = vectors();
    let good = looks["brick-b"].clone().unwrap();
    let mut empty = good.clone();
    empty.nu = 0;
    assert_eq!(look_score(&empty, &good), None);
    assert_eq!(look_score(&good, &empty), None);
    let mut wide = good.clone();
    (wide.nu, wide.nv) = (1 << 40, 1 << 40); // the product would overflow: still more than LOOK_CELLS
    assert_eq!(look_pairing(&wide, &good), None);
    (wide.nu, wide.nv) = (i64::MAX, 2);
    assert_eq!(look_pairing(&good, &wide), None);
    assert_eq!(look_score(&good, &good), Some(1.0));
    let permille: Vec<i64> = [0.8535, -0.0004, -0.0006, 1.0].into_iter().map(look_permille).collect();
    assert_eq!(permille, [854, 0, -1, 1000]);
    assert_eq!(pdigger_factory_sdk::interpreter::LOOK_CELLS, 4096);
}

/// Numbers by value, whole numbers from -2^53 to 2^53; a look that does not fit is dropped and its
/// stone and frame still read.
#[test]
fn a_look_is_read_by_value() {
    let (v, looks) = vectors();
    for name in &v.model_none {
        assert!(stone(v.looks[name].get()).look.is_none(), "{name}: kept");
    }
    assert_eq!(looks["type-nu-whole-float"].as_ref().map(|l| l.nu), Some(18));
    let over = looks["type-over-4096-cells"].as_ref().expect("a look of 65 x 64 cells fits StoneLook");
    assert_eq!(look_pairing(over, over), None);
    for bad in ["null", "[1, 2]", "\"look\"", "{}"] {
        assert!(stone(bad).look.is_none(), "{bad}: kept");
    }
    let base: serde_json::Map<String, serde_json::Value> = serde_json::from_str(v.looks["brick-b"].get()).unwrap();
    let with = |key: &str, raw: &str| {
        let mut parts: Vec<String> =
            base.iter().filter(|(k, _)| *k != key).map(|(k, x)| format!("{}: {x}", serde_json::json!(k))).collect();
        parts.push(format!("{}: {raw}", serde_json::json!(key)));
        stone(&format!("{{{}}}", parts.join(", "))).look
    };
    for (key, raw) in [("sp", "1e400"), ("g", "9007199254740994"), ("g", "1.5"), ("v", "true"), ("nu", "\"18\"")] {
        assert!(with(key, raw).is_none(), "{key} {raw}: kept");
    }
    assert_eq!(with("g", "9007199254740992").map(|l| l.g), Some(1 << 53));
    assert_eq!(with("q", "1e-400").map(|l| l.q), Some(0.0));
    assert_eq!(with("zz", "1").map(|l| l.extra.contains_key("zz")), Some(true));
    assert!(serde_json::from_str::<StoneView>(r#"{"id": "seven"}"#).is_err());
    // a number no double holds drops only the look: the frame and the stone read
    let line = r#"{"cursor": 3, "g": 3, "t": 0.6, "world": {"stones": [{"id": 4, "look": {"sp": 1e400}}]}}"#;
    let frame: Frame = serde_json::from_str(line).unwrap();
    let world = frame.world.unwrap();
    assert!(world.stones[0].id == 4 && world.stones[0].look.is_none());
    let back: serde_json::Value = serde_json::to_value(stone(v.looks["brick-b"].get())).unwrap();
    assert_eq!(back["look"]["nu"], serde_json::json!(looks["brick-b"].as_ref().unwrap().nu));
}
