//! The mid level: frames over SSE and NDJSON with resume, side events, revisions, duplicates,
//! backfill of late layers, the long-poll loop and raw packets.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use pdigger_factory_sdk::models::{Layer, StreamFormat};
use pdigger_factory_sdk::streams::PollOptions;
use pdigger_factory_sdk::testing::{FakeFactory, Fault, Reply};
use pdigger_factory_sdk::transport::{Request, Response, Transport};
use pdigger_factory_sdk::{ErrorCode, FactoryClient, FrameItem, FramesOptions, StreamItem};
use serde_json::json;

use common::{complete, recording, recording_fake};

fn frames_of(items: &[pdigger_factory_sdk::Result<StreamItem>]) -> Vec<&FrameItem> {
    items.iter().filter_map(|i| if let Ok(StreamItem::Frame(f)) = i { Some(&**f) } else { None }).collect()
}

fn world_cards() -> FramesOptions {
    FramesOptions { layers: vec![Layer::World, Layer::Cards], ..FramesOptions::default() }
}

fn frame_count(name: &str) -> usize {
    pdigger_factory_sdk::Replay::open(recording(name)).unwrap().frames().count()
}

#[tokio::test(start_paused = true)]
async fn one_connection_serves_the_recording_in_order() {
    let fake = recording_fake("dry1-0296-0306-full");
    let items: Vec<_> =
        fake.client().streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    let frames = frames_of(&items);
    assert_eq!(frames.len(), frame_count("dry1-0296-0306-full"));
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    assert!(frames.iter().all(|f| f.frame.world.is_some() && f.frame.raw.is_none() && f.event_id.is_some()));
    assert!(items.iter().any(|i| matches!(i, Ok(StreamItem::Clock(_)))));
    assert!(items.iter().any(|i| matches!(i, Ok(StreamItem::Worker(_)))));
    assert_eq!(fake.streams_opened(), 1);
}

#[tokio::test(start_paused = true)]
async fn ended_connections_resume_with_the_last_event_id() {
    let fake = recording_fake("dry1-0296-0306-full");
    let options = FramesOptions {
        limit: Some(10),
        observe: Some(true),
        precision: Some(pdigger_factory_sdk::models::Precision::Full),
        ..world_cards()
    };
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(options)
        .filter(|i| futures::future::ready(matches!(i, Ok(StreamItem::Frame(_)))))
        .take(35)
        .collect()
        .await;
    let frames = frames_of(&items);
    assert_eq!(frames.len(), 35);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g), "no duplicates across reconnects");
    assert!(fake.streams_opened() >= 4);
    let requests = fake.requests();
    assert!(requests.iter().all(|r| r.url.contains("observe=1") && r.url.contains("precision=full")));
    assert!(requests.iter().skip(1).all(|r| r.header("last-event-id").is_some()));
}

#[tokio::test(start_paused = true)]
async fn ndjson_resumes_after_the_newest_cursor() {
    let fake = recording_fake("dry1-0296-0306-full");
    let options = FramesOptions { format: StreamFormat::Ndjson, limit: Some(20), after: Some(1), ..world_cards() };
    let items: Vec<_> = fake.client().streams().frames(options).take(45).collect().await;
    let frames = frames_of(&items);
    assert_eq!(frames.len(), 45);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    assert!(frames.iter().all(|f| f.event_id.is_none()));
    assert!(fake.requests().iter().skip(1).all(|r| r.url.contains("after=") && r.url.contains("format=ndjson")));
}

#[tokio::test(start_paused = true)]
async fn broken_connections_reconnect_after_the_backoff() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.drop_after("streamFrames", 5);
    fake.fail("streamFrames", Fault::status(503, "too_many_streams"));
    let items: Vec<_> = fake.client().streams().frames(world_cards()).take(30).collect().await;
    assert!(items.iter().all(Result::is_ok), "a resumed stream hides the breaks");
    let frames = frames_of(&items);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    assert!(fake.streams_opened() >= 2);
}

#[tokio::test(start_paused = true)]
async fn without_resume_errors_end_the_stream() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.drop_after("streamFrames", 3);
    let items: Vec<_> =
        fake.client().streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    assert_eq!(items.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Transport);
    let bad = FramesOptions { layers: vec![Layer::Other("foo".into())], ..FramesOptions::default() };
    let items: Vec<_> = fake.client().streams().frames(bad).collect().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_ref().unwrap_err().code, ErrorCode::UnknownLayer);
}

