//! The tray rules by look (spec 4.6.3-4.6.10) beyond the golden files: the self-check finds each
//! broken code on accounts made broken on purpose, and the open account and a live question as
//! values. The rules themselves are golden-tested over `trays-looks-synthetic` (32 trays, every
//! rule by look) and the real excerpts.

mod common;

use std::sync::OnceLock;

use pdigger_factory_sdk::replay::Line;
use pdigger_factory_sdk::{
    InterpreterCore, Replay, TimeRef, Tray, TrayCandidate, TrayFact, TrayFactKind, TrayOpen, TrayQuestion, TrayStone,
    TrayStoneStatus,
};
use serde_json::json;

use common::recording;

const LOOKS: &str = "trays-looks-synthetic";

fn left_trays() -> &'static [Tray] {
    static TRAYS: OnceLock<Vec<Tray>> = OnceLock::new();
    TRAYS.get_or_init(|| Replay::open(recording(LOOKS)).unwrap().trays().unwrap())
}

fn tray(n: i64) -> Tray {
    left_trays().iter().find(|t| t.n == n).unwrap().clone()
}

/// The index of the tray stone whose id ends with that world id (`...-13-301`).
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

/// The check of the tray with one open entry of its leaving fact changed.
fn with_entry(t: &Tray, stone: &str, change: impl FnOnce(&mut TrayOpen)) -> Vec<String> {
    let mut facts = t.facts.clone();
    let entries = facts.last_mut().unwrap().open.as_mut().unwrap();
    change(entries.iter_mut().find(|o| o.stone == stone).unwrap());
    with_facts(t, facts)
}

fn entry(t: &Tray, stone: &str) -> TrayOpen {
    t.open.as_ref().unwrap().iter().find(|o| o.stone == stone).unwrap().clone()
}

/// Every check found the code.
fn want(code: &str, checks: &[(&str, Vec<String>)]) {
    for (name, got) in checks {
        assert!(got.iter().any(|c| c == code), "{name}: {code} not found in {got:?}");
    }
}

/// 4.6.7: on each tray the same certain facts on the same frames, up to where the tray is relinked;
/// the copy of the recording without any look is v1.
#[test]
fn the_certain_facts_with_looks_are_the_first_of_v1s() {
    let certain = |trays: &[Tray]| -> Vec<(i64, Vec<String>)> {
        trays
            .iter()
            .map(|t| {
                let facts = t.facts.iter().filter(|f| f.certain == Some(true));
                (t.n, facts.map(|f| format!("{:?} {:?} {} {:?}", f.kind, f.stone, f.g, f.code)).collect())
            })
            .collect()
    };
    let bare = Replay::open(recording(&format!("{LOOKS}-nolooks"))).unwrap().trays().unwrap();
    let (looks, v1) = (certain(left_trays()), certain(&bare));
    assert_eq!(looks.iter().map(|x| x.0).collect::<Vec<_>>(), v1.iter().map(|x| x.0).collect::<Vec<_>>());
    let mut held = Vec::new();
    for ((n, mine), (_, theirs)) in looks.iter().zip(&v1) {
        assert!(mine.len() <= theirs.len() && mine[..] == theirs[..mine.len()], "tray {n}: {mine:?}, v1 {theirs:?}");
        if theirs.len() > mine.len() {
            held.push((*n, theirs.len() - mine.len()));
        }
    }
    // tray 2 a swap, 10 a stone linked by its look, 30 twins, 31 a look that changed (no hand took
    // 15's X: v1 leaves its loss unclear too)
    assert_eq!(held, [(2, 1), (10, 1), (30, 3), (31, 1)]);
}

#[test]
fn every_left_tray_checks_clean() {
    let trays = left_trays();
    assert_eq!(trays.iter().map(|t| t.n).collect::<Vec<_>>(), (1..=32).collect::<Vec<_>>());
    for t in trays {
        assert_eq!(t.check(), Vec::<String>::new(), "tray {}", t.n);
    }
}

#[test]
fn c1_the_facts_run_from_arrived_to_leaving() {
    let t = tray(13);
    let never = Tray {
        n: 99,
        arrived: None,
        leaving: None,
        stones: vec![],
        facts: vec![],
        open: None,
        t_id: None,
        ..t.clone()
    };
    assert_eq!(never.check(), Vec::<String>::new()); // a tray that never arrived passes
    let n = t.facts.len();
    let mut twice = t.facts.clone();
    twice.push(t.facts[n - 1].clone());
    want(
        "C1",
        &[
            ("not left yet", with_facts(&t, t.facts[..n - 1].to_vec())),
            ("no facts", with_facts(&t, vec![])),
            ("a fact after the leaving", with_facts(&t, twice)),
            ("no arrival first", with_facts(&t, t.facts[1..].to_vec())),
        ],
    );
}

