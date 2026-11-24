# Changelog

All notable changes of the Rust SDK (`pdigger-factory-sdk`) and its visualisation crate (`pdigger-factory-viz`).
Versions follow [semantic versioning](https://semver.org); the release workflow takes a version's notes from its
section here.

## 0.1.0 - Unreleased

The first release, for the API v2.2: what the 20.20 unit serves (build `claude-20261123-speckle7`: four recordings, six
plant cameras, the stones' looks and the speckle answers).

- **Reports, simple mode** (SDK-DESIGN.md 4.7). One `TrayReport` per tray, from `Tray::report`,
  `Interpreter::tray_reports` (as each tray leaves a unit) and `Replay::tray_reports` (every tray of a recording): the
  stones removed, added and moved (`TrayChange`), when and where (each one's final place on the tray), and whether
  that is the tray's whole account, as text (`describe`) and as data (`serde`). The tray's open account as it leaves
  (`Tray::open`, `TrayOpen`, SDK-DESIGN.md 4.6.8), each fact's reason as a code (`TrayFact::code`), and the tray
  stones' `first_place`, `left_place` and `touched`, which the report reads; with the stones' looks, `identified`
  facts and the tray stones' `question`, `looks`, `by_look`, `look_lost` and `open`.
- **The unit's speckle answers** (SDK-DESIGN.md 4.6.12, `world.speckle` since build `claude-20261123-speckle3`), read
  as a tray's: `TraySpeckle` in `Update::speckle`, `Interpreter::speckle` and `InterpreterCore::speckle` (the newest
  1024) and `ReplayOutput::speckle`, each tray stone's part as `TrayStone::speckle` (`StoneSpeckle`: `score`, `second`,
  `clean`, `found`, `pair`, `same`), and `describe_speckle_txt`, the golden `speckle.txt`; `WorldLayer::speckle`
  (`WorldSpeckle`) is `None` when the answer does not fit by value.
- **The leaving's speckle answer** (SDK-DESIGN.md 4.6.13, tray rules version 8), in every tray report: the stones the
  unit pairs are sure, confirmed on their world stone or linked again (`identified` facts with codes `speckle_same`,
  `speckle_moved` and `speckle_found`, `SPECKLE_CODES`; `TrayStone::by_speckle`), which the self-check's C3, C4 and C9
  allow; each tray stone's `evidence` as its tray leaves (`TrayEvidence`: `found`, `not_found` or `undecided`); and
  every report change's `confidence` (`TrayChange::confidence`, from the measured constants `CONFIDENCE` and
  `MOVED_CONFIDENCE`), at the end of each change line of `describe` (` (0.98)`, ` (unclear, 0.71)`). A pair whose finds
  give no score links nothing, unless its stone is sure and already on its world stone. dry1-0018-0153's tray: 11
  removed, 13 added, 8 moved and 1 stone not sure.
- **Presence questions** (SDK-DESIGN.md 4.6.1). A tray stone without a look chain that is not seen at two calm moments
  while nothing could have changed it is held in question (`TrayStone::question`, cause `unseen`; not sure, no fact) and
  is sure again once the world sees it again on its place with no hand at work, no held stone, no stone a hand took away
  and no push on the tray meanwhile, no other stone of its size within 0.12 of its place, and no look of its world stone
  that matches another stone in question or lost; otherwise its fact `unsure` comes where the question ends, `opened` at
  its doubt. A question settled on a calm moment that does not show the stone (at once at its doubt, or by a sight
  before that calm moment) holds only if the next calm moment that shows it, or the tray's last frame when the tray
  leaves first and that frame shows it, finds nothing at work since (else a plain `unsure` fact there). A stone with a
  look chain is never held in question: its doubt is a question only its looks settle. Frames the tray rules can not
  read (a frame without a world, a jump in `g`, a `gap` or `reset` of the stream, any revision, the moving frames of a
  tray that goes on) end the questions open then; `InterpreterCore::gap` tells the core about a `gap` or `reset` event
  (the `Interpreter` calls it for those of its stream, `Replay` and `ReplayStream` for the `gap` and `reset` lines that
  decode), and `InterpreterCore::feed_line` feeds a core of your own the lines of a recording as a replay does (the
  viewer reads a recording URL with it). A settled stone gets no `unsure` fact, one whose question ends gets its fact
  there, and the leaving counts its last snapshot's own frame only once for a stone nothing could have changed since its
  last sight (a stone not detected only there gets no `unsure` fact).
