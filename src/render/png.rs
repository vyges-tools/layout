// SPDX-License-Identifier: Apache-2.0
//! Scene → PNG. 8-bit RGB, **really** deflated (this crate already carries `miniz_oxide` for
//! OASIS), so the file is a fraction of the size an uncompressed store would produce — which
//! matters, because "PNG so it is small enough to paste somewhere" is the reason the raster
//! back-end exists at all.
//!
//! No anti-aliasing. Every primitive here is an axis-aligned rectangle, a line, or a glyph, and
//! at the sizes these drawings are viewed the cost of a smoothing pass buys very little. Lines
//! are drawn with a Bresenham walk widened perpendicular to their run.

use super::{anchor_x, font::glyph, Anchor, Rgb, Scene, Shape, ADVANCE, CREDIT_COLOR,
            CREDIT_INSET, CREDIT_SIZE};

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u8>, // RGB, row-major
}

impl Canvas {
    fn new(w: usize, h: usize, bg: Rgb) -> Canvas {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&[bg.0, bg.1, bg.2]);
        }
        Canvas { w, h, px }
    }

    /// Blend `c` at `alpha` into the pixel, clipping silently outside the canvas.
    ///
    /// Clipping rather than panicking is deliberate: a caller's scene may be laid out slightly
    /// larger than its declared size, and losing a few edge pixels is a far better failure than
    /// aborting the render.
    fn blend(&mut self, x: i64, y: i64, c: Rgb, alpha: f64) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h || alpha <= 0.0 {
            return;
        }
        let i = (y as usize * self.w + x as usize) * 3;
        let a = alpha.clamp(0.0, 1.0);
        for (k, src) in [c.0, c.1, c.2].into_iter().enumerate() {
            let dst = self.px[i + k] as f64;
            self.px[i + k] = (dst + (src as f64 - dst) * a).round() as u8;
        }
    }

    fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb, alpha: f64) {
        let x0 = x.floor() as i64;
        let y0 = y.floor() as i64;
        let x1 = (x + w).ceil() as i64;
        let y1 = (y + h).ceil() as i64;
        for py in y0..y1 {
            for px in x0..x1 {
                self.blend(px, py, c, alpha);
            }
        }
    }

    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, c: Rgb, width: f64, dashed: bool) {
        let (dx, dy) = (x2 - x1, y2 - y1);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            self.fill_rect(x1, y1, width.max(1.0), width.max(1.0), c, 1.0);
            return;
        }
        let steps = len.ceil() as i64;
        // Perpendicular unit vector, to give the stroke its width.
        let (nx, ny) = (-dy / len, dx / len);
        let half = (width.max(1.0) - 1.0) / 2.0;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            // A 6-on/4-off pattern in device pixels, close enough to the SVG's "4 2" dash at
            // the scales these are viewed at.
            if dashed && (t * len) as i64 % 10 >= 6 {
                continue;
            }
            let (x, y) = (x1 + dx * t, y1 + dy * t);
            let mut o = -half;
            while o <= half + 1e-9 {
                self.blend((x + nx * o).round() as i64, (y + ny * o).round() as i64, c, 1.0);
                o += 1.0;
            }
        }
    }

    /// Draw `text` with its **baseline** at `y` and its left edge at `x`.
    ///
    /// The 5x7 cell is scaled by an integer factor so glyphs stay crisp; sub-pixel scaling of a
    /// bitmap font produces dropped rows and unreadable text at these sizes.
    fn text(&mut self, x: f64, y: f64, text: &str, size: f64, c: Rgb, bold: bool) {
        let px = ((size / 7.0).round() as i64).max(1); // device pixels per font pixel
        let cell = size * ADVANCE;
        // Baseline sits at the bottom of the 7-row cell.
        let top = y - 7.0 * px as f64;
        for (i, ch) in text.chars().enumerate() {
            let Some(cols) = glyph(ch) else {
                continue; // unmapped: leave the cell blank rather than draw a wrong glyph
            };
            let gx = x + i as f64 * cell;
            for (cx, col) in cols.iter().enumerate() {
                for row in 0..7 {
                    if col & (1 << row) == 0 {
                        continue;
                    }
                    let bx = gx + cx as f64 * px as f64;
                    let by = top + row as f64 * px as f64;
                    self.fill_rect(bx, by, px as f64, px as f64, c, 1.0);
                    if bold {
                        // Smear one device pixel right. Cheap, and at 5x7 a second weight would
                        // mean a second font.
                        self.fill_rect(bx + 1.0, by, px as f64, px as f64, c, 1.0);
                    }
                }
            }
        }
    }
}

