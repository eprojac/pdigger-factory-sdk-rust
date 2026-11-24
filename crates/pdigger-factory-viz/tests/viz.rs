//! The widgets and the viewer, headless: egui contexts fed with input events, the shapes they
//! paint and software-rasterised frames, on the fixture recording `dry1-0296-0306-full` (the
//! carry of stone 643) and the recorded 20.20 renders; the viewer's reports on
//! `trays-synthetic`, `dry1-0018-0153` and `trays-presence-synthetic`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;

use egui::{
    Event, Modifiers, MouseWheelSource, MouseWheelUnit, PointerButton, Pos2, RawInput, Rect, TouchPhase, Ui, Vec2,
    pos2, vec2,
};
use pdigger_factory_sdk::models::{DrawStone3D, Layer, Overlay2D, Scene3D, Side, StoneFamily, StoneShape};
use pdigger_factory_sdk::testing::FakeFactory;
use pdigger_factory_sdk::{Activity, Frame, Replay, Update};
use pdigger_factory_viz::craft::{Accent, Brightness, theme};
use pdigger_factory_viz::draw::{overlay_of_frame, scene_of_frame, stone_style};
use pdigger_factory_viz::raster::render;
use pdigger_factory_viz::{
    EventFeed, FeedLine, HandCards, OrbitCamera, OverlayView, ReportView, SceneView, ViewerApp, ViewerInput, follow,
    follow_until,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

fn recording() -> PathBuf {
    fixtures().join("recordings/dry1-0296-0306-full.ndjson.gz")
}

fn frame(g: i64) -> Frame {
    Replay::open(recording()).unwrap().frames().map(Result::unwrap).find(|f| f.g == g).unwrap()
}

fn reply<T: serde::de::DeserializeOwned>(file: &str) -> T {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join("api").join(file)).unwrap()).unwrap()
}

/// An egui context with the Craft theme and a clock.
struct Harness {
    ctx: egui::Context,
    size: Vec2,
    time: f64,
}

impl Harness {
    fn new(width: f32, height: f32) -> Harness {
        let ctx = egui::Context::default();
        theme::apply(&ctx, Brightness::MediumDark, Accent::Copper);
        Harness { ctx, size: vec2(width, height), time: 0.0 }
    }

    /// One frame with these input events; returns the number of shapes painted.
    fn frame(&mut self, events: Vec<Event>, mut ui: impl FnMut(&mut Ui)) -> usize {
        self.time += 1.0 / 60.0;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            predicted_dt: 1.0 / 60.0,
            events,
            ..RawInput::default()
        };
        let mut output = self.ctx.run_ui(input, &mut ui);
        output.textures_delta.clear();
        output.shapes.len()
    }

    /// A click at `at` (press in one frame, release in the next).
    fn click(&mut self, at: Pos2, mut ui: impl FnMut(&mut Ui)) {
        self.frame(vec![Event::PointerMoved(at)], &mut ui);
        self.frame(
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            &mut ui,
        );
        self.frame(
            vec![Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            &mut ui,
        );
    }
}

#[test]
fn the_overlay_of_the_carry_of_stone_643() {
    let f = frame(1516);
    let overlay = overlay_of_frame(&f).unwrap();
    assert_eq!(overlay.native_wh, [1920, 1080]);
    assert_eq!(overlay.stones.len(), f.world.as_ref().unwrap().stones.len());
    let held = overlay.stones.iter().find(|s| s.id == Some(643)).unwrap();
    assert_eq!((held.style.as_str(), held.side.clone()), ("held", Some(Side::Right)));
    let skeleton = overlay.skeleton.as_ref().unwrap();
    assert!(skeleton["bones"].as_array().unwrap().len() > 20, "the bones of the joints not redacted");
    assert!(
        skeleton["joints"].as_array().unwrap().iter().all(|j| j["j"].as_u64().unwrap() > 4),
        "joints 0-4 are redacted"
    );
    assert_eq!(overlay.wrists.len(), 2);
    assert_eq!(overlay.boxes.len(), 2);
    assert!(overlay.boxes[0]["label"].as_str().unwrap().starts_with("person "));
    let world = f.world.as_ref().unwrap();
    assert!(
        world.stones.iter().any(|s| stone_style(s) == "detected")
            && world.stones.iter().any(|s| stone_style(s) == "kept")
    );
    let mut stone = world.stones.iter().find(|s| s.side.is_none()).unwrap().clone();
    stone.placed = Some(2);
    assert_eq!(stone_style(&stone), "placed");
    (stone.placed, stone.detected, stone.shape) = (None, false, StoneShape::Model);
    assert_eq!(stone_style(&stone), "model");
    // Missing layers and the focus are carried over.
    let mut late = f.clone();
    late.missing = vec![Layer::Cards];
    late.world.as_mut().unwrap().focus =
        Some(serde_json::from_value(serde_json::json!({"id": 643, "path2d": [[960.0, 540.0]], "path3d": []})).unwrap());
    let late = overlay_of_frame(&late).unwrap();
    assert_eq!(
        (late.missing.as_slice(), &late.focus.unwrap()["id"]),
        (["cards".to_string()].as_slice(), &serde_json::json!(643))
    );
    // Hovering stone 643 names it.
    let mut h = Harness::new(960.0, 540.0);
    let outline = &held.outline;
    let centre = outline
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0] / outline.len() as f64, a[1] + p[1] / outline.len() as f64]);
    let mut hovered = None;
    let view = OverlayView::new(&overlay).labels(true).highlight(Some(636));
    let at = view.transform(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 540.0))).to_screen(centre);
    h.frame(vec![Event::PointerMoved(at)], |ui| hovered = view.show(ui).hovered);
    h.frame(vec![], |ui| hovered = view.show(ui).hovered);
    assert_eq!(hovered, Some(643));
    let shot = render(&h.ctx, vec2(480.0, 270.0), 1.0, 2, |ui| {
        view.show(ui);
    });
    assert!(shot.count_not(shot.pixel(1, 1), 4) > 5000, "stones, bones and boxes were painted");
}

