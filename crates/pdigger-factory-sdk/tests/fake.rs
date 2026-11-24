//! The `testing` module itself: construction, routing, validation like the server, faults,
//! files, and the local HTTP server (which also exercises the default `reqwest` transport).

mod common;

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use pdigger_factory_sdk::api::{ByteRange, FrameQuery, FrameSel, MediaQuery, PollQuery, StreamQuery};
use pdigger_factory_sdk::models::{FocusRequest, Layer, LivePhase, SelectRequest};
use pdigger_factory_sdk::testing::{FAKE_EPOCH, FakeFactory, Fault, LIVE_CAMERA, LIVE_LAG_S, Reply};
use pdigger_factory_sdk::transport::{Method, Request, Transport};
use pdigger_factory_sdk::{Clock, ErrorCode, FramesOptions, PollOptions, ReplayStream, StreamItem};
use serde_json::json;

use common::{fixtures, recording, recording_fake};

fn request(method: Method, path: &str, body: Option<&str>, content_type: Option<&str>) -> Request {
    let headers = content_type.map(|t| vec![("Content-Type".to_string(), t.to_string())]).unwrap_or_default();
    Request {
        method,
        url: format!("{}{path}", FakeFactory::URL),
        headers,
        body: body.map(|b| Bytes::from(b.to_string())),
        timeout: None,
    }
}

async fn send(fake: &FakeFactory, method: Method, path: &str, body: Option<&str>) -> (u16, serde_json::Value) {
    let reply = fake.send(request(method, path, body, Some("application/json"))).await.unwrap();
    let status = reply.status;
    let bytes = reply.body.bytes().await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

/// An error reply as (status, code, field, message).
type Refusal<'a> = (u16, &'a str, &'a str, &'a str);

/// The error of a reply: (status, code, field, message).
fn refusal(reply: &(u16, serde_json::Value)) -> Refusal<'_> {
    let (status, body) = reply;
    let text = |key: &str| body[key].as_str().unwrap_or_default();
    (*status, text("code"), text("field"), text("error"))
}

