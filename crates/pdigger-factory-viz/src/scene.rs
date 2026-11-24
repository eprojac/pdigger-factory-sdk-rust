//! [`SceneView`]: the 3-D draw list ([`Scene3D`]) seen through an orbit camera and painted with
//! egui's painter (no GPU needed): the board, extruded stone outlines, the focus path, the skeleton, wrist rings.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2};
use pdigger_factory_sdk::models::{DrawStone3D, Scene3D, StoneFamily};
use serde_json::Value;

use crate::draw::{color, hand_color, topology};

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    if n > 0.0 { scale(a, 1.0 / n) } else { a }
}

fn v3(p: &[f64]) -> Option<V3> {
    match p {
        [x, y, z, ..] => Some([*x as f32, *y as f32, *z as f32]),
        _ => None,
    }
}

/// Rotates `v` by the unit quaternion `[x, y, z, w]`.
pub fn rotate(q: [f32; 4], v: V3) -> V3 {
    let u = [q[0], q[1], q[2]];
    let t = scale(cross(u, v), 2.0);
    add(add(v, scale(t, q[3])), cross(u, t))
}

/// A quaternion `[x, y, z, w]` of four numbers; the identity otherwise.
pub(crate) fn quat(q: &[f64]) -> [f32; 4] {
    match q {
        [x, y, z, w] => [*x as f32, *y as f32, *z as f32, *w as f32],
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

/// The two rings of the stone's footprint in scene coordinates, in matching vertex order.
pub(crate) struct StonePrism {
    bottom: Vec<V3>,
    pub(crate) top: Vec<V3>,
}

impl StonePrism {
    pub(crate) fn new(stone: &DrawStone3D) -> Option<Self> {
        let pos = v3(&stone.pos)?;
        let [length, width, height] = v3(&stone.dims)?;
        let q = quat(&stone.q);
        let half_height = rotate(q, [0.0, 0.0, height * 0.5]);
        // (x along the length, half-width) in the unit footprint. The dogbone's straight ends
        // span 19.5% of its length, its shoulders 10.5%, and each notch cuts in 13.5% of its width.
        let sections: &[(f32, f32)] = match stone.family {
            StoneFamily::Dogbone => {
                &[(-0.5, 0.5), (-0.305, 0.5), (-0.2, 0.365), (0.2, 0.365), (0.305, 0.5), (0.5, 0.5)]
            }
            _ => &[(-0.5, 0.5), (0.5, 0.5)],
        };
        let outline = sections.iter().map(|&(x, y)| (x, -y)).chain(sections.iter().rev().copied());
        let (bottom, top) = outline
            .map(|(x, y)| {
                let point = add(pos, rotate(q, [x * length, y * width, 0.0]));
                (sub(point, half_height), add(point, half_height))
            })
            .unzip();
        Some(Self { bottom, top })
    }

    pub(crate) fn vertices(self) -> impl Iterator<Item = V3> {
        self.bottom.into_iter().chain(self.top)
    }

    pub(crate) fn top_centre(&self) -> V3 {
        scale(add(self.top[0], self.top[self.top.len() / 2]), 0.5)
    }
}

/// An orbit camera around a target point, scene axes (z up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OrbitCamera {
    /// The point looked at.
    pub target: V3,
    /// Angle about z, radians (0: looking from +x).
    pub yaw: f32,
    /// Elevation, radians (π/2: from above).
    pub pitch: f32,
    /// Distance from the target, m.
    pub distance: f32,
    /// Vertical field of view, radians.
    pub fov_y: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        OrbitCamera { target: [0.0, 0.0, 0.0], yaw: -2.2, pitch: 0.75, distance: 2.6, fov_y: 0.8 }
    }
}

/// Points nearer than this to the camera are not drawn.
const NEAR: f32 = 0.05;

impl OrbitCamera {
    /// Looking straight down on `target`.
    pub fn top(target: V3, distance: f32) -> OrbitCamera {
        OrbitCamera { target, yaw: -std::f32::consts::FRAC_PI_2, pitch: 1.55, distance, ..OrbitCamera::default() }
    }

    /// The default view framing a scene: its board (or its stones) in the middle.
    pub fn framing(scene: &Scene3D) -> OrbitCamera {
        let target = match &scene.board {
            Some(b) => [b.cx as f32, b.cy as f32, 0.0],
            None => {
                let centres: Vec<V3> = scene.stones.iter().filter_map(|s| v3(&s.pos)).collect();
                let sum = centres.iter().fold([0.0; 3], |acc, c| add(acc, *c));
                if centres.is_empty() { [0.0; 3] } else { scale(sum, 1.0 / centres.len() as f32) }
            }
        };
        // A little above the board and far enough back for the arms over it.
        let size = scene.board.as_ref().map_or(1.0, |b| b.hw.max(b.hh) as f32);
        let target = add(target, [0.0, 0.0, 0.1 * size]);
        OrbitCamera { target, distance: (size * 3.3).clamp(0.5, 20.0), ..OrbitCamera::default() }
    }