#[tokio::test(start_paused = true)]
async fn side_events_pass_through_and_reset_continues() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.inject("streamFrames", "gap", json!({"gap": 3}), 2);
    fake.inject("streamFrames", "reset", json!({"boot": "other", "cursor": 9}), 4);
    fake.inject("streamFrames", "ping", json!({}), 5);
    fake.inject("streamFrames", "worker", json!("not a worker"), 6);
    let items: Vec<_> =
        fake.client().streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    assert!(items.iter().any(|i| matches!(i, Ok(StreamItem::Gap(3)))));
    assert!(items.iter().any(|i| matches!(i, Ok(StreamItem::Reset(r)) if r.boot == "other")));
    assert!(items.iter().any(|i| matches!(i, Err(e) if e.code == ErrorCode::Decode)), "a bad event is an error item");
    assert_eq!(frames_of(&items).len(), frame_count("dry1-0296-0306-full"));
    // after a reset, a reconnect goes on in the new boot after the reset's cursor
    let fake = recording_fake("dry1-0296-0306-full");
    fake.inject("streamFrames", "reset", json!({"boot": "other", "cursor": 9}), 1);
    fake.drop_after("streamFrames", 2);
    let _: Vec<_> = fake.client().streams().frames(world_cards()).filter(only_frames).take(3).collect().await;
    let resumed = fake.requests().into_iter().filter(|r| r.url.contains("/frames/stream")).nth(1).unwrap();
    assert!(resumed.headers.contains(&("Last-Event-ID".to_string(), "other-9".to_string())), "{:?}", resumed.headers);
}

#[tokio::test(start_paused = true)]
async fn an_id_of_another_boot_gets_a_reset() {
    let fake = recording_fake("dry1-0296-0306-full").with_boot("bootb");
    let api = fake.client().api();
    let query = pdigger_factory_sdk::api::StreamQuery {
        last_event_id: Some("boota-12".into()),
        limit: Some(1),
        ..Default::default()
    };
    let events: Vec<_> = api.stream_frames(&query).await.unwrap().collect().await;
    assert_eq!(events[0].as_ref().unwrap().event, "reset");
    let same = pdigger_factory_sdk::api::StreamQuery {
        last_event_id: Some("bootb-1553".into()),
        limit: Some(1),
        ..Default::default()
    };
    let events: Vec<_> = api.stream_frames(&same).await.unwrap().collect().await;
    assert_eq!(events.len(), 1);
    assert!(events[0].as_ref().unwrap().data.contains("\"g\":1501"));
}

fn sse(frames: &[serde_json::Value]) -> String {
    frames.iter().enumerate().map(|(i, f)| format!("id: b-{}\nevent: frame\ndata: {f}\n\n", i + 1)).collect()
}

fn frame(g: i64, world: Option<i64>, missing: bool) -> serde_json::Value {
    let mut f = json!({"cursor": g, "session": 7, "video": "v", "g": g, "t": g as f64 * 0.2});
    if let Some(rev) = world {
        f["world"] = json!({"g": g, "rev": rev});
    }
    if missing {
        f["missing"] = json!(["world"]);
    }
    complete(f)
}

#[tokio::test(start_paused = true)]
async fn late_worlds_are_backfilled_in_order() {
    // a clock event with every field spec 2.3 rule 9 checks
    let clock = json!({"session": 7, "revision": 1, "video": "v", "playing": true, "rate": 1.0, "anchor_media": 0.0,
        "anchor_wall": 0.0, "hold_until": 0.0, "changed_by": "viewer", "media_time": 0.0, "server_now": 0.0});
    let text = sse(&[frame(5, None, true), frame(6, Some(0), false)]) + &format!("event: clock\ndata: {clock}\n\n");
    let fake = FakeFactory::new()
        .with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", text))
        .with_reply("getFrame-5", Reply::json(200, &frame(5, Some(0), false)));
    let start = tokio::time::Instant::now();
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    assert!(matches!(items[0], Ok(StreamItem::Clock(_))), "side events are never held");
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| f.frame.g).collect::<Vec<_>>(), [5, 6]);
    assert!(frames[0].is_backfill && frames[0].frame.world.is_some() && frames[0].frame.missing.is_empty());
    assert!(!frames[1].is_backfill);
    assert_eq!(start.elapsed(), Duration::from_millis(1500));
    let backfill = fake.requests().into_iter().find(|r| r.url.contains("/frames/5")).unwrap();
    assert!(backfill.url.contains("layers=world") && backfill.url.contains("session=7"));
}

