//! The shared session: controls (dry runs, watch, a live camera's rules and phases) and the clock.

mod common;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use pdigger_factory_sdk::api::{FrameQuery, FrameSel, PollQuery, StreamQuery};
use pdigger_factory_sdk::models::{
    ClockEvent, Layer, LivePhase, SeekRequest, SelectRequest, SessionReply, SourceKind, StreamFormat,
};
use pdigger_factory_sdk::session::LIVE;
use pdigger_factory_sdk::sse::{EventStream, SseEvent};
use pdigger_factory_sdk::testing::{
    CONNECT_S, FakeFactory, Fault, KEEP_PICTURES, LIVE_CAMERA, LIVE_LAG_S, ManualClock, Reply,
};
use pdigger_factory_sdk::transport::{Method, Request, Transport};
use pdigger_factory_sdk::{Clock, ErrorCode, FramesOptions, PollOptions, SessionClock, StreamItem, SystemClock};

use common::{recording, recording_fake, replies_fake};

/// A GET of the fake: the reply's status, headers and body.
async fn get(fake: &FakeFactory, path: &str) -> (u16, Vec<(String, String)>, bytes::Bytes) {
    let request = Request {
        method: Method::Get,
        url: format!("{}{path}", FakeFactory::URL),
        headers: vec![],
        body: None,
        timeout: None,
    };
    let reply = fake.send(request).await.unwrap();
    let headers = ["content-type", "content-length", "cache-control", "x-interp-g"]
        .iter()
        .filter_map(|name| reply.headers.get(name).map(|v| (name.to_string(), v.to_string())))
        .collect();
    (reply.status, headers, reply.body.bytes().await.unwrap())
}

/// A GET of the fake that answers JSON: the status and the body.
async fn get_json(fake: &FakeFactory, path: &str) -> (u16, serde_json::Value) {
    let (status, _, body) = get(fake, path).await;
    (status, serde_json::from_slice(&body).unwrap_or_default())
}

/// The clock events of a stream, to its end.
async fn clock_events(events: EventStream) -> Vec<ClockEvent> {
    let events: Vec<SseEvent> = events.map(Result::unwrap).collect().await;
    events.iter().filter(|e| e.event == "clock").map(|e| serde_json::from_str(&e.data).unwrap()).collect()
}

/// A stream event's camera phase (a clock event's).
fn phase_of(event: &SseEvent) -> Option<LivePhase> {
    serde_json::from_str::<ClockEvent>(&event.data).ok()?.state.live_phase()
}

fn select_camera() -> SelectRequest {
    SelectRequest { video: LIVE_CAMERA.into(), time: None }
}

/// A fake showing its camera live (made from `joined-mid-move`: camera frames g 0 to 21, the
/// first 16 in the warm-up).
fn camera_fake() -> FakeFactory {
    FakeFactory::from_recording(recording("joined-mid-move")).unwrap().with_live()
}

#[tokio::test]
async fn controls_validate_with_dry_run_and_apply_without() {
    let fake = recording_fake("dry1-0296-0306-full");
    let factory = fake.client();
    let session = factory.session();
    assert!(!session.is_dry_run());
    let dry = session.dry_run();
    assert!(dry.is_dry_run());
    let before = session.state().await.unwrap();
    let would = dry.watch_at("dry1", Some(300.0)).await.unwrap();
    assert_eq!(would.session, before.session + 1);
    assert_eq!(session.state().await.unwrap(), fake.state());
    assert_eq!(fake.state().session, before.session, "a dry run moves nothing");
    assert!(!dry.pause().await.unwrap().playing && session.state().await.unwrap().playing);
    let watched = session.watch("dry1").await.unwrap();
    assert_eq!((watched.video.as_str(), watched.anchor_media), ("dry1", 139.0));
    assert_eq!(fake.state().session, before.session + 1);
    let paused = session.pause().await.unwrap();
    assert!(!paused.playing);
    let ops_before = fake.operations().len();
    let again = session.watch_at("dry1", Some(301.0)).await.unwrap();
    assert!(again.playing, "watch plays a paused session first");
    let ops = &fake.operations()[ops_before..];
    assert_eq!(ops, ["listVideos", "getSession", "sessionPlay", "sessionSelect"], "a time: is it a camera?");
    let seeked = session.seek(302.0).await.unwrap();
    assert_eq!(seeked.anchor_media, 302.0);
    assert!(session.play().await.unwrap().playing);
    let id = factory.api().get_world(None).await.unwrap().stones[0].id;
    assert_eq!(session.focus(id).await.unwrap().focus.unwrap().id, id);
    assert!(session.clear_focus().await.unwrap().focus.is_none());
    let info = session.info().await.unwrap();
    assert_eq!(info.state.session, fake.state().session);
    let unknown = session.watch("nope").await.unwrap_err();
    assert_eq!(unknown.code, ErrorCode::UnknownVideo);
}