#[test]
fn recorded_renders_paint() {
    let shapes: Overlay2D = reply("renderOverlay2d.json");
    let primitives: Overlay2D = reply("renderOverlay2d-primitives.json");
    let scene: Scene3D = reply("renderScene3d.json");
    let mut h = Harness::new(800.0, 450.0);
    let n = h.frame(vec![], |ui| {
        OverlayView::new(&shapes).show(ui);
    });
    assert!(n > 0);
    let mut odd = primitives.clone();
    let p = odd.primitives.as_mut().unwrap();
    p.polygons[0].dash = None;
    p.polygons[1].stroke = None;
    p.polygons[2].points.truncate(1);
    p.segments[0].to.clear();
    p.circles[0].at.clear();
    (p.circles[1].stroke, p.circles[1].fill) = (Some("#ffffff".into()), None);
    p.labels[0].at.clear();
    p.labels[1].baseline = Some("bottom".into());
    p.lw = None;
    odd.boxes.push(serde_json::json!({"kind": "stone", "x0": 1}));
    odd.boxes.push(serde_json::json!({"kind": "person", "x0": 10.0, "y0": 10.0, "x1": 300.0, "y1": 200.0}));
    odd.wrists.push(serde_json::json!({"side": "left", "at": [100.0, 100.0], "filled": true, "ring": false}));
    odd.wrists.push(serde_json::json!({"side": "right"}));
    odd.wrists.push(serde_json::json!({"side": "both", "at": [200.0, 100.0], "label": "both"}));
    odd.wrists.push(serde_json::json!({"side": "nobody", "at": [300.0, 100.0]}));
    odd.stones.push(pdigger_factory_sdk::models::DrawStone2D {
        id: None,
        outline: vec![vec![1.0, 1.0]],
        style: "raw".into(),
        ..Default::default()
    });
    let unnamed = vec![vec![50.0, 50.0], vec![80.0, 50.0], vec![80.0, 80.0]];
    odd.stones.push(pdigger_factory_sdk::models::DrawStone2D {
        id: None,
        outline: unnamed,
        style: "held".into(),
        side: Some(Side::Left),
        ..Default::default()
    });
    odd.stones.push(pdigger_factory_sdk::models::DrawStone2D {
        id: Some(9),
        outline: vec![vec![0.0, 0.0], vec![40.0, 0.0], vec![20.0, 10.0], vec![40.0, 40.0], vec![0.0, 40.0]],
        style: "placed".into(),
        uncertain: Some(true),
        ..Default::default()
    });
    odd.skeleton = Some(serde_json::json!({"bones": [{"from": [1, 2]}], "joints": [{"j": 1}]}));
    for overlay in [&primitives, &odd] {
        let shot = render(&h.ctx, vec2(480.0, 270.0), 1.0, 2, |ui| {
            OverlayView::new(overlay).labels(true).show(ui);
        });
        assert!(shot.count_not(shot.pixel(1, 1), 4) > 500);
    }
    let mut camera = OrbitCamera::framing(&scene);
    let shot = render(&h.ctx, vec2(480.0, 270.0), 1.0, 2, |ui| {
        SceneView::new(&scene, &mut camera).highlight(Some(43)).show(ui);
    });
    assert!(shot.count_not(shot.pixel(1, 1), 4) > 2000, "the board, the stones and the skeleton");
    let mut listed = scene.clone();
    listed.skeleton.bones = vec![serde_json::json!({"a": 5, "b": 7}), serde_json::json!({"a": "x"})];
    listed.stones[0].q = vec![1.0];
    listed.stones[1].pos = vec![1.0];
    listed.stones[2].side = Some(Side::Left);
    let mut top = OrbitCamera::top([0.0, 0.0, 0.0], 3.0);
    h.frame(vec![], |ui| {
        SceneView::new(&listed, &mut top).show(ui);
    });
}

#[test]
fn dogbone_caps_leave_the_notches_open_and_the_waist_has_a_wall() {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(480.0, 360.0));
    let stone = DrawStone3D {
        family: StoneFamily::Dogbone,
        pos: vec![0.0, 0.0, 0.0],
        dims: vec![1.0, 0.6, 0.24],
        alpha: 1.0,
        ..Default::default()
    };
    let scene = Scene3D { stones: vec![stone], ..Default::default() };
    let camera = OrbitCamera::top([0.0; 3], 1.7);
    let ctx = egui::Context::default();
    let paint = |scene, mut camera| {
        render(&ctx, rect.size(), 1.0, 2, |ui| SceneView::new(scene, &mut camera).paint(ui.painter(), rect))
    };
    let empty = Scene3D::default();
    let sample = |camera: &OrbitCamera, shot: &pdigger_factory_viz::raster::Snapshot, point| {
        let (p, _) = camera.project(point, rect).unwrap();
        shot.pixel(p.x as usize, p.y as usize)
    };
    for sign in [1.0, -1.0] {
        let camera = OrbitCamera { pitch: sign * camera.pitch, ..camera };
        let (image, background) = (paint(&scene, camera), paint(&empty, camera));
        let z = sign * 0.12;
        let cap = sample(&camera, &image, [0.0, 0.0, z]);
        assert_ne!(cap, sample(&camera, &background, [0.0, 0.0, z]));
        for y in [-0.265, 0.265] {
            let notch = [0.0, y, z];
            assert_eq!(sample(&camera, &image, notch), sample(&camera, &background, notch), "notch at {notch:?}");
            for x in [-0.4, 0.4] {
                assert_eq!(sample(&camera, &image, [x, y, z]), cap, "full-width end");
            }
        }
    }
    let camera = OrbitCamera { pitch: 0.35, ..camera };
    let (image, background) = (paint(&scene, camera), paint(&empty, camera));
    let wall = [0.0, -0.365 * 0.6, 0.0];
    let side = sample(&camera, &image, wall);
    assert_ne!(side, sample(&camera, &background, wall), "the recessed wall has height");
    assert_ne!(side, sample(&camera, &image, [0.0, 0.0, 0.12]), "wall and cap have distinct shading");
    // Seen nearly end-on, the far shoulder projects behind the near end of the top face.
    let camera = OrbitCamera { pitch: 0.28, yaw: 2.86, ..camera };
    let image = paint(&scene, camera);
    assert_eq!(
        sample(&camera, &image, [-0.345, 0.267, 0.12]),
        sample(&camera, &image, [0.0, 0.0, 0.12]),
        "a recessed wall must not show through the cap"
    );
}

#[test]
fn faces_seen_almost_edge_on_draw_no_spikes() {
    // A top view looks down from 0.062 m behind its target (pitch 1.55): the +y faces of these stones, just behind
    // the eye, are seen almost edge-on. egui's anti-aliasing pushed the sharp corners of such slivers out into thin
    // lines across the view.
    let mut camera = OrbitCamera::top([0.0, 0.0, 0.0], 3.0);
    let eye = camera.eye();
    let stones: Vec<DrawStone3D> = [0.000_05, 0.000_5, 0.005]
        .iter()
        .enumerate()
        .map(|(i, gap)| DrawStone3D {
            id: Some(i as i64),
            pos: vec![(i as f64 - 1.0) * 0.4, f64::from(eye[1]) - gap - 0.1, 0.05],
            q: vec![0.0, 0.0, 0.0, 1.0],
            dims: vec![0.3, 0.2, 0.1],
            alpha: 1.0,
            ..Default::default()
        })
        .collect();
    let scene = Scene3D { stones: stones.clone(), ..Default::default() };
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(480.0, 270.0));
    let shot = render(&egui::Context::default(), rect.size(), 1.0, 2, |ui| {
        SceneView::new(&scene, &mut camera).paint(ui.painter(), rect);
    });
    let boxes: Vec<Rect> = stones
        .iter()
        .map(|s| {
            let corners: Vec<Pos2> = (0..8)
                .filter_map(|i| {
                    let at =
                        |k: usize, bit: usize| (s.pos[k] + if i & bit == 0 { -0.5 } else { 0.5 } * s.dims[k]) as f32;
                    camera.project([at(0, 1), at(1, 2), at(2, 4)], rect).map(|p| p.0)
                })
                .collect();
            Rect::from_points(&corners).expand(3.0)
        })
        .collect();
    let stray = (0..shot.height)
        .flat_map(|y| (0..shot.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, _, b, _] = shot.pixel(x, y);
            i32::from(r) > i32::from(b) + 15
                && !boxes.iter().any(|bx| bx.contains(pos2(x as f32 + 0.5, y as f32 + 0.5)))
        })
        .count();
    assert_eq!(stray, 0, "stone-coloured pixels away from the stones");
}

