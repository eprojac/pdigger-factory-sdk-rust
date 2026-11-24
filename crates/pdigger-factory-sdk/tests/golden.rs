//! Golden tests (spec 5): a replay of every `fixtures/recordings/*` must reproduce
//! `fixtures/golden/<name>.events.txt`, `.tray-stones.txt`, `.activities.txt`, `.trays.txt`,
//! `.open.txt`, `.reports.txt`, `.speckle.txt`, `.summary.txt` byte for byte and `.summary.json` after parsing, and every tray
//! must pass its self-check. The folder is iterated, so new recordings are picked up; the four of
//! the presence questions (spec 4.6.1) must be among them.

mod common;

use futures::StreamExt;
use pdigger_factory_sdk::replay::Line;
use pdigger_factory_sdk::{Replay, ReplayStream, Summary};

use common::{fixtures, normalise, recordings};

fn lines<T: ToString>(items: &[T]) -> String {
    items.iter().map(|i| format!("{}\n", i.to_string())).collect()
}

fn golden(name: &str, ext: &str) -> String {
    std::fs::read_to_string(fixtures().join("golden").join(format!("{name}.{ext}")))
        .unwrap_or_else(|e| panic!("golden {name}.{ext}: {e}"))
}

fn check_summary(name: &str, summary: &Summary) {
    let expected: serde_json::Value = serde_json::from_str(&golden(name, "summary.json")).unwrap();
    assert_eq!(normalise(serde_json::to_value(summary).unwrap()), normalise(expected), "{name}.summary.json");
    assert_eq!(summary.describe(), golden(name, "summary.txt"), "{name}.summary.txt");
}

#[test]
fn every_recording_reproduces_its_golden_files() {
    let all = recordings();
    assert!(all.len() >= 17, "the fixtures have at least seventeen recordings");
    for name in [
        "trays-presence-synthetic",
        "trays-presence-looks-synthetic",
        "trays-presence-attacks-synthetic",
        "trays-presence-attacks-looks-synthetic",
    ] {
        assert!(all.iter().any(|(n, _)| n == name), "{name} is a fixture recording");
    }
    for (name, path) in all {
        let out = Replay::open(&path).unwrap().run().unwrap();
        assert_eq!(lines(&out.events), golden(&name, "events.txt"), "{name}.events.txt");
        // every event with a tray stone: the link before its frame, a revision's too
        let stones: String = out
            .events
            .iter()
            .filter_map(|e| {
                let stone = e.tray_stone.as_deref()?;
                Some(format!("{} {} {} {stone}\n", e.g, e.kind, e.stone_id?))
            })
            .collect();
        assert_eq!(stones, golden(&name, "tray-stones.txt"), "{name}.tray-stones.txt");
        assert_eq!(lines(&out.activities), golden(&name, "activities.txt"), "{name}.activities.txt");
        assert_eq!(lines(&out.tray_facts), golden(&name, "trays.txt"), "{name}.trays.txt");
        let open = pdigger_factory_sdk::describe_open_txt(&out.tray_facts);
        assert_eq!(open, golden(&name, "open.txt"), "{name}.open.txt");
        for tray in &out.trays {
            assert_eq!(tray.check(), Vec::<String>::new(), "{name}: tray {} checks", tray.n);
        }
        let reports: String = out.trays.iter().filter_map(|t| t.report()).map(|r| r.describe() + "\n").collect();
        assert_eq!(reports, golden(&name, "reports.txt"), "{name}.reports.txt");
        let speckle = pdigger_factory_sdk::describe_speckle_txt(&out.speckle);
        assert_eq!(speckle, golden(&name, "speckle.txt"), "{name}.speckle.txt");
        let kept: Vec<_> = out.trays.iter().flat_map(|t| t.facts.clone()).collect();
        assert_eq!(kept, out.tray_facts, "{name}: every tray is kept, with its facts in order");
        check_summary(&name, &out.summary);
        assert_eq!(out.header.as_ref().map(|h| h.recording.as_str()), Some("pfr1"));
    }
}

