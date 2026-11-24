//! fixtures/damaged: recordings and streams with lines and events that do not decode (spec
//! section 6, 2.3 rule 9). Every SDK replays and streams them and gets what their JSON files say.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use pdigger_factory_sdk::models::EventKind;
use pdigger_factory_sdk::testing::{FakeFactory, Fault, Reply};
use pdigger_factory_sdk::{ErrorCode, FramesOptions, Replay, ReplayStream, StreamItem, Update};
use serde_json::Value;

use common::fixtures;

fn damaged(name: &str) -> PathBuf {
    fixtures().join("damaged").join(name)
}

fn expected(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(damaged(name)).unwrap()).unwrap()
}

fn numbers(value: &Value) -> Vec<i64> {
    serde_json::from_value(value.clone()).unwrap()
}

/// The frames' `g` and wall times and the final update's activities, as `lines.json` gives them.
fn check_updates(updates: &[Update], want: &Value) {
    let (last, frames) = updates.split_last().expect("updates");
    assert_eq!(frames.iter().map(|u| u.frame.as_ref().unwrap().g).collect::<Vec<_>>(), numbers(&want["updates"]));
    let walls: Vec<f64> = serde_json::from_value(want["walls"].clone()).unwrap();
    assert_eq!(frames.len(), walls.len());
    for (update, wall) in frames.iter().zip(walls) {
        let got = update.state.wall.expect("a wall time");
        assert!((got - wall).abs() < 1e-6, "g {}: {got} != {wall}", update.state.g.unwrap_or(-1));
    }
    assert!(last.is_final);
    let activities: Vec<String> = last.activities.iter().map(|a| a.describe()).collect();
    assert_eq!(activities, serde_json::from_value::<Vec<String>>(want["final_activities"].clone()).unwrap());
}

/// The header as the JSON files give it: its name and whether it has a source, or null.
fn header_of(header: Option<&pdigger_factory_sdk::RecordingHeader>) -> Value {
    header.map_or(Value::Null, |h| serde_json::json!({"name": h.name, "source": h.source.is_some()}))
}

#[test]
fn a_replay_skips_every_damaged_line() {
    let want = expected("lines.json");
    let replay = Replay::open(damaged("lines.ndjson")).unwrap();
    assert_eq!(header_of(replay.header()), want["header"]);
    check_updates(&replay.updates().map(Result::unwrap).collect::<Vec<_>>(), &want);
    let bytes = std::fs::read(damaged("lines.ndjson")).unwrap();
    check_updates(&Replay::from_bytes(bytes.clone()).unwrap().updates().map(Result::unwrap).collect::<Vec<_>>(), &want);
    let summary = Replay::from_bytes(bytes).unwrap().summary().unwrap();
    assert_eq!(serde_json::json!({"frames": summary.frames, "sessions": summary.sessions}), want["summary"]);
}

#[tokio::test]
async fn an_async_replay_skips_every_damaged_line() {
    let want = expected("lines.json");
    let replay = ReplayStream::open(damaged("lines.ndjson")).unwrap();
    assert_eq!(header_of(replay.header()), want["header"]);
    let updates: Vec<_> = replay.updates().map(Result::unwrap).collect().await;
    check_updates(&updates, &want);
    let summary = ReplayStream::open(damaged("lines.ndjson")).unwrap().summary().await.unwrap();
    assert_eq!(summary.frames, want["summary"]["frames"].as_u64().unwrap());
}

#[tokio::test]
async fn the_header_is_the_first_line_when_it_decodes() {
    let mut names: Vec<String> = std::fs::read_dir(fixtures().join("damaged"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| f.starts_with("header-") && f.ends_with(".ndjson"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 11);
    for name in names {
        let want = expected(&name.replace(".ndjson", ".json"));
        let replay = Replay::open(damaged(&name)).unwrap();
        assert_eq!(header_of(replay.header()), want["header"], "{name}");
        let frames: Vec<i64> = replay.frames().map(|f| f.unwrap().g).collect();
        assert_eq!(frames, numbers(&want["updates"]), "{name}");
        let streamed = ReplayStream::open(damaged(&name)).unwrap();
        assert_eq!(header_of(streamed.header()), want["header"], "{name}");
    }
}

/// An SSE body of the events of a JSON file (`{id, event, data}`).
fn sse(events: &Value) -> String {
    let text = |e: &Value, key: &str| e[key].as_str().unwrap().to_string();
    let events = events.as_array().unwrap();
    events
        .iter()
        .map(|e| format!("id: {}\nevent: {}\ndata: {}\n\n", text(e, "id"), text(e, "event"), text(e, "data")))
        .collect()
}

/// A fake whose stream `operation` answers once with the events of a JSON file, then ends.
fn serving(operation: &str, events: &Value) -> FakeFactory {
    let fake = FakeFactory::new();
    fake.fail(operation, Fault::reply(Reply::new(200, "text/event-stream", sse(events))).times(3));
    fake
}

fn decode_errors<T>(items: &[pdigger_factory_sdk::Result<T>]) -> usize {
    items.iter().filter(|i| matches!(i, Err(e) if e.code == ErrorCode::Decode)).count()
}

#[tokio::test(start_paused = true)]
async fn a_stream_skips_every_event_that_does_not_decode() {
    let want = expected("lines.json");
    let stream = &want["stream"];
    let fake = serving("streamFrames", &stream["events"]);
    let options = FramesOptions { resume: false, ..FramesOptions::default() };
    let items: Vec<_> = fake.client().streams().frames(options).collect().await;
    let frames: Vec<i64> =
        items.iter().filter_map(|i| if let Ok(StreamItem::Frame(f)) = i { Some(f.frame.g) } else { None }).collect();
    assert_eq!(frames, numbers(&stream["frames"]));
    assert_eq!(decode_errors(&items) as u64, stream["decode_errors"].as_u64().unwrap());
    // the interpreter's updates go on past each one, to the final update
    let updates: Vec<_> = fake.client().interpreter().resume(false).updates().collect().await;
    assert_eq!(decode_errors(&updates) as u64, stream["decode_errors"].as_u64().unwrap());
    check_updates(&updates.into_iter().filter_map(Result::ok).collect::<Vec<_>>(), &want);
    // and so does wait_for
    let interpreter = fake.client().interpreter().resume(false);
    let found = interpreter.wait_for(EventKind::Confirm, Duration::from_secs(5)).await.unwrap().expect("a confirm");
    assert_eq!((found.kind.to_string(), found.g), ("confirm".to_string(), want["wait_for"]["g"].as_i64().unwrap()));
}

#[tokio::test(start_paused = true)]
async fn a_packet_stream_skips_every_packet_that_does_not_decode() {
    let packets = &expected("lines.json")["packets"];
    let fake = serving("rawStream", &packets["events"]);
    let options = FramesOptions { resume: false, ..FramesOptions::default() };
    let items: Vec<_> = fake.client().streams().raw_packets(options).collect().await;
    let g: Vec<i64> =
        items.iter().filter_map(|i| if let Ok(StreamItem::Packet(p)) = i { Some(p.g) } else { None }).collect();
    assert_eq!(g, numbers(&packets["g"]));
    assert_eq!(decode_errors(&items) as u64, packets["decode_errors"].as_u64().unwrap());
}
