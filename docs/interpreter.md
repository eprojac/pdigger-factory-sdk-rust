# The interpreter

Use the interpreter to follow a hand movement, trace a stone or explain a tray report. Events describe what the unit
saw in a frame. Activities group events into moves and pushes. A recording produces the same results as the live
frame sequence. [Interpreter rules](../spec/SDK-DESIGN.md#4-interpreter-rules-golden-tested).

<!-- example: crates/pdigger-factory-sdk/examples/interpreter.rs -->
```rust
//! The high level, as the unit's videos tell it: the moves and push bursts as sentences while
//! the shared session plays, then what each hand is doing and a summary of what was seen.
//!
//!     cargo run --example interpreter -- http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::{FactoryClient, Side, WireEnum};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let interpreter = FactoryClient::new(&url)?.interpreter();
    let mut updates = interpreter.updates().take(30); // 30 frames: 6 s of media
    while let Some(update) = updates.next().await {
        for activity in &update?.activities {
            println!("{}", activity.describe()); // 5:01.6 Right hand moves stone 643 1.09 units in 1.2 s
        }
    }
    let state = interpreter.state();
    for (name, side) in [("Left", Side::Left), ("Right", Side::Right)] {
        let hand = state.hands.get(&side).cloned().unwrap_or_default();
        let doing = hand.state.as_ref().map_or("unseen", |s| s.as_str());
        let stone = hand.stone.map(|id| format!(", stone {id}")).unwrap_or_default();
        println!("{name} hand: {doing}{stone}");
    }
    print!("{}", interpreter.summary().describe());
    Ok(())
}
```

## Following the session

`FactoryClient::interpreter()` gives an `Interpreter`. Its streams follow the shared session from the newest frame:

| Method | Gives |
|---|---|
| `updates()` | one `Update` per frame: the frame, its events, the activities it closed, the tray facts it made, `is_revision`, `is_backfill`, the state after it |
| `events()` | the events, one by one |
| `activities()` | the moves and push bursts, as they close |
| `tray_facts()` | the tray facts: trays arriving and leaving, tray stones removed, set down or no longer sure ([trays](#trays-and-tray-stones)) |
| `wait_for(kind, timeout)` | the first event of that kind, `Ok(None)` when none came in time |
| `wait_for_match(predicate, timeout)` | the first event the predicate accepts |
| `track(stone_id)` | the path of a stone's solid from the frames fed: a `TrackPoint` (`g`, `t`, `pos`) where it moved, the newest 512 (the server's own track is `api().get_stone`) |
| `state()` | the state after the last frame (any task can read it) |
| `summary()` | the counters so far (the open session counts up to its newest frame) |
| `trays()` | the trays so far, oldest first: the newest 256 that have left and the open one |
| `finish()` | closes what is still open (moves as `interrupted`, push bursts); the open tray leaves |
| `end()` | `finish()` as the final `Update`: the activities still open and the facts of the open tray leaving |

Options: `layers(...)` (`world` and `cards` by default), `observe(true)` (do not count as a viewer), `backfill(false)`,
`resume(false)` (end with the first connection), `after(cursor)` (start after a cursor instead of the newest frame).
The streams resume after a dropped connection with `Last-Event-ID`, read late worlds back, and read the shared state
(`GET /session`) after every connect, so the interpreter sees every frame the unit kept and knows the wall times from
the first frame, across reconnects and session changes. Dropping a stream closes its connection. The streams of one
interpreter share its state: a frame it has already seen (the same `g`, no higher revision) is dropped as a
duplicate (`updates()` skips it; `InterpreterCore::feed` returns its empty update with `is_duplicate` set).

Loops can follow one another: `wait_for`, then `events()`, `activities()` or `updates()`, and so on. A later loop
goes on where the last one stopped and misses nothing in between: it first yields the rest of the frame the last loop
stopped in (the events, activities and tray facts that loop did not yield; after `wait_for`, the events after the
match and all of that frame's activities and tray facts), then the frames after it (the stream resumes after the last
frame fed, while the unit still holds it). Use one loop at a time. A stream the server ends with `resume(false)` ends
with the final update (`is_final`, no frame), as a replay does; a live stream otherwise has no end of its own: when a
program stops following, `end()` (or `finish()`) closes what is open. A stream that fails, or that a program drops,
gives no final update; an event that does not decode is yielded as an `Err` (code `decode`) and the stream goes on, so
it does not take the final update away. A loop dropped inside a frame leaves the rest of that frame to the next loop;
one dropped after its stream ended, after an error, or while it waits for the next frame leaves nothing.

The rules themselves are `InterpreterCore`, which is synchronous and does no I/O: `feed(frame)` returns the `Update`
(`feed_parts(&frame)` its parts without the state), `set_clock(state)` gives it the clock for wall times, `gap()` tells
it that the stream dropped frames (a `gap` or `reset` event), `feed_line(line)` feeds it a line of a recording as a
replay does (a frame, a clock line, a `gap` or `reset` line; [replays](replay.md)), `end()` the final update with what
is still open (`finish()` its activities), `state()`, `summary()` and `trays()` the rest, and `keep_trays(n)` sets how
many trays that have left it keeps (the newest; 0 keeps every one). `Replay` and the streams both use it.

## Events

An `Event` is one entry of a frame's world events: `kind` (`EventKind`), the stone `stone_id`, the `hand` (`Side`),
`why`, `into`, `source`, its frame (`g`, `t`, `wall`), `session`, `video`, the `stone` in that frame's world, and
`tray_stone`: the id of the [tray stone](#trays-and-tray-stones) the world id stood for on the open tray before this
frame, also for an event of a revision (`stone_id` is the world's id, which the engine's tracker renames and reuses). `describe()` is its sentence:
`"{clock} {sentence}"`, the clock being the media time (`M:SS.s`, `H:MM:SS.s` from one hour). `{Hand}` is
`Left hand`, `Right hand`, `Both hands`, or `A hand` when the event has no side; verbs agree with it.

| Kind | Sentence |
|---|---|
| `new` | `stone {id} appears`; found in the image: `stone {id} appears (found in the image)` |
| `confirm` | `stone {id} is confirmed` |
| `freeze` | `stone {id} gets its final shape` |
| `push` | with a side: `{Hand} pushes stone {id}`; `why: course`: `stone {id} is pushed along its row`; `why: reseat`: `stone {id} is reseated`; else `stone {id} is pushed` (plus ` ({why})` for another why) |
| `lift` | `{Hand} lifts stone {id}` |
| `hinge` | `{Hand} tips stone {id} up on its edge` |
| `carry` | `{Hand} carries stone {id}` |
| `place` | `{Hand} places stone {id}` |
| `drop` | `{Hand} drops stone {id}` |
| `gone` | `{Hand} takes stone {id} out of view` |
| `delete` | `stone {id} is removed`, with the reason: `no longer seen`, `overlapped another stone`, `outside the board`, `merged with a neighbour`, `its place is empty`, `a hand emptied its place`, `carried away` |
| `split` | `stone {id} splits into {a} and {b}`; into one: `stone {id} is trimmed`; no `into`: `stone {id} splits` |
| other | `stone {id}: {type}` (a type this SDK version does not know) |

Examples from the fixtures: `5:01.6 Right hand lifts stone 643`, `5:03.6 stone 643 is removed (a hand emptied its
place)`, `4:56.0 stone 664 gets its final shape`.

## Activities

`Activity` is an enum of two kinds, both with `hand()`, `start()`, `end()` (`TimeRef`: `g`, `t`, `wall`),
`duration()`, `session()`, `video()` and `describe()`:

- **`Activity::Move`** (`Move`): one stone taken and put somewhere. It opens on a `lift` or `hinge`, notes a `carry`
  and a tip, and closes on `place` (`MoveOutcome::Placed`), `drop` (`Dropped`), `gone` (`CarriedAway`) or the stone's
  `delete` (`Removed`); at a session change or `finish()` it closes as `Interrupted`. A put-down without a lift seen
  (a stream joined mid-move) makes a `partial` move. `from`, `to` and `distance` come from the stone's solid, in
  template units.
- **`Activity::Push`** (`PushBurst`): a hand pushing stones about: the pushes of one side at most 1.0 s apart, with
  the stones pushed (`stones`, ascending) and the number of pushes. Pushes along a row (`course`, which name no side) and
  the engine reseating a stone (`reseat`) do not count.

Their sentences (from the fixtures):

```text
5:01.6 Right hand moves stone 643 1.09 units in 1.2 s
4:59.0 Left hand pushes stones 630, 636, 637, 642, 643 and 644 (14 pushes in 4.6 s)
```

(the move is a stone placed after a carry) and the other forms: `{Hand} drops stone {id} after {dur} s`,
`{Hand} tips stone {id} and sets it down ({dur} s)`, `{Hand} lifts stone {id} and sets it down ({dur} s)`,
`{Hand} takes stone {id} away ({dur} s)`, `{Hand} lifts stone {id}; it is removed after {dur} s`,
`{Hand} is still holding stone {id} ({dur} s so far)`, and ` (joined mid-move)` after a partial one. A burst of more
than six stones lists the first six and `and {k} more`.

**Order.** Within a frame: the events in world order, then the activities that frame closed, sorted by end, start,
kind (moves first), stone and hand, then the tray facts it made. Activities closed before the frame's events (a stale
burst, a session change) come before them, and so do the facts of a tray that leaves at a session change.

## Trays and tray stones

The tracker assigns a **world ID** to a detection. When a hand pushes a row of similar stones, the tracker can
assign a new ID to a familiar stone or keep an ID at a position now occupied by a different stone. Use the **tray
stone ID** to follow one physical stone across frames. The SDK never reuses it.

For example, `20261122T023847.0Z-1-117` identifies a stone first seen at 02:38:47 UTC on tray 1 under world ID 117.
Without a wall clock the prefix is `s{session}t{media time}`. If two stones receive the same world ID on the same
tray within the same tenth of a second, the later IDs end in `-2`, `-3` and so on.

The SDK tracks two kinds of uncertainty. **Sure** describes a stone’s identity. **Certain** describes a fact, such
as a removal. The SDK keeps every fact it has emitted. When the departure speckle check resolves an earlier `unsure`
fact, it adds an `identified` fact to the history.

### When the SDK records a change

- **Arrival and departure.** A tray must stop for at least two seconds. Arrival is the first calm frame with all
  stones confirmed and no new stone for five frames. A calm frame requires both hands to be free for three
  consecutive frames, with no hand event or stone creation, deletion or split. Departure uses the last calm frame
  unless a hand worked after it, in which case it uses the last frame.
- **Identity.** A push, an overlap, a missing detection or a similar stone near a held stone can put an identity in
  doubt. A stone missing at two calm moments may regain its identity if it reappears in place and nothing could have
  swapped it. The SDK also checks tracking gaps, hand activity, nearby stones and appearance. A stone with an
  appearance history follows the separate matching rules.
- **Removal and addition.** A certain removal requires recent hand contact. For a stone that disappears on the tray,
  the SDK also checks that a calm frame shows its position empty and that no tracked stone could be the missing
  stone. An addition is certain when a new stone stays in place across two calm moments and no similar-sized stone
  is missing or uncertain.
- **Matching at departure.** The SDK links a missing stone to a nearby detection when the two match by speckle or
  appearance, or when no hand could have swapped them. Otherwise it records the possible removal and addition as
  unclear.

The [tray rules](../spec/SDK-DESIGN.md#461-the-tray-rules) give the exact conditions, distance limits and
exceptions.

Some of the facts of `dry1-0018-0153`, a busy tray of dry1, from 0:38.7 to 0:59.4 (`interpreter.tray_facts()`,
`describe()`):

```text
0:38.7 tray 1: stone 20261122T023847.0Z-1-105 is not sure any more (a stone next to it was pushed)
0:38.7 tray 1: stone 20261122T023847.0Z-1-126 is not sure any more (a stone next to it was pushed)
0:45.5 tray 1: stone 20261122T023847.0Z-1-97 is not sure any more (it was pushed)
0:45.5 tray 1: stone 20261122T023847.0Z-1-117 was removed
0:45.5 tray 1: stone 20261122T023847.0Z-1-127 was removed
0:59.4 tray 1: stone 20261122T023847.0Z-1-126 was removed
```

and its end, where the speckle check has settled all but one of the stones the worker pushed, and the SDK reports
the unresolved additions:

```text
2:24.0 tray 1: stone 20261122T024126.9Z-1-284 not recognised (unclear)
2:24.0 tray 1 leaving: 44 stones (30 sure), 0 added, 6 removed, 18 unclear
```

| Fact (`TrayFactKind`) | Sentence |
|---|---|
| `Arrived` | `{clock} tray {n} arrived: {stones} stones` |
| `Unsure` | `{clock} tray {n}: stone {id} is not sure any more ({why})` |
| `Removed`, certain | `{clock} tray {n}: stone {id} was removed`, with ` (carried away)` when a hand took it out of view |
| `Removed`, unclear | `{clock} tray {n}: stone {id} not found (unclear)` |
| `Added`, certain | `{clock} tray {n}: stone {id} was set down` |
| `Added`, unclear | `{clock} tray {n}: stone {id} not recognised (unclear)` |
| `Identified` | `{clock} tray {n}: stone {id} confirmed by its speckle on world stone {world_id} ({score})`; `recognised` or `found again` in place of `confirmed` when the stone was on another world stone or lost; `recognised by its look` for a look's link |
| `Leaving` | `{clock} tray {n} leaving: {stones} stones ({sure} sure), {added} added, {removed} removed, {unclear} unclear` |

A `TrayFact` has `kind`, `tray`, `session`, `video`, `stone` (the tray stone's id), `world_id` (the world id it
stands for now), `certain`, `why`, `g`, `t`, `wall`, and the counts of arrived and leaving (`stones`, `sure`, `added`,
`removed`, `unclear`). `interpreter.trays()` lists the trays so far (those that have left, the newest `TRAYS_KEPT` =
256, and the open one) and `Replay::trays()` every tray of a recording. A `Tray` has `session`, `video`, `n`, the
points `arrived` and `leaving` (`TrayPoint`, a `TimeRef`: `g`, `t`, `wall`), `stones` (`TrayStone`: `id`, `world_id`,
`status` (`TrayStoneStatus::On`, `Lost` or `Removed`), `sure`, `added`, `certain`, `born`, `place`, `size`), its
`facts`, `window()` (the media span from arrival to leaving, `None` until it has left) and `stone(id)`. The two points
are the ones to jump between: the viewer does it with `[` and `]` ([viz](viz.md#the-viewer)), and on the shared session
`session().watch_at(&tray.video, Some(point.t))` shows one ([replay](replay.md#trays)). The state has the open tray
(`tray`).

### Report fields

Call `tray.report()` after a tray leaves. Use `interpreter.tray_reports()` for a live stream of reports or
`Replay::tray_reports()` for a recording.

| Fields | Meaning |
| --- | --- |
| `session`, `video`, `tray` | Source session, video and tray number. |
| `arrived`, `left`, `stones` | Arrival and departure points, and the number of stones at arrival. |
| `removed`, `added`, `moved` | Changes in each category. Each carries a stone ID, time, certainty, reason code, position, size, distance and confidence as applicable. |
| `unclear`, `unsure`, `complete` | Counts of unclear changes and other retained stones with uncertain identities. A report is complete when both counts are zero. |

A move requires a known stone identity, evidence that the stone was touched, and a detection at departure. After
compensating for the motion of the tray itself, the displacement must exceed 0.75 times the stone’s shorter side.
Use `serde_json::to_string(&report)` for the report data.

`tray.open` is the open account of the tray's leaving (`TrayOpen`, SDK-DESIGN 4.6.8: for each stone left open, the
readings the evidence still allows; `None` until the tray has left), and `TrayFact::code` gives each fact's reason as a
code (`carried_away`, `pushed`, `not_recognised`, ...). A tray stone also has `first_place`, `left_place` and
`touched`, which the report reads, and `question` while it is held in question; with the stones' looks, a fact may be
`identified` and a tray stone carries `looks`, `by_look`, `look_lost` and `open`; with the unit's speckle check,
`speckle`, `by_speckle` and `evidence` (below).

### Confidence

Confidence estimates how often a reported change of this kind is correct. The maintainers checked 150 trays across
four recordings against the video. Each table entry uses `(right + 1) / (checked + 2)` for that group of changes.
[Calibration method](../spec/SDK-DESIGN.md#4613-the-leavings-speckle-answer-and-the-confidence-of-a-change).

For a removal, the evidence says whether the surface pattern of the stone was found at **departure**. For an
addition, it says whether that stone was already visible at **arrival**.

| Change | Certain | `NotFound` | `Undecided` | `Found` |
|---|---|---|---|---|
| removed | yes | 0.98 | 0.50 | 0.25 |
| removed | no | 0.71 | 0.58 | 0.09 |
| added | yes or no | 0.96 | 0.52 | 0.02 |

A move is `MOVED_CONFIDENCE`, 0.87. The numbers are constants (`pdigger_factory_sdk::interpreter::CONFIDENCE`), the
same in every language, so a client keeps the changes at or above a threshold of its own: at 0.90, dry1's tray 2 (the
fixture's tray 1) keeps its six certain removals and nine of its thirteen additions, all fifteen right by the camera
check.

### The unit's speckle check

The unit compares the grain on each stone’s surface at arrival with its appearance in a later image. Since build
`claude-20261123-speckle3` it sends each GPU comparison once in `world.speckle`. The published recordings include
comparisons computed after capture. [Camera measurements](BEHAVIOUR-20.20.md#8-the-speckle-answers).

The SDK uses the last calm frame to reconcile identities at departure. Clean matches can confirm a stone in place or
recover it under a new world ID. If a hand worked after the last calm frame, the departure comparison is unavailable
and the report uses undecided evidence.

Reading the answers yourself is advanced mode: to look into an unclear change, to check a tray by hand, or for rules of
your own. The interpreter reads each answer as a tray's (`TraySpeckle`): `update.speckle` holds the answers a frame
gave, `interpreter.speckle()` the answers so far (the newest 1024) and `Replay::run`'s `ReplayOutput::speckle` every
answer of a recording, and while the tray is on, each of its stones in the answer takes its part as `speckle`
(`StoneSpeckle`). As the tray leaves, `TrayStone::by_speckle` says the leaving's answer paired the stone, and
`TrayStone::evidence` what it found of it at the other moment (`TrayEvidence::Found`, `NotFound` or `Undecided`: a
look-alike, or a stone it did not check). An answer at another moment, or a score below the clean bar, is for you to
weigh: the tray rules use only the leaving's clean finds. The last answer for the first tray of the fixture recording
`dry1-0018-0153` (tray 2 of the published dry1), as `describe_speckle_txt` writes it, begins:

```text
732 2:26.0 tray 1: speckle of 0:22.5 and 2:24.2: 42 stones, 30 clean, 26 paired, 24 the same
  20261122T023847.0Z-1-84 arrival 0.632 0.112 clean found 20261122T023847.0Z-1-84 pair 20261122T023847.0Z-1-84
```

and, further down:

```text
  20261122T023847.0Z-1-104 arrival 0.481 0.090 clean found 20261122T023847.0Z-1-104 pair 20261122T023847.0Z-1-104
  20261122T023847.0Z-1-105 arrival 0.102 0.079 found world 193
  20261122T023847.0Z-1-106 arrival 0.706 0.102 clean found world 266
  20261122T023847.0Z-1-107 arrival 0.720 0.115 clean found 20261122T023847.0Z-1-106 pair 20261122T023847.0Z-1-106
```

Stones 84, in question since 2:18.8, and 104, pushed at 2:09.6, are the same stones at the leaving, so they stay sure;
105, removed at 1:05.4, is found nowhere clean; 107, which the world had lost, is found and paired on world stone 191,
where the SDK had linked tray stone 106, and 106 on world stone 266, which no tray stone stood for. As the tray leaves,
the SDK links them so: 107 found again on 191, 106 recognised on 266 (a camera check of this tray: 39 of its stones
right, none wrong). A `StoneSpeckle` has `stone`, `side` (`SpeckleSide::Arrival`: the stone as it was at the arrival, checked at the
later moment; `SpeckleSide::Later`: the other way), `score` and `second` (None: not checked), `clean` (a score of at
least 0.300 and at least twice the second), `found` (the tray stone at the other moment that its best place lies on,
None when none; `found_world` the world stone), `pair` (the tray stone the answer pairs it with, or None) and `same()`
(paired with itself). A `TraySpeckle` has `tray`, `session`, `video`, `g`, `t` and `wall` (the frame it came in),
`unit_tray` (the answer's own tray number, the unit's count), `reference` (`ref` in JSON) and `at` (the two moments: `g`
and `t`), `device`, `gpu_ms`, `wall_ms`, `stones`, `stone(id)` and `describe()`, and it is `serde`.

### Reading the dry1 report

The fixture `dry1-0018-0153` contains tray 2 of the full dry1 recording. The excerpt numbers it tray 1. A worker
straightened the rows and several stones briefly lost their tracker IDs. The SDK counted 42 stones at arrival and 44
at departure, matching the video count at departure.

| Change | What the report recorded | What the maintainers found in the video |
| --- | --- | --- |
| Removed | Six certain and five unclear | All six certain removals were correct. Of the five unclear removals, 87 and 129 left. Stones 121, 128 and 130 stayed. |
| Added | Thirteen unclear | Ten were set down and three (151, 246 and 249) arrived on the tray. At a 0.90 threshold, nine additions and six removals remain. All fifteen were correct. The filter also drops a genuine addition (278). |
| Moved | Eight | The SDK measured displacements of 0.33 to 0.43 scene units after accounting for the tray’s own movement. Each move has a confidence of 0.87. |

The unit’s departure comparison confirmed eight uncertain identities, found four missing stones and matched two to
different world IDs. Stone 98 remained uncertain. In the report, `18 unclear` counts changes. The separate
`1 not sure` counts that remaining identity.

A removal gives the last observed position. An addition or move gives the departure position when observed. For a
stone the world model retained without a detection at departure, the report can use its position in that model.
Stone 281 is an example. The worker set it down about six seconds before departure.

### Limits of the camera evidence

A complete report accounts for the stones visible to the model. If the model detects a stone in an empty slot at
arrival, a later occupant can inherit that ID. If the model loses sight of a stone permanently, the SDK can report a
removal even though the stone stayed. In a separate video check, three of 52 certain removals across the four
recordings were stones a hand had moved on the tray.

At departure the SDK excludes an unseen stone that is held, or whose box overlaps a free stone’s box by at least 30%
of its own area. On a busy tray, similar stones can also remain unresolved if hands worked after the final calm
frame. For the evidence behind any individual change, inspect its tray facts and the departure comparison.

## State

`Interpreter::state()` and `Update::state` are an `InterpreterState`: `session`, `video`, `g`, `t`, `wall`, `stones`
(id to `Stone`), `hands` (`left` and `right`: the hand's `state`, the `stone` it holds and its `card` title, such as
`Holding a stone`), `board`, `stats`, the moves still open (`open_moves`), the newest `clock` state and the open
[tray](#trays-and-tray-stones) (`tray`, `None` between trays). The viewer's hand cards show this state.

## Summary

`summary()` counts what was seen: frames (with and without a world, revisions), sessions, gaps, media time, events by
kind, moves by outcome with their mean duration and distance, push bursts by hand, stones, trays (opened and arrived,
tray stones at the arrivals, certain and unclear additions and removals, tray stones that stopped being sure), and the
time each hand spent in each state. `Summary` serialises to the JSON form of the contract (compared with `fixtures/golden/*.summary.json`)
and `describe()` is its text:

```text
frames: 51 (with world 51, without 0, revisions 0)
media: 0:10.0, sessions 1, gaps 0
stones: last 41, max 42, confirmed 41
moves: 1 (placed 1, dropped 0, carried away 0, removed 0, interrupted 0, partial 0, carried 1), mean 1.2 s, 1.09 units
push bursts: 5 (left 1, right 3, both 1), pushes 31
events: new 2, confirm 1, freeze 13, push 31, lift 1, hinge 0, carry 1, place 1, drop 0, gone 0, delete 1, split 0, other 0
trays: 1 (arrived 1), stones at arrival 42, added 0 (+0 unclear), removed 0 (+1 unclear), not sure 18
left hand: free 0:05.4, touch 0:03.0, grasp 0:01.8, hinge 0:00.0, carry 0:00.0
right hand: free 0:04.0, touch 0:02.6, grasp 0:02.4, hinge 0:00.0, carry 0:01.2
```

(the summary of the fixture recording `dry1-0296-0306-full`). Numbers in text are formatted by the contract's own
rounding (`floor(x × 10^d + 0.5)`), never by `format!("{:.1}")`, so the four languages print the same digits.