#[tokio::test(start_paused = true)]
async fn frames_that_cannot_be_completed_are_released_as_they_are() {
    let text = sse(&[frame(5, None, true), frame(6, None, true), frame(7, Some(0), false)]);
    let fake = FakeFactory::new()
        .with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", text))
        .with_reply("getFrame-5", Reply::json(200, &frame(5, None, true)))
        .with_reply("getFrame-6", Reply::error(404, "not_retained", "frame 6 is not retained", Some("g")));
    let start = tokio::time::Instant::now();
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| f.frame.g).collect::<Vec<_>>(), [5, 6, 7]);
    assert!(frames.iter().all(|f| !f.is_backfill));
    assert_eq!(frames[0].frame.missing, vec![Layer::World]);
    assert_eq!(start.elapsed(), Duration::from_millis(3500), "three tries, 1 s apart, then the 404 at once");
    assert_eq!(fake.requests().iter().filter(|r| r.url.contains("/frames/5")).count(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_reset_releases_held_frames() {
    let text = sse(&[frame(5, None, true), frame(6, Some(0), false)])
        + "event: reset\ndata: {\"boot\":\"n\",\"cursor\":1}\n\n";
    let fake = FakeFactory::new().with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", text));
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    assert!(matches!(&items[0], Ok(StreamItem::Frame(f)) if f.frame.g == 5 && !f.is_backfill));
    assert!(matches!(&items[2], Ok(StreamItem::Reset(_))));
}

#[tokio::test(start_paused = true)]
async fn a_reset_that_does_not_decode_releases_no_held_frame() {
    let text =
        sse(&[frame(5, None, true), frame(6, Some(0), false)]) + "event: reset\ndata: {\"boot\":5,\"cursor\":1}\n\n";
    let fake = FakeFactory::new().with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", text));
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    // the reset is skipped as a decode error (rule 9) while frame 5 still waits for its world, which never comes
    assert!(matches!(&items[0], Err(e) if e.code == ErrorCode::Decode), "{items:?}");
    assert_eq!(frames_of(&items).iter().map(|f| f.frame.g).collect::<Vec<_>>(), [5, 6]);
}

#[tokio::test(start_paused = true)]
async fn revisions_pass_and_duplicates_drop() {
    let text = sse(&[
        frame(5, Some(0), false),
        frame(5, Some(2), false),
        frame(5, Some(1), false),
        frame(6, None, false),
        frame(6, None, false),
    ]) + "event: frame\ndata: {not json\n\n";
    let fake = FakeFactory::new().with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", text));
    let items: Vec<_> =
        fake.client().streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    let frames = frames_of(&items);
    assert_eq!(
        frames.iter().map(|f| (f.frame.g, f.is_revision)).collect::<Vec<_>>(),
        [(5, false), (5, true), (6, false)]
    );
    assert_eq!(items.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Decode);
}

#[tokio::test(start_paused = true)]
async fn empty_connections_back_off() {
    let fake = FakeFactory::new().with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", ""));
    let start = tokio::time::Instant::now();
    let mut stream = fake.client().streams().frames(FramesOptions::default());
    let waited = tokio::time::timeout(Duration::from_secs(3), stream.next()).await;
    assert!(waited.is_err(), "nothing to deliver");
    assert!(start.elapsed() >= Duration::from_secs(3));
    let opened = fake.operations().iter().filter(|o| *o == "streamFrames").count();
    assert!((3..12).contains(&opened), "backed off: {opened}");
}

#[tokio::test(start_paused = true)]
async fn raw_packets_carry_their_cursor() {
    let fake = recording_fake("dry1-0296-0306-full");
    let options = FramesOptions { resume: false, ..FramesOptions::default() };
    let items: Vec<_> = fake.client().streams().raw_packets(options).collect().await;
    let packets: Vec<_> =
        items.iter().filter_map(|i| if let Ok(StreamItem::Packet(p)) = i { Some(p) } else { None }).collect();
    assert_eq!(packets.len(), frame_count("dry1-0296-0306-full"));
    assert!(packets.windows(2).all(|w| w[0].cursor < w[1].cursor));
    let nd = FramesOptions { format: StreamFormat::Ndjson, limit: Some(5), ..FramesOptions::default() };
    let more: Vec<_> = fake.client().streams().raw_packets(nd).take(12).collect().await;
    let packets: Vec<_> =
        more.iter().filter_map(|i| if let Ok(StreamItem::Packet(p)) = i { p.cursor } else { None }).collect();
    assert!(packets.windows(2).all(|w| w[0] < w[1]), "resumed after the newest cursor: {packets:?}");
    let bad = fake.client().streams().raw_packets(FramesOptions { resume: false, ..FramesOptions::default() });
    fake.inject("rawStream", "gap", json!({"gap": "x"}), 0);
    let first = bad.take(1).collect::<Vec<_>>().await;
    assert_eq!(first[0].as_ref().unwrap_err().code, ErrorCode::Decode);
    let none = recording_fake("dry2-0582-0600");
    let items: Vec<_> = none
        .client()
        .streams()
        .raw_packets(FramesOptions { resume: false, ..FramesOptions::default() })
        .collect()
        .await;
    assert!(items.iter().all(|i| !matches!(i, Ok(StreamItem::Packet(_)))), "a world recording has no raw layer");
    let gap_nd = recording_fake("dry1-0296-0306-full");
    gap_nd.inject("rawStream", "gap", json!({"gap": 2}), 1);
    gap_nd.inject("rawStream", "clock", json!({}), 1);
    let items: Vec<_> = gap_nd
        .client()
        .streams()
        .raw_packets(FramesOptions { format: StreamFormat::Ndjson, resume: false, ..FramesOptions::default() })
        .collect()
        .await;
    assert!(items.iter().any(|i| matches!(i, Ok(StreamItem::Gap(2)))));
    assert!(!items.iter().any(|i| matches!(i, Ok(StreamItem::Clock(_)))), "NDJSON has no clock events");
}

#[tokio::test(start_paused = true)]
async fn the_long_poll_continues_from_each_reply() {
    let fake = recording_fake("dry1-0296-0306-full");
    let options = PollOptions {
        layers: vec![Layer::World],
        limit: Some(4),
        wait_ms: Some(100),
        cid: Some("tab-0123456".into()),
        ..PollOptions::default()
    };
    fake.fail("pollFrames", Fault::status(503, "busy").times(1));
    let items: Vec<_> = fake.client().streams().poll(options).take(30).collect().await;
    assert!(matches!(items[0], Ok(StreamItem::Clock(_))));
    assert!(matches!(items[1], Ok(StreamItem::Worker(_))));
    let frames = frames_of(&items);
    assert_eq!(frames.len(), 28);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    let calls: Vec<_> = fake.requests().into_iter().filter(|r| r.url.contains("/session/frames?")).collect();
    assert!(calls[2].url.contains("cursor=") && !calls[2].url.contains("cursor=0&"));
    let all: Vec<_> = fake
        .client()
        .streams()
        .poll(PollOptions { limit: Some(16), ..PollOptions::default() })
        .take(53)
        .collect()
        .await;
    assert_eq!(frames_of(&all).len(), frame_count("dry1-0296-0306-full"));
    let calls = fake.operations().iter().filter(|o| *o == "pollFrames").count();
    let more = tokio::time::timeout(
        Duration::from_secs(2),
        fake.client()
            .streams()
            .poll(PollOptions { limit: Some(16), ..PollOptions::default() })
            .take(54)
            .collect::<Vec<_>>(),
    )
    .await;
    assert!(more.is_err(), "only 53 items exist");
    let waited = fake.operations().iter().filter(|o| *o == "pollFrames").count() - calls;
    assert!((8..=16).contains(&waited), "an unchanged reply waits before the next call: {waited} calls in 2 s");
    let bad: Vec<_> =
        fake.client().streams().poll(PollOptions { limit: Some(99), ..PollOptions::default() }).collect().await;
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].as_ref().unwrap_err().message, "limit must be within 1..16");
}

