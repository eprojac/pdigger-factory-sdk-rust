//! The raw packet's 4x stones block (API 2.2, `Sip1Stones`): exactly the spec's fields typed, every
//! map on its own grid. `tests/data/sample-packet-hr-dry1.json` is a packet of the 4x stone head
//! (dry1, g 736), redacted as the published recordings are: no face keypoints, appearance masks or
//! depth texture.

use std::path::Path;

use pdigger_factory_sdk::codec::decode_zlib_map;
use pdigger_factory_sdk::models::{Sip1Packet, Sip1Stones};

/// A JSON file of the SDK folder, by its path from the crate.
fn read(path: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn packet() -> serde_json::Value {
    read("tests/data/sample-packet-hr-dry1.json")
}

#[test]
fn the_stones_type_is_the_specs_block() {
    // every field by name (a field added to the type and not here does not compile), each one set, so each is written
    let every = Sip1Stones {
        present: true,
        count: 1,
        bbox: vec![0.0; 4],
        rot: true,
        grid: vec![160, 256],
        fg: Some(String::new()),
        tray_grid: vec![80, 128],
        tray: Some(String::new()),
        items: vec![vec![0.0; 11]],
        source: Some("hr4".into()),
        centres: Some("base".into()),
        centre_grid: vec![20, 32],
        extra: Default::default(),
    };
    let written = serde_json::to_value(&every).unwrap();
    let mut typed: Vec<&String> = written.as_object().unwrap().keys().collect();
    let spec = read("../../spec/openapi.json");
    let block = &spec["components"]["schemas"]["Sip1Packet"]["properties"]["stones"]["properties"];
    let mut listed: Vec<&String> = block.as_object().unwrap().keys().collect();
    typed.sort();
    listed.sort();
    assert_eq!(typed, listed);
}

#[test]
fn the_4x_block_is_typed_and_its_maps_decode_on_their_grids() {
    let p: Sip1Packet = serde_json::from_value(packet()).unwrap();
    let s = &p.stones;
    assert!(s.extra.is_empty(), "untyped: {:?}", s.extra.keys().collect::<Vec<_>>());
    assert!(p.extra.is_empty());
    assert_eq!(
        (s.source.as_deref(), s.centres.as_deref(), s.centre_grid.as_slice()),
        (Some("hr4"), Some("base"), &[20, 32][..])
    );
    assert_eq!((s.grid.as_slice(), s.tray_grid.as_slice()), (&[160, 256][..], &[80, 128][..]));
    assert_eq!((s.count, s.items.len()), (39, 39));
    let fg = decode_zlib_map(s.fg.as_deref(), s.grid[0], s.grid[1]).unwrap().unwrap();
    assert_eq!((fg.rows, fg.cols, fg.data.len()), (160, 256, 40960));
    assert!(fg.count(None) > 0);
    let tray = decode_zlib_map(s.tray.as_deref(), s.tray_grid[0], s.tray_grid[1]).unwrap().unwrap();
    assert_eq!((tray.rows, tray.cols, tray.data.len()), (80, 128, 10240));
}

#[test]
fn a_base_block_has_none_of_the_4x_fields_and_writes_none() {
    let mut v = packet();
    let block = v["stones"].as_object_mut().unwrap();
    for key in ["source", "centres", "centre_grid"] {
        block.remove(key);
    }
    let s: Sip1Stones = serde_json::from_value(v["stones"].clone()).unwrap();
    assert_eq!((s.source.as_deref(), s.centres.as_deref(), s.centre_grid.len()), (None, None, 0));
    assert!(s.extra.is_empty());
    let back = serde_json::to_value(&s).unwrap();
    for key in ["source", "centres", "centre_grid"] {
        assert!(back.get(key).is_none(), "{key} written for a base block");
    }
}
