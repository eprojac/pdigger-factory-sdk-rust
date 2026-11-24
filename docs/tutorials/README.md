# Tutorials

Five short tutorials take you from simple mode, one report per tray, through advanced mode to drawing a frame by hand.
Every program in them is an example of this repository, and the code blocks are those files, whole:
`scripts/run-examples.sh` runs each one against the fixture recordings and the SDK's fake unit, and checks that the
pages still show them as they are.

| Tutorial | What you build |
| --- | --- |
| [Start here](00-start-here.md) | simple mode: one report per tray (the stones removed, added and moved, when and where, and whether that is all of it), the same reports as data, the details behind a report that is not complete, and the reports of a unit, live |
| [High level](01-high-level.md) | advanced mode: what the hands did, as sentences and as data: activities, trays and the stones taken off them, the summary, a stone's path; replayed offline, then followed live |
| [Low level](02-low-level.md) | advanced mode: one call per API operation: health and speed, the model, a stone and the focus, frames and the model's maps, draw lists, stones and hands drawn by hand, a frame's image data |
| [Live cameras](03-live-cameras.md) | advanced mode: a plant camera selected, warmed up and followed, its pictures, and what differs on a camera |
| [Rust](04-rust.md) | what is particular to this SDK: tokio and streams, synchronous replays, errors, the egui viewer, the fake unit in your tests |

## Before you start

- Rust 1.88 or newer. Until the crates are on crates.io, take the SDK from this repository:

  ```sh
  cargo add --git https://github.com/eprojac/pdigger-factory-sdk-rust pdigger-factory-sdk
  cargo add tokio --features macros,rt-multi-thread
  cargo add futures
  ```

- To run the tutorials' programs as they are, clone the repository and run them there:
  `cargo run --example tutorial_reports`.
- For the first two tutorials, nothing else: they replay recordings, offline, such as the fixture recordings in
  `fixtures/recordings/` or the published ones at https://factory.pdigger.com/developers/data/.
- For the live steps, a unit's address as the first argument (the examples ask http://factory-orin.local:8120 by
  default). Without a unit, the SDK's fake one serves a recording on 127.0.0.1 and prints its address:
  `cargo run --example fake_server --features testing -- fixtures/recordings/dry1-0296-0306-full.ndjson.gz`
  (add `--live` for a camera made from the recording).

The unit interprets one source at a time for everybody: a recording or a live camera, on one shared session. A
control (select, seek, play, pause, focus) changes what every viewer sees, so the tutorials send dry runs, which the
unit checks without applying them; only the live-camera tutorial really selects a camera.

The same tutorials, step for step, are in the other languages:
[Go](https://github.com/eprojac/pdigger-factory-sdk-go/tree/main/docs/tutorials),
[Python](https://github.com/eprojac/pdigger-factory-sdk-python/tree/main/docs/tutorials) and
[TypeScript](https://github.com/eprojac/pdigger-factory-sdk-ts/tree/main/docs/tutorials).