#[tokio::test]
async fn a_select_on_the_recorded_replies_follows_their_session() {
    let fake = replies_fake();
    let recorded = fake.state().session;
    let request = SelectRequest { video: "dry1".into(), time: None };
    let selected = fake.client().api().session_select(&request, false).await.unwrap().state;
    assert_eq!((recorded, selected.session), (1795314885, 1795314886));
}

#[tokio::test]
async fn a_live_camera_is_listed_after_the_recording_shown_at_the_start_and_not_seekable() {
    let none = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap();
    assert_eq!(
        none.client().live().await.unwrap_err().code,
        ErrorCode::LiveUnavailable,
        "builds before claude-20261120-live2 list no camera"
    );
    assert!(none.client().cameras().await.unwrap().is_empty());
    let fake = none.with_live();
    let factory = fake.client();
    let live = factory.live().await.unwrap();
    assert_eq!(
        (live.id.as_str(), live.kind(), live.is_seekable(), live.duration, live.station.as_deref()),
        (LIVE_CAMERA, SourceKind::Camera, false, None, Some("FakeFactory"))
    );
    assert_eq!((live.fps, live.grid_step, live.hold, live.proxy.as_str()), (5.0, 1, Some(3.0), ""));
    assert_eq!(factory.cameras().await.unwrap(), std::slice::from_ref(&live));
    let list = factory.api().list_videos().await.unwrap();
    assert_eq!(list.default_video.as_deref(), Some("dry1"), "the default stays the recording");
    assert_eq!(list.videos.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["dry1", LIVE_CAMERA]);
    let state = factory.session().state().await.unwrap();
    assert!(state.is_live() && state.duration.is_none() && state.rate == 1.0);
    assert_eq!(state.live_phase(), Some(LivePhase::Live), "already live at the start");
    let ops = fake.operations().len();
    let refused = factory.session().seek(10.0).await.unwrap_err();
    assert_eq!(
        (refused.code.clone(), refused.status, refused.field.as_deref()),
        (ErrorCode::NotSeekable, None, Some("video"))
    );
    assert_eq!(&fake.operations()[ops..], ["getSession"], "no seek request was sent");
    let local = factory.session().watch_at(LIVE_CAMERA, Some(5.0)).await.unwrap_err();
    assert_eq!((local.code.clone(), local.field.as_deref()), (ErrorCode::NotSeekable, Some("time")));
    assert_eq!(local.message, "cam-live is a live camera: it has no time; leave `at` out");
    assert_eq!(&fake.operations()[ops + 1..], ["listVideos"], "the catalog says so; no select was sent");
    let legacy = factory.session().watch_at(LIVE, Some(5.0)).await.unwrap_err();
    assert_eq!(legacy.code, ErrorCode::NotSeekable);
    assert_eq!(fake.operations().len(), ops + 2, "the design's `live` id needs no request");
    let server = factory.api().session_seek(&SeekRequest { video: None, time: 1.0 }, true).await.unwrap_err();
    assert_eq!(
        (server.status, server.code.clone(), server.field.as_deref(), server.message.as_str()),
        (Some(409), ErrorCode::NotSeekable, Some("video"), "cam-live is a live camera and cannot seek")
    );
    let select = SelectRequest { video: LIVE_CAMERA.into(), time: Some(1.0) };
    let select = factory.api().session_select(&select, true).await.unwrap_err();
    assert_eq!(
        (select.status, select.field.as_deref(), select.message.as_str()),
        (Some(400), Some("time"), "cam-live is a live camera: leave time out")
    );
    let media = factory.api().get_video_media(LIVE_CAMERA, &Default::default()).await.unwrap_err();
    assert_eq!(
        (media.status, media.code.clone(), media.field.as_deref()),
        (Some(404), ErrorCode::NotFound, Some("id"))
    );
    assert!(media.message.ends_with("each frame's picture is /api/v2/session/frames/{g}/image.jpg"));
    let at = factory.session().watch_at("dry1", Some(300.0)).await.unwrap();
    assert_eq!((at.anchor_media, at.rate, at.live.is_none()), (300.0, 0.8, true), "a recording after a camera: 0.8");
    let picture = factory.session().picture(FrameSel::Latest).await.unwrap_err();
    assert_eq!(
        (picture.status, picture.code.clone(), picture.field.as_deref(), picture.message.as_str()),
        (Some(404), ErrorCode::NotFound, Some("g"), "dry1 is a recording: its picture is the proxy video")
    );
}

