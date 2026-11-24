//! The opt-in integration tests (TESTING.md). They are `#[ignore]`d, so the unit-test run
//! (`make test`) never runs them, and each also returns at once unless its environment variable
//! is set:
//!
//! ```text
//! PDIGGER_FACTORY_URL=http://<unit>:8120 cargo test -p pdigger-factory-sdk --test integration -- --ignored --nocapture
//! ```
//!
//! checks a unit read-only: every GET operation, streams and long-polls that observe
//! (`observe=1`: they do not count as a viewer, so they never wake an idle worker), and every
//! control as a dry run (`dry_run=1`). It never changes the shared session.
//!
//! ```text
//! PDIGGER_FACTORY_RECORDINGS=<dir> cargo test -p pdigger-factory-sdk --release --test integration -- --ignored --nocapture
//! ```
//!
//! replays every `<video>.world.ndjson.gz` in `<dir>` and compares its summary with
//! `<dir>/<video>.summary.json` (and the text with `<dir>/<video>.summary.txt` when there is
//! one), or else with `fixtures/golden/complete/<video>.summary.json`.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::{Stream, StreamExt};
use pdigger_factory_sdk::api::{ByteRange, FrameQuery, FrameSel, MediaQuery, RenderQuery, WorldEventsQuery};
use pdigger_factory_sdk::models::{ChangedBy, LivePhase, Raster, RenderSurface, SourceKind};
use pdigger_factory_sdk::streams::PollOptions;
use pdigger_factory_sdk::{Error, ErrorCode, FactoryClient, FramesOptions, Layer, Replay, Result, StreamItem};

use common::{fixtures, normalise};

/// A client of the unit named by `PDIGGER_FACTORY_URL`, observing; `None` (the test is skipped)
/// without one.
fn unit() -> Option<FactoryClient> {
    let Some(base) = std::env::var("PDIGGER_FACTORY_URL").ok().filter(|b| !b.is_empty()) else {
        eprintln!("PDIGGER_FACTORY_URL is not set: no unit to check");
        return None;
    };
    Some(FactoryClient::builder(&base).observe(true).user_agent_suffix("integration").build().unwrap())
}

/// The unit has no frame to give (an idle worker: nobody is watching).
fn no_frames(e: &Error) -> bool {
    e.is_not_retained() || matches!(e.code, ErrorCode::NotReady | ErrorCode::WorldUnavailable)
}

/// `Ok`, or one of the errors `allowed`.
fn ok_or<T>(what: &str, result: Result<T>, allowed: impl Fn(&Error) -> bool) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) if allowed(&e) => {
            eprintln!("{what}: {e}");
            None
        }
        Err(e) => panic!("{what}: {e}"),
    }
}

const ALL_LAYERS: [Layer; 5] = [Layer::Raw, Layer::Overlay, Layer::Scene, Layer::Cards, Layer::World];

