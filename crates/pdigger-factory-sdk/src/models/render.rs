//! Render-ready draw lists: `/render/overlay2d` (`r2d1`, native px) and `/render/scene3d`
//! (`r3d1`, scene world).

use serde::{Deserialize, Serialize};

use super::{Extra, Rect, Ring, Side, StoneFamily};
use crate::wire::wire_enum;

wire_enum! {
    /// Which world a render shows.
    RenderMode {
        /// The persistent world (the default).
        Stable = "stable",
        /// Per-packet detections.
        Raw = "raw",
    }
}

wire_enum! {
    /// How a render is drawn.
    RenderDraw {
        /// Semantic objects (the default).
        Shapes = "shapes",
        /// Flat lists with resolved styles.
        Primitives = "primitives",
    }
}

wire_enum! {
    /// How the 3-D render sends the observed surface.
    RenderSurface {
        /// Explicit f32 positions and u16 indices (the default).
        Vertices = "vertices",
        /// Quantised log depth plus masks.
        Compact = "compact",
        /// Omit it.
        None = "none",
    }
}

wire_enum! {
    /// A frame raster (`/frames/{g}/{raster}.png`).
    Raster {
        /// Tray, stones and human composited.
        Regions = "regions",
        /// The person-crop label map.
        Mask = "mask",
        /// The depth map as grey.
        Depth = "depth",
    }
}