    /// The camera's position.
    pub fn eye(&self) -> V3 {
        let (c, s) = (self.pitch.cos(), self.pitch.sin());
        add(self.target, scale([c * self.yaw.cos(), c * self.yaw.sin(), s], self.distance))
    }

    /// Right, up and forward unit vectors.
    fn basis(&self) -> (V3, V3, V3) {
        let forward = normalize(sub(self.target, self.eye()));
        let right = normalize(cross(forward, [0.0, 0.0, 1.0]));
        (right, cross(right, forward), forward)
    }

    /// A scene point on screen in `rect`, with its depth; `None` behind the camera.
    pub fn project(&self, p: V3, rect: Rect) -> Option<(Pos2, f32)> {
        let (right, up, forward) = self.basis();
        let d = sub(p, self.eye());
        let z = dot(d, forward);
        if z < NEAR {
            return None;
        }
        let focal = rect.height() * 0.5 / (self.fov_y * 0.5).tan();
        Some((rect.center() + Vec2::new(dot(d, right), -dot(d, up)) * (focal / z), z))
    }

    /// Turns the camera by a pointer drag of `delta` points.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.5, 1.55);
    }

    /// Moves the camera nearer (positive `scroll`) or further.
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (-scroll * 0.002).exp()).clamp(0.2, 50.0);
    }
}

/// Paints a [`Scene3D`]; drag to orbit, scroll to zoom, double-click to frame the scene again.
///
/// ```
/// # let ctx = egui::Context::default();
/// # ctx.run_ui(Default::default(), |ui| {
/// let scene = pdigger_factory_sdk::models::Scene3D::default();
/// let mut camera = pdigger_factory_viz::OrbitCamera::framing(&scene);
/// pdigger_factory_viz::SceneView::new(&scene, &mut camera).show(ui);
/// # }).drop_without_applying_deltas();
/// ```
pub struct SceneView<'a> {
    scene: &'a Scene3D,
    camera: &'a mut OrbitCamera,
    highlight: Option<i64>,
    tint: Option<Tint<'a>>,
}

/// The colours given to [`SceneView::tint`].
type Tint<'a> = &'a dyn Fn(&DrawStone3D) -> Option<Color32>;

/// One shaded face of a stone, for back-to-front painting.
struct Face {
    depth: f32,
    shape: Shape,
}

impl Face {
    fn project(vertices: &[V3], camera: &OrbitCamera, rect: Rect, base: Color32, stroke: Stroke) -> Option<Self> {
        let first = vertices[0];
        let normal = normalize(cross(sub(vertices[1], first), sub(vertices[vertices.len() - 1], first)));
        if dot(normal, sub(camera.eye(), first)) <= 0.0 {
            return None;
        }
        let mut points = Vec::with_capacity(vertices.len());
        let mut depth = 0.0;
        for &vertex in vertices {
            let (point, z) = camera.project(vertex, rect)?;
            points.push(point);
            depth += z;
        }
        let perimeter = blunt(&points)?;
        let shade = 0.55 + 0.45 * dot(normal, normalize([0.3, -0.5, 1.0])).abs();
        let fill = Color32::from_rgba_premultiplied(
            (f32::from(base.r()) * shade) as u8,
            (f32::from(base.g()) * shade) as u8,
            (f32::from(base.b()) * shade) as u8,
            base.a(),
        );
        let shape = if points.len() == 4 {
            Shape::convex_polygon(perimeter, fill, stroke)
        } else {
            // Join opposite sides of the dogbone at each cross-section. Only the perimeter gets a stroke.
            let n = points.len() as u32;
            let mut mesh = Mesh::default();
            for p in points {
                mesh.colored_vertex(p, fill);
            }
            for i in 0..n / 2 - 1 {
                let opposite = n - 1 - i;
                mesh.add_triangle(i, i + 1, opposite);
                mesh.add_triangle(i + 1, opposite - 1, opposite);
            }
            Shape::Vec(vec![Shape::mesh(mesh), Shape::closed_line(perimeter, stroke)])
        };
        Some(Self { depth: depth / vertices.len() as f32, shape })
    }
}

