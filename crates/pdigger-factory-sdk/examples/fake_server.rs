//! Serves a recording as a unit on 127.0.0.1 (the SDK's fake server, feature `testing`), to try
//! the examples and the viewer without a unit. Prints its URL, then serves until stopped.
//! `--live` offers a live camera, `cam-live` (the recording replayed as the camera, shown at the
//! start); the recording is also served at `/developers/data/<file>` for `replay_url`. The fake's
//! clock follows this machine's, and only this machine's (linked: the fake never moves it on to a
//! camera's next phase), so that a camera selected through it connects, warms up and goes live in
//! real time, as on a unit.
//!
//!     cargo run --example fake_server --features testing -- fixtures/recordings/dry1-0296-0306-full.ndjson.gz --live
use std::path::Path;
use std::time::{Duration, Instant};

use pdigger_factory_sdk::Clock;
use pdigger_factory_sdk::testing::FakeFactory;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let default = "fixtures/recordings/dry1-0296-0306-full.ndjson.gz".to_string();
    let path = args.iter().find(|a| !a.starts_with("--")).unwrap_or(&default);
    let bytes = std::fs::read(path)?;
    let name = Path::new(path).file_name().map_or("recording.ndjson.gz".into(), |n| n.to_string_lossy().into_owned());
    let replies =
        Path::new(path).parent().and_then(Path::parent).map(|fixtures| fixtures.join("api")).filter(|api| api.is_dir());
    let mut fake =
        FakeFactory::from_recording_bytes(&bytes)?.with_file(&format!("/developers/data/{name}"), bytes, 64 * 1024);
    if let Some(api) = replies {
        fake = fake.with_replies(api)?; // the recorded replies answer what the recording cannot
    }
    if args.iter().any(|a| a == "--live") {
        fake = fake.with_live();
    }
    let fake = fake.with_linked_clock(); // a stream waits for the camera's phases in real time
    let clock = fake.clock();
    let (started, epoch) = (Instant::now(), clock.now());
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(50)).await;
            clock.set(epoch + started.elapsed().as_secs_f64()); // the fake's clock runs in real time
        }
    });
    let server = fake.serve().await?;
    println!("{}", server.url());
    std::future::pending::<()>().await;
    Ok(())
}
