//! Headless rendering: a UI run in an egui context, tessellated by egui and rasterised in
//! software to RGBA (and PNG). No window and no GPU: this is how the screenshots in `docs/img`
//! are made, and how the tests look at what the widgets paint.

use std::collections::HashMap;
use std::io::Write;

use egui::epaint::{ClippedPrimitive, Primitive, Vertex};
use egui::{Color32, ColorImage, ImageData, Pos2, RawInput, Rect, TextureId, Ui, Vec2};

/// An RGBA image (premultiplied alpha), row by row.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl Snapshot {
    /// The pixel at (`x`, `y`), `[r, g, b, a]` premultiplied.
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// How many pixels differ from `color` by more than `tolerance` in any channel.
    pub fn count_not(&self, color: [u8; 4], tolerance: u8) -> usize {
        self.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p.iter().zip(color).any(|(a, b)| a.abs_diff(b) > tolerance))
            .count()
    }

    /// The image as a PNG file (straight alpha, 8-bit RGBA).
    pub fn to_png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.width * 4 + 1) * self.height);
        for row in self.rgba.chunks_exact(self.width * 4) {
            raw.push(0);
            for p in row.as_chunks::<4>().0 {
                let a = p[3];
                let un = |c: u8| {
                    if a == 0 { 0 } else { ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8 }
                };
                raw.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), a]);
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&raw).expect("writing to memory");
        let idat = z.finish().expect("writing to memory");
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(self.width as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.height as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [(b"IHDR", &ihdr), (b"IDAT", &idat), (b"IEND", &Vec::new())] {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut crc = flate2::Crc::new();
            crc.update(kind);
            crc.update(data);
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            png.extend_from_slice(&crc.sum().to_be_bytes());
        }
        png
    }
}

/// Runs `ui` for `passes` frames (at least 2, so that fonts and layouts settle) in `ctx` on a
/// screen of `size` points at `pixels_per_point`, and rasterises the last frame.
pub fn render(
    ctx: &egui::Context,
    size: Vec2,
    pixels_per_point: f32,
    passes: usize,
    mut ui: impl FnMut(&mut Ui),
) -> Snapshot {
    ctx.set_pixels_per_point(pixels_per_point);
    let mut textures: HashMap<TextureId, ColorImage> = HashMap::new();
    let mut primitives = Vec::new();
    let start = ctx.input(|i| i.time);
    for pass in 0..passes.max(2) {
        let time = Some(start + (pass + 1) as f64 / 60.0);
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time,
            predicted_dt: 1.0 / 60.0,
            ..RawInput::default()
        };
        let mut output = ctx.run_ui(input, &mut ui);
        apply_textures(&mut textures, &mut output.textures_delta);
        primitives = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
    }
    rasterize(
        &primitives,
        &textures,
        (size.x * pixels_per_point).round() as usize,
        (size.y * pixels_per_point).round() as usize,
        pixels_per_point,
    )
}

/// Applies a frame's texture changes (whole images and patches), and marks them handled.
fn apply_textures(textures: &mut HashMap<TextureId, ColorImage>, delta: &mut egui::TexturesDelta) {
    for (id, deltas) in &delta.set {
        for d in deltas {
            let ImageData::Color(image) = &d.image;
            match d.pos {
                None => {
                    textures.insert(*id, (**image).clone());
                }
                Some([x0, y0]) => {
                    let Some(target) = textures.get_mut(id) else { continue };
                    // Clipped to the target (egui's patches always fit; a stray one must not wrap).
                    let w = target.size[0];
                    for y in 0..image.size[1].min(target.size[1].saturating_sub(y0)) {
                        for x in 0..image.size[0].min(w.saturating_sub(x0)) {
                            target.pixels[(y0 + y) * w + x0 + x] = image.pixels[y * image.size[0] + x];
                        }
                    }
                }
            }
        }
    }
    for id in &delta.free {
        textures.remove(id);
    }
    delta.clear();
}

/// A texel of `image` at normalised `uv` (nearest), premultiplied, as f32.
fn sample(image: Option<&ColorImage>, uv: Pos2) -> [f32; 4] {
    let Some(image) = image.filter(|i| i.size[0] > 0 && i.size[1] > 0) else { return [255.0; 4] };
    let x = ((uv.x * image.size[0] as f32) as usize).min(image.size[0] - 1);
    let y = ((uv.y * image.size[1] as f32) as usize).min(image.size[1] - 1);
    let c = image.pixels[y * image.size[0] + x];
    [f32::from(c.r()), f32::from(c.g()), f32::from(c.b()), f32::from(c.a())]
}

/// The offset of the sample point from the pixel centre (see [`rasterize`]).
const NUDGE: (f32, f32) = (1.0 / 1031.0, 1.0 / 1949.0);