/// The recorded long-poll reply (g 3078 with its world, then g 3079 without it) as a reply whose
/// first frame is late: g 3078 without its world, and g 3079 complete (its world as it was read
/// back, `getFrame-latest`), so that it waits behind g 3078. With g 3078 as recorded, to read back.
fn late_first_world() -> (serde_json::Value, serde_json::Value) {
    let mut reply: serde_json::Value = serde_json::from_str(&common::api_text("pollFrames-world.json")).unwrap();
    let latest: serde_json::Value = serde_json::from_str(&common::api_text("getFrame-latest.json")).unwrap();
    let first = reply["frames"][0].clone();
    reply["frames"][0]["missing"] = json!(["world"]);
    reply["frames"][0].as_object_mut().unwrap().remove("world");
    reply["frames"][1]["world"] = latest["world"].clone();
    reply["frames"][1].as_object_mut().unwrap().remove("missing");
    (reply, first)
}

#[tokio::test(start_paused = true)]
async fn the_long_poll_backfills_too() {
    let (reply, _) = late_first_world();
    let fake = FakeFactory::new()
        .with_reply("pollFrames-world", Reply::json(200, &reply))
        .with_reply("getFrame-3078", Reply::error(410, "session_gone", "session 1 has ended", Some("session")));
    let items: Vec<_> =
        fake.client().streams().poll(PollOptions { backfill: true, ..PollOptions::default() }).take(4).collect().await;
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| f.frame.g).collect::<Vec<_>>(), [3078, 3079]);
    assert!(frames[0].frame.world.is_none() && frames[1].frame.world.is_some());
    let bad = FakeFactory::new().with_reply(
        "pollFrames-world",
        Reply::error(400, "invalid_param", "limit must be within 1..16", Some("limit")),
    );
    let items: Vec<_> = bad.client().streams().poll(PollOptions::default()).collect().await;
    assert!(items[0].is_err());
}

