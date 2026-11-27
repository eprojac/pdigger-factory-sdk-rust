//! Trays and tray stones (spec 4.6) beyond the golden files: the values, ids without a clock, the
//! live streams, the state, the events' tray stones and the trays the core keeps. The rules
//! themselves are golden-tested over every recording, `trays-synthetic` among them (the rules the
//! real excerpts never reach).

mod common;

use futures::StreamExt;
use pdigger_factory_sdk::models::{ClockEvent, EventKind, WorldEvent};
use pdigger_factory_sdk::replay::Line;
use pdigger_factory_sdk::{Frame, InterpreterCore, Replay, TimeRef, TrayFact, TrayFactKind, TrayStoneStatus, Update};
use serde_json::json;

use common::{fixtures, recording, recording_fake};

const SYNTH: &str = "trays-synthetic";

fn golden() -> String {
    std::fs::read_to_string(fixtures().join("golden").join(format!("{SYNTH}.trays.txt"))).unwrap()
}

fn text<'a>(facts: impl IntoIterator<Item = &'a TrayFact>) -> String {
    facts.into_iter().map(|f| f.describe() + "\n").collect()
}

/// Feeds `core` the synthetic recording (its clock line too when `clocked`), calling `each` with
/// every update.
fn feed(core: &mut InterpreterCore, clocked: bool, mut each: impl FnMut(&InterpreterCore, &Update)) {
    let replay = Replay::open(recording(SYNTH)).unwrap();
    for line in replay.lines() {
        match line.unwrap() {
            Line::Frame(frame) => {
                let update = core.feed(*frame);
                each(core, &update);
            }
            Line::Side(side) if clocked && side.sse == "clock" => {
                core.set_clock(serde_json::from_value::<ClockEvent>(side.data).unwrap().state);
            }
            _ => {}
        }
    }
}

fn numbers(trays: &[pdigger_factory_sdk::Tray]) -> Vec<i64> {
    trays.iter().map(|t| t.n).collect()
}

#[test]
fn a_tray_as_values() {
    let trays = Replay::open(recording(SYNTH)).unwrap().trays().unwrap();
    assert_eq!(numbers(&trays), [1, 2, 3, 4, 5, 6, 7, 8]);
    let first = &trays[0];
    assert_eq!(first.window(), Some((0.8, 5.2)));
    let never = &trays[3]; // a stop that never settled
    assert_eq!((never.window(), never.arrived, never.leaving, never.stones.len()), (None, None, None, 0));
    let d = serde_json::to_value(first).unwrap();
    assert_eq!((&d["session"], &d["video"], &d["n"]), (&json!(1_794_684_199), &json!("dry1"), &json!(1)));
    assert_eq!(d["arrived"], json!({"g": 4, "t": 0.8, "wall": 1_794_960_000.8}));
    assert_eq!(d["leaving"]["g"], json!(26));
    assert_eq!(
        d["stones"][1],
        json!({"id": "20261118T000000.8Z-1-2", "world_id": null, "status": "removed", "sure": true, "added": false,
               "certain": true, "born": {"g": 4, "t": 0.8, "wall": 1_794_960_000.8}, "place": [0.0, 0.0],
               "size": [0.3, 0.2], "question": null, "looks": [], "by_look": false, "look_lost": false,
               "open": null, "first_place": [0.0, 0.0],
               // removed: not on the tray when it left; touched: the frame a hand last touched it (it carried it away);
               // no speckle answer for it (spec 4.6.12), and none for the tray's leaving (4.6.13)
               "left_place": null, "touched": 13, "speckle": null, "by_speckle": false, "evidence": "undecided"})
    );
    // seen in the leaving frame, never touched
    assert_eq!((&d["stones"][0]["left_place"], &d["stones"][0]["touched"]), (&json!([-0.6, 0.0]), &json!(null)));
    assert_eq!(
        d["facts"][1],
        json!({"kind": "removed", "tray": 1, "session": 1_794_684_199, "video": "dry1",
               "stone": "20261118T000000.8Z-1-2", "world_id": null, "certain": true, "why": "carried away",
               "code": "carried_away", "g": 22, "t": 4.4, "wall": 1_794_960_004.4, "stones": null, "sure": null,
               "added": null, "removed": null, "unclear": null, "opened": null, "from_world": null, "score": null,
               "margin": null, "bar": null, "t_id": null, "open": null})
    );
    let added = first.stone("20261118T000003.8Z-1-7").unwrap();
    assert_eq!((added.added, added.certain, added.status.clone()), (true, Some(true), TrayStoneStatus::On));
    assert!(first.stone("no such stone").is_none());
    let leaving = first.facts.last().unwrap();
    assert_eq!(leaving.kind, TrayFactKind::Leaving);
    assert_eq!(leaving.clock(), "0:05.2");
    assert_eq!(
        (leaving.stones, leaving.sure, leaving.added, leaving.removed, leaving.unclear),
        (Some(3), Some(3), Some(1), Some(1), Some(0))
    );
    assert_eq!(leaving.to_string(), leaving.describe());
    assert_eq!((&d["open"], &d["t_id"], &leaving.open, leaving.t_id), (&json!([]), &json!(null), &Some(vec![]), None));
}