#[test]
fn a_board_seen_almost_edge_on_draws_no_spikes() {
    // From 1.4 cm above the board's plane (pitch -0.025 about a target 8 cm up) the board is a sliver about a
    // point and a half thin, with corners of about a degree at its ends.
    let board = pdigger_factory_sdk::models::Rect { hw: 0.8, hh: 0.7, ..Default::default() };
    let scene = Scene3D { board: Some(board), ..Default::default() };
    let camera = OrbitCamera { pitch: -0.025, ..OrbitCamera::framing(&scene) };
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(480.0, 270.0));
    let ctx = egui::Context::default();
    let paint = |scene: &Scene3D| {
        let mut camera = camera;
        render(&ctx, rect.size(), 1.0, 2, |ui| SceneView::new(scene, &mut camera).paint(ui.painter(), rect))
    };
    let (with, without) = (paint(&scene), paint(&Scene3D::default()));
    let corners: Vec<Pos2> = [(-0.8, -0.7), (0.8, -0.7), (0.8, 0.7), (-0.8, 0.7)]
        .iter()
        .filter_map(|&(x, y)| camera.project([x, y, 0.0], rect).map(|p| p.0))
        .collect();
    let around = Rect::from_points(&corners).expand(3.0);
    assert!(corners.len() == 4 && around.height() < 12.0, "{around:?}");
    let changed = (0..with.height)
        .flat_map(|y| (0..with.width).map(move |x| (x, y)))
        .filter(|&(x, y)| with.pixel(x, y) != without.pixel(x, y))
        .collect::<Vec<_>>();
    let stray = changed.iter().filter(|&&(x, y)| !around.contains(pos2(x as f32 + 0.5, y as f32 + 0.5))).count();
    assert!(changed.len() > 200 && stray == 0, "{} changed pixels, {stray} away from the board", changed.len());
}

/// The texts painted in one frame of `ui` (in lists of shapes too).
fn texts(mut ui: impl FnMut(&mut Ui)) -> usize {
    fn count(shape: &egui::Shape) -> usize {
        match shape {
            egui::Shape::Text(_) => 1,
            egui::Shape::Vec(list) => list.iter().map(count).sum(),
            _ => 0,
        }
    }
    let input =
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(960.0, 540.0))), ..RawInput::default() };
    let mut output = egui::Context::default().run_ui(input, &mut ui);
    output.textures_delta.clear();
    output.shapes.iter().map(|c| count(&c.shape)).sum()
}

#[test]
fn labels_off_writes_no_text() {
    // The frame's held stone, wrists and boxes; a render's boxes; the primitives' labels.
    let carry = overlay_of_frame(&frame(1516)).unwrap();
    let boxes: Overlay2D = reply("renderOverlay2d.json");
    let primitives: Overlay2D = reply("renderOverlay2d-primitives.json");
    let counts = [&carry, &boxes, &primitives].map(|overlay| {
        let on = texts(|ui| {
            OverlayView::new(overlay).show(ui);
        });
        let off = texts(|ui| {
            OverlayView::new(overlay).labels(false).show(ui);
        });
        (on, off)
    });
    // 643, L and R, and the two boxes' scores; the person and the stones; the two labels.
    assert_eq!(counts, [(5, 0), (2, 0), (2, 0)]);
}

/// The pixels of the focus colour (#ff4fc8) in a picture (the person box's #ed50e9 passes too).
fn pink(shot: &pdigger_factory_viz::raster::Snapshot) -> usize {
    pink_where(shot, |_| true)
}

/// The same, among the pixels whose centre passes `keep`.
fn pink_where(shot: &pdigger_factory_viz::raster::Snapshot, keep: impl Fn(Pos2) -> bool) -> usize {
    (0..shot.height)
        .flat_map(|y| (0..shot.width).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b, _] = shot.pixel(x, y);
            r > 200 && g < 140 && b > 150 && keep(pos2(x as f32 + 0.5, y as f32 + 0.5))
        })
        .count()
}

#[test]
fn the_focus_is_drawn() {
    let ctx = egui::Context::default();
    // In the overlay: a render's focus outline, else a frame's focus path (video px), else nothing.
    let overlay = |focus: serde_json::Value| {
        let overlay = Overlay2D { native_wh: vec![640, 360], focus: Some(focus), ..Default::default() };
        pink(&render(&ctx, vec2(640.0, 360.0), 1.0, 2, |ui| {
            OverlayView::new(&overlay).show(ui);
        }))
    };
    let only = overlay(serde_json::json!({"outline": [[100, 100], [300, 100], [200, 250]]}));
    let outline = overlay(
        serde_json::json!({"outline": [[100, 100], [300, 100], [200, 250]], "path2d": [[100, 300], [500, 300]]}),
    );
    let path = overlay(serde_json::json!({"outline": [[100, 100], [300, 100]], "path2d": [[100, 300], [500, 300]]}));
    let neither = overlay(serde_json::json!({"outline": [[100, 100], [300, 100]], "path2d": [[100, 300]]}));
    // An outline thinner than a quarter of a point is left out, and still wins over the path.
    let sliver = overlay(
        serde_json::json!({"outline": [[100, 100], [300, 100], [200, 100.1]], "path2d": [[100, 300], [500, 300]]}),
    );
    assert!(only > 1500 && outline == only && path > 800 && neither == 0 && sliver == 0, "{only} {outline} {path}");
    // In the scene: a frame's focus path in scene metres; points without three numbers are left out.
    let mut f = frame(1516);
    let focus = serde_json::json!({"id": 643, "path2d": [], "path3d": [[-0.5, 0.0, 0.0], [0.5, 0.0, 0.0], [0.5, 0.3]]});
    f.world.as_mut().unwrap().focus = Some(serde_json::from_value(focus).unwrap());
    let mut scene = scene_of_frame(&f).unwrap();
    assert_eq!(scene.focus.as_ref().unwrap()["path3d"][1], serde_json::json!([0.5, 0.0, 0.0]));
    scene.stones.clear();
    scene.skeleton = Default::default();
    let mut camera = OrbitCamera::top([0.0, 0.0, 0.0], 3.0);
    let mut shot = |scene: &Scene3D| {
        pink(&render(&ctx, vec2(960.0, 540.0), 1.0, 2, |ui| {
            SceneView::new(scene, &mut camera).paint(ui.painter(), ui.max_rect())
        }))
    };
    let drawn = shot(&scene);
    scene.focus = Some(serde_json::json!({"path3d": [[-0.5, 0.0, 0.0], [0.5, 0.0, 0.0]]}));
    let plain = shot(&scene);
    scene.focus = Some(serde_json::json!({"path3d": [[-0.5, 0.0, 0.0], [0.5, null, 0.2], "x", [0.5, 0.0, 0.0]]}));
    let odd = shot(&scene);
    scene.focus = None;
    assert!(drawn > 150 && drawn == plain && odd == plain && shot(&scene) == 0, "{drawn} {plain} {odd}");
}

