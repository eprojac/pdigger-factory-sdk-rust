//! The low-level tutorial (docs/tutorials/02-low-level.md) in one program: one call per API
//! operation, typed replies, and the codec for what the replies pack. Every step only reads, or
//! sends a dry run that changes nothing. The frame is the newest one, or the `g` given after the URL.
//!
//!     cargo run --example tutorial_low_level -- http://factory-orin.local:8120 [g]
use pdigger_factory_sdk::api::{FrameQuery, FrameSel, RenderQuery};
use pdigger_factory_sdk::codec::{Skeleton, decode_zlib_map, joints, uv_to_px};
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::models::{FocusRequest, Raster, RenderDraw, RenderSurface};
use pdigger_factory_sdk::{FactoryClient, Layer, Side};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let at = args.next().and_then(|g| g.parse().ok()).map_or(FrameSel::Latest, FrameSel::G);
    let api = FactoryClient::new(&url)?.api();

    // 1. Is it ready, and how fast? While it starts: the error not_ready, with what it waits for
    let ready = api.get_ready().await?;
    let worker = api.get_status().await?.worker;
    let (fps, ms) = (worker.fps.unwrap_or_default(), worker.frame_ms.unwrap_or_default());
    println!("ready {}, worker {}: {fps} frames a second, {ms} ms each", ready.ready, worker.state);

    // 2. The model, and what to draw with
    let model = api.get_model().await?;
    println!("checkpoint {} on {}: {} engines", model.checkpoint, model.backend, model.engines.len());
    let topology = api.get_topology().await?;
    println!("{} joints, {} bones, {} colours", topology.joints, topology.bones.len(), topology.colors.len());

    // 3. A stone's track, and the focus on it as a dry run: the unit checks it, and nothing changes
    let world = api.get_world(None).await?;
    if let Some(first) = world.stones.first() {
        let track = api.get_stone(first.id).await?; // its path, from frame first_g to last_g
        println!("stone {}: {} points, g {} to {}", track.id, track.path2d.len(), track.first_g, track.last_g);
        let focus = api.put_focus(&FocusRequest { stone_id: track.id, ..Default::default() }, true).await?;
        println!("focus on stone {}: dry run {}", track.id, focus.dry_run == Some(true));
    }

    // 4. A frame, and the model's maps of it
    let world_and_cards = FrameQuery { layers: vec![Layer::World, Layer::Cards], ..Default::default() };
    let frame = api.get_frame(at, &world_and_cards).await?;
    let stones = frame.world.as_ref().map_or(&[][..], |w| &w.stones[..]);
    let mut families: Vec<String> = stones.iter().map(|s| s.family.to_string()).collect();
    families.sort();
    families.dedup();
    println!("frame g {} at {}: {} stones, {}", frame.g, clock(frame.t), stones.len(), families.join(", "));
    let packet = api.raw_packet(frame.g, Some(frame.session)).await?; // what the model saw, before the world
    let (rows, cols) = (packet.maps.grid.first().copied().unwrap_or(0), packet.maps.grid.get(1).copied().unwrap_or(0));
    let tray = decode_zlib_map(packet.maps.tray.as_deref(), rows, cols)?; // one byte a cell
    let stone_cells = decode_zlib_map(packet.maps.stone.as_deref(), rows, cols)?;
    let (tray, stone_cells) = (tray.map_or(0, |m| m.count(None)), stone_cells.map_or(0, |m| m.count(None)));
    println!("maps of g {}: {rows} x {cols} cells, {tray} of the tray, {stone_cells} of stones", packet.g);

    // 5. Draw lists, ready to paint: shapes with their style in video pixels, the scene in scene units
    let query = RenderQuery { draw: Some(RenderDraw::Primitives), ..RenderQuery::at_g(frame.g) };
    let overlay = api.render_overlay2d(&query).await?;
    let (polygons, segments, circles) =
        overlay.primitives.as_ref().map_or((0, 0, 0), |p| (p.polygons.len(), p.segments.len(), p.circles.len()));
    let size = overlay.native_wh.iter().map(i64::to_string).collect::<Vec<_>>().join(" x ");
    println!("overlay of g {}: {polygons} polygons, {segments} segments, {circles} circles at {size} px", overlay.g);
    let query = RenderQuery { surface: Some(RenderSurface::Compact), ..RenderQuery::at_g(frame.g) };
    let scene = api.render_scene3d(&query).await?; // the stones, the hands and the work plane
    println!("scene of g {}: {} stones, {} bones", scene.g, scene.stones.len(), scene.skeleton.bones.len());

    // 6. Stone shapes, drawn by hand: each outline a polygon in video pixels, coloured from the topology
    let colour = |detected: bool| topology.colors.get(if detected { "stones" } else { "stoneBox" }).cloned();
    let outlines: Vec<_> = stones.iter().map(|s| (colour(s.detected), &s.outline)).collect(); // to any drawing library
    let points: usize = outlines.iter().map(|(_, outline)| outline.len()).sum();
    println!("g {}: {} outlines, {points} points", frame.g, outlines.len());

    // 7. Hands and bones, drawn by hand: the joints in video pixels, the bones between those in view
    let overlay_and_cards = FrameQuery { layers: vec![Layer::Overlay, Layer::Cards], ..Default::default() };
    let view = api.get_frame(frame.g, &overlay_and_cards).await?;
    let wh = api.get_video(&view.video).await?.native_wh;
    let (w, h) = (wh.first().copied().unwrap_or(0) as f64, wh.get(1).copied().unwrap_or(0) as f64);
    let js = joints(view.overlay.as_ref().map_or(&[][..], |o| &o.joints[..])); // u, v, visibility, x, y, z
    let px: Vec<Option<(f64, f64)>> = js.iter().map(|j| j.as_ref().map(|j| uv_to_px(j.u, j.v, w, h))).collect();
    let skeleton = Skeleton::new(topology.clone());
    let bones = skeleton.visible_bones(&js); // lines to draw, between px[a] and px[b]
    println!("g {}: {} of {} bones", view.g, bones.len(), topology.bones.len());
    for side in [Side::Left, Side::Right] {
        let card = view.cards.as_ref().map(|c| if side == Side::Left { &c.left } else { &c.right });
        let title = card.map_or("no card", |c| c.title.as_str());
        let wrist = skeleton.wrist_joint(&side).and_then(|j| px.get(j).copied().flatten());
        let at = wrist.map_or("not in view".to_string(), |(x, y)| format!("wrist at {x:.0}, {y:.0} px"));
        println!("{side} hand: {title}, {at}");
    }

    // 8. A frame's image data: the model's rasters as PNG, and the picture it was made from
    let png = api.get_frame_raster(frame.g, &Raster::Regions, None).await?; // also Mask and Depth
    println!("regions of g {}: a {}-byte PNG", frame.g, png.len());
    let source = api.get_video(&frame.video).await?;
    if source.is_live() {
        let picture = api.get_frame_picture(frame.g, Some(frame.session)).await?; // a camera's own JPEG
        println!("picture: a {}-byte JPEG of g {}", picture.jpeg.len(), picture.g);
    } else {
        let size = source.proxy_wh.iter().map(i64::to_string).collect::<Vec<_>>().join(" x ");
        println!("picture: {} at {}, {size}", source.proxy, clock(frame.t)); // a recording's video at frame.t
    }
    Ok(())
}
