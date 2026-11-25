//! The low level: every operation of the spec, typed, against the recorded 20.20 replies
//! (`fixtures/api`) and against a fake synthesising from a recording.

mod common;

use std::time::Duration;

use futures::StreamExt;
use pdigger_factory_sdk::api::{
    ByteRange, ContentRange, FrameQuery, FrameSel, MediaQuery, PollQuery, RenderAt, RenderQuery, StreamQuery,
    WorldEventsQuery,
};
use pdigger_factory_sdk::models::*;
use pdigger_factory_sdk::testing::{FakeFactory, Fault, Reply};
use pdigger_factory_sdk::{ErrorCode, FactoryClient};

use common::{api_text, recording_fake, replies_fake};

#[tokio::test]
async fn meta_operations_parse_the_recorded_replies() {
    let fake = replies_fake();
    let api = fake.client().api();
    assert_eq!(api.client().base_url(), FakeFactory::URL);
    let root = api.get_api_root().await.unwrap();
    assert_eq!(root.build, "claude-20261122-hr4x1");
    assert_eq!(api.get_openapi().await.unwrap(), api_text("getOpenApi.yaml"));
    let model = api.get_model().await.unwrap();
    assert_eq!(model.checkpoint, "8d09531c915f");
    assert!(!model.engines.is_empty());
    let topology = api.get_topology().await.unwrap();
    assert_eq!(topology.joints, 70);
    assert_eq!(topology.color("left"), Some([0xf6, 0x8c, 0x36]));
    assert!(api.get_health().await.unwrap().ok);
    assert!(api.get_ready().await.unwrap().ready);
    let status = api.get_status().await.unwrap();
    assert_eq!(status.viewers, 0);
    let metrics = api.get_metrics().await.unwrap();
    assert_eq!(metrics.value("interp_frames_total", &[("video", "dry1")]), Some(9425.0));
    assert_eq!(metrics.text, api_text("getMetrics.txt"));
}

