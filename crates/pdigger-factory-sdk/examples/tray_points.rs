//! The trays of a recording: when each had arrived and settled, and when it was about to move
//! out, with the stones removed from it for certain. Then one tray on the shared session: its two
//! points as dry runs (a real select moves the one session for every viewer).
//!
//! The tray stones carry the SDK's own ids, one per stone and never reused, so "stone X was
//! removed" holds over time; the world's ids are the engine tracker's, which renames stones and
//! reuses ids.
//!
//!     cargo run --example tray_points -- dry1.world.ndjson.gz 2 http://factory-orin.local:8120
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::{FactoryClient, Replay, TrayStoneStatus};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "fixtures/recordings/dry1-0018-0153.ndjson.gz".to_string());
    let show: i64 = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    let url = args.next().unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let trays = Replay::open(&path)?.trays()?; // every tray, with its two points
    for tray in &trays {
        let Some((arrived, leaving)) = tray.window() else { continue }; // a stop that never settled
        println!("tray {}: arrived {}, leaving {}", tray.n, clock(arrived), clock(leaving));
        for stone in &tray.stones {
            if stone.status == TrayStoneStatus::Removed && stone.certain == Some(true) {
                println!("  removed: {}", stone.id);
            }
        }
    }
    let Some(tray) = trays.iter().find(|t| t.n == show && t.window().is_some()) else { return Ok(()) };
    let session = FactoryClient::new(&url)?.session().dry_run(); // validated by the unit, nothing moves
    for point in [tray.arrived, tray.leaving].into_iter().flatten() {
        let would = session.watch_at(&tray.video, Some(point.t)).await?;
        println!("seek to {} would show {} from {}", clock(point.t), would.video, clock(would.anchor_media));
    }
    Ok(())
}
