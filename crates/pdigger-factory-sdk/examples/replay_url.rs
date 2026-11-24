//! Streams a published recording from its URL and prints its events as they are read, at the
//! recording's own pace (`realtime(4.0)`: four times as fast).
//!
//!     cargo run --example replay_url -- https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz
use futures::StreamExt;
use pdigger_factory_sdk::ReplayStream;

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz".to_string());
    let replay = ReplayStream::open_url(&url).await?;
    if let Some(header) = replay.header() {
        println!("{} ({} frames)", header.video.as_deref().unwrap_or("?"), header.frames.unwrap_or(0));
    }
    let mut events = replay.realtime(4.0).events().take(10);
    while let Some(event) = events.next().await {
        println!("{}", event?.describe());
    }
    Ok(())
}