#[test]
fn the_focus_path_breaks_behind_the_camera() {
    // Looking along +y from 1 m in front of the target, through two runs of the path and a point behind the eye
    // between them (and one before them): no line may join the runs across the gap.
    let camera = OrbitCamera {
        target: [0.0, 0.0, 0.0],
        yaw: -std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
        distance: 1.0,
        ..Default::default()
    };
    let path =
        [[0.0, -1.5, 0.1], [-0.4, 0.0, 0.1], [-0.15, 0.0, 0.1], [0.0, -1.5, 0.1], [0.15, 0.0, 0.1], [0.4, 0.0, 0.1]];
    let scene = Scene3D { focus: Some(serde_json::json!({ "path3d": path })), ..Default::default() };
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(960.0, 540.0));
    let shot = render(&egui::Context::default(), rect.size(), 1.0, 2, |ui| {
        let mut camera = camera;
        SceneView::new(&scene, &mut camera).paint(ui.painter(), rect)
    });
    let at = |p: [f32; 3]| camera.project(p, rect).map(|p| p.0);
    let (a, c) = (at([-0.15, 0.0, 0.1]).unwrap(), at([0.15, 0.0, 0.1]).unwrap());
    assert!(at([0.0, -1.5, 0.1]).is_none(), "the middle point is behind the camera");
    let row = |x0: f32, x1: f32| pink_where(&shot, |p| p.x > x0 && p.x < x1 && (p.y - a.y).abs() < 6.0);
    let (left, gap, right) = (row(0.0, a.x), row(a.x + 4.0, c.x - 4.0), row(c.x, 960.0));
    assert!(left > 100 && right > 100 && gap == 0, "{left} {gap} {right}");
}

#[test]
fn sharp_corners_in_the_overlay_draw_no_spikes() {
    // A stone's outline and the focus outline with a corner of about 7.6° at (400, 120). egui mitres the corners of
    // closed lines and filled polygons without a limit: before the corners were cut, both threw long thin spikes past
    // that corner.
    let sharp = vec![vec![100.0, 100.0], vec![400.0, 120.0], vec![100.0, 140.0]];
    let stone = pdigger_factory_sdk::models::DrawStone2D {
        id: Some(1),
        outline: sharp.clone(),
        style: "kept".into(),
        alpha: 1.0,
        ..Default::default()
    };
    let empty = Overlay2D { native_wh: vec![640, 360], ..Default::default() };
    let drawn =
        Overlay2D { stones: vec![stone], focus: Some(serde_json::json!({ "outline": sharp })), ..empty.clone() };
    let ctx = egui::Context::default();
    let shot = |overlay: &Overlay2D| {
        render(&ctx, vec2(640.0, 360.0), 1.0, 2, |ui| {
            OverlayView::new(overlay).show(ui);
        })
    };
    let (with, without) = (shot(&drawn), shot(&empty));
    let around = Rect::from_min_max(pos2(100.5, 100.5), pos2(400.5, 140.5)).expand(6.0);
    let changed = (0..with.height)
        .flat_map(|y| (0..with.width).map(move |x| (x, y)))
        .filter(|&(x, y)| with.pixel(x, y) != without.pixel(x, y))
        .collect::<Vec<_>>();
    let stray = changed.iter().filter(|&&(x, y)| !around.contains(pos2(x as f32 + 0.5, y as f32 + 0.5))).count();
    assert!(changed.len() > 1000 && stray == 0, "{} changed pixels, {stray} away from the outline", changed.len());
}

#[test]
fn the_scene_of_a_frame_and_its_camera_gestures() {
    let f = frame(1516);
    let scene = scene_of_frame(&f).unwrap();
    assert_eq!(scene.stones.len(), f.world.as_ref().unwrap().stones.iter().filter(|s| s.solid.is_some()).count());
    assert_eq!(scene.skeleton.joints.len(), 70);
    assert!(scene.board.is_some() && scene.plane.is_some());
    let mut only_scene = f.clone();
    only_scene.world = None;
    let bare = scene_of_frame(&only_scene).unwrap();
    assert!(bare.stones.is_empty() && bare.board.is_some(), "the scene layer's board");
    let mut camera = OrbitCamera::framing(&scene);
    let start = camera;
    let mut h = Harness::new(600.0, 400.0);
    let show = |ui: &mut Ui, camera: &mut OrbitCamera| {
        SceneView::new(&scene, camera).show(ui);
    };
    let (a, b) = (pos2(300.0, 200.0), pos2(360.0, 230.0));
    h.frame(vec![Event::PointerMoved(a)], |ui| show(ui, &mut camera));
    h.frame(
        vec![Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }],
        |ui| show(ui, &mut camera),
    );
    h.frame(vec![Event::PointerMoved(b)], |ui| show(ui, &mut camera));
    h.frame(
        vec![Event::PointerButton {
            pos: b,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }],
        |ui| show(ui, &mut camera),
    );
    assert_ne!(camera.yaw, start.yaw, "a drag orbits");
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, 120.0),
        phase: TouchPhase::Move,
        source: MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    };
    for _ in 0..3 {
        h.frame(vec![wheel.clone()], |ui| show(ui, &mut camera));
    }
    assert!(camera.distance < start.distance, "scrolling zooms in");
    for _ in 0..2 {
        h.frame(
            vec![Event::PointerButton {
                pos: a,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: Modifiers::NONE,
            }],
            |ui| show(ui, &mut camera),
        );
        h.frame(
            vec![Event::PointerButton {
                pos: a,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            }],
            |ui| show(ui, &mut camera),
        );
    }
    assert_eq!(camera, OrbitCamera::framing(&scene), "a double click frames the scene again");
    // From past the board looking away from it: what is behind the camera is not drawn.
    assert!(!scene.skeleton.rings.is_empty(), "the wrist rings of the scene layer");
    let board = scene.board.as_ref().unwrap();
    let mut away = OrbitCamera {
        target: [board.cx as f32 - 3.0, board.cy as f32, 0.0],
        yaw: 0.0,
        pitch: 0.3,
        distance: 1.0,
        ..OrbitCamera::default()
    };
    let centre = [board.cx as f32, board.cy as f32, 0.0];
    assert!(
        away.project(centre, Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))).is_none(),
        "the board is behind the camera"
    );
    h.frame(vec![], |ui| show(ui, &mut away));
}

#[test]
fn the_feed_and_the_hand_cards() {
    let updates: Vec<Update> = Replay::open(recording()).unwrap().updates().map(Result::unwrap).collect();
    let mut feed = EventFeed::new(10);
    let mut h = Harness::new(420.0, 300.0);
    h.frame(vec![], |ui| assert!(feed.show(ui).is_none()));
    updates.iter().for_each(|u| feed.push_update(u));
    assert_eq!(feed.len(), 10, "the newest ten lines");
    let all = EventFeed::default();
    assert!(all.is_empty());
    let lines: Vec<&FeedLine> = feed.lines().collect();
    assert!(lines[0].activity, "the activities closed at the end come last, so first");
    assert_eq!(lines[0].text, "5:02.8 Right hand pushes stones 628, 629, 637, 644 and 680 (7 pushes in 1.2 s)");
    let lift =
        updates.iter().flat_map(|u| &u.events).find(|e| e.describe() == "5:01.6 Right hand lifts stone 643").unwrap();
    assert_eq!(FeedLine::event(lift).stone, Some(643));
    // The two toggles sit at the right of the header: activities, then events.
    h.click(pos2(420.0 - 11.0, 11.0), |ui| {
        feed.show(ui);
    });
    assert!(!feed.show_activities && feed.lines().all(|l| !l.activity));
    h.click(pos2(420.0 - 39.0, 11.0), |ui| {
        feed.show(ui);
    });
    assert!(!feed.show_events && feed.lines().next().is_none());
    feed.show_events = true;
    feed.show_activities = true;
    let mut hovered = None;
    h.frame(vec![Event::PointerMoved(pos2(60.0, 45.0))], |ui| hovered = feed.show(ui));
    h.frame(vec![], |ui| hovered = feed.show(ui));
    assert!(hovered.is_some(), "the first line's stone");
    feed.clear();
    assert!(feed.is_empty());
    let state = updates.iter().find(|u| u.frame.as_ref().is_some_and(|f| f.g == 1516)).unwrap().state.clone();
    let cards = HandCards::new(&state);
    assert_eq!(
        cards.lines(&Side::Right),
        [Some("carry".to_string()), Some(state.hands.right.card.clone().unwrap()), Some("stone 643".to_string())]
    );
    assert_eq!(cards.lines(&Side::Both), [None, None, None]);
    h.frame(vec![], |ui| cards.show(ui));
    let empty = pdigger_factory_sdk::InterpreterState::default();
    h.frame(vec![], |ui| HandCards::new(&empty).show(ui));
}

