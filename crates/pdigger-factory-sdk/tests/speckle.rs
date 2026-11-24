//! The unit's speckle answers (spec 4.6.12): read by value, the tray stones that hold them, the
//! answer that comes after its tray left, and the answers as JSON; the leaving's answer (spec
//! 4.6.13) on tray 3 of `trays-speckle-synthetic` as values, and the self-check's C3, C4 and C9 on
//! its facts made broken on purpose. The golden tests compare every answer read (`speckle.txt`)
//! and the facts and reports the leaving's answers give.

mod common;

use std::sync::OnceLock;

use pdigger_factory_sdk::interpreter::{MOVED_CONFIDENCE, SPECKLE_CLEAN, SPECKLE_CODES};
use pdigger_factory_sdk::models::WorldLayer;
use pdigger_factory_sdk::{
    Replay, SpeckleSide, StoneSpeckle, Tray, TrayEvidence, TrayFact, TrayFactKind, TraySpeckle, TrayStone,
};
use serde_json::{Value, json};

use common::fixtures;

/// A well-formed answer: whole numbers written with a fraction and finds without `at`, `score` or
/// `second` (null) still fit; unknown members are kept.
const ANSWER: &str = r#"{"v":1,"tray":2.0,"ref":{"g":4,"t":0.8},"at":{"g":20,"t":4},"device":"d","gpu_ms":1.5,
"wall_ms":2,"stones":[{"id":1,"at":null,"score":null,"second":null},{"id":2,"x":1}],
"refs":[{"id":3,"at":4.0,"score":0.5,"second":-1}],"pairs":[[1,3]],"y":true}"#;