#[test]
fn c2_ids_are_unique_and_every_fact_names_a_stone_of_the_tray() {
    let t = tray(13);
    let mut twice = t.clone();
    twice.stones.push(t.stones[0].clone());
    let mut facts = t.facts.clone();
    facts.iter_mut().find(|x| x.stone.is_some()).unwrap().stone = Some("nobody".into());
    want("C2", &[("a stone twice", twice.check()), ("no such stone", with_facts(&t, facts))]);
}

#[test]
fn c3_each_stones_facts_come_in_order() {
    let t = tray(13);
    let pushed = t.stones[stone_of(&t, 301)].id.clone(); // unsure (pushed) at the leaving
    let i = t.facts.iter().position(|x| x.kind == TrayFactKind::Unsure && x.stone.as_ref() == Some(&pushed)).unwrap();
    let after = |xs: &[TrayFact]| [&t.facts[..=i], xs, &t.facts[i + 1..]].concat();
    let unsure = t.facts[i].clone();
    let again = TrayFact { code: Some("pushed".into()), ..unsure.clone() };
    let added = TrayFact { kind: TrayFactKind::Added, certain: Some(false), ..unsure.clone() };
    let found =
        TrayFact { kind: TrayFactKind::Identified, certain: None, score: Some(900), bar: Some(750), ..unsure.clone() };
    let gone = TrayFact { kind: TrayFactKind::Removed, certain: Some(false), ..unsure };
    let ten = tray(10);
    let j = ten.facts.iter().position(|x| x.kind == TrayFactKind::Removed).unwrap();
    let late = TrayFact {
        kind: TrayFactKind::Identified,
        certain: None,
        score: Some(900),
        bar: Some(750),
        ..ten.facts[j].clone()
    };
    let late = [&ten.facts[..=j], std::slice::from_ref(&late), &ten.facts[j + 1..]].concat();
    want(
        "C3",
        &[
            ("two unsure facts", with_facts(&t, after(&[again]))),
            ("an addition after its first fact", with_facts(&t, after(&[added]))),
            ("identified after unsure", with_facts(&t, after(&[found]))),
            ("two removals, a fact after one", with_facts(&t, after(&[gone.clone(), gone]))),
            ("identified after removed", with_facts(&ten, late)),
        ],
    );
}

#[test]
fn c4_the_final_state_matches_the_facts() {
    let t = tray(13);
    let i = stone_of(&t, 292);
    assert!(t.stones[i].status == TrayStoneStatus::On && t.stones[i].sure);
    let ten = tray(10);
    let gone = ten.stones.iter().position(|s| s.status == TrayStoneStatus::Removed).unwrap();
    let opened = TimeRef { g: 119, t: 23.8, wall: None };
    let question = TrayQuestion { cause: "pushed".into(), why: "it was pushed".into(), since: 120, opened };
    want(
        "C4",
        &[
            (
                "removed with no removal",
                with_stone(&t, i, |s| {
                    s.status = TrayStoneStatus::Removed;
                    s.world_id = None;
                }),
            ),
            ("not sure with no unsure fact", with_stone(&t, i, |s| s.sure = false)),
            ("a question still open", with_stone(&t, i, |s| s.question = Some(question))),
            ("the removal unclear, the stone certain", with_stone(&ten, gone, |s| s.certain = Some(true))),
            ("the removed stone without certainty", with_stone(&ten, gone, |s| s.certain = None)),
        ],
    );
}

