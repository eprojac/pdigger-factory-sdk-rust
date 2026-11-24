//! [`OverlayView`]: an egui painter of the video-space draw list (`r2d1`, [`Overlay2D`]).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};
use pdigger_factory_sdk::models::{DrawStone2D, Overlay2D, Primitives2D, Side};
use serde_json::Value;

use crate::draw::{color, hand_color, parse_color, stone_color};
use crate::scene::blunt;

/// Native video px to screen points: the video fitted into a rectangle, centred.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoTransform {
    /// Points per native px.
    pub scale: f32,
    /// Where the video lands on screen.
    pub video: Rect,
}

impl VideoTransform {
    /// Fits a `native` (width, height) video into `rect`, keeping its aspect.
    pub fn fit(native: [f32; 2], rect: Rect) -> VideoTransform {
        let [w, h] = [native[0].max(1.0), native[1].max(1.0)];
        let scale = (rect.width() / w).min(rect.height() / h).max(0.0);
        let size = vec2(w * scale, h * scale);
        VideoTransform { scale, video: Rect::from_center_size(rect.center(), size) }
    }

    /// A native px position on screen.
    pub fn to_screen(&self, p: [f64; 2]) -> Pos2 {
        self.video.min + vec2(p[0] as f32 + 0.5, p[1] as f32 + 0.5) * self.scale
    }

    /// A screen position in native px.
    pub fn to_native(&self, p: Pos2) -> [f64; 2] {
        let v = (p - self.video.min) / self.scale.max(f32::EPSILON);
        [f64::from(v.x) - 0.5, f64::from(v.y) - 0.5]
    }

    /// A native px width on screen (at least a hairline).
    pub fn width(&self, native: f64) -> f32 {
        (native as f32 * self.scale).max(1.0)
    }
}

/// What [`OverlayView::show`] gives back.
#[derive(Debug)]
pub struct OverlayResponse {
    /// The area's response (hover, clicks).
    pub response: Response,
    /// The stone under the pointer.
    pub hovered: Option<i64>,
}

/// Paints an [`Overlay2D`] over its video rectangle: stones by style (held stones in their
/// hand's colour, uncertain ones faded), boxes, the skeleton, wrists, the focus, and the
/// `primitives` of `draw=primitives`. Hovering a stone highlights it and names it.
///
/// ```
/// # let ctx = egui::Context::default();
/// # ctx.run_ui(Default::default(), |ui| {
/// let overlay = pdigger_factory_sdk::models::Overlay2D { native_wh: vec![1920, 1080], ..Default::default() };
/// let shown = pdigger_factory_viz::OverlayView::new(&overlay).labels(true).show(ui);
/// assert!(shown.hovered.is_none());
/// # }).drop_without_applying_deltas();
/// ```
#[derive(Clone, Copy)]
pub struct OverlayView<'a> {
    overlay: &'a Overlay2D,
    highlight: Option<i64>,
    labels: bool,
    background: Color32,
}

impl<'a> OverlayView<'a> {
    /// A view of `overlay`.
    pub fn new(overlay: &'a Overlay2D) -> Self {
        OverlayView { overlay, highlight: None, labels: true, background: crate::craft::Tokens::default().viewport }
    }

    /// Highlights a stone (e.g. the one an activity is about).
    pub fn highlight(mut self, id: Option<i64>) -> Self {
        self.highlight = id;
        self
    }

    /// Writes the labels, on by default: the ids of held and highlighted stones, the wrists' L and R, the boxes'
    /// scores (`person 0.95`) and the primitives' labels. Off, the view writes nothing but the name of the stone under
    /// the pointer. (`labels` of the TypeScript `OverlayCanvas` switches the boxes' and the primitives' labels, the
    /// only ones it writes.)
    pub fn labels(mut self, on: bool) -> Self {
        self.labels = on;
        self
    }

    /// The colour around and under the video (the recordings carry no pixels).
    pub fn background(mut self, color: Color32) -> Self {
        self.background = color;
        self
    }