#[tokio::test(start_paused = true)]
async fn a_finished_long_poll_still_backfills_its_held_frames() {
    // the recorded reply's second frame, g 3079, came without its world: the poll ends (the 400 after g 3078) while it
    // is held, and it is read back as getFrame-latest
    let reply: serde_json::Value = serde_json::from_str(&common::api_text("pollFrames-world.json")).unwrap();
    let latest: serde_json::Value = serde_json::from_str(&common::api_text("getFrame-latest.json")).unwrap();
    let fake = FakeFactory::new()
        .with_reply("pollFrames-world", Reply::json(200, &reply))
        .with_reply("getFrame-3079", Reply::json(200, &latest));
    let factory = fake.client();
    let mut stream = factory.streams().poll(PollOptions { backfill: true, ..PollOptions::default() });
    let mut seen = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(StreamItem::Frame(f)) = &item {
            seen.push(f.frame.g);
            if seen.len() == 1 {
                fake.fail("pollFrames", Fault::status(400, "invalid_param"));
            }
        }
        if item.is_err() {
            break;
        }
    }
    assert_eq!(seen[..2], [3078, 3079]);
}

fn only_frames(item: &pdigger_factory_sdk::Result<StreamItem>) -> futures::future::Ready<bool> {
    futures::future::ready(matches!(item, Ok(StreamItem::Frame(_))))
}

#[tokio::test(start_paused = true)]
async fn the_sdk_closes_a_connection_at_its_limit() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.set_close_delay(Duration::from_secs(10));
    let start = tokio::time::Instant::now();
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { limit: Some(10), ..world_cards() })
        .filter(only_frames)
        .take(35)
        .collect()
        .await;
    let frames = frames_of(&items);
    assert_eq!(frames.len(), 35);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    assert_eq!(start.elapsed(), Duration::ZERO, "no wait for the server's late close");
    assert_eq!(fake.streams_opened(), 4);
    let once: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { limit: Some(3), resume: false, ..world_cards() })
        .collect()
        .await;
    assert_eq!(frames_of(&once).len(), 3);
    assert_eq!(start.elapsed(), Duration::ZERO);
    let packets: Vec<_> = fake
        .client()
        .streams()
        .raw_packets(FramesOptions { limit: Some(2), resume: false, ..FramesOptions::default() })
        .collect()
        .await;
    assert_eq!(packets.iter().filter(|p| matches!(p, Ok(StreamItem::Packet(_)))).count(), 2);
    assert_eq!(start.elapsed(), Duration::ZERO);
    let all: Vec<_> = fake.client().streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    assert_eq!(frames_of(&all).len(), frame_count("dry1-0296-0306-full"));
    assert_eq!(start.elapsed(), Duration::from_secs(10), "without a limit a stream ends when the server closes it");
}

#[tokio::test(start_paused = true)]
async fn the_sdk_closes_a_connection_after_its_max_s() {
    let fake = recording_fake("dry1-0296-0306-full");
    fake.set_delay(|_, _| Some(Duration::from_millis(400)));
    fake.set_close_delay(Duration::from_secs(10));
    let start = tokio::time::Instant::now();
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { max_s: Some(2), ..world_cards() })
        .filter(only_frames)
        .take(12)
        .collect()
        .await;
    let frames = frames_of(&items);
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g), "nothing lost or repeated across the closes");
    assert!(fake.streams_opened() >= 3, "a connection lasts 2 s");
    assert!(fake.requests().iter().all(|r| r.url.contains("max_s=2")));
    assert!(start.elapsed() < Duration::from_secs(8));
}