#[tokio::test]
#[ignore = "opt-in: needs a unit at PDIGGER_FACTORY_URL"]
async fn the_unit_read_only() {
    let Some(factory) = unit() else { return };
    let api = factory.api();
    let root = api.get_api_root().await.unwrap();
    assert!(root.api.starts_with("2."), "API {}", root.api);
    eprintln!("API {}, build {}, boot {}", root.api, root.build, root.boot);
    assert!(api.get_openapi().await.unwrap().contains("openapi:"));
    api.get_model().await.unwrap();
    let topology = api.get_topology().await.unwrap();
    assert!(topology.joints > 0 && !topology.bones.is_empty());
    api.get_health().await.unwrap();
    ok_or("ready", api.get_ready().await, |e| e.code == ErrorCode::NotReady);
    api.get_status().await.unwrap();
    assert!(!api.get_metrics().await.unwrap().samples.is_empty());

    // Sources, the cameras, media (a camera has none).
    let sources = factory.sources().await.unwrap();
    assert!(!sources.is_empty());
    for source in &sources {
        api.get_video(&source.id).await.unwrap();
        if source.is_live() {
            continue;
        }
        let media = api
            .get_video_media(&source.id, &MediaQuery { range: Some(ByteRange::Span(0, 1023)), ..Default::default() })
            .await
            .unwrap();
        assert_eq!((media.status, media.data.len()), (206, 1024), "media {}", source.id);
    }
    if let Some(live) = ok_or("live", factory.live().await, Error::is_live_unavailable) {
        eprintln!("the unit's first live camera: {:?} ({})", live.id, live.station.as_deref().unwrap_or("no station"));
    }

    let session = factory.session();
    let before = session.state().await.unwrap();
    session.clock().await.unwrap();
    api.get_focus().await.unwrap();

    // The newest frame and what is derived from it; an idle unit may have none.
    let query = FrameQuery { layers: ALL_LAYERS.to_vec(), ..Default::default() };
    if let Some(frame) = ok_or("frame", api.get_frame(FrameSel::Latest, &query).await, no_frames) {
        let guard = Some(frame.session);
        ok_or("raw packet", api.raw_packet(frame.g, guard).await, no_frames);
        if let Some(png) = ok_or("raster", api.get_frame_raster(frame.g, &Raster::Regions, None).await, no_frames) {
            assert!(png.starts_with(b"\x89PNG"), "a PNG");
        }
        ok_or("tensors", api.raw_tensors(frame.g).await, |e| no_frames(e) || e.code == ErrorCode::DebugDisabled);
        let render = RenderQuery { session: guard, ..RenderQuery::at_g(frame.g) };
        ok_or("overlay2d", api.render_overlay2d(&render).await, |e| no_frames(e) || e.is_session_gone());
        let scene = RenderQuery { surface: Some(RenderSurface::Compact), ..render };
        ok_or("scene3d", api.render_scene3d(&scene).await, |e| no_frames(e) || e.is_session_gone());
    }
    if let Some(world) = ok_or("world", api.get_world(None).await, no_frames) {
        api.get_world_events(&WorldEventsQuery { limit: Some(16), ..Default::default() }).await.unwrap();
        if let Some(stone) = world.stones.first() {
            ok_or("stone", api.get_stone(stone.id).await, |e| e.code == ErrorCode::UnknownStone);
            let focus = session.dry_run().focus(stone.id).await;
            ok_or("focus dry run", focus, |e| matches!(e.code, ErrorCode::UnknownStone | ErrorCode::StaleSession));
        }
    }

    // Every control as a dry run: the unit checks it and changes nothing.
    let dry = session.dry_run();
    dry.pause().await.unwrap();
    dry.play().await.unwrap();
    dry.clear_focus().await.unwrap();
    if !before.is_live() {
        ok_or("seek dry run", dry.seek(before.media_time).await, |e| e.code == ErrorCode::TimeOutOfRange);
    }
    dry.watch(&before.video).await.unwrap();
    let refused = dry.seek(-5.0).await.unwrap_err();
    assert!(matches!(refused.code, ErrorCode::TimeOutOfRange | ErrorCode::NotSeekable), "a seek to -5 s: {refused}");
    let after = session.state().await.unwrap();
    if after.session != before.session && after.changed_by == ChangedBy::Viewer {
        eprintln!(
            "the session changed during the test ({} -> {}): another client moved it",
            before.session, after.session
        );
    }
}

#[tokio::test]
#[ignore = "opt-in: needs a unit at PDIGGER_FACTORY_URL"]
async fn the_unit_dry_runs_watching_a_camera_and_serves_pictures_only_for_one() {
    let Some(factory) = unit() else { return };
    let cameras = factory.cameras().await.unwrap();
    let Some(camera) = cameras.first() else {
        eprintln!("the unit lists no camera");
        return;
    };
    let shape = (camera.kind(), camera.is_live(), camera.is_seekable(), camera.duration);
    assert_eq!(shape, (SourceKind::Camera, true, false, None), "{camera:?}");
    let session = factory.session();
    let before = session.state().await.unwrap();
    let dry = session.dry_run();
    let would = dry.watch(&camera.id).await.unwrap();
    let got = (would.video.as_str(), would.rate, would.duration, would.media_time);
    assert_eq!(got, (camera.id.as_str(), 1.0, None, 0.0));
    assert_eq!(would.live_phase(), Some(LivePhase::Connecting));
    let timed = dry.watch_at(&camera.id, Some(1.0)).await.unwrap_err();
    assert_eq!((timed.code, timed.field.as_deref()), (ErrorCode::NotSeekable, Some("time")));
    let media = factory.api().get_video_media(&camera.id, &MediaQuery::default()).await.unwrap_err();
    assert_eq!((media.status, media.code), (Some(404), ErrorCode::NotFound), "a camera has no media");
    if before.is_live() {
        let picture = session.picture_in(FrameSel::Latest, before.session).await;
        if let Some(picture) = ok_or("picture", picture, |e| no_frames(e) || e.is_session_gone()) {
            assert!(picture.jpeg.starts_with(&[0xff, 0xd8]), "a JPEG");
        }
    } else {
        let error = session.picture(FrameSel::Latest).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound, "a recording's picture is its proxy video");
    }
    let after = session.state().await.unwrap();
    if after.session != before.session {
        eprintln!(
            "the session changed during the test ({} -> {}): another client moved it",
            before.session, after.session
        );
    }
}