#[tokio::test]
async fn sources_and_media() {
    let fake = replies_fake();
    let factory = fake.client();
    let api = factory.api();
    let list = api.list_videos().await.unwrap();
    let cameras = ["CAM00000001", "CAM00000002", "CAM00000003", "CAM00000004", "CAM00000005", "CAM00000006"];
    assert_eq!(
        list.videos.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
        [["aug28", "jul30", "dry1", "dry2"].as_slice(), &cameras].concat(),
        "the four recordings, then the six plant cameras"
    );
    let cached = api.list_videos_if_changed(None).await.unwrap();
    let etag = cached.etag.clone().unwrap();
    let again = api.list_videos_if_changed(Some(&etag)).await.unwrap();
    assert_eq!(again.value, None);
    assert_eq!(again.etag.as_deref(), Some(etag.as_str()));
    assert_eq!(factory.sources().await.unwrap().len(), 10);
    let dry1 = factory.source("dry1").await.unwrap();
    assert_eq!(dry1.hold, Some(2.2));
    assert!(dry1.is_seekable() && !dry1.is_live());
    let missing = api.get_video("nope").await.unwrap_err();
    assert_eq!((missing.status, missing.code.clone()), (Some(404), ErrorCode::UnknownVideo));
    let first = factory.live().await.unwrap();
    assert_eq!((first.id.as_str(), first.is_live()), (cameras[0], true), "the first camera of the recorded catalog");

    let range = MediaQuery { range: Some(ByteRange::Span(0, 1023)), ..MediaQuery::default() };
    let media = api.get_video_media("dry1", &range).await.unwrap();
    assert_eq!((media.status, media.data.len()), (206, 1024));
    assert_eq!(media.content_range, Some(ContentRange { start: Some(0), end: Some(1023), total: Some(184_866_573) }));
    assert_eq!(media.etag.as_deref(), Some("\"b04d70d-6af0f48f\""));
    assert_eq!(media.accept_ranges.as_deref(), Some("bytes"));
    assert_eq!(media.content_type.as_deref(), Some("video/mp4"));
    let tail = api
        .get_video_media(
            "dry1",
            &MediaQuery {
                range: Some(ByteRange::From(1000)),
                if_range: Some("\"b04d70d-6af0f48f\"".into()),
                ..MediaQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(tail.data.len(), 24);
    let past = api
        .get_video_media("dry1", &MediaQuery { range: Some(ByteRange::Last(10)), ..MediaQuery::default() })
        .await
        .unwrap_err();
    assert_eq!(past.status, Some(416));
    let bad = api
        .get_video_media("dry1", &MediaQuery { range: Some(ByteRange::Span(9, 3)), ..MediaQuery::default() })
        .await
        .unwrap_err();
    assert_eq!(
        (bad.code.clone(), bad.field.as_deref(), bad.status),
        (ErrorCode::InvalidParam, Some("range"), Some(416))
    );
    let whole = api.get_video_media("dry1", &MediaQuery::default()).await.unwrap();
    assert_eq!((whole.status, whole.content_length), (200, Some(1024)));
    let unchanged = api
        .get_video_media("dry1", &MediaQuery { if_none_match: media.etag.clone(), ..MediaQuery::default() })
        .await
        .unwrap();
    assert_eq!((unchanged.status, unchanged.data.len()), (304, 0));
    let head = api.head_video_media("dry1").await.unwrap();
    assert_eq!((head.status, head.content_length, head.data.len()), (200, Some(184_866_573), 0));
    assert_eq!(api.get_video_media("nope", &MediaQuery::default()).await.unwrap_err().code, ErrorCode::UnknownVideo);
    let legacy = api.get_video_media("live", &MediaQuery::default()).await.unwrap_err();
    assert_eq!(
        (legacy.status, legacy.code),
        (Some(404), ErrorCode::UnknownVideo),
        "`live` is sent: no catalog lists it"
    );
    assert_eq!(ByteRange::Span(1, 2).header(), "bytes=1-2");
    assert_eq!(ContentRange::parse("bytes */100"), Some(ContentRange { start: None, end: None, total: Some(100) }));
    assert_eq!(ContentRange::parse("items 1-2/3"), None);
    assert_eq!(ContentRange::parse("bytes 1-2"), None);
    let no_media = FakeFactory::new().client().api().get_video_media("x", &MediaQuery::default()).await.unwrap_err();
    assert_eq!(no_media.code, ErrorCode::UnknownVideo);
}

#[tokio::test]
async fn session_world_and_focus() {
    let fake = replies_fake();
    let api = fake.client().api();
    let session = api.get_session().await.unwrap();
    assert_eq!(session.state.video, "dry2");
    assert_eq!(session.state.session, 1_795_314_885);
    assert_eq!(api.get_focus().await.unwrap(), FocusReply::default());
    let world = api.get_world(Some(Precision::Full)).await.unwrap();
    assert_eq!(world.schema, "stw1");
    let events = api
        .get_world_events(&WorldEventsQuery {
            after: Some(10),
            limit: Some(50),
            types: vec![EventKind::Lift, EventKind::Place],
        })
        .await
        .unwrap();
    assert_eq!(events.next_after, 2935);
    assert_eq!(events.events[0].event.kind, EventKind::New);
    let stone = api.get_stone(43).await.unwrap();
    assert_eq!((stone.id, stone.first_g, stone.path2d.len()), (43, 3004, 20));
}

#[tokio::test]
async fn frames_rasters_packets_and_renders() {
    let fake = replies_fake();
    let api = fake.client().api();
    let latest = api
        .get_frame(
            FrameSel::Latest,
            &FrameQuery {
                layers: vec![Layer::World, Layer::Cards],
                session: Some(1_795_314_885),
                precision: Some(Precision::Rounded),
            },
        )
        .await
        .unwrap();
    assert_eq!(latest.g, 3079);
    assert!(latest.has(&Layer::World) && latest.has(&Layer::Cards));
    let gone = api.get_frame(0, &FrameQuery::default()).await.unwrap_err();
    assert_eq!(gone.code, ErrorCode::NotRetained, "no recorded reply: the session has no frame 0");
    let packet = api.raw_packet(FrameSel::Latest, None).await.unwrap();
    assert_eq!((packet.g, packet.cursor), (3079, Some(10391)));
    let png = api.get_frame_raster(3079, &Raster::Regions, Some(2)).await.unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let scale = api.get_frame_raster(3079, &Raster::Regions, Some(9)).await.unwrap_err();
    assert_eq!(scale.field.as_deref(), Some("scale"));
    assert_eq!(api.get_frame_raster(3079, &Raster::Mask, None).await.unwrap_err().code, ErrorCode::NotFound);
    let tensors = api.raw_tensors(3079).await.unwrap_err();
    assert_eq!(tensors.code, ErrorCode::DebugDisabled);
    let overlay = api.render_overlay2d(&RenderQuery::at_g(3079)).await.unwrap();
    assert_eq!((overlay.schema.as_str(), overlay.mode.clone()), ("r2d1", RenderMode::Stable));
    let prims = api
        .render_overlay2d(&RenderQuery {
            at: RenderAt::T(614.1),
            draw: Some(RenderDraw::Primitives),
            mode: Some(RenderMode::Stable),
            ..RenderQuery::default()
        })
        .await
        .unwrap();
    assert!(prims.primitives.is_some());
    let scene =
        api.render_scene3d(&RenderQuery { session: Some(1_795_314_885), ..RenderQuery::at_g(3079) }).await.unwrap();
    assert_eq!(scene.surface.as_ref().unwrap().form, RenderSurface::Vertices);
    let compact = api
        .render_scene3d(&RenderQuery { surface: Some(RenderSurface::Compact), ..RenderQuery::at_t(614.1) })
        .await
        .unwrap();
    assert_eq!(compact.surface.unwrap().form, RenderSurface::Compact);
    let none = api
        .render_scene3d(&RenderQuery { surface: Some(RenderSurface::None), ..RenderQuery::at_g(3079) })
        .await
        .unwrap();
    assert!(none.surface.is_none());
    let surface = api
        .render_scene3d(&RenderQuery { surface: Some(RenderSurface::Other("mesh".into())), ..RenderQuery::at_g(1) })
        .await
        .unwrap_err();
    assert_eq!(surface.message, "surface must be vertices, compact or none");
    let gone = api.render_overlay2d(&RenderQuery { session: Some(1), ..RenderQuery::at_g(1) }).await.unwrap_err();
    assert!(gone.is_session_gone());
    let empty = FakeFactory::new().client().api().render_overlay2d(&RenderQuery::at_g(1)).await.unwrap_err();
    assert_eq!(empty.code, ErrorCode::NotFound);
    assert_eq!(RenderAt::default(), RenderAt::G(0));
    assert_eq!(FrameSel::from(5), FrameSel::G(5));
}

#[tokio::test]
async fn poll_and_stream_replies() {
    let fake = replies_fake();
    let api = fake.client().api();
    let poll = api
        .poll_frames(&PollQuery {
            cursor: Some(10389),
            rev: Some(429),
            cid: Some("tab-12345".into()),
            layers: vec![Layer::World],
            limit: Some(2),
            wait_ms: Some(10),
            precision: None,
        })
        .await
        .unwrap();
    assert_eq!((poll.cursor, poll.frames.len()), (10391, 2));
    assert_eq!(poll.session().viewers, poll.viewers);
    let mut sse = api
        .stream_frames(&StreamQuery {
            layers: vec![Layer::World, Layer::Cards],
            observe: true,
            ..StreamQuery::default()
        })
        .await
        .unwrap();
    let first = sse.next().await.unwrap().unwrap();
    assert_eq!((first.event.as_str(), first.id.as_deref()), ("frame", Some("a2011b037b-10765")));
    assert_eq!(sse.count().await, 2);
    let ndjson =
        api.stream_frames(&StreamQuery { format: StreamFormat::Ndjson, ..StreamQuery::default() }).await.unwrap();
    assert_eq!(ndjson.count().await, 3);
    let raw = api
        .raw_stream(&StreamQuery { last_event_id: Some("a2011b037b-10773".into()), ..StreamQuery::default() })
        .await
        .unwrap();
    let events: Vec<_> = raw.collect().await;
    assert_eq!(events.len(), 2);
    let raw_nd = api
        .raw_stream(&StreamQuery {
            format: StreamFormat::Ndjson,
            after: Some(1),
            limit: Some(2),
            max_s: Some(5),
            ..StreamQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(raw_nd.count().await, 2);
    assert!(
        fake.requests().iter().any(|r| r.url.contains("observe=1") && r.header("accept") == Some("text/event-stream"))
    );
    assert!(fake.requests().iter().any(|r| r.header("last-event-id") == Some("a2011b037b-10773")));
}

#[tokio::test]
async fn controls_and_their_errors() {
    let fake = recording_fake("dry1-0296-0306-full");
    let api = fake.client().api();
    let before = api.get_session().await.unwrap().state;
    let dry = api.session_select(&SelectRequest { video: "dry1".into(), time: Some(300.0) }, true).await.unwrap();
    assert_eq!(dry.dry_run, Some(true));
    assert_eq!(dry.state.session, before.session + 1);
    assert_eq!(api.get_session().await.unwrap().state.session, before.session);
    let unknown = api.session_select(&SelectRequest { video: "nope".into(), time: None }, true).await.unwrap_err();
    assert_eq!(
        (unknown.code.clone(), unknown.message.as_str(), unknown.field.as_deref()),
        (ErrorCode::UnknownVideo, "Unknown video 'nope'", Some("video"))
    );
    let range = api.session_seek(&SeekRequest { video: None, time: 99_999.0 }, true).await.unwrap_err();
    assert_eq!(
        (range.code.clone(), range.message.as_str()),
        (ErrorCode::TimeOutOfRange, "time must be within 0..1807.158 s")
    );
    let seek = api.session_seek(&SeekRequest { video: Some("dry1".into()), time: 301.0 }, false).await.unwrap();
    assert_eq!(seek.dry_run, Some(false));
    assert_eq!(seek.state.anchor_media, 301.0);
    let paused = api.session_pause(false).await.unwrap();
    assert!(!paused.state.playing);
    assert!(!api.session_pause(false).await.unwrap().state.playing);
    assert!(api.session_play(true).await.unwrap().state.playing);
    let played = api.session_play(false).await.unwrap();
    assert!(played.state.playing);
    let world = api.get_world(None).await.unwrap();
    let id = world.stones[0].id;
    let focus = api
        .put_focus(&FocusRequest { stone_id: id, g: None, session: Some(played.state.session) }, false)
        .await
        .unwrap();
    assert_eq!(focus.state.focus.as_ref().unwrap().id, id);
    let reply = api.get_focus().await.unwrap();
    assert_eq!(reply.resolved.unwrap().id, id);
    let stale = api.put_focus(&FocusRequest { stone_id: id, g: Some(1), session: Some(1) }, true).await.unwrap_err();
    assert_eq!(stale.code, ErrorCode::StaleSession);
    let no_stone = api.put_focus(&FocusRequest { stone_id: 999_999, g: None, session: None }, true).await.unwrap_err();
    assert_eq!((no_stone.status, no_stone.code.clone()), (Some(404), ErrorCode::UnknownStone));
    let cleared = api.delete_focus(false).await.unwrap();
    assert!(cleared.state.focus.is_none());
    assert!(api.delete_focus(true).await.unwrap().state.focus.is_none());
}

#[tokio::test(start_paused = true)]
async fn retries_follow_the_policy() {
    let fake = replies_fake();
    let api = fake.client().api();
    fake.fail("getModel", Fault::status(503, "busy").retry_ms(40).times(2));
    let start = tokio::time::Instant::now();
    assert_eq!(api.get_model().await.unwrap().checkpoint, "8d09531c915f");
    assert_eq!(start.elapsed(), Duration::from_millis(80));
    fake.fail("getModel", Fault::status(429, "too_many_requests").retry_after("1"));
    let start = tokio::time::Instant::now();
    api.get_model().await.unwrap();
    assert_eq!(start.elapsed(), Duration::from_secs(1));
    fake.fail("getModel", Fault::broken().times(4));
    let broken = api.get_model().await.unwrap_err();
    assert_eq!((broken.code.clone(), broken.status), (ErrorCode::Transport, None));
    fake.fail("getTopology", Fault::status(502, "internal").times(1));
    api.get_topology().await.unwrap();
    fake.fail("getHealth", Fault::status(400, "invalid_param"));
    assert_eq!(api.get_health().await.unwrap_err().status, Some(400));
    fake.fail("sessionPause", Fault::status(503, "busy"));
    assert_eq!(api.session_pause(true).await.unwrap_err().status, Some(503));
    fake.fail("sessionPause", Fault::status(429, "too_many_controls"));
    assert!(api.session_pause(true).await.unwrap().state.revision > 0);
    fake.fail("getReady", Fault::reply(Reply::new(503, "text/html", "<html>bad gateway</html>")));
    let html = api.get_ready().await;
    assert!(html.is_ok(), "retried after the HTML 503");
    let ops = fake.operations();
    assert_eq!(ops.iter().filter(|o| *o == "getModel").count(), 3 + 2 + 4);
}

#[tokio::test]
async fn decode_errors_and_odd_replies() {
    let fake = FakeFactory::new()
        .with_reply("getModel", Reply::new(200, "application/json", "{not json"))
        .with_reply("getOpenApi", Reply::new(200, "application/yaml", vec![0xff, 0xfe]))
        .with_reply("getStatus", Reply::json(200, &serde_json::json!({"build": "x"})))
        .with_reply("getFramePicture", Reply::new(200, "image/jpeg", &b"\xff\xd8\xff\xd9"[..]));
    let api = fake.client().api();
    let unnumbered = api.get_frame_picture(FrameSel::Latest, None).await.unwrap_err();
    assert_eq!((unnumbered.code.clone(), unnumbered.status), (ErrorCode::Decode, Some(200)));
    assert_eq!(unnumbered.message, "the picture reply has no X-Interp-G header");
    let e = api.get_model().await.unwrap_err();
    assert_eq!((e.code.clone(), e.status), (ErrorCode::Decode, Some(200)));
    assert!(e.message.starts_with("not the documented JSON"));
    let text = api.get_openapi().await.unwrap_err();
    assert_eq!(text.message, "the reply is not UTF-8 text");
    assert_eq!(api.get_status().await.unwrap().build, "x");
    assert_eq!(api.get_topology().await.unwrap_err().code, ErrorCode::NotFound);
    let synthesised = api.get_api_root().await.unwrap();
    assert_eq!(synthesised.build, "fake");
    assert!(api.get_health().await.unwrap().ok);
    assert!(api.get_ready().await.unwrap().waiting.is_empty());
}

#[tokio::test]
async fn the_client_sends_its_headers_and_options() {
    let fake = replies_fake();
    let factory = fake
        .client_builder()
        .header("X-Test", "1")
        .bearer_token("secret")
        .user_agent_suffix("tests/1")
        .request_timeout(Duration::from_secs(2))
        .connect_timeout(Duration::from_secs(1))
        .retries(0)
        .observe(true)
        .backoff(pdigger_factory_sdk::Backoff {
            base: Duration::from_millis(1),
            factor: 2.0,
            max: Duration::from_millis(4),
        })
        .build()
        .unwrap();
    assert!(factory.observe_default());
    assert!(format!("{factory:?}").contains("fake.factory.test"));
    factory.api().get_health().await.unwrap();
    let request = fake.requests().pop().unwrap();
    assert_eq!(request.header("x-test"), Some("1"));
    assert_eq!(request.header("authorization"), Some("Bearer secret"));
    assert_eq!(request.header("user-agent"), Some("pdigger-factory-sdk-rust/1.0.0 tests/1"));
    assert_eq!(request.timeout, Some(Duration::from_secs(2)));
    fake.fail("getHealth", Fault::status(503, "busy"));
    assert_eq!(factory.api().get_health().await.unwrap_err().status, Some(503));
    assert!(FactoryClient::new("ftp://x").is_err());
    let trimmed = FactoryClient::new("http://factory-orin.local:8120/").unwrap();
    assert_eq!(trimmed.base_url(), "http://factory-orin.local:8120");
    assert!(trimmed.now() > 1.0e9);
}

#[test]
fn the_backoff_grows_to_its_cap_whatever_the_factor() {
    let default = pdigger_factory_sdk::Backoff::default();
    let caps: Vec<_> = (0..6).map(|k| default.cap(k).as_millis()).collect();
    assert_eq!(caps, [250, 500, 1000, 2000, 4000, 5000]);
    assert_eq!(default.cap(u32::MAX), Duration::from_secs(5), "a long outage: the cap");
    let slow = pdigger_factory_sdk::Backoff { factor: 1.01, ..default };
    assert_eq!(slow.cap(400), Duration::from_secs(5), "a slow growth still reaches the cap");
    let fast = pdigger_factory_sdk::Backoff { factor: 1.0e5, ..default };
    assert_eq!(fast.cap(64), Duration::from_secs(5));
    // a base of zero waits zero at every attempt, also where factor^attempt is infinite (0 * inf is NaN)
    let zero = pdigger_factory_sdk::Backoff { base: Duration::ZERO, ..fast };
    assert_eq!([0, 64, 1024, u32::MAX].map(|k| zero.cap(k)), [Duration::ZERO; 4]);
}

#[tokio::test]
async fn the_recording_fake_synthesises_world_frames_and_tracks() {
    let fake = recording_fake("dry1-0296-0306-full");
    let api = fake.client().api();
    let world = api.get_world(None).await.unwrap();
    assert_eq!(world.g, 1530);
    let frame = api
        .get_frame(1505, &FrameQuery { layers: vec![Layer::World, Layer::Raw], ..FrameQuery::default() })
        .await
        .unwrap();
    assert!(frame.raw.is_some() && frame.world.is_some() && frame.scene.is_none());
    let latest = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    assert_eq!(latest.g, 1530);
    assert!(latest.raw.is_none() && latest.overlay.is_some());
    let not = api.get_frame(5, &FrameQuery::default()).await.unwrap_err();
    assert!(not.is_not_retained());
    assert_eq!(not.retained().unwrap().g0, Some(1480));
    let gone = api.get_frame(1505, &FrameQuery { session: Some(1), ..FrameQuery::default() }).await.unwrap_err();
    assert!(gone.is_session_gone());
    let layer = api
        .get_frame(1505, &FrameQuery { layers: vec![Layer::Other("foo".into())], ..FrameQuery::default() })
        .await
        .unwrap_err();
    assert_eq!((layer.code.clone(), layer.message.as_str()), (ErrorCode::UnknownLayer, "unknown layer 'foo'"));
    let packet = api.raw_packet(1505, Some(world.session)).await.unwrap();
    assert_eq!((packet.g, packet.schema.as_str()), (1505, "sip1"));
    let events = api.get_world_events(&WorldEventsQuery::default()).await.unwrap();
    assert_eq!(events.oldest_g, Some(1480));
    let lifts = api
        .get_world_events(&WorldEventsQuery { types: vec![EventKind::Lift], ..WorldEventsQuery::default() })
        .await
        .unwrap();
    assert_eq!(lifts.events.len(), 1);
    let page = api
        .get_world_events(&WorldEventsQuery { after: Some(1500), limit: Some(1), ..WorldEventsQuery::default() })
        .await
        .unwrap();
    assert!(page.next_after > 1500);
    let bad = api
        .get_world_events(&WorldEventsQuery {
            types: vec![EventKind::Other("nonsense".into())],
            ..WorldEventsQuery::default()
        })
        .await
        .unwrap_err();
    assert_eq!(bad.message, "unknown event type 'nonsense'");
    let limit =
        api.get_world_events(&WorldEventsQuery { limit: Some(5000), ..WorldEventsQuery::default() }).await.unwrap_err();
    assert_eq!(limit.message, "limit must be within 1..1024");
    let stone = api.get_stone(world.stones[0].id).await.unwrap();
    assert_eq!(stone.last_g, 1530);
    assert!(!stone.path3d.is_empty());
    assert_eq!(api.get_stone(999_999).await.unwrap_err().code, ErrorCode::UnknownStone);
    let poll = api.poll_frames(&PollQuery { cursor: Some(0), limit: Some(3), ..PollQuery::default() }).await.unwrap();
    assert_eq!(poll.frames.len(), 3);
    let ahead = api.poll_frames(&PollQuery { cursor: Some(1_000_000), ..PollQuery::default() }).await.unwrap();
    assert!(ahead.frames.is_empty() && ahead.cursor < 1_000_000);
    let limit = api.poll_frames(&PollQuery { limit: Some(17), ..PollQuery::default() }).await.unwrap_err();
    assert_eq!(limit.message, "limit must be within 1..16");
    let cid = api.poll_frames(&PollQuery { cid: Some("x".into()), ..PollQuery::default() }).await.unwrap_err();
    assert_eq!(cid.field.as_deref(), Some("cid"));
}

/// A transport whose media replies never end and have neither a length nor byte ranges.
struct EndlessMedia(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl pdigger_factory_sdk::transport::Transport for EndlessMedia {
    fn send(
        &self,
        _request: pdigger_factory_sdk::transport::Request,
    ) -> futures::future::BoxFuture<'static, pdigger_factory_sdk::Result<pdigger_factory_sdk::transport::Response>>
    {
        use pdigger_factory_sdk::transport::{Body, Headers, Response};
        struct Endless(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for Endless {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let guard = Endless(self.0.clone());
        let chunks = futures::stream::repeat(bytes::Bytes::from_static(b"moof")).map(move |c| {
            let _ = &guard;
            Ok(c)
        });
        let headers = Headers(vec![("Content-Type".into(), "video/mp4".into())]);
        Box::pin(async move { Ok(Response { status: 200, headers, body: Body::from_stream(chunks) }) })
    }
}

#[tokio::test]
async fn media_without_an_end_is_refused_unread() {
    let closed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let factory = FactoryClient::builder("http://factory.test")
        .transport(std::sync::Arc::new(EndlessMedia(closed.clone())))
        .build()
        .unwrap();
    let error = factory.api().get_video_media("cam2", &MediaQuery::default()).await.unwrap_err();
    assert_eq!(
        (error.code.clone(), error.status, error.field.as_deref()),
        (ErrorCode::NotSeekable, Some(200), Some("id"))
    );
    assert!(error.message.contains("open_video_media"));
    assert!(closed.load(std::sync::atomic::Ordering::SeqCst), "the connection is closed unread");
    let mut stream = factory.api().open_video_media("cam2", &MediaQuery::default()).await.unwrap();
    assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"moof");
}

/// A transport whose error replies break while their body is read.
struct BrokenErrorBody;

impl pdigger_factory_sdk::transport::Transport for BrokenErrorBody {
    fn send(
        &self,
        _request: pdigger_factory_sdk::transport::Request,
    ) -> futures::future::BoxFuture<'static, pdigger_factory_sdk::Result<pdigger_factory_sdk::transport::Response>>
    {
        use pdigger_factory_sdk::transport::{Body, Headers, Response};
        Box::pin(async {
            let chunks = futures::stream::iter([
                Ok(bytes::Bytes::from_static(b"{\"err")),
                Err(pdigger_factory_sdk::Error::transport(std::io::Error::other("reset"))),
            ]);
            Ok(Response { status: 500, headers: Headers(Vec::new()), body: Body::from_stream(chunks) })
        })
    }
}

#[tokio::test]
async fn tensors_escaped_ids_and_unreadable_error_bodies() {
    let fake =
        FakeFactory::new().with_reply("rawTensors", Reply::new(200, "application/octet-stream", &b"PK\x03\x04"[..]));
    let api = fake.client().api();
    assert_eq!(&api.raw_tensors(7).await.unwrap()[..], b"PK\x03\x04");
    let unknown = api.get_video("dry 1/x").await.unwrap_err();
    assert_eq!(unknown.status, Some(404));
    assert!(fake.requests().last().unwrap().url.ends_with("/api/v2/videos/dry%201%2Fx"), "path segments are escaped");
    let broken = FactoryClient::builder("http://factory.test")
        .transport(std::sync::Arc::new(BrokenErrorBody))
        .retries(0)
        .build()
        .unwrap();
    let error = broken.api().get_health().await.unwrap_err();
    assert_eq!(
        (error.code.clone(), error.status),
        (ErrorCode::Transport, Some(500)),
        "the status of a reply whose body broke"
    );
    assert!(std::error::Error::source(&error).is_some());
    assert!(std::error::Error::source(&pdigger_factory_sdk::Error::new(ErrorCode::Decode, "x")).is_none());
}
