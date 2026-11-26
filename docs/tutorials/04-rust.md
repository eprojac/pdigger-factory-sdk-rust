# Rust API conventions

<a id="rust-what-is-particular-to-this-sdk"></a>

Choose between synchronous file replay and Tokio streams, handle SDK errors, and run tests against the fake unit.
The optional egui crate adds a viewer.

## Two crates

`pdigger-factory-sdk` is the SDK, async on tokio, with HTTP over reqwest and rustls (no OpenSSL); it needs Rust 1.88
or newer. `pdigger-factory-viz` draws the SDK's data with egui: the widgets and the `pdigger-factory-viewer` app; it
needs Rust 1.95 (egui 0.37).

## tokio, streams, and synchronous replays

Every call that talks to the unit is an `async fn`, and a `FactoryClient` is cheap to clone and share between tasks.
Streams are `futures::Stream`s of `Result<T>`: dropping one cancels it and closes its HTTP connection, as do
`tokio::time::timeout` and `select!` around it. A stream resumes by itself after a dropped connection or a restart of
the unit, yields an error for an item that does not decode and goes on, and ends after an error it cannot recover from.

`Replay`'s `frames()`, `updates()`, `events()`, `activities()` and `tray_facts()` are synchronous iterators, with no
runtime needed (the [high-level tutorial](01-high-level.md) uses them), and `ReplayStream` is the async variant,
streamed from a file or a URL and paced in real time if you like. The interpreter's rules live in `InterpreterCore`,
which is synchronous and does no I/O: feed it frames from anywhere.

## One error type

Every error is a `pdigger_factory_sdk::Error`, with the HTTP status, a stable `code` (`ErrorCode`) to branch on, the
server's message, the field at fault and the server's `retry_ms`. Reads are retried by themselves on connection errors
and on 429, 502, 503 and 504, with exponential backoff and full jitter; a control only on 429, which the unit sends
before it applies anything. [examples/errors.rs](../../crates/pdigger-factory-sdk/examples/errors.rs) branches on the
codes, and [low-level.md](../low-level.md) lists them.

## The viewer

`pdigger-factory-viewer` opens a recording file, a recording URL or a unit, and shows the video-space overlay, the 3-D
scene, the hand cards and the event feed; `[` and `]` jump to the previous and next tray point, and the Reports
button shows the reports of the trays that left. Its widgets (`OverlayView`, `SceneView`, `EventFeed`, `HandCards`,
`ReportView`) go into your own egui app, and
[examples/screenshots.rs](../../crates/pdigger-factory-viz/examples/screenshots.rs) renders them without a window
([viz.md](../viz.md)). Install the viewer from this repository at the release tag:
`cargo install --locked --git https://github.com/eprojac/pdigger-factory-sdk-rust --tag v1.0.1 pdigger-factory-viz`.

## The fake unit in your tests

The feature `testing` adds `FakeFactory`, a fake unit that answers the whole API from a recording and the recorded
replies, in process or on a loopback port, with fault injection and a manual clock, so the code you build on the SDK
is tested without a unit, a network or a real clock:

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

`with_live()` adds the camera `cam-live`, and
[examples/fake_server.rs](../../crates/pdigger-factory-sdk/examples/fake_server.rs) serves the fake on 127.0.0.1 for
tools that need an address, such as these tutorials' programs and the viewer ([testing.md](../testing.md)).

## Types

The models are plain structs with serde, and every one keeps the fields this version does not know in `extra`, so a
newer unit's fields are never lost; the enums of the wire format have an `Other(String)` for values a newer unit
sends. Integers are `i64`, and the `codec` returns `Grid` maps, `Joint`s and `StoneItem`s.