/// A stone of the 2-D draw list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawStone2D {
    /// Stone id (`None` for raw detections).
    pub id: Option<i64>,
    /// Outline, native px.
    pub outline: Vec<Vec<f64>>,
    /// `held`, `placed`, `model`, `detected`, `kept` or `raw`.
    pub style: String,
    /// The hand holding it.
    pub side: Option<Side>,
    /// Opacity.
    pub alpha: f64,
    /// Detected.
    pub detected: Option<bool>,
    /// Uncertain.
    pub uncertain: Option<bool>,
    /// Shape.
    pub shape: Option<String>,
    /// Mode.
    pub mode: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A filled or stroked polygon, native px.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Polygon2D {
    /// Vertices.
    pub points: Vec<Vec<f64>>,
    /// Stroke colour (CSS).
    pub stroke: Option<String>,
    /// Fill colour (CSS).
    pub fill: Option<String>,
    /// Fill opacity.
    pub fill_alpha: f64,
    /// Stroke width in line units.
    pub width: f64,
    /// Dash pattern.
    pub dash: Option<Vec<f64>>,
    /// Opacity.
    pub alpha: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A line segment, native px.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Segment2D {
    /// Start.
    pub from: Vec<f64>,
    /// End.
    pub to: Vec<f64>,
    /// Colour (CSS).
    pub color: String,
    /// Width in line units.
    pub width: f64,
    /// Opacity.
    pub alpha: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A circle, native px.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Circle2D {
    /// Centre.
    pub at: Vec<f64>,
    /// Radius in line units.
    pub r: f64,
    /// Stroke colour (CSS).
    pub stroke: Option<String>,
    /// Fill colour (CSS).
    pub fill: Option<String>,
    /// Opacity.
    pub alpha: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A text label, native px.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Label2D {
    /// Anchor.
    pub at: Vec<f64>,
    /// Text.
    pub text: String,
    /// Colour (CSS).
    pub color: String,
    /// Background (CSS).
    pub background: Option<String>,
    /// The canvas `textBaseline` that `at` is given for (`top`, `middle`).
    pub baseline: Option<String>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// An image (a frame raster) placed in native px.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Image2D {
    /// URL of the raster.
    pub url: String,
    /// Left.
    pub x: f64,
    /// Top.
    pub y: f64,
    /// Width.
    pub w: f64,
    /// Height.
    pub h: f64,
    /// Opacity.
    pub alpha: f64,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Flat draw lists in native px (`draw=primitives`); sizes are in line units `lw`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Primitives2D {
    /// The line unit: clamp(native width / 420, 1.2, 3).
    pub lw: Option<f64>,
    /// Images.
    pub images: Vec<Image2D>,
    /// Polygons.
    pub polygons: Vec<Polygon2D>,
    /// Segments.
    pub segments: Vec<Segment2D>,
    /// Circles.
    pub circles: Vec<Circle2D>,
    /// Labels.
    pub labels: Vec<Label2D>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The 2-D draw list, final, native px (`r2d1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay2D {
    /// `r2d1`.
    pub schema: String,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Packet a.
    pub g: i64,
    /// Packet b, or `None`.
    pub bg: Option<i64>,
    /// Fraction from a to b.
    pub f: f64,
    /// Media time.
    pub t: f64,
    /// `stable` or `raw`.
    pub mode: RenderMode,
    /// Missing layers.
    pub missing: Vec<String>,
    /// Native size `[w, h]`.
    pub native_wh: Vec<i64>,
    /// Region raster.
    pub regions: Option<serde_json::Value>,
    /// Mask raster.
    pub mask: Option<serde_json::Value>,
    /// Boxes.
    pub boxes: Vec<serde_json::Value>,
    /// Stones.
    pub stones: Vec<DrawStone2D>,
    /// Skeleton bones and joints.
    pub skeleton: Option<serde_json::Value>,
    /// Wrists.
    pub wrists: Vec<serde_json::Value>,
    /// The focus.
    pub focus: Option<serde_json::Value>,
    /// Flat draw lists (`draw=primitives`).
    pub primitives: Option<Primitives2D>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// A stone of the 3-D draw list.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DrawStone3D {
    /// Stone id.
    pub id: Option<i64>,
    /// `rect`, `dogbone` or `box`.
    pub family: StoneFamily,
    /// Centre.
    pub pos: Vec<f64>,
    /// Quaternion `[x, y, z, w]`.
    pub q: Vec<f64>,
    /// `[L, W, h]`.
    pub dims: Vec<f64>,
    /// The hand holding it.
    pub side: Option<Side>,
    /// Opacity.
    pub alpha: f64,
    /// Uncertain.
    pub uncertain: Option<bool>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The observed surface of the 3-D render.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SurfaceMesh {
    /// `vertices` or `compact`.
    pub form: RenderSurface,
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// base64 f32le `x, y, z` per vertex (world), `vertices` form.
    pub positions: Option<String>,
    /// base64 u16le triangle list, `vertices` form.
    pub indices: Option<String>,
    /// Opacity.
    pub opacity: Option<f64>,
    /// Unknown fields (the `compact` form's `q`, `lo`, `hi`, `a`, `b`, `flat`, `quads`).
    #[serde(flatten)]
    pub extra: Extra,
}

/// The skeleton of the 3-D render.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Skeleton3D {
    /// 70 joints `[x, y, z]` or null.
    pub joints: Vec<Option<Vec<f64>>>,
    /// Bones (`a`, `b`, `side`, `hand`).
    pub bones: Vec<serde_json::Value>,
    /// Wrist rings.
    pub rings: Vec<Ring>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// Flat 3-D draw lists.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Primitives3D {
    /// Boxes.
    pub boxes: Vec<serde_json::Value>,
    /// Segments.
    pub segments: Vec<serde_json::Value>,
    /// Spheres.
    pub spheres: Vec<serde_json::Value>,
    /// Mesh.
    pub mesh: Option<serde_json::Value>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}

/// The 3-D scene, final, scene world (`r3d1`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scene3D {
    /// `r3d1`.
    pub schema: String,
    /// Session.
    pub session: i64,
    /// Video.
    pub video: String,
    /// Packet a.
    pub g: i64,
    /// Packet b.
    pub bg: Option<i64>,
    /// Fraction from a to b.
    pub f: f64,
    /// Media time.
    pub t: f64,
    /// `stable` or `raw`.
    pub mode: RenderMode,
    /// Missing layers.
    pub missing: Vec<String>,
    /// The source camera (`K`, `R_c2w`, `t_c2w`, `native_wh`, `centre`).
    pub camera: Option<serde_json::Value>,
    /// The tray plane.
    pub plane: Option<serde_json::Value>,
    /// The board.
    pub board: Option<Rect>,
    /// Stones.
    pub stones: Vec<DrawStone3D>,
    /// Skeleton.
    pub skeleton: Skeleton3D,
    /// The observed surface.
    pub surface: Option<SurfaceMesh>,
    /// The focus.
    pub focus: Option<serde_json::Value>,
    /// Flat lists.
    pub primitives: Option<Primitives3D>,
    /// Unknown fields.
    #[serde(flatten)]
    pub extra: Extra,
}