#[tokio::test(start_paused = true)]
async fn backfill_runs_while_a_connection_is_quiet() {
    let fake = recording_fake("dry2-0582-0600");
    fake.set_delay(|_, _| Some(Duration::from_secs(1)));
    let options = FramesOptions { backfill: true, resume: false, max_s: Some(3600), ..world_cards() };
    let items: Vec<_> = fake.client().streams().frames(options).collect().await;
    let frames = frames_of(&items);
    assert_eq!(frames.len(), frame_count("dry2-0582-0600"));
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    assert_eq!(
        frames.iter().filter(|f| !f.frame.missing.is_empty()).count(),
        13,
        "the fake has no later world: released as they were"
    );
    assert_eq!(fake.requests().iter().filter(|r| r.url.contains("/session/frames/2938?")).count(), 3, "three tries");
}

#[tokio::test(start_paused = true)]
async fn an_error_that_ends_a_stream_comes_after_its_held_frames() {
    let fake = recording_fake("dry2-0582-0600");
    fake.drop_after("streamFrames", 30);
    let items: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    assert_eq!(items.last().unwrap().as_ref().unwrap_err().code, ErrorCode::Transport);
    assert_eq!(items.iter().filter(|i| i.is_err()).count(), 1);
    let frames = frames_of(&items);
    assert!(frames.iter().any(|f| f.frame.g == 2938), "the held frame comes out before the error");
    assert!(frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g));
    fake.drop_after("streamFrames", 30);
    let resumed: Vec<_> = fake
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, ..world_cards() })
        .filter(only_frames)
        .take(60)
        .collect()
        .await;
    let frames = frames_of(&resumed);
    assert!(
        frames.windows(2).all(|w| w[0].frame.g < w[1].frame.g),
        "a reconnect while frames are held keeps the order"
    );
    assert!(frames.iter().any(|f| f.frame.g == 2938));
}

#[tokio::test(start_paused = true)]
async fn the_long_poll_retries_and_ends_after_its_held_frames() {
    let (reply, first) = late_first_world();
    let fake = FakeFactory::new()
        .with_reply("pollFrames-world", Reply::json(200, &reply))
        .with_reply("getFrame-3078", Reply::json(200, &first));
    let factory = fake.client_builder().retries(0).build().unwrap();
    fake.fail("pollFrames", Fault::status(503, "busy"));
    let start = tokio::time::Instant::now();
    let mut stream = factory.streams().poll(PollOptions { backfill: true, ..PollOptions::default() });
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        if matches!(item, Ok(StreamItem::Worker(_))) {
            fake.fail("pollFrames", Fault::status(400, "invalid_param"));
        }
        items.push(item);
    }
    assert_eq!(fake.operations().iter().filter(|o| *o == "pollFrames").count(), 3, "a 503, a reply, the 400");
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| f.frame.g).collect::<Vec<_>>(), [3078, 3079]);
    assert!(frames[0].is_backfill && !frames[1].is_backfill);
    assert_eq!(items.len(), 5);
    assert_eq!(items[4].as_ref().unwrap_err().status, Some(400), "the error comes last");
    assert!(start.elapsed() >= Duration::from_millis(1500));
}

/// The fake behind a transport that answers the requests whose URL `slow` picks only after
/// `wait` (tokio time).
struct Slow {
    fake: FakeFactory,
    slow: fn(&str) -> bool,
    wait: Duration,
}

impl Transport for Slow {
    fn send(&self, request: Request) -> BoxFuture<'static, pdigger_factory_sdk::Result<Response>> {
        let wait = (self.slow)(&request.url).then_some(self.wait);
        let reply = self.fake.send(request);
        Box::pin(async move {
            if let Some(wait) = wait {
                tokio::time::sleep(wait).await;
            }
            reply.await
        })
    }
}

fn slow_client(fake: &FakeFactory, slow: fn(&str) -> bool, wait: Duration) -> FactoryClient {
    fake.client_builder().transport(Arc::new(Slow { fake: fake.clone(), slow, wait })).build().unwrap()
}

fn count(fake: &FakeFactory, operation: &str) -> usize {
    fake.operations().iter().filter(|o| o.as_str() == operation).count()
}