/// Items of `stream` that `wanted` accepts, up to three, within 20 s (fewer from an idle unit);
/// an error the stream yields fails the test (the errors a resume rides out are not yielded).
async fn count(name: &str, stream: impl Stream<Item = Result<StreamItem>>, wanted: fn(&StreamItem) -> bool) -> usize {
    let mut stream = std::pin::pin!(stream);
    let mut n = 0;
    let _ = tokio::time::timeout(Duration::from_secs(20), async {
        while n < 3 {
            match stream.next().await {
                Some(Ok(item)) => n += usize::from(wanted(&item)),
                Some(Err(error)) => panic!("{name}: {error}"),
                None => break,
            }
        }
    })
    .await;
    n
}

/// A few items of each stream, observing; an idle unit sends none, which is not an error (nobody
/// is watching, so nothing is interpreted).
#[tokio::test]
#[ignore = "opt-in: needs a unit at PDIGGER_FACTORY_URL"]
async fn the_unit_streams() {
    let Some(factory) = unit() else { return };
    let streams = factory.streams();
    let world = FramesOptions { layers: vec![Layer::World], observe: Some(true), ..Default::default() };
    let frame = |item: &StreamItem| matches!(item, StreamItem::Frame(_));
    let packet = |item: &StreamItem| matches!(item, StreamItem::Packet(_));
    let poll = PollOptions { layers: vec![Layer::World], ..Default::default() };
    for (name, n) in [
        ("frames", count("frames", streams.frames(world.clone()), frame).await),
        ("poll", count("poll", streams.poll(poll), frame).await),
        (
            "raw packets",
            count(
                "raw packets",
                streams.raw_packets(FramesOptions { observe: Some(true), ..Default::default() }),
                packet,
            )
            .await,
        ),
    ] {
        eprintln!(
            "{name}: {n} items in 20 s{}",
            if n == 0 { " (the worker is idle or the session paused)" } else { "" }
        );
    }
    match factory.interpreter().observe(true).wait_for_match(|_| true, Duration::from_secs(20)).await {
        Ok(Some(event)) => eprintln!("interpreter: {}", event.describe()),
        Ok(None) => eprintln!("interpreter: no event in 20 s (the worker is idle or the session paused)"),
        Err(error) => panic!("interpreter: {error}"),
    }
}

/// `<dir>/<video>.summary.<ext>`, else `fixtures/golden/complete/<video>.summary.<ext>`.
fn complete_golden(dir: &Path, video: &str, ext: &str) -> Option<(String, PathBuf)> {
    let name = format!("{video}.summary.{ext}");
    [dir.join(&name), fixtures().join("golden/complete").join(&name)]
        .into_iter()
        .find_map(|p| std::fs::read_to_string(&p).ok().map(|text| (text, p)))
}

#[test]
#[ignore = "opt-in: needs the complete recordings in PDIGGER_FACTORY_RECORDINGS"]
fn the_complete_recordings() {
    let Some(dir) = std::env::var_os("PDIGGER_FACTORY_RECORDINGS").filter(|d| !d.is_empty()).map(PathBuf::from) else {
        eprintln!("PDIGGER_FACTORY_RECORDINGS is not set: no recordings to replay");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".world.ndjson.gz"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no <video>.world.ndjson.gz in {}", dir.display());
    for file in files {
        let video = file.file_name().unwrap().to_string_lossy().trim_end_matches(".world.ndjson.gz").to_string();
        let Some((want, want_path)) = complete_golden(&dir, &video, "json") else {
            eprintln!("{video}: no golden summary");
            continue;
        };
        let summary = Replay::open(&file).unwrap().summary().unwrap();
        let want: serde_json::Value = serde_json::from_str(&want).unwrap();
        assert_eq!(normalise(serde_json::to_value(&summary).unwrap()), normalise(want), "{}", want_path.display());
        if let Some((text, text_path)) = complete_golden(&dir, &video, "txt") {
            assert_eq!(summary.describe(), text, "{}", text_path.display());
        }
        eprintln!(
            "{video}: {} frames, {} moves, {} push bursts",
            summary.frames, summary.moves.total, summary.push_bursts.total
        );
    }
}
