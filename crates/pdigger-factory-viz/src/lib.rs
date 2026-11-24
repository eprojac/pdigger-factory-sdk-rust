//! egui visualisation helpers for the pDigger Factory SDK, in the Craft look of storytold's
//! TextureCraft (vendored in [`craft`]).
//!
//! - [`OverlayView`]: the video-space draw list ([`Overlay2D`](pdigger_factory_sdk::models::Overlay2D),
//!   `renderOverlay2d`, or [`draw::overlay_of_frame`] of any frame): stones by style, the
//!   skeleton, wrists, boxes, the focus, primitives; hover names a stone.
//! - [`SceneView`]: the 3-D draw list ([`Scene3D`](pdigger_factory_sdk::models::Scene3D)) through
//!   an [`OrbitCamera`], painted with egui's painter (no GPU).
//! - [`EventFeed`]: the interpreter's sentences, newest first, with hand colours.
//! - [`HandCards`]: what each hand is doing.
//! - [`ReportView`]: a tray's report (simple mode) as a picture: the tray as it arrived and as it
//!   left, from above, each stone coloured by what the report says of it ([`report_marks`]).
//! - [`ViewerApp`]: the first four in a split layout, playing a replay or following a server, and
//!   the reports of the trays that left; the `pdigger-factory-viewer` binary opens it in a window.
//! - [`raster`]: headless rendering to RGBA/PNG (the tests and the screenshots in `docs/img`).
//!
//! ```
//! use pdigger_factory_viz::ViewerApp;
//! # fn demo(replay: pdigger_factory_sdk::Replay) -> pdigger_factory_sdk::Result<()> {
//! let mut app = ViewerApp::from_replay("dry1", replay)?;
//! app.seek_time(301.6);
//! let shot = pdigger_factory_viz::raster::render(&egui::Context::default(), egui::vec2(1280.0, 720.0), 1.0, 3, |ui| app.ui(ui));
//! std::fs::write("viewer.png", shot.to_png()).ok();
//! # Ok(()) }
//! ```

pub mod app;
pub mod craft;
pub mod draw;
pub mod feed;
pub mod hands;
pub mod overlay;
pub mod raster;
pub mod report;
pub mod scene;

// The window glue: needs a display, so it is the one part the container tests cannot run
// (excluded from coverage, listed in TESTING.md).
pub mod native;

pub use app::{TrayMark, VIEWER_LAYERS, ViewerApp, ViewerInput, follow, follow_until};
pub use feed::{EventFeed, FeedLine};
pub use hands::HandCards;
pub use overlay::{OverlayResponse, OverlayView, VideoTransform};
pub use report::{REPORT_COLORS, ReportMark, ReportMarks, ReportRole, ReportView, report_marks};
pub use scene::{OrbitCamera, SceneView};
