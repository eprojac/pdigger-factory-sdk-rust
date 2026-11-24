//! Draw lists from frames, and the colours of the 20.20 topology.
//!
//! [`crate::OverlayView`] paints an [`Overlay2D`] (`r2d1`, what `renderOverlay2d` returns) and
//! [`crate::SceneView`] a [`Scene3D`] (`renderScene3d`). A replay or a stream has frames instead:
//! [`overlay_of_frame`] and [`scene_of_frame`] build the same draw lists from a frame's world,
//! overlay and scene layers, so one painter serves both.

use std::sync::OnceLock;

use egui::Color32;
use pdigger_factory_sdk::WireEnum;
use pdigger_factory_sdk::codec::{Joint, joints, uv_to_px};
use pdigger_factory_sdk::models::{
    DrawStone2D, DrawStone3D, Frame, Overlay2D, OverlayBoxes, RenderMode, Scene3D, Side, Skeleton3D, StoneShape,
    StoneView, Topology, Wrist,
};
use serde_json::{Value, json};

/// The topology of the 20.20 unit (`getTopology`, schema `top1`): joints, bones, hand joints and
/// colours. Use the server's own when you have a client.
pub fn topology() -> &'static Topology {
    static TOPOLOGY: OnceLock<Topology> = OnceLock::new();
    TOPOLOGY.get_or_init(|| serde_json::from_str(include_str!("topology.json")).expect("the bundled topology is valid"))
}

/// A CSS colour: `#rgb`, `#rrggbb`, `#rrggbbaa`, `rgb(r,g,b)` or `rgba(r,g,b,a)` (a in 0..1).
pub fn parse_color(text: &str) -> Option<Color32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
        let [r, g, b, a] = match digits[..] {
            [r, g, b] => [r * 17, g * 17, b * 17, 255],
            [r1, r2, g1, g2, b1, b2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255],
            [r1, r2, g1, g2, b1, b2, a1, a2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, a1 * 16 + a2],
            _ => return None,
        };
        return Some(Color32::from_rgba_unmultiplied(r, g, b, a));
    }
    let inner = text.strip_prefix("rgba(").or_else(|| text.strip_prefix("rgb("))?.strip_suffix(')')?;
    let parts: Vec<f32> = inner.split(',').map(|p| p.trim().parse::<f32>().ok()).collect::<Option<_>>()?;
    let channel = |v: f32| v.clamp(0.0, 255.0).round() as u8;
    match parts[..] {
        [r, g, b] => Some(Color32::from_rgb(channel(r), channel(g), channel(b))),
        [r, g, b, a] => Some(Color32::from_rgba_unmultiplied(channel(r), channel(g), channel(b), channel(a * 255.0))),
        _ => None,
    }
}

/// A colour of the topology by name (`left`, `right`, `stones`, `skelBody`, `focus`, ...), grey
/// when it has none.
pub fn color(name: &str) -> Color32 {
    topology().colors.get(name).and_then(|c| parse_color(c)).unwrap_or(Color32::from_gray(160))
}

/// The colour of a hand: left and right from the topology, both between them, none grey.
pub fn hand_color(side: Option<&Side>) -> Color32 {
    match side {
        Some(Side::Left) => color("left"),
        Some(Side::Right) => color("right"),
        Some(Side::Both) => color("left").lerp_to_gamma(color("right"), 0.5),
        _ => Color32::from_gray(170),
    }
}

/// The colour of a stone of `style` (`held` takes its hand's).
pub fn stone_color(style: &str, side: Option<&Side>) -> Color32 {
    match style {
        "held" => hand_color(side),
        "placed" => Color32::from_rgb(0xff, 0xf3, 0xb0),
        "detected" | "raw" => color("stones"),
        "model" => color("stoneBox"),
        _ => color("stones").gamma_multiply(0.65),
    }
}

/// The `r2d1` style of a world stone: held by a hand, placed, detected this frame, a model
/// pose, or kept from earlier frames.
pub fn stone_style(stone: &StoneView) -> &'static str {
    if stone.side.is_some() {
        "held"
    } else if stone.placed.is_some() {
        "placed"
    } else if stone.detected {
        "detected"
    } else if stone.shape == StoneShape::Model {
        "model"
    } else {
        "kept"
    }
}

/// The hand a joint belongs to (the hand joints of the topology), `body` otherwise.
fn joint_side(j: usize) -> &'static str {
    let t = topology();
    // A finger is its tip and the three joints after it, towards the wrist.
    let hand = |wrist: usize, tips: &[usize]| j == wrist || tips.iter().any(|&tip| (tip..tip + 4).contains(&j));
    if hand(t.wrist.left, &t.tips.left) {
        "left"
    } else if hand(t.wrist.right, &t.tips.right) {
        "right"
    } else {
        "body"
    }
}

