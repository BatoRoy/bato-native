//! `QrCode`: a QR code (`qrcode`'s defaults as Bato apps use them: error
//! correction M, a one-module quiet zone, black modules on a white plate),
//! drawn as quads snapped to device pixels so every module is crisp at any
//! UI scale. Square, `size` px wide (default 160); the modules take the
//! largest whole number of device pixels that fits, centred.
//!
//! ```ignore
//! qr_code("https://example.com/s/abc").size(176.0)
//! ```

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::{Color, Element, Length, Rectangle, Size, mouse};
use qrcodegen::{QrCode as Code, QrCodeEcc};

use crate::anim::fade;
use crate::scale::device_scale;

pub struct QrCode {
    /// Module rows, `true` = dark; `None` when the text does not fit a QR
    /// code (then the plate is drawn empty).
    modules: Option<Vec<Vec<bool>>>,
    size: f32,
    quiet: usize,
    dark: Color,
    light: Color,
}

/// The QR code for `text` (error correction M).
pub fn qr_code(text: &str) -> QrCode {
    QrCode {
        modules: encode(text),
        size: 160.0,
        quiet: 1,
        dark: Color::BLACK,
        light: Color::WHITE,
    }
}

/// The module matrix for `text`, or `None` if it is too long.
pub fn encode(text: &str) -> Option<Vec<Vec<bool>>> {
    let qr = Code::encode_text(text, QrCodeEcc::Medium).ok()?;
    let n = qr.size();
    Some(
        (0..n)
            .map(|y| (0..n).map(|x| qr.get_module(x, y)).collect())
            .collect(),
    )
}

impl QrCode {
    /// Width and height, logical px.
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }
    /// Quiet zone in modules (default 1).
    pub fn quiet_zone(mut self, modules: usize) -> Self {
        self.quiet = modules;
        self
    }
    pub fn colors(mut self, dark: Color, light: Color) -> Self {
        self.dark = dark;
        self.light = light;
        self
    }
    /// Whether the text could be encoded.
    pub fn is_valid(&self) -> bool {
        self.modules.is_some()
    }
}

/// Device-pixel module size and the offset that centres `n` modules in
/// `side` device px.
fn fit(side: f32, n: usize) -> (f32, f32) {
    let m = (side / n as f32).floor().max(1.0);
    (m, ((side - m * n as f32) / 2.0).floor())
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for QrCode {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let s = Size::new(self.size, self.size);
        layout::Node::new(limits.resolve(self.size, self.size, s))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let b = layout.bounds();
        let s = device_scale();
        let quad = |x: f32, y: f32, w: f32, h: f32| Quad {
            bounds: Rectangle {
                x: x / s,
                y: y / s,
                width: w / s,
                height: h / s,
            },
            ..Quad::default()
        };
        let (x0, y0) = ((b.x * s).round(), (b.y * s).round());
        let side = (b.width.min(b.height) * s).round();
        renderer.fill_quad(quad(x0, y0, side, side), fade(self.light));
        let Some(rows) = &self.modules else {
            return;
        };
        let n = rows.len() + 2 * self.quiet;
        let (m, off) = fit(side, n);
        let origin = (
            x0 + off + m * self.quiet as f32,
            y0 + off + m * self.quiet as f32,
        );
        let dark = fade(self.dark);
        for (y, row) in rows.iter().enumerate() {
            let mut x = 0;
            while x < row.len() {
                if !row[x] {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < row.len() && row[x] {
                    x += 1;
                }
                renderer.fill_quad(
                    quad(
                        origin.0 + m * start as f32,
                        origin.1 + m * y as f32,
                        m * (x - start) as f32,
                        m,
                    ),
                    dark,
                );
            }
        }
    }
}

impl<'a, M: 'a> From<QrCode> for Element<'a, M, iced::Theme, iced::Renderer> {
    fn from(q: QrCode) -> Self {
        Element::new(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_the_qrcode_package() {
        // A short URL: version 2 (25 modules) at error correction M.
        let m = encode("https://example.com/s/abc").expect("fits");
        assert_eq!(m.len(), 25);
        assert!(m.iter().all(|r| r.len() == 25));
        // The finder pattern's corners are dark.
        assert!(m[0][0] && m[0][6] && m[6][0] && m[24][0] && m[0][24]);
        assert!(!m[7][7]);
        // Too long for any version.
        assert!(encode(&"x".repeat(5000)).is_none());
        assert!(!qr_code(&"x".repeat(5000)).is_valid());
    }

    #[test]
    fn modules_are_whole_device_pixels() {
        // 160 px at 1x, 27 modules (25 + quiet zone): 5 px each, centred.
        assert_eq!(fit(160.0, 27), (5.0, 12.0));
        // At 1.5x: 240 device px, 8 px modules.
        assert_eq!(fit(240.0, 27), (8.0, 12.0));
        assert_eq!(fit(10.0, 27).0, 1.0);
    }
}
