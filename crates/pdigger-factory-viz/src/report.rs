//! [`ReportView`]: a tray's report (spec 4.7 of the SDK design, simple mode) as a picture. The tray as it arrived and
//! as it left, side by side, each from straight above, every stone in the colour of what the report says of it
//! ([`report_marks`], [`ReportRole::color`]), the stones removed, added and moved labelled with their ids, a dashed
//! red outline where each removed stone was, a legend and the report's text. The TypeScript SDK's `TrayReportView`
//! draws the same, in the same colours.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{
    Align2, Color32, CornerRadius, FontId, Galley, Painter, Pos2, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, UiBuilder, pos2, vec2,
};
use pdigger_factory_sdk::interpreter::text::clock;
use pdigger_factory_sdk::models::{DrawStone3D, Scene3D, Skeleton3D, StoneFamily};
use pdigger_factory_sdk::{Tray, TrayChange, TrayReport, TrayStone, TrayStoneStatus};

use crate::craft::Tokens;
use crate::craft::theme::{hex, semibold};
use crate::scene::{OrbitCamera, SceneView, box_corners, quat, rotate};

/// What a tray's report says of a stone: removed, added or moved; else whether the SDK is sure of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ReportRole {
    /// Removed from the tray (`TrayReport::removed`).
    Removed,
    /// Added to the tray (`TrayReport::added`).
    Added,
    /// Moved on the tray (`TrayReport::moved`).
    Moved,
    /// Neither, and the SDK is sure of the stone.
    Sure,
    /// Neither, and the SDK is not sure of the stone.
    Unsure,
}

impl ReportRole {
    /// Every role, in the legend's order.
    pub const ALL: [ReportRole; 5] =
        [ReportRole::Removed, ReportRole::Added, ReportRole::Moved, ReportRole::Sure, ReportRole::Unsure];

    /// Its colour ([`REPORT_COLORS`]).
    pub fn color(self) -> Color32 {
        match self {
            ReportRole::Removed => hex(0xD8432E),
            ReportRole::Added => hex(0x1E96C8),
            ReportRole::Moved => hex(0x6E48B9),
            ReportRole::Sure => hex(0x0E8A72),
            ReportRole::Unsure => hex(0xDE7C34),
        }
    }

    /// Its name in the legend: `removed`, `added`, `moved`, `sure` or `not sure`.
    pub fn label(self) -> &'static str {
        match self {
            ReportRole::Removed => "removed",
            ReportRole::Added => "added",
            ReportRole::Moved => "moved",
            ReportRole::Sure => "sure",
            ReportRole::Unsure => "not sure",
        }
    }
}

/// The colours of the roles, the same as the TypeScript SDK's `TrayReportView` (`REPORT_COLORS`):
/// red removed, blue added, violet moved, green sure, amber not sure.
pub const REPORT_COLORS: [(ReportRole, &str); 5] = [
    (ReportRole::Removed, "#D8432E"),
    (ReportRole::Added, "#1E96C8"),
    (ReportRole::Moved, "#6E48B9"),
    (ReportRole::Sure, "#0E8A72"),
    (ReportRole::Unsure, "#DE7C34"),
];

/// A world stone without a mark (not on the tray at that point) is drawn dimmed grey ...
const DIM: Color32 = hex(0x7D8790);
const DIM_ALPHA: f32 = 0.3;
/// ... and an unclear removal or addition translucent, with a dashed outline.
const UNCLEAR_ALPHA: f32 = 0.45;

/// A stone of one side of a report view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportMark {
    /// The tray stone's id.
    pub stone: String,
    /// What the report says of it.
    pub role: ReportRole,
    /// False only for a removal or an addition that is unclear (a move is always certain).
    pub certain: bool,
    /// For removed, added and moved: the tray stone's id after the tray's number, the world id it was first seen under
    /// (with `-2`, `-3`, ... for a second or third stone of the tray first seen under it in the same tenth of a
    /// second); empty for sure and not sure.
    pub label: String,
}

impl ReportMark {
    /// The colour its stone is drawn in: its role's, translucent when it is unclear.
    pub fn fill(&self) -> Color32 {
        let color = self.role.color();
        if self.certain { color } else { color.gamma_multiply(UNCLEAR_ALPHA) }
    }
}

/// The marks of a report's two sides, by world id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportMarks {
    /// Every stone the tray arrived with, by the world id it was first seen under (in its id, after the tray's number).
    pub before: BTreeMap<i64, ReportMark>,
    /// Every stone on the tray as it left (status on), by the world id it stands for then.
    pub after: BTreeMap<i64, ReportMark>,
}

/// A tray stone's id after its tray's number (spec 4.6): the world id it was first seen under, then `-2`, `-3`, ...
/// when another stone of the tray was first seen under that world id in the same tenth of a second.
fn id_tail(id: &str, tray: i64) -> &str {
    let head = format!("-{tray}-");
    id.find(&head).map_or(id, |at| &id[at + head.len()..])
}

/// The world id a tray stone was first seen under, from its id.
fn first_world_id(id: &str, tray: i64) -> Option<i64> {
    id_tail(id, tray).split('-').next()?.parse().ok()
}

/// The marks of a tray's report (spec 4.7) for the two sides of a [`ReportView`], by world id; `None` for a tray
/// without a report (it has not left, or it never arrived).
///
/// - `before`: every stone the tray arrived with, by the world id it was first seen under (in its id, after the
///   tray's number): [`ReportRole::Removed`] for a stone the report removes (certain as its change), [`ReportRole::Moved`] for
///   one it reports moved, else [`ReportRole::Sure`] or [`ReportRole::Unsure`] by the stone's `sure`;
/// - `after`: every stone on the tray as it left (status on), by the world id it stands for then:
///   [`ReportRole::Added`] for a stone the report adds (certain as its change), [`ReportRole::Moved`], else sure or
///   not sure.
///
/// The label of a removed, added or moved stone is its id after the tray's number: the world id it was first seen
/// under, with `-2`, `-3`, ... for a second or third stone first seen under it in the same tenth of a second. Sure and
/// unsure stones have none.
///
/// ```
/// # fn demo(tray: &pdigger_factory_sdk::Tray) {
/// if let Some(marks) = pdigger_factory_viz::report_marks(tray) {
///     for (world_id, mark) in &marks.before {
///         println!("{world_id}: {:?} {}", mark.role, mark.label);
///     }
/// }
/// # }
/// ```
pub fn report_marks(tray: &Tray) -> Option<ReportMarks> {
    Some(marks_of(tray, &tray.report()?))
}

fn marks_of(tray: &Tray, report: &TrayReport) -> ReportMarks {
    let find = |changes: &[TrayChange], role: ReportRole, id: &str| {
        changes.iter().find(|c| c.stone == id).map(|c| (role, c.certain))
    };
    let mark = |s: &TrayStone, change: Option<(ReportRole, bool)>| match change {
        Some((role, certain)) => {
            ReportMark { stone: s.id.clone(), role, certain, label: id_tail(&s.id, tray.n).to_string() }
        }
        None => ReportMark {
            stone: s.id.clone(),
            role: if s.sure { ReportRole::Sure } else { ReportRole::Unsure },
            certain: true,
            label: String::new(),
        },
    };
    let mut marks = ReportMarks::default();
    for s in &tray.stones {
        let moved = find(&report.moved, ReportRole::Moved, &s.id);
        if let Some(first) = first_world_id(&s.id, tray.n).filter(|_| !s.added) {
            marks.before.insert(first, mark(s, find(&report.removed, ReportRole::Removed, &s.id).or(moved)));
        }
        if let Some(now) = s.world_id.filter(|_| s.status == TrayStoneStatus::On) {
            marks.after.insert(now, mark(s, find(&report.added, ReportRole::Added, &s.id).or(moved)));
        }
    }
    marks
}

