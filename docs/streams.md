# Streams

The mid level: frames as they are released, with the API's stream rules handled for you. `FactoryClient::streams()`
gives `Streams`; each method returns a `futures::Stream` of `Result<StreamItem>`, and dropping it closes the
connection.

<!-- example: crates/pdigger-factory-sdk/examples/streams.rs -->
```rust
//! The mid level: frames over SSE (resumed after a drop, late layers backfilled, revisions
//! flagged) and the long-poll alternative.
//!
//!     cargo run --example streams -- http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::streams::PollOptions;
use pdigger_factory_sdk::{FactoryClient, FramesOptions, Layer, StreamItem};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let streams = FactoryClient::new(&url)?.streams();
    let options = FramesOptions { layers: vec![Layer::World, Layer::Cards], observe: Some(true), ..Default::default() };
    let mut frames = streams.frames(options).take(5);
    while let Some(item) = frames.next().await {
        match item? {
            StreamItem::Frame(item) => {
                let stones = item.frame.world.as_ref().map_or(0, |w| w.stones.len());
                println!(
                    "frame g {} at {:.1} s: {stones} stones (revision: {})",
                    item.frame.g, item.frame.t, item.is_revision
                );
            }
            StreamItem::Clock(clock) => println!("clock: {} at {:.1} s", clock.state.video, clock.state.media_time),
            other => println!("{other:?}"),
        }
    }
    let poll = PollOptions { layers: vec![Layer::World], ..Default::default() };
    let mut polled = streams.poll(poll).take(2);
    while let Some(item) = polled.next().await {
        if let StreamItem::Frame(item) = item? {
            println!("polled g {} (cursor {})", item.frame.g, item.frame.cursor);
        }
    }
    Ok(())
}
```

(Against the fake unit, which shows its camera `cam-live` at the start, it prints `clock: cam-live at 3.7 s` and then
`frame g 16 at 3.7 s: 42 stones (revision: false)` and the next frames: the camera's frames after its warm-up.)

## Items

| `StreamItem` | What |
|---|---|
| `Frame(FrameItem)` | a frame with the layers asked for: `frame`, `is_revision`, `is_backfill`, `event_id` (`<boot>-<cursor>`) |
| `Packet(Sip1Packet)` | a raw packet (`raw_packets`) |
| `Clock(ClockEvent)` | the shared clock changed (play, pause, seek, select, rate): the new `State` |
| `Worker(WorkerStatus)` | the worker's state (`idle` when nobody watches) |
| `Gap(i64)` | the unit dropped that many frames for this client (it was too slow) |
| `Reset(ResetEvent)` | the unit restarted (another boot): cursors start again |

## `frames(options)`: SSE with the rules

`FramesOptions`:

| Field | Default | What |
|---|---|---|
| `layers` | the server's (`overlay`, `scene`, `cards`, `world`) | the layers to ask for |
| `after` | the newest frame | start after this cursor |
| `last_event_id` | none | start after this SSE event id (a `FrameItem`'s `event_id`), as a reconnect does |
| `observe` | the client's (`false`) | `Some(true)`: do not count as a viewer |
| `precision` | `rounded` | `full` for unrounded numbers |
| `resume` | `true` | reconnect after a dropped connection, with `Last-Event-ID` |
| `backfill` | `false` (`true` in the interpreter) | hold frames whose layers are late (`missing`) and read them back |
| `clock_on_connect` | `false` (`true` in the interpreter) | read the shared state after every connect and pass it on as a `Clock` item |
| `format` | SSE | NDJSON is the other wire format |
| `limit`, `max_s` | none | per connection: the SDK closes it after that many frames or seconds and, with `resume`, reconnects |

What the stream does for you:

- **Resume.** A dropped connection is reopened with `Last-Event-ID` (exponential backoff with full jitter, from
  250 ms to 5 s); the unit sends every frame it still holds after that id. After a `reset` (another boot) the stream
  starts again from the newest frame.
- **Finished connections.** The unit closes a connection 10 s late after `limit` frames or `max_s` seconds; the SDK
  counts them itself and closes the connection on time. With `resume` (the default) it reconnects at once, so the
  stream goes on; set `resume: false` to end it there.
- **Silent connections.** A connection that has sent something and then nothing for the client's stream idle
  timeout (30 s, `ClientBuilder::stream_idle_timeout`; `Duration::ZERO` turns it off) is closed and resumed: a
  half-open connection does not hang the stream. A stream whose reply headers do not come within the request
  timeout (10 s) counts as broken.
- **The state on connect.** The unit sends no `clock` event when a connection opens. With `clock_on_connect`, the
  stream reads `GET /session` after every connect (one request, at most 1 s) and passes it on as a `Clock` item
  before the connection's frames, so a reconnect across a session change keeps wall times right.
- **Backfill.** With `backfill`, a frame released without some of its layers (its `missing`, the world mostly) is
  held, with every frame after it; the stream reads those layers back with `get_frame` 1.5 s after the frame came, then 1 s apart, three tries at most, each one
  request cut off after 1 s. The reads run while the stream goes on. The frames are released in order, the
  backfilled one with `is_backfill`; a frame whose late layers did not all come is released with those that did
  merged in (`is_backfill` when one did), `missing` naming the rest.
- **Revisions and duplicates.** A frame repeated with a higher world `rev` (the paused frame being refined) comes
  with `is_revision`; one with the same or a lower revision is dropped.
- **Errors.** An event that does not decode (its data not JSON, or not the shape of its type: a field that spec 2.3
  rule 9 lists for it missing, null or of the wrong type, the same check in every SDK; further inside, the models
  read the data as they do) is yielded as an `Err` (code `decode`) and the stream goes on, and
  `Interpreter::wait_for` waits on past it (a `reset` that does not decode releases no held frame either); an
  error the stream cannot recover from (a 404, a refused request) is
  yielded last, after the frames it held. `fixtures/damaged` holds an event of each kind; `tests/damaged.rs` streams
  them.

## `poll(options)`: the long-poll

`PollOptions` (`layers`, `cursor`, `observe`, `cid`, `limit`, `wait_ms`, `precision`, `backfill`) drive `pollFrames`
in a loop: each reply's cursor is the next request's, and a reply that moved nothing waits the backoff's base before
the next. It suits networks that do not keep a connection open. A poll with a `cid` counts as a viewer: unless
`observe` is on, the poll sends the `cid` given or an id of its own (`rs-` and 16 random characters).

## `raw_packets(options)`: the model's packets

`rawStream`: one `Packet` per grid frame (the `sip1` packet: boxes, joints, stone tuples, packed maps). The codec
(`codec::decode_zlib_map`, `codec::joints`, `codec::stone_items`) reads them; see [low-level.md](low-level.md).

## The streams on the wire

`Api::stream_frames` and `Api::raw_stream` give the raw `EventStream` of SSE events (`sse::SseEvent`: `id`, `event`,
`data`) when you want to handle them yourself; `sse::SseParser` parses any SSE byte stream.