fn px(joint: &Joint, w: f64, h: f64) -> [f64; 2] {
    let (x, y) = uv_to_px(joint.u, joint.v, w, h);
    [x, y]
}

/// The `r2d1` skeleton (`bones` and `joints`, native px) of a frame's overlay joints.
fn skeleton_2d(values: &[Option<Vec<f64>>], w: f64, h: f64) -> Value {
    let found = joints(values);
    let bones: Vec<Value> = topology()
        .visible_bones()
        .into_iter()
        .filter_map(|[a, b]| {
            let (ja, jb) = (found.get(a)?.as_ref()?, found.get(b)?.as_ref()?);
            let side = if joint_side(a) != "body" { joint_side(a) } else { joint_side(b) };
            Some(json!({"a": a, "b": b, "from": px(ja, w, h), "to": px(jb, w, h), "side": side, "alpha": ja.vis.min(jb.vis), "hand": side != "body"}))
        })
        .collect();
    let points: Vec<Value> = found
        .iter()
        .enumerate()
        .filter_map(|(j, joint)| joint.as_ref().map(|joint| (j, joint)))
        .map(|(j, joint)| json!({"j": j, "at": px(joint, w, h), "side": joint_side(j), "alpha": joint.vis, "hand": joint_side(j) != "body"}))
        .collect();
    json!({"bones": bones, "joints": points})
}

/// The `r2d1` wrists of a frame's overlay.
fn wrists_2d(wrists: [(&Wrist, &str); 2], w: f64, h: f64) -> Vec<Value> {
    wrists
        .into_iter()
        .filter(|(wrist, _)| wrist.visible)
        .filter_map(|(wrist, side)| {
            let uv = wrist.uv.as_deref().filter(|uv| uv.len() >= 2)?;
            let (x, y) = uv_to_px(uv[0], uv[1], w, h);
            Some(json!({"side": side, "at": [x, y], "filled": wrist.filled, "ring": wrist.ring, "label": if side == "left" { "L" } else { "R" }}))
        })
        .collect()
}

/// The `r2d1` boxes of a frame's overlay (`[x0, y0, x1, y1, rot, presence]` in uv).
fn boxes_2d(boxes: &OverlayBoxes, w: f64, h: f64) -> Vec<Value> {
    [("person", &boxes.person, "person"), ("stone", &boxes.stone, "stones")]
        .into_iter()
        .filter_map(|(kind, b, word)| {
            let n = |i: usize| b.get(i).and_then(Value::as_f64);
            let (x0, y0, x1, y1, presence) = (n(0)?, n(1)?, n(2)?, n(3)?, n(5).unwrap_or(1.0));
            let (a, c) = (uv_to_px(x0, y0, w, h), uv_to_px(x1, y1, w, h));
            Some(json!({"kind": kind, "x0": a.0, "y0": a.1, "x1": c.0, "y1": c.1, "rot": b.get(4).and_then(Value::as_bool).unwrap_or(false), "presence": presence, "label": format!("{word} {presence:.2}")}))
        })
        .collect()
}

/// The 2-D draw list of a frame (stones of its world layer; skeleton, wrists and boxes of its
/// overlay layer), in the frame's native px; `None` without either layer. The video size is the
/// overlay layer's, else 1920 x 1080: [`overlay_of_frame_in`] takes the source's.
pub fn overlay_of_frame(frame: &Frame) -> Option<Overlay2D> {
    overlay_of_frame_in(frame, None)
}