#[tokio::test]
async fn a_recording_after_a_start_on_a_camera_plays_at_0_8() {
    let mut reply: serde_json::Value = serde_json::from_str(&common::api_text("getSession.json")).unwrap();
    reply["state"]["video"] = LIVE.into();
    reply["state"]["rate"] = 1.0.into();
    let fake = replies_fake().with_reply("getSession", Reply::json(200, &reply));
    let request = SelectRequest { video: "dry1".into(), time: None };
    let selected = fake.client().api().session_select(&request, false).await.unwrap().state;
    assert_eq!((selected.video.as_str(), selected.rate), ("dry1", 0.8));
}

#[tokio::test]
async fn a_recording_after_the_offered_camera_plays_at_0_8() {
    let mut reply: serde_json::Value = serde_json::from_str(&common::api_text("getSession.json")).unwrap();
    reply["state"]["rate"] = 1.0.into();
    let fake = replies_fake().with_reply("getSession", Reply::json(200, &reply)).with_live();
    let request = SelectRequest { video: "dry1".into(), time: None };
    let selected = fake.client().api().session_select(&request, false).await.unwrap().state;
    assert_eq!((selected.video.as_str(), selected.rate), ("dry1", 0.8));
}

#[tokio::test]
async fn until_live_waits_out_the_connection_and_the_warm_up() {
    let fake = recording_fake("dry1-0296-0306-full").with_live();
    let factory = fake.client();
    let (clock, session) = (fake.clock(), factory.session());
    let recording = session.watch("dry1").await.unwrap();
    assert_eq!(session.until_live(Duration::ZERO).await.unwrap().session, recording.session, "a recording: at once");
    let t0 = clock.now();
    let connecting = session.watch(LIVE_CAMERA).await.unwrap();
    let phase = connecting.live.clone().unwrap();
    assert_eq!((phase.phase, phase.warmup, phase.since, phase.lag), (LivePhase::Connecting, 3.0, t0, 0.0));
    assert_eq!((connecting.hold_until, connecting.rate, connecting.duration), (t0 + 3600.0, 1.0, None));
    let none = session.picture(FrameSel::Latest).await.unwrap_err();
    assert_eq!(none.code, ErrorCode::NotRetained, "nothing is released before the camera is live");
    assert_eq!(none.message, "no frame is retained yet");
    let window = none.retained().unwrap();
    assert_eq!((window.session, window.g0, window.n), (connecting.session, None, 0));
    let live = session.until_live(Duration::from_secs(30)).await.unwrap();
    assert_eq!(live.live_phase(), Some(LivePhase::Live));
    assert_eq!(live.revision, connecting.revision + 2, "each phase bumps the revision");
    assert!(live.anchor_media >= CONNECT_S + 3.0, "the first frame released is after the warm-up");
    let went_live = t0 + live.anchor_media + LIVE_LAG_S;
    assert_eq!(
        (live.anchor_wall, live.hold_until, live.live.as_ref().unwrap().lag),
        (went_live, went_live, LIVE_LAG_S)
    );
    let waited = clock.now() - went_live;
    assert!((0.0..0.25 + 1e-6).contains(&waited), "read every 250 ms on the client's clock: {waited}");
    let picture = session.picture(FrameSel::Latest).await.unwrap();
    assert!(picture.jpeg.starts_with(&[0xff, 0xd8, 0xff, 0xfe]), "a JPEG with a comment segment first");
    assert!(String::from_utf8_lossy(&picture.jpeg).contains(&format!("FakeFactory {LIVE_CAMERA} g {}", picture.g)));
    let world = factory.api().get_world(None).await.unwrap();
    assert_eq!((world.video.as_str(), world.session, world.g), (LIVE_CAMERA, live.session, picture.g));
    assert_eq!(session.picture_in(picture.g, live.session).await.unwrap(), picture);
    let gone = session.picture_in(picture.g, live.session - 1).await.unwrap_err();
    assert_eq!((gone.status, gone.code.clone()), (Some(410), ErrorCode::SessionGone));
    let old = factory.api().get_frame_picture(picture.g + 1_000_000, None).await.unwrap_err();
    assert_eq!((old.status, old.code.clone()), (Some(404), ErrorCode::NotRetained));
    assert_eq!(factory.api().get_frame_picture(0, None).await.unwrap_err().code, ErrorCode::NotRetained, "warm-up");
    // a pause freezes the camera; play goes on at the live edge: now - since - lag
    session.pause().await.unwrap();
    clock.advance(10.0);
    let played = session.play().await.unwrap();
    let edge = clock.now() - phase.since - LIVE_LAG_S;
    assert!((played.anchor_media - edge).abs() < 1e-9 && played.anchor_wall == clock.now());
    let requests = fake.requests();
    let asked = requests.iter().rev().find(|r| r.url.contains("image.jpg")).unwrap();
    assert!(asked.headers.iter().any(|(k, v)| k == "Accept" && v == "image/jpeg"), "{:?}", asked.headers);
    assert_eq!(asked.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("accept")).count(), 1);
}

