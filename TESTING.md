# Testing

```sh
make test
```

`make test` builds the [test image](Dockerfile.test) with Rust 1.100.0 and cargo-llvm-cov 0.9.1. The build fetches
and compiles the dependencies. The container then runs the [test script](scripts/test.sh) with networking disabled.
It checks the following in order.

1. `cmp` of `LICENSE`, `NOTICE` and `CHANGELOG.md` with each crate's copy in `crates/*/` (the published crates carry
   them; CI's lint job checks the same), and a run of
   [scripts/third-party-licenses.py](scripts/third-party-licenses.py), which writes the licences of the crates the
   viewer is built from (the release workflow puts them beside the viewer binary).
2. `cargo llvm-cov --workspace --all-targets --fail-under-lines 100 --fail-under-functions 100`: every unit and
   integration test of both crates, under coverage. It fails below 100 % of lines or functions; regions are reported
   too.
3. `cargo test --doc --workspace --all-features`: the examples in the doc comments.
4. [scripts/run-examples.sh](scripts/run-examples.sh): every example, over the fixture recordings and against the fake
   unit on 127.0.0.1 (`examples/fake_server.rs` replaying `dry1-0296-0306-full` as its camera), the headless
   screenshots, and the check that every Rust block of the READMEs and `docs/` is an example, word for word.

Tests run against fixtures and `pdigger_factory_sdk::testing`. They control time with Tokio’s paused clock or
`ManualClock` and use seeded randomness. Each real sleep is at most 50 ms.

## What the tests cover

| Suite | File | What |
|---|---|---|
| unit | `src/**` (`#[cfg(test)]`) | wire enums, error bodies, the codec, the SSE and NDJSON parsers, the Craft widgets, the rasteriser; the report view on the TypeScript SDK's tray 3 and on `dry1-0018-0153`'s tray 1 (marks, colours, layout, cameras, labels, places, painting) |
| API | `tests/api.rs` | the 32 operations against the recorded 20.20 replies: requests (paths, queries, headers, bodies), replies, errors, retries |
| streams | `tests/streams.rs` | resume with `Last-Event-ID`, backfill in order, revisions and duplicates, `limit` and `max_s`, side events, the long-poll, raw packets, the viewers an open stream counts, errors |
| session | `tests/session.rs` | sources, a live camera (its catalog entry, refusals, phases, a play that connects again after a break, `until_live`, `live_timeout`, the newest 64 pictures, a stream and a long poll through its warm-up, a stream that follows a new program, a linked clock), controls and dry runs, clock events, the session clock |
| interpreter | `tests/interpreter.rs` | the rules of section 4 of the contract, `wait_for`, state, summary, edge cases |
| trays | `tests/trays.rs` | trays and tray stones as values, ids without a clock, the live tray facts, the events' tray stones, the trays the core keeps |
| presence | `tests/trays_presence.rs` | the presence questions (SDK-DESIGN.md 4.6.1) beyond the golden files: every tray of the four presence recordings as its generator ends it, a stone held in question and sure again, settles at once on probation, stones with a look chain, busy frames from held and taken stones, revisions, the leaving's own frame, a hand on a stone since its last sight, the frames the rules can not read: the core's `gap()`, a replay's `gap` and `reset` lines (and those that do not decode), a live stream's `gap` and `reset` events, and a core of your own fed a recording's lines (`feed_line`), from a file and from a URL |
| replay | `tests/replay.rs` | recordings without a header, lines that are not JSON (sync and async), lines that do not decode as what they claim, readers that fail |
| damaged | `tests/damaged.rs` | `fixtures/damaged`, the same in every SDK: every line a reader skips and every event that does not decode (spec section 6 and 2.3 rule 9), replayed (sync and async), as headers, streamed live (frames, the interpreter's updates, `wait_for`) and as raw packets, with the frames, wall times and final update its JSON files give |
| fake | `tests/fake.rs` | the fake unit itself: routes, validation like the unit's (in its order, with its messages), the catalog, the camera's frame numbers, the status, faults, files for URL replays, the local HTTP server |
| golden | `tests/golden.rs` | every recording of `fixtures/recordings` (the folder is listed; nine excerpts and nine synthetic ones) through `run()`, the iterators, `Updates` and `ReplayStream`, against `fixtures/golden` byte for byte (events, each event's tray stone, activities, tray facts, open accounts, speckle answers, summaries); real-time pacing |
| viz | `crates/pdigger-factory-viz/tests/viz.rs` | the widgets headless: egui contexts fed with clicks, drags, scrolls and hovers on the carry of stone 643, the shapes painted and software-rendered frames; the jumps between tray points, by method and by key; the reports: the Reports button and the list of trays that left, the report view's worlds from the frames held, the trays of a recording URL (damaged lines and a gap line too) and of a unit followed |

## What coverage leaves out

The coverage run ignores (`--ignore-filename-regex 'crates/pdigger-factory-viz/src/(bin/|native\.rs)'`):

| File | Why |
|---|---|
| `crates/pdigger-factory-viz/src/native.rs` | opens a native window with `eframe::run_native`, which needs a display and a GPU context; marked in the source |
| `crates/pdigger-factory-viz/src/bin/pdigger-factory-viewer.rs` | the viewer's `main`: argument parsing and the call into `native::run`; marked in the source |

Everything the window shows is `ViewerApp`, which is tested headless. `tests/`, `examples/` and the doc tests are not
counted (cargo-llvm-cov's defaults); the examples are run by `scripts/run-examples.sh`.

## Opt-in integration tests

`crates/pdigger-factory-sdk/tests/integration.rs` holds the tests that need a unit or the complete recordings. They
are `#[ignore]`d, so `make test` never runs them, and each returns at once unless its variable is set:

In Docker, with nothing installed but Docker, `make integration` runs them in the test image (set one variable or
both; the recordings folder is mounted read-only):

```sh
PDIGGER_FACTORY_RECORDINGS=$PWD/recordings make integration
PDIGGER_FACTORY_URL=http://<unit>:8120 make integration
```

With a local toolchain:

```sh
# A unit, read-only: every GET operation, streams and long-polls that observe (observe=1: not a viewer),
# and every control as a dry run (dry_run=1). It never moves the shared session.
PDIGGER_FACTORY_URL=http://factory-orin.local:8120 \
  cargo test -p pdigger-factory-sdk --test integration -- --ignored --nocapture

# The complete recordings (<video>.world.ndjson.gz from https://factory.pdigger.com/developers/data/):
# each summary against <dir>/<video>.summary.json and .txt, else fixtures/golden/complete/.
PDIGGER_FACTORY_RECORDINGS=$HOME/recordings \
  cargo test -p pdigger-factory-sdk --release --test integration -- --ignored --nocapture
```

The `integration` workflow runs the recordings test every week and on demand, after downloading the published
recordings; with the repository variable `PDIGGER_FACTORY_URL` set (a unit its runner can reach), it checks that unit
too.
