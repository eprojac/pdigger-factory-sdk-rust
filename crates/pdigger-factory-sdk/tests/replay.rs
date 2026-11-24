//! Replays of odd recordings: no header, blank and non-object lines, clock lines that are not a
//! state, invalid lines and readers that fail.

use std::io::Read;

use futures::StreamExt;
use pdigger_factory_sdk::replay::{Line, parse_line};
use pdigger_factory_sdk::{ErrorCode, Replay};

/// A frame with every field spec 2.3 rule 9 checks (`"g": 10, "t": 1.0` twice: in the frame and in its world).
const FRAME: &str = r#"{"cursor": 1, "session": 2, "video": "v", "g": 10, "t": 1.0, "i": 20, "past_valid": true, "dt_past": 0.6, "world": {"schema": "stw1", "session": 2, "video": "v", "g": 10, "t": 1.0, "motion": "static", "logF": 0.0, "hands": {}, "stats": {}, "events": [{"type": "lift", "id": 5, "side": "left"}], "stones": [{"id": 5}]}}"#;

#[test]
fn lines_without_a_header() {
    // a clock line that does not decode, and a gap line that does (a side line the frames pass over)
    let side = r#"{"sse": "clock", "data": {"session": "x"}}
{"sse": "gap", "data": {"gap": 2}}"#;
    let text = format!("\n[1, 2]\n{side}\n{{\"x\": 1}}\n{FRAME}\n");
    let replay = Replay::from_bytes(text.clone()).unwrap();
    assert!(replay.header().is_none());
    assert!(format!("{replay:?}").contains("Replay"));
    let out = replay.run().unwrap();
    assert_eq!(out.events.len(), 1);
    assert_eq!(out.events[0].at().wall, None, "a clock line that is not a state sets no clock");
    assert_eq!(Replay::from_bytes(text).unwrap().frames().count(), 1);
    assert!(matches!(parse_line(r#"{"x": 1}"#, true), Ok(None)));
    assert!(matches!(parse_line("  ", false), Ok(None)));
    assert!(matches!(parse_line(r#"{"recording": "pfr1"}"#, true), Ok(None)), "a second header is not a header");
    assert!(matches!(parse_line(r#"{"recording": "pfr1"}"#, false), Ok(Some(Line::Header(_)))));
    assert_eq!(parse_line("{nope", false).unwrap_err().code, ErrorCode::Decode);
    assert!(matches!(parse_line(r#"["recording", "sse", 1, 2]"#, false), Ok(None)), "an array is not a line");
    assert_eq!(parse_line("nope", false).unwrap_err().code, ErrorCode::Decode);
}

#[test]
fn a_line_that_does_not_decode_as_what_it_claims_is_an_error() {
    let why = |line: &str, have_header: bool| parse_line(line, have_header).unwrap_err().message;
    // the fields spec 2.3 rule 9 checks: missing, null, or of the wrong type
    assert_eq!(
        why(r#"{"recording": 5}"#, false),
        "a recording's header that does not decode: recording is not a string"
    );
    assert_eq!(
        why(r#"{"recording": "pfr1", "frames": 1.5}"#, false),
        "a recording's header that does not decode: frames is not a whole number"
    );
    // Rust holds no string with a lone surrogate (spec 2.3 rule 9 leaves it to each language): no header then
    assert!(
        why(r#"{"recording": "pfr1", "name": "\ud800"}"#, false)
            .starts_with("a recording's header that does not decode: ")
    );
    assert_eq!(
        why(r#"{"sse": "gap", "data": {"gap": 1}, "recv": "x"}"#, true),
        "a recording's side line that does not decode: recv is not a number"
    );
    assert!(
        why(r#"{"sse": "gap", "data": {"gap": 1}, "recv": 1e400}"#, true)
            .starts_with("a recording's side line that does not decode: ")
    );
    let world5 = FRAME.replace(r#""world": {"#, r#""world": 5, "was": {"#);
    assert_eq!(why(&world5, true), "a recording's frame that does not decode: world: not an object");
    let session = FRAME.replacen(r#""session": 2"#, r#""session": null"#, 1);
    assert_eq!(why(&session, true), "a recording's frame that does not decode: no session");
    let big = FRAME.replacen(r#""g": 10"#, r#""g": 99999999999999999999"#, 1); // a whole number past i64
    assert!(why(&big, true).starts_with("a recording's frame that does not decode: "));
    // a field that is null counts as missing: an optional one is fine (missing, world, a header's lists)
    let nulls = FRAME.replacen(r#""i": 20,"#, r#""i": 20, "missing": null, "raw": null,"#, 1);
    assert!(matches!(parse_line(&nulls, true), Ok(Some(Line::Frame(f))) if f.missing.is_empty() && f.raw.is_none()));
    let header = r#"{"recording": "pfr1", "layers": null, "sessions": null, "redacted": null, "source": {"id": 5}}"#;
    let Ok(Some(Line::Header(h))) = parse_line(header, false) else { panic!("{header}") };
    assert!(
        h.layers.is_empty() && h.sessions.is_empty() && h.source.is_none(),
        "a source that does not decode is left out"
    );
}

#[test]
fn a_header_member_the_model_cannot_hold_is_left_out() {
    // spec 6: only the header's table decides; what the model cannot hold is left out, the rest kept as it is
    let odd = r#"{"recording": "pfr1", "name": "h", "frames": -1, "g_first": 99999999999999999999, "layers": [1], "redacted": [2], "sessions": ["a"], "complete": {"why": 5}, "holes_filled": "x"}"#;
    let Ok(Some(Line::Header(h))) = parse_line(odd, false) else { panic!("{odd}") };
    assert_eq!((h.name.as_deref(), h.frames, h.g_first), (Some("h"), Some(-1), None), "a whole number past i64");
    assert!(h.layers.is_empty() && h.redacted.is_empty() && h.sessions.is_empty());
    assert_eq!(
        (&h.extra["complete"], &h.extra["holes_filled"]),
        (&serde_json::json!({"why": 5}), &serde_json::json!("x"))
    );
    // a source is kept when it is a catalog entry, and left out when it lacks one of the entry's fields
    let source = |source: &str| match parse_line(&format!(r#"{{"recording": "pfr1", "source": {source}}}"#), false) {
        Ok(Some(Line::Header(h))) => h.source.map(|s| s.id),
        other => panic!("{other:?}"),
    };
    let entry =
        r#"{"id": "v", "fps": 5, "grid_step": 1, "proxy": "p.mp4", "proxy_wh": [816, 770], "native_wh": [1920, 1080]}"#;
    assert_eq!(source(entry), Some("v".to_string()));
    assert_eq!(source(r#"{"id": "v", "fps": 5}"#), None);
    assert_eq!(
        source(&entry.replace(r#""proxy_wh": [816, 770]"#, r#""proxy_wh": ["a"]"#)),
        None,
        "no catalog entry by its model"
    );
}

#[test]
fn a_clock_line_the_model_cannot_read_sets_no_clock() {
    // its fields are there with their types (spec 2.3 rule 9), but its focus is not one the model reads
    let clock = r#"{"sse": "clock", "data": {"session": 2, "revision": 1, "video": "v", "playing": true, "rate": 1.0, "anchor_media": 0.0, "anchor_wall": 100.0, "hold_until": 0.0, "changed_by": "viewer", "media_time": 0.0, "server_now": 100.0, "focus": {"n": "x"}}}"#;
    assert!(matches!(parse_line(clock, true), Ok(Some(Line::Side(_)))));
    let out = Replay::from_bytes(format!("{clock}\n{FRAME}\n")).unwrap().run().unwrap();
    assert_eq!(out.events[0].at().wall, None);
    let readable = clock.replace(r#", "focus": {"n": "x"}"#, "");
    let out = Replay::from_bytes(format!("{readable}\n{FRAME}\n")).unwrap().run().unwrap();
    assert!(out.events[0].at().wall.is_some(), "the same clock without that focus sets the clock");
}

/// Two frames (g 10 and 11) with lines a reader does not understand between them (spec 6): not
/// JSON, a frame whose `g` is not a number, a side line whose `sse` is not a name, not UTF-8.
fn with_bad_lines() -> Vec<u8> {
    let bad_g = FRAME.replace(r#""g": 10"#, r#""g": "x""#);
    let mut bytes = format!("{FRAME}\n{{not json\n{bad_g}\n{{\"sse\": 5, \"data\": {{}}}}\n").into_bytes();
    bytes.extend_from_slice(b"{\"x\": \"\xff\"}\n{\xff\n");
    bytes.extend_from_slice(format!("{}\n", FRAME.replace(r#""g": 10"#, r#""g": 11"#)).as_bytes());
    bytes
}

#[test]
fn lines_a_reader_does_not_understand_are_skipped() {
    let frames: Vec<_> = Replay::from_bytes(with_bad_lines()).unwrap().frames().map(|f| f.unwrap().g).collect();
    assert_eq!(frames, [10, 11]);
    let updates: Vec<_> = Replay::from_bytes(with_bad_lines()).unwrap().updates().map(Result::unwrap).collect();
    assert_eq!(
        updates.iter().map(|u| (u.frame.as_ref().map(|f| f.g), u.is_final)).collect::<Vec<_>>(),
        [(Some(10), false), (Some(11), false), (None, true)]
    );
    assert_eq!(Replay::from_bytes(with_bad_lines()).unwrap().summary().unwrap().frames, 2);
    assert_eq!(Replay::from_bytes(with_bad_lines()).unwrap().run().unwrap().events.len(), 2);
    let replay = Replay::from_bytes([b"x\n".to_vec(), with_bad_lines()].concat()).unwrap(); // a bad first line too
    assert!(replay.header().is_none());
    assert_eq!(replay.frames().count(), 2);
}

#[tokio::test]
async fn lines_a_reader_does_not_understand_are_skipped_in_the_async_replay() {
    let updates: Vec<_> = Replay::from_bytes(with_bad_lines()).unwrap().into_stream().updates().collect().await;
    assert_eq!(updates.len(), 3);
    assert!(updates.iter().all(Result::is_ok) && updates[2].as_ref().unwrap().is_final);
    assert_eq!(Replay::from_bytes(with_bad_lines()).unwrap().into_stream().summary().await.unwrap().frames, 2);
}

/// A reader that gives `data` (as much as each read asks for), then fails on every read.
struct Failing {
    data: Option<Vec<u8>>,
}

impl Read for Failing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let Some(data) = self.data.as_mut() else { return Err(std::io::Error::other("the disk went away")) };
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        data.drain(..n);
        if data.is_empty() {
            self.data = None;
        }
        Ok(n)
    }
}

/// A reader that gives one byte a read (a pipe, a socket).
struct OneByte(std::io::Cursor<Vec<u8>>);

impl Read for OneByte {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(1);
        self.0.read(&mut buf[..n])
    }
}

fn header_and_frame() -> Vec<u8> {
    format!("{{\"recording\": \"pfr1\", \"video\": \"v\"}}\n{FRAME}\n").into_bytes()
}

fn gzipped(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut gz, bytes).unwrap();
    gz.finish().unwrap()
}

#[test]
fn readers_that_fail() {
    let replay = Replay::from_reader(Failing { data: Some(header_and_frame()) }).unwrap();
    assert_eq!(replay.header().and_then(|h| h.video.as_deref()), Some("v"));
    let lines: Vec<_> = replay.lines().take(100).collect();
    assert_eq!(lines.len(), 2, "a read error ends the lines");
    assert!(matches!(lines[0], Ok(Line::Frame(_))));
    let error = lines[1].as_ref().unwrap_err();
    assert_eq!(error.code, ErrorCode::Decode);
    assert!(error.message.contains("the disk went away") && std::error::Error::source(error).is_some());
    // the updates end with the error, without a final update (as in the other languages)
    let updates: Vec<_> =
        Replay::from_reader(Failing { data: Some(header_and_frame()) }).unwrap().updates().take(100).collect();
    assert_eq!(updates.len(), 2);
    assert!(updates[0].as_ref().is_ok_and(|u| !u.is_final) && updates[1].is_err());
    assert!(Replay::from_reader(Failing { data: Some(b"\n".to_vec()) }).is_err(), "no first line");
    assert!(Replay::from_reader(Failing { data: None }).unwrap_err().message.starts_with("cannot read the recording"));
}

#[tokio::test]
async fn a_cut_gzip_file_ends_with_one_error() {
    let mut cut = gzipped(&[header_and_frame(), format!("{FRAME}\n").repeat(20).into_bytes()].concat());
    cut.truncate(cut.len() - 10);
    let frames: Vec<_> = Replay::from_bytes(cut.clone()).unwrap().frames().take(100).collect();
    assert_eq!(frames.iter().filter(|f| f.is_err()).count(), 1);
    assert!(frames.last().unwrap().is_err(), "the error is the last item");
    let updates: Vec<_> = Replay::from_bytes(cut.clone()).unwrap().updates().take(100).collect();
    assert!(updates.last().unwrap().is_err() && !updates.iter().any(|u| u.as_ref().is_ok_and(|u| u.is_final)));
    let streamed: Vec<_> = Replay::from_bytes(cut.clone()).unwrap().into_stream().updates().take(100).collect().await;
    assert!(streamed.last().unwrap().is_err(), "no final update after the error");
    let events: Vec<_> = Replay::from_bytes(cut.clone()).unwrap().events().take(100).collect();
    assert!(events.last().unwrap().is_err(), "the events end with the read error");
    let streamed: Vec<_> = Replay::from_bytes(cut.clone()).unwrap().into_stream().events().take(100).collect().await;
    assert!(streamed.last().unwrap().is_err());
    assert!(Replay::from_bytes(cut).unwrap().summary().is_err());
}

#[test]
fn a_reader_of_one_byte_a_read() {
    for bytes in [gzipped(&header_and_frame()), header_and_frame()] {
        let replay = Replay::from_reader(OneByte(std::io::Cursor::new(bytes))).unwrap();
        assert_eq!(replay.header().and_then(|h| h.video.as_deref()), Some("v"));
        assert_eq!(replay.frames().filter(Result::is_ok).count(), 1);
    }
    assert!(Replay::from_reader(OneByte(std::io::Cursor::new(b"\n".to_vec()))).unwrap().header().is_none());
    assert_eq!(Replay::from_reader(OneByte(std::io::Cursor::new(Vec::new()))).unwrap().frames().count(), 0);
}

#[test]
fn a_line_that_is_not_utf8_is_skipped() {
    let bytes = [format!("{FRAME}\n").into_bytes(), b"\xff\n".to_vec(), format!("{FRAME}\n").into_bytes()].concat();
    let lines: Vec<_> = Replay::from_bytes(bytes).unwrap().lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|l| matches!(l, Ok(Line::Frame(_)))));
    // a byte that is not UTF-8 inside a string reads as the replacement character, as in the other languages
    let mut bytes = FRAME.replace(r#""video": "v""#, r#""video": "v@""#).into_bytes();
    let at = bytes.iter().position(|&b| b == b'@').unwrap();
    bytes[at] = 0xff;
    let frames: Vec<_> = Replay::from_bytes(bytes).unwrap().frames().map(Result::unwrap).collect();
    assert_eq!(frames[0].video, "v\u{fffd}");
}

#[tokio::test(start_paused = true)]
async fn pacing_too_far_ahead_starts_again() {
    let later = FRAME.replace("\"g\": 10, \"t\": 1.0", "\"g\": 11, \"t\": 3.0");
    let text = format!("{FRAME}\n{later}\n");
    let start = tokio::time::Instant::now();
    let frames: Vec<_> = Replay::from_bytes(text).unwrap().into_stream().realtime(1e-300).frames().collect().await;
    assert_eq!(frames.len(), 2);
    assert_eq!(start.elapsed(), std::time::Duration::ZERO, "2 s of media at a speed of 1e-300: no wait");
}