#[tokio::test(start_paused = true)]
async fn backfill_reads_are_single_requests_bounded_by_a_second() {
    let fake = FakeFactory::new()
        .with_reply(
            "streamFrames-sse",
            Reply::new(200, "text/event-stream", sse(&[frame(5, None, true), frame(6, Some(0), false)])),
        )
        .with_reply("getFrame-5", Reply::json(200, &frame(5, Some(0), false)));
    // every read takes 5 s: each try gives up after 1 s, and the frame goes out as it is
    let start = tokio::time::Instant::now();
    let items: Vec<_> = slow_client(&fake, |url| url.contains("/frames/5?"), Duration::from_secs(5))
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| (f.frame.g, f.is_backfill)).collect::<Vec<_>>(), [(5, false), (6, false)]);
    assert!(frames[0].frame.world.is_none());
    assert_eq!(start.elapsed(), Duration::from_millis(4500), "tries at 1.5, 2.5 and 3.5 s, each cut off after 1 s");
    assert_eq!(count(&fake, "getFrame"), 3);
    // a read that fails is not retried within its try: three tries, three requests
    let failing = FakeFactory::new()
        .with_reply("streamFrames-sse", Reply::new(200, "text/event-stream", sse(&[frame(5, None, true)])));
    failing.fail("getFrame", Fault::status(503, "not_ready").times(10));
    let items: Vec<_> = failing
        .client()
        .streams()
        .frames(FramesOptions { backfill: true, resume: false, ..world_cards() })
        .collect()
        .await;
    assert_eq!(frames_of(&items).len(), 1);
    assert_eq!(count(&failing, "getFrame"), 3);
}