/// Tray 5: slab 47 is set down for certain, then gone after the last calm moment, so the tray's
/// leaving finds it removed unclear; the leaving still counts its certain addition, as its facts
/// and the summary do.
#[test]
fn a_certain_addition_counts_when_the_stone_is_removed_unclear() {
    let five = &Replay::open(recording(SYNTH)).unwrap().trays().unwrap()[4];
    let slab = five.stone("20261118T000025.8Z-5-47").unwrap();
    assert_eq!((slab.added, slab.status.clone(), slab.certain), (true, TrayStoneStatus::Removed, Some(false)));
    let said: Vec<_> = five
        .facts
        .iter()
        .filter(|x| x.stone.as_deref() == Some(slab.id.as_str()))
        .map(|x| (x.kind.clone(), x.certain))
        .collect();
    assert_eq!(said, [(TrayFactKind::Added, Some(true)), (TrayFactKind::Removed, Some(false))]);
    let leaving = five.facts.last().unwrap();
    assert_eq!(
        (leaving.stones, leaving.sure, leaving.added, leaving.removed, leaving.unclear),
        (Some(2), Some(2), Some(1), Some(0), Some(1))
    );
    assert!(five.check().is_empty());
    // the self-check counts the leaving's additions, removals and unclear from the facts: each one off is C5
    let off = |change: fn(&mut pdigger_factory_sdk::TrayFact)| {
        let mut broken = five.clone();
        change(broken.facts.last_mut().unwrap());
        broken.check()
    };
    let offs = [off(|x| x.added = Some(0)), off(|x| x.removed = Some(1)), off(|x| x.unclear = Some(0))];
    assert_eq!(offs, [["C5"], ["C5"], ["C5"]]);
    let summary = Replay::open(recording(SYNTH)).unwrap().summary().unwrap();
    assert_eq!(summary.trays.added, 2); // tray 1's slab and this one
}

/// The world renames stone 12 to 17: the tray stone keeps its id and stands for 17 (still sure).
#[test]
fn a_relinked_stone_keeps_its_id() {
    let tray = &Replay::open(recording(SYNTH)).unwrap().trays().unwrap()[1];
    let stone = tray.stone("20261118T000006.4Z-2-12").unwrap();
    assert_eq!((stone.world_id, stone.status.clone(), stone.sure), (Some(17), TrayStoneStatus::On, true));
    let found = tray.facts.iter().find(|f| f.why.as_deref() == Some("found again, not for sure")).unwrap();
    assert_eq!((found.stone.as_deref(), found.world_id), (Some("20261118T000006.4Z-2-13"), Some(18)));
}