/// [`overlay_of_frame`] for a source of this video size (`native_wh` of its catalog entry or of
/// its recording's header), used when the frame has no overlay layer (a `world` recording).
pub fn overlay_of_frame_in(frame: &Frame, native_wh: Option<&[i64]>) -> Option<Overlay2D> {
    if frame.world.is_none() && frame.overlay.is_none() {
        return None;
    }
    let native = frame
        .overlay
        .as_ref()
        .map(|o| o.native_wh.clone())
        .filter(|wh| wh.len() == 2)
        .or_else(|| native_wh.filter(|wh| wh.len() == 2).map(<[i64]>::to_vec))
        .unwrap_or_else(|| vec![1920, 1080]);
    let (w, h) = (native[0] as f64, native[1] as f64);
    let stones = frame
        .world
        .iter()
        .flat_map(|world| &world.stones)
        .map(|s| DrawStone2D {
            id: Some(s.id),
            outline: s.outline.clone(),
            style: stone_style(s).to_string(),
            side: s.side.clone(),
            alpha: if s.uncertain { 0.55 } else { 1.0 },
            detected: Some(s.detected),
            uncertain: Some(s.uncertain),
            shape: Some(s.shape.as_str().to_string()),
            mode: Some(s.mode.as_str().to_string()),
            ..DrawStone2D::default()
        })
        .collect();
    let overlay = frame.overlay.as_ref();
    Some(Overlay2D {
        schema: "r2d1".to_string(),
        session: frame.session,
        video: frame.video.clone(),
        g: frame.g,
        t: frame.t,
        mode: RenderMode::Stable,
        missing: frame.missing.iter().map(|l| l.as_str().to_string()).collect(),
        native_wh: native,
        stones,
        skeleton: overlay.map(|o| skeleton_2d(&o.joints, w, h)),
        wrists: overlay
            .map(|o| wrists_2d([(&o.wrists.left, "left"), (&o.wrists.right, "right")], w, h))
            .unwrap_or_default(),
        boxes: overlay.map(|o| boxes_2d(&o.boxes, w, h)).unwrap_or_default(),
        focus: frame.world.as_ref().and_then(|world| world.focus.as_ref()).and_then(|f| serde_json::to_value(f).ok()),
        ..Overlay2D::default()
    })
}

/// The 3-D draw list of a frame (stones with a solid of its world layer, and its focus; board, plane,
/// skeleton and wrist rings of its scene layer); `None` without either layer.
pub fn scene_of_frame(frame: &Frame) -> Option<Scene3D> {
    if frame.world.is_none() && frame.scene.is_none() {
        return None;
    }
    let world = frame.world.as_ref();
    let scene = frame.scene.as_ref();
    let stones = world
        .iter()
        .flat_map(|w| &w.stones)
        .filter_map(|s| {
            let solid = s.solid.as_ref().filter(|solid| solid.pos.len() == 3 && solid.dims.len() == 3)?;
            Some(DrawStone3D {
                id: Some(s.id),
                family: s.family.clone(),
                pos: solid.pos.clone(),
                q: solid.q.clone(),
                dims: solid.dims.clone(),
                side: s.side.clone(),
                alpha: if s.uncertain { 0.55 } else { 1.0 },
                uncertain: Some(s.uncertain),
                ..DrawStone3D::default()
            })
        })
        .collect();
    let board = world
        .and_then(|w| w.draw.as_ref())
        .and_then(|d| d.board.clone())
        .or_else(|| scene.and_then(|s| s.board.raw.clone()));
    Some(Scene3D {
        schema: "r3d1".to_string(),
        session: frame.session,
        video: frame.video.clone(),
        g: frame.g,
        t: frame.t,
        mode: RenderMode::Stable,
        plane: scene.and_then(|s| serde_json::to_value(&s.plane).ok()),
        board,
        stones,
        skeleton: Skeleton3D {
            joints: scene.map(|s| s.skeleton.world.clone()).unwrap_or_default(),
            rings: scene.map(|s| s.rings.clone()).unwrap_or_default(),
            ..Skeleton3D::default()
        },
        focus: world.and_then(|w| w.focus.as_ref()).and_then(|f| serde_json::to_value(f).ok()),
        ..Scene3D::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        assert_eq!(parse_color("#f68c36"), Some(Color32::from_rgb(0xf6, 0x8c, 0x36)));
        assert_eq!(parse_color("#fff"), Some(Color32::WHITE));
        assert_eq!(parse_color("#00000080"), Some(Color32::from_rgba_unmultiplied(0, 0, 0, 128)));
        assert_eq!(parse_color("rgba(0,0,0,.6)"), Some(Color32::from_rgba_unmultiplied(0, 0, 0, 153)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Color32::from_rgb(1, 2, 3)));
        for bad in ["#12", "#ggg", "rgb(1,2)", "red", "rgba(1,2,x,1)"] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
        assert_eq!(color("left"), hand_color(Some(&Side::Left)));
        assert_eq!(color("nope"), Color32::from_gray(160));
        assert_ne!(hand_color(Some(&Side::Both)), hand_color(None));
        assert_eq!(hand_color(Some(&Side::Right)), color("right"));
        for style in ["held", "placed", "detected", "model", "kept"] {
            assert_ne!(stone_color(style, Some(&Side::Left)).a(), 0);
        }
        assert_eq!(topology().joints, 70);
        assert_eq!(
            (joint_side(62), joint_side(45), joint_side(41), joint_side(25), joint_side(5)),
            ("left", "left", "right", "right", "body")
        );
    }

    #[test]
    fn frames_without_layers_have_no_draw_lists() {
        let frame = Frame::default();
        assert!(overlay_of_frame(&frame).is_none() && scene_of_frame(&frame).is_none());
    }
}