impl<'a> SceneView<'a> {
    /// A view of `scene` through `camera`.
    pub fn new(scene: &'a Scene3D, camera: &'a mut OrbitCamera) -> Self {
        SceneView { scene, camera, highlight: None, tint: None }
    }

    /// Outlines a stone (e.g. the one an activity is about).
    pub fn highlight(mut self, id: Option<i64>) -> Self {
        self.highlight = id;
        self
    }

    /// Colours the stones: `tint` gives a stone's colour as it is (its alpha included: the stone's own `alpha` is not
    /// applied), or `None` for the usual one (its hand's, or the stones' colour, faded by its `alpha`).
    /// [`ReportView`](crate::ReportView) colours each stone by what a tray's report says of it.
    pub fn tint(mut self, tint: &'a dyn Fn(&DrawStone3D) -> Option<Color32>) -> Self {
        self.tint = Some(tint);
        self
    }

    /// Takes the space left in `ui`, handles the camera gestures and paints.
    pub fn show(self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        if response.dragged() {
            self.camera.orbit(response.drag_delta());
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
        }
        if response.double_clicked() {
            *self.camera = OrbitCamera::framing(self.scene);
        }
        self.paint(&ui.painter_at(rect), rect);
        response
    }

    /// Paints into `rect`.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let cam = *self.camera;
        let t = crate::craft::Tokens::default();
        painter.rect_filled(rect, 0.0, t.viewport);
        let line = |a: V3, b: V3, stroke: Stroke| {
            if let (Some((p, _)), Some((q, _))) = (cam.project(a, rect), cam.project(b, rect)) {
                painter.line_segment([p, q], stroke);
            }
        };
        // The floor grid, 25 cm, around the target.
        let grid = Stroke::new(1.0, Color32::from_white_alpha(14));
        let [cx, cy, _] = cam.target;
        for i in -8..=8 {
            let k = i as f32 * 0.25;
            line([cx + k, cy - 2.0, 0.0], [cx + k, cy + 2.0, 0.0], grid);
            line([cx - 2.0, cy + k, 0.0], [cx + 2.0, cy + k, 0.0], grid);
        }
        if let Some(b) = &self.scene.board {
            let (c, s) = ((b.theta as f32).cos(), (b.theta as f32).sin());
            let corner = |x: f32, y: f32| [b.cx as f32 + x * c - y * s, b.cy as f32 + x * s + y * c, 0.0];
            let (hw, hh) = (b.hw as f32, b.hh as f32);
            let ring: Vec<Pos2> = [corner(-hw, -hh), corner(hw, -hh), corner(hw, hh), corner(-hw, hh)]
                .iter()
                .filter_map(|&p| cam.project(p, rect).map(|(p, _)| p))
                .collect();
            if ring.len() == 4
                && let Some(ring) = blunt(&ring)
            {
                painter.add(Shape::convex_polygon(ring.clone(), Color32::from_white_alpha(10), Stroke::NONE));
                painter.add(Shape::closed_line(ring, Stroke::new(1.5, t.viewport_text.gamma_multiply(0.6))));
            }
        }
        let mut faces = Vec::new();
        for stone in &self.scene.stones {
            self.stone_faces(stone, rect, &mut faces);
        }
        faces.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        for face in faces {
            painter.add(face.shape);
        }
        // The focused stone's path (`focus.path3d`), as the TypeScript `SceneView` draws a frame's: points without
        // three numbers are left out, and the line breaks at a point behind the camera.
        let stroke = Stroke::new(2.0, color("focus"));
        let path3d = self.scene.focus.as_ref().and_then(|f| f.get("path3d")).and_then(Value::as_array);
        let mut run: Vec<Pos2> = Vec::new();
        for p in path3d.into_iter().flatten() {
            let n = |i: usize| p.get(i).and_then(Value::as_f64).map(|v| v as f32);
            let (Some(x), Some(y), Some(z)) = (n(0), n(1), n(2)) else { continue };
            match cam.project([x, y, z], rect) {
                Some((at, _)) => run.push(at),
                None => {
                    if run.len() >= 2 {
                        painter.add(Shape::line(std::mem::take(&mut run), stroke));
                    }
                    run.clear();
                }
            }
        }
        if run.len() >= 2 {
            painter.add(Shape::line(run, stroke));
        }
        self.paint_skeleton(painter, rect);
        for ring in &self.scene.skeleton.rings {
            if let Some((p, z)) = v3(&ring.pos).and_then(|p| cam.project(p, rect)) {
                let r = (0.06 * rect.height() / z).clamp(3.0, 40.0);
                painter.circle_stroke(p, r, Stroke::new(2.5, hand_color(Some(&ring.side))));
            }
        }
    }

    fn stone_faces(&self, stone: &DrawStone3D, rect: Rect, out: &mut Vec<Face>) {
        let Some(StonePrism { mut bottom, top }) = StonePrism::new(stone) else { return };
        let lit = stone.id.is_some() && stone.id == self.highlight;
        let base = match self.tint.and_then(|tint| tint(stone)) {
            Some(tinted) => tinted,
            None => match &stone.side {
                Some(side) => hand_color(Some(side)),
                None => color("stones").lerp_to_gamma(Color32::from_gray(150), 0.35),
            }
            .gamma_multiply(stone.alpha.clamp(0.1, 1.0) as f32),
        };
        let stroke =
            if lit { Stroke::new(2.0, Color32::WHITE) } else { Stroke::new(1.0, Color32::from_black_alpha(90)) };
        let mut face = |vertices: &[V3]| out.extend(Face::project(vertices, self.camera, rect, base, stroke));
        for i in 0..top.len() {
            let j = (i + 1) % top.len();
            face(&[bottom[i], bottom[j], top[j], top[i]]);
        }
        face(&top);
        bottom.reverse(); // outward winding for the underside
        face(&bottom);
    }

    fn paint_skeleton(&self, painter: &Painter, rect: Rect) {
        let cam = *self.camera;
        let joints = &self.scene.skeleton.joints;
        let at = |j: usize| {
            joints.get(j).and_then(Option::as_ref).and_then(|p| v3(p)).and_then(|p| cam.project(p, rect)).map(|p| p.0)
        };
        let listed: Vec<[usize; 2]> = self
            .scene
            .skeleton
            .bones
            .iter()
            .filter_map(|b: &Value| Some([b.get("a")?.as_u64()? as usize, b.get("b")?.as_u64()? as usize]))
            .collect();
        let bones = if listed.is_empty() { topology().visible_bones() } else { listed };
        let t = topology();
        let tone = |j: usize| {
            if j == t.wrist.left || t.tips.left.iter().any(|&tip| (tip..tip + 4).contains(&j)) {
                color("skelLeft")
            } else if j == t.wrist.right || t.tips.right.iter().any(|&tip| (tip..tip + 4).contains(&j)) {
                color("skelRight")
            } else {
                color("skelBody")
            }
        };
        for [a, b] in bones {
            if let (Some(p), Some(q)) = (at(a), at(b)) {
                painter.line_segment([p, q], Stroke::new(2.0, tone(a.max(b))));
            }
        }
        for j in 0..joints.len() {
            if let Some(p) = at(j) {
                painter.circle_filled(p, 2.5, tone(j));
            }
        }
    }
}