#[tokio::test]
async fn until_live_gives_up_on_a_camera_that_does_not_go_live() {
    let fake = recording_fake("dry1-0296-0306-full").with_live();
    let factory = fake.client();
    factory.session().pause().await.unwrap();
    let select = SelectRequest { video: LIVE_CAMERA.into(), time: None };
    factory.api().session_select(&select, false).await.unwrap(); // paused: its phases do not come
    let error = factory.session().until_live(Duration::from_secs(2)).await.unwrap_err();
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::LiveTimeout, "cam-live is still connecting after 2 s")
    );
    let half = factory.session().until_live(Duration::from_millis(500)).await.unwrap_err();
    assert_eq!(half.message, "cam-live is still connecting after 0.5 s");
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Connecting));
    let played = factory.session().play().await.unwrap();
    assert_eq!(played.live_phase(), Some(LivePhase::Connecting), "a play connects again");
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Connecting), "the phases are planned from the play");
    fake.clock().advance(CONNECT_S);
    let warming = fake.state();
    assert_eq!(warming.live_phase(), Some(LivePhase::Warming));
    assert_eq!(warming.live.unwrap().lag, 0.0, "a camera not yet live has no lag");
    let live = factory.session().until_live(Duration::from_secs(30)).await.unwrap();
    assert_eq!(live.live_phase(), Some(LivePhase::Live));
    let dry = factory.session().dry_run().watch(LIVE_CAMERA).await.unwrap();
    assert_eq!(dry.live_phase(), Some(LivePhase::Connecting));
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Live), "a dry run moves nothing");
    SystemClock.sleep(Duration::from_millis(1)).await;
}

#[tokio::test]
async fn a_stream_passes_the_camera_phases_and_then_its_frames() {
    let fake = recording_fake("dry1-0000-0130").with_live();
    let factory = fake.client();
    factory.session().watch("dry1").await.unwrap();
    let selected = factory.session().watch(LIVE_CAMERA).await.unwrap();
    let options = FramesOptions { layers: vec![Layer::World], resume: false, ..FramesOptions::default() };
    let items: Vec<StreamItem> = factory.streams().frames(options).take(4).map(Result::unwrap).collect().await;
    let phases: Vec<_> =
        items.iter().filter_map(|i| if let StreamItem::Clock(c) = i { c.state.live_phase() } else { None }).collect();
    assert_eq!(phases, [LivePhase::Connecting, LivePhase::Warming, LivePhase::Live]);
    let StreamItem::Frame(first) = &items[3] else { panic!("a frame after the phases: {items:?}") };
    assert!(first.frame.t >= CONNECT_S + 3.0, "the warm-up's frames are never released");
    assert_eq!((first.frame.video.as_str(), first.frame.session), (LIVE_CAMERA, selected.session));
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Live), "the stream waited for the camera");
    factory.session().watch(LIVE_CAMERA).await.unwrap();
    let polled: Vec<StreamItem> =
        factory.streams().poll(PollOptions::default()).take(4).map(Result::unwrap).collect().await;
    assert!(polled.iter().any(|i| matches!(i, StreamItem::Frame(_))), "a long poll waits for the phases too");
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Live));
}

#[tokio::test]
async fn the_session_clock_maps_media_and_wall_time() {
    let fake = replies_fake();
    let clock = fake.clock();
    let reply: SessionReply = serde_json::from_str(&common::api_text("getSession.json")).unwrap();
    clock.set(reply.server_now);
    let sampled = SessionClock::new(&reply, reply.server_now - 0.2, reply.server_now, clock.clone());
    assert!((sampled.offset - (reply.server_recv - reply.server_now + 0.1)).abs() < 1e-6);
    assert!((sampled.rtt - 0.2).abs() < 1e-6);
    let exact = SessionClock::new(&reply, reply.server_recv, reply.server_recv, clock.clone());
    assert_eq!(exact.media_time_at(reply.server_now), reply.state.media_time, "the server's own formula");
    assert!(!exact.is_live());
    let wall = exact.wall_of(622.8642175793648).unwrap();
    assert!((wall - reply.server_now).abs() < 1e-3);
    assert!((exact.local_wall_of(622.8642175793648).unwrap() - (wall - exact.offset)).abs() < 1e-9);
    assert_eq!(exact.media_time_at(reply.state.hold_until - 5.0), 586.0, "the hold counts as not played");
    assert_eq!(exact.media_time_at(reply.server_now + 1.0e6), reply.state.duration.unwrap(), "within the duration");
    let mut paused = exact.clone();
    let mut state = reply.state.clone();
    state.playing = false;
    state.anchor_media = 12.5;
    state.rate = 0.0;
    paused.update(state);
    assert_eq!(paused.media_time_now(), 12.5);
    assert_eq!(paused.wall_of(1.0), None);
    let mut held = reply.state.clone();
    held.playing = false;
    held.hold_until = held.anchor_wall + 5.0;
    paused.update(held.clone());
    assert_eq!(
        paused.wall_of(held.anchor_media + 0.8),
        Some(held.anchor_wall + 0.8 / held.rate),
        "paused: mapped back from the pause, not from the hold"
    );
    assert!(format!("{paused:?}").contains("SessionClock"));
    let mut live = reply.state.clone();
    live.duration = None;
    live.video = "live".into();
    let mut c = exact.clone();
    c.update(live);
    assert!(c.is_live() && c.media_time_at(reply.server_now + 100.0) > 600.0);
    let from_api = fake.client().session().clock().await.unwrap();
    assert_eq!(from_api.rtt, 0.0, "a manual clock does not move during the request");
    assert!((from_api.server_now() - clock.now() - from_api.offset).abs() < 1e-9);
}

