//! Tray reports (spec 4.7) beyond the golden files: the live stream, the trays that have none,
//! the text without a clock, each change's final place and confidence, the tray's own shift and a
//! tray where nothing it arrived with is left.

mod common;

use futures::StreamExt;
use pdigger_factory_sdk::testing::FakeFactory;
use pdigger_factory_sdk::{
    TimeRef, Tray, TrayChange, TrayChangeKind, TrayEvidence, TrayFact, TrayFactKind, TrayReport, TrayStone,
    TrayStoneStatus,
};
use serde_json::json;

use common::{fixtures, recording_fake};

fn golden(name: &str) -> String {
    std::fs::read_to_string(fixtures().join("golden").join(format!("{name}.reports.txt"))).unwrap()
}

fn at(t: f64, wall: Option<f64>) -> TimeRef {
    TimeRef { g: (t * 5.0).round() as i64, t, wall }
}

fn fact(kind: TrayFactKind, t: f64) -> TrayFact {
    serde_json::from_value(json!({"kind": kind.to_string(), "tray": 1, "session": 7, "video": "v", "stone": null,
        "world_id": null, "certain": null, "why": null, "g": 10, "t": t, "wall": null, "stones": null, "sure": null,
        "added": null, "removed": null, "unclear": null}))
    .unwrap()
}

fn stone(id: &str, place: [f64; 2], first: [f64; 2], sure: bool) -> TrayStone {
    serde_json::from_value(json!({"id": id, "world_id": 1, "status": "on", "sure": sure, "added": false,
        "certain": null, "born": {"g": 5, "t": 1.0, "wall": null}, "place": place, "size": [0.4, 0.2],
        "first_place": first, "left_place": place})) // seen at the leaving point; nothing touched it
    .unwrap()
}

fn touched(mut s: TrayStone, g: i64) -> TrayStone {
    s.touched = Some(g);
    s
}

fn tray(
    n: i64,
    arrived: Option<TimeRef>,
    leaving: Option<TimeRef>,
    stones: Vec<TrayStone>,
    facts: Vec<TrayFact>,
) -> Tray {
    Tray { session: 7, video: "v".into(), n, arrived, leaving, stones, facts, open: None, t_id: None }
}

#[tokio::test(start_paused = true)]
async fn the_interpreter_gives_a_report_as_each_tray_leaves() {
    for name in ["trays-synthetic", "dry1-0018-0153"] {
        let fake = recording_fake(name);
        let interpreter = fake.client().interpreter().resume(false);
        let reports: Vec<TrayReport> = interpreter.tray_reports().map(Result::unwrap).collect().await;
        let text: String = reports.iter().map(|r| r.describe() + "\n").collect();
        assert_eq!(text, golden(name), "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn an_error_of_the_stream_comes_through_the_reports() {
    let fake = FakeFactory::from_values(vec![]);
    fake.inject("streamFrames", "frame", json!({"g": "x"}), usize::MAX);
    let interpreter = fake.client().interpreter().resume(false).backfill(false);
    let items: Vec<_> = interpreter.tray_reports().collect().await;
    assert!(!items.is_empty() && items.iter().all(Result::is_err), "only the error: no tray left");
}

#[test]
fn a_tray_that_never_arrived_is_on_or_does_not_end_in_its_leaving_has_no_report() {
    assert_eq!(tray(1, None, None, vec![], vec![]).report(), None);
    assert_eq!(tray(1, Some(at(1.0, None)), None, vec![], vec![]).report(), None);
    assert_eq!(tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![], vec![]).report(), None);
    let only_arrived = vec![fact(TrayFactKind::Arrived, 1.0)];
    assert_eq!(tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![], only_arrived).report(), None);
}

