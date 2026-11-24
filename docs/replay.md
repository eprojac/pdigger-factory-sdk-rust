# Replays

A recording is what an API client saw of a session: every frame with its layers and the side events (clock, worker,
gaps, resets), in the order received. The interpreter reads one exactly as it follows a unit, so a replay prints
the same sentences the unit's session did. Replays need no unit and no network (a file), or only an HTTP server (a
URL).

- The four complete recordings (aug28, jul30, dry1, dry2: every frame interpreted, with their summaries; dry1
  recorded on the 20.20 unit, the other three in a replay of the same model and build on a Mac, as each header's
  `recorded_from` says) are published at https://factory.pdigger.com/developers/data/.
- Short excerpts of them are in [fixtures/recordings](../fixtures/recordings), with the outputs of the reference
  interpreter in [fixtures/golden](../fixtures/golden); the golden tests replay every one of them.

## `Replay`: a file, synchronous

<!-- example: crates/pdigger-factory-sdk/examples/replay.rs -->
```rust
//! Reads a recording offline and prints its activities and its summary.
//!
//!     cargo run --example replay -- dry1.world.ndjson.gz
use pdigger_factory_sdk::Replay;

fn main() -> pdigger_factory_sdk::Result<()> {
    let path =
        std::env::args().nth(1).unwrap_or_else(|| "fixtures/recordings/dry1-0296-0306-full.ndjson.gz".to_string());
    let out = Replay::open(&path)?.run()?; // events, activities and the summary in one pass
    for activity in &out.activities {
        println!("{}", activity.describe()); // 5:01.6 Right hand moves stone 643 1.09 units in 1.2 s
    }
    print!("{}", out.summary.describe());
    Ok(())
}
```

`Replay::open(path)` reads `.ndjson` or `.ndjson.gz` (gzip is detected); `Replay::from_bytes` and
`Replay::from_reader` read from memory or any `Read`. The header (`header()`: `RecordingHeader` with `video`,
`source`, `build`, `layers`, `redacted`, `frames`, `g_first`, `g_last`, `sessions`, `notes`) is read when the replay
opens. Then, each consuming the replay:

