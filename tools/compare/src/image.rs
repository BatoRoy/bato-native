//! RGB images, the diff metrics and the diff / side-by-side renderings.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::Path;

#[derive(Clone)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    /// Row-major, 3 bytes per pixel. Alpha is dropped on load.
    pub px: Vec<u8>,
}

/// An axis-aligned pixel rectangle, `x0..x1` × `y0..y1` (exclusive ends).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    pub fn contains(&self, x: usize, y: usize) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

impl Rgb {
    pub fn new(w: usize, h: usize, fill: [u8; 3]) -> Self {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&fill);
        }
        Rgb { w, h, px }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut dec = png::Decoder::new(BufReader::new(file));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
        let mut buf = vec![0; reader.output_buffer_size().ok_or("png: image too large")?];
        let info = reader.next_frame(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        let (w, h) = (info.width as usize, info.height as usize);
        let src = &buf[..info.buffer_size()];
        let ch = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err(format!("{}: unexpanded palette", path.display())),
        };
        let mut px = Vec::with_capacity(w * h * 3);
        for p in src.chunks_exact(ch) {
            match ch {
                1 | 2 => px.extend_from_slice(&[p[0], p[0], p[0]]),
                _ => px.extend_from_slice(&p[..3]),
            }
        }
        Ok(Rgb { w, h, px })
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(BufWriter::new(file), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);
        let mut wr = enc.write_header().map_err(|e| format!("{}: {e}", path.display()))?;
        wr.write_image_data(&self.px).map_err(|e| format!("{}: {e}", path.display()))?;
        wr.finish().map_err(|e| format!("{}: {e}", path.display()))
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * 3;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, c: [u8; 3]) {
        let i = (y * self.w + x) * 3;
        self.px[i..i + 3].copy_from_slice(&c);
    }

    /// Pixel-equal outside `mask`.
    pub fn same_as(&self, other: &Rgb, mask: &[Rect]) -> bool {
        if self.w != other.w || self.h != other.h {
            return false;
        }
        if mask.is_empty() {
            return self.px == other.px;
        }
        let row = self.w * 3;
        (0..self.h).all(|y| {
            let a = &self.px[y * row..(y + 1) * row];
            let b = &other.px[y * row..(y + 1) * row];
            a == b || (0..self.w).all(|x| a[x * 3..x * 3 + 3] == b[x * 3..x * 3 + 3] || mask.iter().any(|m| m.contains(x, y)))
        })
    }
}

pub struct Metrics {
    /// Mean absolute difference per RGB channel, 0–255, over unmasked pixels.
    pub mean: f64,
    /// Pixels whose largest channel difference exceeds the threshold.
    pub over: usize,
    pub over_pct: f64,
    /// Pixels whose summed RGB difference exceeds 24 (the PoC README's second column).
    pub over24_sum: usize,
    /// Bounding box of the `over` pixels.
    pub bbox: Option<Rect>,
    pub compared: usize,
    /// Set when the images differ in size: only the common top-left area is compared.
    pub size_note: Option<String>,
}

/// The compositor rounds window corners (and blends them with whatever is
/// behind), so the 4×4 px in each corner of a window capture are not app pixels.
pub const CORNER: usize = 4;

fn corners(w: usize, h: usize) -> [Rect; 4] {
    let c = CORNER.min(w).min(h);
    [
        Rect { x0: 0, y0: 0, x1: c, y1: c },
        Rect { x0: w - c, y0: 0, x1: w, y1: c },
        Rect { x0: 0, y0: h - c, x1: c, y1: h },
        Rect { x0: w - c, y0: h - c, x1: w, y1: h },
    ]
}

