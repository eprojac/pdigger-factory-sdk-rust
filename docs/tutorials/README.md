# Tutorials

Start with a tray report. The later tutorials show how to read the events and frames behind it.

| Tutorial | What you’ll do |
| --- | --- |
| [Read a tray report](00-start-here.md) | Read the changes, export a row and investigate uncertainty. |
| [Follow the interpreter](01-high-level.md) | Follow hand movements, get tray facts and trace a stone. |
| [Read individual frames](02-low-level.md) | Inspect model maps, draw stones and hands, and read a frame image. |
| [Watch a live camera](03-live-cameras.md) | Select a camera, wait for warm-up and read its pictures. |
| [Use the Rust SDK](04-rust.md) | Use Tokio streams, synchronous file replay and the viewer. |

## Before you start

Follow the [installation steps](../../README.md#install) to get the SDK and its example recordings. Run the commands
below from the repository root.

The replay steps use the bundled files in `fixtures/recordings/`. You can also
[download longer recordings](https://factory.pdigger.com/developers/data/).

For a live step, pass the unit’s base URL as the example’s argument. To use a fake unit, run
`cargo run --example fake_server --features testing -- fixtures/recordings/dry1-0296-0306-full.ndjson.gz` in a
second terminal and use the address it prints. Add `--live` to simulate a camera.

A unit has one shared session. Selecting a camera, seeking or changing playback affects everyone watching. The
live-camera tutorial selects a real source. The other control examples use dry runs.

The same tutorials are available in
[Python](https://github.com/eprojac/pdigger-factory-sdk-python/tree/main/docs/tutorials) ·
[Go](https://github.com/eprojac/pdigger-factory-sdk-go/tree/main/docs/tutorials) ·
[TypeScript](https://github.com/eprojac/pdigger-factory-sdk-ts/tree/main/docs/tutorials).