    /// The transform of this overlay into `rect`.
    pub fn transform(&self, rect: Rect) -> VideoTransform {
        let wh = |i: usize| self.overlay.native_wh.get(i).copied().unwrap_or(0) as f32;
        VideoTransform::fit([wh(0), wh(1)], rect)
    }

    /// Takes the space left in `ui` and paints into it.
    pub fn show(self, ui: &mut Ui) -> OverlayResponse {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let hovered = self.paint(&ui.painter_at(rect), rect, response.hover_pos());
        OverlayResponse { response, hovered }
    }

    /// Paints into `rect`; returns the stone under `pointer`.
    pub fn paint(&self, painter: &Painter, rect: Rect, pointer: Option<Pos2>) -> Option<i64> {
        let tf = self.transform(rect);
        painter.rect_filled(rect, 0.0, self.background);
        painter.rect_filled(tf.video, 0.0, self.background.gamma_multiply(1.18));
        self.paint_grid(painter, &tf);
        let o = self.overlay;
        for b in &o.boxes {
            paint_box(painter, &tf, b, self.labels);
        }
        // Held stones are painted last (they are lifted above the others) and picked first.
        let mut order: Vec<&DrawStone2D> = o.stones.iter().filter(|s| s.style != "held").collect();
        order.extend(o.stones.iter().filter(|s| s.style == "held"));
        let hovered = pointer.and_then(|p| {
            let at = tf.to_native(p);
            order.iter().rev().find(|s| contains(&s.outline, at)).and_then(|s| s.id)
        });
        for stone in order {
            let points: Vec<Pos2> =
                stone.outline.iter().filter(|p| p.len() >= 2).map(|p| tf.to_screen([p[0], p[1]])).collect();
            if points.len() < 2 {
                continue;
            }
            // Corners sharper than about 25° cut and slivers left out, as in SceneView: egui mitres the corners of
            // closed lines without a limit.
            let Some(points) = (if points.len() >= 3 { blunt(&points) } else { Some(points) }) else { continue };
            let base =
                stone_color(&stone.style, stone.side.as_ref()).gamma_multiply(stone.alpha.clamp(0.0, 1.0) as f32);
            let lit = stone.id.is_some() && (stone.id == hovered || stone.id == self.highlight);
            let stroke = Stroke::new(tf.width(if lit { 5.0 } else { 2.5 }), if lit { Color32::WHITE } else { base });
            if convex(&points) {
                painter.add(Shape::convex_polygon(
                    points.clone(),
                    base.gamma_multiply(if stone.style == "held" { 0.45 } else { 0.18 }),
                    Stroke::NONE,
                ));
            }
            if stone.uncertain == Some(true) {
                let mut ring = points.clone();
                ring.push(points[0]);
                painter.extend(Shape::dashed_line(&ring, stroke, 6.0, 4.0));
            } else {
                painter.add(Shape::closed_line(points.clone(), stroke));
            }
            if self.labels && (lit || stone.style == "held") {
                let centre = points.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / points.len() as f32;
                label(
                    painter,
                    centre.to_pos2(),
                    &stone.id.map_or_else(|| "?".to_string(), |id| id.to_string()),
                    Color32::WHITE,
                    Some(Color32::from_black_alpha(150)),
                    Align2::CENTER_CENTER,
                );
            }
        }
        if let Some(skeleton) = &o.skeleton {
            paint_skeleton(painter, &tf, skeleton);
        }
        for w in &o.wrists {
            paint_wrist(painter, &tf, w, self.labels);
        }
        if let Some(focus) = &o.focus {
            paint_focus(painter, &tf, focus);
        }
        if let Some(primitives) = &o.primitives {
            paint_primitives(painter, &tf, primitives, self.labels);
        }
        if let Some(id) = hovered {
            let at = pointer.unwrap_or(rect.center()) + vec2(12.0, 12.0);
            label(
                painter,
                at,
                &format!("stone {id}"),
                Color32::WHITE,
                Some(Color32::from_black_alpha(190)),
                Align2::LEFT_TOP,
            );
        }
        hovered
    }

