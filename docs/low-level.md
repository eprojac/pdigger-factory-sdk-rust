# The low level

Use `FactoryClient::api()` to request a specific frame, fetch a model output or control the shared session. The 32
typed methods correspond to the [OpenAPI operations](../spec/openapi.yaml). The codec decodes packed maps, bitsets,
depth and joint data.

<!-- example: crates/pdigger-factory-sdk/examples/low_level.rs -->
```rust
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
```

## The operations

| `operationId` | Request | `Api` method → reply |
|---|---|---|
| `getApiRoot` | `GET /api/v2` | `get_api_root()` → `ApiRoot` |
| `getOpenApi` | `GET /api/v2/openapi.yaml` | `get_openapi()` → `String` |
| `getModel` | `GET /api/v2/model` | `get_model()` → `ModelInfo` |
| `getTopology` | `GET /api/v2/topology` | `get_topology()` → `Topology` |
| `getHealth` | `GET /api/v2/health` | `get_health()` → `Health` |
| `getReady` | `GET /api/v2/ready` | `get_ready()` → `Ready` |
| `getStatus` | `GET /api/v2/status` | `get_status()` → `Status` |
| `getMetrics` | `GET /metrics` | `get_metrics()` → `Metrics` (every sample parsed) |
| `listVideos` | `GET /api/v2/videos` | `list_videos()` → `VideoList`; `list_videos_if_changed(etag)` → `Cached<VideoList>` |
| `getVideo` | `GET /api/v2/videos/{id}` | `get_video(id)` → `VideoEntry` |
| `getVideoMedia` | `GET /api/v2/videos/{id}/media` | `get_video_media(id, query)` → `Media` read into memory (one byte range, `If-Range`, `If-None-Match`; a camera has none: 404 `not_found`; a reply that would never end is refused with `not_seekable`); `open_video_media(id, query)` → `MediaStream` (headers, then the body as a `Stream` of chunks); `head_video_media(id)` |
| `getSession` | `GET /api/v2/session` | `get_session()` → `SessionReply` |
| `sessionSelect` | `POST /api/v2/session/select` | `session_select(request, dry_run)` → `ControlReply` |
| `sessionSeek` | `POST /api/v2/session/seek` | `session_seek(request, dry_run)` → `ControlReply` |
| `sessionPlay` | `POST /api/v2/session/play` | `session_play(dry_run)` → `ControlReply` |
| `sessionPause` | `POST /api/v2/session/pause` | `session_pause(dry_run)` → `ControlReply` |
| `getFocus` | `GET /api/v2/session/focus` | `get_focus()` → `FocusReply` |
| `putFocus` | `PUT /api/v2/session/focus` | `put_focus(request, dry_run)` → `ControlReply` |
| `deleteFocus` | `DELETE /api/v2/session/focus` | `delete_focus(dry_run)` → `ControlReply` |
| `getWorld` | `GET /api/v2/session/world` | `get_world(precision)` → `WorldLayer` |
| `getWorldEvents` | `GET /api/v2/session/world/events` | `get_world_events(query)` → `EventLog` |
| `getStone` | `GET /api/v2/session/world/stones/{id}` | `get_stone(id)` → `StoneTrack` |
| `getFrame` | `GET /api/v2/session/frames/{g}` | `get_frame(g or FrameSel::Latest, query)` → `Frame` |
| `getFramePicture` | `GET /api/v2/session/frames/{g}/image.jpg` | `get_frame_picture(g, session)` → `Picture` (`g` from `X-Interp-G`, `jpeg`): the camera picture a live frame was interpreted from (404 `not_found` on a recording) |
| `getFrameRaster` | `GET /api/v2/session/frames/{g}/{raster}.png` | `get_frame_raster(g, raster, scale)` → PNG `Bytes` |
| `pollFrames` | `GET /api/v2/session/frames` | `poll_frames(query)` → `LiveReply` |
| `streamFrames` | `GET /api/v2/session/frames/stream` | `stream_frames(query)` → `EventStream` (SSE or NDJSON events) |
| `renderOverlay2d` | `GET /api/v2/session/render/overlay2d` | `render_overlay2d(query)` → `Overlay2D` |
| `renderScene3d` | `GET /api/v2/session/render/scene3d` | `render_scene3d(query)` → `Scene3D` |
| `rawStream` | `GET /api/v2/raw/stream` | `raw_stream(query)` → `EventStream` |
| `rawPacket` | `GET /api/v2/raw/packets/{g}` | `raw_packet(g or FrameSel::Latest, session)` → `Sip1Packet` |
| `rawTensors` | `GET /api/v2/raw/tensors/{g}` | `raw_tensors(g)` → the tensors' bytes |

