//! Decoders of the wire's compact encodings (spec 7): maps, bitsets, depth, surface buffers,
//! joints and stone tuples, and the coordinate helpers.
//!
//! Every decoder takes the field as the models hold it (`Option<&str>` for the fields the public
//! recordings may redact) and returns `Ok(None)` for an absent field.

use std::io::Read;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::error::{Error, ErrorCode, Result};
use crate::models::{Side, Topology};

fn b64(data: &str, what: &str) -> Result<Vec<u8>> {
    STANDARD.decode(data.trim()).map_err(|e| Error::new(ErrorCode::Decode, format!("{what}: not base64: {e}")))
}

/// A row-major `rows × cols` map of bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Grid {
    /// Rows.
    pub rows: usize,
    /// Columns.
    pub cols: usize,
    /// `rows * cols` bytes, row-major.
    pub data: Vec<u8>,
}

impl Grid {
    /// The byte at `row`, `col` (`None` outside the grid).
    pub fn at(&self, row: usize, col: usize) -> Option<u8> {
        (row < self.rows && col < self.cols).then(|| self.data[row * self.cols + col])
    }

    /// One row.
    pub fn row(&self, row: usize) -> &[u8] {
        let start = (row * self.cols).min(self.data.len());
        &self.data[start..(start + self.cols).min(self.data.len())]
    }

    /// The number of cells equal to `value`, or of non-zero cells for `None`.
    pub fn count(&self, value: Option<u8>) -> usize {
        match value {
            Some(v) => self.data.iter().filter(|&&b| b == v).count(),
            None => self.data.iter().filter(|&&b| b != 0).count(),
        }
    }
}

fn grid(data: Vec<u8>, rows: usize, cols: usize, what: &str) -> Result<Grid> {
    if data.len() != rows * cols {
        return Err(Error::new(ErrorCode::Decode, format!("{what}: {} bytes, expected {rows} x {cols}", data.len())));
    }
    Ok(Grid { rows, cols, data })
}

/// An overlay map: base64 of `rows * cols` bytes.
pub fn decode_u8_map(data: Option<&str>, rows: usize, cols: usize) -> Result<Option<Grid>> {
    data.map(|d| grid(b64(d, "u8 map")?, rows, cols, "u8 map")).transpose()
}

/// A `sip1` map: base64 of zlib of `rows * cols` bytes.
pub fn decode_zlib_map(data: Option<&str>, rows: usize, cols: usize) -> Result<Option<Grid>> {
    let Some(data) = data else { return Ok(None) };
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(b64(data, "zlib map")?.as_slice())
        .read_to_end(&mut raw)
        .map_err(|e| Error::new(ErrorCode::Decode, format!("zlib map: {e}")))?;
    grid(raw, rows, cols, "zlib map").map(Some)
}

/// `n` booleans, least significant bit first within each byte.
pub fn decode_bitset(data: Option<&str>, n: usize) -> Result<Option<Vec<bool>>> {
    let Some(data) = data else { return Ok(None) };
    let raw = b64(data, "bitset")?;
    if raw.len() * 8 < n {
        return Err(Error::new(ErrorCode::Decode, format!("bitset: {} bytes for {n} bits", raw.len())));
    }
    Ok(Some((0..n).map(|i| raw[i >> 3] >> (i & 7) & 1 == 1).collect()))
}

/// The log depth of a quantised depth value: `a * (lo + q / 255 * (hi - lo)) + b`.
pub fn depth_log_z(q: f64, lo: f64, hi: f64, a: f64, b: f64) -> f64 {
    a * (lo + q / 255.0 * (hi - lo)) + b
}

/// The depth: `exp(depth_log_z(..))`.
pub fn depth_z(q: f64, lo: f64, hi: f64, a: f64, b: f64) -> f64 {
    depth_log_z(q, lo, hi, a, b).exp()
}

/// The depth of every cell of a quantised depth map.
pub fn decode_depth(map: Option<&Grid>, lo: f64, hi: f64, a: f64, b: f64) -> Option<Vec<f64>> {
    let table: Vec<f64> = (0..=255).map(|q| depth_z(f64::from(q), lo, hi, a, b)).collect();
    map.map(|m| m.data.iter().map(|&q| table[usize::from(q)]).collect())
}

/// Little-endian f32s (surface positions, `x, y, z` per vertex).
pub fn decode_f32le(data: Option<&str>) -> Result<Option<Vec<f32>>> {
    let Some(data) = data else { return Ok(None) };
    let raw = b64(data, "f32 buffer")?;
    if raw.len() % 4 != 0 {
        return Err(Error::new(ErrorCode::Decode, format!("f32 buffer: {} bytes", raw.len())));
    }
    Ok(Some(raw.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()))
}