    fn paint_grid(&self, painter: &Painter, tf: &VideoTransform) {
        let stroke = Stroke::new(1.0, Color32::from_white_alpha(10));
        for i in 1..8 {
            let x = tf.video.min.x + tf.video.width() * i as f32 / 8.0;
            painter.line_segment([pos2(x, tf.video.min.y), pos2(x, tf.video.max.y)], stroke);
            let y = tf.video.min.y + tf.video.height() * i as f32 / 8.0;
            painter.line_segment([pos2(tf.video.min.x, y), pos2(tf.video.max.x, y)], stroke);
        }
    }
}

/// A number of a JSON object.
fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

/// A `[x, y]` point of a JSON object.
fn point(v: &Value, key: &str) -> Option<[f64; 2]> {
    let p = v.get(key)?.as_array()?;
    Some([p.first()?.as_f64()?, p.get(1)?.as_f64()?])
}

/// A side of a JSON object (`left`, `right`, `both`, anything else: none).
fn side(v: &Value) -> Option<Side> {
    match v.get("side").and_then(Value::as_str) {
        Some("left") => Some(Side::Left),
        Some("right") => Some(Side::Right),
        Some("both") => Some(Side::Both),
        _ => None,
    }
}

fn paint_box(painter: &Painter, tf: &VideoTransform, b: &Value, labels: bool) {
    let (Some(x0), Some(y0), Some(x1), Some(y1)) = (num(b, "x0"), num(b, "y0"), num(b, "x1"), num(b, "y1")) else {
        return;
    };
    let rect = Rect::from_two_pos(tf.to_screen([x0, y0]), tf.to_screen([x1, y1]));
    let c =
        if b.get("kind").and_then(Value::as_str) == Some("person") { color("personBox") } else { color("stoneBox") };
    let ring = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    painter.extend(Shape::dashed_line(&ring, Stroke::new(tf.width(2.0), c.gamma_multiply(0.8)), 6.0, 4.0));
    if labels && let Some(text) = b.get("label").and_then(Value::as_str) {
        label(
            painter,
            rect.left_top() + vec2(4.0, 3.0),
            text,
            c,
            Some(Color32::from_black_alpha(150)),
            Align2::LEFT_TOP,
        );
    }
}

fn paint_skeleton(painter: &Painter, tf: &VideoTransform, skeleton: &Value) {
    let tone = |v: &Value| match v.get("side").and_then(Value::as_str) {
        Some("left") => color("skelLeft"),
        Some("right") => color("skelRight"),
        _ => color("skelBody"),
    };
    for bone in skeleton.get("bones").and_then(Value::as_array).into_iter().flatten() {
        let (Some(a), Some(b)) = (point(bone, "from"), point(bone, "to")) else { continue };
        let alpha = num(bone, "alpha").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
        painter.line_segment(
            [tf.to_screen(a), tf.to_screen(b)],
            Stroke::new(tf.width(3.0), tone(bone).gamma_multiply(alpha)),
        );
    }
    for joint in skeleton.get("joints").and_then(Value::as_array).into_iter().flatten() {
        let Some(at) = point(joint, "at") else { continue };
        let alpha = num(joint, "alpha").unwrap_or(1.0).clamp(0.0, 1.0) as f32;
        painter.circle_filled(tf.to_screen(at), tf.width(3.5), tone(joint).gamma_multiply(alpha));
    }
}

