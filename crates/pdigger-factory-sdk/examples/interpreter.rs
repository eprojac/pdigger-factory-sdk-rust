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