/// A closed outline on screen (a projected face, the board, a stone's outline in the overlay) as egui should paint it:
/// `None` for a sliver thinner than about a quarter of a point (a face seen almost edge-on), otherwise its corners
/// with the ones sharper than about 25° cut off. egui's anti-aliasing moves every corner of a closed line or a filled
/// polygon out along its mitre, without a limit, and the mitre of a sharp corner is a thin spike far out of the shape.
pub(crate) fn blunt(points: &[Pos2]) -> Option<Vec<Pos2>> {
    let n = points.len();
    let edge = |i: usize| points[(i + 1) % n] - points[i];
    let twice_area: f32 = (0..n).map(|i| points[i].x * edge(i).y - points[i].y * edge(i).x).sum();
    let longest = (0..n).map(|i| edge(i).length()).fold(0.0, f32::max);
    if twice_area.abs() < 0.5 * longest {
        return None;
    }
    let mut out = Vec::with_capacity(2 * n);
    for (i, &p) in points.iter().enumerate() {
        let (a, b) = (points[(i + n - 1) % n] - p, edge(i));
        let (la, lb) = (a.length(), b.length());
        if a.dot(b) > 0.9 * la * lb {
            let cut = (0.25 * la.min(lb)).min(1.0);
            out.extend([p + a * (cut / la), p + b * (cut / lb)]);
        } else {
            out.push(p);
        }
    }
    Some(out)
}

