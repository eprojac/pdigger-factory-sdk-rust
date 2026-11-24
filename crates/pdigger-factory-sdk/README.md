# pdigger-factory-sdk

Our SDK for the pDigger Factory interpreter API (v2), in Rust. A pDigger Factory unit watches a work area through one
camera and says which hand moved which stone, when, and where it went. This crate reads that record in two modes, live
from a unit or offline from a recording: simple mode, one report per tray (which stones were taken off, which were put
on and which moved, when and where on the tray, and whether that is the tray's whole account); and advanced mode, the
detail underneath, for research and for correcting a report that is not complete: typed events, activities and plain
sentences (`5:01.6 Right hand lifts stone 643`), every tray fact with its reason, down to every model output. The SDK
comes in four languages (Go, Python, Rust, TypeScript), which print the same text for the same data.

```toml
[dependencies]
pdigger-factory-sdk = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
futures = "0.3"
```

Until the first crates.io release, take it from the repository: `cargo add --git https://github.com/eprojac/pdigger-factory-sdk-rust pdigger-factory-sdk`.

<!-- example: crates/pdigger-factory-sdk/examples/quickstart_reports.rs -->
```rust
//! Simple mode: one report per tray, which stones were removed, which were added and which
//! moved, when and where, and whether that is the tray's whole account. A recording needs no unit; with
//! `--follow` the same report comes from a unit as each tray leaves.
//!
//!     cargo run --example quickstart_reports -- fixtures/recordings/dry1-0018-0153.ndjson.gz
//!     cargo run --example quickstart_reports -- --follow http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::{FactoryClient, Replay};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let mut args = std::env::args().skip(1);
    let source = args.next().unwrap_or_else(|| "fixtures/recordings/dry1-0018-0153.ndjson.gz".to_string());
    if source != "--follow" {
        for report in Replay::open(&source)?.tray_reports()? {
            println!("{}", report.describe());
        }
        return Ok(());
    }
    let url = args.next().unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    let mut reports = factory.interpreter().tray_reports(); // one as each tray leaves
    while let Some(report) = reports.next().await {
        println!("{}", report?.describe()); // serde_json::to_string(&report) is the same as data
    }
    Ok(())
}
```

The detail underneath, as the unit's videos say it:

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

The same over a recording, offline and synchronous:

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

- **Reports:** `TrayReport`, one per tray, from `Replay::tray_reports` or `Interpreter::tray_reports` as each tray
  leaves: the stones removed, added and moved, when and where, and whether that is the tray's whole account, as far as the world
  sees ([trays and tray stones](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/interpreter.md#trays-and-tray-stones)).
- **High level:** `Interpreter` (events, moves, push bursts, hands, stones, tray facts, summary, `wait_for`), `Replay` and
  `ReplayStream`, `Session` (the shared session's controls, with dry runs, and its clock), sources and the live cameras.
- **Mid level:** `Streams`: frames over SSE that resume with `Last-Event-ID`, backfill late layers and flag
  revisions; the long-poll; raw packets.
- **Low level:** `Api`, one async method per API operation (32), typed `models`, one `Error` type with stable codes,
  retries; the `codec`.
- **Testing** (feature `testing`): `FakeFactory`, a fake unit that answers the whole API from a recording, in process
  or on 127.0.0.1, with fault injection.

Rust 1.88 or later; tokio and reqwest with rustls. Try the API in your browser at
https://factory.pdigger.com/developers/api/ and download the recordings at https://factory.pdigger.com/developers/data/.
The guides, examples and the egui viewer (`pdigger-factory-viz`) are in the repository:
https://github.com/eprojac/pdigger-factory-sdk-rust.

Licensed under Apache-2.0.