#[test]
fn manual_clock() {
    let clock: Arc<ManualClock> = ManualClock::new(10.0);
    clock.advance(2.5);
    assert_eq!(clock.now(), 12.5);
    clock.set(1.0);
    assert_eq!(clock.now(), 1.0);
    assert!(format!("{clock:?}").contains("ManualClock"));
}

#[tokio::test(start_paused = true)]
async fn a_clock_sample_is_retried_as_a_whole() {
    let fake = recording_fake("dry1-0296-0306-full");
    let session = fake.client().session();
    fake.fail("getSession", Fault::status(503, "not_ready"));
    assert!(session.clock().await.is_ok(), "the retry is a new sample");
    fake.fail("getSession", Fault::status(400, "invalid_param"));
    assert_eq!(session.clock().await.unwrap_err().code, ErrorCode::InvalidParam);
    fake.fail("getSession", Fault::status(503, "not_ready").times(10));
    assert_eq!(session.clock().await.unwrap_err().code, ErrorCode::NotReady, "retries used up");
}

#[tokio::test]
async fn a_play_connects_a_live_camera_again_after_a_break() {
    let fake = camera_fake();
    let factory = fake.client();
    let (clock, session, api) = (fake.clock(), factory.session(), factory.api());
    let last = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    assert_eq!((last.g, last.i), (21, 21));
    let since = fake.state().live.unwrap().since;
    session.pause().await.unwrap();
    clock.advance(10.0);
    let played = session.play().await.unwrap();
    let live = played.live.clone().unwrap();
    assert_eq!((live.phase, live.lag, live.since), (LivePhase::Connecting, LIVE_LAG_S, since), "the lag kept");
    assert!((played.anchor_media - (clock.now() - since - LIVE_LAG_S)).abs() < 1e-9, "from the live edge");
    clock.advance(CONNECT_S);
    let warming = fake.state();
    assert_eq!((warming.live_phase(), warming.live.unwrap().lag), (Some(LivePhase::Warming), LIVE_LAG_S));
    clock.advance(30.0);
    let on_air = fake.state();
    assert_eq!((on_air.live_phase(), on_air.revision), (Some(LivePhase::Live), played.revision + 2));
    let g0 = last.g + 1 + 4; // a break of four grid frames, then a new warm-up
    for g in [last.g + 1, g0, g0 + 15] {
        let error = api.get_frame(g, &FrameQuery::default()).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::NotRetained, "g {g}");
    }
    let first = api.get_frame(g0 + 16, &FrameQuery::default()).await.unwrap();
    assert!(first.t > last.t + 10.0, "after the break: {} after {}", first.t, last.t);
    assert_eq!(session.picture(FrameSel::Latest).await.unwrap().g, g0 + 21);
}

#[tokio::test]
async fn a_play_right_after_a_pause_sends_its_run_after_the_frames_sent() {
    let fake = camera_fake();
    let (clock, api) = (fake.clock(), fake.client().api());
    let last = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    api.session_pause(false).await.unwrap();
    api.session_play(false).await.unwrap();
    clock.advance(30.0);
    let first = api.get_frame(last.g + 1 + 4 + 16, &FrameQuery::default()).await.unwrap();
    assert!(
        first.t >= last.t + CONNECT_S + 3.0,
        "after the frames already sent and a new warm-up: {} after {}",
        first.t,
        last.t
    );
}

