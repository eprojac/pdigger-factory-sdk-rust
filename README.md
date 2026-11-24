# pDigger Factory SDK for Rust

[![CI](https://github.com/eprojac/pdigger-factory-sdk-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/eprojac/pdigger-factory-sdk-rust/actions/workflows/ci.yml)

Use Rust to read reports from a pDigger Factory camera unit or a saved recording. The `pdigger-factory-sdk` crate
handles live streams and recordings. The optional `pdigger-factory-viz` crate adds an egui viewer.

Simple mode gives you one report per tray. It lists the stones added, removed or moved and flags uncertain changes.
Advanced mode gives you the events, frames and tracking evidence behind each report.

## Install

The SDK requires Rust 1.88 or later and uses Tokio. Add these dependencies to your application.

```sh
cargo add --git https://github.com/eprojac/pdigger-factory-sdk-rust pdigger-factory-sdk
cargo add tokio --features macros,rt-multi-thread
cargo add futures
```

To run the examples, clone this repository and use its root as your working directory.

```sh
git clone https://github.com/eprojac/pdigger-factory-sdk-rust
cd pdigger-factory-sdk-rust
```

<a id="simple-mode-one-report-per-tray"></a>

## Read a tray report

The repository includes a short recording of a worker sorting stones on a tray. Run the report example on that file.

```sh
cargo run --example quickstart_reports -- fixtures/recordings/dry1-0018-0153.ndjson.gz
```

In Rust, `Replay::open(path)?.tray_reports()?` reads the completed reports from a local file. Call
`report.describe()` for the printed form or serialize the report with Serde.
[Complete example](crates/pdigger-factory-sdk/examples/quickstart_reports.rs).

The first two lines for `dry1-0018-0153` show the tray totals and the first removal.

```text
tray 1 (2026-11-22T02:38:47.0Z), 0:22.5 to 2:24.0: 11 removed, 13 added, 8 moved, not complete: 18 unclear, 1 not sure
  0:45.5 removed 20261122T023847.0Z-1-117 from (0.68, -1.06) (0.98)
```

Of the 11 removals, the SDK marked six as certain and five as unclear. The number at the end of each change is its
confidence score. [Walk through this report](docs/tutorials/00-start-here.md).

For a complete recording, download the file and pass its path to the same example.

```sh
curl -fLO https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz
cargo run --example quickstart_reports -- dry1.world.ndjson.gz
```

## Follow a live unit

Use the same example with your unit’s address to print a report as each tray leaves the camera.

```sh
cargo run --example quickstart_reports -- --follow http://factory-orin.local:8120
```

For live reports, consume the stream returned by `factory.interpreter().tray_reports()`. Each item contains a report
or an SDK error.

<a id="advanced-mode-follow-the-unit"></a>
<a id="advanced-mode-replay-a-recording"></a>

## Read individual events

The interpreter can also tell you when a hand lifts a stone, which stones it pushes and where it places them. The
live example prints events from whichever source the unit is watching.

```sh
cargo run --example quickstart -- http://factory-orin.local:8120
```

<details>
<summary>Rust event loop</summary>

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

</details>

To inspect events from a recording, pass its URL to the replay example.

```sh
cargo run --example replay_url -- https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz
```

[Live event example](crates/pdigger-factory-sdk/examples/quickstart.rs) ·
[Replay example](crates/pdigger-factory-sdk/examples/replay_url.rs)

![Replay of dry1-0296-0306-full, frames 1480–1530](docs/img/quickstart-output.svg)

Replay of dry1-0296-0306-full, frames 1480–1530.

<a id="advanced-mode-in-three-layers"></a>
<a id="high-level-the-interpreter"></a>
<a id="mid-level-streams"></a>
<a id="low-level-one-method-per-operation"></a>

## Choose an API

| Task | SDK entry point | Example | Guide |
| --- | --- | --- | --- |
| Follow hand movements and stone events | `factory.interpreter().activities()` | [Source](crates/pdigger-factory-sdk/examples/interpreter.rs) | [Interpreter](docs/interpreter.md) |
| Read individual frames and late layers | `factory.streams().frames` | [Source](crates/pdigger-factory-sdk/examples/streams.rs) | [Streams](docs/streams.md) |
| Request a frame, map or draw list | `factory.api().get_frame` | [Source](crates/pdigger-factory-sdk/examples/low_level.rs) | [HTTP API](docs/low-level.md) |

<a id="trays-ids-never-reused-facts-never-revised"></a>

Each tray stone has a stable ID, even if the camera tracker loses it and assigns another ID.
[How the SDK tracks tray stones](docs/interpreter.md#trays-and-tray-stones).

<a id="live-cameras-and-the-shared-session"></a>

Selecting a camera or seeking a recording changes the source or playhead for everyone watching that unit.
[Live camera tutorial](docs/tutorials/03-live-cameras.md).

## Errors

Use `pdigger_factory_sdk::Error` to handle SDK failures. Branch on the stable error code when deciding whether to
retry or choose another frame. [Error handling and retries](docs/low-level.md).

## Async

Share a `FactoryClient` between Tokio tasks by cloning it. Live streams yield `Result` items. Drop a stream to close
its connection. `Replay` reads files synchronously, and `ReplayStream` supports async reads and URLs.

## Testing your code

Enable the `testing` feature to use `FakeFactory` in your tests. It serves a recording in process or through a
loopback HTTP server. [Testing guide](docs/testing.md).

## Visualisation

Install the egui viewer with Rust 1.95 or later.

```sh
cargo install --locked --git https://github.com/eprojac/pdigger-factory-sdk-rust pdigger-factory-viz
pdigger-factory-viewer fixtures/recordings/dry1-0018-0153.ndjson.gz
```

Use the playback controls to compare the tray at arrival and departure. [Viewer and widget guide](docs/viz.md).

![Viewer on dry1-0296-0306-full at frame 1516, with stone 643 in the right hand](docs/img/viewer-dry1-0296-0306-full-g1516.png)

## Docs

- [Work through the tutorials](docs/tutorials/README.md)
- [Understand sessions, coordinates and frame layers](docs/concepts.md)
- [Open and replay recordings](docs/replay.md)
- [Look up camera timings on the 20.20 unit](docs/BEHAVIOUR-20.20.md)
- [Browse the HTTP API](https://factory.pdigger.com/developers/api/) · [OpenAPI specification](spec/openapi.yaml)

Build the Rust API reference with `make docs` from the repository root.

![How camera frames reach an application through the factory unit and SDK](docs/img/architecture.png)

## The other languages

All four SDKs produce the same report text for a given recording. [Shared SDK contract](spec/SDK-DESIGN.md).

| SDK | Reports, events, streams and replay | Viewer |
| --- | --- | --- |
| [Go](https://github.com/eprojac/pdigger-factory-sdk-go) | Yes | — |
| [Python](https://github.com/eprojac/pdigger-factory-sdk-python) | Yes | — |
| [Rust](https://github.com/eprojac/pdigger-factory-sdk-rust) | Yes | egui |
| [TypeScript](https://github.com/eprojac/pdigger-factory-sdk-ts) | Yes | three.js |

## Development

Run `make test` from the repository root to check the SDK in Docker. [Test instructions](TESTING.md) ·
[Contributing](CONTRIBUTING.md) · [Release instructions](RELEASING.md)

## Licence

[Apache License 2.0](LICENSE) · [Third-party notices](NOTICE)