#[test]
fn a_tray_without_a_clock_and_with_nothing_it_arrived_with_left() {
    let mut gone = stone("s1t1.0-1-4", [0.0, 0.0], [0.0, 0.0], true);
    gone.status = TrayStoneStatus::Removed;
    gone.certain = Some(true);
    let mut removed = fact(TrayFactKind::Removed, 4.0);
    removed.stone = Some(gone.id.clone());
    removed.certain = Some(true);
    removed.code = Some("carried_away".into());
    let mut other = fact(TrayFactKind::Removed, 5.0);
    other.stone = Some("not-a-tray-stone".into()); // a fact of no stone of the tray is left out
    let mut leaving = fact(TrayFactKind::Leaving, 9.0);
    (leaving.stones, leaving.sure) = (Some(0), Some(0)); // no unclear count
    let facts = vec![fact(TrayFactKind::Arrived, 1.0), removed, other, leaving.clone()]; // no stone count on the arrival
    let r = tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![gone.clone()], facts.clone()).report().unwrap();
    assert_eq!((r.stones, r.unclear, r.unsure, r.complete, r.moved.len()), (0, 0, 0, true, 0));
    assert_eq!(
        r.describe(),
        "tray 1, 0:01.0 to 0:09.0: 1 removed, 0 added, 0 moved\n  \
         0:04.0 removed s1t1.0-1-4 from (0.00, 0.00) (0.50)"
    );
    assert_eq!(r.removed[0].code.as_deref(), Some("carried_away"));
    // a removed stone's final place is where it was last seen, whatever its place at the leaving point
    (gone.place, gone.left_place) = ([0.68, -1.04], Some([3.0, 3.0]));
    let r = tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![gone], facts).report().unwrap();
    assert_eq!(r.removed[0].describe(), "0:04.0 removed s1t1.0-1-4 from (0.68, -1.04) (0.50)");
    // an addition that is not certain: counted unclear by the leaving fact, "unclear" in the text; it is not
    // counted as not sure
    let mut set = stone("s1t1.0-1-5", [1.0, 1.0], [1.0, 1.0], false);
    (set.added, set.certain) = (true, Some(false));
    let mut added = fact(TrayFactKind::Added, 6.0);
    (added.stone, added.certain, added.code) = (Some(set.id.clone()), Some(false), Some("not_recognised".into()));
    (leaving.stones, leaving.unclear) = (Some(1), Some(1));
    let facts = vec![fact(TrayFactKind::Arrived, 1.0), added, leaving];
    let report = |set: &TrayStone| {
        tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![set.clone()], facts.clone()).report().unwrap()
    };
    let r = report(&set);
    assert_eq!(
        r.describe(),
        "tray 1, 0:01.0 to 0:09.0: 0 removed, 1 added, 0 moved, not complete: 1 unclear\n  \
         0:06.0 added s1t1.0-1-5 at (1.00, 1.00) (unclear, 0.52)"
    );
    assert_eq!(r.unsure, 0);
    // its final place is its place at the leaving point; not seen there, where it was last seen
    (set.place, set.left_place) = ([2.0, -0.001], Some([1.5, -0.004]));
    let r = report(&set);
    assert_eq!(r.added[0].describe(), "0:06.0 added s1t1.0-1-5 at (1.50, 0.00) (unclear, 0.52)");
    assert_eq!(r.added[0].place, [1.5, -0.004]);
    set.left_place = None;
    assert_eq!(report(&set).added[0].describe(), "0:06.0 added s1t1.0-1-5 at (2.00, 0.00) (unclear, 0.52)");
}

#[test]
fn the_trays_own_shift_is_taken_out_before_a_stone_is_called_moved() {
    // stones the tray carried 0.1 along y as it left, one of them pushed 0.3 along x besides; d is not
    // sure, so it is counted as not sure and not as moved however far its place is
    let kept = vec![
        stone("a", [0.0, 0.1], [0.0, 0.0], true),
        stone("b", [1.0, 0.1], [1.0, 0.0], true),
        touched(stone("c", [2.3, 0.1], [2.0, 0.0], true), 40),
        touched(stone("d", [3.9, 0.1], [3.0, 0.0], false), 41),
        stone("e", [4.0, 0.1], [4.0, 0.0], true),
    ];
    let mut arrived = fact(TrayFactKind::Arrived, 2.0);
    (arrived.stones, arrived.wall) = (Some(5), Some(1_794_960_000.25));
    let mut leaving = fact(TrayFactKind::Leaving, 9.0);
    (leaving.stones, leaving.sure, leaving.unclear) = (Some(5), Some(4), Some(0));
    let facts = vec![arrived, leaving];
    let report = |stones: Vec<TrayStone>| {
        tray(2, Some(at(1.0, None)), Some(at(9.0, None)), stones, facts.clone()).report().unwrap()
    };
    let names = |r: &TrayReport| r.moved.iter().map(|c| c.stone.clone()).collect::<Vec<_>>();
    let r = report(kept.clone());
    let moved: Vec<_> = r.moved.iter().map(|c| (c.stone.as_str(), c.distance, c.certain, c.place)).collect();
    assert_eq!(moved, [("c", Some(0.3), true, [2.3, 0.1])]);
    assert_eq!((r.unsure, r.complete), (1, false));
    assert_eq!(
        r.describe(),
        "tray 2 (2026-11-18T00:00:00.3Z), 0:02.0 to 0:09.0: 0 removed, 0 added, 1 moved, not complete: 1 not sure\n  \
         moved c 0.30 to (2.30, 0.10) (0.87)"
    );
    // the median of an odd half: one of two sure stones, three of five
    assert_eq!(names(&report(vec![kept[0].clone(), kept[2].clone()])), ["c"]);
    let mut five = kept[..3].to_vec();
    five.push(stone("f", [5.0, 0.1], [5.0, 0.0], true));
    five.push(stone("g", [6.0, 0.1], [6.0, 0.0], true));
    assert_eq!(names(&report(five)), ["c"]);
    // nothing sure: nothing moved
    assert!(report(vec![kept[3].clone()]).moved.is_empty());
    // the same place, but nothing touched it: an outline the world refined is not a move
    let still = stone("c", [2.3, 0.1], [2.0, 0.0], true);
    assert!(report(vec![kept[0].clone(), kept[1].clone(), still]).moved.is_empty());
    // not seen at the leaving point: its place there is not known, and it is left out of the shift as well
    let mut unseen = kept[2].clone();
    unseen.left_place = None;
    assert!(report(vec![kept[0].clone(), kept[1].clone(), unseen.clone()]).moved.is_empty());
    let v = serde_json::to_value(&unseen).unwrap();
    assert_eq!((&v["left_place"], &v["touched"]), (&json!(null), &json!(40)));
    assert_eq!(pdigger_factory_sdk::interpreter::MOVED, 0.75);
}