#[test]
fn the_iterators_agree_with_one_pass() {
    for (name, path) in recordings() {
        let out = Replay::open(&path).unwrap().run().unwrap();
        let events: Vec<_> = Replay::open(&path).unwrap().events().map(Result::unwrap).collect();
        assert_eq!(events, out.events, "{name} events()");
        let activities: Vec<_> = Replay::open(&path).unwrap().activities().map(Result::unwrap).collect();
        assert_eq!(activities, out.activities, "{name} activities()");
        let facts: Vec<_> = Replay::open(&path).unwrap().tray_facts().map(Result::unwrap).collect();
        assert_eq!(facts, out.tray_facts, "{name} tray_facts()");
        assert_eq!(Replay::open(&path).unwrap().trays().unwrap(), out.trays, "{name} trays()");
        let reports: Vec<_> = out.trays.iter().filter_map(|t| t.report()).collect();
        assert_eq!(Replay::open(&path).unwrap().tray_reports().unwrap(), reports, "{name} tray_reports()");
        assert_eq!(Replay::open(&path).unwrap().summary().unwrap(), out.summary, "{name} summary()");
        let frames = Replay::open(&path).unwrap().frames().count() as u64;
        assert_eq!(frames, out.summary.frames + out.summary.revisions + duplicates(&path), "{name} frames()");
        // an update per frame fed, duplicates skipped, then the final one
        let mut updates = Replay::open(&path).unwrap().updates();
        let all: Vec<_> = updates.by_ref().map(Result::unwrap).collect();
        let (last, fed) = all.split_last().unwrap();
        assert!(last.is_final && last.frame.is_none());
        assert_eq!(fed.len() as u64, out.summary.frames + out.summary.revisions, "{name} updates()");
        assert!(fed.iter().all(|u| !u.is_duplicate && !u.is_final));
        assert_eq!(updates.core().summary(), out.summary);
    }
}

/// Frames the interpreter drops as duplicates (same g, no higher rev).
fn duplicates(path: &std::path::Path) -> u64 {
    let mut core = pdigger_factory_sdk::InterpreterCore::new();
    let mut n = 0;
    for frame in Replay::open(path).unwrap().frames() {
        n += u64::from(core.feed_parts(&frame.unwrap()).is_duplicate);
    }
    n
}

#[tokio::test]
async fn the_async_replay_agrees() {
    for (name, path) in recordings() {
        let out = Replay::open(&path).unwrap().run().unwrap();
        let events: Vec<_> = ReplayStream::open(&path).unwrap().events().map(Result::unwrap).collect().await;
        assert_eq!(events, out.events, "{name} async events");
        let activities: Vec<_> = ReplayStream::open(&path).unwrap().activities().map(Result::unwrap).collect().await;
        assert_eq!(activities, out.activities, "{name} async activities");
        let facts: Vec<_> = ReplayStream::open(&path).unwrap().tray_facts().map(Result::unwrap).collect().await;
        assert_eq!(facts, out.tray_facts, "{name} async tray facts");
        assert_eq!(ReplayStream::open(&path).unwrap().trays().await.unwrap(), out.trays, "{name} async trays");
        let reports: Vec<_> = out.trays.iter().filter_map(|t| t.report()).collect();
        assert_eq!(ReplayStream::open(&path).unwrap().tray_reports().await.unwrap(), reports, "{name} async reports");
        assert_eq!(ReplayStream::open(&path).unwrap().summary().await.unwrap(), out.summary, "{name} async summary");
        let frames = ReplayStream::open(&path).unwrap().frames().count().await;
        assert!(frames as u64 >= out.summary.frames);
        let updates: Vec<_> = ReplayStream::open(&path).unwrap().updates().map(Result::unwrap).collect().await;
        assert_eq!(updates.len() as u64, out.summary.frames + out.summary.revisions + 1, "{name} async updates");
    }
}

#[tokio::test(start_paused = true)]
async fn realtime_pacing_follows_media_time() {
    let path = fixtures().join("recordings/dry1-0296-0306-full.ndjson.gz");
    let start = tokio::time::Instant::now();
    let replay = ReplayStream::open(&path).unwrap().realtime(10.0);
    assert!(format!("{replay:?}").contains("speed: Some(10.0)"));
    let frames: Vec<_> = replay.frames().map(Result::unwrap).collect().await;
    let span = frames.last().unwrap().t - frames[0].t;
    let waited = start.elapsed().as_secs_f64();
    assert!((waited - span / 10.0).abs() < 0.01, "waited {waited} for a span of {span}");
    let unpaced = ReplayStream::open(&path).unwrap().realtime(-1.0);
    let lines: Vec<_> = unpaced.lines().collect().await;
    assert!(lines.iter().all(Result::is_ok));
    assert!(lines.iter().any(|l| matches!(l, Ok(Line::Side(_)))));
}
