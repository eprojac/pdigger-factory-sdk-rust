# Follow hand movements

<a id="high-level-what-the-hands-did"></a>

Follow one worker’s hands through a recording. You’ll read individual events, group them into moves and pushes, then
trace a single stone. The same interpreter can follow a live session.

From the repository root, run the
[examples/tutorial_high_level.rs](../../crates/pdigger-factory-sdk/examples/tutorial_high_level.rs) program.

```sh
cargo run --example tutorial_high_level -- fixtures/recordings/dry1-0018-0153.ndjson.gz 155
```

<details>
<summary>Show the complete Rust example</summary>

<!-- example: crates/pdigger-factory-sdk/examples/tutorial_high_level.rs -->
```rust
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
```

</details>

`Replay`'s methods are synchronous iterators (no runtime needed), and each is one pass over the file, streamed: only
the interpreter's state is in memory, so a recording of hours works the same. Run it on the fixture
`fixtures/recordings/dry1-0018-0153.ndjson.gz`, dry1 from 0:18 to 2:33, where a tray arrives with 42 stones, the hands
take some off and push the others into place; any published recording works the same.

## 1. The activities

The first activities of dry1-0018-0153:

```text
0:20.5 Both hands push stones 86, 90 and 94 (6 pushes in 0.6 s)
0:20.3 Right hand pushes stones 89, 90, 94, 95 and 109 (13 pushes in 1.6 s)
0:21.9 Left hand pushes stone 86 (1 push)
```

## 2. The same activities as data

A sentence is for people; your program reads the fields. `Activity::Move` is one stone moved by one hand, with where
it went (`outcome`: `placed`, `dropped`, `carried_away`, `removed`, or `interrupted` when the recording or the session
ended first); `Activity::Push` is a burst of pushes. In dry1-0018-0153 that is nine moves, among them
`1:19.2 stone 155: left hand, carried_away, 1.0 s`: five placed, two dropped and two carried away.

## 3. Tray facts

The world's stone ids belong to the engine's tracker, which renames stones and reuses ids. For trays the SDK keeps
ids of its own, one per stone and never reused, so that "this stone was removed" stays true over time. Its tray facts
say what happened to each tray and its stones; the first removals from dry1-0018-0153's tray:

```text
0:45.5 tray 1: stone 20261122T023847.0Z-1-117 was removed
0:45.5 tray 1: stone 20261122T023847.0Z-1-127 was removed
```

and its last fact, as it moves out: `2:24.0 tray 1 leaving: 44 stones (30 sure), 0 added, 6 removed, 18 unclear`.

## 4. The trays

A tray has two points to jump to: when it had arrived and settled, and its last calm frame before it moved out.
`window()` gives both, and each tray stone says whether it was removed, and whether that is certain. For
dry1-0018-0153: `tray 1: 0:22.5 to 2:24.0, 6 stones removed for certain`.

## 5. One pass, and its core

`updates()` hands out each frame's events, activities and tray facts; after the pass, `core()` is the interpreter that
made them: ask it for its summary, its state, or the path of any stone of the world. The summary of dry1-0018-0153
begins:

```text
frames: 676 (with world 676, without 0, revisions 0)
media: 2:14.6, sessions 1, gaps 0
stones: last 42, max 49, confirmed 42
moves: 9 (placed 5, dropped 2, carried away 2, removed 0, interrupted 0, partial 0, carried 6), mean 1.4 s, 0.18 units
```

and stone 155's path has 5 points, from 1:15.6 to 1:19.8.

## 6. Follow a unit, live

The same interpreter follows the unit's shared session, whatever it shows: a recording it plays, or a live camera.
[examples/quickstart.rs](../../crates/pdigger-factory-sdk/examples/quickstart.rs) prints its events as they happen:

<!-- example: crates/pdigger-factory-sdk/examples/quickstart.rs -->
```rust
//! Follows the shared session (a recording, or a live camera) and prints what happens on the
//! tray, as the unit's videos say it.
//!
//!     cargo run --example quickstart -- http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::FactoryClient;

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    let mut events = factory.interpreter().events().take(20); // drop .take(20) to follow for ever
    while let Some(event) = events.next().await {
        println!("{}", event?.describe()); // 5:01.6 Right hand lifts stone 643
    }
    Ok(())
}
```

The stream resumes by itself when the connection drops, holds a frame whose world came late until it is read back,
and gives every event its wall time from the session's clock. To follow the activities, read the hands and the
summary, and wait for one event with a time limit, see
[examples/interpreter.rs](../../crates/pdigger-factory-sdk/examples/interpreter.rs) and
[interpreter.md](../interpreter.md).

## 7. Show a tray on the shared screen

Every viewer of the unit sees the same session. To show a tray there, seek to one of its points:
[examples/tray_points.rs](../../crates/pdigger-factory-sdk/examples/tray_points.rs) lists a recording's trays and asks
the unit with dry runs (`session().dry_run()`), which it checks and does not apply; leave out `dry_run()` to really
move the session for everybody.

## 8. Test your program without a unit

The feature `testing` adds `FakeFactory`, which answers the whole API from a recording, in process: see
[the Rust tutorial](04-rust.md#the-fake-unit-in-your-tests).

## Next

- [Low level](02-low-level.md): the operations under the interpreter, one call each.
- [Live cameras](03-live-cameras.md): select a plant camera and follow it.
- The reference: [interpreter.md](../interpreter.md) (every event, activity, tray fact and the summary) and
  [replay.md](../replay.md) (replays, pacing, the recording format).