#[test]
fn the_viewer_plays_and_scrubs_a_replay() {
    let mut app = ViewerApp::from_replay("dry1-0296-0306-full", Replay::open(recording()).unwrap()).unwrap();
    assert_eq!((app.len(), app.index(), app.is_empty()), (51, 0, false));
    assert!(app.feed().is_empty(), "the first frame has no event");
    app.seek(2);
    assert_eq!(app.feed().len(), 1, "the freeze of the third frame");
    app.seek_time(302.3);
    assert_eq!(app.current().unwrap().frame.g, 1516);
    assert_eq!(app.state().hands.right.stone, Some(643));
    assert!(app.feed().lines().any(|l| l.text == "5:01.6 Right hand lifts stone 643"));
    assert_eq!(app.highlight(), None, "no activity closed at this frame");
    app.seek(1000);
    assert_eq!(app.index(), 50);
    assert!(app.highlight().is_some(), "the last frame closes the bursts");
    let placed = (0..app.len()).find(|&i| {
        app.seek(i);
        app.current().unwrap().update.activities.iter().any(|a| matches!(a, Activity::Move(_)))
    });
    assert!(placed.is_some());
    assert_eq!((app.current().unwrap().frame.g, app.highlight()), (1518, Some(643)), "the carry closes at the place");
    app.seek_time(1.0e9);
    assert_eq!(app.index(), 50);
    app.set_speed(100.0);
    assert_eq!(app.speed(), 16.0);
    app.set_speed(1.0);
    app.seek(0);
    app.set_playing(true);
    app.advance(1.0);
    assert!(app.is_playing() && (4..=6).contains(&app.index()), "one second is five frames: {}", app.index());
    app.advance(100.0);
    assert!(!app.is_playing() && app.index() == 50, "it stops at the end");
    app.advance(1.0);
    app.cycle_speed();
    assert_eq!(app.speed(), 2.0);
    for _ in 0..4 {
        app.cycle_speed();
    }
    assert_eq!(app.speed(), 1.0);
    app.set_speed(0.3);
    app.cycle_speed();
    assert_eq!(app.speed(), 1.0, "an odd speed goes back to 1x");
    // The top bar: back, play, next and speed at fixed places.
    let mut h = Harness::new(1280.0, 720.0);
    h.frame(vec![], |ui| app.ui(ui));
    h.click(pos2(53.0, 20.0), |ui| app.ui(ui));
    assert!(app.is_playing(), "play at the end starts again from the first frame");
    h.click(pos2(53.0, 20.0), |ui| app.ui(ui));
    assert!(!app.is_playing());
    let at = app.index();
    h.click(pos2(85.0, 20.0), |ui| app.ui(ui));
    assert_eq!(app.index(), at + 1);
    h.click(pos2(21.0, 20.0), |ui| app.ui(ui));
    assert_eq!(app.index(), at);
    h.click(pos2(126.0, 20.0), |ui| app.ui(ui));
    assert_eq!(app.speed(), 2.0);
    h.click(pos2(154.0 + 1280.0 * 0.15, 20.0), |ui| app.ui(ui));
    assert!((20..=30).contains(&app.index()), "the middle of the scrub bar: {}", app.index());
    // Hovering a stone in the video view highlights it everywhere.
    app.seek_time(302.3);
    let overlay = app.current().unwrap().overlay.clone().unwrap();
    let held = overlay.stones.iter().find(|s| s.id == Some(643)).unwrap();
    let rect = Rect::from_min_max(pos2(14.0, 66.0), pos2(1280.0 * 0.58 - 10.0, 712.0));
    let centre = held.outline.iter().fold([0.0, 0.0], |a, p| [a[0] + p[0] / 4.0, a[1] + p[1] / 4.0]);
    let at = OverlayView::new(&overlay).transform(rect).to_screen(centre);
    h.frame(vec![Event::PointerMoved(at)], |ui| app.ui(ui));
    h.frame(vec![], |ui| app.ui(ui));
    assert_eq!(app.highlight(), Some(643));
    let shot = render(&h.ctx, vec2(640.0, 360.0), 1.0, 2, |ui| app.ui(ui));
    assert!(shot.count_not(shot.pixel(0, 0), 6) > 20_000);
}

fn small(g: i64, world: bool) -> Update {
    let mut frame: Frame = serde_json::from_value(
        serde_json::json!({"cursor": g, "session": 1, "video": "live", "g": g, "t": g as f64 * 0.2}),
    )
    .unwrap();
    if world {
        frame.world = Some(Default::default());
    }
    Update {
        frame: Some(frame),
        events: Vec::new(),
        activities: Vec::new(),
        tray_facts: Vec::new(),
        speckle: Vec::new(),
        is_revision: false,
        is_backfill: false,
        is_final: false,
        is_duplicate: false,
        state: Default::default(),
    }
}

