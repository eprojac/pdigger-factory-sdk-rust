//! Renders the pictures of the docs headless (no window, no GPU) from the fixture recording
//! `dry1-0296-0306-full`: the viewer, the video-space overlay and the 3-D scene at g 1516 (the
//! right hand carries stone 643), and the output of the replay quickstart as an SVG; then the
//! viewer's reports on `dry1-0018-0153`, whose tray 1 arrives at 0:22.5 and leaves at 2:24.0.
//!
//!     cargo run -p pdigger-factory-viz --example screenshots -- docs/img
use std::path::{Path, PathBuf};

use egui::vec2;
use pdigger_factory_sdk::{Replay, Side};
use pdigger_factory_viz::craft::{Accent, Brightness, theme};
use pdigger_factory_viz::raster::render;
use pdigger_factory_viz::{OrbitCamera, OverlayView, SceneView, ViewerApp};

const RECORDING: &str = "dry1-0296-0306-full";
const FIXTURES: &str = "fixtures/recordings";

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    theme::apply(&ctx, Brightness::MediumDark, Accent::Copper);
    ctx
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "docs/img".to_string()));
    std::fs::create_dir_all(&out)?;
    let recording = format!("{FIXTURES}/{RECORDING}.ndjson.gz");
    let mut app = ViewerApp::from_replay(RECORDING, Replay::open(&recording)?)?;
    app.seek_time(302.3);
    let step = app.current().expect("the recording has frames");
    let (g, overlay, scene) =
        (step.frame.g, step.overlay.clone().expect("a world"), step.scene.clone().expect("a scene"));
    let write = |name: &str, png: Vec<u8>| -> std::io::Result<()> {
        let file = out.join(format!("{name}-{RECORDING}-g{g}.png"));
        std::fs::write(&file, png)?;
        println!("{}", file.display());
        Ok(())
    };
    write("viewer", render(&context(), vec2(1280.0, 720.0), 1.0, 3, |ui| app.ui(ui)).to_png())?;
    let view = OverlayView::new(&overlay).labels(true).highlight(Some(643));
    write("overlay", render(&context(), vec2(960.0, 540.0), 1.0, 2, |ui| drop(view.show(ui))).to_png())?;
    let mut camera = OrbitCamera::framing(&scene);
    let shot = render(&context(), vec2(720.0, 480.0), 1.0, 2, |ui| {
        drop(SceneView::new(&scene, &mut camera).highlight(Some(643)).show(ui))
    });
    write("scene", shot.to_png())?;
    let svg = out.join("quickstart-output.svg");
    std::fs::write(&svg, terminal(&recording)?)?;
    println!("{}", svg.display());
    // the Reports button: the trays that left and the report of the first
    let mut reports =
        ViewerApp::from_replay("dry1-0018-0153", Replay::open(format!("{FIXTURES}/dry1-0018-0153.ndjson.gz"))?)?;
    reports.show_report(Some(0));
    let file = out.join("reports-dry1-0018-0153.png");
    std::fs::write(&file, render(&context(), vec2(1280.0, 720.0), 1.0, 3, |ui| reports.ui(ui)).to_png())?;
    println!("{}", file.display());
    Ok(())
}

/// The output of `examples/replay.rs` over `recording` as a terminal picture: the command, the
/// activities (in their hand's colour) and the summary.
fn terminal(recording: &str) -> pdigger_factory_sdk::Result<String> {
    let out = Replay::open(recording)?.run()?;
    let command = format!("$ cargo run --example replay -- {}", Path::new(recording).display());
    let mut lines = vec![(command, "#c9a27a")];
    for activity in &out.activities {
        let colour = match activity.hand() {
            Some(Side::Left) => "#f68c36",
            Some(Side::Right) => "#51bcff",
            _ => "#e6e8ea",
        };
        lines.push((activity.describe(), colour));
    }
    lines.extend(out.summary.describe().lines().map(|l| (l.to_string(), "#a4b0ba")));
    let (width, line_height, top) =
        (8.2 * lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(40) as f64 + 48.0, 19.0, 48.0);
    let height = top + line_height * lines.len() as f64 + 20.0;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-label=\"Output of the replay quickstart over {RECORDING}\">\n\
         <rect width=\"100%\" height=\"100%\" rx=\"10\" fill=\"#1b1f24\"/>\n\
         <circle cx=\"20\" cy=\"18\" r=\"5\" fill=\"#ed6a5e\"/><circle cx=\"38\" cy=\"18\" r=\"5\" fill=\"#f5bf4f\"/><circle cx=\"56\" cy=\"18\" r=\"5\" fill=\"#61c454\"/>\n\
         <text font-family=\"ui-monospace, SFMono-Regular, Menlo, Consolas, monospace\" font-size=\"13\" xml:space=\"preserve\">\n"
    );
    for (i, (line, colour)) in lines.iter().enumerate() {
        let y = top + line_height * i as f64;
        svg += &format!("<tspan x=\"24\" y=\"{y:.0}\" fill=\"{colour}\">{}</tspan>\n", escape(line));
    }
    svg += "</text>\n</svg>\n";
    Ok(svg)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