#[tokio::test(start_paused = true)]
async fn the_state_is_read_after_every_connect() {
    let fake = FakeFactory::new().with_reply(
        "streamFrames-sse",
        Reply::new(200, "text/event-stream", sse(&[frame(5, Some(0), false), frame(6, Some(0), false)])),
    );
    let options = FramesOptions { clock_on_connect: true, ..world_cards() };
    let items: Vec<_> = fake.client().streams().frames(options.clone()).take(4).collect().await;
    let kinds: Vec<&str> = items
        .iter()
        .map(|i| match i {
            Ok(StreamItem::Clock(_)) => "clock",
            Ok(StreamItem::Frame(f)) if f.frame.g == 5 => "f5",
            _ => "f6",
        })
        .collect();
    // the connection ends after its two frames; the next one starts with a state again
    assert_eq!(kinds, ["clock", "f5", "f6", "clock"], "a state before the frames of each connection");
    assert_eq!((count(&fake, "streamFrames"), count(&fake, "getSession")), (2, 2));
    // a state read that takes longer than 1 s is left out; the frames come all the same
    let start = tokio::time::Instant::now();
    let items: Vec<_> = slow_client(&fake, |url| url.ends_with("/api/v2/session"), Duration::from_secs(2))
        .streams()
        .frames(FramesOptions { resume: false, ..options })
        .collect()
        .await;
    assert_eq!(frames_of(&items).len(), items.len());
    assert_eq!(start.elapsed(), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_silent_connection_is_closed_and_resumed() {
    let fake = recording_fake("dry1-0296-0306-full");
    let delayed = Arc::new(AtomicBool::new(false));
    let once = delayed.clone();
    fake.set_delay(move |op, i| {
        (op == "streamFrames" && i == 3 && !once.swap(true, Ordering::SeqCst)).then_some(Duration::from_secs(100))
    });
    let start = tokio::time::Instant::now();
    let items: Vec<_> = fake.client().streams().frames(world_cards()).filter(only_frames).take(6).collect().await;
    let waited = start.elapsed();
    assert!(waited >= Duration::from_secs(30) && waited < Duration::from_secs(32), "{waited:?}");
    let gs: Vec<i64> = frames_of(&items).iter().map(|f| f.frame.g).collect();
    assert!(gs.windows(2).all(|w| w[1] == w[0] + 1), "no frame lost or repeated: {gs:?}");
    let streams: Vec<_> = fake.requests().into_iter().filter(|r| r.url.contains("/frames/stream")).collect();
    assert_eq!(streams.len(), 2);
    assert!(streams[1].headers.iter().any(|(name, _)| name == "Last-Event-ID"));
    // with the watchdog off the connection waits
    let quiet = recording_fake("dry1-0296-0306-full");
    quiet.set_delay(|op, i| (op == "streamFrames" && i == 3).then_some(Duration::from_secs(100)));
    let client = quiet.client_builder().stream_idle_timeout(Duration::ZERO).build().unwrap();
    let frames = client.streams().frames(world_cards()).filter(only_frames).take(6).collect::<Vec<_>>();
    assert!(tokio::time::timeout(Duration::from_secs(60), frames).await.is_err(), "still waiting after 60 s");
    assert_eq!(count(&quiet, "streamFrames"), 1);
}

#[tokio::test(start_paused = true)]
async fn a_stream_without_reply_headers_counts_as_broken() {
    let fake = recording_fake("dry1-0296-0306-full");
    let client = fake
        .client_builder()
        .transport(Arc::new(Slow {
            fake: fake.clone(),
            slow: |url| url.contains("/frames/stream"),
            wait: Duration::from_secs(60),
        }))
        .request_timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let start = tokio::time::Instant::now();
    let items: Vec<_> = client.streams().frames(FramesOptions { resume: false, ..world_cards() }).collect().await;
    let error = items[0].as_ref().unwrap_err();
    assert_eq!((error.code.clone(), start.elapsed()), (ErrorCode::Transport, Duration::from_secs(2)));
    assert!(error.message.contains("no reply headers within the request timeout"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn the_long_poll_counts_as_a_viewer_unless_observing() {
    let reply: serde_json::Value = serde_json::from_str(&common::api_text("pollFrames-world.json")).unwrap();
    let fake = FakeFactory::new().with_reply("pollFrames-world", Reply::json(200, &reply));
    let client = fake.client();
    for options in [
        PollOptions::default(),
        PollOptions { observe: Some(true), ..PollOptions::default() },
        PollOptions { cid: Some("tab-0123456789".into()), ..PollOptions::default() },
    ] {
        let _: Vec<_> = client.streams().poll(options).take(1).collect().await;
    }
    let polls: Vec<String> =
        fake.requests().into_iter().map(|r| r.url).filter(|url| url.contains("/session/frames?")).collect();
    assert_eq!(polls.len(), 3);
    let cid = polls[0].split("cid=").nth(1).unwrap().split('&').next().unwrap();
    assert!(cid.starts_with("rs-") && cid.len() == 19, "{cid}");
    assert!(cid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'), "{cid}");
    assert!(!polls[1].contains("cid="), "observing: not a viewer");
    assert!(polls[2].contains("cid=tab-0123456789"));
}

#[tokio::test(start_paused = true)]
async fn held_frames_are_read_back_while_a_poll_waits() {
    let (reply, first) = late_first_world();
    let fake = FakeFactory::new()
        .with_reply("pollFrames-world", Reply::json(200, &reply))
        .with_reply("getFrame-3078", Reply::json(200, &first));
    // every poll takes 2 s; the read back is due 1.5 s after the frame came, during the next poll
    let client = slow_client(&fake, |url| url.contains("/session/frames?"), Duration::from_secs(2));
    let start = tokio::time::Instant::now();
    let items: Vec<_> = client
        .streams()
        .poll(PollOptions { backfill: true, ..PollOptions::default() })
        .filter(only_frames)
        .take(2)
        .collect()
        .await;
    let frames = frames_of(&items);
    assert_eq!(frames.iter().map(|f| (f.frame.g, f.is_backfill)).collect::<Vec<_>>(), [(3078, true), (3079, false)]);
    assert_eq!(start.elapsed(), Duration::from_millis(3500));
}

#[tokio::test(start_paused = true)]
async fn open_streams_count_as_viewers_until_they_close() {
    use pdigger_factory_sdk::api::{PollQuery, StreamQuery};
    let fake = FakeFactory::from_recording(recording("dry1-0296-0306-full")).unwrap();
    let api = fake.client().api();
    let held = api.stream_frames(&StreamQuery::default()).await.unwrap();
    let (health, status) = (api.get_health().await.unwrap(), api.get_status().await.unwrap());
    assert_eq!((health.viewers, status.viewers, status.streams), (1, 1, Some(1)));
    drop(held);
    assert_eq!(api.get_health().await.unwrap().viewers, 0, "a stream dropped is no viewer");
    let observing = api.stream_frames(&StreamQuery { observe: true, ..StreamQuery::default() }).await.unwrap();
    let status = api.get_status().await.unwrap();
    assert_eq!((status.viewers, status.streams), (0, Some(1)), "an observer is no viewer");
    assert!(observing.count().await > 0);
    assert_eq!(api.get_status().await.unwrap().streams, Some(0), "a stream that ended is closed");
    api.poll_frames(&PollQuery { cid: Some("client-0001".into()), ..PollQuery::default() }).await.unwrap();
    assert_eq!(api.get_health().await.unwrap().viewers, 0, "a long poll is no viewer");
}