/// Rasterises tessellated meshes into a `width × height` image (premultiplied alpha blending in
/// gamma space, as egui's own renderers do).
pub fn rasterize(
    primitives: &[ClippedPrimitive],
    textures: &HashMap<TextureId, ColorImage>,
    width: usize,
    height: usize,
    pixels_per_point: f32,
) -> Snapshot {
    let mut rgba = vec![0u8; width * height * 4];
    for clipped in primitives {
        let Primitive::Mesh(mesh) = &clipped.primitive else { continue };
        let clip = clipped.clip_rect;
        let (cx0, cy0) =
            ((clip.min.x * pixels_per_point).max(0.0) as usize, (clip.min.y * pixels_per_point).max(0.0) as usize);
        let (cx1, cy1) = (
            ((clip.max.x * pixels_per_point).ceil().max(0.0) as usize).min(width),
            ((clip.max.y * pixels_per_point).ceil().max(0.0) as usize).min(height),
        );
        let texture = textures.get(&mesh.texture_id);
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v: [&Vertex; 3] =
                [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let p: [Pos2; 3] =
                [v[0].pos, v[1].pos, v[2].pos].map(|p| Pos2::new(p.x * pixels_per_point, p.y * pixels_per_point));
            let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[1].y - p[0].y) * (p[2].x - p[0].x);
            if area.abs() < 1e-9 {
                continue;
            }
            let x0 = (p.iter().map(|q| q.x).fold(f32::INFINITY, f32::min).floor().max(cx0 as f32)) as usize;
            let x1 = (p.iter().map(|q| q.x).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(cx1);
            let y0 = (p.iter().map(|q| q.y).fold(f32::INFINITY, f32::min).floor().max(cy0 as f32)) as usize;
            let y1 = (p.iter().map(|q| q.y).fold(f32::NEG_INFINITY, f32::max).ceil().max(0.0) as usize).min(cy1);
            for y in y0..y1 {
                for x in x0..x1 {
                    // The pixel centre, nudged off the half-pixel grid egui's vertices sit on: an
                    // edge shared by two triangles then covers it exactly once (no gaps, no seams).
                    let c = Pos2::new(x as f32 + 0.5 + NUDGE.0, y as f32 + 0.5 + NUDGE.1);
                    let w0 = ((p[1].x - c.x) * (p[2].y - c.y) - (p[1].y - c.y) * (p[2].x - c.x)) / area;
                    let w1 = ((p[2].x - c.x) * (p[0].y - c.y) - (p[2].y - c.y) * (p[0].x - c.x)) / area;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let uv = Pos2::new(
                        v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2,
                        v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2,
                    );
                    let texel = sample(texture, uv);
                    let channel = |f: fn(&Color32) -> u8, k: usize| {
                        let vc = f32::from(f(&v[0].color)) * w0
                            + f32::from(f(&v[1].color)) * w1
                            + f32::from(f(&v[2].color)) * w2;
                        vc * texel[k] / 255.0
                    };
                    let src = [
                        channel(Color32::r, 0),
                        channel(Color32::g, 1),
                        channel(Color32::b, 2),
                        channel(Color32::a, 3),
                    ];
                    let i = (y * width + x) * 4;
                    let keep = 1.0 - src[3] / 255.0;
                    for k in 0..4 {
                        rgba[i + k] = (src[k] + f32::from(rgba[i + k]) * keep).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    Snapshot { width, height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_on_a_filled_rect() {
        let ctx = egui::Context::default();
        let shot = render(&ctx, egui::vec2(120.0, 40.0), 1.0, 2, |ui| {
            ui.painter().rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(10, 20, 30));
            ui.label("stone 643");
        });
        assert_eq!((shot.width, shot.height), (120, 40));
        assert_eq!(shot.pixel(110, 35), [10, 20, 30, 255]);
        assert!(shot.count_not([10, 20, 30, 255], 8) > 20, "the text left pixels of its own");
        let png = shot.to_png();
        assert!(
            png.starts_with(b"\x89PNG\r\n\x1a\n") && png.ends_with(&[0xae, 0x42, 0x60, 0x82]),
            "IEND's CRC closes the file"
        );
        let empty = Snapshot { width: 1, height: 1, rgba: vec![0; 4] };
        assert!(empty.to_png().len() > 40);
    }

    #[test]
    fn texture_patches_and_frees_apply() {
        let mut textures = HashMap::new();
        let id = TextureId::User(7);
        let mut delta = egui::TexturesDelta::default();
        delta.set.insert(
            id,
            [egui::epaint::ImageDelta::full(
                ColorImage::new([2, 2], vec![Color32::BLACK; 4]),
                egui::TextureOptions::NEAREST,
            )]
            .into_iter()
            .collect(),
        );
        apply_textures(&mut textures, &mut delta);
        let mut patch = egui::TexturesDelta::default();
        let one = ColorImage::new([1, 1], vec![Color32::WHITE]);
        patch.set.insert(
            id,
            [egui::epaint::ImageDelta::partial([1, 1], one.clone(), egui::TextureOptions::NEAREST)]
                .into_iter()
                .collect(),
        );
        patch.set.insert(
            TextureId::User(8),
            [egui::epaint::ImageDelta::partial([0, 0], one, egui::TextureOptions::NEAREST)].into_iter().collect(),
        );
        apply_textures(&mut textures, &mut patch);
        assert_eq!(textures[&id].pixels[3], Color32::WHITE);
        assert_eq!(sample(textures.get(&id), Pos2::new(0.9, 0.9)), [255.0; 4]);
        assert_eq!(sample(None, Pos2::ZERO), [255.0; 4]);
        // A patch over the right and bottom edges is clipped, not wrapped into the next row.
        let mut edge = egui::TexturesDelta::default();
        let red = ColorImage::new([2, 2], vec![Color32::RED; 4]);
        edge.set.insert(
            id,
            [egui::epaint::ImageDelta::partial([1, 0], red, egui::TextureOptions::NEAREST)].into_iter().collect(),
        );
        apply_textures(&mut textures, &mut edge);
        assert_eq!(textures[&id].pixels, [Color32::BLACK, Color32::RED, Color32::BLACK, Color32::RED]);
        let mut free = egui::TexturesDelta::default();
        free.free.insert(id);
        apply_textures(&mut textures, &mut free);
        assert!(textures.is_empty());
    }
}