#[tokio::test]
async fn a_camera_selected_while_paused_connects_on_the_play() {
    let fake = camera_fake();
    let factory = fake.client();
    let (clock, api) = (fake.clock(), factory.api());
    let selected = api.session_select(&select_camera(), false).await.unwrap().state;
    api.session_pause(false).await.unwrap();
    clock.advance(10.0);
    let options = FramesOptions { resume: false, limit: Some(1), ..FramesOptions::default() };
    let items: Vec<_> = factory.streams().frames(options).collect().await;
    assert!(!items.iter().any(|i| matches!(i, Ok(StreamItem::Frame(_)))), "a paused camera sends nothing");
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Connecting), "and takes no phase");
    let played = api.session_play(false).await.unwrap().state;
    assert_eq!((played.live_phase(), played.anchor_media), (Some(LivePhase::Connecting), 0.0));
    clock.advance(CONNECT_S);
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Warming));
    clock.advance(3.5);
    let live = fake.state();
    assert_eq!((live.live_phase(), live.revision), (Some(LivePhase::Live), played.revision + 2));
    assert!(live.anchor_wall > played.anchor_wall + 3.0, "the warm-up starts at the play");
    let first = api.get_frame(16, &FrameQuery::default()).await.unwrap();
    assert_eq!(first.session, selected.session, "numbered from 0: nothing was released before");
    assert!(api.get_frame(15, &FrameQuery::default()).await.is_err());
}

#[tokio::test]
async fn a_dry_run_play_of_a_paused_camera_moves_nothing() {
    let fake = camera_fake();
    let (clock, api) = (fake.clock(), fake.client().api());
    let paused = api.session_pause(false).await.unwrap().state;
    clock.advance(10.0);
    let would = api.session_play(true).await.unwrap();
    assert_eq!((would.dry_run, would.state.live_phase()), (Some(true), Some(LivePhase::Connecting)));
    let since = paused.live.as_ref().unwrap().since;
    assert!((would.state.anchor_media - (clock.now() - since - LIVE_LAG_S)).abs() < 1e-9);
    let state = fake.state();
    assert_eq!((state.live_phase(), state.playing, state.revision), (Some(LivePhase::Live), false, paused.revision));
    clock.advance(30.0);
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Live), "no phase was planned");
    let latest = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    assert_eq!(latest.g, 21, "no frame was added");
    api.session_play(false).await.unwrap();
    clock.advance(30.0);
    let after = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    assert_eq!(after.g, 21 + 1 + 4 + 21, "the real play's run is numbered as if no dry run came first");
}

#[tokio::test]
async fn a_dry_run_select_leaves_the_program_and_its_streams_alone() {
    let fake = camera_fake();
    let factory = fake.client();
    let api = factory.api();
    let options = FramesOptions { layers: vec![Layer::World], resume: false, ..FramesOptions::default() };
    let mut stream = factory.streams().frames(options);
    let mut frames = 0;
    while frames < 6 {
        if let StreamItem::Frame(_) = stream.next().await.unwrap().unwrap() {
            frames += 1;
        }
    }
    api.session_select(&SelectRequest { video: "dry1".into(), time: Some(50.0) }, true).await.unwrap();
    let latest = api.get_frame(FrameSel::Latest, &FrameQuery::default()).await.unwrap();
    assert_eq!(latest.g, 21, "the camera's frames are still served");
    let paused = api.session_pause(false).await.unwrap().state;
    let Some(Ok(StreamItem::Clock(next))) = stream.next().await else { panic!("a clock event") };
    assert_eq!(
        (next.state.revision, next.state.video.as_str()),
        (paused.revision, LIVE_CAMERA),
        "the pause's clock event, not the dry run's"
    );
}

#[tokio::test(start_paused = true)]
async fn an_open_stream_follows_a_new_program_on_the_fakes_clock() {
    let fake = camera_fake();
    let factory = fake.client();
    let clock = fake.clock();
    let options = FramesOptions {
        layers: vec![Layer::World],
        resume: false,
        format: StreamFormat::Ndjson,
        ..FramesOptions::default()
    };
    let mut stream = factory.streams().frames(options);
    let mut gs = Vec::new();
    while gs.len() < 6 {
        if let StreamItem::Frame(item) = stream.next().await.unwrap().unwrap() {
            gs.push(item.frame.g);
        }
    }
    assert_eq!(gs, (16..=21).collect::<Vec<_>>());
    let selected = fake.client().session().watch(LIVE_CAMERA).await.unwrap(); // another client
    let since = selected.live.as_ref().unwrap().since;
    let Some(Ok(StreamItem::Frame(next))) = stream.next().await else { panic!("the new program's first frame") };
    assert_eq!((next.frame.g, next.frame.session), (16, selected.session));
    let went_live = since + 3.6911 + LIVE_LAG_S;
    assert!((clock.now() - went_live).abs() < 1e-6, "the stream moved the clock on to the live phase");
    drop(stream);
    factory.session().watch(LIVE_CAMERA).await.unwrap();
    let (status, _) = get_json(&fake, "/api/v2/session/frames/stream?format=xml").await;
    assert_eq!(status, 400);
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Connecting), "a refused stream takes no phase");
}