- **Certain removals and continuation** (SDK-DESIGN.md 4.6.1). A lost tray stone is removed for certain only when a hand
  touched its world id (a `lift`, `hinge`, `carry`, `place`, `drop` or `gone`, a `push` with a `side`, or a `delete`
  with why `hand` or `carried-away`) from the last calm moment before its loss, or from 3.0 s before it was last seen if
  that is earlier, up to its loss; else it stays lost, and a rename or the leaving pairs it, or the leaving reports it
  removed, unclear, with the open cause `no_hand`. A visit continues the open tray only when nine tenths of its first
  frame's stones were seen on it, so a new board that carries over many of the last board's world ids is a tray of its
  own. On the four published recordings: 52 certain removals, 49 of them right by the maintainers' camera check, and 628
  trays with 532 reports.
- **The leaving's evidence** (SDK-DESIGN.md 4.6.1, Leaving). As a tray leaves, a lost tray stone is linked to a new
  world stone near it only with evidence that it is the same stone: their looks agree, or the world stone was first seen
  at or after the lost stone was last seen (less 3.0 s), no hand worked within 0.40 of either place since, and their
  looks do not disagree. Without it the lost stone is removed and the world stone added, both unclear (each the other's
  candidate in the open account), so a stone's id never passes to a stone that may be another: by the maintainers'
  camera check, every stone that left dry1's and dry2's trays is reported removed. An unidentified stone that the
  leaving point's world shows detected and not held is present as the tray leaves, at its place there (positive evidence
  at departure). So is one that world keeps there (lists it, not detected), not held, with less than 0.30 of its box
  under one other unheld stone's box there: by the maintainers' camera check, 32 of the 63 stones this adds on the four
  published recordings were there (dry1's tray 2 accounts for the 44 stones the video shows), and 25 are boxes on
  jul30's bare boards that are not there.
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
- **Testing** (feature `testing`): `FakeFactory`, a fake unit that answers the whole API from a recording and recorded
  replies, in process or on 127.0.0.1 (`FakeServer`), with a live camera, `LIVE_CAMERA` (the recording as a camera
  session that connects, warms up and goes live on the fake's clock, with a picture for each of its newest frames),
  fault injection and a manual clock (a wait on it moves it on).
- The fake unit's camera follows build live3: a play connects again and warms up on the camera's next frames (after a
  break, `g` jumping by 5), only the newest 64 pictures are kept (`KEEP_PICTURES`), frames and raw packets are numbered
  as the unit numbers them, controls and errors come in the unit's order and words, an open stream follows a new
  program and the camera's phases, and on the fake's own clock (starting at the recording's time) a stream or a
  long-poll waiting for a phase moves it on; `with_linked_clock()` leaves the clock to its owner.
- **Visualisation** (`pdigger-factory-viz`): egui widgets in the look of texturecraft (`OverlayView`, `SceneView`,
  `EventFeed`, `HandCards`, `ViewerApp`), `ReportView` (a tray's report as a picture: the tray as it arrived and as it
  left, from above, its stones coloured by the report; `report_marks()` and `REPORT_COLORS`, the colours of the
  TypeScript SDK's), the `pdigger-factory-viewer` app for recording files, recording URLs and units (`[` and `]` jump
  between tray points; its Reports button lists the trays that left and shows their reports), `follow` and
  `follow_until` (which also ends when a stop future completes, even while an idle unit sends nothing), and software
  rendering for headless tests and pictures.
- Four tutorials (`docs/tutorials/`: high level, low level, live cameras, Rust), whose programs
  (`tutorial_high_level`, `tutorial_low_level`) print what the other languages' tutorials print over the same frames.
- Unit tests in Docker with no network and 100 % line and function coverage; golden tests over the eighteen fixture
  recordings; doc tests; every example run against the fixtures and the fake unit; opt-in integration tests against a
  unit (read-only) and over the complete recordings.
