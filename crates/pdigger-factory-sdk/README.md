# pdigger-factory-sdk

Use this Rust crate to read one report per tray from a pDigger Factory unit or a saved recording. A report lists
added, removed and moved stones. Use events and individual frames to investigate a change the SDK marks as
uncertain.

## Install

The core crate requires Rust 1.88 or later. Live calls use Tokio and reqwest with rustls.

```toml
[dependencies]
pdigger-factory-sdk = { git = "https://github.com/eprojac/pdigger-factory-sdk-rust", branch = "main" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
futures = "0.3"
```

The crate is not on crates.io: it comes from the repository at a release tag.

## Read a tray report

From the repository root, run the same example on a recording or on a live unit.

```sh
cargo run --example quickstart_reports -- fixtures/recordings/dry1-0018-0153.ndjson.gz
cargo run --example quickstart_reports -- --follow http://factory-orin.local:8120
```

[Quickstart source](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/crates/pdigger-factory-sdk/examples/quickstart_reports.rs)
·
[Read the sample report](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/tutorials/00-start-here.md)

## Choose a data source

- `Replay` reads a local recording synchronously. Use `ReplayStream` for an async stream from a path or URL.
- `FactoryClient` connects to a unit. Its interpreter provides events and activities. Its streams provide individual
  frames, and its low-level API exposes typed HTTP operations.
- Enable the `testing` feature to use `FakeFactory` in a test. For an egui viewer, use the
  [visualisation crate](https://github.com/eprojac/pdigger-factory-sdk-rust/tree/main/crates/pdigger-factory-viz).

[SDK guides and examples](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/README.md) ·
[Shared API specification](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/spec/openapi.yaml) ·
[Apache License 2.0](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/LICENSE)