The queries are plain structs with `Default` (`FrameQuery`, `StreamQuery`, `PollQuery`, `RenderQuery` with
`RenderQuery::at_g` and `at_t`, `WorldEventsQuery`, `MediaQuery` with `ByteRange`). The controls take `dry_run`; the
`Session` helpers ([concepts.md](concepts.md)) wrap them. A render or raw read can carry a `session` guard: when the
session changed, the unit answers `410 session_gone` instead of another session's data.

## Models

`models` has a type for every schema of the document, with the API's field names. Every struct also keeps the fields
this SDK version does not know in `extra` (a JSON map), and every enum has an `Other(String)` variant for values it
does not know (`WireEnum::as_str()` gives the wire name back), so a newer unit does not break an older SDK. Frames
from public recordings have no face keypoints, appearance masks or depth textures (`redacted` in the recording's
header); those fields are `Option`s.

## Errors

<!-- example: crates/pdigger-factory-sdk/examples/errors.rs -->
```rust
//! Errors: one type with a stable code to branch on. GET requests are retried (connection
//! errors, 429, 502, 503, 504) before an error reaches you; a control only on 429.
//!
//!     cargo run --example errors -- http://factory-orin.local:8120
use std::time::Duration;

use pdigger_factory_sdk::api::FrameQuery;
use pdigger_factory_sdk::models::EventKind;
use pdigger_factory_sdk::{FactoryClient, WireEnum};

#[tokio::main]
async fn main() -> pdigger_factory_sdk::Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "http://factory-orin.local:8120".to_string());
    let factory = FactoryClient::new(&url)?;
    // An API error: branch on the code, never on the message.
    match factory.api().get_frame(1, &FrameQuery::default()).await {
        Ok(frame) => println!("frame 1 is still held (session {})", frame.session),
        Err(e) if e.is_not_retained() => {
            println!("frame 1: HTTP {} {}", e.status.unwrap_or(0), e.code.as_str());
            if let Some(held) = e.retained() {
                println!("the unit holds frames {:?} to {:?} of session {}", held.g0, held.g1, held.session);
            }
        }
        Err(e) => return Err(e),
    }
    // A control the unit refuses, checked as a dry run: the shared session does not move.
    if let Err(e) = factory.session().dry_run().seek(-5.0).await {
        println!("seek to -5 s: {} (retryable: {})", e.code.as_str(), e.is_retryable());
    }
    // Time limits: wait_for gives Ok(None) when nothing came in time.
    let split = factory.interpreter().wait_for(EventKind::Split, Duration::from_millis(500)).await?;
    println!("a split within 0.5 s: {}", split.is_some());
    Ok(())
}
```

`Error` has `status` (the HTTP status, if there was a reply), `code` (`ErrorCode`: the API's stable codes and the
SDK's own `transport`, `decode`, `live_unavailable`, `live_timeout` and `stream_closed`), `message`, `field`, `retry_ms`,
`retained()`, `waiting()` (what a `503 not_ready` waits for), `retry_after_s()`, and the underlying `cause()` of a
transport error. Helpers: `is_retryable()`, `is_not_retained()`, `is_session_gone()`, `is_live_unavailable()`,
`retry_delay()`. Branch on the code, never on the message.

**Retries.** GET and HEAD requests are retried on connection errors, 429, 502, 503 and 504; the other methods on 429
only. `ClientBuilder::retries` (3) and `ClientBuilder::backoff` (`Backoff { base: 250 ms, factor: 2, max: 5 s }`,
full jitter) set how; the server's `retry_ms` or `Retry-After` wins when it gives one. The jitter's random source and
the clock are injectable for tests.

## The codec

`codec` decodes what the layers pack, with the contract's exact formulas:

| Function | Decodes |
|---|---|
| `decode_u8_map(b64, rows, cols)` | an overlay map (base64 of `rows × cols` bytes) into a `Grid` (`at`, `row`, `count`) |
| `decode_zlib_map(b64, rows, cols)` | a `sip1` map (base64 of zlib) |
| `decode_bitset(b64, n)` | `n` booleans, least significant bit first |
| `depth_log_z`, `depth_z`, `decode_depth` | quantised log depth to `Z` |
| `decode_f32le`, `decode_u16le` | surface positions and triangle indices |
| `Joint::from_values`, `joints` | `[u, v, vis, x, y, z]` joints (or null) |
| `StoneItem::from_values`, `stone_items` | `[u, v, w, h, score, X, Y, Z, w_m, h_m, yaw]` stone tuples |
| `uv_to_px`, `px_to_uv`, `scene_to_three`, `three_to_scene`, `quat_to_yaw` | coordinates |
| `Skeleton::new(topology)` | the skeleton of `get_topology`: wrists, finger tips, the bones between visible joints |

## Transports

The default transport is reqwest with rustls (`ReqwestTransport`). `ClientBuilder::transport` takes any
`Arc<dyn Transport>`: one async `send(Request) -> Result<Response>` whose response body is a byte stream. The fake unit
of the `testing` feature is one ([testing.md](testing.md)).
