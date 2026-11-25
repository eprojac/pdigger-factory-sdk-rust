# Concepts

A factory unit follows one camera or recording at a time. Its **world** is the tracker’s current view of the stones,
hands and events. Use this guide to interpret frames, coordinate systems and changes to the shared session.
[Measurements from the 20.20 unit](BEHAVIOUR-20.20.md).

## The unit and its API

A pDigger Factory unit is a small computer over a work area (the 20.20 test unit is a Jetson Orin NX on the factory
LAN). It runs a model on one camera's frames and keeps a *world*: the stones on the board, the hands, and the events
that change them. Its HTTP API (v2, [spec/openapi.yaml](../spec/openapi.yaml)) serves that world as it happens, plus
everything the model saw on the way.

```text
http://factory-orin.local:8120/api/v2/      the API root (the SDK's base URL is http://factory-orin.local:8120)
```

`FactoryClient::new(base_url)` makes a client; `FactoryClient::builder(base_url)` sets timeouts (10 s per request,
5 s to connect), retries (3), the backoff, extra headers, a bearer token, the user agent's suffix, the transport, the
clock and the random source. The docs use `http://factory-orin.local:8120`; the SDK's integration tests read
`PDIGGER_FACTORY_URL`. Requests carry the user agent `pdigger-factory-sdk-rust/1.0.0`.

## The shared session

Every client shares one source, playhead and interpretation. Selecting a source or seeking starts a new session and
assigns a new session ID. Play and pause keep the current ID. Frames, events and activities carry that ID. When it
changes, the interpreter closes any open moves as `interrupted` and starts tracking the new session.

The controls (`Session::watch`, `watch_at`, `seek`, `pause`, `play`, `focus`, `clear_focus`) change the session for
every viewer. `Session::dry_run()` returns a session whose controls the server only checks: it says what it would do
and changes nothing. Today any client on the LAN can use them; the shipped API may require a token
(`ClientBuilder::bearer_token`).