/// Draw the stones in the final account at their reported places. A matching solid supplies its
/// orientation and height, even when only the arrival frame shows that stone.
fn leaving_stones(tray: &Tray, before: &Scene3D, after: &Scene3D) -> Vec<DrawStone3D> {
    let solid = |scene: &Scene3D, id: i64| {
        scene.stones.iter().find(|s| s.id == Some(id) && s.dims.len() >= 3 && s.pos.len() >= 3).cloned()
    };
    let mut stones = Vec::new();
    for stone in &tray.stones {
        let Some(id) = stone.world_id.filter(|_| stone.status == TrayStoneStatus::On) else { continue };
        let template = solid(after, id).or_else(|| {
            let first = first_world_id(&stone.id, tray.n).filter(|_| !stone.added)?;
            solid(before, first)
        });
        let [x, y] = stone.left_place.unwrap_or(stone.place);
        let long = stone.size[0].max(stone.size[1]).max(0.001);
        let short = stone.size[0].min(stone.size[1]).max(0.001);
        let mut shape = template.unwrap_or_else(|| DrawStone3D {
            family: StoneFamily::Box,
            dims: vec![long, short, short * 0.3],
            pos: vec![x, y, short * 0.15],
            q: vec![0.0, 0.0, 0.0, 1.0],
            ..Default::default()
        });
        let across = shape.dims[1] > shape.dims[0];
        shape.dims[..2].copy_from_slice(&if across { [short, long] } else { [long, short] });
        shape.pos[..2].copy_from_slice(&[x, y]);
        shape.id = Some(id);
        shape.alpha = 1.0;
        stones.push(shape);
    }
    stones
}

/// The vertical field of view of the sides' cameras: narrow, so that from straight above the stones' sides hardly show.
const FOV: f32 = 0.25;
/// The share of a side's view its extent fills (the room around it).
const FILL: f32 = 0.94;
/// The height of a side's title row, and the gap between the parts.
const TITLE_H: f32 = 22.0;
const GAP: f32 = 10.0;
/// The view puts the report's text beside the sides when it is at least this many times as wide as it is high (else
/// under them), in a column of this share of its width (between 220 and 520 points).
const WIDE: f32 = 1.4;
const COLUMN: f32 = 0.4;
/// A row of the legend.
const LEGEND_ROW: f32 = 18.0;

/// A side's extent in scene (x, y): its board and its stones (a unit square about the origin without either).
fn extent(scene: &Scene3D) -> Rect {
    let mut points: Vec<Pos2> =
        scene.stones.iter().filter_map(box_corners).flatten().map(|c| pos2(c[0], c[1])).collect();
    if let Some(b) = &scene.board {
        let (c, s) = ((b.theta as f32).cos(), (b.theta as f32).sin());
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let (dx, dy) = (x * b.hw as f32, y * b.hh as f32);
            points.push(pos2(b.cx as f32 + dx * c - dy * s, b.cy as f32 + dx * s + dy * c));
        }
    }
    if points.is_empty() { Rect::from_center_size(Pos2::ZERO, vec2(1.0, 1.0)) } else { Rect::from_points(&points) }
}

/// A camera straight above the middle of `extent` (scene x to the right, y up), as far as all of it needs to fit
/// `view` with a little room.
fn camera(extent: Rect, view: Rect) -> OrbitCamera {
    let focal = view.height() * 0.5 / (FOV * 0.5).tan();
    let fit = |scene: f32, screen: f32| scene * focal / (screen * FILL).max(1.0);
    let distance = fit(extent.width(), view.width()).max(fit(extent.height(), view.height())).max(0.05);
    OrbitCamera { fov_y: FOV, ..OrbitCamera::top([extent.center().x, extent.center().y, 0.0], distance) }
}