/// A point of the screen for tests and pickers: the projection of a scene point, if visible.
pub fn screen_point(camera: &OrbitCamera, p: V3, rect: Rect) -> Option<Pos2> {
    camera.project(p, rect).map(|(p, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn both_rings_of_an_aug28_dogbone_match_its_video_outline() {
        let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/recordings/aug28-1276-1284-full.ndjson.gz");
        let frame = pdigger_factory_sdk::Replay::open(recording).unwrap().frames().next().unwrap().unwrap();
        let overlay = crate::draw::overlay_of_frame(&frame).unwrap();
        let scene = crate::draw::scene_of_frame(&frame).unwrap();
        let outline = &overlay.stones.iter().find(|s| s.id == Some(2637)).unwrap().outline;
        let stone = scene.stones.iter().find(|s| s.id == Some(2637)).unwrap();
        let prism = StonePrism::new(stone).unwrap();

        // The two end corners and the opposite corner give the image axes of this flat, rotated stone.
        let pixel = |i: usize| pos2(outline[i][0] as f32, outline[i][1] as f32);
        let (origin, along, across) = (pixel(0), pixel(5) - pixel(0), pixel(11) - pixel(0));
        let [x, y, z, w] = quat(&stone.q);
        let centre = v3(&stone.pos).unwrap();
        let [length, width, height] = v3(&stone.dims).unwrap();
        for (ring, face_z) in [(prism.top, height / 2.0), (prism.bottom, -height / 2.0)] {
            assert_eq!(ring.len(), outline.len());
            for (i, vertex) in ring.into_iter().enumerate() {
                let local = rotate([-x, -y, -z, w], sub(vertex, centre));
                let at = origin + along * (0.5 + local[0] / length) + across * (0.5 + local[1] / width);
                assert!((at - pixel(i)).length() < 0.02, "outline vertex {i}: {at:?} vs {:?}", pixel(i));
                assert!((local[2] - face_z).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn the_camera_projects_orbits_and_zooms() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        let mut cam = OrbitCamera::top([0.0, 0.0, 0.0], 2.0);
        let (centre, depth) = cam.project([0.0, 0.0, 0.0], rect).unwrap();
        assert!((centre - rect.center()).length() < 1e-3 && (depth - 2.0).abs() < 1e-3);
        assert!(cam.project([0.0, 0.0, 5.0], rect).is_none(), "behind the camera");
        cam.orbit(vec2(10.0, 1000.0));
        assert_eq!(cam.pitch, 1.55);
        let d = cam.distance;
        cam.zoom(100.0);
        assert!(cam.distance < d);
        cam.zoom(-1.0e6);
        assert_eq!(cam.distance, 50.0);
        let r = rotate([0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2], [1.0, 0.0, 0.0]);
        assert!((r[0]).abs() < 1e-6 && (r[1] - 1.0).abs() < 1e-6);
        assert_eq!(normalize([0.0; 3]), [0.0; 3]);
        assert!(v3(&[1.0]).is_none());
        assert!(screen_point(&cam, [0.0, 0.0, 0.0], rect).is_some());
    }

    #[test]
    fn slivers_are_dropped_and_sharp_corners_cut() {
        let square = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0), pos2(0.0, 10.0)];
        assert_eq!(blunt(&square).unwrap(), square);
        assert!(blunt(&[pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 0.2), pos2(0.0, 0.2)]).is_none());
        // A face seen nearly edge-on: its two far corners are each cut into two, none sharper than a right angle.
        let sliver = [pos2(0.0, 0.0), pos2(50.0, -0.5), pos2(100.0, 0.0), pos2(50.0, 1.0)];
        let cut = blunt(&sliver).unwrap();
        assert_eq!(cut.len(), 6);
        for i in 0..6 {
            let (a, b) = (cut[(i + 5) % 6] - cut[i], cut[(i + 1) % 6] - cut[i]);
            assert!(a.dot(b) <= 1e-3 * a.length() * b.length(), "corner {i} of {cut:?}");
        }
        assert!((cut[0] - sliver[0]).length() <= 1.0 && (cut[1] - sliver[0]).length() <= 1.0);
    }

    #[test]
    fn framing_follows_the_board_or_the_stones() {
        let mut scene = Scene3D::default();
        assert_eq!(OrbitCamera::framing(&scene).target, [0.0, 0.0, 0.1]);
        scene.stones.push(pdigger_factory_sdk::models::DrawStone3D { pos: vec![1.0, 2.0, 0.0], ..Default::default() });
        assert_eq!(OrbitCamera::framing(&scene).target, [1.0, 2.0, 0.1]);
        scene.board =
            Some(pdigger_factory_sdk::models::Rect { cx: 0.5, cy: -0.75, hw: 0.8, hh: 0.7, ..Default::default() });
        let cam = OrbitCamera::framing(&scene);
        assert!(
            (cam.target[0] - 0.5).abs() < 1e-6
                && (cam.target[1] + 0.75).abs() < 1e-6
                && (cam.target[2] - 0.08).abs() < 1e-6
        );
        assert!((cam.distance - 2.64).abs() < 1e-3);
    }
}