**Viewers.** A stream or long-poll counts as a viewer unless it observes (`observe=1`: `FramesOptions::observe`,
`Interpreter::observe(true)`, or `ClientBuilder::observe(true)` for every stream of a client). The unit's worker
interprets only while someone watches: with nobody watching, nothing is interpreted and the frames of that time never
exist. The `Interpreter` therefore counts as a viewer by default. An integration that must not miss anything keeps one
interpreter running; tools that only look (dashboards, the SDK's integration tests) observe.

## Sources and the live cameras

A *source* is what the session can show. The API calls sources videos: `GET /api/v2/videos` is the catalog, and
`FactoryClient::sources` returns it (`Source` is the catalog's `VideoEntry`: `id`, `title`, `kind()`, `is_live()`,
`is_seekable()`, `duration`, `frames`, `fps`, `hold`, `station`, `native_wh`, `camera`, ...). There are two kinds:

- **recordings** (`kind` `recording`): a video file with a `duration` and a `default_start`, which you can seek in.
  The session plays it at its `rate`. On build `claude-20261114-int17` that is 0.8 (its worker keeps up at 0.8, not at
  1.0, on the full-frame videos); build `claude-20261122-hr4x1` sets it itself to what its worker keeps up with (a
  `clock` change by `auto-rate`): dry1 played at 0.75 for all but about a minute (down to 0.63), its dry2 sessions in
  the fixtures at 0.75, and in `dry1-0000-0130` the rate goes from 0.8 to 0.74 at 0:04.2 and to 0.75 at 0:23.1. When
  it ends, the session moves on to the next recording of the catalog (cameras are skipped; after the last recording,
  the first).
- **live cameras** (`kind` `camera`, `live` true, `seekable` false), listed after the recordings by their serials
  (`CAM00000001`): the camera itself, at rate 1.0, with no duration, no frame count and no media. `hold` is the warm-up:
  the seconds of camera frames the interpreter runs on before the first is shown (3), and `station` the plant station of
  the camera's rig. The 20.20 unit (build `claude-20261122-hr4x1`) lists the six cameras of the plant rigs. On build
  `claude-20261120-looks1` they streamed at 1.6 to 3.9 frames a second; build `claude-20261122-hr4x1` interprets about
  3.6 a second on a camera (274 ms a frame, 35 ms of it decoding the camera's JPEG), live about 0.5 s behind the camera.
  `FactoryClient::cameras` returns them, and `FactoryClient::live` the first (or the error `live_unavailable`,
  `Error::is_live_unavailable`, from a build that lists none).

<!-- example: crates/pdigger-factory-sdk/examples/session.rs -->
```rust
//! Sources, the live cameras and the shared session: its state, its clock and its controls,
//! checked as dry runs (a real control moves the one session for every viewer).
//!
//!     cargo run --example session -- http://factory-orin.local:8120
use pdigger_factory_sdk::FactoryClient;

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    for source in factory.sources().await? {
        let kind = if source.is_live() { "live camera" } else { "recording" };
        println!("{:11} {kind}: {}", source.id, source.name());
    }
    match factory.live().await {
        Ok(live) => println!("first live camera: {} ({})", live.id, live.station.as_deref().unwrap_or("no station")),
        Err(e) if e.is_live_unavailable() => println!("this unit lists no camera: the shared session plays recordings"),
        Err(e) => return Err(e),
    }
    let session = factory.session();
    let state = session.state().await?;
    let clock = session.clock().await?;
    println!(
        "the session plays {} (playing: {}), media time {:.1} s",
        state.video,
        state.playing,
        clock.media_time_now()
    );
    let dry = session.dry_run(); // validated by the unit, nothing moves
    let checked = dry.watch(&state.video).await?;
    println!("watch {} (dry run): accepted, session {}", state.video, checked.session);
    if let Err(e) = dry.seek(-5.0).await {
        println!("seek to -5 s (dry run): refused, {e}");
    }
    Ok(())
}
```

Call `session.watch(&camera.id)` to start a camera session at media time zero and playback rate 1.0. Read
`state.live` (`phase`, `warmup`, `since`, `lag`) to follow the connection.

| Phase | What the unit is doing |
| --- | --- |
| `connecting` | Waiting for the camera’s first frame. The worker status explains a delay. |
| `warming` | Feeding the first frames to the interpreter. The unit discards these frames after warm-up. |
| `live` | Publishing each interpreted frame and its camera image. |

Each transition increments the session revision and reaches clients through long-poll or a `clock` event.
`Session::until_live(timeout)` polls every 250 ms. It returns `live_timeout` when the timeout expires.

Frame numbers start at zero and include warm-up. At five frames per second, the first visible frame is about 15.
Media time measures seconds since the camera started, and the playhead trails the camera by its reported lag. The
unit keeps the latest 64 camera JPEGs. Read one with `Session::picture(g)`. Use `Session::picture_in(g, session)` to
check the session ID.

| | A recording | A live camera |
|---|---|---|
| `Source` | `kind` `recording`, `is_live()` false, seekable, a `duration` | `kind` `camera`, `is_live()` true, not seekable, no duration or frame count, a `station` |
| Rate | 0.8 on build `claude-20261114-int17`; build `claude-20261122-hr4x1` sets its own (`auto-rate`): 0.75 for most of dry1 | 1.0, with frames at the camera's own rate as far as the worker keeps up (1.6 to 3.9 a second on build `claude-20261120-looks1`, about 3.6 on build `claude-20261122-hr4x1`); `g` jumps (by 5) only over a pause, a camera that stopped answering or a spell with nobody watching |
| Media time `t` | seconds from the start of the video | seconds since the camera session started (`live.since`) |
| `watch` | at a time (`watch_at`), or the source's default start | `connecting`, `warming`, then `live` (`until_live`); a time is refused (`not_seekable`, field `time`, without sending the select; for an id the SDK reads the catalog) |
| `seek` | a new session at that time | refused with `not_seekable` (field `video`) without sending the seek (the SDK reads the session first) |
| `pause`, `play` | pause the playhead, go on from it | freeze the shared view and stop the interpreter; play goes on at the live edge after a new warm-up (`g` jumps over the pause, for everybody) |
| End | the unit moves on to the next recording by itself (cameras are skipped; after the last recording, the first) | none |
| Pictures | the proxy video: an MP4 with byte ranges (`get_video_media`, `open_video_media`) | no media (404 `not_found`): each frame's picture (`get_frame_picture`, `Session::picture`) |

An entry without `kind`, `live` or `seekable` (the unit's recordings omit them) is a seekable recording.

On a live camera the useful timestamp of an event is its wall time (`TimeRef::wall` of `Event::at()` and of every
activity's `start` and `end`), which the SDK computes from the session clock.

## Frames, cursors and time

The unit interprets on a grid of about five frames per media second, whatever the video's frame rate (every 12th
frame of the 60 fps recordings, every 2nd of the 10 fps ones), numbered `g` from 0 at the start of the video. A
`Frame` carries:

- `g` and `t`: the grid index and the frame's media time in seconds;
- `session` and `video`: the session and the source;
- `cursor`: the frame's position in the unit's output, increasing over a boot (across sessions). A stream event's id
  is `<boot>-<cursor>`, which is what `Last-Event-ID` resumes from;
- its layers, and `missing`: the layers it was released without.

The unit keeps the newest 512 frames (102 s of media). A stream resumed with `Last-Event-ID` inside that window gets
every frame it missed; `Api::get_frame` of an older frame fails with `404 not_retained`, and `Error::retained()` gives
the window the unit still holds.

**Wall time.** While a session plays, the playhead is `anchor_media + (now - max(anchor_wall, hold_until)) × rate`.
`Session::clock()` returns a `SessionClock` that also corrects for the offset between the unit's clock and yours (from
the request's round trip): `media_time_now()`, `media_time_at(server_wall)`, `wall_of(t)`, `local_wall_of(t)`,
`is_live()`. The interpreter turns media times into wall times with the newest clock state it has seen.

## Layers

A frame has up to five layers (`Layer`); ask only for the ones you need:

| Layer | What | Rust type |
|---|---|---|
| `raw` | the model's packet (`sip1`): boxes, joints, stone tuples, packed maps (tray, depth, hands) | `Sip1Packet` |
| `overlay` | what to draw over the video: regions, joints, masks | `OverlayLayer` |
| `scene` | the 3-D scene: skeleton, observed surface | `SceneLayer` |
| `cards` | the hands' cards, such as "Holding a stone" | `CardsLayer` |
| `world` | the Stable sim's world (`stw1`): stones, hands, events, stats | `WorldLayer` |

The interpreter uses `world` and `cards`. A frame is released at most 300 ms after its packet with the layers that are
ready. On the 20.20 unit, build `claude-20261114-int17` released 3 to 4 % of the frames of dry1 and dry2 without their
world (none on aug28 and jul30), and every one was there when read again 1.5 s later; build `claude-20261122-hr4x1`
released 507 of dry1's 9,042 frames (5.6 %) without it, and every one was read back. The SDK's streams read such a
world back 1.5 s later and release the frames in order (`FramesOptions::backfill`, on in the interpreter), so the
interpreter sees every world.

**Revisions.** While the session is paused, the world of the paused frame is refined and released again with a higher
`rev`. The streams pass such a frame on with `FrameItem::is_revision` set, and the interpreter emits only the events
it had not emitted for that frame. A frame repeated with the same or a lower revision is dropped.

## The world, events and activities

The world's *stones* have persistent ids (the same for every viewer over a session), a family (`rect` or `dogbone`), a
shape (`model`, `provisional`, or `frozen` once final), a mode, a 2-D outline in image pixels and a 3-D solid. The two
*hands* are each `free`, `touch`, `grasp`, `hinge` or `carry`, with the stone they hold.

An *event* is what the world says happened to a stone in a frame: `new`, `confirm`, `freeze`, `push`, `lift`, `hinge`,
`carry`, `place`, `drop`, `gone`, `delete` or `split`. Most events are the world keeping its inventory (stones appear,
are confirmed, get their final shape, are removed); the others are hands pushing, lifting, carrying and putting down
stones. An *activity* is what the SDK joins from events at the scale people talk about: a *move* (one stone taken and
put somewhere) or a *push burst* (a hand pushing stones about). [interpreter.md](interpreter.md) has the rules and
every sentence.

## Coordinates and the template gauge

- **Image:** `u`, `v` in 0..1 of the source's native image; pixels are `px = u × W - 0.5`
  (`codec::uv_to_px`, `codec::px_to_uv`). Outlines are in native pixels.
- **Scene:** `[x, y, z]` with the board in the x-y plane and z up; quaternions are `[x, y, z, w]`. three.js and other
  y-up renderers take `[x, z, -y]` (`codec::scene_to_three`).
- **Template gauge:** the camera is not calibrated, so the scene is measured in the scale of a template adult body, not
  in metres (the catalog's camera says `"gauge": "template (MHR adult), not metric"`). The SDK reports distances in
  *units* for that reason (`0.84 units`); compare them with each other, not with a tape measure.