/// Whether a stone's long side lies nearer the scene's y axis than its x axis.
fn lies_along_y(stone: &DrawStone3D) -> bool {
    let side = |i: usize| stone.dims.get(i).copied().unwrap_or(0.0);
    let axis = if side(0) >= side(1) { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let [x, y, _] = rotate(quat(&stone.q), axis);
    y.abs() > x.abs()
}

/// The place a removed stone was taken from, axis-aligned: the middle and the half sizes of its change's `place` and
/// `size`, its long side along x, or along y when its world stone lay that way as the tray arrived (in `before`).
fn place_of(change: &TrayChange, tray: i64, before: &Scene3D) -> (Pos2, [f32; 2]) {
    let long = change.size[0].max(0.001) as f32 / 2.0;
    let short = change.size[1].max(0.001) as f32 / 2.0;
    let first = first_world_id(&change.stone, tray);
    let upright = before.stones.iter().find(|s| s.id.is_some() && s.id == first).is_some_and(lies_along_y);
    (pos2(change.place[0] as f32, change.place[1] as f32), if upright { [short, long] } else { [long, short] })
}

/// A closed dashed outline through `points`.
fn dashed(painter: &Painter, points: &[Pos2], stroke: Stroke) {
    let mut ring = points.to_vec();
    ring.extend(points.first());
    painter.extend(Shape::dashed_line(&ring, stroke, 5.0, 3.5));
}

/// A label on a stone: its text in white on a plate of `fill` with a thin white ring, centred at `at`.
fn plate(painter: &Painter, at: Pos2, text: &str, fill: Color32) {
    let galley = painter.layout_no_wrap(text.to_string(), semibold(11.0), Color32::WHITE);
    let rect = Rect::from_center_size(at, galley.size()).expand2(vec2(4.0, 1.0));
    painter.rect(
        rect,
        CornerRadius::same(3),
        fill,
        Stroke::new(1.0, Color32::from_white_alpha(190)),
        StrokeKind::Outside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// A key of the legend.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    Role(ReportRole),
    Unclear,
    Place,
    Dim,
}

impl Key {
    const ALL: [Key; 8] = [
        Key::Role(ReportRole::Removed),
        Key::Role(ReportRole::Added),
        Key::Role(ReportRole::Moved),
        Key::Role(ReportRole::Sure),
        Key::Role(ReportRole::Unsure),
        Key::Unclear,
        Key::Place,
        Key::Dim,
    ];

    fn text(self) -> &'static str {
        match self {
            Key::Role(role) => role.label(),
            Key::Unclear => "unclear",
            Key::Place => "where a removed stone was",
            Key::Dim => "not on the tray",
        }
    }

    /// Its swatch in `rect`.
    fn paint(self, painter: &Painter, rect: Rect) {
        let outline = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        match self {
            Key::Role(role) => {
                painter.rect_filled(rect, 2.0, role.color());
            }
            Key::Unclear => {
                painter.rect_filled(rect, 2.0, ReportRole::Added.color().gamma_multiply(UNCLEAR_ALPHA));
                dashed(painter, &outline, Stroke::new(1.5, ReportRole::Added.color()));
            }
            Key::Place => dashed(painter, &outline, Stroke::new(1.5, ReportRole::Removed.color())),
            Key::Dim => {
                painter.rect_filled(rect, 2.0, DIM.gamma_multiply(DIM_ALPHA));
            }
        }
    }
}

/// Where the parts of a view go: each side's title row and its viewport, the legend under the sides, and the
/// report's text (beside them in a wide rectangle, else under the legend) with its font size.
#[derive(Debug, Clone, PartialEq)]
struct Layout {
    titles: [Rect; 2],
    views: [Rect; 2],
    legend: Vec<(Key, Pos2)>,
    text: Rect,
    size: f32,
}

/// A tray's report as a picture, for a tray that has left: the tray as it arrived (on the left) and as it left (on
/// the right), side by side, each from straight above, with a title ("as it arrived 0:22.5"). Every stone is filled in
/// the colour of its [`report_marks`] role (red removed, blue added, violet moved; the others green when the SDK is
/// sure of them, amber when not); an unclear removal or addition is translucent with a dashed outline, and a world
/// stone outside the arrival account dimmed grey. The leaving side draws the final account. The stones removed, added and moved carry their
/// labels, and on the side as it left a dashed red outline marks each removed stone's final place
/// (`TrayChange::place` and `size`, axis-aligned: its long side along x, or along y when its world stone lay that way
/// as the tray arrived). A legend under the sides, and the report's text (`TrayReport::describe`) beside them in a
/// wide rectangle, else under them.
///
/// ```
/// # fn demo(tray: &pdigger_factory_sdk::Tray, ui: &mut egui::Ui) {
/// use pdigger_factory_sdk::models::Scene3D;
/// // the 3-D draw lists of the world at the tray's arrival point and at its leaving point (draw::scene_of_frame)
/// let (before, after) = (Scene3D::default(), Scene3D::default());
/// if let Some(view) = pdigger_factory_viz::ReportView::new(tray, &before, &after) {
///     view.show(ui);
/// }
/// # }
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ReportView {
    report: TrayReport,
    marks: ReportMarks,
    before: Scene3D,
    after: Scene3D,
    places: Vec<(Pos2, [f32; 2])>,
    text: String,
}

impl ReportView {
    /// The view of `tray`'s report; `before` and `after` are the 3-D draw lists of the world at its arrival point
    /// (`tray.arrived`) and at its leaving point (`tray.leaving`), as [`draw::scene_of_frame`](crate::draw::scene_of_frame)
    /// makes them of the frames there. The leaving account supplies its stones, places and sizes, even
    /// when the leaving frame omits them. `None` for a tray without a report (it
    /// has not left, or it never arrived).
    pub fn new(tray: &Tray, before: &Scene3D, after: &Scene3D) -> Option<ReportView> {
        let report = tray.report()?;
        let marks = marks_of(tray, &report);
        let bare = |scene: &Scene3D| Scene3D { skeleton: Skeleton3D::default(), focus: None, ..scene.clone() };
        let places = report.removed.iter().map(|c| place_of(c, report.tray, before)).collect();
        let text = report.describe();
        let leaving = Scene3D { stones: leaving_stones(tray, before, after), ..bare(after) };
        Some(ReportView { report, marks, before: bare(before), after: leaving, places, text })
    }

    /// The report shown.
    pub fn report(&self) -> &TrayReport {
        &self.report
    }

    /// The marks the stones are coloured by.
    pub fn marks(&self) -> &ReportMarks {
        &self.marks
    }

    /// Takes the space left in `ui` and paints into it; the report's text scrolls when it does not fit.
    pub fn show(&self, ui: &mut Ui) -> Response {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let t = Tokens::get(ui.ctx());
        let painter = ui.painter_at(rect);
        let layout = self.layout(&painter, rect);
        self.paint_sides(&painter, &layout, &t);
        let mut text = ui.new_child(UiBuilder::new().max_rect(layout.text));
        egui::ScrollArea::vertical().id_salt("report-text").auto_shrink([false, false]).show(&mut text, |ui| {
            let words = RichText::new(&self.text).font(FontId::monospace(layout.size)).color(t.text);
            ui.add(egui::Label::new(words).wrap());
        });
        response
    }

    /// Paints the view into `rect` with `painter` alone: the report's text is cut where it does not fit.
    pub fn paint(&self, painter: &Painter, rect: Rect) {
        let t = Tokens::get(painter.ctx());
        let layout = self.layout(painter, rect);
        self.paint_sides(painter, &layout, &t);
        let text: Arc<Galley> =
            painter.layout(self.text.clone(), FontId::monospace(layout.size), t.text, layout.text.width());
        painter.with_clip_rect(layout.text.intersect(painter.clip_rect())).galley(layout.text.min, text, t.text);
    }

    /// The two sides as `layout` places them: as it arrived, then as it left.
    fn sides(&self, layout: &Layout) -> [Side<'_>; 2] {
        [
            Side::new(&self.before, &self.marks.before, layout.views[0]),
            Side::new(&self.after, &self.marks.after, layout.views[1]),
        ]
    }

    /// The parts' places in `rect`, the text's font size measured with `painter`'s fonts.
    fn layout(&self, painter: &Painter, rect: Rect) -> Layout {
        let wide = rect.width() >= WIDE * rect.height();
        let column = if wide { (rect.width() * COLUMN).clamp(220.0, 520.0) } else { rect.width() };
        // the font that fits the longest change line into the column (the first line wraps), 8 to 11.5 points
        let longest = self.text.lines().skip(1).max_by_key(|l| l.chars().count()).unwrap_or_default();
        let at_10 = painter.layout_no_wrap(longest.to_string(), FontId::monospace(10.0), Color32::WHITE).size().x;
        let size = (10.0 * column / at_10.max(1.0)).clamp(8.0, 11.5);
        let (left, text) = if wide {
            let split = rect.max.x - column;
            (
                Rect::from_min_max(rect.min, pos2(split - GAP, rect.max.y)),
                Rect::from_min_max(pos2(split, rect.min.y), rect.max),
            )
        } else {
            let lines = painter.layout(self.text.clone(), FontId::monospace(size), Color32::WHITE, column).size().y;
            let top = rect.max.y - lines.min(rect.height() * 0.4);
            (
                Rect::from_min_max(rect.min, pos2(rect.max.x, top - GAP)),
                Rect::from_min_max(pos2(rect.min.x, top), rect.max),
            )
        };
        // the legend's keys, flowed into rows under the sides
        let (mut legend, mut x, mut row) = (Vec::new(), 0.0, 0.0);
        for key in Key::ALL {
            let width = 21.0
                + painter.layout_no_wrap(key.text().to_string(), FontId::proportional(11.5), Color32::WHITE).size().x;
            if x > 0.0 && x + width > left.width() {
                (x, row) = (0.0, row + 1.0);
            }
            legend.push((key, pos2(x, row * LEGEND_ROW)));
            x += width + 14.0;
        }
        let legend_top = left.max.y - (row + 1.0) * LEGEND_ROW;
        for (_, at) in &mut legend {
            *at += vec2(left.min.x, legend_top);
        }
        let half = ((left.width() - GAP) / 2.0).max(0.0);
        let sides = [left.min.x, left.min.x + half + GAP];
        let titles = sides.map(|x| Rect::from_min_size(pos2(x, left.min.y), vec2(half, TITLE_H)));
        let views = sides.map(|x| Rect::from_min_max(pos2(x, left.min.y + TITLE_H), pos2(x + half, legend_top - GAP)));
        Layout { titles, views, legend, text, size }
    }

    /// The two sides and the legend.
    fn paint_sides(&self, painter: &Painter, layout: &Layout, t: &Tokens) {
        let titles = [
            format!("as it arrived {}", clock(self.report.arrived.t)),
            format!("as it left {}", clock(self.report.left.t)),
        ];
        for (i, side) in self.sides(layout).into_iter().enumerate() {
            let at = layout.titles[i].center_bottom() - vec2(0.0, 5.0);
            painter.text(at, Align2::CENTER_BOTTOM, &titles[i], semibold(13.0), t.text_strong);
            // the places the removed stones were taken from, on the side as it left
            let places: &[(Pos2, [f32; 2])] = if i == 1 { &self.places } else { &[] };
            if side.view.width() >= 1.0 && side.view.height() >= 1.0 {
                side.paint(&painter.with_clip_rect(side.view.intersect(painter.clip_rect())), t, places);
            }
        }
        for (key, at) in &layout.legend {
            let swatch = Rect::from_min_size(*at + vec2(0.0, 3.5), vec2(16.0, 11.0));
            key.paint(painter, swatch);
            let text = painter.layout_no_wrap(key.text().to_string(), FontId::proportional(11.5), t.text_dim);
            painter.galley(pos2(swatch.max.x + 5.0, swatch.center().y - text.size().y / 2.0), text, t.text_dim);
        }
    }
}

/// A side of the view as drawn: the world at its point, the marks of its stones, its viewport and the camera straight
/// above it.
struct Side<'a> {
    scene: &'a Scene3D,
    marks: &'a BTreeMap<i64, ReportMark>,
    view: Rect,
    camera: OrbitCamera,
}

