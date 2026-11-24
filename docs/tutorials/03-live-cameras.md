# Watch a live camera

<a id="live-cameras-one-source-at-a-time"></a>

List the available sources, select a camera and wait for its first live frame. The unit has one shared session.
Selecting a camera changes what operators and other SDK clients see, so use a test unit or the
[fake camera](../testing.md) while learning.

From the repository root, run the
[examples/live_camera.rs](../../crates/pdigger-factory-sdk/examples/live_camera.rs) program.

```sh
cargo run --example live_camera -- http://factory-orin.local:8120
```

<details>
<summary>Show the complete Rust example</summary>

<!-- example: crates/pdigger-factory-sdk/examples/live_camera.rs -->
```rust
//! Watch a plant camera live: select it for the shared session, wait out its warm-up, then print
//! what happens.
//!
//! Selecting a camera changes what every viewer of the unit sees (the page follows the session).
//! The unit connects to the camera, runs the interpreter on its first 3 seconds of frames to warm
//! it up, and then shows it live. Give a camera's serial to watch that one; stop it with Ctrl-C
//! (or give a number of events to stop after).
//!
//!     cargo run --example live_camera -- http://factory-orin.local:8120 [serial] [events]
use std::time::Duration;

use futures::StreamExt;
use pdigger_factory_sdk::FactoryClient;
use pdigger_factory_sdk::api::FrameSel;

/// The UTC time of day of an epoch second, `hh:mm:ss`.
fn utc(wall: f64) -> String {
    let s = wall.floor().rem_euclid(86_400.0) as u32;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let url = args.first().cloned().unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let serial = args.get(1).filter(|s| !s.is_empty() && !s.bytes().all(|b| b.is_ascii_digit()));
    let stop_after = args.iter().skip(1).find_map(|a| a.parse::<usize>().ok()).unwrap_or(usize::MAX);
    let factory = FactoryClient::new(&url)?;

    let camera = match serial {
        Some(serial) => factory.source(serial).await?, // that camera
        None => factory.live().await?,                 // the first camera
    };
    factory.session().watch(&camera.id).await?; // connecting, then warming up on the camera's first 3 s
    let state = factory.session().until_live(Duration::from_secs(30)).await?;
    let lag = state.live.as_ref().map_or(0.0, |live| live.lag);
    println!("{} ({}) is live: session {}, {lag:.2} s behind the camera", camera.id, camera.name(), state.session);

    let picture = factory.session().picture(FrameSel::Latest).await?; // the JPEG the newest frame was interpreted from
    println!("frame g {}: a {}-byte JPEG", picture.g, picture.jpeg.len());

    let mut events = factory.interpreter().events();
    let mut shown = 0;
    while let Some(event) = events.next().await {
        let event = event?;
        let at = event.wall.map_or_else(|| "--:--:--".to_string(), utc);
        println!("{at} UTC  {}", event.describe()); // on a camera, the wall time is the timestamp that matters
        shown += 1;
        if shown >= stop_after {
            break;
        }
    }
    Ok(())
}
```

</details>

## The sources

`factory.sources()` lists what the unit can show: the recordings, which have a duration and can be sought in, and then
the cameras, by their serials (`kind` camera, `is_live()`), which have neither: a camera is the camera itself, at rate
1.0. `factory.cameras()` returns only the cameras, and `factory.live()` the first (or the error `live_unavailable`).

## Watch, warm up, go live

`watch` starts a new session on the camera. The unit connects to it, runs the interpreter on its first seconds of frames
to warm the world up (those frames are never shown or stored), and then shows it live: the session's `live.phase` goes
from `connecting` to `warming` to `live`, and `until_live()` waits for the last (the error `live_timeout` when it does
not come in time). The unit interprets only while someone watches: the watch counts as a viewer for 15 s, and the
interpreter's stream opened next keeps the camera live. To check a watch without changing anything, send it as a dry
run: `factory.session().dry_run().watch(&camera.id)`.

## Pictures and wall times

Every frame of a camera comes with the camera's own picture, the JPEG the model saw: `session().picture(FrameSel::Latest)`
gives the newest one, and the unit keeps the newest 64. The interpreter follows a camera exactly as it follows a
recording; on a camera, media time only counts the seconds since the session started, so the time that matters is
each event's wall time (`event.wall`, seconds since 1970).

## What differs on a camera

- A camera cannot be sought: a seek gives `not_seekable`, and the SDK refuses one locally when the shared state shows
  a camera. `pause` freezes the shared view, and `play` goes on at the live edge, after a new warm-up.
- A camera's frames come at the camera's own rate as far as the worker keeps up (1.6 to 3.9 a second on build
  `claude-20261120-looks1`, about 3.6 on build `claude-20261122-hr4x1`; a recording gives 5), and `g` counts them
  from the start of the session.
- For your tests, the fake unit offers a camera, `cam-live`, made from its recording, with its phases and pictures:
  `FakeFactory::with_live()` in process, or `fake_server --live` on 127.0.0.1 ([testing.md](../testing.md)).

The details are in [concepts.md](../concepts.md), and [BEHAVIOUR-20.20.md](../BEHAVIOUR-20.20.md) has the 20.20
unit's cameras, measured.