fn paint_wrist(painter: &Painter, tf: &VideoTransform, w: &Value, labels: bool) {
    let Some(at) = point(w, "at") else { return };
    let c = hand_color(side(w).as_ref());
    let centre = tf.to_screen(at);
    let r = tf.width(18.0).max(5.0);
    if w.get("filled").and_then(Value::as_bool) == Some(true) {
        painter.circle_filled(centre, r, c.gamma_multiply(0.35));
    }
    painter.circle_stroke(
        centre,
        r,
        Stroke::new(tf.width(if w.get("ring").and_then(Value::as_bool) == Some(true) { 4.0 } else { 2.0 }), c),
    );
    if labels && let Some(text) = w.get("label").and_then(Value::as_str) {
        label(painter, centre + vec2(0.0, -r - 2.0), text, c, None, Align2::CENTER_BOTTOM);
    }
}

/// The focus: the outline of a render's focus at twice the width of a stone's line, or else a frame's focus path
/// (`path2d`, video px) at one and a half times. (The TypeScript `OverlayCanvas` draws the same two at 2 and 1.5 times
/// its base line `lw`, and its stones at 1.2 `lw`.)
fn paint_focus(painter: &Painter, tf: &VideoTransform, focus: &Value) {
    let points = |key: &str| -> Vec<Pos2> {
        let list = focus.get(key).and_then(Value::as_array).into_iter().flatten();
        list.filter_map(|p| Some(tf.to_screen([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?]))).collect()
    };
    let (outline, path) = (points("outline"), points("path2d"));
    if outline.len() >= 3 {
        if let Some(outline) = blunt(&outline) {
            painter.add(Shape::closed_line(outline, Stroke::new(tf.width(5.0), color("focus"))));
        }
    } else if path.len() >= 2 {
        painter.add(Shape::line(path, Stroke::new(tf.width(3.75), color("focus"))));
    }
}

fn paint_primitives(painter: &Painter, tf: &VideoTransform, p: &Primitives2D, labels: bool) {
    let lw = p.lw.unwrap_or(2.0);
    let col =
        |c: Option<&str>, alpha: f64| c.and_then(parse_color).map(|c| c.gamma_multiply(alpha.clamp(0.0, 1.0) as f32));
    for image in &p.images {
        let rect =
            Rect::from_min_size(tf.to_screen([image.x, image.y]), vec2(image.w as f32, image.h as f32) * tf.scale);
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, Color32::from_white_alpha(24)), StrokeKind::Inside);
    }
    for poly in &p.polygons {
        let points: Vec<Pos2> =
            poly.points.iter().filter(|q| q.len() >= 2).map(|q| tf.to_screen([q[0], q[1]])).collect();
        if points.len() < 2 {
            continue;
        }
        let Some(points) = (if points.len() >= 3 { blunt(&points) } else { Some(points) }) else { continue };
        if let Some(fill) = col(poly.fill.as_deref(), poly.fill_alpha * poly.alpha).filter(|_| convex(&points)) {
            painter.add(Shape::convex_polygon(points.clone(), fill, Stroke::NONE));
        }
        let Some(c) = col(poly.stroke.as_deref(), poly.alpha) else { continue };
        let stroke = Stroke::new(tf.width(if poly.width > 0.0 { poly.width } else { lw }), c);
        match poly.dash.as_deref() {
            Some([on, off, ..]) => {
                let mut ring = points.clone();
                ring.push(points[0]);
                painter.extend(Shape::dashed_line(
                    &ring,
                    stroke,
                    (*on as f32 * tf.scale).max(1.0),
                    (*off as f32 * tf.scale).max(1.0),
                ));
            }
            _ => {
                painter.add(Shape::closed_line(points, stroke));
            }
        }
    }
    for s in &p.segments {
        let (Some(a), Some(b)) = (s.from.get(..2), s.to.get(..2)) else { continue };
        let c = col(Some(&s.color), s.alpha).unwrap_or(Color32::WHITE);
        painter.line_segment(
            [tf.to_screen([a[0], a[1]]), tf.to_screen([b[0], b[1]])],
            Stroke::new(tf.width(if s.width > 0.0 { s.width } else { lw }), c),
        );
    }
    for c in &p.circles {
        let Some(at) = c.at.get(..2) else { continue };
        let centre = tf.to_screen([at[0], at[1]]);
        let r = (c.r as f32 * tf.scale).max(1.5);
        if let Some(fill) = col(c.fill.as_deref(), c.alpha) {
            painter.circle_filled(centre, r, fill);
        }
        if let Some(stroke) = col(c.stroke.as_deref(), c.alpha) {
            painter.circle_stroke(centre, r, Stroke::new(tf.width(lw), stroke));
        }
    }
    for l in p.labels.iter().filter(|_| labels) {
        let Some(at) = l.at.get(..2) else { continue };
        let align = if l.baseline.as_deref() == Some("top") { Align2::LEFT_TOP } else { Align2::LEFT_BOTTOM };
        let c = parse_color(&l.color).unwrap_or(Color32::WHITE);
        label(painter, tf.to_screen([at[0], at[1]]), &l.text, c, l.background.as_deref().and_then(parse_color), align);
    }
}

