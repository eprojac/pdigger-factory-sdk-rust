# pdigger-factory-viz

View a factory camera feed or replay a recording with the `pdigger-factory-viewer` application. The same egui
widgets can be embedded in a Rust application.

## Run the viewer

Install with Rust 1.95 or later, then pass a recording file, a recording URL or a live unit address.

```sh
cargo install --locked --git https://github.com/eprojac/pdigger-factory-sdk-rust pdigger-factory-viz
pdigger-factory-viewer https://factory.pdigger.com/developers/data/dry1.world.ndjson.gz
pdigger-factory-viewer http://factory-orin.local:8120
```

![Viewer with the right hand carrying stone 643](https://raw.githubusercontent.com/eprojac/pdigger-factory-sdk-rust/main/docs/img/viewer-dry1-0296-0306-full-g1516.png)

Frame 1516 of `dry1-0296-0306-full`, at 5:02.4. The viewer processes a full recording before playback. As a
reference, `dry1.world` (9,042 frames) took about 14 seconds and 1.5 GB of memory in a release build.

## Widgets

| Widget | Use |
| --- | --- |
| `OverlayView` | Draw stones, hand joints and the selected stone over a frame. Supports hover picking. |
| `SceneView` | Orbit a 3D view of the board, stone boxes and hand skeletons. |
| `EventFeed` and `HandCards` | Show events and the current activity of each hand. |
| `ReportView` | Compare the tray at arrival and departure, with changed stones labelled and the report text alongside. `report_marks` and `REPORT_COLORS` expose the same marks and colours. |
| `ViewerApp` | Combine the views with playback controls and a list of finished tray reports. |
| `raster::render` | Render a view without a window or GPU. |

The widgets use egui 0.37. The Craft theme and controls come from TextureCraft and craft-libs under MIT or
Apache-2.0. [Third-party notices](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/NOTICE).

[Viewer and widget guide](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/docs/viz.md) ·
[SDK](https://github.com/eprojac/pdigger-factory-sdk-rust/tree/main/crates/pdigger-factory-sdk) ·
[Apache License 2.0](https://github.com/eprojac/pdigger-factory-sdk-rust/blob/main/LICENSE)
