# Contributing

Run `make test` for both crates, coverage, doc tests and examples. Run `make lint` for rustfmt and Clippy with
`-D warnings`. [Test instructions](TESTING.md).

## The contract comes first

All four SDKs follow the [same behaviour contract](spec/SDK-DESIGN.md) and [HTTP API](spec/openapi.yaml). The
maintainers sync the specification, fixtures, camera measurements and architecture diagram from the shared project.
Open an issue or pull request here to propose a change to these shared files.

The tests compare event, activity and summary text with `fixtures/golden/` byte for byte. Use the API’s
`operationId` names in Rust naming style. The contract also defines retries, stream recovery and interpreter counts.

## Code

- The SDK crate depends only on crates.io crates (tokio, reqwest with rustls, serde, futures, base64, flate2, bytes)
  and builds with Rust 1.88; the viz crate adds egui and eframe and needs Rust 1.95. CI and Docker use the toolchain
  of `rust-toolchain.toml`.
- Everything that talks to a unit is `async`; streams are `futures::Stream`s cancelled by dropping them; the
  interpreter's rules stay synchronous in `InterpreterCore`.
- One error type, `Error`, with a stable `ErrorCode`; keep the cause (`with_source`).
- Tests use the fake unit (`testing`), tokio's paused time or `ManualClock`, and seeded randomness: no network, no
  real sleeps over 50 ms, no timing-dependent assertions. New code comes with tests that keep line and function
  coverage at 100 %; the only exclusions are listed in [TESTING.md](TESTING.md).
- `#![forbid(unsafe_code)]` holds for both crates (`unsafe_code = "forbid"` in the workspace lints); every public item
  has a doc comment, which is the API reference.
- Files vendored from texturecraft and craft-libs keep their header (source, revision, copyright, SPDX licence, what
  changed); add any new one to [NOTICE](NOTICE) and copy `NOTICE` into `crates/*/`.

## Docs and examples

Every Rust code block in the READMEs and guides has an `<!-- example: <path> -->` marker and contains that complete
example file. Edit the example, run it, then copy it into the guide. `scripts/run-examples.sh` checks the two
copies. Use `make pictures` to regenerate the recording images. The architecture diagram comes from the shared
project.

## Commits, pull requests and releases

Keep each change focused. Explain what it changes and why. Add user-visible changes to the unreleased section of
`CHANGELOG.md` and copy it into both crates. The test suite compares the copies.

```sh
for crate in crates/*/; do cp CHANGELOG.md "$crate"; done
```

Follow the [release instructions](RELEASING.md) to publish both crates and the Linux, macOS and Windows viewer
binaries.