impl<'a> Side<'a> {
    /// The side of `scene` with `marks`, seen in `view` by a camera that fits it.
    fn new(scene: &'a Scene3D, marks: &'a BTreeMap<i64, ReportMark>, view: Rect) -> Side<'a> {
        Side { scene, marks, view, camera: camera(extent(scene), view) }
    }

    /// The stones that have a mark, with it.
    fn marked(&self) -> impl Iterator<Item = (&DrawStone3D, &ReportMark)> {
        self.scene.stones.iter().filter_map(|s| Some((s, self.marks.get(&s.id?)?)))
    }

    /// A scene point on screen.
    fn at(&self, p: [f32; 3]) -> Option<Pos2> {
        self.camera.project(p, self.view).map(|p| p.0)
    }

    /// The labels: each removed, added or moved stone's mark, and the middle of its top on screen.
    fn labels(&self) -> Vec<(&ReportMark, Pos2)> {
        let top = |stone: &DrawStone3D| {
            let c = box_corners(stone)?;
            self.at([4, 5, 6, 7]
                .iter()
                .fold([0.0; 3], |m, &k| [m[0] + c[k][0] / 4.0, m[1] + c[k][1] / 4.0, m[2] + c[k][2] / 4.0]))
        };
        self.marked().filter(|(_, m)| !m.label.is_empty()).filter_map(|(s, m)| Some((m, top(s)?))).collect()
    }

    /// The outline of an axis-aligned place on the tray (its middle and half sizes) on screen.
    fn place(&self, (middle, [hx, hy]): (Pos2, [f32; 2])) -> Option<[Pos2; 4]> {
        let corner = |sx: f32, sy: f32| self.at([middle.x + sx * hx, middle.y + sy * hy, 0.002]);
        Some([corner(-1.0, -1.0)?, corner(1.0, -1.0)?, corner(1.0, 1.0)?, corner(-1.0, 1.0)?])
    }

    /// Paints the side into its viewport (`painter` clipped to it): the world from above with its stones filled by
    /// their marks, a dashed outline round each unclear one, the `places` removed stones were taken from, and the
    /// labels.
    fn paint(&self, painter: &Painter, t: &Tokens, places: &[(Pos2, [f32; 2])]) {
        let fill = |s: &DrawStone3D| {
            Some(s.id.and_then(|id| self.marks.get(&id)).map_or(DIM.gamma_multiply(DIM_ALPHA), ReportMark::fill))
        };
        let mut camera = self.camera;
        SceneView::new(self.scene, &mut camera).tint(&fill).paint(painter, self.view);
        if self.scene.stones.is_empty() && self.scene.board.is_none() {
            let font = FontId::proportional(12.0);
            painter.text(self.view.center(), Align2::CENTER_CENTER, "no world at this point", font, t.viewport_text);
        }
        for (stone, mark) in self.marked().filter(|(_, m)| !m.certain) {
            let top: Option<Vec<Pos2>> =
                box_corners(stone).and_then(|c| [4, 5, 7, 6].iter().map(|&k| self.at(c[k])).collect());
            if let Some(top) = top {
                dashed(painter, &top, Stroke::new(1.5, mark.role.color()));
            }
        }
        for outline in places.iter().filter_map(|&p| self.place(p)) {
            dashed(painter, &outline, Stroke::new(2.0, ReportRole::Removed.color()));
        }
        for (mark, at) in self.labels() {
            plate(painter, at, &mark.label, mark.role.color());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft::{Accent, Brightness, theme};
    use crate::draw::{parse_color, scene_of_frame};
    use crate::raster::{Snapshot, render};
    use pdigger_factory_sdk::Replay;
    use serde_json::{Value, json};

    fn sid(world: i64) -> String {
        format!("s7t1.0-3-{world}")
    }

    fn at(t: f64) -> Value {
        json!({"g": (t * 5.0).round() as i64, "t": t, "wall": null})
    }

    fn merge(mut into: Value, more: Value) -> Value {
        for (k, v) in more.as_object().unwrap() {
            into[k] = v.clone();
        }
        into
    }

    /// A tray stone of tray 3 first seen under world id `world`, on the tray and sure, at its place as it arrived and
    /// as it left, then `more`.
    fn tray_stone(world: i64, more: Value) -> Value {
        let place = more.get("place").cloned().unwrap_or(json!([0.0, 0.0]));
        let stone = json!({
            "id": sid(world), "world_id": world, "status": "on", "sure": true, "added": false, "certain": null,
            "born": at(1.0), "size": [0.4, 0.2], "first_place": place, "left_place": place,
        });
        merge(stone, more)
    }

    fn removed(more: Value) -> Value {
        merge(json!({"status": "removed", "world_id": null, "certain": true, "left_place": null}), more)
    }

    fn fact(kind: &str, more: Value) -> Value {
        merge(json!({"kind": kind, "tray": 3, "session": 7, "video": "v", "g": 10, "t": 2.0}), more)
    }

    /// Tray 3, the TypeScript SDK's: it arrived with 1, 2, 10, 11, 3, 4, 5 and 9; 1, 10 and 11 were removed for
    /// certain, 2 unclear; 3 moved; 4 is on it as world stone 14; 5 is not sure; 9 has no world stone as it left; 6
    /// was set down for certain, 7 is an unclear addition; 8 was set down and taken again.
    fn tray3() -> Tray {
        let stones = json!([
            tray_stone(1, removed(json!({"place": [0.2, 0.3]}))),
            tray_stone(2, removed(json!({"place": [0.7, 0.3], "certain": false, "sure": false}))),
            tray_stone(10, removed(json!({"place": [1.2, 0.3]}))),
            tray_stone(11, removed(json!({"place": [1.7, 0.3]}))),
            tray_stone(
                3,
                json!({"place": [0.7, 1.0], "first_place": [0.2, 1.0], "left_place": [0.7, 1.0], "touched": 30})
            ),
            tray_stone(4, json!({"world_id": 14, "place": [1.2, 1.0]})),
            tray_stone(5, json!({"place": [1.7, 1.0], "sure": false})),
            tray_stone(9, json!({"world_id": null, "place": [2.2, 1.0]})),
            tray_stone(6, json!({"added": true, "certain": true, "place": [2.2, 0.3], "born": at(4.0)})),
            tray_stone(
                7,
                json!({"added": true, "certain": false, "sure": false, "place": [1.2, 0.3], "born": at(8.0)})
            ),
            tray_stone(8, removed(json!({"added": true, "certain": false, "place": [2.2, 1.4], "born": at(5.0)}))),
        ]);
        let facts = json!([
            fact("arrived", json!({"g": 5, "t": 1.0, "stones": 8})),
            fact("removed", json!({"stone": sid(1), "certain": true, "code": "missing", "t": 3.0})),
            fact("added", json!({"stone": sid(6), "world_id": 6, "certain": true, "code": "set_down", "t": 4.0})),
            fact("removed", json!({"stone": sid(11), "certain": true, "code": "carried_away", "t": 4.5})),
            fact("added", json!({"stone": sid(8), "world_id": 8, "certain": true, "code": "set_down", "t": 5.0})),
            fact("removed", json!({"stone": sid(10), "certain": true, "code": "missing", "t": 6.0})),
            fact("removed", json!({"stone": sid(8), "certain": false, "code": "not_found", "t": 9.0})),
            fact("removed", json!({"stone": sid(2), "certain": false, "code": "not_found", "t": 9.0})),
            fact(
                "added",
                json!({"stone": sid(7), "world_id": 7, "certain": false, "code": "not_recognised", "t": 9.0})
            ),
            fact("leaving", json!({"g": 45, "t": 9.0, "stones": 7, "sure": 4, "added": 2, "removed": 3, "unclear": 3})),
        ]);
        let tray = json!({"session": 7, "video": "v", "n": 3, "arrived": at(1.0), "leaving": at(9.0), "stones": stones,
            "facts": facts});
        serde_json::from_value(tray).unwrap()
    }

    /// A world stone at a scene place, 0.1 high: 0.4 by 0.2 unless `dims`, lengthwise unless `q`.
    fn ws(id: i64, [x, y]: [f64; 2], dims: [f64; 3], q: [f64; 4]) -> Value {
        json!({"id": id, "pos": [x, y, 0.05], "q": q, "dims": dims, "alpha": 1.0})
    }

    const LONG: [f64; 3] = [0.4, 0.2, 0.1];
    const FLAT: [f64; 4] = [0.0, 0.0, 0.0, 1.0];
    const TURNED: [f64; 4] = [0.0, 0.0, std::f64::consts::FRAC_1_SQRT_2, std::f64::consts::FRAC_1_SQRT_2];

    /// The world as tray 3 arrived (10 not seen; 50, not a tray stone, off the board) and as it left (4 as world
    /// stone 14, the additions, and 51 where 9 was).
    fn worlds() -> (Scene3D, Scene3D) {
        let before = json!({
            "board": {"cx": 1.2, "cy": 0.7, "theta": 0.0, "hw": 1.3, "hh": 0.8},
            "stones": [
                ws(1, [0.2, 0.3], LONG, TURNED), // its long side along y: turned
                ws(2, [0.7, 0.3], [0.2, 0.4, 0.1], FLAT), // its long side along y: the longer second side
                ws(11, [1.7, 0.3], LONG, FLAT),
                ws(3, [0.2, 1.0], LONG, FLAT),
                ws(4, [1.2, 1.0], LONG, FLAT),
                ws(5, [1.7, 1.0], LONG, FLAT),
                ws(9, [2.2, 1.0], LONG, FLAT),
                ws(50, [3.0, 0.3], LONG, FLAT),
            ],
            "skeleton": {"points": [[0.0, 0.0, 0.5]]},
            "focus": {"id": 3, "path": [[0.2, 1.0, 0.1]]},
        });
        let after = json!({
            "board": {"cx": 1.25, "cy": 0.65, "theta": 0.05, "hw": 1.3, "hh": 0.8},
            "stones": [
                ws(3, [0.7, 1.0], LONG, FLAT),
                ws(14, [1.2, 1.0], LONG, FLAT),
                ws(5, [1.7, 1.0], LONG, FLAT),
                ws(6, [2.2, 0.3], LONG, FLAT),
                ws(7, [1.2, 0.3], LONG, FLAT),
                ws(51, [2.2, 1.0], LONG, FLAT),
            ],
        });
        (serde_json::from_value(before).unwrap(), serde_json::from_value(after).unwrap())
    }

    fn mark(world: i64, role: ReportRole, certain: bool, label: &str) -> ReportMark {
        ReportMark { stone: sid(world), role, certain, label: label.to_string() }
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        theme::apply(&ctx, Brightness::MediumDark, Accent::Copper);
        ctx
    }

    #[test]
    fn the_marks_of_a_tray_by_world_id() {
        assert_eq!(report_marks(&Tray { leaving: None, open: None, ..tray3() }), None, "still on: no report");
        let marks = report_marks(&tray3()).unwrap();
        use ReportRole::*;
        let before: Vec<(i64, ReportMark)> = marks.before.into_iter().collect();
        assert_eq!(
            before,
            [
                (1, mark(1, Removed, true, "1")),
                (2, mark(2, Removed, false, "2")),
                (3, mark(3, Moved, true, "3")),
                (4, mark(4, Sure, true, "")),
                (5, mark(5, Unsure, true, "")),
                (9, mark(9, Sure, true, "")),
                (10, mark(10, Removed, true, "10")),
                (11, mark(11, Removed, true, "11")),
            ]
        );
        // as it left: 4 by the world id it stands for then, 9 without one left out, 8 (taken again) on neither side
        let after: Vec<(i64, ReportMark)> = marks.after.into_iter().collect();
        assert_eq!(
            after,
            [
                (3, mark(3, Moved, true, "3")),
                (5, mark(5, Unsure, true, "")),
                (6, mark(6, Added, true, "6")),
                (7, mark(7, Added, false, "7")),
                (14, mark(4, Sure, true, "")),
            ]
        );
        assert_eq!(mark(7, Added, false, "7").fill(), Added.color().gamma_multiply(0.45), "unclear: translucent");
        assert_eq!(mark(6, Added, true, "6").fill(), Added.color());
        // the id after the tray's number: the world id, and -2, -3, ... for a second, third stone first seen under it
        // in one tenth of a second; a stamp without a clock may hold a dash (a negative media time)
        assert_eq!(id_tail("20261122T023847.0Z-2-84", 2), "84");
        assert_eq!(id_tail("20261118T000010.0Z-1-1-2", 1), "1-2");
        assert_eq!(id_tail("s7t-1.0-1-84-3", 1), "84-3");
        assert_eq!(first_world_id("20261118T000010.0Z-1-1-2", 1), Some(1));
        assert_eq!(first_world_id("s7t-1.0-1-84", 1), Some(84));
        assert_eq!(id_tail("plain", 1), "plain");
        assert_eq!(first_world_id("plain", 1), None);
    }

    #[test]
    fn a_second_stone_first_seen_under_a_world_id_is_labelled_with_its_suffix() {
        // tray 3's unclear addition 7 as the second stone of the tray first seen under world 7 in one tenth (spec 4.6)
        let mut tray = tray3();
        let twin = format!("{}-2", sid(7));
        for s in tray.stones.iter_mut().filter(|s| s.id == sid(7)) {
            s.id.clone_from(&twin);
        }
        for f in tray.facts.iter_mut().filter(|f| f.stone == Some(sid(7))) {
            f.stone = Some(twin.clone());
        }
        let marks = report_marks(&tray).unwrap();
        let label = "7-2".to_string();
        assert_eq!(
            marks.after.get(&7),
            Some(&ReportMark { stone: twin, role: ReportRole::Added, certain: false, label })
        );
    }

    #[test]
    fn the_colours_are_the_typescript_sdks() {
        assert_eq!(REPORT_COLORS.map(|(role, _)| role), ReportRole::ALL);
        for (role, color) in REPORT_COLORS {
            assert_eq!(parse_color(color), Some(role.color()), "{role:?}");
        }
        let labels = ReportRole::ALL.map(ReportRole::label);
        assert_eq!(labels, ["removed", "added", "moved", "sure", "not sure"]);
        assert_eq!(Key::ALL.map(Key::text)[5..], ["unclear", "where a removed stone was", "not on the tray"]);
    }

    #[test]
    fn leaving_stones_come_from_the_account_even_when_the_frame_omits_them() {
        let (mut before, mut after) = worlds();
        let mut tray = tray3();
        let moved = tray.stones.iter_mut().find(|s| s.id == sid(3)).unwrap();
        moved.place = [3.0, 3.0]; // last sight after the leaving point
        moved.size = [0.5, 0.1];
        let original = before.stones.iter_mut().find(|s| s.id == Some(3)).unwrap();
        original.q = TURNED.to_vec();
        original.dims = vec![0.2, 0.4, 0.1];
        // 3 can borrow its arrival solid, 7 was added and has none. 51 is an extra detection.
        after.stones.retain(|s| !matches!(s.id, Some(3 | 7)));
        let view = ReportView::new(&tray, &before, &after).unwrap();
        let boxes = view.after.stones.iter().map(|s| (s.id.unwrap(), s)).collect::<BTreeMap<_, _>>();
        assert_eq!(boxes.keys().copied().collect::<Vec<_>>(), [3, 5, 6, 7, 14]);
        assert_eq!(boxes[&3].pos, [0.7, 1.0, 0.05]);
        assert_eq!(boxes[&3].dims, [0.1, 0.5, 0.1]);
        assert_eq!(boxes[&3].q, TURNED);
        assert_eq!(boxes[&7].dims, [0.4, 0.2, 0.06]);
        assert_eq!(boxes[&7].pos, [1.2, 0.3, 0.03]);
    }

    #[test]
    fn a_view_of_tray_3() {
        let (before, after) = worlds();
        assert!(ReportView::new(&Tray { leaving: None, ..tray3() }, &before, &after).is_none());
        let view = ReportView::new(&tray3(), &before, &after).unwrap();
        assert_eq!(
            view.report().describe().lines().next(),
            Some("tray 3, 0:01.0 to 0:09.0: 5 removed, 3 added, 1 moved, not complete: 3 unclear, 1 not sure")
        );
        assert_eq!(view.marks(), &report_marks(&tray3()).unwrap());
        assert_eq!(view.text, view.report().describe());
        // the worlds without their skeleton and focus
        assert!(view.before.focus.is_none() && view.before.skeleton == Skeleton3D::default());
        assert_eq!(view.before.stones, before.stones);
        // where the removed stones were, in the report's order (1, 11, 10, 8, 2): 1 lay turned and 2 with its longer
        // second side, upright; 11 lengthwise; 10 and 8 were not in the world as the tray arrived: lengthwise
        let removed: Vec<String> = view.report().removed.iter().map(|c| c.stone.clone()).collect();
        assert_eq!(removed, [1, 11, 10, 8, 2].map(sid));
        let round = |v: f32| (v * 1000.0).round() / 1000.0;
        let places: Vec<[f32; 4]> =
            view.places.iter().map(|(m, [hx, hy])| [round(m.x), round(m.y), round(*hx), round(*hy)]).collect();
        assert_eq!(
            places,
            [
                [0.2, 0.3, 0.1, 0.2],
                [1.7, 0.3, 0.2, 0.1],
                [1.2, 0.3, 0.2, 0.1],
                [2.2, 1.4, 0.2, 0.1],
                [0.7, 0.3, 0.1, 0.2],
            ]
        );
        // a stone without a size still has a (tiny) place
        let mut flat = tray3();
        flat.stones[0].size = [0.0, 0.0];
        let tiny = ReportView::new(&flat, &before, &after).unwrap();
        assert_eq!(tiny.places[0].1, [0.0005, 0.0005]);
    }

    #[test]
    fn the_sides_are_seen_from_straight_above_and_fit_their_views() {
        let (before, after) = worlds();
        // the before side runs from the board's left edge (-0.1) to the end of stone 50 off it (3.2); the board 1.6 deep
        let e = extent(&before);
        assert!((e.min.x + 0.1).abs() < 1e-5 && (e.max.x - 3.2).abs() < 1e-5, "{e:?}");
        assert!((e.min.y + 0.1).abs() < 1e-5 && (e.max.y - 1.5).abs() < 1e-5, "{e:?}");
        assert_eq!(extent(&Scene3D::default()), Rect::from_center_size(Pos2::ZERO, vec2(1.0, 1.0)), "nothing");
        let stones_only = Scene3D { board: None, ..before.clone() };
        // without a board: the stones' extent; 1 (turned) and 2 lie along y, 0.4 long
        let e = extent(&stones_only);
        assert!(e.min.x.abs() < 1e-5 && (e.min.y - 0.1).abs() < 1e-5 && (e.max.y - 1.1).abs() < 1e-5, "{e:?}");
        let marks = report_marks(&tray3()).unwrap();
        for (scene, marks) in [(&before, &marks.before), (&after, &marks.after)] {
            for view in [
                Rect::from_min_size(pos2(10.0, 20.0), vec2(300.0, 200.0)),
                Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 400.0)),
            ] {
                let side = Side::new(scene, marks, view);
                assert_eq!(side.camera.fov_y, FOV);
                // every corner of every stone inside the view, the outermost with a little room
                let corners: Vec<Pos2> =
                    scene.stones.iter().filter_map(box_corners).flatten().map(|c| side.at(c).unwrap()).collect();
                let inside = view.shrink(view.width().min(view.height()) * 0.005);
                assert!(corners.iter().all(|p| inside.contains(*p)), "{view:?}");
                // the extent (board and stones) fills the view's width or height, but for a little room
                let e = extent(scene);
                let span = Rect::from_two_pos(
                    side.at([e.min.x, e.min.y, 0.0]).unwrap(),
                    side.at([e.max.x, e.max.y, 0.0]).unwrap(),
                );
                let fill = (span.width() / view.width()).max(span.height() / view.height());
                assert!((fill - FILL).abs() < 1e-3, "{span:?} in {view:?}");
                // scene x to the right, y up
                assert!(side.at([1.0, 0.0, 0.0]).unwrap().x > side.at([0.0, 0.0, 0.0]).unwrap().x);
                assert!(side.at([0.0, 1.0, 0.0]).unwrap().y < side.at([0.0, 0.0, 0.0]).unwrap().y);
            }
        }
        let close = camera(
            Rect::from_center_size(Pos2::ZERO, vec2(0.001, 0.001)),
            Rect::from_min_size(Pos2::ZERO, vec2(50.0, 50.0)),
        );
        assert_eq!(close.distance, 0.05, "never closer");
        let mut lone = before.stones[2].clone();
        assert!(!lies_along_y(&lone));
        lone.dims.clear();
        assert!(!lies_along_y(&lone), "no size: lengthwise");
    }

    #[test]
    fn labels_sit_on_the_stones_and_the_places_on_the_side_as_it_left() {
        let (before, after) = worlds();
        let marks = report_marks(&tray3()).unwrap();
        let view = Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0));
        let first = Side::new(&before, &marks.before, view);
        let texts = |side: &Side| side.labels().iter().map(|(m, _)| (m.label.clone(), m.role)).collect::<Vec<_>>();
        use ReportRole::*;
        // 10 was not in the world as the tray arrived: no stone, no label
        let want = [("1", Removed), ("2", Removed), ("11", Removed), ("3", Moved)].map(|(l, r)| (l.to_string(), r));
        assert_eq!(texts(&first), want);
        let second = Side::new(&after, &marks.after, view);
        assert_eq!(texts(&second), [("3", Moved), ("6", Added), ("7", Added)].map(|(l, r)| (l.to_string(), r)));
        // at the middle of the stone's top
        let (_, top) = first.labels()[0];
        assert!(top.distance(first.at([0.2, 0.3, 0.1]).unwrap()) < 0.01);
        // a removed stone's place: its outline on the tray, axis-aligned
        let outline = second.place((pos2(0.2, 0.3), [0.1, 0.2])).unwrap();
        assert!(outline[0].distance(second.at([0.1, 0.1, 0.002]).unwrap()) < 0.01);
        assert!(outline[2].distance(second.at([0.3, 0.5, 0.002]).unwrap()) < 0.01);
        // a stone whose box does not decode has no label
        let mut odd = after.clone();
        odd.stones[3].pos.clear();
        assert_eq!(
            texts(&Side::new(&odd, &marks.after, view)),
            [("3", Moved), ("7", Added)].map(|(l, r)| (l.to_string(), r))
        );
    }

    /// The share of the pixels in `area` (in points, at 1 pixel per point) whose colour is near `want`.
    fn share(shot: &Snapshot, area: Rect, want: Color32) -> f32 {
        let (mut near, mut all) = (0, 0);
        for y in area.min.y as usize..area.max.y as usize {
            for x in area.min.x as usize..area.max.x as usize {
                let [r, g, b, _] = shot.pixel(x, y);
                all += 1;
                if r.abs_diff(want.r()) < 24 && g.abs_diff(want.g()) < 24 && b.abs_diff(want.b()) < 24 {
                    near += 1;
                }
            }
        }
        near as f32 / all.max(1) as f32
    }

    /// The colour of a stone's top: its fill, lit from above.
    fn lit(color: Color32) -> Color32 {
        let shade = 0.55 + 0.45 / (0.3f32 * 0.3 + 0.5 * 0.5 + 1.0).sqrt();
        Color32::from_rgb(
            (f32::from(color.r()) * shade) as u8,
            (f32::from(color.g()) * shade) as u8,
            (f32::from(color.b()) * shade) as u8,
        )
    }

    #[test]
    fn the_view_lays_out_its_parts() {
        let (before, after) = worlds();
        let view = ReportView::new(&tray3(), &before, &after).unwrap();
        let ctx = context();
        let mut layouts = Vec::new();
        render(&ctx, vec2(900.0, 450.0), 1.0, 2, |ui| {
            layouts.clear();
            for size in [vec2(900.0, 450.0), vec2(400.0, 500.0), vec2(3000.0, 400.0), vec2(450.0, 500.0)] {
                layouts.push(view.layout(ui.painter(), Rect::from_min_size(pos2(0.0, 0.0), size)));
            }
        });
        let [wide, narrow, long, roomy] = [&layouts[0], &layouts[1], &layouts[2], &layouts[3]];
        // wide: the text in a column on the right, 0.4 of the width (at most 520 points), the sides on the left
        assert_eq!(wide.text, Rect::from_min_max(pos2(540.0, 0.0), pos2(900.0, 450.0)));
        assert_eq!(long.text.width(), 520.0);
        assert_eq!(wide.titles[0], Rect::from_min_size(pos2(0.0, 0.0), vec2(260.0, TITLE_H)));
        assert_eq!(wide.views[1].min, pos2(270.0, TITLE_H));
        assert_eq!(wide.views[1].max.x, 530.0);
        // the legend under the sides, every key once, in rows inside the left part
        assert_eq!(wide.legend.iter().map(|(k, _)| *k).collect::<Vec<_>>(), Key::ALL);
        assert!(wide.legend.iter().all(|(_, at)| at.x >= 0.0 && at.x < 530.0 && at.y > wide.views[0].max.y));
        assert!(wide.legend.last().unwrap().1.y > wide.legend[0].1.y, "two rows");
        assert!((8.0..=11.5).contains(&wide.size));
        // narrow: the text under the legend, across the width, at most 0.4 of the height
        assert_eq!((narrow.text.min.x, narrow.text.width()), (0.0, 400.0));
        assert!(narrow.text.height() <= 200.0 && narrow.text.max.y == 500.0);
        assert!(narrow.views[0].max.y < narrow.text.min.y && narrow.views[1].max.x == 400.0);
        // the font that fits tray 3's longest change line, `  0:09.0 removed s7t1.0-3-8 from (2.20, 1.40) (unclear,
        // 0.58)` (61 characters, 367.25 points wide at 10 points), into 400 points; 450 points take it at the largest
        assert_eq!(narrow.size, 10.0 * 400.0 / 367.25);
        assert_eq!(roomy.size, 11.5, "the largest font: tray 3's lines fit");
    }

    #[test]
    fn the_view_paints_the_stones_by_their_marks() {
        let (before, after) = worlds();
        let view = ReportView::new(&tray3(), &before, &after).unwrap();
        let ctx = context();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(1800.0, 900.0));
        let mut layout = None;
        let shot = render(&ctx, rect.size(), 1.0, 2, |ui| {
            layout = Some(view.layout(ui.painter(), rect));
            view.show(ui);
        });
        let layout = layout.unwrap();
        let [first, second] = view.sides(&layout);
        let around =
            |side: &Side, [x, y]: [f32; 2]| Rect::from_center_size(side.at([x, y, 0.1]).unwrap(), vec2(6.0, 6.0));
        use ReportRole::*;
        // the stones without a label, at their middles; the labelled ones off their labels
        assert!(share(&shot, around(&first, [1.2, 1.0]), lit(Sure.color())) > 0.9);
        assert!(share(&shot, around(&first, [1.7, 1.0]), lit(Unsure.color())) > 0.9);
        assert!(share(&shot, around(&first, [0.2, 0.42]), lit(Removed.color())) > 0.9, "1, turned");
        assert!(share(&shot, around(&first, [1.85, 0.3]), lit(Removed.color())) > 0.9);
        assert!(share(&shot, around(&second, [2.33, 0.3]), lit(Added.color())) > 0.9, "6, for certain");
        assert!(share(&shot, around(&second, [0.83, 1.0]), lit(Moved.color())) > 0.9);
        assert!(share(&shot, around(&second, [1.2, 1.0]), lit(Sure.color())) > 0.9, "4, as world stone 14");
        // 7 is translucent (an unclear addition); the extra world stone 51 is absent
        assert!(share(&shot, around(&second, [1.33, 0.3]), lit(Added.color())) < 0.1);
        assert!(share(&shot, around(&second, [2.2, 1.0]), lit(DIM)) < 0.1);
        // the place 8 was taken from (its lower edge), dashed red, on the side as it left only
        let edge = |side: &Side| {
            let a = side.at([2.0, 1.3, 0.002]).unwrap();
            let b = side.at([2.4, 1.3, 0.002]).unwrap();
            Rect::from_min_max(pos2(a.x, a.y - 2.0), pos2(b.x, b.y + 2.0))
        };
        assert!(share(&shot, edge(&second), Removed.color()) > 0.2);
        assert!(share(&shot, edge(&first), Removed.color()) < 0.05);
        // painted with a painter alone: the same sides, the text cut to its rectangle
        let painted = render(&ctx, rect.size(), 1.0, 2, |ui| view.paint(ui.painter(), rect));
        assert!(share(&painted, around(&first, [1.2, 1.0]), lit(Sure.color())) > 0.9);
        assert!(share(&painted, around(&second, [2.33, 0.3]), lit(Added.color())) > 0.9);
    }

    #[test]
    fn a_side_without_a_world_says_so() {
        let empty = Scene3D::default();
        let view = ReportView::new(&tray3(), &empty, &empty).unwrap();
        let ctx = context();
        let shot = render(&ctx, vec2(600.0, 300.0), 1.0, 2, |ui| drop(view.show(ui)));
        assert_eq!((shot.width, shot.height), (600, 300));
        // a view too small for its sides paints what fits
        render(&ctx, vec2(40.0, 30.0), 1.0, 2, |ui| drop(view.show(ui)));
        // an unclear addition whose raw box does not decode still has a place in the account, as it left; an unclear
        // removal whose raw box does not decode, as it arrived, goes without its dashed outline
        let (mut before, mut after) = worlds();
        after.stones[4].pos.clear();
        before.stones[1].pos.clear();
        let odd = ReportView::new(&tray3(), &before, &after).unwrap();
        assert!(odd.after.stones.iter().any(|s| s.id == Some(7) && s.pos == [1.2, 0.3, 0.03]));
        let shot = render(&ctx, vec2(600.0, 300.0), 1.0, 2, |ui| drop(odd.show(ui)));
        assert_eq!((shot.width, shot.height), (600, 300));
    }

    /// The fixture recording's tray 1 (dry1 from 0:22.5 to 2:24.0), as docs/img/reports-dry1-0018-0153.png
    /// shows it.
    #[test]
    fn the_report_of_dry1s_tray_1() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/recordings/dry1-0018-0153.ndjson.gz");
        let tray = Replay::open(path).unwrap().trays().unwrap().into_iter().find(|t| t.n == 1).unwrap();
        let (arrived, leaving) = (tray.arrived.unwrap().g, tray.leaving.unwrap().g);
        let scene = |g: i64| {
            let frame = Replay::open(path).unwrap().frames().map(Result::unwrap).find(|f| f.g == g).unwrap();
            scene_of_frame(&frame).unwrap()
        };
        let (before, after) = (scene(arrived), scene(leaving));
        let view = ReportView::new(&tray, &before, &after).unwrap();
        let roles = |marks: &BTreeMap<i64, ReportMark>| {
            let mut counts = BTreeMap::new();
            for m in marks.values() {
                *counts.entry((m.role, m.certain)).or_insert(0) += 1;
            }
            counts.into_iter().collect::<Vec<_>>()
        };
        use ReportRole::*;
        assert_eq!(
            roles(&view.marks().before),
            [((Removed, false), 5), ((Removed, true), 6), ((Moved, true), 8), ((Sure, true), 22), ((Unsure, true), 1)]
        );
        assert_eq!(
            roles(&view.marks().after),
            [((Added, false), 13), ((Moved, true), 8), ((Sure, true), 22), ((Unsure, true), 1)]
        );
        let layout = Layout {
            titles: [Rect::NOTHING; 2],
            views: [Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0)); 2],
            legend: Vec::new(),
            text: Rect::NOTHING,
            size: 10.0,
        };
        let [first, second] = view.sides(&layout);
        let labels = |side: &Side| {
            let mut l: Vec<i64> = side.labels().iter().map(|(m, _)| m.label.parse().unwrap()).collect();
            l.sort();
            l
        };
        assert_eq!(
            labels(&first),
            [85, 86, 87, 88, 89, 90, 104, 105, 112, 117, 118, 120, 121, 125, 126, 127, 128, 129, 130]
        );
        assert_eq!(
            labels(&second),
            [85, 86, 88, 89, 90, 104, 118, 120, 151, 177, 193, 201, 203, 246, 249, 278, 279, 280, 281, 282, 284]
        );
        // This tray's 44 final stones include 230 and 238, seen on the frame after the departure, and 240, seen at it,
        // where the leaving's speckle answer finds 119 and 120 again and recognises 116, and 281, a box the world kept
        // at it although the model had not boxed it yet, of unclear origin.
        assert_eq!(first.marked().count(), before.stones.len());
        let unmarked: Vec<i64> =
            after.stones.iter().filter_map(|s| s.id).filter(|id| !view.marks().after.contains_key(id)).collect();
        assert_eq!(unmarked, Vec::<i64>::new());
        assert_eq!(second.marked().count(), 44);
        assert_eq!(view.after.stones.len(), 44);
        // where the removed stones were (117, 127, 126, 105, 112, 125, then the unclear 87, 121, 128, 129, 130): 126,
        // 129 and 130 lay along y
        let upright: Vec<bool> = view.places.iter().map(|(_, [hx, hy])| hy > hx).collect();
        let along_y = [false, false, true, false, false, false, false, false, false, true, true];
        assert_eq!(upright, along_y);
    }
}