#[test]
fn the_viewer_follows_live_updates() {
    let (tx, rx) = channel();
    let mut app = ViewerApp::live("live", rx);
    let mut h = Harness::new(900.0, 560.0);
    h.frame(vec![], |ui| app.ui(ui));
    assert!(app.is_empty() && app.state().g.is_none());
    tx.send(Ok(small(1, false))).unwrap();
    tx.send(Err(pdigger_factory_sdk::Error::new(pdigger_factory_sdk::ErrorCode::Decode, "a bad event"))).unwrap();
    for g in 2..3005 {
        tx.send(Ok(small(g, g % 2 == 0))).unwrap();
    }
    let mut last = small(3005, true);
    last.frame = None;
    tx.send(Ok(last)).unwrap();
    h.frame(vec![], |ui| app.ui(ui));
    assert_eq!(app.len(), ViewerApp::LIVE_KEEP);
    assert_eq!(app.current().unwrap().frame.g, 3004, "it follows the newest");
    assert_eq!(app.status(), Some("a bad event (decode)"));
    app.set_playing(true);
    assert!(!app.is_playing(), "a live stream does not play");
    h.click(pos2(21.0, 20.0), |ui| app.ui(ui));
    tx.send(Ok(small(3006, true))).unwrap();
    h.frame(vec![], |ui| app.ui(ui));
    assert_eq!(app.current().unwrap().frame.g, 3003, "paused: it stays");
    h.click(pos2(53.0, 20.0), |ui| app.ui(ui));
    h.frame(vec![], |ui| app.ui(ui));
    assert_eq!(app.current().unwrap().frame.g, 3006, "play follows again");
    drop(tx);
    h.frame(vec![], |ui| app.ui(ui));
    assert_eq!(app.status(), Some("a bad event (decode)"), "the first problem stays shown");
    let (tx, rx) = channel::<pdigger_factory_sdk::Result<Update>>();
    drop(tx);
    let mut ended = ViewerApp::live("x", rx);
    ended.poll_live();
    assert_eq!(ended.status(), Some("the stream ended"));
    let mut empty = ViewerApp::from_updates("none", Vec::new());
    h.frame(vec![], |ui| empty.ui(ui));
    let mut bare = ViewerApp::from_updates("bare", vec![small(1, false), small(2, false)]);
    h.frame(vec![], |ui| bare.ui(ui));
    assert!(bare.current().unwrap().overlay.is_none());
    let mut final_only = small(1, false);
    final_only.frame = None;
    assert!(ViewerApp::from_updates("final", vec![final_only]).is_empty());
    let mut tiny = Harness::new(100.0, 60.0);
    tiny.frame(vec![], |ui| app.ui(ui));
    let odd = Replay::from_bytes(b"{\"recording\":\"pfr1\"}\n{nope\n".to_vec()).unwrap();
    assert!(ViewerApp::from_replay("odd", odd).unwrap().is_empty(), "a line it does not understand is skipped");
    let broken = Replay::from_reader(Broken(b"{\"recording\":\"pfr1\"}\n".to_vec())).unwrap();
    assert!(ViewerApp::from_replay("bad", broken).is_err(), "a file that cannot be read to its end");
}

/// A reader that gives its bytes, then fails on every read.
struct Broken(Vec<u8>);

impl std::io::Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0.is_empty() {
            return Err(std::io::Error::other("the disk went away"));
        }
        let n = self.0.len().min(buf.len());
        buf[..n].copy_from_slice(&self.0[..n]);
        self.0.drain(..n);
        Ok(n)
    }
}

#[test]
fn a_live_viewer_keeps_the_final_update() {
    // a stream that ended (resume off) ends with a final update without a frame: the activities
    // still open and the last tray's leaving go to the newest frame, as in a replay
    let path = fixtures().join("recordings/dry1-0000-0130.ndjson.gz");
    let updates: Vec<Update> = Replay::open(&path).unwrap().updates().map(Result::unwrap).collect();
    let last = updates.last().unwrap().clone();
    assert!(last.is_final && last.frame.is_none() && !last.activities.is_empty() && !last.tray_facts.is_empty());
    let mut replayed = ViewerApp::from_updates("replay", updates.clone());
    replayed.seek(replayed.len() - 1);
    let (tx, rx) = channel();
    let mut app = ViewerApp::live("live", rx);
    for update in &updates[..updates.len() - 1] {
        tx.send(Ok(update.clone())).unwrap();
    }
    app.poll_live();
    assert!(!app.tray_marks().iter().any(|m| !m.arrived && m.tray == 2), "tray 2 is still on");
    tx.send(Ok(last.clone())).unwrap();
    app.poll_live();
    assert_eq!(app.len(), updates.len() - 1, "no step of its own");
    let newest = &app.current().unwrap().update;
    assert_eq!(newest.tray_facts, replayed.current().unwrap().update.tray_facts);
    assert_eq!(newest.activities, replayed.current().unwrap().update.activities);
    assert!(app.tray_marks().iter().any(|m| !m.arrived && m.tray == 2), "the last tray's leaving point");
    let shown = app.feed().lines().next().unwrap();
    assert!(shown.activity && shown.text == FeedLine::activity(&last.activities[0]).text, "in the feed at once");
    // a final update before any frame (a stream that ended at once): no step, its lines in the feed
    let (tx, rx) = channel();
    let mut empty = ViewerApp::live("live", rx);
    tx.send(Ok(last.clone())).unwrap();
    empty.poll_live();
    assert_eq!(empty.len(), 0);
    assert!(empty.feed().lines().any(|l| l.activity && l.text == FeedLine::activity(&last.activities[0]).text));
}

#[tokio::test(start_paused = true)]
async fn follow_feeds_a_viewer_from_a_client() {
    let fake = FakeFactory::from_recording(recording()).unwrap();
    let (rx, task) = follow(fake.client().interpreter());
    assert!(tokio::time::timeout(Duration::from_secs(5), task).await.is_err(), "an interpreter stream does not end");
    let mut app = ViewerApp::live("fake", rx);
    app.poll_live();
    assert_eq!(app.len(), 51);
    let (rx, task) = follow(fake.client().interpreter());
    drop(rx);
    task.await;
}

#[tokio::test(start_paused = true)]
async fn follow_until_ends_at_once_for_an_idle_unit() {
    // a unit that sends nothing for an hour (a fake without frames would answer the stream with a 404, which ends
    // it): the task sends nothing, so only `stop` ends it
    let fake = FakeFactory::from_recording(recording()).unwrap();
    fake.set_delay(|op, _| (op == "streamFrames").then_some(Duration::from_secs(3600)));
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (rx, task) = follow_until(fake.client().interpreter(), async move {
        let _ = stopped.await;
    });
    let task = tokio::spawn(task);
    tokio::time::sleep(Duration::from_secs(30)).await;
    assert!(!task.is_finished(), "an idle unit's stream goes on");
    drop(stop);
    tokio::time::timeout(Duration::from_secs(1), task).await.expect("it ends at once").unwrap();
    assert!(rx.try_recv().is_err(), "nothing was sent");
}