#[test]
fn c5_the_counts_and_the_open_list() {
    let t = tray(13);
    let n = t.facts.len();
    let (mut arrived, mut counted, mut unlisted) = (t.facts.clone(), t.facts.clone(), t.facts.clone());
    arrived[0].stones = arrived[0].stones.map(|x| x + 1);
    counted[n - 1].sure = counted[n - 1].sure.map(|x| x + 1);
    unlisted[n - 1].open = Some(vec![]);
    want(
        "C5",
        &[
            ("the arrival count", with_facts(&t, arrived)),
            ("the sure count", with_facts(&t, counted)),
            ("no open list", with_facts(&t, unlisted)),
        ],
    );
    // one entry per open stone, in tray order
    let open = t.facts[n - 1].open.clone().unwrap();
    assert!(open.len() >= 2 && t.check().is_empty());
    let (mut twice, mut swapped) = (t.facts.clone(), t.facts.clone());
    twice[n - 1].open = Some([&open[..], &open[..1]].concat());
    swapped[n - 1].open = Some([&open[1..2], &open[..1], &open[2..]].concat());
    assert_eq!(with_facts(&t, twice), ["C5"], "an entry twice");
    assert_eq!(with_facts(&t, swapped), ["C5"], "out of tray order");
    // an id on the tray twice (C2): each stone is open or settled by itself (4.6.8), so an open copy of a sure stone
    // needs its own entry
    let sure = t.stones[stone_of(&t, 291)].clone();
    assert!(sure.status == TrayStoneStatus::On && sure.sure);
    let mut twin = t.clone();
    twin.stones.push(TrayStone {
        sure: false,
        added: true,
        certain: Some(false),
        world_id: Some(99999),
        ..sure.clone()
    });
    let mut added = open.iter().find(|o| o.kind == TrayFactKind::Added).unwrap().clone();
    added.stone = sure.id.clone();
    let (mut listed, mut unlisted) = (t.facts.clone(), t.facts.clone());
    for facts in [&mut listed, &mut unlisted] {
        facts[n - 1].stones = facts[n - 1].stones.map(|s| s + 1);
    }
    listed[n - 1].open = Some([&open[..], &[added]].concat());
    assert_eq!(with_facts(&twin, listed), ["C2"], "an open copy, listed");
    assert_eq!(with_facts(&twin, unlisted), ["C2", "C5"], "an open copy, not listed");
}

#[test]
fn c6_world_ids_are_unique_on_the_tray() {
    let t = tray(13);
    let (a, other) = (stone_of(&t, 292), t.stones[stone_of(&t, 294)].world_id);
    want(
        "C6",
        &[
            ("one world id twice", with_stone(&t, a, |s| s.world_id = other)),
            ("no world id", with_stone(&t, a, |s| s.world_id = None)),
        ],
    );
}

#[test]
fn c7_the_open_entries_are_well_formed() {
    let t = tray(13);
    let (unsure, added) = (t.stones[stone_of(&t, 301)].id.clone(), t.stones[stone_of(&t, 311)].id.clone());
    let (cands, added_cands) = (entry(&t, &unsure).candidates, entry(&t, &added).candidates);
    let ten = tray(10);
    let removed = ten.stones.iter().find(|s| s.status == TrayStoneStatus::Removed).unwrap().id.clone();
    let removed_cands = entry(&ten, &removed).candidates;
    let set = |c: Vec<TrayCandidate>| move |o: &mut TrayOpen| o.candidates = c;
    let not_gone: Vec<TrayCandidate> = cands.iter().filter(|c| c.kind != "gone").cloned().collect();
    let twice = [&cands[..], &cands[..1]].concat();
    want(
        "C7",
        &[
            ("no such cause", with_entry(&t, &unsure, |o| o.cause = "no such cause".into())),
            ("no such look", with_entry(&t, &unsure, |o| o.look = "no such look".into())),
            ("an entry of no open kind", with_entry(&t, &unsure, |o| o.kind = TrayFactKind::Leaving)),
            ("no candidates", with_entry(&t, &unsure, set(vec![]))),
            ("a candidate twice", with_entry(&t, &unsure, set(twice))),
            ("its own world stone not first", with_entry(&t, &unsure, set(cands[1..].to_vec()))),
            ("its world id gone", with_entry(&t, &unsure, |o| o.world_id = None)),
            ("no gone candidate", with_entry(&t, &unsure, set(not_gone))),
            ("no new last", with_entry(&t, &added, set(added_cands[..added_cands.len() - 1].to_vec()))),
            ("no unseen last", with_entry(&ten, &removed, set(removed_cands[..removed_cands.len() - 1].to_vec()))),
        ],
    );
}