/// The lines of an NDJSON body.
async fn ndjson(fake: &FakeFactory, path: &str) -> Vec<serde_json::Value> {
    let reply = fake.send(request(Method::Get, path, None, None)).await.unwrap();
    let bytes = reply.body.bytes().await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[tokio::test]
async fn routes_and_methods() {
    let fake = recording_fake("dry1-0296-0306-full");
    assert_eq!(send(&fake, Method::Get, "/nope", None).await.0, 404);
    let (status, body) = send(&fake, Method::Post, "/api/v2/videos", None).await;
    assert_eq!((status, body["code"].as_str()), (405, Some("method_not_allowed")));
    assert_eq!(send(&fake, Method::Delete, "/api/v2/session/frames/1/regions.png", None).await.0, 405);
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/frames/x", None).await;
    assert_eq!((status, body["field"].as_str()), (400, Some("g")));
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/y/regions.png", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/1/bogus.png", None).await.0, 404);
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/frames/1x/image.jpg?session=1", None).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("g must be an integer or latest")), "g first");
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/frames/latest/image.jpg?session=1", None).await;
    assert_eq!((status, body["code"].as_str()), (410, Some("session_gone")), "a stale session, whatever it showed");
    assert_eq!(send(&fake, Method::Post, "/api/v2/session/frames/1/image.jpg", None).await.0, 405);
    assert_eq!(send(&fake, Method::Get, "/api/v2/raw/tensors/x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/world/stones/x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/world/events?after=x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/world?precision=x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/render/overlay2d", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/render/overlay2d?g=1&t=2", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/render/overlay2d?t=-1", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/render/overlay2d?g=1&mode=x", None).await.0, 400);
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/render/scene3d?g=1&draw=primitives", None).await;
    assert_eq!((status, body["code"].as_str()), (404, Some("not_found")), "no recorded scene3d primitives");
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/stream?format=xml", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/stream?observe=2", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/stream?precision=x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/raw/stream?limit=0", None).await.0, 400);
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/frames?wait=5000", None).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("wait must be within 0..1000 ms")));
    let (status, body) = send(&fake, Method::Get, "/api/v2/session/frames?layers=a%2Cb&x+y=1&flag", None).await;
    assert_eq!((status, body["error"].as_str()), (400, Some("unknown layer 'a'")));
    let (_, body) = send(&fake, Method::Get, "/api/v2/session/frames?layers=world,,cards%zz", None).await;
    assert_eq!(body["code"], "unknown_layer");
    let ops = fake.operations();
    assert_eq!(ops[0], "-notFound");
    assert_eq!(ops[1], "-methodNotAllowed");
}

/// A control request and the error it gets: path, body, content type, status, code.
type Case = (&'static str, Option<&'static str>, Option<&'static str>, u16, &'static str);

#[tokio::test]
async fn bodies_are_validated_like_the_server() {
    let fake = recording_fake("dry1-0296-0306-full");
    let cases: [Case; 13] = [
        ("/api/v2/session/select", Some("{"), Some("application/json"), 400, "invalid_json"),
        ("/api/v2/session/select", Some("[1]"), Some("application/json"), 400, "not_an_object"),
        ("/api/v2/session/select", Some("{}"), Some("application/json"), 400, "missing_field"),
        ("/api/v2/session/select", Some("{\"video\":3}"), Some("application/json"), 400, "invalid_type"),
        (
            "/api/v2/session/select",
            Some("{\"video\":\"dry1\",\"time\":true}"),
            Some("application/json"),
            400,
            "invalid_type",
        ),
        (
            "/api/v2/session/select",
            Some("{\"video\":\"dry1\",\"time\":-1}"),
            Some("application/json"),
            400,
            "time_out_of_range",
        ),
        ("/api/v2/session/select", Some("{\"video\":\"dry1\"}"), Some("text/plain"), 415, "json_only"),
        ("/api/v2/session/select", None, Some("application/json"), 400, "invalid_json"),
        ("/api/v2/session/seek", Some("{\"video\":\"dry1\"}"), Some("application/json"), 400, "missing_field"),
        ("/api/v2/session/seek", Some("{\"time\":\"x\"}"), Some("application/json"), 400, "invalid_type"),
        ("/api/v2/session/focus", Some("{}"), Some("application/json"), 400, "missing_field"),
        ("/api/v2/session/focus", Some("{\"stone_id\":7.5}"), Some("application/json"), 400, "invalid_type"),
        ("/api/v2/session/pause", Some("[]"), Some("application/json"), 400, "not_an_object"),
    ];
    for (path, body, content_type, status, code) in cases {
        let method = if path.ends_with("focus") { Method::Put } else { Method::Post };
        let reply = fake.send(request(method, &format!("{path}?dry_run=1"), body, content_type)).await.unwrap();
        assert_eq!(reply.status, status, "{path} {body:?}");
        let error = pdigger_factory_sdk::Error::from_reply(reply.status, &reply.body.bytes().await.unwrap(), None);
        assert_eq!(error.code.to_string(), code, "{path} {body:?}");
    }
    let empty = fake.send(request(Method::Post, "/api/v2/session/play", None, None)).await.unwrap();
    assert_eq!(empty.status, 200, "play takes an empty body");
    let world = FakeFactory::new()
        .client()
        .api()
        .put_focus(&FocusRequest { stone_id: 1, g: None, session: None }, true)
        .await
        .unwrap_err();
    assert_eq!(world.code, ErrorCode::WorldUnavailable);
}

#[tokio::test(start_paused = true)] // a 503 is retried after its `retry_ms`
async fn construction_without_clocks_or_frames() {
    let empty = FakeFactory::default();
    assert!(format!("{empty:?}").contains("FakeFactory"));
    let state = empty.state();
    assert_eq!((state.video.as_str(), state.session, state.anchor_wall), ("fake", 1, FAKE_EPOCH));
    assert_eq!(empty.clock().now(), FAKE_EPOCH);
    let recorded = FakeFactory::from_recording(recording("joined-mid-move")).unwrap();
    assert_eq!(recorded.clock().now(), 1795315127.8454037, "the clock starts at the first clock line's server_now");
    let api = empty.client().api();
    assert_eq!(api.get_world(None).await.unwrap_err().code, ErrorCode::WorldUnavailable, "the empty session's");
    assert_eq!(api.get_session().await.unwrap().stable.state.to_string(), "running");
    let header_only =
        FakeFactory::from_values(vec![json!({"recording": "pfr1", "source": {"id": "dry9", "duration": 10.0}})]);
    assert_eq!(header_only.state().video, "dry9");
    assert_eq!(header_only.client().api().get_video("dry9").await.unwrap().duration, Some(10.0));
    let frames_only = FakeFactory::from_values(vec![
        json!({"cursor": 1, "session": 4, "video": "v", "g": 1, "t": 0.2, "world": {"stones": []}}),
        json!({"sse": "worker", "data": {"state": "idle"}}),
        json!({"other": 1}),
    ]);
    let s = frames_only.state();
    assert_eq!((s.video.as_str(), s.session, s.anchor_media, s.rate), ("v", 4, 0.2, 0.8));
    let health = frames_only.client().api().get_health().await.unwrap();
    assert_eq!(health.worker.to_string(), "running", "the health's worker is always running");
    let stone = frames_only.client().api().get_stone(3).await.unwrap_err();
    assert_eq!(stone.code, ErrorCode::UnknownStone);
    let no_world = FakeFactory::from_values(vec![json!({"cursor": 1, "session": 4, "g": 1, "t": 0.2})]);
    assert_eq!(no_world.client().api().get_stone(1).await.unwrap_err().code, ErrorCode::UnknownStone, "as the unit");
    assert_eq!(no_world.client().api().get_world(None).await.unwrap_err().code, ErrorCode::WorldUnavailable);
    assert_eq!(no_world.client().api().raw_packet(1, None).await.unwrap_err().code, ErrorCode::NotRetained);
    // a raw packet carries the numbers its frame has (this one has no video and no i)
    let bare =
        FakeFactory::from_values(vec![json!({"cursor": 1, "session": 4, "g": 1, "t": 0.2, "raw": {"schema": "sip1"}})]);
    let (status, packet) = send(&bare, Method::Get, "/api/v2/raw/packets/1", None).await;
    assert_eq!((status, packet["g"].as_i64(), packet.get("i"), packet.get("video")), (200, Some(1), None, None));
    let plain = std::fs::read(recording("joined-mid-move")).unwrap();
    let mut text = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(plain.as_slice()), &mut text).unwrap();
    let from_plain = FakeFactory::from_recording_bytes(&text).unwrap();
    assert_eq!(from_plain.state().video, "dry1");
    assert!(FakeFactory::from_recording("/nope.ndjson").is_err());
    assert!(FakeFactory::from_recording_bytes(b"{nope").is_err());
    assert!(FakeFactory::from_recording_bytes(&[0x1f, 0x8b, 0, 0]).is_err());
    assert!(FakeFactory::from_recording_bytes(&[0xff, 0xfe, b'\n']).is_err());
    assert!(FakeFactory::new().with_replies("/nope").is_err());
    let index_only = std::env::temp_dir().join(format!("pdigger-fake-{}", std::process::id()));
    std::fs::create_dir_all(&index_only).unwrap();
    std::fs::write(index_only.join("INDEX.json"), r#"{"getModel":{"file":"missing.json"}}"#).unwrap();
    assert!(FakeFactory::new().with_replies(&index_only).is_err());
    std::fs::write(
        index_only.join("INDEX.json"),
        r#"{"getModel":{"file":"m.json","headers":{"Content-Length":"2","X-A":"b"}}}"#,
    )
    .unwrap();
    std::fs::write(index_only.join("m.json"), "{}").unwrap();
    let with_model = FakeFactory::new().with_replies(&index_only).unwrap();
    let reply = with_model.send(request(Method::Get, "/api/v2/model", None, None)).await.unwrap();
    assert_eq!((reply.status, reply.headers.get("x-a"), reply.headers.get("content-length")), (200, Some("b"), None));
    std::fs::remove_dir_all(&index_only).unwrap();
}

#[tokio::test]
async fn reply_and_fault_helpers() {
    let reply = Reply::error(409, "stale_session", "old", Some("session")).header("X-Y", "z");
    assert_eq!(reply.header_value("x-y"), Some("z"));
    assert_eq!(reply.header_value("nope"), None);
    let broken = Fault::broken().retry_ms(5).retry_after("1").times(2);
    let fake = FakeFactory::new();
    fake.fail("getHealth", broken);
    let factory = fake.client_builder().retries(0).build().unwrap();
    assert_eq!(factory.api().get_health().await.unwrap_err().code, ErrorCode::Transport);
    assert_eq!(factory.api().get_health().await.unwrap_err().code, ErrorCode::Transport);
    assert!(factory.api().get_health().await.is_ok());
    let custom = Fault::reply(Reply::new(500, "text/plain", "x")).retry_ms(5);
    fake.fail("getReady", custom);
    assert_eq!(factory.api().get_ready().await.unwrap_err().status, Some(500));
}

#[tokio::test(start_paused = true)]
async fn files_serve_recordings_for_url_replays() {
    let gz = std::fs::read(recording("dry1-0296-0306-full")).unwrap();
    let cut = gz[..gz.len() - 10].to_vec();
    let fake = FakeFactory::new()
        .with_file("/data/dry1.ndjson.gz", gz.clone(), 4096)
        .with_file("/data/cut.ndjson.gz", cut, 4096)
        .with_file("/data/tiny.ndjson", "x", 1)
        .with_file("/data/empty.ndjson", "", 1)
        .with_file("/data/odd.ndjson", vec![b'{', 0xff, b'\n'], 1);
    let transport: Arc<dyn Transport> = Arc::new(fake.clone());
    let url = format!("{}/data/dry1.ndjson.gz", FakeFactory::URL);
    let replay = ReplayStream::open_url_with(transport.clone(), &url).await.unwrap();
    assert_eq!(replay.header().unwrap().name.as_deref(), Some("dry1-0296-0306-full"));
    assert!(format!("{replay:?}").contains("ReplayStream"));
    let activities: Vec<_> = replay.activities().map(Result::unwrap).collect().await;
    assert_eq!(activities.len(), 6);
    // lines a reader does not understand are skipped (spec 6), the first one too
    let tiny = ReplayStream::open_url_with(transport.clone(), &format!("{}/data/tiny.ndjson", FakeFactory::URL)).await;
    let tiny = tiny.unwrap();
    assert!(tiny.header().is_none());
    assert_eq!(tiny.frames().count().await, 0, "a line that is not JSON");
    let empty_url = format!("{}/data/empty.ndjson", FakeFactory::URL);
    let empty = ReplayStream::open_url_with(transport.clone(), &empty_url).await.unwrap();
    let updates: Vec<_> = empty.updates().collect().await;
    assert!(updates.len() == 1 && updates[0].as_ref().unwrap().is_final, "an empty file: just the final update");
    let odd = ReplayStream::open_url_with(transport.clone(), &format!("{}/data/odd.ndjson", FakeFactory::URL)).await;
    let updates: Vec<_> = odd.unwrap().updates().collect().await;
    assert!(updates.len() == 1 && updates[0].as_ref().unwrap().is_final, "a line that is not UTF-8");
    // a download that cannot be read to its end gives its error last, and no final update
    let cut_url = format!("{}/data/cut.ndjson.gz", FakeFactory::URL);
    let updates: Vec<_> =
        ReplayStream::open_url_with(transport.clone(), &cut_url).await.unwrap().updates().collect().await;
    assert_eq!(updates.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Decode, "a cut gzip file");
    assert!(
        updates.iter().filter(|u| u.is_err()).count() == 1
            && !updates.iter().any(|u| u.as_ref().is_ok_and(|u| u.is_final))
    );
    fake.drop_after("getFile", 3);
    let updates: Vec<_> = ReplayStream::open_url_with(transport.clone(), &url).await.unwrap().updates().collect().await;
    assert_eq!(updates.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Transport, "a broken download");
    assert!(!updates.iter().any(|u| u.as_ref().is_ok_and(|u| u.is_final)));
    let missing =
        ReplayStream::open_url_with(transport.clone(), &format!("{}/data/none.ndjson", FakeFactory::URL)).await;
    assert_eq!(missing.unwrap_err().status, Some(404));
    fake.drop_after("getFile", 3);
    let broken = ReplayStream::open_url_with(transport.clone(), &url).await.unwrap();
    let frames: Vec<_> = broken.frames().collect().await;
    assert_eq!(frames.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Transport);
    fake.drop_after("getFile", 0);
    assert!(ReplayStream::open_url_with(transport.clone(), &url).await.is_err());
    let frame = r#"{"cursor":1,"session":1,"video":"v","g":1,"i":2,"t":0.2,"past_valid":true,"dt_past":0.6}"#;
    let plain_lines = format!("\n{frame}\n{{\"recording\":\"pfr1\",\"name\":\"late\"}}\n").into_bytes();
    let late = FakeFactory::new().with_file("/late.ndjson", plain_lines, 7);
    let replay =
        ReplayStream::open_url_with(Arc::new(late), &format!("{}/late.ndjson", FakeFactory::URL)).await.unwrap();
    assert!(replay.header().is_none());
    let lines: Vec<_> = replay.lines().map(Result::unwrap).collect().await;
    // the header can only be the first line that is not blank: a `recording` line further down is skipped (spec 6)
    assert!(matches!(&lines[..], [pdigger_factory_sdk::replay::Line::Frame(_)]));
    let paced = FakeFactory::new().with_file(
        "/paced.ndjson",
        format!("{{\"recording\":\"pfr1\"}}\n{frame}\n").into_bytes(),
        37, // three pieces of the 110 bytes
    );
    paced.set_delay(|op, i| (op == "getFile").then_some(Duration::from_millis(100 * (i as u64 + 1))));
    let start = tokio::time::Instant::now();
    let replay =
        ReplayStream::open_url_with(Arc::new(paced), &format!("{}/paced.ndjson", FakeFactory::URL)).await.unwrap();
    assert_eq!(replay.summary().await.unwrap().frames, 1);
    assert_eq!(start.elapsed(), Duration::from_millis(100 + 200 + 300), "the delay hook paces file chunks too");
}

#[tokio::test]
async fn a_dropped_stream_ends_with_its_error() {
    let fake = FakeFactory::new().with_file("/data/abc.ndjson", "abc", 1);
    fake.drop_after("getFile", 1);
    let mut body = fake.send(request(Method::Get, "/data/abc.ndjson", None, None)).await.unwrap().body;
    assert_eq!(body.next().await.unwrap().unwrap(), Bytes::from("a"));
    assert_eq!(body.next().await.unwrap().unwrap_err().code, ErrorCode::Transport);
    assert!(body.next().await.is_none(), "nothing after the error");
    let stream = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap();
    stream.drop_after("streamFrames", 1);
    let mut body = stream.send(request(Method::Get, "/api/v2/session/frames/stream", None, None)).await.unwrap().body;
    assert!(body.next().await.unwrap().is_ok());
    assert!(body.next().await.unwrap().is_err());
    assert_eq!(stream.client().api().get_status().await.unwrap().streams, Some(0), "a dropped stream is closed");
    assert!(body.next().await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_local_server_speaks_http() {
    let fake = recording_fake("dry1-0296-0306-full").with_file(
        "/dry1.ndjson.gz",
        std::fs::read(recording("dry1-0296-0306-full")).unwrap(),
        65536,
    );
    let server = fake.serve().await.unwrap();
    assert!(server.url().starts_with("http://127.0.0.1:"));
    assert!(format!("{server:?}").contains("FakeServer"));
    let factory = server.client();
    let api = factory.api();
    assert_eq!(api.get_world(None).await.unwrap().g, 1530);
    let list = api.list_videos_if_changed(None).await.unwrap();
    assert!(api.list_videos_if_changed(list.etag.as_deref()).await.unwrap().value.is_none());
    let media = api.head_video_media("dry1").await.unwrap();
    assert_eq!(media.status, 200);
    let paused = factory.session().pause().await.unwrap();
    assert!(!paused.playing);
    let id = api.get_world(None).await.unwrap().stones[0].id;
    assert_eq!(
        api.put_focus(&FocusRequest { stone_id: id, g: None, session: None }, false)
            .await
            .unwrap()
            .state
            .focus
            .unwrap()
            .id,
        id
    );
    assert!(api.delete_focus(false).await.unwrap().state.focus.is_none());
    let items: Vec<_> = factory
        .streams()
        .frames(FramesOptions { layers: vec![Layer::World], resume: false, ..FramesOptions::default() })
        .collect()
        .await;
    assert!(items.iter().filter(|i| matches!(i, Ok(StreamItem::Frame(_)))).count() > 40);
    fake.drop_after("streamFrames", 2);
    let items: Vec<_> =
        factory.streams().frames(FramesOptions { resume: false, ..FramesOptions::default() }).collect().await;
    assert_eq!(items.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Transport);
    fake.fail("getModel", Fault::broken());
    let refused = server.client().api().get_model().await;
    assert!(refused.is_ok(), "a refused connection is retried");
    let fast = pdigger_factory_sdk::FactoryClient::builder(server.url()).retries(0).build().unwrap();
    fake.fail("getModel", Fault::broken());
    assert_eq!(fast.api().get_model().await.unwrap_err().code, ErrorCode::Transport);
    let replay = ReplayStream::open_url(&format!("{}/dry1.ndjson.gz", server.url())).await.unwrap();
    assert_eq!(replay.summary().await.unwrap().frames, 51);
    let slow = pdigger_factory_sdk::FactoryClient::builder(server.url())
        .request_timeout(Duration::from_millis(20))
        .retries(0)
        .build()
        .unwrap();
    assert!(slow.api().get_health().await.is_ok());
    let media = api.get_video_media("dry1", &Default::default()).await.unwrap();
    assert_eq!((media.status, media.content_length), (200, None), "a chunked body has no Content-Length");
    assert!(!media.data.is_empty());
    let closed = pdigger_factory_sdk::FactoryClient::builder("http://127.0.0.1:9").retries(0).build().unwrap();
    assert_eq!(closed.api().get_health().await.unwrap_err().code, ErrorCode::Transport);
    let _ = fixtures();
    server.shutdown().await;
}

#[tokio::test]
async fn layers_media_ranges_tracks_and_sessions_without_every_layer() {
    let fake = recording_fake("dry2-0582-0600").with_live().with_live();
    let factory = fake.client();
    let list = factory.api().list_videos().await.unwrap();
    let cameras = list.videos.iter().filter(|v| v.id == LIVE_CAMERA).count();
    assert_eq!(cameras, 1, "one camera entry however often it is offered");
    let watched = factory.session().watch("dry2").await.unwrap();
    assert_eq!(watched.video, "dry2");
    let late = factory.api().get_frame(2938, &pdigger_factory_sdk::api::FrameQuery::default()).await.unwrap();
    assert_eq!(late.missing, vec![Layer::World], "a select replays frames that lack a layer");
    let cards = factory
        .api()
        .get_frame(2938, &pdigger_factory_sdk::api::FrameQuery { layers: vec![Layer::Cards], ..Default::default() })
        .await
        .unwrap();
    assert!(cards.missing.is_empty() && cards.cards.is_some(), "missing is cut to the layers asked for");
    let open = recording_fake("dry1-0296-0306-full");
    let mut ranged = request(Method::Get, "/api/v2/videos/dry1/media", None, None);
    ranged.headers.push(("Range".into(), "bytes=-".into()));
    let reply = open.send(ranged).await.unwrap();
    assert_eq!(reply.status, 416, "a range with neither end");
    let stones = FakeFactory::from_values(vec![
        json!({"recording": "pfr1", "video": "v"}),
        json!({"cursor": 1, "session": 2, "video": "v", "g": 1, "t": 0.1, "world": {"stones": [{"id": 4}]}}),
    ]);
    let stone = stones.client().api().get_stone(4).await.unwrap();
    assert!(stone.path2d.is_empty() && stone.path3d.is_empty(), "a stone without an outline or a solid has no path");
    let bare = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap();
    assert_eq!(
        bare.client().api().get_api_root().await.unwrap().build,
        "claude-20261122-hr4x1",
        "the build of the recording header"
    );
    assert_eq!(
        bare.client().api().get_video_media("dry1", &Default::default()).await.unwrap_err().status,
        Some(404),
        "no recorded media"
    );
    let media = bare.clone().with_reply("getVideoMedia", Reply::new(200, "video/mp4", &b"0123456789"[..]));
    let range = MediaQuery { range: Some(ByteRange::Span(2, 4)), ..MediaQuery::default() };
    assert_eq!(&media.client().api().get_video_media("dry1", &range).await.unwrap().data[..], b"234");
    let mut accept = request(Method::Get, "/api/v2/session/frames/stream?limit=1", None, None);
    accept.headers.push(("Accept".into(), "application/x-ndjson".into()));
    let reply = bare.send(accept).await.unwrap();
    assert_eq!(reply.headers.get("content-type"), Some("application/x-ndjson"), "the Accept header picks NDJSON");
    assert!(
        pdigger_factory_sdk::Replay::open("/no/such/recording.ndjson")
            .unwrap_err()
            .message
            .starts_with("cannot open /no/such")
    );
}

#[tokio::test]
async fn controls_are_checked_in_the_units_order() {
    let fake = recording_fake("joined-mid-move").with_live(); // showing the camera
    let (seek, select) = ("/api/v2/session/seek?dry_run=1", "/api/v2/session/select?dry_run=1");
    let camera = (409, "not_seekable", "video", "cam-live is a live camera and cannot seek");
    for body in [r#"{"time":3}"#, "{}", r#"{"time":true}"#, r#"{"video":"cam-live","time":1}"#] {
        assert_eq!(refusal(&send(&fake, Method::Post, seek, Some(body)).await), camera, "the camera first: {body}");
    }
    let range = "time must be within 0..1807.158 s";
    let seeks: [(&str, Refusal); 10] = [
        (r#"{"video":"nope","time":1}"#, (400, "unknown_video", "video", "Unknown video 'nope'")),
        (r#"{"video":null,"time":1}"#, (400, "invalid_type", "video", "Unknown video None")),
        (r#"{"video":5}"#, (400, "invalid_type", "video", "Unknown video 5")),
        (r#"{"video":true}"#, (400, "invalid_type", "video", "Unknown video True")),
        (r#"{"video":false}"#, (400, "invalid_type", "video", "Unknown video False")),
        (r#"{"video":"dry1"}"#, (400, "missing_field", "time", range)),
        (r#"{"video":"dry1","time":true}"#, (400, "invalid_type", "time", range)),
        (r#"{"video":"dry1","time":null}"#, (400, "invalid_type", "time", range)),
        (r#"{"video":"dry1","time":-1}"#, (400, "time_out_of_range", "time", range)),
        (r#"{"video":"dry1","time":99999}"#, (400, "time_out_of_range", "time", range)),
    ];
    for (body, refused) in seeks {
        assert_eq!(refusal(&send(&fake, Method::Post, seek, Some(body)).await), refused, "seek {body}");
    }
    let (status, sought) = send(&fake, Method::Post, seek, Some(r#"{"video":"dry1","time":3}"#)).await;
    let state = &sought["state"];
    assert_eq!(
        (status, state["video"].as_str(), state["anchor_media"].as_f64(), sought["dry_run"].as_bool()),
        (200, Some("dry1"), Some(3.0), Some(true)),
        "a seek naming a recording is made while a camera is shown"
    );
    assert!(state["live"].is_null() && state["rate"] == 0.8);
    let leave_out = (400, "not_seekable", "time", "cam-live is a live camera: leave time out");
    let selects: [(&str, Refusal); 7] = [
        ("{}", (400, "missing_field", "video", "Unknown video None")),
        (r#"{"video":null}"#, (400, "invalid_type", "video", "Unknown video None")),
        (r#"{"video":"cam-live","time":3}"#, leave_out),
        (r#"{"video":"cam-live","time":null}"#, leave_out),
        (r#"{"video":"dry1","time":null}"#, (400, "invalid_type", "time", range)),
        (r#"{"video":"dry1","time":-1}"#, (400, "time_out_of_range", "time", range)),
        (r#"{"video":"dry1","time":99999}"#, (400, "time_out_of_range", "time", range)),
    ];
    for (body, refused) in selects {
        assert_eq!(refusal(&send(&fake, Method::Post, select, Some(body)).await), refused, "select {body}");
    }
    let dry_run = (400, "invalid_param", "dry_run", "dry_run must be 0 or 1");
    let two = send(&fake, Method::Post, "/api/v2/session/select?dry_run=2", Some(r#"{"video":"dry1"}"#)).await;
    assert_eq!(refusal(&two), dry_run);
    assert_eq!(refusal(&send(&fake, Method::Post, "/api/v2/session/play?dry_run=true", None).await), dry_run);
    let (_, body) = send(&fake, Method::Post, "/api/v2/session/select?dry_run=2", Some("{")).await;
    assert_eq!(body["code"], "invalid_json", "the body before dry_run");
    assert_eq!(fake.state().video, LIVE_CAMERA, "nothing was applied");
    let nothing = send(&FakeFactory::new(), Method::Post, seek, Some(r#"{"time":1}"#)).await;
    assert_eq!(refusal(&nothing), (409, "not_seekable", "video", "the session shows no video"));
    let open_ended = FakeFactory::from_values(vec![
        json!({"recording": "pfr1", "source": {"id": "v"}}),
        json!({"cursor": 1, "g": 1, "t": 0.2}),
    ]);
    let before = send(&open_ended, Method::Post, seek, Some(r#"{"time":-1}"#)).await;
    assert_eq!(refusal(&before), (400, "time_out_of_range", "time", "time must be at least 0 s"));
    let (_, selected) = send(&open_ended, Method::Post, "/api/v2/session/select", Some(r#"{"video":"v"}"#)).await;
    let state = &selected["state"];
    let held = state["hold_until"].as_f64().unwrap() - state["anchor_wall"].as_f64().unwrap();
    assert!((held - 1.4).abs() < 1e-6, "a source without a hold of its own is held for 1.4 s: {held}");
    assert_eq!(state["session"], 2, "the next session id");
    let kind_only = FakeFactory::from_values(vec![
        json!({"recording": "pfr1", "source": {"id": "k", "kind": "camera"}}),
        json!({"cursor": 1, "session": 2, "video": "k", "g": 1, "t": 0.2}),
    ]);
    let (status, selected) = send(&kind_only, Method::Post, "/api/v2/session/select", Some(r#"{"video":"k"}"#)).await;
    assert_eq!(
        (status, selected["state"]["rate"].as_f64(), selected["state"]["live"].is_null()),
        (200, Some(0.8), true),
        "only `live: true` makes a camera"
    );
    assert_eq!(send(&kind_only, Method::Post, "/api/v2/session/seek", Some(r#"{"time":0.1}"#)).await.0, 200);
}

#[tokio::test]
async fn a_cameras_frames_are_numbered_from_its_select() {
    let source = json!({"id": "cam9", "kind": "camera", "live": true, "seekable": false, "duration": null});
    let frame = |session: i64, g: i64, t: f64| json!({"cursor": g + 1, "session": session, "video": "cam9", "g": g, "i": 2 * g + 100, "t": t});
    let fake = FakeFactory::from_values(vec![
        json!({"recording": "pfr1", "source": source}),
        frame(5, 7, 0.0),
        frame(5, 8, 3.2),
        frame(6, 0, 0.4),
    ]);
    assert_eq!(fake.state().rate, 0.8);
    let (status, selected) = send(&fake, Method::Post, "/api/v2/session/select", Some(r#"{"video":"cam9"}"#)).await;
    let state = &selected["state"];
    assert_eq!(
        (status, state["session"].as_i64(), state["live"]["phase"].as_str()),
        (200, Some(7), Some("connecting"))
    );
    let frames = ndjson(&fake, "/api/v2/session/frames/stream?limit=2&format=ndjson").await;
    let numbers: Vec<_> = frames
        .iter()
        .map(|f| (f["g"].as_i64().unwrap(), f["i"].as_i64().unwrap(), f["t"].as_f64().unwrap(), f["session"].clone()))
        .collect();
    assert_eq!(
        numbers,
        [(1, 1, 3.7, json!(7)), (2, 2, 3.9, json!(7))],
        "the recording's sessions end to end, from g 0 and t 0.5; i is g; the warm-up's first 3 s never released"
    );
    assert!(frames.iter().all(|f| f["video"] == "cam9"));
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Live));
    assert!((fake.clock().now() - (FAKE_EPOCH + 3.7 + LIVE_LAG_S)).abs() < 1e-6, "the stream waited for the camera");
}

#[tokio::test]
async fn camera_frames_and_their_packets_are_numbered_alike() {
    let fake = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap().with_live();
    let (status, frame) = send(&fake, Method::Get, "/api/v2/session/frames/latest?layers=raw,overlay", None).await;
    assert_eq!(
        (status, frame["g"].as_i64(), frame["i"].as_i64(), frame["video"].as_str()),
        (200, Some(50), Some(50), Some(LIVE_CAMERA))
    );
    for key in ["cursor", "session", "video", "g", "i", "t"] {
        assert_eq!(frame["raw"][key], frame[key], "raw.{key}");
    }
    assert_eq!(frame["overlay"]["prev_g"], 49, "the overlay's previous frame moves with g");
    let (_, packet) = send(&fake, Method::Get, "/api/v2/raw/packets/latest", None).await;
    for key in ["cursor", "session", "video", "g", "i", "t"] {
        assert_eq!(packet[key], frame[key], "the packet's {key}");
    }
    let streamed = ndjson(&fake, "/api/v2/raw/stream?limit=1&format=ndjson").await;
    let (_, sixteen) = send(&fake, Method::Get, "/api/v2/session/frames/16?layers=raw", None).await;
    assert_eq!(streamed[0]["g"], 16, "the first frame after the warm-up");
    for key in ["cursor", "session", "video", "g", "i", "t"] {
        assert_eq!(streamed[0][key], sixteen[key], "the streamed packet's {key}");
    }
}

#[tokio::test]
async fn frame_numbers_and_sessions_are_read_as_the_unit_reads_them() {
    let fake = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap().with_live();
    let g = (400, "invalid_param", "g", "g must be an integer or latest");
    for path in ["frames/-1", "frames/+5", "frames/1.5", "frames/12345678901", "frames/12345678901/image.jpg"] {
        assert_eq!(refusal(&send(&fake, Method::Get, &format!("/api/v2/session/{path}"), None).await), g, "{path}");
    }
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/0000000016", None).await.0, 200, "10 digits");
    let session = send(&fake, Method::Get, "/api/v2/session/frames/latest/image.jpg?session=x", None).await;
    assert_eq!(refusal(&session), (400, "invalid_param", "session", "session must be an integer"));
    let gone = send(&fake, Method::Get, "/api/v2/session/frames/x?session=1", None).await;
    assert_eq!(refusal(&gone), (410, "session_gone", "session", "session 1 has ended"), "a frame's session before g");
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/latest?session=-1", None).await.0, 410);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/latest?precision=x", None).await.0, 400);
    assert_eq!(send(&fake, Method::Get, "/api/v2/session/frames/latest?session=", None).await.0, 200, "no session");
    let empty = FakeFactory::new().with_live();
    let (status, body) = send(&empty, Method::Get, "/api/v2/session/frames/latest/image.jpg", None).await;
    assert_eq!(
        (status, body["code"].as_str(), body["error"].as_str(), body["retained"]["n"].as_i64()),
        (404, Some("not_retained"), Some("no frame is retained yet"), Some(0))
    );
    let api = empty.client().api();
    let latest = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap_err();
    assert_eq!((latest.code, latest.message.as_str()), (ErrorCode::NotRetained, "no frame is retained yet"));
    assert_eq!(api.get_frame(5, &FrameQuery::default()).await.unwrap_err().message, "frame 5 is not retained");
}

#[tokio::test]
async fn the_camera_is_listed_as_the_units_cameras_are() {
    let (_, list) = send(&FakeFactory::new().with_live(), Method::Get, "/api/v2/videos", None).await;
    let camera = json!({
        "id": "cam-live", "title": "Live camera", "subtitle": "The recording, replayed as a live camera",
        "duration": null, "fps": 5.0, "frames": null, "grid_step": 1, "default_start": null, "hold": 3.0,
        "proxy": "", "proxy_wh": [960, 540], "native_wh": [1920, 1080], "held_out": [], "kind": "camera",
        "live": true, "seekable": false, "station": "FakeFactory",
    });
    assert_eq!((&list["default_video"], &list["videos"]), (&json!("cam-live"), &json!([camera])));
    let (_, sized) = send(
        &FakeFactory::from_recording(recording("aug28-1270-1300")).unwrap().with_live(),
        Method::Get,
        "/api/v2/videos",
        None,
    )
    .await;
    let (recorded, live) = (&sized["videos"][0], &sized["videos"][1]);
    assert_eq!(
        (live["fps"].as_f64(), &live["proxy_wh"], &live["native_wh"]),
        (Some(5.0), &recorded["proxy_wh"], &recorded["native_wh"]),
        "sized like the recording"
    );
    let given = Reply::json(
        200,
        &json!({"default_video": "c1", "videos": [{"id": "c1", "kind": "camera"}, {"id": "c2", "live": true}]}),
    );
    let fake = FakeFactory::new().with_reply("listVideos", given.clone());
    let reply = fake.send(request(Method::Get, "/api/v2/videos", None, None)).await.unwrap();
    assert_eq!(reply.body.bytes().await.unwrap(), given.body, "a catalog given is sent as it is");
    let cameras: Vec<String> = fake.client().cameras().await.unwrap().into_iter().map(|c| c.id).collect();
    assert_eq!(cameras, ["c1", "c2"]);
    let api = fixtures().join("api");
    for fake in [
        FakeFactory::new().with_live().with_replies(&api).unwrap(),
        FakeFactory::new().with_replies(&api).unwrap().with_live(),
    ] {
        let list = fake.client().api().list_videos().await.unwrap();
        let ids: Vec<String> = list.videos.into_iter().map(|v| v.id).collect();
        let cameras = ["CAM00000001", "CAM00000002", "CAM00000003", "CAM00000004", "CAM00000005", "CAM00000006"];
        let listed_ids = [["aug28", "jul30", "dry1", "dry2"].as_slice(), &cameras, &[LIVE_CAMERA]].concat();
        assert_eq!(ids, listed_ids, "in either order: the recordings and the plant cameras, then the fake's camera");
        let (_, listed) = send(&fake, Method::Get, "/api/v2/videos", None).await;
        let (first, camera) = (&listed["videos"][0], &listed["videos"][10]);
        assert_eq!(
            (&camera["proxy_wh"], &camera["native_wh"]),
            (&first["proxy_wh"], &first["native_wh"]),
            "sized like the first entry in either order"
        );
        assert_eq!(fake.state().video, LIVE_CAMERA);
    }
}

#[tokio::test]
async fn a_recording_without_a_source_is_listed_from_its_frames() {
    let fake =
        FakeFactory::from_values(vec![json!({"cursor": 1, "session": 2, "video": "v1", "g": 0, "t": 0.0})]).with_live();
    let (_, list) = send(&fake, Method::Get, "/api/v2/videos", None).await;
    assert_eq!(list["default_video"], "v1");
    assert_eq!(
        list["videos"][0],
        json!({
            "id": "v1", "title": "v1", "duration": null, "fps": 10.0, "frames": null, "grid_step": 2,
            "default_start": 0.0, "hold": 2.2, "proxy": "/api/v2/videos/v1/media", "proxy_wh": [960, 540],
            "native_wh": [1920, 1080],
        })
    );
    let live = &list["videos"][1];
    assert_eq!(
        (live["id"].as_str(), live["fps"].as_f64(), &live["proxy_wh"]),
        (Some(LIVE_CAMERA), Some(5.0), &json!([960, 540]))
    );
    let watched = fake.client().session().watch("v1").await.unwrap();
    assert_eq!((watched.video.as_str(), watched.rate, watched.live.is_none()), ("v1", 0.8, true));
    assert!((watched.hold_until - watched.anchor_wall - 2.2).abs() < 1e-6, "the entry's hold");
}

#[tokio::test(start_paused = true)]
async fn the_status_and_the_worker_are_the_units() {
    let no_world = FakeFactory::from_values(vec![json!({"cursor": 1, "session": 4, "g": 1, "t": 0.2})]);
    let (_, session) = send(&no_world, Method::Get, "/api/v2/session", None).await;
    assert_eq!(session["worker"], json!({"state": "running", "reason": "", "session": 4, "video": "fake"}));
    assert_eq!(session["stable"], json!({"state": "running", "fits": 4, "evidence": true, "fixer": "thin-v1"}));
    let fake = FakeFactory::from_recording(recording("joined-mid-move")).unwrap().with_live();
    let status = fake.client().api().get_status().await.unwrap();
    let retained = status.retained.clone().unwrap();
    assert_eq!(
        (retained.g0, retained.g1, retained.n),
        (Some(16), Some(21), 6),
        "the camera's frames after the warm-up"
    );
    assert_eq!((status.streams, status.long_polls, status.viewers), (Some(0), Some(0), 0));
    assert_eq!(
        (status.build.as_str(), status.worker.state.to_string()),
        ("claude-20261122-hr4x1", "running".to_string())
    );
}

#[tokio::test]
async fn focus_and_sessions_change_as_on_the_unit() {
    let fake = recording_fake("dry1-0296-0306-full");
    let api = fake.client().api();
    let id = api.get_world(None).await.unwrap().stones[0].id;
    let focused = api.put_focus(&FocusRequest { stone_id: id, g: None, session: None }, false).await.unwrap();
    let cleared = api.delete_focus(false).await.unwrap().state.revision;
    let again = api.delete_focus(false).await.unwrap().state.revision;
    assert_eq!((cleared, again), (focused.state.revision + 1, cleared), "clearing no focus changes nothing");
    let changed = FakeFactory::from_recording(recording("session-change-mid-move")).unwrap();
    assert_eq!(changed.state().session, 1795314878, "the first clock line's session");
    let api = changed.client().api();
    let select = SelectRequest { video: "dry1".into(), time: None };
    let next = api.session_select(&select, true).await.unwrap().state.session;
    assert_eq!(next, 1795314879, "after the newest session of the recording's frames");
    assert_eq!(api.session_select(&select, false).await.unwrap().state.session, next, "a dry run took no id");
    assert_eq!(api.session_select(&select, false).await.unwrap().state.session, next + 1);
}

#[tokio::test(start_paused = true)]
async fn the_state_is_the_recordings_clock_line_that_playback_has_passed() {
    // dry1-0000-0130's clock lines: revision 398 (rate 0.8) at the start, 399 (0.74, the auto-rate)
    // after g 22 (cursor 75) and 400 (0.75) after g 117 (cursor 170)
    let fake = recording_fake("dry1-0000-0130");
    let api = fake.client().api();
    assert_eq!(fake.clock().now(), 1795315094.809069, "the clock starts at the first clock line's server_now");
    let start = api.get_session().await.unwrap();
    assert_eq!((start.state.revision, start.state.rate, start.server_now), (398, 0.8, fake.clock().now()));
    // a long poll serves the frames up to the next clock line, which the next reply carries
    let poll = |cursor| PollQuery { cursor: Some(cursor), limit: Some(16), ..PollQuery::default() };
    let first = api.poll_frames(&poll(0)).await.unwrap();
    let gs: Vec<i64> = first.frames.iter().map(|f| f.g).collect();
    assert_eq!((first.state.revision, first.cursor, gs), (398, 75, vec![0, 20, 21, 22]));
    let next = api.poll_frames(&poll(first.cursor)).await.unwrap();
    assert_eq!((next.state.revision, next.state.rate, next.frames[0].g, next.frames.len()), (399, 0.74, 23, 16));
    assert_eq!(api.get_session().await.unwrap().state.revision, 399, "the session answers the state in force");
    // so a polling client takes each line before the frames after it, as a replay does
    let polled: Vec<_> = recording_fake("dry1-0000-0130")
        .client()
        .streams()
        .poll(PollOptions { limit: Some(16), ..PollOptions::default() })
        .take(120)
        .collect()
        .await;
    assert!(matches!(&polled[0], Ok(StreamItem::Clock(c)) if c.state.revision == 398));
    let clocks: Vec<(i64, i64)> = polled
        .windows(2)
        .filter_map(|w| match (&w[0], &w[1]) {
            (Ok(StreamItem::Clock(c)), Ok(StreamItem::Frame(f))) => Some((c.state.revision, f.frame.g)),
            _ => None,
        })
        .collect();
    assert_eq!(clocks, [(399, 23), (400, 118)]);
    // a stream passes each line as it sends it; one after a cursor starts there
    let options = FramesOptions { resume: false, ..FramesOptions::default() };
    let _: Vec<_> = fake.client().streams().frames(options.clone()).collect().await;
    assert_eq!(fake.state().revision, 400, "the stream passed the last line");
    let late = recording_fake("dry1-0000-0130");
    drop(late.client().api().stream_frames(&StreamQuery { after: Some(170), ..StreamQuery::default() }).await.unwrap());
    assert_eq!(late.state().revision, 400, "a stream after cursor 170 starts after the last line");
    // a control takes over: its revision comes after the recording's, whose later lines no longer apply
    let paused = recording_fake("dry1-0000-0130");
    let state = paused.client().api().session_pause(false).await.unwrap().state;
    assert_eq!((state.revision, state.playing), (401, false));
    let _: Vec<_> = paused.client().streams().frames(options).collect().await;
    let state = paused.state();
    assert_eq!((state.revision, state.playing), (401, false), "the recording's later lines no longer apply");
}