#[test]
fn a_change_without_its_point_or_distance() {
    let change = |kind, certain| TrayChange {
        kind,
        stone: "x".into(),
        at: None,
        certain,
        code: None,
        place: [0.0, 0.0],
        size: [1.0, 1.0],
        distance: None,
        confidence: 0.5,
    };
    assert_eq!(change(TrayChangeKind::Removed, false).describe(), "removed x from (0.00, 0.00) (unclear, 0.50)");
    assert_eq!(change(TrayChangeKind::Moved, true).describe(), "moved x 0.00 to (0.00, 0.00) (0.50)");
    assert_eq!(change(TrayChangeKind::Added, true).describe(), "added x at (0.00, 0.00) (0.50)");
    // a coordinate that rounds to zero prints without its sign
    let far = TrayChange { place: [-1.236, -0.0049], confidence: 0.984, ..change(TrayChangeKind::Removed, true) };
    assert_eq!(far.describe(), "removed x from (-1.24, 0.00) (0.98)");
    assert_eq!(pdigger_factory_sdk::interpreter::iso(1_794_960_000.04), "2026-11-18T00:00:00.0Z");
}

#[test]
fn a_changes_confidence_is_that_of_its_kind_its_certainty_and_its_stones_evidence() {
    // the measured constants (spec 4.7): removals and additions by certainty and evidence; a move 0.87
    use pdigger_factory_sdk::interpreter::{CONFIDENCE, MOVED_CONFIDENCE};
    assert_eq!((CONFIDENCE.len(), MOVED_CONFIDENCE), (12, 0.87));
    let removal = |certain: bool, evidence: serde_json::Value| {
        let mut s = serde_json::to_value(stone("s", [0.0, 0.0], [0.0, 0.0], true)).unwrap();
        (s["status"], s["certain"], s["evidence"]) = (json!("removed"), json!(certain), evidence);
        let mut removed = fact(TrayFactKind::Removed, 4.0);
        (removed.stone, removed.certain) = (Some("s".into()), Some(certain));
        let mut leaving = fact(TrayFactKind::Leaving, 9.0);
        (leaving.stones, leaving.sure, leaving.unclear) = (Some(0), Some(0), Some(u64::from(!certain)));
        let facts = vec![fact(TrayFactKind::Arrived, 1.0), removed, leaving];
        let s: TrayStone = serde_json::from_value(s).unwrap();
        let r = tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![s], facts).report().unwrap();
        r.removed[0].confidence
    };
    for (certain, evidence, want) in [
        (true, json!("not_found"), 0.98),
        (true, json!("undecided"), 0.50),
        (true, json!("found"), 0.25), // the leaving's answer found the stone it calls taken off
        (false, json!("not_found"), 0.71),
        (false, json!("undecided"), 0.58),
        (false, json!("found"), 0.09),
        (true, json!(null), 0.50),    // no evidence (a stone read without it): undecided
        (true, json!("maybe"), 0.50), // a value this SDK does not know: undecided
    ] {
        assert_eq!(removal(certain, evidence.clone()), want, "{certain} {evidence}");
    }
    for (kind, certain, evidence, want) in [
        (TrayChangeKind::Added, true, TrayEvidence::NotFound, 0.96),
        (TrayChangeKind::Added, false, TrayEvidence::Undecided, 0.52),
        (TrayChangeKind::Added, false, TrayEvidence::Found, 0.02),
    ] {
        let row = CONFIDENCE.iter().find(|(k, c, e, _)| (k, *c, e) == (&kind, certain, &evidence)).unwrap();
        assert_eq!(row.3, want, "{kind} {certain} {evidence}");
    }
    // the JSON of a change holds it
    let mut set = stone("a", [1.0, 1.0], [1.0, 1.0], false);
    (set.added, set.certain, set.evidence) = (true, Some(false), Some(TrayEvidence::Found));
    let mut added = fact(TrayFactKind::Added, 6.0);
    (added.stone, added.certain) = (Some("a".into()), Some(false));
    let mut leaving = fact(TrayFactKind::Leaving, 9.0);
    (leaving.stones, leaving.unclear) = (Some(1), Some(1));
    let facts = vec![fact(TrayFactKind::Arrived, 1.0), added, leaving];
    let r = tray(1, Some(at(1.0, None)), Some(at(9.0, None)), vec![set], facts).report().unwrap();
    assert_eq!(r.added[0].describe(), "0:06.0 added a at (1.00, 1.00) (unclear, 0.02)");
    assert_eq!(serde_json::to_value(&r.added[0]).unwrap()["confidence"], json!(0.02));
}