/// Without a clock state the stamp is the session and the media time: ids stay unique after a
/// seek back and in another session.
/// Tray 2 with a twin of stone 12 (14: the same size, 0.1 beside it) that the world deletes with 12 at frame 58,
/// where it renames 12 to 17. 17 fits both lost stones, so while both are lost it stands for neither: a rename
/// needs the one lost stone that fits. As the tray leaves, 14 is not found (the Python SDK's
/// test_a_new_stone_that_fits_two_lost_stones_stands_for_neither; the reference gives the same tray facts).
#[test]
fn a_new_stone_that_fits_two_lost_stones_stands_for_neither() {
    let mut core = InterpreterCore::new();
    let mut while_lost = Vec::new();
    for line in Replay::open(recording(SYNTH)).unwrap().lines() {
        match line.unwrap() {
            Line::Frame(mut frame) => {
                if let Some(world) = frame.world.as_mut() {
                    let twins: Vec<_> = world
                        .stones
                        .iter()
                        .filter(|s| s.id == 12)
                        .map(|s| {
                            let mut twin = s.clone();
                            twin.id = 14;
                            if let Some(solid) = twin.solid.as_mut() {
                                solid.pos[0] += 0.1;
                            }
                            twin
                        })
                        .collect();
                    world.stones.extend(twins);
                    let deletes: Vec<WorldEvent> = world
                        .events
                        .iter()
                        .filter(|e| e.kind == EventKind::Delete && e.id == Some(12))
                        .map(|e| WorldEvent { id: Some(14), ..e.clone() })
                        .collect();
                    world.events.extend(deletes);
                }
                let g = frame.g;
                core.feed(*frame);
                if (58..=67).contains(&g) {
                    // the rename to the tray's last frame (0:13.4), and the revision of 58
                    let two = core.trays().into_iter().find(|t| t.n == 2).unwrap();
                    while_lost.push([12, 14].map(|n| {
                        two.stone(&format!("20261118T000006.4Z-2-{n}")).map(|s| (s.world_id, s.status.clone()))
                    }));
                }
            }
            Line::Side(side) if side.sse == "clock" => {
                core.set_clock(serde_json::from_value::<ClockEvent>(side.data).unwrap().state);
            }
            _ => {}
        }
    }
    core.end();
    let lost = Some((None, TrayStoneStatus::Lost));
    assert_eq!(while_lost, vec![[lost.clone(), lost]; 11]);
    let tray = core.trays().into_iter().find(|t| t.n == 2).unwrap();
    let again: Vec<_> = tray.facts.iter().filter(|f| f.code.as_deref() == Some("found_again")).collect();
    assert_eq!(again.iter().map(|f| f.stone.as_deref()).collect::<Vec<_>>(), [Some("20261118T000006.4Z-2-13")]);
    assert_eq!(
        text(&tray.facts[tray.facts.len() - 2..]),
        "0:13.4 tray 2: stone 20261118T000006.4Z-2-14 not found (unclear)\n\
         0:13.4 tray 2 leaving: 5 stones (0 sure), 0 added, 0 removed, 1 unclear\n"
    );
    assert_eq!(tray.check(), Vec::<String>::new());
}

#[test]
fn ids_without_a_clock_name_the_session() {
    let mut core = InterpreterCore::new(); // no clock fed
    let mut facts = Vec::new();
    feed(&mut core, false, |_, u| facts.extend(u.tray_facts.clone()));
    facts.extend(core.end().tray_facts);
    let text = text(&facts);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "0:04.4 tray 1: stone s1794684199t0.8-1-2 was removed (carried away)");
    assert_eq!(lines[2], "0:05.2 tray 1: stone s1794684199t3.8-1-7 was set down");
    assert!(facts.iter().all(|f| f.wall.is_none()));
}

#[test]
fn events_name_their_tray_stone_and_the_state_its_tray() {
    let mut core = InterpreterCore::new();
    let mut seen = std::collections::BTreeMap::new();
    let mut trays = std::collections::BTreeSet::new();
    feed(&mut core, true, |_, u| {
        for e in &u.events {
            seen.insert(format!("{} {}", e.kind, e.stone_id.unwrap_or(-1)), e.tray_stone.clone());
        }
        if let Some(tray) = &u.state.tray {
            trays.insert(tray.n);
            assert!(tray.leaving.is_none(), "the open tray");
        }
    });
    assert_eq!(seen["gone 2"].as_deref(), Some("20261118T000000.8Z-1-2")); // it stood for 2 before this frame
    assert_eq!(seen["new 7"], None); // a stone not on the tray yet
    assert_eq!(seen["lift 13"].as_deref(), Some("20261118T000006.4Z-2-13"));
    assert_eq!(seen["place 18"].as_deref(), Some("20261118T000006.4Z-2-13")); // renamed: the same tray stone
    // revisions: the tray stone before the revised frame, not the links of the newest one
    assert_eq!(seen["confirm 2"].as_deref(), Some("20261118T000000.8Z-1-2")); // frame 13: 2 still stood for it
    assert_eq!(seen["confirm 7"], None); // frame 26, still the newest, set 7 down: none before it
    assert_eq!(seen["confirm 17"], None); // frame 58, whose rename linked 17
    assert_eq!(trays.into_iter().collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6, 7, 8]);
    core.end();
    assert!(core.state().tray.is_none());
}