/// Compares `a` (reference) with `b` (candidate). `mask` rectangles (the stray
/// cursor; see capture.rs) and the window corners are left out of every number.
pub fn diff(a: &Rgb, b: &Rgb, threshold: u8, mask: &[Rect]) -> (Metrics, Rgb) {
    let (w, h) = (a.w.min(b.w), a.h.min(b.h));
    let mask: Vec<Rect> = mask.iter().copied().chain(corners(w, h)).collect();
    let mut img = Rgb::new(w, h, [0, 0, 0]);
    let (mut sum, mut n, mut over, mut over24) = (0u64, 0usize, 0usize, 0usize);
    let mut bbox: Option<Rect> = None;
    for y in 0..h {
        for x in 0..w {
            let (p, q) = (a.get(x, y), b.get(x, y));
            let luma = (p[0] as u32 * 2 + p[1] as u32 * 5 + p[2] as u32) / 8;
            let base = (luma * 35 / 100) as u8;
            if mask.iter().any(|m| m.contains(x, y)) {
                img.set(x, y, [base / 2, base / 2, base / 2 + 70]);
                continue;
            }
            let d = [p[0].abs_diff(q[0]), p[1].abs_diff(q[1]), p[2].abs_diff(q[2])];
            let dmax = d[0].max(d[1]).max(d[2]);
            let dsum = d[0] as u32 + d[1] as u32 + d[2] as u32;
            sum += dsum as u64;
            n += 1;
            if dsum > 24 {
                over24 += 1;
            }
            let c = if dmax > threshold {
                over += 1;
                bbox = Some(match bbox {
                    None => Rect { x0: x, y0: y, x1: x + 1, y1: y + 1 },
                    Some(r) => Rect { x0: r.x0.min(x), y0: r.y0.min(y), x1: r.x1.max(x + 1), y1: r.y1.max(y + 1) },
                });
                // Red, brighter with the size of the difference.
                [(150 + (dmax as u32).min(105)) as u8, 30, 40]
            } else if dmax > 0 {
                // Below threshold (anti-aliasing noise): faint amber.
                let k = 40 + dmax as u32 * 60 / threshold.max(1) as u32;
                [(base as u32 + k).min(255) as u8, (base as u32 + k * 3 / 4).min(255) as u8, base]
            } else {
                [base, base, base]
            };
            img.set(x, y, c);
        }
    }
    if let Some(r) = bbox {
        outline(&mut img, r, [0, 220, 230]);
    }
    let size_note = (a.w != b.w || a.h != b.h).then(|| format!("size {}x{} vs {}x{}", a.w, a.h, b.w, b.h));
    let m = Metrics {
        mean: if n > 0 { sum as f64 / (n as f64 * 3.0) } else { 0.0 },
        over,
        over_pct: if n > 0 { over as f64 * 100.0 / n as f64 } else { 0.0 },
        over24_sum: over24,
        bbox,
        compared: n,
        size_note,
    };
    (m, img)
}

fn outline(img: &mut Rgb, r: Rect, c: [u8; 3]) {
    let (x1, y1) = (r.x1.min(img.w) - 1, r.y1.min(img.h) - 1);
    for x in r.x0..=x1 {
        img.set(x, r.y0, c);
        img.set(x, y1, c);
    }
    for y in r.y0..=y1 {
        img.set(r.x0, y, c);
        img.set(x1, y, c);
    }
}

/// `[reference | candidate | diff]`, 8 px gutters.
pub fn side_by_side(panels: &[&Rgb]) -> Rgb {
    const GAP: usize = 8;
    let h = panels.iter().map(|p| p.h).max().unwrap_or(0);
    let w = panels.iter().map(|p| p.w).sum::<usize>() + GAP * panels.len().saturating_sub(1);
    let mut out = Rgb::new(w, h, [70, 70, 78]);
    let mut ox = 0;
    for p in panels {
        for y in 0..p.h {
            let (s, d) = (y * p.w * 3, (y * w + ox) * 3);
            out.px[d..d + p.w * 3].copy_from_slice(&p.px[s..s + p.w * 3]);
        }
        ox += p.w + GAP;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_have_zero_diff() {
        let a = Rgb::new(4, 3, [10, 20, 30]);
        let (m, _) = diff(&a, &a.clone(), 8, &[]);
        assert_eq!((m.mean, m.over, m.bbox), (0.0, 0, None));
    }

    #[test]
    fn metrics_threshold_bbox_and_mask() {
        let a = Rgb::new(20, 10, [0, 0, 0]);
        let mut b = a.clone();
        b.set(5, 3, [9, 0, 0]); // over 8
        b.set(7, 5, [0, 8, 0]); // not over 8
        b.set(9, 9, [255, 255, 255]); // masked
        b.set(0, 0, [255, 255, 255]); // window corner, always masked
        let mask = [Rect { x0: 9, y0: 9, x1: 10, y1: 10 }];
        let (m, _) = diff(&a, &b, 8, &mask);
        assert_eq!(m.over, 1);
        let n = 200 - 1 - 4 * 16;
        assert_eq!(m.compared, n);
        assert_eq!(m.bbox, Some(Rect { x0: 5, y0: 3, x1: 6, y1: 4 }));
        assert!((m.mean - 17.0 / (n as f64 * 3.0)).abs() < 1e-9);
        assert!(a.same_as(&a.clone(), &[]));
        assert!(!a.same_as(&b, &mask));
    }

    #[test]
    fn png_roundtrip() {
        let mut a = Rgb::new(5, 2, [1, 2, 3]);
        a.set(4, 1, [200, 100, 50]);
        let p = std::env::temp_dir().join(format!("bato-compare-test-{}.png", std::process::id()));
        a.save(&p).unwrap();
        let b = Rgb::load(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert!(a.same_as(&b, &[]));
    }
}
