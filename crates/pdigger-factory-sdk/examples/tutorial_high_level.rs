//! The high-level tutorial (docs/tutorials/01-high-level.md) in one program, offline: a recording's
//! activities, as sentences and as data; its tray facts, and its trays with the stones removed from
//! them; and from one pass, its summary and a stone's path. A `Replay` is one pass over the file,
//! so each step opens the recording again; only the interpreter's state is in memory.
//!
//!     cargo run --example tutorial_high_level -- fixtures/recordings/dry1-0018-0153.ndjson.gz 155
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::{Activity, Replay, TrayStoneStatus};

fn main() -> pdigger_factory_sdk::Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "fixtures/recordings/dry1-0018-0153.ndjson.gz".to_string());
    let stone: i64 = args.next().and_then(|id| id.parse().ok()).unwrap_or(155);

    // 1. The activities, as sentences
    for activity in Replay::open(&path)?.activities() {
        println!("{}", activity?.describe()); // 1:19.2 Left hand takes stone 155 away (1.0 s)
    }

    // 2. The same activities as data: a move is one hand moving one stone; a push is a burst of pushes
    for activity in Replay::open(&path)?.activities() {
        let Activity::Move(m) = activity? else { continue };
        let id = m.stone_id.map_or("?".to_string(), |id| id.to_string());
        let hand = m.hand.map_or("no".to_string(), |hand| hand.to_string());
        println!("{} stone {id}: {hand} hand, {}, {:.1} s", clock(m.start.t), m.outcome, m.duration);
    }

    // 3. Tray facts: what happened to each tray and its stones, under stone ids of the SDK's own
    for fact in Replay::open(&path)?.tray_facts() {
        println!("{}", fact?.describe()); // 0:22.5 tray 1 arrived: 42 stones
    }

    // 4. The trays, with their two points: arrived and settled, and the last calm frame before leaving
    for tray in Replay::open(&path)?.trays()? {
        let Some((arrived, leaving)) = tray.window() else { continue }; // a stop that never settled
        let gone = tray.stones.iter().filter(|s| s.status == TrayStoneStatus::Removed && s.certain == Some(true));
        println!(
            "tray {}: {} to {}, {} stones removed for certain",
            tray.n,
            clock(arrived),
            clock(leaving),
            gone.count()
        );
    }

    // 5. One pass, and its core: the summary, and where a stone of the world was, frame by frame
    let mut updates = Replay::open(&path)?.updates();
    for update in &mut updates {
        update?; // the events, activities and tray facts of one frame
    }
    let core = updates.core();
    print!("{}", core.summary().describe());
    let track = core.track(stone); // scene units
    if let (Some(first), Some(last)) = (track.first(), track.last()) {
        println!("stone {stone}: {} points from {} to {}", track.len(), clock(first.t), clock(last.t));
    }
    Ok(())
}