#[tokio::test]
async fn a_long_poll_takes_the_phases_due_within_its_wait() {
    let fake = camera_fake();
    let (clock, api) = (fake.clock(), fake.client().api());
    api.session_select(&select_camera(), false).await.unwrap();
    let t0 = clock.now();
    let at_once = api.poll_frames(&PollQuery { wait_ms: Some(0), ..PollQuery::default() }).await.unwrap();
    assert_eq!((at_once.state.live_phase(), clock.now()), (Some(LivePhase::Connecting), t0));
    let warming = api.poll_frames(&PollQuery::default()).await.unwrap();
    assert_eq!(
        (warming.state.live_phase(), warming.frames.len(), clock.now()),
        (Some(LivePhase::Warming), 0, t0 + CONNECT_S),
        "the phases due within the wait, the clock moved on to each"
    );
    clock.advance(3.0);
    let live = api.poll_frames(&PollQuery::default()).await.unwrap();
    assert_eq!((live.state.live_phase(), live.frames.len()), (Some(LivePhase::Live), 6));
    assert_eq!(live.frames[0].g, 16);
    api.session_select(&select_camera(), false).await.unwrap();
    let (status, body) = get_json(&fake, "/api/v2/session/frames?wait=5000").await;
    assert_eq!((status, body["error"].as_str()), (400, Some("wait must be within 0..1000 ms")));
    assert_eq!(fake.state().live_phase(), Some(LivePhase::Connecting), "a refused poll takes no phase");
    let warming = api.poll_frames(&PollQuery { wait_ms: Some(600), ..PollQuery::default() }).await.unwrap();
    let seen = PollQuery { cursor: Some(warming.cursor), rev: Some(warming.state.revision), ..PollQuery::default() };
    let before = clock.now();
    let idle = api.poll_frames(&seen).await.unwrap();
    assert_eq!(
        (idle.frames.len(), idle.state.live_phase(), clock.now()),
        (0, Some(LivePhase::Warming), before + 1.0),
        "nothing new for this client: the poll waits out its wait"
    );
}

#[tokio::test(start_paused = true)]
async fn a_linked_clock_moves_only_when_its_owner_moves_it() {
    let fake = camera_fake().with_linked_clock();
    let factory = fake.client();
    let (clock, api, session) = (fake.clock(), factory.api(), factory.session());
    api.session_select(&select_camera(), false).await.unwrap();
    let t0 = clock.now();
    let polled = api.poll_frames(&PollQuery::default()).await.unwrap();
    assert_eq!((polled.state.live_phase(), clock.now()), (Some(LivePhase::Connecting), t0), "the clock stays");
    let seen = PollQuery { cursor: Some(polled.cursor), rev: Some(polled.state.revision), ..PollQuery::default() };
    api.poll_frames(&seen).await.unwrap();
    assert_eq!(clock.now(), t0, "an idle long poll leaves a linked clock alone");
    let mut events =
        api.stream_frames(&StreamQuery { layers: vec![Layer::World], ..StreamQuery::default() }).await.unwrap();
    let first = events.next().await.unwrap().unwrap();
    assert_eq!((first.event.as_str(), phase_of(&first)), ("clock", Some(LivePhase::Connecting)));
    let waiting = tokio::time::timeout(Duration::from_millis(100), events.next()).await;
    assert!(waiting.is_err() && clock.now() == t0, "the stream waits for the clock to reach the warm-up");
    clock.advance(1.0);
    assert_eq!(phase_of(&events.next().await.unwrap().unwrap()), Some(LivePhase::Warming));
    clock.advance(3.0);
    assert_eq!(phase_of(&events.next().await.unwrap().unwrap()), Some(LivePhase::Live));
    assert_eq!(events.next().await.unwrap().unwrap().event, "frame");
    drop(events);
    api.session_select(&select_camera(), false).await.unwrap();
    let (ops, t1) = (fake.operations().len(), clock.now());
    assert_eq!(session.until_live(Duration::from_secs(30)).await.unwrap().live_phase(), Some(LivePhase::Live));
    let reads = fake.operations()[ops..].iter().filter(|op| *op == "getSession").count();
    assert_eq!((reads, clock.now() - t1), (17, 4.0), "every 250 ms on the client's clock until live");
    api.session_select(&select_camera(), false).await.unwrap();
    api.session_pause(false).await.unwrap();
    let t2 = clock.now();
    let stuck = session.until_live(Duration::from_millis(600)).await.unwrap_err();
    assert_eq!(
        (stuck.code, stuck.message.as_str()),
        (ErrorCode::LiveTimeout, "cam-live is still connecting after 0.6 s")
    );
    assert!((clock.now() - t2 - 0.6).abs() < 1e-6);
    session.watch(LIVE_CAMERA).await.unwrap();
    clock.advance(1.0);
    session.pause().await.unwrap();
    let warm = session.until_live(Duration::from_secs(1)).await.unwrap_err();
    assert_eq!(warm.message, "cam-live is still warming after 1 s");
}