| Method | Gives |
|---|---|
| `events()`, `activities()` | iterators of `Result<Event>`, `Result<Activity>` |
| `tray_facts()`, `trays()` | the tray facts (an iterator of `Result<TrayFact>`), or every tray of the recording ([trays](#trays)) |
| `updates()` | `Updates`, an iterator of `Result<Update>` (duplicates skipped; the last one `is_final`, with what `end()` closed: the activities still open and the facts of the last tray leaving); `Updates::core()` is the interpreter |
| `frames()`, `lines()` | the frames, or every line (frames and side events) |
| `summary()` | the `Summary` |
| `run()` | everything in one pass: `ReplayOutput { header, events, activities, tray_facts, trays, summary }` |
| `into_stream()` | the same replay as a `ReplayStream` |

`frames()` gives the frames only, without the side lines (the clock, and the `gap` and `reset` lines whose dropped
frames end the trays' presence questions): to feed an `InterpreterCore` of your own, give it every line of `lines()`
with `core.feed_line(line)` rather than feeding it `frames()`. `feed_line` feeds a line as the methods above do: a
frame gives its `Update` (a duplicate gives none), a clock line goes to `set_clock`, and a `gap` or `reset` line whose
data decodes (as `GapEvent` or `ResetEvent`) to `gap()`; any other line changes nothing.

No runtime is needed: the methods above return plain iterators, lazy, one line at a time, so a complete recording is
never held in memory. Bytes that are not UTF-8 read as U+FFFD, and the lines a reader does not
understand are skipped (spec 6): blank lines, lines that are not JSON (`NaN` and `Infinity` are
not), JSON that is not an object, frame and side lines that do not decode (a field of spec 2.3
rule 9 missing, null or of the wrong type: a `g` that is not a whole number, a frame whose `world`
is no object, a clock line whose `recv` is not a number), side lines of events the SDK does not
know, and any other line. Rust's models read a whole number only when it is written without a
fraction or an exponent (`2.0` is no `g`), one of the edges spec 2.3 rule 9 leaves to each SDK. The
header is the first line that is not blank, when it has `recording` and decodes by its table (a
member its model cannot hold is left out of it: a `source` that is no catalog entry, `layers` or
`redacted` that are not arrays of strings, `sessions` that are not whole numbers; `complete` and
the recorder's counts stay in `extra` as they are); otherwise the recording has none, and a
`recording` line further down is skipped like any other line. So a file that is no recording at all replays as
nothing. `parse_line` reads one line: a line that is not JSON, or one that does not decode as the
header, side line or frame it claims to be, is an error (`decode`), for a program that wants to
know; a blank line, JSON that is not an object and any other line give `None`. `fixtures/damaged`
holds a line of each kind; `tests/damaged.rs` replays them. A file that cannot be read to its end (a failed
read, a gzip stream cut short) gives its error once and ends there, with no final update; the
error is then the only `Err` a replay gives, so `summary()`, `trays()` and `run()` fail only on
it.

## `ReplayStream`: a file or a URL, async, paced

<!-- example: crates/pdigger-factory-sdk/examples/replay_url.rs -->
```rust
//! Streams a published recording from its URL and prints its events as they are read, at the
//! recording's own pace (`realtime(4.0)`: four times as fast).
//!
//!     cargo run --example replay_url -- https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz
use futures::StreamExt;
use pdigger_factory_sdk::ReplayStream;

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz".to_string());
    let replay = ReplayStream::open_url(&url).await?;
    if let Some(header) = replay.header() {
        println!("{} ({} frames)", header.video.as_deref().unwrap_or("?"), header.frames.unwrap_or(0));
    }
    let mut events = replay.realtime(4.0).events().take(10);
    while let Some(event) = events.next().await {
        println!("{}", event?.describe());
    }
    Ok(())
}
```

`ReplayStream::open(path)` and `ReplayStream::open_url(url)` (streamed as it downloads, gzip decoded on the way;
`open_url_with(transport, url)` for your own transport) give the same methods as streams: `events()`, `activities()`,
`tray_facts()`, `updates()`, `frames()`, `lines()`, `trays().await` and `summary().await`. `realtime(speed)` paces the frames by their media times
(`realtime(1.0)` as recorded, `realtime(4.0)` four times as fast), which is what a dashboard demo wants; without it a
replay runs as fast as it reads. The clock lines of the recording give the events their wall times.

## Trays

`trays()` reads the whole recording and lists its trays, each with the point where it had arrived and settled and the
point just before it moved out, its tray stones (the SDK's own ids, never reused) and its facts
([the interpreter](interpreter.md#trays-and-tray-stones)). `window()` is the media span to replay or to show. To show
a point on the unit, select the tray's video there; that moves the session for every viewer, so the example sends dry
runs:

<!-- example: crates/pdigger-factory-sdk/examples/tray_points.rs -->
```rust
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
```

Over the fixture `dry1-0018-0153` it prints `tray 1: arrived 0:22.5, leaving 2:24.0`, the six stones removed for
certain (`  removed: 20261122T023847.0Z-1-105` and the others), then `seek to 0:22.5 would show dry1 from 0:22.5` and
`seek to 2:24.0 would show dry1 from 2:24.0`. The viewer jumps between the same points with `[` and `]`
([viz](viz.md#the-viewer)).

## The recording format (`pfr1`)

NDJSON, optionally gzip. Line 1 is the header (`{"recording": "pfr1", "video": "dry1", "source": {...}, ...}`); every
other line is either a frame exactly as the API sent it (it has `cursor` and `g`; backfilled layers merged in, `missing`
then lists only the layers still missing) or a side event `{"sse": "clock" | "worker" | "gap" | "reset", "data": {...},
"recv": epoch}`. Public recordings are redacted: face keypoints (joints 0 to 4 and 69) are null, appearance masks and
depth textures are removed; the header's `redacted` lists them. Section 6 of [spec/SDK-DESIGN.md](../spec/SDK-DESIGN.md)
is the full description.

## The complete recordings

The opt-in test `the_complete_recordings` (see [TESTING.md](../TESTING.md)) replays the complete recordings and compares
their summaries with the published ones:

```sh
PDIGGER_FACTORY_RECORDINGS=<dir> cargo test -p pdigger-factory-sdk --release --test integration -- --ignored
```