#[test]
fn the_core_keeps_the_newest_trays() {
    let mut core = InterpreterCore::new();
    core.keep_trays(2);
    let mut last = Vec::new();
    feed(&mut core, true, |core, u| {
        if u.frame.as_ref().is_some_and(|f: &Frame| f.g == 80) {
            assert_eq!(numbers(&core.trays()), [1, 2, 3]); // tray 3 is on
        }
        last = numbers(&core.trays());
    });
    assert_eq!(last, [6, 7, 8]); // tray 8 is on, after the 2 newest that have left
    core.end();
    assert_eq!(numbers(&core.trays()), [7, 8]);
    let mut one = InterpreterCore::new();
    one.keep_trays(1);
    feed(&mut one, true, |_, _| {});
    one.end();
    assert_eq!(numbers(&one.trays()), [8]);
    let mut all = InterpreterCore::new();
    all.keep_trays(0);
    feed(&mut all, true, |_, _| {});
    assert_eq!(numbers(&all.trays()), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(all.state().tray.map(|t| t.n), Some(8)); // tray 8 is still on
}

#[test]
fn finish_leaves_the_tray_facts_in_trays() {
    let mut core = InterpreterCore::new();
    feed(&mut core, true, |_, _| {});
    assert!(core.finish().is_empty()); // no activity was open
    let trays = core.trays();
    assert_eq!(text(trays.iter().flat_map(|t| &t.facts)), golden());
}

#[tokio::test(start_paused = true)]
async fn the_interpreter_streams_tray_facts() {
    let fake = recording_fake(SYNTH);
    let interpreter = fake.client().interpreter().resume(false);
    let facts: Vec<TrayFact> = interpreter.tray_facts().map(Result::unwrap).collect().await;
    assert_eq!(text(&facts), golden()); // the stream's end makes tray 8 leave, as a replay's does
    assert_eq!(numbers(&interpreter.trays()), [1, 2, 3, 4, 5, 6, 7, 8]);
    let last = interpreter.end();
    assert!(last.is_final && last.tray_facts.is_empty(), "the stream's end made the last tray leave");
}

/// Three facts come with one frame (0:08.6); a loop that stops after the first leaves the other
/// two to the next.
#[tokio::test(start_paused = true)]
async fn a_tray_facts_loop_stopped_inside_a_frame_leaves_the_rest() {
    let fake = recording_fake(SYNTH);
    let interpreter = fake.client().interpreter().resume(false);
    let mut facts = interpreter.tray_facts();
    while let Some(fact) = facts.next().await {
        if fact.unwrap().clock() == "0:08.6" {
            break;
        }
    }
    drop(facts);
    let rest: Vec<TrayFact> = interpreter.tray_facts().map(Result::unwrap).collect().await;
    assert_eq!(
        rest[0].describe(),
        "0:08.6 tray 2: stone 20261118T000006.4Z-2-15 is not sure any more (its box overlaps another stone)"
    );
    let golden = golden();
    assert!(golden.ends_with(&text(&rest)));
}

#[test]
fn a_summary_without_trays_reads_as_zeros() {
    let summary = Replay::open(recording(SYNTH)).unwrap().summary().unwrap();
    let mut json = summary.to_json();
    assert_eq!(
        json["trays"],
        json!({"total": 8, "arrived": 7, "stones": 33, "added": 2, "added_unclear": 1, "removed": 1,
               "removed_unclear": 2, "unsure": 5})
    );
    json.as_object_mut().unwrap().remove("trays"); // a summary published before trays were counted
    let old: pdigger_factory_sdk::Summary = serde_json::from_value(json).unwrap();
    assert_eq!(old.trays, Default::default());
    let at = TimeRef { g: 1, t: 1.0, wall: None };
    assert_eq!(at, TimeRef { g: 1, t: 1.0, wall: None }); // a tray point is a TimeRef
}

#[test]
fn sheet_actions_and_revisions() {
    let replay = Replay::open(recording("trays-sheet-synthetic")).unwrap().run().unwrap();
    let mut event = replay.events.iter().find(|e| e.kind == EventKind::Sheet).unwrap().clone();
    event.ids = Some(vec![1, 2, 3, 4, 5, 6, 7]);
    event.why = Some("laid".into());
    assert_eq!(event.sentence(), "a sheet is laid over stones 1, 2, 3, 4, 5, 6 and 1 more");
    event.why = Some("rolled".into());
    assert_eq!(event.sentence(), "a sheet event (rolled)");
    event.why = None;
    assert_eq!(event.sentence(), "a sheet event");
    event.why = Some("lifted".into());
    event.ids = None;
    assert_eq!(event.sentence(), "a sheet is lifted");
    let first = &replay.trays[0];
    let text = serde_json::to_value(first.report().unwrap()).unwrap();
    assert!(text["sheets"].as_u64().unwrap() > 0);
    assert_eq!(text["sheet_facts"][0]["kind"], "sheet");
    assert!(text["sheet_facts"][0]["sure"].is_null());
    assert!(!text["under_sheet"].as_array().unwrap().is_empty());
    for case in ["size", "id", "duplicate", "unknown action", "early lift", "departure"] {
        let mut tr = first.clone();
        match case {
            "size" => tr.facts[1].stones = Some(99),
            "id" => tr.facts[1].tray_stones = vec!["unknown".into()],
            "duplicate" => tr.facts.insert(1, tr.facts[1].clone()),
            "unknown action" => tr.facts[1].code = Some("rolled".into()),
            "early lift" => tr.facts[1].code = Some("sheet_lifted".into()),
            _ => tr.facts.last_mut().unwrap().under_sheet = None,
        }
        assert!(tr.check().iter().any(|c| c == if case == "departure" { "C11" } else { "C10" }));
    }
}

#[test]
fn sheet_facts_format_the_names_and_preserve_the_ordinary_report_format() {
    let paper = Replay::open(recording("trays-sheet-synthetic")).unwrap().trays().unwrap();
    let ordinary = Replay::open(recording("trays-sheet-synthetic-nosheet")).unwrap().trays().unwrap();
    let mut fact = paper[0].facts[1].clone();
    for (n, ending) in
        [(0, "stone ?"), (1, "stone s1"), (2, "stones s1 and s2"), (7, "stones s1, s2, s3, s4, s5, s6 and 1 more")]
    {
        fact.tray_stones = (1..=n).map(|i| format!("s{i}")).collect();
        assert!(fact.describe().ends_with(ending));
    }
    for (tray, has_paper) in [(&paper[0], true), (&ordinary[0], false)] {
        let report = tray.report().unwrap();
        let doc = serde_json::to_value(&report).unwrap();
        let bytes = serde_json::to_vec(&report).unwrap();
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(), doc);
        for key in ["sheets", "sheet_facts", "under_sheet"] {
            assert_eq!(doc.get(key).is_some(), has_paper);
        }
        let whole = serde_json::to_value(tray).unwrap();
        assert_eq!(whole["stones"].as_array().unwrap().len(), tray.stones.len());
    }
}

