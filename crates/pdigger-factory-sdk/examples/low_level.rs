//! The low level: one method per API operation, typed replies, and the codec of the maps.
//!
//!     cargo run --example low_level -- http://factory-orin.local:8120
use pdigger_factory_sdk::api::{FrameQuery, FrameSel, RenderQuery};
use pdigger_factory_sdk::codec::decode_u8_map;
use pdigger_factory_sdk::{FactoryClient, Layer};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let api = FactoryClient::new(&url)?.api();
    let root = api.get_api_root().await?;
    println!("API {}, build {}", root.api, root.build);
    let session = api.get_session().await?;
    println!("session {} plays {}, {} viewers", session.state.session, session.state.video, session.viewers);
    let query = FrameQuery { layers: vec![Layer::World, Layer::Overlay], ..Default::default() };
    let frame = api.get_frame(FrameSel::Latest, &query).await?;
    let stones = frame.world.as_ref().map_or(0, |w| w.stones.len());
    println!("newest frame: g {} at {:.1} s, {stones} stones", frame.g, frame.t);
    if let Some(overlay) = &frame.overlay {
        let regions = &overlay.regions;
        if let Some(map) = decode_u8_map(regions.stone.as_deref(), regions.rows, regions.cols)? {
            println!("stone map {}x{}: {} cells with a stone", regions.rows, regions.cols, map.count(None));
        }
    }
    let draw =
        api.render_overlay2d(&RenderQuery { session: Some(frame.session), ..RenderQuery::at_g(frame.g) }).await?;
    println!(
        "overlay2d: {} stones, {} boxes, {} wrists (g {})",
        draw.stones.len(),
        draw.boxes.len(),
        draw.wrists.len(),
        draw.g
    );
    Ok(())
}