/// Text with an optional background plate.
pub(crate) fn label(
    painter: &Painter,
    at: Pos2,
    text: &str,
    color: Color32,
    background: Option<Color32>,
    align: Align2,
) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.0), color);
    let rect = align.anchor_size(at, galley.size());
    if let Some(bg) = background {
        painter.rect_filled(rect.expand(2.0), 3.0, bg);
    }
    painter.galley(rect.min, galley, color);
}

/// Whether a closed polygon is convex (either winding).
pub fn convex(points: &[Pos2]) -> bool {
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0.0f32;
    for i in 0..n {
        let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
        let cross = (b - a).x * (c - b).y - (b - a).y * (c - b).x;
        if cross.abs() > 1e-6 {
            if sign != 0.0 && cross.signum() != sign {
                return false;
            }
            sign = cross.signum();
        }
    }
    sign != 0.0
}

/// Whether `at` is inside the outline (native px; ray casting).
pub fn contains(outline: &[Vec<f64>], at: [f64; 2]) -> bool {
    let points: Vec<[f64; 2]> = outline.iter().filter(|p| p.len() >= 2).map(|p| [p[0], p[1]]).collect();
    let mut inside = false;
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + points.len() - 1) % points.len()];
        if (a[1] > at[1]) != (b[1] > at[1]) && at[0] < (b[0] - a[0]) * (at[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transforms_round_trip() {
        let tf = VideoTransform::fit([1920.0, 1080.0], Rect::from_min_size(Pos2::ZERO, vec2(960.0, 1000.0)));
        assert_eq!(tf.scale, 0.5);
        assert_eq!(tf.video.height(), 540.0);
        let p = tf.to_screen([100.0, 50.0]);
        let back = tf.to_native(p);
        assert!((back[0] - 100.0).abs() < 1e-3 && (back[1] - 50.0).abs() < 1e-3);
        assert_eq!(tf.width(0.1), 1.0);
    }

    #[test]
    fn polygons() {
        let square = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)];
        assert!(convex(&square));
        assert!(!convex(&[pos2(0.0, 0.0), pos2(2.0, 0.0), pos2(1.0, 0.3), pos2(2.0, 2.0), pos2(0.0, 2.0)]));
        assert!(!convex(&square[..2]));
        assert!(!convex(&[pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(2.0, 0.0)]), "a line has no inside");
        let outline = vec![vec![0.0, 0.0], vec![10.0, 0.0], vec![10.0, 10.0], vec![0.0, 10.0]];
        assert!(contains(&outline, [5.0, 5.0]));
        assert!(!contains(&outline, [15.0, 5.0]));
        assert!(!contains(&[], [0.0, 0.0]));
        let sides = ["left", "right", "both", "up"].map(|name| side(&serde_json::json!({ "side": name })));
        assert_eq!(sides, [Some(Side::Left), Some(Side::Right), Some(Side::Both), None]);
        assert_eq!(side(&serde_json::json!({})), None);
    }
}
