# The interpreter

The high level of the SDK: it follows the shared session (or reads a recording) and says what happens on the tray, as
typed values and as sentences. The four languages share these rules and print the same text for the same data; the
golden files in `fixtures/golden/` are the reference (the contract is section 4 of
[spec/SDK-DESIGN.md](../spec/SDK-DESIGN.md)).

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

The world's stone ids are the engine tracker's. It renames a stone it lost and found again, and it can keep an id on
another stone: a stone set down where a removed one lay, a neighbour pushed into its place, a row of equal stones pushed
one place along while the ids stay on the places. For anything that must hold over time ("stone X was removed"), use the
**tray stones**: the interpreter follows each tray from the moment it has arrived and settled to the moment it is about
to move out, and gives every stone on it an id that is never reused, no other tray stone is ever given it:
`{first seen}-{tray}-{world id}`, for example `20261122T023847.0Z-1-117` (the server's wall time in UTC when the stone
was first seen on tray 1, under world id 117; without a clock `s{session}t{media time}-{tray}-{world id}`; a second
stone first seen on the tray in the same tenth of a second under the same world id takes `-2` at the end, a third
`-3`).

A tray stone says whether its identity is still **sure**, and a fact about it is **certain** or **unclear**. Tray facts
are final: a fact, once given, is never revised or withdrawn, and a stone that is not sure (fact `unsure`) is sure
again only through the unit's speckle check as its tray leaves, once, with an `identified` fact
([below](#the-units-speckle-check)). On a worked tray the world stone a tray stone stands for can turn out to be another
stone; the rules mark a stone not sure whenever what the world shows leaves another reading, but they do not tell which
of the not-sure links are wrong. The rules prefer saying "unclear" to guessing:

- A **tray** is a stop of at least 2 s. It has **arrived** at the first calm frame with every stone confirmed and no new
  stone for five frames (a calm frame: both hands free for three frames in a row, no hand event, no stone new, deleted
  or split). It is **leaving** just before it moves out: at its last calm frame, when no hand worked after it (else at
  its last frame). A quiet omission after arrival pauses the calm count. A stone can keep its identity across one or two
  omitted frames when it returns at a compatible size and place, with both hands free, no push and no gap in the stream.
  An unresolved omission is settled as a loss at its first missing frame.
- A stone stays **sure** while nothing could have swapped it: it stops being sure when it was pushed, when a stone of
  its size next to it was pushed, when its box overlaps another stone, when it is not seen at two calm moments, or when
  a stone of its size appeared next to it while a hand held it. A stone without a look chain that is not seen at two
  calm moments while nothing could have changed it since it was last seen is first held in question (not sure,
  `question` set, no fact yet): it is sure again once the world sees it again on its place, with no hand at work, no
  held stone, no stone a hand took away, no push on the tray and no frame the SDK could not read (a frame without a
  world, a jump in `g`, a `gap` or `reset` of the stream, a revision, the moving frames of a tray that goes on) from its
  last sight until then, no hand on it or near it, no other stone of its size within 0.12 of its place, and no look of
  its world stone that matches another stone in question or lost; otherwise it stops being sure. When the question is
  settled on a calm moment that does not show the stone (at once at its doubt, or by a sight before that calm moment),
  the next calm moment that shows it, or the tray's last frame when the tray leaves first and that frame shows it, must
  find nothing at work since either. A stone with a look chain is never held in question: its doubt is a question only
  its looks settle. When the world renames a stone, the tray stone follows only if exactly one new world stone fits,
  close by and within 3 s.
- A **removal** is certain when the stone's place is empty at a calm moment, as far as the world shows, and no world
  stone could be it. An **addition** is certain when a new stone stays at one place over two calm moments while no stone
  of its size is lost or not sure. When the tray leaves, a lost stone is matched to a new world stone near it only
  with evidence that they are the same stone (the unit's speckle check pairs them, their looks agree, or no hand worked
  near either since the stone was last seen); everything else is reported as unclear, so a stone's id never passes to
  a stone that may be another.

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

A tray's report is the SDK's simple mode (SDK-DESIGN 4.7): `tray.report()` once the tray has left (`None` before),
`interpreter.tray_reports()` from a unit as each tray leaves, and `Replay::tray_reports()` for every tray of a
recording. A `TrayReport` has the tray's `session`, `video` and number (`tray`), its points `arrived` and `left`, the
number of `stones` it arrived with, its changes `removed`, `added` and `moved` (`TrayChange`: `kind`, `stone`, `at`,
`certain`, `code`, `place`, `size`, `distance`, `confidence`), the counts `unclear` and `unsure` (the stones still on
that are not sure), and `complete`: no unclear change and no stone that is not sure. `describe()` writes it as text, and it is a
`serde` type for data. A stone is reported moved only when the SDK is sure it is the same stone, saw it at the leaving
point and something touched it on the tray, and it moved more than `MOVED` (0.75) times its short side once the tray's
own shift is taken out.

`tray.open` is the open account of the tray's leaving (`TrayOpen`, SDK-DESIGN 4.6.8: for each stone left open, the
readings the evidence still allows; `None` until the tray has left), and `TrayFact::code` gives each fact's reason as a
code (`carried_away`, `pushed`, `not_recognised`, ...). A tray stone also has `first_place`, `left_place` and
`touched`, which the report reads, and `question` while it is held in question; with the stones' looks, a fact may be
`identified` and a tray stone carries `looks`, `by_look`, `look_lost` and `open`; with the unit's speckle check,
`speckle`, `by_speckle` and `evidence` (below).

### Confidence

Every change of a report has a `confidence`, the probability that it is right: the share of changes of its kind that
were right on 150 trays of the four complete recordings checked against their video (the maintainers' offline checks,
not published: each stone followed by its look both ways, and a camera check of the certain removals), as
`(right + 1) / (checked + 2)` (SDK-DESIGN 4.6.13). Its kind is the change, whether it is certain, and what the unit's
speckle check found of the stone at the other moment (`TrayStone::evidence`, a `TrayEvidence`): for a removal, whether
the stone is on the tray as it left; for an addition, whether it was on the tray as it arrived.

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

Every report uses it; nothing needs to be called. Since build `claude-20261123-speckle3` the unit compares each tray's
stones between its arrival and a later moment by their speckle (the grain of each stone's face), on its GPU, and sends
each answer once, in the world (`world.speckle`; [how the unit does it](BEHAVIOUR-20.20.md#8-the-speckle-answers)); the
four published recordings carry answers made afterwards. The answer for the tray's last calm frame is the leaving's
(SDK-DESIGN 4.6.13): as the tray leaves, the stones it pairs are sure, confirmed on their world stone or linked again
where the world had lost them or put them on other world stones, and each change takes its confidence from what it
found. A tray whose hands worked after its last calm frame has no leaving answer: its report is made without one, its
changes `Undecided`.

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

What the SDK can not see, since it sees only the world: a box the model sees on an empty slot when the tray arrives
becomes a tray stone, and a stone later set into that slot keeps its id; and a stone the model stops seeing for good
while it stays on the tray looks removed: unclear when the tray leaves, or for certain when a hand touched it before it
was lost and no world stone could be it (3 of the 52 certain removals in a camera check of the four complete recordings
were such stones, each one a stone a hand had moved). On busy trays, where hands push rows of equal stones, the
speckle check as the tray leaves settles most pushed stones; where hands worked after the tray's last calm frame there
is no leaving answer, and most pushed stones end up not sure. Stones set down are mostly unclear: their confidence says
how far to rely on each.

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