#[tokio::test]
async fn only_the_newest_pictures_are_kept() {
    let fake = FakeFactory::from_recording(recording("aug28-1270-1300")).unwrap().with_live();
    let factory = fake.client();
    let session = factory.session();
    assert_eq!(session.picture(FrameSel::Latest).await.unwrap().g, 150);
    let oldest = 150 - KEEP_PICTURES as i64 + 1;
    assert_eq!(session.picture(oldest).await.unwrap().g, oldest);
    let gone = session.picture(oldest - 1).await.unwrap_err();
    assert_eq!(
        (gone.status, gone.code.clone(), gone.field.as_deref(), gone.message.as_str()),
        (
            Some(404),
            ErrorCode::NotRetained,
            Some("g"),
            "the picture of frame 86 is no longer kept: only the newest 64 pictures are"
        )
    );
    assert!(gone.retained().is_none());
    assert!(factory.api().get_frame(oldest - 1, &FrameQuery::default()).await.is_ok(), "the frame is retained");
    assert_eq!(session.picture(14).await.unwrap_err().message, "frame 14 is not retained", "a warm-up frame");
    let (status, headers, jpeg) = get(&fake, "/api/v2/session/frames/latest/image.jpg").await;
    let length = jpeg.len().to_string();
    let header = |name: &str| headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    assert_eq!(
        (status, header("content-type"), header("content-length"), header("cache-control"), header("x-interp-g")),
        (200, Some("image/jpeg"), Some(length.as_str()), Some("no-store"), Some("150"))
    );
}

#[tokio::test]
async fn clock_events_carry_the_state_at_their_time() {
    let fake = recording_fake("dry1-0296-0306-full");
    let factory = fake.client();
    let (clock, api) = (fake.clock(), factory.api());
    factory.session().watch("dry1").await.unwrap();
    let query = StreamQuery { layers: vec![Layer::World], ..StreamQuery::default() };
    let selected = clock_events(api.stream_frames(&query).await.unwrap()).await;
    assert_eq!((selected[0].state.media_time, selected[0].server_now), (139.0, Some(clock.now())));
    clock.advance(5.0);
    let id = api.get_world(None).await.unwrap().stones[0].id;
    factory.session().focus(id).await.unwrap();
    let focused = clock_events(api.stream_frames(&query).await.unwrap()).await;
    let last = focused.last().unwrap();
    assert_eq!(last.server_now, Some(clock.now()), "sent when the focus changed");
    assert!((last.state.media_time - (139.0 + (5.0 - 2.2) * 0.75)).abs() < 1e-6, "{}", last.state.media_time);
    let camera = camera_fake();
    let (clock, api) = (camera.clock(), camera.client().api());
    api.session_select(&select_camera(), false).await.unwrap();
    api.session_pause(false).await.unwrap();
    clock.advance(10.0);
    let played = api.session_play(false).await.unwrap().state;
    clock.advance(30.0);
    let events = clock_events(api.stream_frames(&query).await.unwrap()).await;
    let [.., warming, live] = &events[..] else { panic!("the phases' clock events: {events:?}") };
    assert_eq!(
        (warming.state.live_phase(), warming.server_now),
        (Some(LivePhase::Warming), Some(played.anchor_wall + CONNECT_S)),
        "sent when the phase came"
    );
    assert_eq!((live.state.live_phase(), live.server_now), (Some(LivePhase::Live), Some(live.state.anchor_wall)));
    assert_eq!(live.state.media_time, live.state.anchor_media);
}

#[tokio::test(start_paused = true)] // a 503 is retried after its `retry_ms`
async fn a_camera_without_frames_has_no_world_yet() {
    let fake = camera_fake();
    let factory = fake.client_builder().retries(0).build().unwrap();
    factory.session().watch(LIVE_CAMERA).await.unwrap();
    let world = factory.api().get_world(None).await.unwrap_err();
    assert_eq!(
        (world.status, world.code.clone(), world.message.as_str(), world.retry_ms),
        (Some(503), ErrorCode::WorldUnavailable, "the Stable sim is not running", Some(1000))
    );
    let focus = factory.session().focus(1).await.unwrap_err();
    assert_eq!((focus.code, focus.retry_ms), (ErrorCode::WorldUnavailable, Some(1000)));
    let (status, events) = get_json(&fake, "/api/v2/session/world/events").await;
    assert_eq!((status, &events["events"], events.get("oldest_g")), (200, &serde_json::json!([]), None));
    let stone = factory.api().get_stone(1).await.unwrap_err();
    assert_eq!((stone.status, stone.code), (Some(404), ErrorCode::UnknownStone));
    let stable = factory.api().get_session().await.unwrap().stable;
    assert_eq!((stable.state.to_string(), stable.fits), ("running".to_string(), Some(4)), "the sim runs on");
    let empty = FakeFactory::new().with_live().client_builder().retries(0).build().unwrap();
    assert_eq!(empty.api().get_world(None).await.unwrap_err().code, ErrorCode::WorldUnavailable);
}
