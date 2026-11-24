# Changelog

This SDK follows [semantic versioning](https://semver.org/). The release workflow uses the matching version section
as its release notes.

## 0.1.0 - Unreleased

Initial SDK for API v2.2. The reference unit ran build `claude-20261123-speckle7` with four recordings and six plant
cameras. It supplies stone appearance data and surface-pattern comparisons.

### Tray reports and tracking

- Read one `TrayReport` for each completed tray, live or from a recording. Each change carries a stone ID, a time, a
  position and a confidence score. Use `serde_json::to_string(&report)` for the complete data.
  [Report fields](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/interpreter.md#report-fields).
  Inspect `tray.open` for unresolved identities and use reason codes to investigate individual facts.
- Read the unit’s `world.speckle` comparisons (build `claude-20261123-speckle3` onwards). `TraySpeckle` groups
  matches by tray. `StoneSpeckle` gives a stone’s best and second-best scores, match quality and pairing. The
  interpreter keeps the latest 1,024 comparisons.
  [Speckle fields and examples](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/interpreter.md#the-units-speckle-check).
- Version 8 of the tray rules uses the departure comparison to confirm or recover stone IDs. It records
  `speckle_same`, `speckle_moved` or `speckle_found` as the reason. The evidence can be found, absent or undecided.
  Each change’s confidence also appears in the printed report.
  [Confidence scores](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/interpreter.md#confidence).
- If a stone disappears at two calm moments, the SDK can hold its identity open. It restores the identity if the
  stone returns in place and the intervening frames rule out a swap. Hand activity, competing matches, missing
  frames or a stream reset can end the question. Stones with an appearance history follow their own matching rules.
  [Presence and identity rules](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/spec/SDK-DESIGN.md#461-the-tray-rules).
- A certain removal requires evidence of recent hand contact. Otherwise a lost stone stays available for matching or
  becomes an unclear removal at departure. A visit continues the same tray only if at least 90% of its initial
  stones were already seen on it. Across the four recordings, the SDK found 628 trays and produced 532 reports. A
  video check confirmed 49 of 52 certain removals.
- At departure, the SDK uses appearance, timing and nearby hand activity to decide whether a new tracker ID belongs
  to a lost stone. If the evidence is ambiguous, it records an unclear removal and addition and lists each as a
  candidate for the other. The departure rules also allow some previously tracked stones that have no bounding box
  at departure. In a video check, 32 of the 63 extra stones counted this way were present. Another 25 were false
  detections on empty jul30 boards.
  [Departure rules](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/spec/SDK-DESIGN.md#461-the-tray-rules).

### Client, replay and streams

- **High level.** `Interpreter`: follows the shared session (a recording or a plant camera) and yields typed events,
  activities (moves and push bursts), hand states, the stone inventory and a summary, each with the sentence the four
  languages share; trays with tray stones, stone ids of the SDK's own that are never reused (a second stone first seen
  on a tray in the same tenth of a second under one world id gets `-2` at the end, a third `-3`) and hold whatever ids
  the world gives the stone, certain and unclear facts about them (`tray_facts`), and each tray's arrival and leaving
  points to jump between (`trays`, `Tray::window`); `wait_for` and `wait_for_match`. `Replay` (a synchronous iterator
  over a recording file) and `ReplayStream` (async, from a file or a URL, with real-time pacing). Sources and the live
  cameras (`FactoryClient::sources`, `cameras`, `live`; a `Source` with `hold` and `station`); watching a camera:
  `Session::watch` (a time for a camera refused locally), `Session::until_live` through its `connecting` and `warming`
  phases (`live_timeout`), and `Session::picture`, the JPEG a live frame was interpreted from; the shared session's
  controls with dry runs, and the session clock. A later loop over an `Interpreter` goes on where the last one stopped
  (the rest of its frame first); a stream the server ends (`resume(false)`) ends with the final update (`is_final`), as
  a replay does; `Interpreter::track` gives a stone's path. `InterpreterCore`: the rules, synchronous and without I/O.
- **Mid level.** `Streams`: frames over SSE (or NDJSON) that resume with `Last-Event-ID`, close finished connections
  themselves (`limit`, `max_s`), backfill late worlds in order (reads 1.5, 2.5 and 3.5 s after the frame arrived,
  each one request of at most 1 s, beside the stream), flag revisions and drop duplicates, read the shared state after
  every connect for the interpreter, end a silent connection after 30 s (`ClientBuilder::stream_idle_timeout`) and
  bound lines and events (64 MiB); the long-poll (with a client id unless observing); raw packets; side events
  (clock, worker, gap, reset).
- **Low level.** `Api`: one async method for each of the 32 operations of the OpenAPI document (`get_frame_picture`
  gives a `Picture`: the frame's `g` and the JPEG), typed models of every
  schema (unknown fields and enum values kept), one `Error` type with stable codes, retries with backoff and full
  jitter; the `codec` module (maps, bitsets, depth, surfaces, joints, stone tuples, coordinates, skeleton).
- **Damaged input.** A stream event that does not decode (the fields of spec 2.3 rule 9 missing, null or of the wrong
  type) is yielded as an `Err` (code `decode`) and the stream goes on (its id still counts, an interpreter loop still
  ends with the final update, `wait_for` waits on, and a reset that does not decode releases no held frame); a replay
  skips the lines a reader does not understand (not JSON, a header, frame or side line that does not decode; bytes
  that are not UTF-8 read as U+FFFD), its header is the first line that is not blank when that decodes by its table (a
  member its model cannot hold is left out, a `source` that is no catalog entry too), and a read error (a gzip stream
  cut short, a dropped download) is its one `Err`, with no final update. A backoff with a base of zero waits zero at
  every attempt.

### Visualisation

- The 3D viewer and tray reports extrude each dogbone stone’s 12-vertex footprint to its recorded height. The
  shoulders and narrow waist follow the shape shown in the video overlay.
- **Visualisation** (`pdigger-factory-viz`): egui widgets in the look of texturecraft (`OverlayView`, `SceneView`,
  `EventFeed`, `HandCards`, `ViewerApp`), `ReportView` (a tray's report as a picture: the tray as it arrived and as it
  left, from above, its stones coloured by the report; `report_marks()` and `REPORT_COLORS`, the colours of the
  TypeScript SDK's), the `pdigger-factory-viewer` app for recording files, recording URLs and units (`[` and `]` jump
  between tray points; its Reports button lists the trays that left and shows their reports), `follow` and
  `follow_until` (which also ends when a stop future completes, even while an idle unit sends nothing), and software
  rendering for headless tests and pictures.

### Testing and documentation

- **Testing** (feature `testing`): `FakeFactory`, a fake unit that answers the whole API from a recording and recorded
  replies, in process or on 127.0.0.1 (`FakeServer`), with a live camera, `LIVE_CAMERA` (the recording as a camera
  session that connects, warms up and goes live on the fake's clock, with a picture for each of its newest frames),
  fault injection and a manual clock (a wait on it moves it on).
- The fake unit's camera follows build live3: a play connects again and warms up on the camera's next frames (after a
  break, `g` jumping by 5), only the newest 64 pictures are kept (`KEEP_PICTURES`), frames and raw packets are numbered
  as the unit numbers them, controls and errors come in the unit's order and words, an open stream follows a new
  program and the camera's phases, and on the fake's own clock (starting at the recording's time) a stream or a
  long-poll waiting for a phase moves it on; `with_linked_clock()` leaves the clock to its owner.
- Five [runnable tutorials](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/tutorials/README.md)
  cover tray reports, the interpreter, frame data, live cameras and Rust conventions. The first tutorial explains a
  real tray report and follows the uncertain changes back to their evidence.
- Unit tests in Docker with no network and 100 % line and function coverage; golden tests over the eighteen fixture
  recordings; doc tests; every example run against the fixtures and the fake unit; opt-in integration tests against a
  unit (read-only) and over the complete recordings.
