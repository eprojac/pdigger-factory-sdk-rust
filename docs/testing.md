# Testing

Enable the `testing` feature to use `FakeFactory` in your tests. It serves a recording in process or through a
loopback HTTP server. See [the SDK test suite](../TESTING.md) for coverage and integration checks.

## The fake unit

```toml
[dev-dependencies]
pdigger-factory-sdk = { git = "https://github.com/eprojac/pdigger-factory-sdk-rust", features = ["testing"] }
tokio = { version = "1", features = ["macros", "rt", "test-util"] }
```

Until the first release the crate comes from the repository; from then on,
`pdigger-factory-sdk = { version = "0.1", features = ["testing"] }`.

`testing::FakeFactory` answers the whole v2 API from a recording and/or recorded replies, with no network and no real
clock. Its client talks to it in process (`fake.client()`: the fake is the client's `Transport`), or over a real
socket on 127.0.0.1 (`fake.serve().await` gives a `FakeServer` with `url()` and `client()`).

<!-- example: crates/pdigger-factory-sdk/examples/testing_quickstart.rs -->
```rust
//! Tests your own code against the SDK's fake unit: it answers the API from a recording, in
//! process (no socket, no unit), and injects faults.
//!
//!     cargo run --example testing_quickstart --features testing
use std::time::Duration;

use pdigger_factory_sdk::models::EventKind;
use pdigger_factory_sdk::testing::{FakeFactory, Fault};

#[tokio::main(flavor = "current_thread")]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let fake = FakeFactory::from_recording("fixtures/recordings/dry1-0296-0306-full.ndjson.gz")?;
    let factory = fake.client();
    let lift = factory.interpreter().wait_for(EventKind::Lift, Duration::from_secs(5)).await?.expect("a lift");
    assert_eq!(lift.describe(), "5:01.6 Right hand lifts stone 643");
    println!("{}", lift.describe());
    fake.fail("getSession", Fault::status(503, "busy").times(1)); // one 503, then the retry succeeds
    let state = factory.session().state().await?;
    println!("session {} on {} after {} requests", state.session, state.video, fake.operations().len());
    Ok(())
}
```

What it serves:

- **A recording** (`FakeFactory::from_recording(path)`, `from_recording_bytes`, `from_values`): its frames as the
  current session's, through every stream (SSE with ids and `Last-Event-ID`, NDJSON, the long-poll), `get_frame`,
  `get_world`, the world's events and stones, the clock and session replies. The catalog lists the header's `source`
  (else an entry made from the first frame's video, as the unit lists a recording); the session state and the fake's
  clock start where the recording's first clock line is, and each later clock line is the state from when a stream or
  the long poll passes it (a long-poll reply ends before the line, the next reply carries it) until a control takes
  over. A stream without `after` starts at the first frame of the current program and follows the program while it
  is open: a select or a seek starts it again at the new program's first frame, and every change of the state is a
  `clock` event.
- **The recorded replies** (`with_replies("fixtures/api")`: the real 20.20 replies of every GET operation and real
  error replies) for what the recording cannot answer (the model, topology, status, metrics, renders, rasters,
  tensors; without them the fake makes up the health, the readiness and the status), and `with_reply(key, Reply)` for
  your own (a `listVideos` given so is the catalog, sent as it is).
- **A live camera** (`with_live()`): the catalog lists `cam-live` (`LIVE_CAMERA`) after the recording, as the 20.20
  unit lists its plant cameras, and the session shows it at the start, already live. The camera sends the recording's
  frames as a camera session does: `g` from 0 (`i` the same, and the raw packet's `g`, `i`, `t` and `cursor` the
  frame's), `t` the arrival in seconds since the session started (from `CONNECT_S`, 0.5), its sessions end to end. A
  select of it starts a session as the unit (build live3) does: `connecting` for `CONNECT_S`, then `warming` while the
  interpreter runs on the first `hold` (3) seconds of frames, which are never released, then `live` (`LIVE_LAG_S`,
  0.25, behind the camera), each phase a new `revision` and a `clock` event. The phases follow the fake's clock while
  the session plays; nothing runs while it is paused (a pause keeps the phase). Play goes back to `connecting` in its
  own reply and plans the phases again from then (warming changes only the phase, as on the unit: its `lag` stays the
  live one, 0 when the camera was not live yet); on a camera that was live, play goes on at the live edge
  (`now - since - lag`), and the camera's next frames come after a break (`g` jumps by 5) and after the frames it sent
  already (the fake's camera sends its whole run at once when it goes live), and their warm-up is never released. On
  the fake's own clock, a stream that waits for the next phase moves the clock on to it, and so does a long poll for
  the phases within its `wait` (and while a phase is to come, one with nothing new for a client of the current
  revision moves it to the end of its `wait`, as the unit's waits). `Session::until_live` waits on the client's clock,
  which for `fake.client()` is the fake's `ManualClock`, so the wait moves it on at once. The newest `KEEP_PICTURES`
  (64) released frames have a picture (`frames/{g}/image.jpg`: a small grey JPEG with `FakeFactory <video> g <g>` in a
  comment segment); an older frame's is `404 not_retained`, without a `retained` window, as on the unit. While no
  frame is released, `latest` is `404 not_retained` "no frame is retained yet" and the world `503 world_unavailable`
  (`retry_ms` 1000). The camera has no media (404 `not_found`), a seek in it is refused (409 `not_seekable`, field
  `video`) and so is a select of it with any time, null too (400 `not_seekable`, field `time`); a recording selected
  after the camera plays at 0.8.
- **Controls**, checked like the unit checks them, in its order and with its messages (the body, `dry_run`, which is
  `0` or `1`, then the video, a camera, the time: `"time": null` is a time of the wrong type), and applied to the
  fake's state: a select or a seek starts a new session that replays the recording from that time, its id one higher
  than the newest so far; a seek naming a recording is applied, and leaves the camera, as on the unit. `fake.state()`
  is the shared state.
- **Files** (`with_file(path, bytes, chunk)`): bytes at a URL path, served in chunks, e.g. a recording for
  `ReplayStream::open_url`.

Unlike the unit, the fake interprets whether anyone watches or not, and its camera never stops: a session goes back
to `connecting` only on a play. Time is not simulated: a long poll answers at once with the next frames (up to the
recording's next clock line), and a stream ends when nothing more is due (the unit's stays open). The viewers it
counts (`get_health`, `get_status`) are the open streams without `observe=1`; the unit also counts a long poll with
a `cid`. Over `serve()` a reply with a body is chunked, without a `Content-Length`.

Faults, for the paths your code must survive:

| Method | Fault |
|---|---|
| `fail(operation, Fault::status(503, "busy").times(2))` | error replies for the next requests of an operation (`retry_ms`, `retry_after`) |
| `fail(operation, Fault::broken())` | no reply at all (a transport error) |
| `fail(operation, Fault::reply(Reply::new(...)))` | any reply |
| `drop_after("streamFrames", n)` | the next stream breaks after `n` events, as a dropped connection does |
| `inject("streamFrames", "reset", data, after)` | a side event (`clock`, `worker`, `gap`, `reset`) after some events |
| `set_delay(\|operation, index\| ...)` | a delay before each event of a stream or chunk of a file |
| `set_close_delay(duration)` | the unit's late close of a finished stream |

`fake.requests()` and `fake.operations()` record what your code asked for; `fake.streams_opened()` counts the
streams. `fake.clock()` is a `ManualClock` (`set`, `advance`) shared with the fake's client; with tokio's paused time
(`#[tokio::test(start_paused = true)]`) retries and backoff take no real time. `with_linked_clock()` links the fake's
clock to its owner: streams and long polls never move it, only the client's sleeps, `set` and `advance` do, and a
stream that waits for a camera's next phase looks again every 10 ms (tokio time) until the clock has passed it. A
client on another clock (over `serve()`) therefore needs a clock that runs, as in `examples/fake_server.rs`.

To try programs and the viewer against it from outside a test, `examples/fake_server.rs` serves a recording on
127.0.0.1 and prints its URL; its clock is linked and runs in real time, so a camera selected through it connects,
warms up and goes live as on a unit:

<!-- example: crates/pdigger-factory-sdk/examples/fake_server.rs -->
```rust
//! Serves a recording as a unit on 127.0.0.1 (the SDK's fake server, feature `testing`), to try
//! the examples and the viewer without a unit. Prints its URL, then serves until stopped.
//! `--live` offers a live camera, `cam-live` (the recording replayed as the camera, shown at the
//! start); the recording is also served at `/developers/data/<file>` for `replay_url`. The fake's
//! clock follows this machine's, and only this machine's (linked: the fake never moves it on to a
//! camera's next phase), so that a camera selected through it connects, warms up and goes live in
//! real time, as on a unit.
//!
//!     cargo run --example fake_server --features testing -- fixtures/recordings/dry1-0296-0306-full.ndjson.gz --live
use std::path::Path;
use std::time::{Duration, Instant};

use pdigger_factory_sdk::Clock;
use pdigger_factory_sdk::testing::FakeFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let default = "fixtures/recordings/dry1-0296-0306-full.ndjson.gz".to_string();
    let path = args.iter().find(|a| !a.starts_with("--")).unwrap_or(&default);
    let bytes = std::fs::read(path)?;
    let name = Path::new(path).file_name().map_or("recording.ndjson.gz".into(), |n| n.to_string_lossy().into_owned());
    let replies =
        Path::new(path).parent().and_then(Path::parent).map(|fixtures| fixtures.join("api")).filter(|api| api.is_dir());
    let mut fake =
        FakeFactory::from_recording_bytes(&bytes)?.with_file(&format!("/developers/data/{name}"), bytes, 64 * 1024);
    if let Some(api) = replies {
        fake = fake.with_replies(api)?; // the recorded replies answer what the recording cannot
    }
    if args.iter().any(|a| a == "--live") {
        fake = fake.with_live();
    }
    let fake = fake.with_linked_clock(); // a stream waits for the camera's phases in real time
    let clock = fake.clock();
    let (started, epoch) = (Instant::now(), clock.now());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            clock.set(epoch + started.elapsed().as_secs_f64()); // the fake's clock runs in real time
        }
    });
    let server = fake.serve().await?;
    println!("{}", server.url());
    std::future::pending::<()>().await;
    Ok(())
}
```

## How this SDK is tested

- `make test` builds `Dockerfile.test` (Rust 1.100.0, cargo-llvm-cov 0.9.1, every dependency fetched and compiled at
  build time) and runs it with `--network none`: every unit and integration test of both crates, the golden tests,
  the 100 % line and function coverage gate (`cargo llvm-cov --fail-under-lines 100 --fail-under-functions 100`), the
  doc tests, then every example (over the fixtures and against the fake unit on 127.0.0.1) and the check that every
  Rust block of the docs is an example, word for word.
- **Golden tests** (`crates/pdigger-factory-sdk/tests/golden.rs`): a replay of every recording in `fixtures/recordings`
  (the folder is listed, so a new recording is picked up) must give `fixtures/golden/<name>.events.txt`,
  `.tray-stones.txt`, `.activities.txt`, `.trays.txt`, `.open.txt`, `.speckle.txt` and `.summary.txt` byte for
  byte and `.summary.json` after parsing, through `run()`, the iterators, `Updates` and the async `ReplayStream`; the
  tray facts also from the trays `run()` keeps. Eighteen recordings: nine excerpts and nine synthetic ones that say so
  in their headers (`revisions-synthetic`, `trays-synthetic` for the tray rules the real excerpts never reach,
  `trays-looks-synthetic` with its copy without looks, `trays-looks-synthetic-nolooks`, the four of the presence
  questions, `trays-presence-synthetic`, `trays-presence-looks-synthetic`, `trays-presence-attacks-synthetic` and
  `trays-presence-attacks-looks-synthetic`, and `trays-speckle-synthetic` for the unit's speckle answers (SDK-DESIGN
  4.6.12) and the tray rules' use of the leaving's answer (4.6.13)).
- **Damaged input** (`crates/pdigger-factory-sdk/tests/damaged.rs`): `fixtures/damaged`, the same in every SDK, with
  every line a reader skips and every event that does not decode (spec section 6 and 2.3 rule 9).
- Tests talk to the fake unit or to fixture data only. Time is tokio's paused clock or a `ManualClock`, randomness is
  seeded: no real sleeps over 50 ms, no timing-dependent assertions.
- The viz crate's widgets are tested headless: egui contexts fed with input events (clicks, drags, scrolls, hovers),
  the shapes they paint and frames rasterised in software.
- What coverage leaves out, and the opt-in tests against a unit and over the complete recordings:
  [TESTING.md](../TESTING.md).
