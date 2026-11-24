//! Follows the shared session (a recording, or a live camera) and prints what happens on the
//! tray, as the unit's videos say it.
//!
//!     cargo run --example quickstart -- http://factory-orin.local:8120
use futures::StreamExt;
use pdigger_factory_sdk::FactoryClient;

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    let mut events = factory.interpreter().events().take(20); // drop .take(20) to follow for ever
    while let Some(event) = events.next().await {
        println!("{}", event?.describe()); // 5:01.6 Right hand lifts stone 643
    }
    Ok(())
}