#[test]
fn the_viewer_opens_files_recording_urls_and_units() {
    let (title, app) = ViewerApp::open(&ViewerInput::File(recording())).unwrap();
    assert_eq!((title.as_str(), app.len()), ("dry1-0296-0306-full.ndjson.gz", 51));
    assert!(ViewerApp::open(&ViewerInput::File("..".into())).is_err(), "a folder is no recording");
    // A recording URL and a unit: the fake unit on 127.0.0.1, serving the recording both ways.
    let bytes = std::fs::read(recording()).unwrap();
    let fake = FakeFactory::from_recording(recording()).unwrap().with_file("/data/dry1.ndjson.gz", bytes, 4096);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(fake.serve()).unwrap();
    let (title, app) =
        ViewerApp::open(&ViewerInput::RecordingUrl(format!("{}/data/dry1.ndjson.gz", server.url()))).unwrap();
    assert_eq!((title.as_str(), app.len()), ("dry1.ndjson.gz", 51));
    assert!(ViewerApp::open(&ViewerInput::RecordingUrl(format!("{}/data/none.ndjson.gz", server.url()))).is_err());
    let (title, mut live) = ViewerApp::open(&ViewerInput::Server(server.url().to_string())).unwrap();
    assert_eq!(title, server.url());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while live.len() < 51 && std::time::Instant::now() < deadline {
        live.poll_live();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(live.len(), 51, "the unit's frames reach the viewer from its thread");
    assert!(live.current().is_some_and(|s| s.frame.overlay.is_some()), "it streams the overlay layer too");
    let layers: Vec<String> = fake
        .requests()
        .iter()
        .filter(|r| r.url.contains("/frames/stream"))
        .filter_map(|r| {
            r.url.split_once("layers=").map(|(_, rest)| rest.split('&').next().unwrap_or("").replace("%2C", ","))
        })
        .collect();
    assert!(
        !layers.is_empty() && layers.iter().all(|l| l == "world,cards,scene,overlay"),
        "a viewer following a unit streams every layer it draws: {layers:?}"
    );
    assert!(ViewerApp::open(&ViewerInput::Server("factory-orin.local".into())).is_err(), "no http:// scheme");
    drop(live);
    rt.block_on(server.shutdown());
}

#[test]
fn a_world_recording_takes_its_video_size_from_its_header() {
    // aug28 is 1088 x 1026 and this excerpt has no overlay layer: the header's source sizes the video space.
    let path = fixtures().join("recordings/aug28-1270-1300.ndjson.gz");
    let size = |app: &mut ViewerApp| {
        app.seek(app.len() - 1);
        app.current().and_then(|s| s.overlay.as_ref()).map(|o| o.native_wh.clone()).unwrap()
    };
    let (_, mut app) = ViewerApp::open(&ViewerInput::File(path.clone())).unwrap();
    assert_eq!(size(&mut app), [1088, 1026]);
    let updates = Replay::open(&path).unwrap().updates().collect::<pdigger_factory_sdk::Result<Vec<_>>>().unwrap();
    assert_eq!(size(&mut ViewerApp::from_updates("aug28", updates)), [1920, 1080], "without a header: the default");
}

#[test]
fn inputs() {
    assert_eq!(ViewerInput::parse("dry1.ndjson.gz"), ViewerInput::File("dry1.ndjson.gz".into()));
    assert_eq!(
        ViewerInput::parse("https://factory.pdigger.com/developers/data/dry1.full.ndjson.gz"),
        ViewerInput::RecordingUrl("https://factory.pdigger.com/developers/data/dry1.full.ndjson.gz".into())
    );
    assert_eq!(
        ViewerInput::parse("http://factory-orin.local:8120"),
        ViewerInput::Server("http://factory-orin.local:8120".into())
    );
    assert_eq!(ViewerInput::parse("http://x/a.ndjson"), ViewerInput::RecordingUrl("http://x/a.ndjson".into()));
}

/// The viewer jumps between tray points, each tray's arrival and leaving point: with
/// `next_tray_point` and `previous_tray_point`, and with the keys `]`, Page Down, `[` and Page Up.
#[test]
fn the_viewer_jumps_between_tray_points() {
    let path = fixtures().join("recordings/trays-synthetic.ndjson.gz");
    let mut app = ViewerApp::from_replay("trays", Replay::open(&path).unwrap()).unwrap();
    let points: Vec<(i64, bool, i64)> = app.tray_marks().iter().map(|m| (m.tray, m.arrived, m.at.g)).collect();
    // tray 4 never arrives; tray 7 leaves with the final update, at its leaving point (g 181, the last frame)
    let want = [(1, true, 4), (1, false, 26), (2, true, 32), (2, false, 67), (3, true, 73), (3, false, 98)];
    assert_eq!(points[..6], want);
    assert_eq!(
        points[6..],
        [(5, true, 120), (5, false, 136), (6, true, 145), (6, false, 166), (7, true, 175), (7, false, 181)]
    );
    app.seek(0);
    let first = app.next_tray_point().unwrap();
    assert_eq!((first.tray, first.arrived, first.at.t, app.current().unwrap().frame.g), (1, true, 0.8, 4));
    let key = |key, pressed| Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let mut h = Harness::new(1280.0, 720.0);
    let mut press = |k| {
        h.frame(vec![key(k, true)], |ui| app.ui(ui));
        h.frame(vec![key(k, false)], |ui| app.ui(ui));
        app.current().unwrap().frame.g
    };
    assert_eq!(press(egui::Key::CloseBracket), 26, "the leaving point of tray 1");
    assert_eq!(press(egui::Key::PageDown), 32, "tray 2 has arrived");
    assert_eq!(press(egui::Key::OpenBracket), 26);
    assert_eq!(press(egui::Key::PageUp), 4);
    app.seek(app.len() - 1);
    assert!(app.next_tray_point().is_none(), "none after the last");
    // the last frame is tray 7's leaving point itself: the point before it is tray 7's arrival
    assert_eq!(app.previous_tray_point().map(|m| (m.tray, m.arrived)), Some((7, true)));
    app.seek(0);
    assert!(app.previous_tray_point().is_none(), "none before the first");
    // a point whose frame the viewer does not hold is left out: here tray 1's leaving point (g 26)
    let updates = Replay::open(path).unwrap().updates().map(Result::unwrap);
    let held = ViewerApp::from_updates("trays", updates.filter(|u| u.frame.as_ref().is_none_or(|f| f.g != 26)));
    let points: Vec<(i64, bool)> = held.tray_marks().iter().map(|m| (m.tray, m.arrived)).collect();
    assert_eq!(points[..5], [(1, true), (2, true), (2, false), (3, true), (3, false)]);
    assert_eq!(points[5..], [(5, true), (5, false), (6, true), (6, false), (7, true), (7, false)]);
}

/// The trays that left, by number.
fn numbers(app: &ViewerApp) -> Vec<i64> {
    app.trays().iter().map(|t| t.n).collect()
}

/// The top bar's Reports button shows the trays that left and the report of the one picked, in
/// place of the four views; picking another tray in the list shows its report.
#[test]
fn the_viewer_shows_the_reports_of_the_trays_that_left() {
    let path = fixtures().join("recordings/trays-synthetic.ndjson.gz");
    let mut app = ViewerApp::from_replay("trays", Replay::open(&path).unwrap()).unwrap();
    // tray 4 never arrived: no report
    assert_eq!(numbers(&app), [1, 2, 3, 5, 6, 7]);
    assert!(app.trays().iter().all(|t| t.report().is_some()));
    assert_eq!((app.report_index(), app.report_view()), (None, None), "the four views");
    let mut h = Harness::new(1280.0, 720.0);
    let reports = pos2(1229.0, 28.0);
    // the button opens the report of the tray of the frame shown: the last that had arrived by it
    let at_g = |app: &mut ViewerApp, g: i64| {
        let i = (0..app.len()).find(|&i| {
            app.seek(i);
            app.current().unwrap().frame.g >= g
        });
        app.seek(i.unwrap());
    };
    at_g(&mut app, 80);
    h.frame(vec![], |ui| app.ui(ui));
    h.click(reports, |ui| app.ui(ui));
    assert_eq!(app.report_index(), Some(2), "tray 3 arrived at g 73");
    let view = app.report_view().unwrap();
    assert_eq!(view.report().tray, 3);
    let shot = render(&h.ctx, vec2(1280.0, 720.0), 1.0, 2, |ui| app.ui(ui));
    assert!(shot.count_not(shot.pixel(0, 0), 6) > 50_000);
    h.time += 1.0; // after the render's own frames
    // the list: a row per tray, under its caption
    let row = |i: usize| pos2(130.0, 102.0 + 62.0 * i as f32);
    h.click(row(0), |ui| app.ui(ui));
    assert_eq!(app.report_index(), Some(0));
    h.click(row(3), |ui| app.ui(ui));
    assert_eq!((app.report_index(), app.report_view().map(|v| v.report().tray)), (Some(3), Some(5)));
    // the button again: the four views
    h.click(reports, |ui| app.ui(ui));
    assert_eq!(app.report_index(), None);
    // before the first tray arrived: the first tray's report
    app.seek(0);
    h.click(reports, |ui| app.ui(ui));
    assert_eq!(app.report_index(), Some(0));
    // past the last tray: the last one; fewer trays: the last of those
    app.show_report(Some(99));
    assert_eq!(app.report_index(), Some(5));
    app.set_trays(Replay::open(&path).unwrap().trays().unwrap().into_iter().take(2).collect());
    assert_eq!((numbers(&app), app.report_index()), (vec![1, 2], Some(1)));
    // none: the panel says so
    app.set_trays(Vec::new());
    assert_eq!((app.report_index(), app.report_view()), (Some(0), None));
    h.frame(vec![], |ui| app.ui(ui));
    app.show_report(None);
    assert_eq!(app.report_index(), None);
    let mut empty = ViewerApp::from_updates("none", Vec::new());
    h.click(reports, |ui| empty.ui(ui));
    assert_eq!((empty.report_index(), empty.trays().len()), (Some(0), 0));
}

/// The report view of the viewer is the tray's, with the worlds of its arrival and leaving points
/// using the frames it holds for the scene and the account for the leaving stones.
#[test]
fn the_report_view_takes_its_worlds_from_the_frames_held() {
    let path = fixtures().join("recordings/dry1-0018-0153.ndjson.gz");
    let mut app = ViewerApp::from_replay("dry1", Replay::open(&path).unwrap()).unwrap();
    assert_eq!(numbers(&app), [1]);
    let tray = app.trays()[0].clone();
    let (arrived, leaving) = (tray.arrived.unwrap().g, tray.leaving.unwrap().g);
    let scene = |g: i64| {
        scene_of_frame(&Replay::open(&path).unwrap().frames().map(Result::unwrap).find(|f| f.g == g).unwrap()).unwrap()
    };
    app.show_report(Some(0));
    let view = app.report_view().unwrap();
    assert_eq!(view, ReportView::new(&tray, &scene(arrived), &scene(leaving)).unwrap());
    assert_eq!((view.marks().before.len(), view.marks().after.len()), (42, 44));
    let updates = Replay::open(&path).unwrap().updates().map(Result::unwrap);
    let mut held = ViewerApp::from_updates("dry1", updates.filter(|u| u.frame.as_ref().is_none_or(|f| f.g != arrived)));
    assert!(held.trays().is_empty(), "a viewer made of updates lists the trays it is given");
    held.set_trays(Replay::open(&path).unwrap().trays().unwrap());
    held.show_report(Some(0));
    assert_eq!(held.report_view().unwrap(), ReportView::new(&tray, &Scene3D::default(), &scene(leaving)).unwrap());
}

/// A recording URL lists the same trays as the file, its damaged lines and its gap line too; a unit
/// followed lists them as they leave.
#[test]
fn the_viewer_lists_the_trays_of_a_recording_url_and_of_a_unit() {
    let path = fixtures().join("recordings/dry1-0018-0153.ndjson.gz");
    let bytes = std::fs::read(&path).unwrap();
    let fake = FakeFactory::from_recording(&path).unwrap().with_file("/data/dry1.ndjson.gz", bytes, 4096);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(fake.serve()).unwrap();
    let (_, app) =
        ViewerApp::open(&ViewerInput::RecordingUrl(format!("{}/data/dry1.ndjson.gz", server.url()))).unwrap();
    let (_, file) = ViewerApp::open(&ViewerInput::File(path.clone())).unwrap();
    assert_eq!(app.trays(), file.trays(), "the clock line too: the same wall times");
    let wall = app.trays()[0].report().unwrap().arrived.wall.unwrap();
    assert!((wall - 1795315127.0).abs() < 0.05, "2026-11-22T02:38:47.0Z: {wall}");
    assert_eq!(app.len(), file.len());
    rt.block_on(server.shutdown());
    // damaged lines, and a clock line the model cannot read (its focus), are skipped as a replay skips them
    let unreadable = r#"{"sse": "clock", "data": {"session": 2, "revision": 1, "video": "v", "playing": true, "rate": 1.0, "anchor_media": 0.0, "anchor_wall": 100.0, "hold_until": 0.0, "changed_by": "viewer", "media_time": 0.0, "server_now": 100.0, "focus": {"n": "x"}}}"#;
    let mut bytes = std::fs::read(fixtures().join("damaged/lines.ndjson")).unwrap();
    bytes.extend_from_slice(format!("{unreadable}\n").as_bytes());
    let fake = FakeFactory::new().with_file("/data/lines.ndjson", bytes.clone(), 512);
    let server = rt.block_on(fake.serve()).unwrap();
    let (_, mut app) =
        ViewerApp::open(&ViewerInput::RecordingUrl(format!("{}/data/lines.ndjson", server.url()))).unwrap();
    let mut file = ViewerApp::from_replay("lines", Replay::from_bytes(bytes).unwrap()).unwrap();
    assert!(!app.is_empty() && app.len() == file.len());
    for i in 0..file.len() {
        app.seek(i);
        file.seek(i);
        assert_eq!(app.current().unwrap().update, file.current().unwrap().update, "step {i}");
    }
    assert_eq!(app.trays(), file.trays());
    rt.block_on(server.shutdown());
    // a gap line (`trays-presence-synthetic` has one): its dropped frames end tray 11's presence
    // question there, read from a URL as from the file (stone 1101 is not sure)
    let path = fixtures().join("recordings/trays-presence-synthetic.ndjson.gz");
    let fake = FakeFactory::new().with_file("/data/presence.ndjson.gz", std::fs::read(&path).unwrap(), 4096);
    let server = rt.block_on(fake.serve()).unwrap();
    let (_, mut app) =
        ViewerApp::open(&ViewerInput::RecordingUrl(format!("{}/data/presence.ndjson.gz", server.url()))).unwrap();
    let (_, mut file) = ViewerApp::open(&ViewerInput::File(path)).unwrap();
    assert_eq!(app.trays(), file.trays());
    let eleven = app.trays().iter().find(|t| t.n == 11).and_then(|t| t.report()).unwrap();
    assert_eq!(eleven.unsure, 1);
    assert_eq!(app.len(), file.len());
    for i in 0..file.len() {
        app.seek(i);
        file.seek(i);
        assert_eq!(app.current().unwrap().update, file.current().unwrap().update, "step {i}");
    }
    rt.block_on(server.shutdown());
    // a unit: its trays as they leave (tray 7, the last, leaves only when the frames end)
    let path = fixtures().join("recordings/trays-synthetic.ndjson.gz");
    let frames = Replay::open(&path).unwrap().frames().count();
    let fake = FakeFactory::from_recording(&path).unwrap();
    let server = rt.block_on(fake.serve()).unwrap();
    let (_, mut live) = ViewerApp::open(&ViewerInput::Server(server.url().to_string())).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while (live.len() < frames || live.trays().len() < 5) && std::time::Instant::now() < deadline {
        live.poll_live();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(numbers(&live), [1, 2, 3, 5, 6]);
    live.show_report(Some(2));
    assert_eq!(live.report_view().unwrap().report().tray, 3);
    drop(live);
    rt.block_on(server.shutdown());
}
