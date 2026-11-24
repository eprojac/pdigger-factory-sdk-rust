//! The first tutorial (docs/tutorials/00-start-here.md) in one program, offline: one report per
//! tray, the same reports as data, and the details behind a report that is not complete. A
//! `Replay` is one pass over the file, so each step opens the recording again.
//!
//!     cargo run --example tutorial_reports -- fixtures/recordings/dry1-0018-0153.ndjson.gz
use pdigger_factory_sdk::{Replay, TrayChange};
use serde_json::json;

const THRESHOLD: f64 = 0.9; // keep the changes at least this likely to be right; every move is kept

fn main() -> pdigger_factory_sdk::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "fixtures/recordings/dry1-0018-0153.ndjson.gz".to_string());

    // 1. One report per tray: the stones removed, added and moved, when and where, and whether that is all of it
    for report in Replay::open(&path)?.tray_reports()? {
        println!("{}", report.describe());
    }

    // 2. The same reports as data, one JSON line per tray, for a database, an MES or a spreadsheet
    // the stones of the changes whose confidence (the probability that they are right) is at least floor
    let likely = |changes: &[TrayChange], floor: f64| -> Vec<String> {
        changes.iter().filter(|change| change.confidence >= floor).map(|change| change.stone.clone()).collect()
    };
    for report in Replay::open(&path)?.tray_reports()? {
        let row = json!({
            "tray": report.tray,
            "arrived": report.arrived.wall, // UTC epoch seconds (null when the unit's clock is not known)
            "left": report.left.wall,
            "removed": likely(&report.removed, THRESHOLD),
            "added": likely(&report.added, THRESHOLD),
            "moved": likely(&report.moved, 0.0),
            "unclear": report.unclear,
            "complete": report.complete,
        });
        println!("{row}"); // serde_json::to_string(&report) has every field
    }

    // 3. Behind a report that is not complete, advanced mode: the tray's facts, each with its reason
    for tray in Replay::open(&path)?.trays()? {
        if tray.report().is_none_or(|report| report.complete) {
            continue;
        }
        for fact in &tray.facts {
            // 0:30.1 tray 1: stone 20261122T023847.0Z-1-94 is not sure any more (it was pushed)
            println!("{}", fact.describe());
        }
    }
    Ok(())
}
