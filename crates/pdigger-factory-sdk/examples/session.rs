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