/// Little-endian u16s (triangle indices).
pub fn decode_u16le(data: Option<&str>) -> Result<Option<Vec<u16>>> {
    let Some(data) = data else { return Ok(None) };
    let raw = b64(data, "u16 buffer")?;
    if raw.len() % 2 != 0 {
        return Err(Error::new(ErrorCode::Decode, format!("u16 buffer: {} bytes", raw.len())));
    }
    Ok(Some(raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()))
}

/// A skeleton joint `[u, v, vis, x, y, z]`: image position (0..1), visibility and camera-space
/// position.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(missing_docs)]
pub struct Joint {
    pub u: f64,
    pub v: f64,
    pub vis: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Joint {
    /// A joint from its tuple (`None` for null or fewer than 6 numbers).
    pub fn from_values(values: Option<&[f64]>) -> Option<Joint> {
        match values? {
            [u, v, vis, x, y, z, ..] => Some(Joint { u: *u, v: *v, vis: *vis, x: *x, y: *y, z: *z }),
            _ => None,
        }
    }
}

/// All joints of a skeleton (`raw.person.joints`, `overlay.joints`).
pub fn joints(values: &[Option<Vec<f64>>]) -> Vec<Option<Joint>> {
    values.iter().map(|v| Joint::from_values(v.as_deref())).collect()
}

/// A stone detection of `sip1` `stones.items`: `[u, v, w, h, score, X, Y, Z, w_m, h_m, yaw]`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(missing_docs)]
pub struct StoneItem {
    pub u: f64,
    pub v: f64,
    pub w: f64,
    pub h: f64,
    pub score: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub w_m: f64,
    pub h_m: f64,
    pub yaw: f64,
}

impl StoneItem {
    /// An item from its tuple (`None` for fewer than 11 numbers).
    pub fn from_values(values: &[f64]) -> Option<StoneItem> {
        match values {
            [u, v, w, h, score, x, y, z, w_m, h_m, yaw, ..] => Some(StoneItem {
                u: *u,
                v: *v,
                w: *w,
                h: *h,
                score: *score,
                x: *x,
                y: *y,
                z: *z,
                w_m: *w_m,
                h_m: *h_m,
                yaw: *yaw,
            }),
            _ => None,
        }
    }
}

/// All items (malformed ones are skipped).
pub fn stone_items(values: &[Vec<f64>]) -> Vec<StoneItem> {
    values.iter().filter_map(|v| StoneItem::from_values(v)).collect()
}

/// Normalised image coordinates to pixels of a `width × height` image (`px = u * W - 0.5`).
pub fn uv_to_px(u: f64, v: f64, width: f64, height: f64) -> (f64, f64) {
    (u * width - 0.5, v * height - 0.5)
}

/// Pixels to normalised image coordinates.
pub fn px_to_uv(px: f64, py: f64, width: f64, height: f64) -> (f64, f64) {
    ((px + 0.5) / width, (py + 0.5) / height)
}

/// Scene (z up) to three.js (y up) axes: `[x, y, z]` → `[x, z, -y]`.
pub fn scene_to_three(p: [f64; 3]) -> [f64; 3] {
    [p[0], p[2], -p[1]]
}

/// three.js to scene axes.
pub fn three_to_scene(p: [f64; 3]) -> [f64; 3] {
    [p[0], -p[2], p[1]]
}

/// The yaw (about z) of a quaternion `[x, y, z, w]`.
pub fn quat_to_yaw(q: [f64; 4]) -> f64 {
    let [x, y, z, w] = q;
    (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
}

/// Skeleton topology helpers from `getTopology`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Skeleton {
    topology: Topology,
}

impl Skeleton {
    /// The helpers of `topology`.
    pub fn new(topology: Topology) -> Skeleton {
        Skeleton { topology }
    }

    /// The wrist joint of a hand.
    pub fn wrist_joint(&self, side: &Side) -> Option<usize> {
        self.topology.wrist.get(side).copied()
    }

    /// The finger-tip joints of a hand.
    pub fn tip_joints(&self, side: &Side) -> Vec<usize> {
        self.topology.tips.get(side).cloned().unwrap_or_default()
    }