pub fn to_png(s: &Scene, scale: f64) -> Vec<u8> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let w = ((s.width * scale).round() as usize).max(1);
    let h = ((s.height * scale).round() as usize).max(1);
    let mut cv = Canvas::new(w, h, s.background);

    for shape in &s.shapes {
        match shape {
            Shape::Rect {
                x,
                y,
                w,
                h,
                fill,
                fill_opacity,
                stroke,
                stroke_width,
            } => {
                let (x, y, w, h) = (x * scale, y * scale, w * scale, h * scale);
                if let Some(c) = fill {
                    cv.fill_rect(x, y, w, h, *c, *fill_opacity);
                }
                if let Some(c) = stroke {
                    let t = (stroke_width * scale).max(1.0);
                    cv.fill_rect(x, y, w, t, *c, 1.0);
                    cv.fill_rect(x, y + h - t, w, t, *c, 1.0);
                    cv.fill_rect(x, y, t, h, *c, 1.0);
                    cv.fill_rect(x + w - t, y, t, h, *c, 1.0);
                }
            }
            Shape::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
                width,
                dashed,
            } => cv.line(
                x1 * scale,
                y1 * scale,
                x2 * scale,
                y2 * scale,
                *stroke,
                width * scale,
                *dashed,
            ),
            Shape::Text {
                x,
                y,
                text,
                size,
                fill,
                anchor,
                bold,
            } => {
                // Anchor in scene units so the two back-ends place text identically.
                let left = anchor_x(*x, text, *size, *anchor);
                cv.text(left * scale, y * scale, text, size * scale, *fill, *bold);
            }
        }
    }
    // Last, so it is never painted over by the scene's own shapes.
    if let Some(c) = &s.credit {
        let x = anchor_x(s.width - CREDIT_INSET, c, CREDIT_SIZE, Anchor::End);
        cv.text(
            x * scale,
            (s.height - CREDIT_INSET) * scale,
            c,
            CREDIT_SIZE * scale,
            CREDIT_COLOR,
            false,
        );
    }
    encode(&cv.px, w as u32, h as u32)
}

// ── PNG container ───────────────────────────────────────────────────────────────────────────

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, e) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn chunk(out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(data);
    let mut crc_in = Vec::with_capacity(4 + data.len());
    crc_in.extend_from_slice(typ);
    crc_in.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

fn encode(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    // Each scanline is prefixed with its filter type. Filter 0 (None) keeps this simple; the
    // deflate pass below recovers most of what a smarter filter would have won on drawings whose
    // rows are mostly flat colour.
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    for row in 0..h as usize {
        raw.push(0);
        let s = row * w as usize * 3;
        raw.extend_from_slice(&px[s..s + w as usize * 3]);
    }
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);

    let mut out = Vec::with_capacity(z.len() + 128);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit, truecolour RGB, no interlace
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::super::tests::sample;
    use super::super::{Anchor, Scene, Shape};
    use super::*;

    fn dims(png: &[u8]) -> (u32, u32) {
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        (w, h)
    }

    #[test]
    fn it_is_a_valid_png() {
        let p = to_png(&sample(), 1.0);
        assert_eq!(&p[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
        assert_eq!(dims(&p), (200, 100));
    }

    #[test]
    fn scale_multiplies_the_pixel_dimensions() {
        assert_eq!(dims(&to_png(&sample(), 2.0)), (400, 200));
        // A nonsense scale must not produce a zero-sized or panicking render.
        assert_eq!(dims(&to_png(&sample(), 0.0)), (200, 100));
        assert_eq!(dims(&to_png(&sample(), f64::NAN)), (200, 100));
    }

    #[test]
    fn the_output_is_actually_compressed() {
        // The whole reason to prefer PNG here is size. A flat drawing is enormously
        // compressible, so an encoder that stored raw would show up immediately.
        let s = Scene::new(400.0, 400.0);
        let p = to_png(&s, 1.0);
        let raw = 400 * 400 * 3;
        assert!(p.len() * 20 < raw, "a blank 400x400 encoded to {} bytes", p.len());
    }

    #[test]
    fn shapes_actually_reach_the_pixels() {
        // Guards the whole draw path: an encoder that emits a valid but blank PNG passes every
        // structural check above.
        let blank = to_png(&Scene::new(200.0, 100.0).with_background((255, 255, 255)), 1.0);
        let drawn = to_png(&sample(), 1.0);
        assert_ne!(blank, drawn, "the scene rendered to an empty image");
    }

    #[test]
    fn text_lands_where_the_anchor_says() {
        // Middle-anchored text must not render at the same pixels as start-anchored text, which
        // is what happens if the raster path ignores the anchor the vector path honours.
        let mk = |a: Anchor| {
            let mut s = Scene::new(200.0, 40.0).with_background((255, 255, 255));
            s.push(Shape::text(100.0, 25.0, "abcd", 12.0, (0, 0, 0)).anchored(a));
            to_png(&s, 1.0)
        };
        assert_ne!(mk(Anchor::Start), mk(Anchor::Middle));
        assert_ne!(mk(Anchor::Middle), mk(Anchor::End));
    }

    #[test]
    fn an_unmapped_character_is_skipped_rather_than_fatal() {
        let mut s = Scene::new(100.0, 40.0);
        s.push(Shape::text(5.0, 25.0, "a\u{4e2d}b", 12.0, (0, 0, 0)));
        assert_eq!(dims(&to_png(&s, 1.0)), (100, 40));
    }

    #[test]
    fn geometry_outside_the_canvas_is_clipped_not_fatal() {
        // A scene laid out slightly larger than its declared size is an ordinary caller bug;
        // losing the overhang beats aborting the render.
        let mut s = Scene::new(50.0, 50.0);
        s.push(Shape::rect(-100.0, -100.0, 500.0, 500.0, (1, 2, 3), 0.5, 2.0));
        s.push(Shape::Line {
            x1: -50.0,
            y1: -50.0,
            x2: 500.0,
            y2: 500.0,
            stroke: (9, 9, 9),
            width: 3.0,
            dashed: false,
        });
        assert_eq!(dims(&to_png(&s, 1.0)), (50, 50));
    }
}