#[test]
fn a_partial_sheet_lift_can_leave_an_unresolved_stone_on_the_tray() {
    let trays = Replay::open(recording("trays-sheet-partial-synthetic")).unwrap().trays().unwrap();
    let tr = &trays[0];
    assert!(tr.check().is_empty());
    let unresolved = tr.stones.iter().find(|s| s.world_id.is_none()).unwrap();
    let open = unresolved.open.as_ref().unwrap();
    assert!(open.world_id.is_none());
    assert_eq!(open.candidates[0].kind, "unseen");
    let mut invalid = tr.clone();
    invalid.stones.iter_mut().find(|s| s.id == unresolved.id).unwrap().under_sheet = true;
    assert!(invalid.check().iter().any(|c| c == "C6"));
}

#[test]
fn covered_stones_must_stay_in_place_to_establish_a_partial_lift() {
    for absent in [false, true] {
        let mut core = InterpreterCore::new();
        for line in Replay::open(recording("trays-sheet-partial-synthetic")).unwrap().lines() {
            let Line::Frame(mut f) = line.unwrap() else { continue };
            if f.g > 26 {
                break;
            }
            if f.g == 20 {
                let w = f.world.as_mut().unwrap();
                if absent {
                    w.stones.retain(|s| !matches!(s.id, 3 | 4));
                } else {
                    for s in w.stones.iter_mut().filter(|s| matches!(s.id, 3 | 4)) {
                        s.solid.as_mut().unwrap().pos[0] += 0.4;
                    }
                }
            }
            core.feed(*f);
        }
        core.end();
        assert_eq!(core.trays()[0].leaving.unwrap().g, 20);
    }
}