    /// The bones to draw for `joints`: shown bones whose two joints are present.
    pub fn visible_bones<T>(&self, joints: &[Option<T>]) -> Vec<[usize; 2]> {
        let present = |i: usize| joints.get(i).is_some_and(Option::is_some);
        self.topology.visible_bones().into_iter().filter(|&[a, b]| present(a) && present(b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn enc(bytes: &[u8]) -> String {
        STANDARD.encode(bytes)
    }

    #[test]
    fn maps_and_bitsets() {
        let g = decode_u8_map(Some(&enc(&[0, 1, 2, 0, 5, 1])), 2, 3).unwrap().unwrap();
        assert_eq!(g.at(1, 1), Some(5));
        assert_eq!(g.at(2, 0), None);
        assert_eq!(g.at(0, 3), None);
        assert_eq!(g.row(1), &[0, 5, 1]);
        assert_eq!(g.row(9), &[] as &[u8]);
        assert_eq!(g.count(None), 4);
        assert_eq!(g.count(Some(1)), 2);
        assert!(decode_u8_map(None, 2, 3).unwrap().is_none());
        assert!(decode_u8_map(Some("!!"), 2, 3).is_err());
        assert!(decode_u8_map(Some(&enc(&[1])), 2, 3).is_err());
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&[7; 6]).unwrap();
        let zlib = enc(&z.finish().unwrap());
        assert_eq!(decode_zlib_map(Some(&zlib), 3, 2).unwrap().unwrap().count(Some(7)), 6);
        assert!(decode_zlib_map(None, 3, 2).unwrap().is_none());
        assert!(decode_zlib_map(Some(&enc(b"not zlib")), 3, 2).is_err());
        assert_eq!(
            decode_bitset(Some(&enc(&[0b0000_0101, 0b1])), 10).unwrap().unwrap(),
            vec![true, false, true, false, false, false, false, false, true, false]
        );
        assert!(decode_bitset(None, 3).unwrap().is_none());
        assert!(decode_bitset(Some(&enc(&[1])), 9).is_err());
    }

    #[test]
    fn depth_and_buffers() {
        assert_eq!(depth_log_z(255.0, 0.0, 2.0, 1.0, 0.5), 2.5);
        assert!((depth_z(0.0, 0.0, 1.0, 1.0, 0.0) - 1.0).abs() < 1e-12);
        let m = Grid { rows: 1, cols: 2, data: vec![0, 255] };
        let d = decode_depth(Some(&m), 0.0, 1.0, 1.0, 0.0).unwrap();
        assert!((d[1] - std::f64::consts::E).abs() < 1e-12);
        assert!(decode_depth(None, 0.0, 1.0, 1.0, 0.0).is_none());
        let f: Vec<u8> = [1.5f32, -2.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        assert_eq!(decode_f32le(Some(&enc(&f))).unwrap().unwrap(), vec![1.5, -2.0]);
        assert!(decode_f32le(None).unwrap().is_none());
        assert!(decode_f32le(Some(&enc(&[1, 2, 3]))).is_err());
        assert_eq!(decode_u16le(Some(&enc(&[1, 0, 0, 1]))).unwrap().unwrap(), vec![1, 256]);
        assert!(decode_u16le(None).unwrap().is_none());
        assert!(decode_u16le(Some(&enc(&[1]))).is_err());
    }

    #[test]
    fn tuples_and_coordinates() {
        let js = joints(&[Some(vec![0.5, 0.25, 0.9, 1.0, 2.0, 3.0]), None, Some(vec![1.0])]);
        assert_eq!(js[0].unwrap().z, 3.0);
        assert!(js[1].is_none() && js[2].is_none());
        let items = stone_items(&[(0..11).map(f64::from).collect(), vec![1.0]]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].yaw, 10.0);
        assert_eq!(uv_to_px(0.5, 0.25, 1920.0, 1080.0), (959.5, 269.5));
        assert_eq!(px_to_uv(959.5, 269.5, 1920.0, 1080.0), (0.5, 0.25));
        assert_eq!(scene_to_three([1.0, 2.0, 3.0]), [1.0, 3.0, -2.0]);
        assert_eq!(three_to_scene([1.0, 3.0, -2.0]), [1.0, 2.0, 3.0]);
        let half = std::f64::consts::FRAC_PI_4;
        assert!((quat_to_yaw([0.0, 0.0, half.sin(), half.cos()]) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn skeleton_helpers() {
        let t: Topology = serde_json::from_str(
            r#"{"never":[2],"wrist":{"left":0,"right":1},"tips":{"left":[3],"right":[4]},"bones":[[0,1],[1,2],[0,3]]}"#,
        )
        .unwrap();
        let s = Skeleton::new(t);
        assert_eq!(s.wrist_joint(&Side::Right), Some(1));
        assert_eq!(s.wrist_joint(&Side::Both), None);
        assert_eq!(s.tip_joints(&Side::Left), vec![3]);
        assert!(s.tip_joints(&Side::Both).is_empty());
        let present = [Some(()), Some(()), Some(()), None];
        assert_eq!(s.visible_bones(&present), vec![[0, 1]]);
    }
}