fn world(speckle: &str) -> WorldLayer {
    serde_json::from_str(&format!(r#"{{"g":7,"speckle":{speckle}}}"#)).unwrap()
}

#[test]
fn an_answer_is_read_by_value() {
    let w = world(ANSWER);
    let sp = w.speckle.as_ref().unwrap();
    assert_eq!((w.g, sp.v, sp.tray, sp.reference.g, sp.reference.t, sp.at.g), (7, 1, 2, 4, 0.8, 20));
    assert_eq!((sp.device.as_str(), sp.gpu_ms, sp.wall_ms, sp.stones.len()), ("d", 1.5, 2.0, 2));
    assert_eq!((sp.stones[0].at, sp.stones[0].score, sp.stones[1].second), (None, None, None));
    assert_eq!((sp.refs[0].at, sp.refs[0].score, sp.refs[0].second), (Some(4), Some(0.5), Some(-1.0)));
    assert_eq!(sp.pairs, vec![[1, 3]]);
    assert!(sp.extra.contains_key("y") && sp.stones[1].extra.contains_key("x"));
    let again: WorldLayer = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(again.speckle, w.speckle, "the answer as JSON again");
    // an answer that does not fit is dropped; the world reads as it would without it
    let base: Value = serde_json::from_str(ANSWER).unwrap();
    let mut bad: Vec<String> = ["null", "5", "\"x\"", "[]", "{}", "1e400"].map(String::from).to_vec();
    for (k, v) in [
        ("v", json!(2)),
        ("v", json!(true)),
        ("tray", json!("2")),
        ("tray", json!(1.5)),
        ("ref", json!({"g": 4})),
        ("at", json!({"g": 1.5, "t": 0})),
        ("at", json!(3)),
        ("device", json!(5)),
        ("gpu_ms", json!("1")),
        ("wall_ms", Value::Null),
        ("stones", json!({})),
        ("stones", json!([5])),
        ("refs", json!([{"id": 1.5}])),
        ("refs", json!([{"id": 1, "at": "x"}])),
        ("refs", json!([{"id": 1, "score": 0.5}])),
        ("refs", json!([{"id": 1, "score": 0.5, "second": "x"}])),
        ("pairs", json!([[1]])),
        ("pairs", json!([5])),
        ("pairs", json!([[1, 2.5]])),
        ("pairs", json!({})),
    ] {
        let mut m = base.as_object().unwrap().clone();
        if v.is_null() {
            m.remove(k);
        } else {
            m.insert(k.to_string(), v);
        }
        bad.push(Value::Object(m).to_string());
    }
    for s in bad {
        let w = world(&s);
        assert!(w.speckle.is_none() && w.g == 7, "{s}");
    }
}

#[test]
fn the_tray_stones_hold_the_newest_answer_read_while_their_tray_was_on() {
    let path = fixtures().join("recordings/trays-speckle-synthetic.ndjson.gz");
    let mut updates = Replay::open(&path).unwrap().updates();
    let all: Vec<_> = updates.by_ref().map(Result::unwrap).collect();
    let gs: Vec<i64> = all.iter().flat_map(|u| u.speckle.iter().map(|a| a.g)).collect();
    assert_eq!(gs, [24, 30, 32, 36, 46, 50, 123], "the frames that gave an answer");
    let revised: Vec<_> = all
        .iter()
        .filter(|u| {
            u.is_revision && u.frame.as_ref().and_then(|f| f.world.as_ref()).is_some_and(|w| w.speckle.is_some())
        })
        .collect();
    assert!(revised.len() == 1 && revised[0].speckle.is_empty(), "a revision's answer is not read again");
    let core = updates.core();
    let trays = core.trays();
    assert_eq!(trays.len(), 3);
    let (one, two) = ("20261118T000000.8Z-1-", "20261118T000007.8Z-2-");
    let stone = |t: &Tray, id: String| -> StoneSpeckle { t.stone(&id).and_then(|s| s.speckle.clone()).unwrap() };
    let s1 = stone(&trays[0], format!("{one}1")); // g 32's
    assert!(s1.found.is_none() && s1.found_world == Some(1) && s1.clean);
    let s2 = stone(&trays[0], format!("{one}2")); // g 36's: the tray was still on
    assert!(s2.score == Some(0.8) && s2.same());
    let s4 = stone(&trays[0], format!("{one}4")); // g 30's, not g 46's
    assert!(s4.pair == Some(format!("{one}5")) && !s4.same() && s4.score == Some(0.5));
    let s13 = stone(&trays[1], format!("{two}13"));
    assert!(s13.side == SpeckleSide::Later && s13.found == Some(format!("{two}13")));
    let s12 = stone(&trays[1], format!("{two}12"));
    assert!(s12.pair.is_none() && !s12.clean && s12.side.as_str() == "arrival");
    let answers = core.speckle();
    let late = &answers[4];
    assert_eq!((answers.len(), late.g, late.tray, late.unit_tray), (7, 46, 1, 1));
    assert!(late.stone(&format!("{one}4")).is_some() && late.stone(&format!("{one}1")).is_none());
    assert!(late.to_string() == late.describe() && late.wall.is_some());
    let json = serde_json::to_value(late).unwrap();
    assert_eq!(json["stones"][0]["side"], "arrival");
    assert_eq!(serde_json::from_value::<TraySpeckle>(json).unwrap(), *late);
    assert_eq!(serde_json::from_value::<Tray>(serde_json::to_value(&trays[0]).unwrap()).unwrap(), trays[0]);
}

/// The trays of the synthetic speckle recording.
fn speckle_trays() -> &'static [Tray] {
    static TRAYS: OnceLock<Vec<Tray>> = OnceLock::new();
    TRAYS.get_or_init(|| {
        Replay::open(fixtures().join("recordings/trays-speckle-synthetic.ndjson.gz")).unwrap().trays().unwrap()
    })
}

/// The index of the tray stone whose id ends with that world id (`...-3-61`).
fn stone_of(t: &Tray, wid: i64) -> usize {
    let suffix = format!("-{}-{wid}", t.n);
    t.stones.iter().position(|s| s.id.ends_with(&suffix)).unwrap_or_else(|| panic!("tray {}: no ...{suffix}", t.n))
}

fn with_facts(t: &Tray, facts: Vec<TrayFact>) -> Vec<String> {
    Tray { facts, ..t.clone() }.check()
}

fn with_stone(t: &Tray, i: usize, change: impl FnOnce(&mut TrayStone)) -> Vec<String> {
    let mut t = t.clone();
    change(&mut t.stones[i]);
    t.check()
}

/// Every check found the code.
fn want(code: &str, checks: &[(&str, Vec<String>)]) {
    for (name, got) in checks {
        assert!(got.iter().any(|c| c == code), "{name}: {code} not found in {got:?}");
    }
}

/// Tray 3's leaving beyond its golden files: the identified facts of its answer as values, which
/// stones the answer linked or confirmed, every stone's evidence and the confidence of every
/// change; no stone has evidence while its tray is on, and trays 1 and 2, none of whose answers is
/// a leaving's, leave without one.
#[test]
fn the_leavings_answer_confirms_relinks_and_finds_tray_3s_stones() {
    let path = fixtures().join("recordings/trays-speckle-synthetic.ndjson.gz");
    let mut open = 0;
    for u in Replay::open(&path).unwrap().updates() {
        for s in u.unwrap().state.tray.iter().flat_map(|t| t.stones.clone()) {
            assert!(s.evidence.is_none() && !s.by_speckle, "{s:?}");
            open += 1;
        }
    }
    assert!(open > 0);
    let trays = speckle_trays();
    for s in trays[..2].iter().flat_map(|t| &t.stones) {
        assert_eq!((s.by_speckle, s.evidence.clone()), (false, Some(TrayEvidence::Undecided)), "{}", s.id);
    }
    let t = &trays[2];
    assert_eq!(t.check(), Vec::<String>::new());
    let id = |wid| t.stones[stone_of(t, wid)].id.clone();
    let leaving = t.leaving.unwrap();
    let identified: Vec<_> = t
        .facts
        .iter()
        .filter(|x| x.kind == TrayFactKind::Identified)
        .map(|x| {
            assert!(x.certain.is_none() && x.g == leaving.g, "{x:?}");
            (x.stone.clone().unwrap(), x.code.as_deref().unwrap(), x.world_id.unwrap(), x.from_world, x.score, x.margin)
        })
        .collect();
    assert!(t.facts.iter().filter(|x| x.kind == TrayFactKind::Identified).all(|x| x.bar == Some(SPECKLE_CLEAN)));
    assert_eq!(
        identified,
        [
            (id(61), "speckle_same", 61, None, Some(600), Some(500)),
            (id(62), "speckle_same", 62, None, Some(620), Some(520)),
            (id(63), "speckle_same", 63, None, Some(600), Some(480)),
            (id(65), "speckle_found", 75, None, Some(660), Some(560)),
            (id(66), "speckle_moved", 67, Some(66), Some(640), Some(540)),
            (id(67), "speckle_moved", 66, Some(67), Some(630), Some(530)),
        ]
    );
    assert!(identified.iter().all(|x| SPECKLE_CODES.contains(&x.1)));
    // linked or confirmed by the answer: sure and by speckle; 68's pair would have moved it, which nothing
    // disturbed, and 64 was removed before
    use TrayEvidence::{Found, NotFound, Undecided};
    for (wid, evidence) in [
        (61, Found),
        (62, Found),
        (63, Found),
        (64, Found),
        (65, Found),
        (66, Found),
        (67, Found),
        (68, Found),
        (69, NotFound),
        (71, NotFound),
        (72, Found),
        (73, Undecided),
        (78, Found),
    ] {
        let s = &t.stones[stone_of(t, wid)];
        let by_speckle = (61..=67).contains(&wid) && wid != 64;
        assert_eq!((s.evidence.clone(), s.by_speckle), (Some(evidence), by_speckle), "{wid}");
        assert!(!by_speckle || s.sure, "{wid}");
    }
    let s68 = &t.stones[stone_of(t, 68)];
    assert_eq!((s68.sure, s68.touched, s68.world_id), (true, None, Some(68)));
    let r = t.report().unwrap();
    let confidence: Vec<(String, String, f64)> = [&r.removed, &r.added, &r.moved]
        .into_iter()
        .flatten()
        .map(|c| (c.kind.to_string(), c.stone.clone(), c.confidence))
        .collect();
    let change = |kind: &str, wid, c: f64| (kind.to_string(), id(wid), c);
    assert_eq!(
        confidence,
        [
            change("removed", 64, 0.25), // certain, but the leaving's answer found it on the tray
            change("removed", 69, 0.71),
            change("added", 71, 0.96),
            change("added", 72, 0.02),
            change("added", 73, 0.52),
            change("added", 78, 0.02),
            change("moved", 62, MOVED_CONFIDENCE),
            change("moved", 65, MOVED_CONFIDENCE),
            change("moved", 66, MOVED_CONFIDENCE),
            change("moved", 67, MOVED_CONFIDENCE),
        ]
    );
    // as JSON
    let json = serde_json::to_value(&t.stones[stone_of(t, 65)]).unwrap();
    assert_eq!((&json["by_speckle"], &json["evidence"]), (&json!(true), &json!("found")));
    assert_eq!(serde_json::to_value(&r.removed[0]).unwrap()["confidence"], json!(0.25));
    assert_eq!(serde_json::from_value::<Tray>(serde_json::to_value(t).unwrap()).unwrap(), *t);
    // read without them, a tray stone is as it was before the leaving's answer
    let mut bare = serde_json::to_value(&t.stones[0]).unwrap();
    for key in ["by_speckle", "evidence"] {
        bare.as_object_mut().unwrap().remove(key);
    }
    let bare: TrayStone = serde_json::from_value(bare).unwrap();
    assert_eq!((bare.by_speckle, bare.evidence), (false, None));
}

/// The self-check (spec 4.6.10) of tray 3, whose stones not sure are sure again through the
/// leaving's answer, on copies broken on purpose.
#[test]
fn c3_c4_and_c9_allow_the_leavings_speckle_facts_and_nothing_else() {
    let t = &speckle_trays()[2];
    let i = t.facts.iter().position(|x| x.code.as_deref() == Some("speckle_same")).unwrap();
    let same = t.facts[i].clone(); // 61's, after its unsure fact
    let s = stone_of(t, 61);
    assert!(same.stone.as_ref() == Some(&t.stones[s].id) && t.stones[s].sure);
    let insert = |xs: &[TrayFact]| [&t.facts[..=i], xs, &t.facts[i + 1..]].concat();
    let first = |f: &dyn Fn(&TrayFact) -> bool| t.facts.iter().find(|x| f(x)).unwrap().clone();
    let unsure = first(&|x| x.kind == TrayFactKind::Unsure && x.stone == same.stone);
    let gone = first(&|x| x.kind == TrayFactKind::Removed && x.certain == Some(true)); // 64's
    let look = TrayFact { code: Some("look_found".into()), ..same.clone() };
    let late = TrayFact { stone: gone.stone.clone(), ..same.clone() };
    let n = t.facts.len();
    want(
        "C3",
        &[
            ("two identified facts of the speckle", with_facts(t, insert(std::slice::from_ref(&same)))),
            ("identified by look after unsure", with_facts(t, [&t.facts[..i], &[look], &t.facts[i + 1..]].concat())),
            (
                "identified by the speckle after a removal",
                with_facts(t, [&t.facts[..n - 1], &[late], &t.facts[n - 1..]].concat()),
            ),
        ],
    );
    let mut without = t.facts.clone();
    without.remove(i);
    want(
        "C4",
        &[
            ("not sure after its speckle fact", with_stone(t, s, |x| x.sure = false)),
            ("an unsure fact after its speckle fact", with_facts(t, insert(&[unsure]))),
            ("sure with no speckle fact", with_facts(t, without)),
        ],
    );
    let broken = [
        ("a look's bar", TrayFact { bar: Some(750), score: Some(760), ..same.clone() }),
        ("below its bar", TrayFact { score: Some(SPECKLE_CLEAN - 1), ..same.clone() }),
        ("certain", TrayFact { certain: Some(true), ..same.clone() }),
        ("no score", TrayFact { score: None, ..same.clone() }),
        ("no bar", TrayFact { bar: None, ..same.clone() }),
        ("fine: a speckle fact has no margin rule, unlike a look's", TrayFact { margin: Some(10), ..same }),
    ];
    let mut checks: Vec<(&str, Vec<String>)> = broken
        .into_iter()
        .map(|(name, x)| {
            let mut facts = t.facts.clone();
            facts[i] = x;
            (name, with_facts(t, facts))
        })
        .collect();
    assert_eq!(checks.pop().unwrap().1, Vec::<String>::new());
    want("C9", &checks);
}