#[test]
fn c8_a_candidate_is_never_a_settled_stone() {
    let t = tray(13);
    let added = entry(&t, &t.stones[stone_of(&t, 311)].id);
    let sure = &t.stones[stone_of(&t, 292)];
    let first = |c: TrayCandidate| {
        let rest = added.candidates.clone();
        move |o: &mut TrayOpen| o.candidates = [vec![c], rest].concat()
    };
    let settled = TrayCandidate { kind: "stone".into(), stone: Some(sure.id.clone()), ..TrayCandidate::default() };
    let taken = TrayCandidate { kind: "world".into(), world_id: sure.world_id, ..settled.clone() };
    want(
        "C8",
        &[
            ("a settled stone", with_entry(&t, &added.stone, first(settled.clone()))),
            ("a sure stone's world id", with_entry(&t, &added.stone, first(taken))),
        ],
    );
    // an entry with a bad cause breaks C7, and its candidate naming a sure stone C8 as well
    let malformed = with_entry(&t, &added.stone, |o| {
        o.cause = "no such cause".into();
        o.candidates = [vec![settled], added.candidates.clone()].concat();
    });
    assert_eq!(malformed, ["C7", "C8"]);
}

#[test]
fn c9_an_identified_fact_cleared_its_bar() {
    let t = tray(12);
    let i = t.facts.iter().position(|x| x.kind == TrayFactKind::Identified).unwrap();
    let found = t.facts[i].clone();
    let bar = found.bar.unwrap();
    assert!(found.score.is_some());
    // a hand-made fact without its score says so
    let text = TrayFact { score: None, ..found.clone() }.describe();
    assert!(text.ends_with(&format!("by its look on world stone {} (-)", found.world_id.unwrap())), "{text}");
    let broken = [
        ("below its bar", TrayFact { score: Some(bar - 1), ..found.clone() }),
        ("a bar below 0.750", TrayFact { bar: Some(700), score: Some(720), ..found.clone() }),
        ("certain", TrayFact { certain: Some(true), ..found.clone() }),
        ("a margin below 150", TrayFact { margin: Some(149), ..found.clone() }),
        ("no score", TrayFact { score: None, ..found }),
    ];
    let checks: Vec<(&str, Vec<String>)> = broken
        .into_iter()
        .map(|(name, x)| {
            let mut facts = t.facts.clone();
            facts[i] = x;
            (name, with_facts(&t, facts))
        })
        .collect();
    want("C9", &checks);
}

#[test]
fn the_open_account_as_values() {
    let t = tray(13);
    let d = serde_json::to_value(&t).unwrap();
    assert_eq!(d["t_id"], json!(750));
    let tie = entry(&t, &t.stones[stone_of(&t, 311)].id);
    let pushed = t.stones[stone_of(&t, 301)].id.clone();
    assert_eq!(
        serde_json::to_value(&tie).unwrap(),
        json!({"stone": tie.stone, "world_id": 311, "kind": "added", "cause": "doubtful", "look": "tie",
               "score": tie.score, "hand_near": true, "seen_at_arrival": false,
               "candidates": serde_json::to_value(&tie.candidates).unwrap()})
    );
    assert_eq!(
        serde_json::to_value(&tie.candidates[0]).unwrap(),
        json!({"kind": "stone", "world_id": null, "stone": pushed, "score": tie.score, "why": null})
    );
    assert_eq!(tie.candidates.last(), Some(&TrayCandidate { kind: "new".into(), ..TrayCandidate::default() }));
    assert_eq!(d["open"], serde_json::to_value(&t.open).unwrap());
    assert_eq!(t.stones[stone_of(&t, 311)].open.as_ref(), Some(&tie));
    assert_eq!(t.facts.last().unwrap().open, t.open);
    assert_eq!(serde_json::from_value::<Tray>(d).unwrap(), t);
}

/// A stone in question is not sure while its tray is on, and holds its question; the live tray
/// breaks C1 (it has not left) and C4 (a question is open) until it leaves.
#[test]
fn a_question_while_its_tray_is_on() {
    let mut core = InterpreterCore::new();
    for line in Replay::open(recording(LOOKS)).unwrap().lines() {
        let Line::Frame(frame) = line.unwrap() else { continue };
        core.feed(*frame);
        let Some(live) = core.trays().pop().filter(|t| t.leaving.is_none()) else { continue };
        let Some(s) = live.stones.iter().find(|s| s.question.is_some()) else { continue };
        let q = s.question.as_ref().unwrap();
        assert!(!s.sure && ["pushed", "neighbour_pushed", "found_again"].contains(&q.cause.as_str()), "{s:?}");
        let checks = live.check();
        assert!(checks.iter().any(|c| c == "C1") && checks.iter().any(|c| c == "C4"), "{checks:?}");
        assert_eq!(serde_json::to_value(s).unwrap()["question"]["since"], json!(q.since));
        return;
    }
    panic!("no question while a tray was on");
}
