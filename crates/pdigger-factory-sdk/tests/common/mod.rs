//! Shared helpers of the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use pdigger_factory_sdk::testing::FakeFactory;

/// The repository's `fixtures/` (shared by the SDK's four languages).
pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Every recording of `fixtures/recordings`, by name, sorted.
pub fn recordings() -> Vec<(String, PathBuf)> {
    let mut all: Vec<(String, PathBuf)> = std::fs::read_dir(fixtures().join("recordings"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter_map(|p| {
            let file = p.file_name()?.to_str()?.to_string();
            let name = file.strip_suffix(".ndjson.gz").or_else(|| file.strip_suffix(".ndjson"))?.to_string();
            Some((name, p))
        })
        .collect();
    all.sort();
    all
}

/// One recording by name.
pub fn recording(name: &str) -> PathBuf {
    fixtures().join("recordings").join(format!("{name}.ndjson.gz"))
}

/// JSON with every number as an f64, so `1` and `1.0` compare equal.
pub fn normalise(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match value {
        Value::Number(n) => n.as_f64().and_then(serde_json::Number::from_f64).map_or(Value::Null, Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(normalise).collect()),
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, normalise(v))).collect()),
        other => other,
    }
}

/// A test frame (JSON) with every field spec 2.3 rule 9 checks, so that it decodes as a unit's
/// frames do: the ones it lacks are filled in (`session` 1, `video` "v", `i` 2g, `t` g/5,
/// `past_valid` true, `dt_past` 0.6, and a world's or a cards layer's own from the frame or empty).
pub fn complete(mut frame: serde_json::Value) -> serde_json::Value {
    use serde_json::json;
    let g = frame["g"].as_i64().unwrap_or(0);
    let f = frame.as_object_mut().expect("a frame is an object");
    for (key, value) in [
        ("session", json!(1)),
        ("video", json!("v")),
        ("i", json!(2 * g)),
        ("t", json!(g as f64 * 0.2)),
        ("past_valid", json!(true)),
        ("dt_past", json!(0.6)),
    ] {
        f.entry(key).or_insert(value);
    }
    let (session, video, t) = (f["session"].clone(), f["video"].clone(), f["t"].clone());
    if let Some(world) = f.get_mut("world").and_then(serde_json::Value::as_object_mut) {
        for (key, value) in [
            ("schema", json!("stw1")),
            ("session", session),
            ("video", video),
            ("g", json!(g)),
            ("t", t),
            ("motion", json!("static")),
            ("logF", json!(0.0)),
            ("stones", json!([])),
            ("hands", json!({})),
            ("events", json!([])),
            ("stats", json!({})),
        ] {
            world.entry(key).or_insert(value);
        }
    }
    if let Some(cards) = f.get_mut("cards").and_then(serde_json::Value::as_object_mut) {
        for (key, value) in
            [("schema", json!("crd1")), ("left", json!({})), ("right", json!({})), ("numbers", json!({}))]
        {
            cards.entry(key).or_insert(value);
        }
    }
    frame
}

/// A fake with the recorded 20.20 replies of `fixtures/api` and no recording.
pub fn replies_fake() -> FakeFactory {
    FakeFactory::new().with_replies(fixtures().join("api")).unwrap()
}

/// A fake serving a fixture recording (and the recorded replies for what it cannot answer).
pub fn recording_fake(name: &str) -> FakeFactory {
    FakeFactory::from_recording(recording(name)).unwrap().with_replies(fixtures().join("api")).unwrap()
}

/// The text of a fixture reply file.
pub fn api_text(file: &str) -> String {
    std::fs::read_to_string(fixtures().join("api").join(file)).unwrap()
}
