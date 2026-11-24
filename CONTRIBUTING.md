# Contributing

Thank you for improving the SDK. A change is ready when `make test` passes: it runs every test of both crates with
the 100 % line and function coverage gate, the golden tests, the doc tests, every example and the check that the docs'
code blocks are the examples, in Docker, with no network. `make lint` runs rustfmt and clippy with `-D warnings`.

## The contract comes first

This SDK is one of four (Go, Python, Rust, TypeScript) that follow one contract,
[spec/SDK-DESIGN.md](spec/SDK-DESIGN.md), and the API document, [spec/openapi.yaml](spec/openapi.yaml). Both, and the
fixtures, are shared by the four languages and change in all four at once: do not edit `spec/`, `fixtures/` or
`docs/BEHAVIOUR-20.20.md` here; open an issue or a pull request here and the maintainers make the change in all four. In particular:

- the sentences of events, activities and the summary are golden-tested byte for byte against `fixtures/golden/`; a
  change to the text is a change to the contract;
- names follow the API's `operationId`s and the contract's words (source, session, frame, layer, world, event,
  activity), in Rust's snake case;
- the behaviour (what is retried, how streams resume, what the interpreter counts) is the contract's; the shape
  (builders, streams, errors) is Rust's.

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

Every Rust block of `README.md`, the crates' READMEs and `docs/` is a file of `crates/*/examples/`, marked
`<!-- example: <path> -->` above the block; `scripts/run-examples.sh` runs every example and fails when a block is
not that file word for word. To change a sample, change the example first, run it, then copy it into the docs. The
pictures in `docs/img/` are drawn by `make pictures` from the fixture recording (except `architecture.*`, the
picture shared by the four languages); never edit them by hand.

## Commits, pull requests and releases

Keep a change to one subject, say in the message what it changes and why, and add a line to `CHANGELOG.md` under a
heading `## Unreleased` for anything a user would notice, then copy it into both crates
(`for c in crates/*/; do cp CHANGELOG.md "$c"; done`): `make test` and CI compare the copies.

To release, follow [RELEASING.md](RELEASING.md): the changelog section, both versions in the workspace `Cargo.toml`,
`Cargo.lock`, `make test`, then a tag `v<version>`. The release workflow runs the tests again, publishes `pdigger-factory-sdk` and then
`pdigger-factory-viz` to crates.io (secret `CARGO_REGISTRY_TOKEN`), and attaches the viewer's binaries for Linux,
macOS and Windows to a GitHub release whose notes are that section of the changelog, in the order and with the
re-run rule of [RELEASING.md](RELEASING.md).
