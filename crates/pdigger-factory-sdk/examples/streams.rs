//! The mid level: frames over SSE (resumed after a drop, late layers backfilled, revisions
//! flagged) and the long-poll alternative.
//!
//!     cargo run --example streams -- http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::streams::PollOptions;
use pdigger_factory_sdk::{FactoryClient, FramesOptions, Layer, StreamItem};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let streams = FactoryClient::new(&url)?.streams();
    let options = FramesOptions { layers: vec![Layer::World, Layer::Cards], observe: Some(true), ..Default::default() };
    let mut frames = streams.frames(options).take(5);
    while let Some(item) = frames.next().await {
        match item? {
            StreamItem::Frame(item) => {
                let stones = item.frame.world.as_ref().map_or(0, |w| w.stones.len());
                println!(
                    "frame g {} at {:.1} s: {stones} stones (revision: {})",
                    item.frame.g, item.frame.t, item.is_revision
                );
            }
            StreamItem::Clock(clock) => println!("clock: {} at {:.1} s", clock.state.video, clock.state.media_time),
            other => println!("{other:?}"),
        }
    }
    let poll = PollOptions { layers: vec![Layer::World], ..Default::default() };
    let mut polled = streams.poll(poll).take(2);
    while let Some(item) = polled.next().await {
        if let StreamItem::Frame(item) = item? {
            println!("polled g {} (cursor {})", item.frame.g, item.frame.cursor);
        }
    }
    Ok(())
}