#[test]
fn visible_stones_preserve_a_partial_bounded_board() {
    use pdigger_factory_sdk::models::StoneMode;
    let mut c = InterpreterCore::new();
    assert!(c.set_tray_bounds(Some([0., 0., 0., 1.])).is_err());
    assert!(c.set_tray_bounds(Some([0., f64::NAN, 5., 1.])).is_err());
    assert!(c.set_tray_bounds(None).is_ok());
    for case in ["anchored", "missing", "moved", "gap", "unbounded"] {
        let mut core = InterpreterCore::new();
        if case != "unbounded" {
            core.set_tray_bounds(Some([0., 0., 1920., 1080.])).unwrap();
        }
        for line in Replay::open(recording("trays-sheet-partial-synthetic")).unwrap().lines() {
            let Line::Frame(mut f) = line.unwrap() else { continue };
            if f.g > 31 {
                break;
            }
            if f.g == 19 && case == "gap" {
                continue;
            }
            let w = f.world.as_mut().unwrap();
            for s in w.stones.iter_mut().filter(|s| matches!(s.id, 3 | 4)) {
                s.under_sheet = false;
                s.detected = true;
                s.mode = StoneMode::Seen;
                if f.g == 20 && s.id == 4 {
                    if case == "missing" {
                        s.detected = false;
                        s.mode = StoneMode::Kept;
                    }
                    if case == "moved" {
                        s.solid.as_mut().unwrap().pos[0] += 0.4;
                    }
                }
            }
            if f.g == 14 {
                w.events[0].ids = Some(vec![1, 2]);
            }
            core.feed(*f);
        }
        core.end();
        let g = core.trays()[0].leaving.unwrap().g;
        if case == "anchored" { assert!(g > 26) } else { assert_eq!(g, 20, "{case}") }
    }
}

#[test]
fn bounded_arrival_keeps_the_delete_and_reveals_cover() {
    use pdigger_factory_sdk::models::StoneMode;
    let mut core = InterpreterCore::new();
    core.set_tray_bounds(Some([0., 0., 100., 100.])).unwrap();
    let mut facts = Vec::new();
    let mut deleted = Vec::new();
    for line in Replay::open(recording(SYNTH)).unwrap().lines() {
        let Line::Frame(mut f) = line.unwrap() else { continue };
        let g = f.g;
        if g > 16 {
            break;
        }
        let w = f.world.as_mut().unwrap();
        w.stones.truncate(2);
        for s in &mut w.stones {
            s.outline = vec![vec![1., 1.], vec![9., 1.], vec![9., 9.]];
            s.mode = StoneMode::Seen;
            s.detected = true;
        }
        w.events.clear();
        if g < 4 {
            let mut old = w.stones[0].clone();
            old.id = 99;
            old.detected = false;
            old.mode = StoneMode::Kept;
            old.outline = vec![vec![110., 0.], vec![120., 0.], vec![120., 10.]];
            w.stones.push(old);
        }
        if g == 4 || g == 16 {
            let (id, why) = if g == 4 { (99, "stranded") } else { (9999, "sheet") };
            w.events.push(WorldEvent {
                kind: EventKind::Delete,
                id: Some(id),
                why: Some(why.into()),
                ..WorldEvent::default()
            });
        }
        if (11..14).contains(&g) {
            w.stones[0].under_sheet = true;
        }
        w.stats.stones = w.stones.len() as i64;
        w.stats.confirmed = w.stats.stones;
        let u = core.feed(*f);
        facts.extend(u.tray_facts);
        deleted.extend(u.events.iter().filter(|e| e.kind == EventKind::Delete).filter_map(|e| e.stone_id));
    }
    assert_eq!(deleted, [99, 9999]);
    assert_eq!(facts[0].g, 4);
    assert_eq!(facts[1].code.as_deref(), Some("sheet_laid"));
    assert_eq!(facts[2].code.as_deref(), Some("sheet_lifted"));
}
