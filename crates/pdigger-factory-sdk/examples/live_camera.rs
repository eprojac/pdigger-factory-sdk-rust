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
